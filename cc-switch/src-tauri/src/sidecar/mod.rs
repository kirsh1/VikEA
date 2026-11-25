//! Headless CC-Switch Sidecar Mode
//!
//! Exposes the full CC-Switch protocol bridge (`OpenAI Responses` ⇄ `OpenAI Chat` ⇄
//! `Anthropic Messages` ⇄ `Gemini`), streaming SSE rectifiers, circuit-breaker failover,
//! and SQLite provider configuration as an isolated dual-port headless service with
//! zero Tauri desktop windows and zero reads/writes to host `~/.cc-switch` or `~/.codex`.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) mod credentials;
pub(crate) mod routing;

use axum::{
    extract::{Path as AxumPath, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, patch, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use subtle::ConstantTimeEq;

use crate::app_config::AppType;
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::provider::{LocalProxyRequestOverrides, Provider};
use crate::proxy::circuit_breaker::CircuitBreakerConfig;
use crate::proxy::server::{ProxyServer, ProxyState};
use crate::proxy::types::{AppProxyConfig, ProxyConfig, ProxyStatus, RectifierConfig};
use crate::services::model_fetch::{self, FetchedModel};
use crate::services::stream_check::{StreamCheckConfig, StreamCheckResult, StreamCheckService};

#[derive(Debug, Clone)]
pub struct SidecarCliOptions {
    pub data_dir: PathBuf,
    pub bind_host: String,
    pub proxy_port: u16,
    pub admin_port: u16,
    pub admin_token_env: String,
    pub proxy_token_env: String,
}

impl SidecarCliOptions {
    pub fn from_args<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut data_dir: Option<PathBuf> = None;
        let mut bind_host = "127.0.0.1".to_string();
        let mut proxy_port: u16 = 0;
        let mut admin_port: u16 = 0;
        let mut admin_token_env = "SIDECAR_ADMIN_TOKEN".to_string();
        let mut proxy_token_env = "LOCAL_GATEWAY_TOKEN".to_string();

        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--data-dir" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| "Missing value for --data-dir".to_string())?;
                    data_dir = Some(PathBuf::from(val));
                }
                "--bind-host" | "--bind" => {
                    bind_host = iter
                        .next()
                        .ok_or_else(|| "Missing value for --bind-host".to_string())?;
                }
                "--proxy-port" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| "Missing value for --proxy-port".to_string())?;
                    proxy_port = val
                        .parse::<u16>()
                        .map_err(|e| format!("Invalid --proxy-port '{val}': {e}"))?;
                }
                "--admin-port" => {
                    let val = iter
                        .next()
                        .ok_or_else(|| "Missing value for --admin-port".to_string())?;
                    admin_port = val
                        .parse::<u16>()
                        .map_err(|e| format!("Invalid --admin-port '{val}': {e}"))?;
                }
                "--admin-token-env" => {
                    admin_token_env = iter
                        .next()
                        .ok_or_else(|| "Missing value for --admin-token-env".to_string())?;
                }
                "--proxy-token-env" => {
                    proxy_token_env = iter
                        .next()
                        .ok_or_else(|| "Missing value for --proxy-token-env".to_string())?;
                }
                "--help" | "-h" => {
                    return Err(
                        "Usage: cc-switch-sidecar --data-dir <DIR> [--bind-host 127.0.0.1] [--proxy-port 0] [--admin-port 0] [--admin-token-env SIDECAR_ADMIN_TOKEN] [--proxy-token-env LOCAL_GATEWAY_TOKEN]".to_string(),
                    );
                }
                "--version" | "-V" => {
                    return Err(format!("cc-switch-sidecar {}", env!("CARGO_PKG_VERSION")));
                }
                other => {
                    return Err(format!("Unknown argument: {other}"));
                }
            }
        }

        let data_dir = data_dir
            .ok_or_else(|| "--data-dir <PATH> is required for isolated sidecar mode".to_string())?;

        if bind_host != "127.0.0.1" {
            return Err("--bind-host must be 127.0.0.1".to_string());
        }

        Ok(Self {
            data_dir,
            bind_host,
            proxy_port,
            admin_port,
            admin_token_env,
            proxy_token_env,
        })
    }
}

#[derive(Clone)]
struct AdminState {
    db: Arc<Database>,
    proxy_server: Arc<ProxyServer>,
    admin_token: Arc<String>,
    data_dir: PathBuf,
    proxy_base_url: String,
    admin_base_url: String,
}

fn constant_time_str_eq(a: &str, b: &str) -> bool {
    if a.is_empty() || a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

fn extract_token_from_headers(headers: &HeaderMap, fallback_header: &str) -> Option<String> {
    if let Some(auth_val) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let trimmed = auth_val.trim();
        if let Some(rest) = trimmed
            .strip_prefix("Bearer ")
            .or_else(|| trimmed.strip_prefix("bearer "))
        {
            let token = rest.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    headers
        .get(fallback_header)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Middleware mounted on `ProxyServer` to validate inbound `LOCAL_GATEWAY_TOKEN`
/// before swapping it with the real upstream provider API key.
pub async fn verify_proxy_inbound_token(
    State(state): State<ProxyState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(expected) = state.proxy_auth_token.as_ref() else {
        return next.run(request).await;
    };

    if request.headers().contains_key(header::ORIGIN) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Browser Origin requests are forbidden on the gateway port" })),
        )
            .into_response();
    }

    let provided = extract_token_from_headers(request.headers(), "x-api-key");
    match provided {
        Some(token) if constant_time_str_eq(&token, expected) => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "message": "Invalid or missing local gateway token",
                    "type": "authentication_error",
                }
            })),
        )
            .into_response(),
    }
}

/// Middleware mounted on the Admin HTTP plane:
/// 1. Rejects any browser-originated request (`Origin` header present) with 403;
/// 2. Enforces constant-time Bearer / `X-Admin-Token` authentication.
async fn verify_admin_request(
    State(state): State<AdminState>,
    request: Request,
    next: Next,
) -> Response {
    if request.headers().contains_key(header::ORIGIN) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Browser Origin requests are forbidden on sidecar admin port" })),
        )
            .into_response();
    }

    let provided = extract_token_from_headers(request.headers(), "x-admin-token");
    match provided {
        Some(token) if constant_time_str_eq(&token, &state.admin_token) => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Invalid or missing sidecar admin token" })),
        )
            .into_response(),
    }
}

