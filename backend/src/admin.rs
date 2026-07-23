use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    config::StorageDriver,
    error::{AppError, AppResult},
    storage::StorageRegistry,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/admin/storage",
            get(get_storage).put(update_storage),
        )
        .route("/api/v1/admin/storage/test", post(test_storage))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageView {
    active_driver: &'static str,
    target_config: Value,
    local_asset_count: i64,
    s3_asset_count: i64,
    local_path: String,
    s3_configured: bool,
}

async fn get_storage(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<Json<StorageView>> {
    current.require_admin()?;
    current.require_password_changed()?;
    let target = sqlx::query_scalar::<_, Value>(
        "SELECT value_json FROM system_settings WHERE setting_key = 'storage.target'",
    )
    .fetch_optional(&state.db)
    .await?
    .unwrap_or_else(|| settings_storage_json(&state));
    let counts = sqlx::query_as::<_, (String, i64)>(
        "SELECT storage_driver, COUNT(*)::BIGINT FROM image_assets GROUP BY storage_driver",
    )
    .fetch_all(&state.db)
    .await?;
    let count = |driver: &str| {
        counts
            .iter()
            .find(|(name, _)| name == driver)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    };
    Ok(Json(StorageView {
        active_driver: storage_driver_name(state.settings.storage_driver),
        target_config: target,
        local_asset_count: count("local"),
        s3_asset_count: count("s3"),
        local_path: state.settings.storage_local_path.display().to_string(),
        s3_configured: state.settings.storage_s3_enabled,
    }))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageTarget {
    driver: String,
    local_path: Option<String>,
    s3_bucket: Option<String>,
    s3_region: Option<String>,
    s3_endpoint: Option<String>,
    s3_prefix: Option<String>,
    s3_force_path_style: Option<bool>,
}

async fn update_storage(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<StorageTarget>,
) -> AppResult<Json<Value>> {
    current.require_admin()?;
    current.require_password_changed()?;
    validate_target(&input)?;
    let value = serde_json::to_value(&input).map_err(|error| AppError::Internal(error.into()))?;
    sqlx::query(
        r#"
        INSERT INTO system_settings (setting_key, value_json, description, updated_by)
        VALUES ('storage.target', $1, 'Target storage configuration; applies after restart', $2)
        ON CONFLICT (setting_key) DO UPDATE SET
            value_json = EXCLUDED.value_json,
            description = EXCLUDED.description,
            updated_by = EXCLUDED.updated_by,
            updated_at = NOW()
        "#,
    )
    .bind(value)
    .bind(current.id)
    .execute(&state.db)
    .await?;
    Ok(Json(json!({ "restartRequired": true })))
}

async fn test_storage(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<StorageTarget>,
) -> AppResult<Json<Value>> {
    current.require_admin()?;
    current.require_password_changed()?;
    validate_target(&input)?;
    let mut settings = (*state.settings).clone();
    apply_target(&mut settings, &input)?;
    let registry = StorageRegistry::from_settings(&settings)
        .await
        .map_err(AppError::Internal)?;
    let key = format!("health-check/{}.bin", Uuid::new_v4());
    let expected = Bytes::from_static(b"ai-image-studio-storage-check");
    let stored = registry
        .put(&key, expected.clone())
        .await
        .map_err(AppError::Internal)?;
    let read = registry
        .get(stored.driver, &stored.container, &stored.key)
        .await
        .map_err(AppError::Internal)?;
    let exists = registry
        .exists(stored.driver, &stored.container, &stored.key)
        .await
        .map_err(AppError::Internal)?;
    registry
        .delete(stored.driver, &stored.container, &stored.key)
        .await
        .map_err(AppError::Internal)?;
    if read != expected || !exists {
        return Err(AppError::Internal(anyhow::anyhow!(
            "storage round-trip verification failed"
        )));
    }
    Ok(Json(json!({ "ok": true, "driver": stored.driver })))
}

fn validate_target(input: &StorageTarget) -> AppResult<()> {
    match input.driver.as_str() {
        "local"
            if input
                .local_path
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()) =>
        {
            Ok(())
        }
        "s3" if input
            .s3_bucket
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()) =>
        {
            if let Some(endpoint) = input.s3_endpoint.as_deref() {
                let url = url::Url::parse(endpoint)
                    .map_err(|_| AppError::Validation("invalid S3 endpoint".to_owned()))?;
                if !matches!(url.scheme(), "http" | "https") {
                    return Err(AppError::Validation(
                        "invalid S3 endpoint scheme".to_owned(),
                    ));
                }
            }
            Ok(())
        }
        "local" => Err(AppError::Validation("localPath is required".to_owned())),
        "s3" => Err(AppError::Validation("s3Bucket is required".to_owned())),
        _ => Err(AppError::Validation(
            "storage driver must be local or s3".to_owned(),
        )),
    }
}

fn apply_target(settings: &mut crate::Settings, input: &StorageTarget) -> AppResult<()> {
    settings.storage_driver = match input.driver.as_str() {
        "local" => StorageDriver::Local,
        "s3" => StorageDriver::S3,
        _ => return Err(AppError::Validation("invalid storage driver".to_owned())),
    };
    if let Some(path) = &input.local_path {
        settings.storage_local_path = path.into();
    }
    if input.driver == "s3" {
        settings.storage_s3_enabled = true;
        settings.storage_s3_bucket = input.s3_bucket.clone();
        settings.storage_s3_region = input.s3_region.clone().unwrap_or_else(|| "auto".to_owned());
        settings.storage_s3_endpoint = input.s3_endpoint.clone();
        settings.storage_s3_prefix = input
            .s3_prefix
            .clone()
            .unwrap_or_else(|| "ai-image-studio/".to_owned());
        settings.storage_s3_force_path_style = input.s3_force_path_style.unwrap_or(false);
    }
    Ok(())
}

fn settings_storage_json(state: &AppState) -> Value {
    json!({
        "driver": storage_driver_name(state.settings.storage_driver),
        "localPath": state.settings.storage_local_path,
        "s3Bucket": state.settings.storage_s3_bucket,
        "s3Region": state.settings.storage_s3_region,
        "s3Endpoint": state.settings.storage_s3_endpoint,
        "s3Prefix": state.settings.storage_s3_prefix,
        "s3ForcePathStyle": state.settings.storage_s3_force_path_style
    })
}

fn storage_driver_name(driver: StorageDriver) -> &'static str {
    match driver {
        StorageDriver::Local => "local",
        StorageDriver::S3 => "s3",
    }
}
