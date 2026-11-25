use crate::error::AppError;

#[cfg(windows)]
const PREFIX: &str = "dpapi:v1:";

#[cfg(windows)]
pub(crate) fn migrate_legacy_storage(
    db: &crate::database::Database,
    data_dir: &std::path::Path,
) -> Result<(), AppError> {
    use crate::database::lock_conn;
    use rusqlite::{Connection, OpenFlags};
    use std::fs;

    {
        let mut conn = lock_conn!(db.conn);
        migrate_connection(&mut conn)?;
    }

    // Earlier builds could make ordinary SQLite snapshots before protecting
    // the live database. Only migrate recognized backups in the isolated store.
    let backup_dir = data_dir.join("backups");
    let entries = match fs::read_dir(&backup_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(AppError::io(&backup_dir, error)),
    };
    let canonical_backup =
        fs::canonicalize(&backup_dir).map_err(|e| AppError::io(&backup_dir, e))?;
    let canonical_data = fs::canonicalize(data_dir).map_err(|e| AppError::io(data_dir, e))?;
    if canonical_backup.parent() != Some(canonical_data.as_path()) {
        return Err(AppError::Config(
            "Credential backup directory is outside the sidecar store".into(),
        ));
    }
    for entry in entries {
        let entry = entry.map_err(|e| AppError::io(&backup_dir, e))?;
        let path = entry.path();
        let name = entry.file_name();
        if !name.to_str().is_some_and(is_database_backup_name)
            || !entry
                .file_type()
                .map_err(|e| AppError::io(&path, e))?
                .is_file()
        {
            continue;
        }
        let migrate = || {
            let mut conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            migrate_connection(&mut conn)
        };
        if let Err(error) = migrate() {
            log::warn!(
                "Could not protect isolated backup {}: {error}",
                name.to_string_lossy()
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_database_backup_name(name: &str) -> bool {
    let Some(timestamp) = name
        .strip_prefix("db_backup_")
        .and_then(|name| name.strip_suffix(".db"))
    else {
        return false;
    };
    let mut parts = timestamp.split('_');
    let date = parts.next().unwrap_or_default();
    let time = parts.next().unwrap_or_default();
    let suffix = parts.next();
    date.len() == 8
        && time.len() == 6
        && date
            .bytes()
            .chain(time.bytes())
            .all(|byte| byte.is_ascii_digit())
        && suffix
            .is_none_or(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts.next().is_none()
}

#[cfg(windows)]
fn migrate_connection(conn: &mut rusqlite::Connection) -> Result<(), AppError> {
    use rusqlite::{params, OptionalExtension};
    use serde_json::Value;

    const MIGRATION: &str = "vikea_dpapi_storage_v1";
    const GLOBAL_PROXY_URL: &str = "global_proxy_url";
    conn.execute_batch("PRAGMA secure_delete = ON; PRAGMA temp_store = MEMORY;")?;
    let has_settings: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'settings')",
        [],
        |row| row.get(0),
    )?;
    if has_settings
        && conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE key = ?1)",
            [MIGRATION],
            |row| row.get::<_, bool>(0),
        )?
    {
        return Ok(());
    }

    let tx = conn.transaction()?;
    let cols = {
        let mut stmt = tx.prepare("SELECT name FROM pragma_table_info('providers')")?;
        let names = stmt.query_map([], |row| row.get::<_, String>(0))?;
        names.collect::<Result<Vec<_>, _>>()?
    };
    let rows = if cols.iter().any(|column| column == "settings_config") {
        // The pre-schema-migration database can predate the meta column.
        let query = if cols.iter().any(|column| column == "meta") {
            "SELECT rowid, id, settings_config, COALESCE(meta, '{}') FROM providers"
        } else {
            "SELECT rowid, id, settings_config, '{}' FROM providers"
        };
        let mut stmt = tx.prepare(query)?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    for (rowid, id, settings, meta) in rows {
        if !settings.starts_with(PREFIX) {
            tx.execute(
                "UPDATE providers SET settings_config = ?1 WHERE rowid = ?2",
                params![protect(settings)?, rowid],
            )?;
        }
        let mut meta: Value = serde_json::from_str(&meta).map_err(|_| {
            AppError::Database(format!(
                "Provider '{id}' has invalid meta during credential migration"
            ))
        })?;
        if let Some(headers) = meta
            .pointer_mut("/localProxyRequestOverrides/headers")
            .and_then(Value::as_object_mut)
        {
            let mut changed = false;
            for value in headers.values_mut() {
                if let Some(raw) = value.as_str().filter(|raw| !raw.starts_with(PREFIX)) {
                    *value = Value::String(protect(raw.to_owned())?);
                    changed = true;
                }
            }
            if changed {
                tx.execute(
                    "UPDATE providers SET meta = ?1 WHERE rowid = ?2",
                    params![meta.to_string(), rowid],
                )?;
            }
        }
    }
    let proxy_url: Option<String> = if has_settings {
        tx.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [GLOBAL_PROXY_URL],
            |row| row.get(0),
        )
        .optional()?
        .flatten()
    } else {
        None
    };
    if let Some(proxy_url) = proxy_url.filter(|url| !url.starts_with(PREFIX)) {
        tx.execute(
            "UPDATE settings SET value = ?1 WHERE key = ?2",
            params![protect(proxy_url)?, GLOBAL_PROXY_URL],
        )?;
    }
    tx.commit()?;

    // Scrub free pages and the previous WAL before recording completion. A restart retries cleanup.
    conn.execute_batch("VACUUM;")?;
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE);", [], |row| row.get(0))?;
    if busy != 0 {
        return Err(AppError::Database(
            "Could not finish credential migration checkpoint".into(),
        ));
    }
    if has_settings {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, '1')",
            [MIGRATION],
        )?;
    }
    Ok(())
}

pub(crate) fn protect(value: String) -> Result<String, AppError> {
    #[cfg(windows)]
    if crate::config::is_sidecar_headless_mode() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let encrypted = dpapi(value.as_bytes(), true)?;
        return Ok(format!("{PREFIX}{}", STANDARD.encode(encrypted)));
    }
    Ok(value)
}

pub(crate) fn unprotect(value: String) -> Result<String, AppError> {
    #[cfg(windows)]
    if crate::config::is_sidecar_headless_mode() {
        if let Some(encoded) = value.strip_prefix(PREFIX) {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            let encrypted = STANDARD
                .decode(encoded)
                .map_err(|_| AppError::Database("Invalid encrypted credential".into()))?;
            return String::from_utf8(dpapi(&encrypted, false)?)
                .map_err(|_| AppError::Database("Invalid decrypted credential".into()));
        }
    }
    Ok(value)
}

#[cfg(windows)]
fn dpapi(input: &[u8], encrypt: bool) -> Result<Vec<u8>, AppError> {
    use std::{ptr, slice};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(input.len())
            .map_err(|_| AppError::Database("Credential payload too large".into()))?,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    // DPAPI owns the output allocation and never mutates the input buffer.
    let result = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if result == 0 {
        return Err(AppError::Database(format!(
            "DPAPI failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    let bytes = if output.cbData == 0 {
        Vec::new()
    } else {
        unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
    };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(bytes)
}