/// Pure in-memory Codex model catalog builder for headless sidecar `GET /v1/models`.
pub async fn build_sidecar_codex_models_catalog(db: &Arc<Database>) -> Value {
    let db_clone = db.clone();
    let result = tokio::task::spawn_blocking(move || -> Value {
        let app = AppType::Codex;
        let current_id = db_clone.get_current_provider(&app).ok().flatten();
        let Some(provider_id) = current_id else {
            return json!({ "object": "list", "data": [], "models": [] });
        };
        let Some(provider) = db_clone
            .get_provider_by_id(&provider_id, &app)
            .ok()
            .flatten()
        else {
            return json!({ "object": "list", "data": [], "models": [] });
        };

        let config_text = provider
            .settings_config
            .get("config")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let profile = crate::proxy::providers::resolve_codex_catalog_tool_profile(&provider);

        let mut catalog = match crate::codex_config::plan_codex_model_catalog(
            &provider.settings_config,
            &config_text,
            profile,
        ) {
            Ok(plan) => plan.catalog.unwrap_or_else(|| json!({"models": []})),
            Err(_) => json!({ "models": [] }),
        };

        let model_ids = extract_provider_models(&provider);
        let data: Vec<Value> = model_ids
            .iter()
            .map(|id| {
                json!({
                    "id": id,
                    "object": "model",
                    "owned_by": provider.name,
                })
            })
            .collect();

        if let Some(obj) = catalog.as_object_mut() {
            obj.insert("object".to_string(), json!("list"));
            obj.insert("data".to_string(), Value::Array(data));
        }

        catalog
    })
    .await;

    result.unwrap_or_else(|_| json!({ "object": "list", "data": [], "models": [] }))
}

fn parse_app_type(raw: Option<&str>) -> Result<AppType, (StatusCode, Json<Value>)> {
    let raw = raw.unwrap_or("codex").trim();
    if raw.is_empty() {
        return Ok(AppType::Codex);
    }
    raw.parse::<AppType>().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Invalid app type '{raw}': {e}") })),
        )
    })
}

fn normalize_protocol(raw: Option<&str>) -> (&'static str, &'static str) {
    let normalized = raw
        .unwrap_or("openai_responses")
        .trim()
        .to_ascii_lowercase();
    if crate::codex_config::is_anthropic_wire_api(&normalized) {
        ("anthropic", "anthropic")
    } else if crate::codex_config::is_chat_wire_api(&normalized) {
        ("openai_chat", "chat")
    } else {
        ("openai_responses", "responses")
    }
}

