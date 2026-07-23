use std::sync::Arc;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    routing::{any, get},
};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower_http::{
    catch_panic::CatchPanicLayer,
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};

use crate::{
    Settings,
    security::{CredentialCipher, SessionManager},
    storage::StorageRegistry,
};

#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
    pub db: PgPool,
    pub sessions: Arc<SessionManager>,
    pub credential_cipher: Arc<CredentialCipher>,
    pub http_client: reqwest::Client,
    pub storage: Arc<StorageRegistry>,
    pub redis: Option<redis::Client>,
    pub rate_limiter: crate::rate_limit::RateLimiter,
    pub maintenance_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AppState {
    pub async fn initialize(mut settings: Settings, db: PgPool) -> anyhow::Result<Self> {
        apply_persisted_storage_settings(&db, &mut settings).await?;
        if settings.storage_driver == crate::config::StorageDriver::Local {
            tokio::fs::create_dir_all(&settings.storage_local_path)
                .await
                .with_context(|| {
                    format!(
                        "failed to create storage directory {}",
                        settings.storage_local_path.display()
                    )
                })?;
        }
        crate::users::bootstrap_admin(&db, &settings).await?;
        let sessions = SessionManager::new(&settings);
        let credential_cipher = CredentialCipher::new(&settings.credential_master_key)?;
        let mut http_client_builder = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(
                settings.connect_timeout_seconds,
            ))
            .timeout(std::time::Duration::from_secs(
                settings.request_timeout_seconds,
            ))
            .redirect(reqwest::redirect::Policy::none());
        if let Some(path) = &settings.http_ca_cert_file {
            let pem = tokio::fs::read(path).await.with_context(|| {
                format!("failed to read HTTP CA certificate {}", path.display())
            })?;
            let certificate = reqwest::Certificate::from_pem(&pem)
                .context("HTTP_CA_CERT_FILE must contain one PEM certificate")?;
            http_client_builder = http_client_builder.add_root_certificate(certificate);
        }
        let http_client = http_client_builder
            .build()
            .context("failed to build provider HTTP client")?;
        let storage = StorageRegistry::from_settings(&settings).await?;
        let rate_limiter = crate::rate_limit::RateLimiter::new(&settings);
        let redis = settings
            .redis_url
            .as_ref()
            .map(|url| redis::Client::open(url.expose_secret()))
            .transpose()
            .context("failed to configure Redis client")?;

        Ok(Self {
            settings: Arc::new(settings),
            db,
            sessions: Arc::new(sessions),
            credential_cipher: Arc::new(credential_cipher),
            http_client,
            storage: Arc::new(storage),
            redis,
            rate_limiter,
            maintenance_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }
}

async fn apply_persisted_storage_settings(
    db: &PgPool,
    settings: &mut Settings,
) -> anyhow::Result<()> {
    let Some(value) = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT value_json FROM system_settings WHERE setting_key = 'storage.target'",
    )
    .fetch_optional(db)
    .await?
    else {
        return Ok(());
    };
    let object = value
        .as_object()
        .context("system setting storage.target must be a JSON object")?;
    match object.get("driver").and_then(serde_json::Value::as_str) {
        Some("local") => settings.storage_driver = crate::config::StorageDriver::Local,
        Some("s3") => {
            settings.storage_driver = crate::config::StorageDriver::S3;
            settings.storage_s3_enabled = true;
        }
        _ => anyhow::bail!("storage.target.driver must be local or s3"),
    }
    if let Some(value) = object.get("localPath").and_then(serde_json::Value::as_str) {
        settings.storage_local_path = value.into();
    }
    if let Some(value) = object.get("s3Bucket").and_then(serde_json::Value::as_str) {
        settings.storage_s3_bucket = Some(value.to_owned());
    }
    if let Some(value) = object.get("s3Region").and_then(serde_json::Value::as_str) {
        settings.storage_s3_region = value.to_owned();
    }
    settings.storage_s3_endpoint = object
        .get("s3Endpoint")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(value) = object.get("s3Prefix").and_then(serde_json::Value::as_str) {
        settings.storage_s3_prefix = value.to_owned();
    }
    if let Some(value) = object
        .get("s3ForcePathStyle")
        .and_then(serde_json::Value::as_bool)
    {
        settings.storage_s3_force_path_style = value;
    }
    Ok(())
}

pub fn build_router(state: AppState) -> Router {
    let upload_limit = state.settings.max_upload_size_mb * 1024 * 1024;
    let static_dir = state.settings.static_dir.clone();
    let static_service =
        ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html")));
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/ready", get(ready))
        .route("/api/v1/config", get(public_config))
        .merge(crate::auth::routes())
        .merge(crate::users::routes())
        .merge(crate::providers::routes())
        .merge(crate::models::routes())
        .merge(crate::conversations::routes())
        .merge(crate::tasks::routes())
        .merge(crate::images::routes())
        .merge(crate::history::routes())
        .merge(crate::prompt_templates::routes())
        .merge(crate::admin::routes())
        .merge(crate::consistency::routes())
        .merge(crate::analytics::routes())
        .merge(crate::updates::routes())
        .route("/api", any(api_not_found))
        .route("/api/{*path}", any(api_not_found))
        .fallback_service(static_service)
        .layer(DefaultBodyLimit::max(upload_limit))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::rate_limit::enforce,
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            http::header::X_CONTENT_TYPE_OPTIONS,
            http::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            http::header::REFERRER_POLICY,
            http::HeaderValue::from_static("same-origin"),
        ))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn api_not_found() -> crate::error::AppError {
    crate::error::AppError::NotFound
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn ready(axum::extract::State(state): axum::extract::State<AppState>) -> Json<Value> {
    let database = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .is_ok();
    let redis = if state.settings.task_execution_mode == crate::config::TaskExecutionMode::Redis {
        match &state.redis {
            Some(client) => match client.get_multiplexed_async_connection().await {
                Ok(mut connection) => redis::cmd("PING")
                    .query_async::<String>(&mut connection)
                    .await
                    .is_ok(),
                Err(_) => false,
            },
            None => false,
        }
    } else {
        true
    };
    Json(json!({
        "status": if database && redis { "ready" } else { "not_ready" },
        "checks": { "database": database, "redis": redis }
    }))
}

async fn public_config(axum::extract::State(state): axum::extract::State<AppState>) -> Json<Value> {
    Json(json!({
        "app_name": state.settings.app_name,
        "max_upload_size_mb": state.settings.max_upload_size_mb,
        "storage_driver": match state.settings.storage_driver {
            crate::config::StorageDriver::Local => "local",
            crate::config::StorageDriver::S3 => "s3",
        },
        "features": {
            "streaming": true,
            "s3": state.settings.storage_s3_enabled,
            "custom_base_url": state.settings.allow_custom_base_url,
            "task_queue": match state.settings.task_execution_mode {
                crate::config::TaskExecutionMode::Inline => "inline",
                crate::config::TaskExecutionMode::Redis => "redis",
            },
        }
    }))
}
