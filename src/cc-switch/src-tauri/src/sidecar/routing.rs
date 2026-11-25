use crate::app_config::AppType;
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::proxy::provider_router::provider_supports_failover;
use crate::proxy::types::AppProxyConfig;
use rusqlite::params;
use std::collections::HashSet;

pub(super) enum Change<'a> {
    Activate {
        id: &'a str,
        queue: Option<Vec<String>>,
    },
    Configure {
        enabled: Option<bool>,
        queue: Option<Vec<String>>,
    },
    Tune(&'a AppProxyConfig),
    Delete(&'a str),
}

/// Call with the sidecar route lock held. Validation precedes all changes, and
/// the selection, queue membership, ordering, and enable flag commit together.
pub(super) fn apply(
    db: &Database,
    app: &AppType,
    change: Change<'_>,
) -> Result<(Option<String>, bool), AppError> {
    let mut providers = db.get_provider_rows(app)?.readable;
    let mut current = db
        .get_current_provider(app)?
        .filter(|id| providers.contains_key(id));
    let (_, mut enabled) = db.get_proxy_flags_sync(app);
    let (requested_queue, provider_to_delete, proxy_config) = match change {
        Change::Activate { id, queue } => {
            if !providers.contains_key(id) {
                return Err(AppError::InvalidInput(format!("Provider '{id}' not found")));
            }
            current = Some(id.to_string());
            (queue, None, None)
        }
        Change::Configure {
            enabled: value,
            queue,
        } => {
            enabled = value.unwrap_or(enabled);
            (queue, None, None)
        }
        Change::Tune(config) => {
            enabled = config.auto_failover_enabled;
            (None, None, Some(config))
        }
        Change::Delete(id) => {
            providers.shift_remove(id);
            if current.as_deref() == Some(id) {
                current = None;
            }
            if providers.is_empty() {
                enabled = false;
            }
            (None, Some(id), None)
        }
    };

    let stored_queue = db.get_failover_queue(app)?;
    let eligible = |id: &str| {
        providers
            .get(id)
            .is_some_and(|provider| provider_supports_failover(app, provider))
    };
    let mut queue = match requested_queue {
        Some(ids) => {
            if let Some(id) = ids.iter().find(|id| !eligible(id)) {
                return Err(AppError::InvalidInput(format!(
                    "Provider '{id}' cannot be added to the failover queue"
                )));
            }
            ids
        }
        None => stored_queue
            .iter()
            .filter(|entry| eligible(&entry.provider_id))
            .map(|entry| entry.provider_id.clone())
            .collect(),
    };

    if current.is_none() && enabled {
        current = queue.first().cloned();
    }
    if current.is_none() && provider_to_delete.is_some() {
        current = providers.first().map(|(id, _)| id.clone());
    }
    if enabled {
        let Some(id) = current.as_ref() else {
            return Err(AppError::InvalidInput(
                "Activate a provider before enabling automatic failover".into(),
            ));
        };
        if eligible(id) {
            queue.insert(0, id.clone());
        }
    }
    let mut seen = HashSet::new();
    queue.retain(|id| seen.insert(id.clone()));

    let mut conn = lock_conn!(db.conn);
    let tx = conn.transaction()?;
    if let Some(id) = provider_to_delete {
        tx.execute(
            "DELETE FROM providers WHERE id = ?1 AND app_type = ?2",
            params![id, app],
        )?;
    }
    tx.execute(
        "UPDATE providers SET is_current = (id IS ?2) WHERE app_type = ?1 AND is_current != (id IS ?2)",
        params![app, current],
    )?;
    for entry in stored_queue
        .iter()
        .filter(|entry| !seen.contains(&entry.provider_id))
    {
        tx.execute(
            "UPDATE providers SET in_failover_queue = 0 WHERE id = ?1 AND app_type = ?2",
            params![entry.provider_id, app],
        )?;
        tx.execute(
            "DELETE FROM provider_health WHERE provider_id = ?1 AND app_type = ?2",
            params![entry.provider_id, app],
        )?;
    }
    for (index, id) in queue.iter().enumerate() {
        tx.execute(
            "UPDATE providers SET in_failover_queue = 1, sort_index = ?3
             WHERE app_type = ?1 AND id = ?2 AND (in_failover_queue = 0 OR sort_index IS NOT ?3)",
            params![app, id, index],
        )?;
    }
    if let Some(config) = proxy_config {
        Database::write_app_proxy_config(&tx, config)?;
    } else {
        tx.execute(
            "UPDATE proxy_config SET enabled = 1, auto_failover_enabled = ?2, updated_at = datetime('now')
             WHERE app_type = ?1 AND (auto_failover_enabled != ?2 OR enabled != 1)",
            params![app, enabled],
        )?;
    }
    tx.commit()?;
    db.advance_route_revision(app);
    Ok((current, enabled))
}

pub(crate) fn record_failover(
    db: &Database,
    app: &AppType,
    id: &str,
    revision: u64,
) -> Result<bool, AppError> {
    db.with_sidecar_route(|| {
        if db.route_revision(app) != revision || !db.get_proxy_flags_sync(app).1 {
            return Ok(false);
        }
        let Some(provider) = db.get_provider_by_id(id, app)? else {
            return Ok(false);
        };
        if !provider.in_failover_queue || !provider_supports_failover(app, &provider) {
            return Ok(false);
        }
        apply(db, app, Change::Activate { id, queue: None })?;
        Ok(true)
    })
}

pub(super) fn reconcile_existing_queues(db: &Database) -> Result<(), AppError> {
    db.with_sidecar_route(|| {
        for app in AppType::ALL {
            if !db.get_proxy_flags_sync(&app).1 {
                continue;
            }
            let change = Change::Configure {
                enabled: None,
                queue: None,
            };
            match apply(db, &app, change) {
                Ok(_) => {}
                Err(AppError::InvalidInput(_)) => {
                    apply(
                        db,
                        &app,
                        Change::Configure {
                            enabled: Some(false),
                            queue: None,
                        },
                    )?;
                }
                Err(other) => return Err(other),
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Provider;

    #[test]
    fn stale_failover_result_cannot_restore_a_deleted_provider() {
        let db = Database::memory().unwrap();
        let app = AppType::Codex;
        for id in ["primary", "fallback"] {
            let mut provider = Provider::with_id(id.into(), id.into(), serde_json::json!({}), None);
            provider.in_failover_queue = true;
            db.save_provider(&app, &provider).unwrap();
        }
        db.set_current_provider(&app, "primary").unwrap();
        db.set_proxy_flags_sync(&app, true, true).unwrap();
        let request_revision = db.route_revision(&app);
        db.with_sidecar_route(|| apply(&db, &app, Change::Delete("fallback")))
            .unwrap();
        assert!(!record_failover(&db, &app, "fallback", request_revision).unwrap());
        assert!(!record_failover(&db, &app, "fallback", db.route_revision(&app)).unwrap());
        assert_eq!(
            db.get_current_provider(&app).unwrap().as_deref(),
            Some("primary")
        );
    }
}