fn extract_raw_api_key(app: &AppType, provider: &Provider) -> Option<String> {
    if matches!(app, AppType::Claude) {
        if let Some(env) = provider.settings_config.get("env") {
            for key_name in [
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_API_KEY",
                "OPENROUTER_API_KEY",
                "OPENAI_API_KEY",
            ] {
                if let Some(val) = env
                    .get(key_name)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    return Some(val.to_string());
                }
            }
        }
    }

    if let Some(key) = provider
        .settings_config
        .get("apiKey")
        .or_else(|| provider.settings_config.get("api_key"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(key.to_string());
    }

    if let Some(auth) = provider.settings_config.get("auth") {
        if let Some(key) = crate::codex_config::extract_codex_auth_api_key(auth) {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    if let Some(env) = provider.settings_config.get("env") {
        if let Some(key) = env
            .get("OPENAI_API_KEY")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(key.to_string());
        }
    }

    provider
        .settings_config
        .get("config")
        .and_then(|v| v.as_str())
        .and_then(crate::codex_config::extract_codex_experimental_bearer_token)
        .filter(|s| !s.trim().is_empty())
}

fn extract_provider_base_url(app: &AppType, provider: &Provider) -> String {
    if matches!(app, AppType::Claude) {
        if let Some(url) = provider
            .settings_config
            .get("env")
            .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return url.to_string();
        }
    }

    if let Some(url) = provider
        .settings_config
        .get("base_url")
        .or_else(|| provider.settings_config.get("baseURL"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return url.to_string();
    }

    provider
        .settings_config
        .get("config")
        .and_then(|v| v.as_str())
        .and_then(crate::codex_config::extract_codex_base_url)
        .unwrap_or_default()
}

fn extract_provider_protocol(provider: &Provider) -> String {
    if crate::proxy::providers::codex_provider_uses_anthropic(provider) {
        "anthropic".to_string()
    } else if crate::proxy::providers::codex_provider_uses_chat_completions(provider) {
        "openai_chat".to_string()
    } else if let Some(fmt) = provider
        .meta
        .as_ref()
        .and_then(|m| m.api_format.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        normalize_protocol(Some(fmt)).0.to_string()
    } else {
        "openai_responses".to_string()
    }
}

fn extract_provider_upstream_model(app: &AppType, provider: &Provider) -> Option<String> {
    if matches!(app, AppType::Claude) {
        if let Some(m) = provider
            .settings_config
            .get("env")
            .and_then(|e| e.get("ANTHROPIC_MODEL"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(m.to_string());
        }
    }
    crate::proxy::providers::codex_provider_upstream_model(provider)
}

fn extract_provider_models(provider: &Provider) -> Vec<String> {
    let mut seen = Vec::new();
    if let Some(models) = provider
        .settings_config
        .get("modelCatalog")
        .and_then(|c| c.get("models"))
        .and_then(|m| m.as_array())
    {
        for item in models {
            if let Some(m) = item
                .get("model")
                .or_else(|| item.get("slug"))
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if !seen.iter().any(|existing: &String| existing == m) {
                    seen.push(m.to_string());
                }
            }
        }
    }
    if let Some(upstream) = crate::proxy::providers::codex_provider_upstream_model(provider) {
        if !seen.iter().any(|existing| existing == &upstream) {
            seen.insert(0, upstream);
        }
    }
    seen
}

fn build_codex_toml_config(base_url: &str, wire_api: &str, upstream_model: Option<&str>) -> String {
    let mut doc = toml_edit::DocumentMut::new();
    doc["model_provider"] = toml_edit::value("custom");
    if let Some(model) = upstream_model.map(str::trim).filter(|m| !m.is_empty()) {
        doc["model"] = toml_edit::value(model);
    }
    let mut providers_table = toml_edit::Table::new();
    let mut custom_table = toml_edit::Table::new();
    custom_table["name"] = toml_edit::value("custom");
    custom_table["base_url"] = toml_edit::value(base_url.trim());
    custom_table["wire_api"] = toml_edit::value(wire_api);
    custom_table["requires_openai_auth"] = toml_edit::value(false);
    providers_table.insert("custom", toml_edit::Item::Table(custom_table));
    doc.insert("model_providers", toml_edit::Item::Table(providers_table));
    doc.to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarProviderDto {
    pub id: String,
    pub app: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub has_api_key: bool,
    pub masked_api_key: Option<String>,
    pub upstream_model: Option<String>,
    pub models: Vec<String>,
    pub anthropic_auth_mode: String,
    pub impersonate_claude_code: bool,
    pub max_output_tokens: Option<u64>,
    pub is_full_url: bool,
    pub custom_user_agent: Option<String>,
    pub custom_headers: HashMap<String, String>,
    pub is_current: bool,
    pub in_failover_queue: bool,
    pub sort_index: Option<usize>,
    pub notes: Option<String>,
    pub website_url: Option<String>,
    pub is_healthy: bool,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
}

fn provider_to_dto(
    app: &AppType,
    provider: &Provider,
    current_id: Option<&str>,
    health: Option<&crate::proxy::types::ProviderHealth>,
) -> SidecarProviderDto {
    let has_api_key = extract_raw_api_key(app, provider).is_some();
    let meta = provider.meta.as_ref();
    let anthropic_auth_mode = meta
        .and_then(|m| m.api_key_field.as_deref())
        .map(|f| {
            if f.eq_ignore_ascii_case("ANTHROPIC_API_KEY") {
                "x-api-key"
            } else {
                "bearer"
            }
        })
        .unwrap_or("bearer")
        .to_string();

    let custom_headers = meta
        .and_then(|m| m.local_proxy_request_overrides.as_ref())
        .map(|o| {
            o.headers
                .keys()
                .map(|name| (name.clone(), String::new()))
                .collect()
        })
        .unwrap_or_default();

    SidecarProviderDto {
        id: provider.id.clone(),
        app: app.as_str().to_string(),
        name: provider.name.clone(),
        protocol: extract_provider_protocol(provider),
        base_url: extract_provider_base_url(app, provider),
        has_api_key,
        masked_api_key: has_api_key.then(|| "••••••••".to_owned()),
        upstream_model: extract_provider_upstream_model(app, provider),
        models: extract_provider_models(provider),
        anthropic_auth_mode,
        impersonate_claude_code: meta
            .and_then(|m| m.impersonate_claude_code)
            .unwrap_or(false),
        max_output_tokens: meta.and_then(|m| m.max_output_tokens),
        is_full_url: meta.and_then(|m| m.is_full_url).unwrap_or(false),
        custom_user_agent: meta.and_then(|m| m.custom_user_agent.clone()),
        custom_headers,
        is_current: current_id == Some(provider.id.as_str()),
        in_failover_queue: provider.in_failover_queue,
        sort_index: provider.sort_index,
        notes: provider.notes.clone(),
        website_url: provider.website_url.clone(),
        is_healthy: health.map(|h| h.is_healthy).unwrap_or(true),
        consecutive_failures: health.map(|h| h.consecutive_failures).unwrap_or(0),
        last_error: health.and_then(|h| h.last_error.clone()),
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct UpsertProviderPayload {
    pub id: Option<String>,
    pub app: Option<String>,
    pub name: Option<String>,
    pub protocol: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
    pub upstream_model: Option<String>,
    pub models: Option<Vec<String>>,
    pub anthropic_auth_mode: Option<String>,
    pub impersonate_claude_code: Option<bool>,
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub clear_max_output_tokens: bool,
    pub is_full_url: Option<bool>,
    pub custom_user_agent: Option<String>,
    pub custom_headers: Option<HashMap<String, String>>,
    pub in_failover_queue: Option<bool>,
    pub sort_index: Option<usize>,
    pub notes: Option<String>,
    pub website_url: Option<String>,
    pub activate: Option<bool>,
}

fn slugify_provider_id(name: &str) -> String {
    let base: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let prefix = if base.is_empty() {
        "provider".to_string()
    } else {
        base.chars().take(24).collect()
    };
    let short_uuid = &uuid::Uuid::new_v4().simple().to_string()[..8];
    format!("{prefix}-{short_uuid}")
}

fn apply_upsert_to_provider(
    app: &AppType,
    existing: Option<&Provider>,
    forced_id: Option<&str>,
    payload: &UpsertProviderPayload,
) -> Result<Provider, String> {
    let id = forced_id
        .map(ToString::to_string)
        .or_else(|| existing.map(|p| p.id.clone()))
        .or_else(|| {
            payload
                .id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| slugify_provider_id(payload.name.as_deref().unwrap_or("provider")));

    let name = payload
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| existing.map(|p| p.name.clone()))
        .ok_or_else(|| "Provider name is required".to_string())?;

    let base_url = payload
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| existing.map(|p| extract_provider_base_url(app, p)))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Provider baseUrl is required".to_string())?;

    if !url::Url::parse(&base_url).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    }) {
        return Err("Base URL must be an HTTP or HTTPS URL without embedded credentials".into());
    }

    let existing_protocol = existing.map(extract_provider_protocol);
    let (api_format, wire_api) =
        normalize_protocol(payload.protocol.as_deref().or(existing_protocol.as_deref()));

    let resolved_api_key = if payload.clear_api_key {
        None
    } else if let Some(new_key) = payload
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(new_key.to_string())
    } else {
        existing.and_then(|p| extract_raw_api_key(app, p))
    };

    let upstream_model = match &payload.upstream_model {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None => existing.and_then(|p| extract_provider_upstream_model(app, p)),
    };

    let mut models = match &payload.models {
        Some(list) => {
            let mut dedup = Vec::new();
            for item in list {
                let m = item.trim();
                if !m.is_empty() && !dedup.iter().any(|existing_m: &String| existing_m == m) {
                    dedup.push(m.to_string());
                }
            }
            dedup
        }
        None => existing.map(extract_provider_models).unwrap_or_default(),
    };
    if let Some(ref primary) = upstream_model {
        if !models.iter().any(|m| m == primary) {
            models.insert(0, primary.clone());
        }
    }

    let mut meta = existing.and_then(|p| p.meta.clone()).unwrap_or_default();
    meta.api_format = Some(api_format.to_string());

    if let Some(ref mode) = payload.anthropic_auth_mode {
        if mode.trim().eq_ignore_ascii_case("x-api-key")
            || mode.trim().eq_ignore_ascii_case("ANTHROPIC_API_KEY")
        {
            meta.api_key_field = Some("ANTHROPIC_API_KEY".to_string());
        } else {
            meta.api_key_field = Some("ANTHROPIC_AUTH_TOKEN".to_string());
        }
    } else if meta.api_key_field.is_none() {
        meta.api_key_field = Some("ANTHROPIC_AUTH_TOKEN".to_string());
    }

    if let Some(impersonate) = payload.impersonate_claude_code {
        meta.impersonate_claude_code = Some(impersonate);
    }
    if payload.clear_max_output_tokens {
        meta.max_output_tokens = None;
    } else if let Some(max_tokens) = payload.max_output_tokens {
        meta.max_output_tokens = if max_tokens > 0 {
            Some(max_tokens)
        } else {
            None
        };
    }
    if let Some(is_full) = payload.is_full_url {
        meta.is_full_url = Some(is_full);
    }
    if let Some(ref ua) = payload.custom_user_agent {
        let trimmed = ua.trim();
        meta.custom_user_agent = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    if let Some(ref headers) = payload.custom_headers {
        let cleaned: HashMap<String, String> = headers
            .iter()
            .filter_map(|(k, v)| {
                let kt = k.trim();
                let vt = v.trim();
                if kt.is_empty() {
                    None
                } else {
                    Some((kt.to_string(), vt.to_string()))
                }
            })
            .collect();
        let mut overrides = meta.local_proxy_request_overrides.unwrap_or_default();
        overrides.headers = cleaned;
        meta.local_proxy_request_overrides = if overrides.is_empty() {
            None
        } else {
            Some(LocalProxyRequestOverrides {
                headers: overrides.headers,
                body: overrides.body,
            })
        };
    }

    let catalog_entries: Vec<Value> = models.iter().map(|m| json!({ "model": m })).collect();
    let api_key_str = resolved_api_key.clone().unwrap_or_default();

    let settings_config = if matches!(app, AppType::Claude) {
        let key_field = meta
            .api_key_field
            .as_deref()
            .unwrap_or("ANTHROPIC_AUTH_TOKEN");
        let mut env_obj = serde_json::Map::new();
        env_obj.insert("ANTHROPIC_BASE_URL".to_string(), json!(base_url));
        if !api_key_str.is_empty() {
            env_obj.insert(key_field.to_string(), json!(api_key_str));
        }
        if let Some(ref m) = upstream_model {
            env_obj.insert("ANTHROPIC_MODEL".to_string(), json!(m));
        }
        json!({
            "env": Value::Object(env_obj),
            "apiFormat": api_format,
            "modelCatalog": { "models": catalog_entries },
        })
    } else {
        let toml_cfg = build_codex_toml_config(&base_url, wire_api, upstream_model.as_deref());
        let mut obj = serde_json::Map::new();
        obj.insert("base_url".to_string(), json!(base_url));
        obj.insert("api_format".to_string(), json!(api_format));
        obj.insert("apiFormat".to_string(), json!(api_format));
        obj.insert("config".to_string(), json!(toml_cfg));
        obj.insert(
            "modelCatalog".to_string(),
            json!({ "models": catalog_entries }),
        );
        if let Some(ref m) = upstream_model {
            obj.insert("model".to_string(), json!(m));
        }
        if !api_key_str.is_empty() {
            obj.insert("apiKey".to_string(), json!(api_key_str.clone()));
            obj.insert(
                "auth".to_string(),
                json!({ "OPENAI_API_KEY": api_key_str.clone() }),
            );
            obj.insert("env".to_string(), json!({ "OPENAI_API_KEY": api_key_str }));
        } else {
            obj.insert("auth".to_string(), json!({}));
        }
        Value::Object(obj)
    };

    let now_ms = chrono::Utc::now().timestamp_millis();
    Ok(Provider {
        id,
        name,
        settings_config,
        website_url: payload
            .website_url
            .clone()
            .or_else(|| existing.and_then(|p| p.website_url.clone())),
        category: Some("custom".to_string()),
        created_at: existing.and_then(|p| p.created_at).or(Some(now_ms)),
        sort_index: payload
            .sort_index
            .or_else(|| existing.and_then(|p| p.sort_index)),
        notes: payload
            .notes
            .clone()
            .or_else(|| existing.and_then(|p| p.notes.clone())),
        meta: Some(meta),
        icon: existing.and_then(|p| p.icon.clone()),
        icon_color: existing.and_then(|p| p.icon_color.clone()),
        in_failover_queue: payload
            .in_failover_queue
            .unwrap_or_else(|| existing.map(|p| p.in_failover_queue).unwrap_or(true)),
    })
}

fn internal_err(err: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": err.to_string() })),
    )
}

#[derive(Debug, Deserialize, Default)]
struct AppQuery {
    app: Option<String>,
}

async fn admin_health(
    State(state): State<AdminState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let proxy_status: ProxyStatus = state.proxy_server.get_status().await;
    let db = state.db.clone();
    let active_codex = tokio::task::spawn_blocking(move || -> Option<SidecarProviderDto> {
        let app = AppType::Codex;
        let current_id = db.get_current_provider(&app).ok().flatten()?;
        let provider = db.get_provider_by_id(&current_id, &app).ok().flatten()?;
        Some(provider_to_dto(&app, &provider, Some(&current_id), None))
    })
    .await
    .map_err(internal_err)?;

    Ok(Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "dataDir": state.data_dir,
        "proxyBaseUrl": state.proxy_base_url,
        "adminBaseUrl": state.admin_base_url,
        "proxyStatus": proxy_status,
        "activeCodexProvider": active_codex,
    })))
}

async fn admin_list_providers(
    State(state): State<AdminState>,
    Query(query): Query<AppQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let app = parse_app_type(query.app.as_deref())?;
    let db = state.db.clone();
    let app_clone = app.clone();

    let (rows, current_id, enabled) = tokio::task::spawn_blocking(move || {
        db.with_sidecar_route(|| {
            let rows = db.get_provider_rows(&app_clone)?;
            let current_id = db.get_current_provider(&app_clone)?;
            Ok((
                rows.readable,
                current_id,
                db.get_proxy_flags_sync(&app_clone).1,
            ))
        })
    })
    .await
    .map_err(internal_err)?
    .map_err(internal_err)?;

    let mut items = Vec::with_capacity(rows.len());
    for (_, provider) in rows {
        let health = state.db.get_provider_health(&provider.id, &app).await.ok();
        items.push(provider_to_dto(
            &app,
            &provider,
            current_id.as_deref(),
            health.as_ref(),
        ));
    }

    Ok(Json(json!({
        "app": app.as_str(),
        "currentProviderId": current_id,
        "autoFailoverEnabled": enabled,
        "providers": items,
    })))
}

async fn admin_create_provider(
    State(state): State<AdminState>,
    Json(payload): Json<UpsertProviderPayload>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let app = parse_app_type(payload.app.as_deref())?;
    let provider = apply_upsert_to_provider(&app, None, None, &payload)
        .map_err(|msg| (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))))?;

    let db = state.db.clone();
    let app_clone = app.clone();
    let provider_clone = provider.clone();
    let explicit_activate = payload.activate;

    let (current_id, provider) = tokio::task::spawn_blocking(move || {
        db.with_sidecar_route(|| {
            let existing_current = db.get_current_provider(&app_clone)?;
            db.save_provider(&app_clone, &provider_clone)?;
            let should_activate = explicit_activate.unwrap_or(existing_current.is_none());
            let change = if should_activate {
                routing::Change::Activate {
                    id: &provider_clone.id,
                    queue: None,
                }
            } else {
                routing::Change::Configure {
                    enabled: None,
                    queue: None,
                }
            };
            let (current, _) = routing::apply(&db, &app_clone, change)?;
            let saved = db
                .get_provider_by_id(&provider_clone.id, &app_clone)?
                .ok_or_else(|| AppError::Database("The new provider is missing".into()))?;
            Ok((current, saved))
        })
    })
    .await
    .map_err(internal_err)?
    .map_err(internal_err)?;

    state
        .proxy_server
        .state()
        .provider_router
        .forget(&app, Some(&provider.id))
        .await;

    let dto = provider_to_dto(&app, &provider, current_id.as_deref(), None);
    Ok((
        StatusCode::CREATED,
        Json(json!({ "provider": dto, "currentProviderId": current_id })),
    ))
}

async fn admin_update_provider(
    State(state): State<AdminState>,
    AxumPath((app_raw, id)): AxumPath<(String, String)>,
    Json(payload): Json<UpsertProviderPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let app = parse_app_type(Some(&app_raw))?;
    let db = state.db.clone();
    let app_clone = app.clone();
    let id_clone = id.clone();

    let (current_id, updated) = tokio::task::spawn_blocking(move || {
        db.with_sidecar_route(|| {
            let Some(existing) = db.get_provider_by_id(&id_clone, &app_clone)? else {
                return Ok(None);
            };
            let updated =
                apply_upsert_to_provider(&app_clone, Some(&existing), Some(&id_clone), &payload)
                    .map_err(AppError::InvalidInput)?;
            db.save_provider(&app_clone, &updated)?;
            if let Some(in_queue) = payload.in_failover_queue {
                if in_queue {
                    db.add_to_failover_queue(&app_clone, &id_clone)?;
                } else {
                    db.remove_from_failover_queue(&app_clone, &id_clone)?;
                }
            }
            let change = if payload.activate == Some(true) {
                routing::Change::Activate {
                    id: &id_clone,
                    queue: None,
                }
            } else {
                routing::Change::Configure {
                    enabled: None,
                    queue: None,
                }
            };
            let (current, _) = routing::apply(&db, &app_clone, change)?;
            let saved = db
                .get_provider_by_id(&id_clone, &app_clone)?
                .ok_or_else(|| AppError::Database("The updated provider is missing".into()))?;
            Ok(Some((current, saved)))
        })
    })
    .await
    .map_err(internal_err)?
    .map_err(route_error)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(
                json!({ "error": format!("Provider '{id}' not found in app '{}'", app.as_str()) }),
            ),
        )
    })?;

    let _ = state
        .db
        .update_provider_health_with_threshold(&id, &app, true, None, 4)
        .await;
    state
        .proxy_server
        .state()
        .provider_router
        .forget(&app, Some(&id))
        .await;

    let health = state.db.get_provider_health(&id, &app).await.ok();
    let dto = provider_to_dto(&app, &updated, current_id.as_deref(), health.as_ref());
    Ok(Json(
        json!({ "provider": dto, "currentProviderId": current_id }),
    ))
}

async fn admin_delete_provider(
    State(state): State<AdminState>,
    AxumPath((app_raw, id)): AxumPath<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let app = parse_app_type(Some(&app_raw))?;
    let db = state.db.clone();
    let app_clone = app.clone();
    let id_clone = id.clone();

    let (next_current, _) = tokio::task::spawn_blocking(move || {
        db.with_sidecar_route(|| {
            routing::apply(&db, &app_clone, routing::Change::Delete(&id_clone))
        })
    })
    .await
    .map_err(internal_err)?
    .map_err(internal_err)?;

    state
        .proxy_server
        .state()
        .provider_router
        .forget(&app, Some(&id))
        .await;

    Ok(Json(json!({
        "deleted": id,
        "currentProviderId": next_current,
    })))
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ActivateProviderPayload {
    pub fallback_ids: Option<Vec<String>>,
}

async fn admin_activate_provider(
    State(state): State<AdminState>,
    AxumPath((app_raw, id)): AxumPath<(String, String)>,
    payload: Option<Json<ActivateProviderPayload>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let app = parse_app_type(Some(&app_raw))?;
    let fallback_ids = payload.and_then(|Json(p)| p.fallback_ids);

    let db = state.db.clone();
    let app_clone = app.clone();
    let id_clone = id.clone();

    let provider = tokio::task::spawn_blocking(move || {
        db.with_sidecar_route(|| {
            if db.get_provider_by_id(&id_clone, &app_clone)?.is_none() {
                return Ok(None);
            }
            routing::apply(
                &db,
                &app_clone,
                routing::Change::Activate {
                    id: &id_clone,
                    queue: fallback_ids,
                },
            )?;
            db.get_provider_by_id(&id_clone, &app_clone)
        })
    })
    .await
    .map_err(internal_err)?
    .map_err(route_error)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Provider '{id}' not found") })),
        )
    })?;

    state
        .proxy_server
        .state()
        .current_providers
        .write()
        .await
        .insert(
            app.as_str().to_string(),
            (provider.id.clone(), provider.name.clone()),
        );

    Ok(Json(json!({
        "currentProviderId": provider.id,
        "provider": provider_to_dto(&app, &provider, Some(&provider.id), None),
    })))
}

async fn admin_probe_provider(
    State(state): State<AdminState>,
    AxumPath((app_raw, id)): AxumPath<(String, String)>,
) -> Result<Json<StreamCheckResult>, (StatusCode, Json<Value>)> {
    let app = parse_app_type(Some(&app_raw))?;
    let db = state.db.clone();
    let app_clone = app.clone();
    let id_clone = id.clone();

    let provider =
        tokio::task::spawn_blocking(move || db.get_provider_by_id(&id_clone, &app_clone))
            .await
            .map_err(internal_err)?
            .map_err(internal_err)?
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": format!("Provider '{id}' not found") })),
                )
            })?;

    let result =
        StreamCheckService::check_with_retry(&app, &provider, &StreamCheckConfig::default(), None)
            .await
            .map_err(internal_err)?;

    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchModelsPayload {
    pub app: Option<String>,
    pub provider_id: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub protocol: Option<String>,
    pub anthropic_auth_mode: Option<String>,
    pub is_full_url: Option<bool>,
    pub custom_headers: Option<BTreeMap<String, String>>,
}

fn model_discovery_format(protocol: &str, anthropic_auth_mode: Option<&str>) -> &'static str {
    match normalize_protocol(Some(protocol)).0 {
        "anthropic"
            if anthropic_auth_mode.is_some_and(|mode| {
                mode.eq_ignore_ascii_case("bearer")
                    || mode.eq_ignore_ascii_case("ANTHROPIC_AUTH_TOKEN")
            }) =>
        {
            "anthropic-messages-bearer"
        }
        "anthropic" => "anthropic-messages",
        protocol => protocol,
    }
}

async fn admin_fetch_models(
    State(state): State<AdminState>,
    Json(payload): Json<FetchModelsPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let app = parse_app_type(payload.app.as_deref())?;

    let (base_url, api_key, is_full_url, user_agent, api_format, headers) =
        if let Some(provider_id) = payload
            .provider_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let db = state.db.clone();
            let app_clone = app.clone();
            let pid = provider_id.to_string();
            let provider =
                tokio::task::spawn_blocking(move || db.get_provider_by_id(&pid, &app_clone))
                    .await
                    .map_err(internal_err)?
                    .map_err(internal_err)?
                    .ok_or_else(|| {
                        (
                            StatusCode::NOT_FOUND,
                            Json(json!({ "error": format!("Provider '{provider_id}' not found") })),
                        )
                    })?;

            let resolved_url = payload
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| extract_provider_base_url(&app, &provider));
            let resolved_key = payload
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .or_else(|| extract_raw_api_key(&app, &provider))
                .unwrap_or_default();
            let full = payload.is_full_url.unwrap_or_else(|| {
                provider
                    .meta
                    .as_ref()
                    .and_then(|m| m.is_full_url)
                    .unwrap_or(false)
            });
            let ua = provider
                .meta
                .as_ref()
                .and_then(|m| m.custom_user_agent.as_deref())
                .and_then(|s| HeaderValue::from_str(s).ok());
            let protocol = payload
                .protocol
                .clone()
                .unwrap_or_else(|| extract_provider_protocol(&provider));
            let auth_mode = payload.anthropic_auth_mode.as_deref().or_else(|| {
                provider
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.api_key_field.as_deref())
            });
            let fmt = model_discovery_format(&protocol, auth_mode.or(Some("bearer")));
            let hdrs = payload.custom_headers.or_else(|| {
                provider
                    .meta
                    .as_ref()
                    .and_then(|m| m.local_proxy_request_overrides.as_ref())
                    .map(|o| {
                        o.headers
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect::<BTreeMap<_, _>>()
                    })
            });
            (resolved_url, resolved_key, full, ua, fmt, hdrs)
        } else {
            let url = payload
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "Either providerId or baseUrl is required" })),
                    )
                })?
                .to_string();
            let key = payload.api_key.unwrap_or_default();
            let fmt = model_discovery_format(
                payload.protocol.as_deref().unwrap_or("openai_responses"),
                payload.anthropic_auth_mode.as_deref(),
            );
            (
                url,
                key,
                payload.is_full_url.unwrap_or(false),
                None,
                fmt,
                payload.custom_headers,
            )
        };

    let models: Vec<FetchedModel> = model_fetch::fetch_models(
        &base_url,
        &api_key,
        is_full_url,
        None,
        user_agent,
        Some(api_format),
        headers.as_ref(),
    )
    .await
    .map_err(|msg| (StatusCode::BAD_GATEWAY, Json(json!({ "error": msg }))))?;

    Ok(Json(json!({ "models": models })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProxySettingsPayload {
    pub app_proxy_config: Option<AppProxyConfig>,
    pub rectifier_config: Option<RectifierConfig>,
    pub global_proxy_url: Option<String>,
    #[serde(default)]
    pub clear_global_proxy_url: bool,
}

async fn admin_get_proxy_settings(
    State(state): State<AdminState>,
    AxumPath(app_raw): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let app = parse_app_type(Some(&app_raw))?;
    let db = state.db.clone();
    let (app_config, rectifier, global_proxy_url, failover_queue) =
        tokio::task::spawn_blocking(move || {
            db.with_sidecar_route(|| {
                Ok((
                    db.get_proxy_config_for_app_sync(&app)?,
                    db.get_rectifier_config().unwrap_or_default(),
                    db.get_global_proxy_url()?,
                    db.get_failover_queue(&app)?,
                ))
            })
        })
        .await
        .map_err(internal_err)?
        .map_err(internal_err)?;

    Ok(Json(json!({
        "appProxyConfig": app_config,
        "rectifierConfig": rectifier,
        "hasGlobalProxyUrl": global_proxy_url.is_some(),
        "failoverQueue": failover_queue,
    })))
}

async fn admin_put_proxy_settings(
    State(state): State<AdminState>,
    AxumPath(app_raw): AxumPath<String>,
    Json(payload): Json<UpdateProxySettingsPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let app = parse_app_type(Some(&app_raw))?;
    if let Some(url) = payload.global_proxy_url.as_deref() {
        crate::proxy::http_client::validate_proxy(Some(url.trim())).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Invalid upstream proxy URL" })),
            )
        })?;
    }

    if let Some(mut cfg) = payload.app_proxy_config {
        cfg.app_type = app.as_str().to_string();
        cfg.enabled = true;
        let cb_cfg = CircuitBreakerConfig {
            failure_threshold: cfg.circuit_failure_threshold,
            success_threshold: cfg.circuit_success_threshold,
            timeout_seconds: cfg.circuit_timeout_seconds as u64,
            error_rate_threshold: cfg.circuit_error_rate_threshold,
            min_requests: cfg.circuit_min_requests,
        };
        let db = state.db.clone();
        let app_clone = app.clone();
        tokio::task::spawn_blocking(move || {
            db.with_sidecar_route(|| {
                routing::apply(&db, &app_clone, routing::Change::Tune(&cfg))?;
                Ok(())
            })
        })
        .await
        .map_err(internal_err)?
        .map_err(route_error)?;
        state
            .proxy_server
            .update_circuit_breaker_config_for_app(&app, cb_cfg)
            .await;
    }

    if let Some(rectifier) = payload.rectifier_config {
        let db = state.db.clone();
        tokio::task::spawn_blocking(move || db.set_rectifier_config(&rectifier))
            .await
            .map_err(internal_err)?
            .map_err(internal_err)?;
    }

    if payload.clear_global_proxy_url {
        let db = state.db.clone();
        tokio::task::spawn_blocking(move || db.set_global_proxy_url(None))
            .await
            .map_err(internal_err)?
            .map_err(internal_err)?;
        let _ = crate::proxy::http_client::apply_proxy(None);
    } else if let Some(ref proxy_url) = payload.global_proxy_url {
        let trimmed = proxy_url.trim();
        let opt = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        };
        let db = state.db.clone();
        let opt_owned = opt.map(ToString::to_string);
        tokio::task::spawn_blocking(move || db.set_global_proxy_url(opt_owned.as_deref()))
            .await
            .map_err(internal_err)?
            .map_err(internal_err)?;
        let _ = crate::proxy::http_client::apply_proxy(opt);
    }

    admin_get_proxy_settings(State(state), AxumPath(app_raw)).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateFailoverPayload {
    pub auto_failover_enabled: Option<bool>,
    pub queue_provider_ids: Option<Vec<String>>,
}

fn route_error(error: AppError) -> (StatusCode, Json<Value>) {
    match error {
        AppError::InvalidInput(message) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": message })))
        }
        other => internal_err(other),
    }
}

async fn admin_put_failover(
    State(state): State<AdminState>,
    AxumPath(app_raw): AxumPath<String>,
    Json(payload): Json<UpdateFailoverPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let app = parse_app_type(Some(&app_raw))?;
    let db = state.db.clone();
    let (enabled, queue) = tokio::task::spawn_blocking(move || {
        db.with_sidecar_route(|| {
            let (_, enabled) = routing::apply(
                &db,
                &app,
                routing::Change::Configure {
                    enabled: payload.auto_failover_enabled,
                    queue: payload.queue_provider_ids,
                },
            )?;
            Ok((enabled, db.get_failover_queue(&app)?))
        })
    })
    .await
    .map_err(internal_err)?
    .map_err(route_error)?;

    Ok(Json(json!({
        "autoFailoverEnabled": enabled,
        "failoverQueue": queue,
    })))
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ResetBreakerPayload {
    pub app: Option<String>,
    pub provider_id: Option<String>,
}

async fn admin_reset_circuit_breakers(
    State(state): State<AdminState>,
    payload: Option<Json<ResetBreakerPayload>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let body = payload.map(|Json(p)| p).unwrap_or_default();
    let app = parse_app_type(body.app.as_deref())?;
    if let Some(ref pid) = body.provider_id {
        state
            .db
            .update_provider_health_with_threshold(pid, &app, true, None, 4)
            .await
            .map_err(internal_err)?;
        state
            .proxy_server
            .state()
            .provider_router
            .forget(&app, Some(pid))
            .await;
    } else {
        state
            .db
            .clear_provider_health_for_app(&app)
            .await
            .map_err(internal_err)?;
        state
            .proxy_server
            .state()
            .provider_router
            .forget(&app, None)
            .await;
    }

    Ok(Json(json!({ "reset": true })))
}

#[derive(Debug, Deserialize, Default)]
struct UsageQuery {
    limit: Option<usize>,
}

async fn admin_usage_summary(
    State(state): State<AdminState>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let limit = query.limit.unwrap_or(40).min(200);
    let proxy_status = state.proxy_server.get_status().await;
    let db = state.db.clone();

    let (totals, recent_logs) = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
        let conn = lock_conn!(db.conn);
        let totals = conn
            .query_row(
                "SELECT
                    COUNT(*),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_read_tokens), 0),
                    COALESCE(SUM(cache_creation_tokens), 0)
                 FROM proxy_request_logs",
                [],
                |row| {
                    Ok(json!({
                        "loggedRequests": row.get::<_, i64>(0)?,
                        "inputTokens": row.get::<_, i64>(1)?,
                        "outputTokens": row.get::<_, i64>(2)?,
                        "cacheReadTokens": row.get::<_, i64>(3)?,
                        "cacheCreationTokens": row.get::<_, i64>(4)?,
                    }))
                },
            )
            .unwrap_or_else(|_| {
                json!({
                    "loggedRequests": 0,
                    "inputTokens": 0,
                    "outputTokens": 0,
                    "cacheReadTokens": 0,
                    "cacheCreationTokens": 0,
                })
            });

        let mut stmt = conn
            .prepare(
                "SELECT
                    l.request_id,
                    l.provider_id,
                    p.name,
                    l.app_type,
                    l.model,
                    l.request_model,
                    l.input_tokens,
                    l.output_tokens,
                    l.cache_read_tokens,
                    l.total_cost_usd,
                    l.latency_ms,
                    l.first_token_ms,
                    l.status_code,
                    l.error_message,
                    l.is_streaming,
                    l.created_at
                 FROM proxy_request_logs l
                 LEFT JOIN providers p ON p.id = l.provider_id AND p.app_type = l.app_type
                 ORDER BY l.created_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(json!({
                    "requestId": row.get::<_, String>(0)?,
                    "providerId": row.get::<_, String>(1)?,
                    "providerName": row.get::<_, Option<String>>(2)?,
                    "appType": row.get::<_, String>(3)?,
                    "model": row.get::<_, String>(4)?,
                    "requestModel": row.get::<_, Option<String>>(5)?,
                    "inputTokens": row.get::<_, i64>(6)?,
                    "outputTokens": row.get::<_, i64>(7)?,
                    "cacheReadTokens": row.get::<_, i64>(8)?,
                    "totalCostUsd": row.get::<_, String>(9)?,
                    "latencyMs": row.get::<_, i64>(10)?,
                    "firstTokenMs": row.get::<_, Option<i64>>(11)?,
                    "statusCode": row.get::<_, i64>(12)?,
                    "errorMessage": row.get::<_, Option<String>>(13)?,
                    "isStreaming": row.get::<_, i64>(14)? != 0,
                    "createdAt": row.get::<_, i64>(15)?,
                }))
            })
            .map_err(|e| AppError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok((totals, rows))
    })
    .await
    .map_err(internal_err)?
    .map_err(internal_err)?;

    Ok(Json(json!({
        "proxyStatus": proxy_status,
        "totals": totals,
        "recentLogs": recent_logs,
    })))
}

fn build_admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/admin/v1/health", get(admin_health))
        .route(
            "/admin/v1/providers",
            get(admin_list_providers).post(admin_create_provider),
        )
        .route(
            "/admin/v1/providers/:app/:id",
            patch(admin_update_provider)
                .put(admin_update_provider)
                .delete(admin_delete_provider),
        )
        .route(
            "/admin/v1/providers/:app/:id/activate",
            post(admin_activate_provider),
        )
        .route(
            "/admin/v1/providers/:app/:id/probe",
            post(admin_probe_provider),
        )
        .route("/admin/v1/models/fetch", post(admin_fetch_models))
        .route(
            "/admin/v1/proxy-config/:app",
            get(admin_get_proxy_settings).put(admin_put_proxy_settings),
        )
        .route("/admin/v1/failover/:app", put(admin_put_failover))
        .route(
            "/admin/v1/circuit-breakers/reset",
            post(admin_reset_circuit_breakers),
        )
        .route("/admin/v1/usage/summary", get(admin_usage_summary))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            verify_admin_request,
        ))
        .with_state(state)
}

fn ensure_sidecar_proxy_rows_enabled(db: &Database, bind_host: &str) -> Result<(), AppError> {
    for app in [AppType::Codex, AppType::Claude, AppType::Gemini] {
        let (enabled, auto_failover) = db.get_proxy_flags_sync(&app);
        if !enabled {
            let _ = db.set_proxy_flags_sync(&app, true, auto_failover);
        }
    }
    let conn = lock_conn!(db.conn);
    let _ = conn.execute(
        "UPDATE proxy_config SET proxy_enabled = 1, listen_address = ?1, enable_logging = 1",
        rusqlite::params![bind_host],
    );
    Ok(())
}

pub async fn run_sidecar(options: SidecarCliOptions) -> Result<(), String> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let admin_token = std::env::var(&options.admin_token_env)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "Required admin token environment variable '{}' is missing or empty",
                options.admin_token_env
            )
        })?;

    let proxy_token = std::env::var(&options.proxy_token_env)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "Required proxy token environment variable '{}' is missing or empty",
                options.proxy_token_env
            )
        })?;

    if constant_time_str_eq(&admin_token, &proxy_token) {
        return Err("Admin and proxy tokens must differ".to_string());
    }

    let canonical_data_dir = crate::config::init_sidecar_isolation(&options.data_dir)
        .map_err(|e| format!("Failed to initialize sidecar isolation directory: {e}"))?;

    let db = Arc::new(
        Database::open_in_dir(&canonical_data_dir)
            .map_err(|e| format!("Failed to initialize isolated SQLite database: {e}"))?,
    );
    ensure_sidecar_proxy_rows_enabled(&db, &options.bind_host)
        .map_err(|e| format!("Failed to configure sidecar proxy rows: {e}"))?;
    routing::reconcile_existing_queues(&db)
        .map_err(|e| format!("Failed to reconcile the failover queue: {e}"))?;

    let proxy_url = db
        .get_global_proxy_url()
        .map_err(|error| error.to_string())?;
    crate::proxy::http_client::init(proxy_url.as_deref())?;

    let proxy_config = ProxyConfig {
        listen_address: options.bind_host.clone(),
        listen_port: options.proxy_port,
        enable_logging: true,
    };

    let proxy_server = Arc::new(
        ProxyServer::new(proxy_config, db.clone(), None).with_proxy_auth_token(Some(proxy_token)),
    );
    let proxy_info = proxy_server
        .start()
        .await
        .map_err(|e| format!("Failed to start proxy listener: {e}"))?;

    let admin_bind = format!("{}:{}", options.bind_host, options.admin_port);
    let admin_listener = tokio::net::TcpListener::bind(&admin_bind)
        .await
        .map_err(|e| format!("Failed to bind admin listener on {admin_bind}: {e}"))?;
    let admin_local_addr = admin_listener
        .local_addr()
        .map_err(|e| format!("Failed to query admin local_addr: {e}"))?;

    let host_for_url = if options.bind_host.contains(':') && !options.bind_host.starts_with('[') {
        format!("[{}]", options.bind_host)
    } else {
        options.bind_host.clone()
    };
    let proxy_base_url = format!("http://{}:{}", host_for_url, proxy_info.port);
    let admin_base_url = format!("http://{}:{}", host_for_url, admin_local_addr.port());

    let admin_state = AdminState {
        db: db.clone(),
        proxy_server: proxy_server.clone(),
        admin_token: Arc::new(admin_token),
        data_dir: canonical_data_dir.clone(),
        proxy_base_url: proxy_base_url.clone(),
        admin_base_url: admin_base_url.clone(),
    };
    let admin_router = build_admin_router(admin_state);

    let (admin_shutdown_tx, admin_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let admin_handle = tokio::spawn(async move {
        let _ = axum::serve(admin_listener, admin_router)
            .with_graceful_shutdown(async move {
                let _ = admin_shutdown_rx.await;
            })
            .await;
    });

    // Emit single-line ready JSON handshake on stdout and flush immediately.
    let ready_payload = json!({
        "event": "ready",
        "pid": std::process::id(),
        "proxyPort": proxy_info.port,
        "adminPort": admin_local_addr.port(),
        "proxyBaseUrl": proxy_base_url,
        "adminBaseUrl": admin_base_url,
        "dataDir": canonical_data_dir,
        "version": format!("sidecar-{}", env!("CARGO_PKG_VERSION")),
    });
    {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{ready_payload}");
        let _ = stdout.flush();
    }

    // Watch parent stdin EOF so if parent dies or closes pipe, sidecar exits cleanly.
    let (stdin_eof_tx, stdin_eof_rx) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match stdin.lock().read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = stdin_eof_tx.send(());
                    break;
                }
                Ok(_) => {
                    if line.trim().eq_ignore_ascii_case("shutdown") {
                        let _ = stdin_eof_tx.send(());
                        break;
                    }
                }
            }
        }
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            eprintln!("[cc-switch-sidecar] Received Ctrl+C, shutting down...");
        }
        _ = stdin_eof_rx => {
            eprintln!("[cc-switch-sidecar] Parent stdin closed/shutdown received, shutting down...");
        }
    }

    let _ = admin_shutdown_tx.send(());
    let _ = admin_handle.await;
    let _ = proxy_server.stop().await;
    let _ = db.checkpoint_wal();

    Ok(())
}

pub fn main_entry(_path_check: &Path) -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match SidecarCliOptions::from_args(args) {
        Ok(opts) => opts,
        Err(msg) => {
            if msg.starts_with("Usage:") || msg.starts_with("cc-switch-sidecar ") {
                println!("{msg}");
                return 0;
            }
            eprintln!("[cc-switch-sidecar] {msg}");
            return 2;
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[cc-switch-sidecar] Failed to create Tokio runtime: {e}");
            return 1;
        }
    };

    match runtime.block_on(run_sidecar(options)) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("[cc-switch-sidecar] Fatal error: {err}");
            1
        }
    }
}
