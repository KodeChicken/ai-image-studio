use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use url::{Host, Url};
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{AppError, AppResult},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/providers", get(list).post(create))
        .route(
            "/api/v1/providers/{id}",
            get(get_one).patch(update).delete(remove),
        )
        .route(
            "/api/v1/providers/{id}/test",
            axum::routing::post(test_connection),
        )
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub id: Uuid,
    pub provider_key: String,
    pub provider_type: String,
    pub display_name: String,
    pub base_url: String,
    pub enabled: bool,
    pub config_json: Value,
    pub credential_configured: bool,
    pub model_count: i64,
    pub health_status: String,
    pub last_health_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_health_error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ProviderCredentialRow {
    pub id: Uuid,
    pub provider_type: String,
    pub base_url: String,
    pub credential_ciphertext: Option<Vec<u8>>,
    pub credential_nonce: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateProviderRequest {
    provider_key: String,
    provider_type: String,
    display_name: String,
    base_url: String,
    api_key: Option<String>,
    #[serde(default)]
    config: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProviderRequest {
    display_name: Option<String>,
    base_url: Option<String>,
    enabled: Option<bool>,
    api_key: Option<String>,
    config: Option<Value>,
}

async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<Json<Vec<ProviderSummary>>> {
    current.require_password_changed()?;
    let providers = sqlx::query_as::<_, ProviderSummary>(PROVIDER_SELECT)
        .bind(current.id)
        .bind(None::<Uuid>)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(providers))
}

async fn get_one(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ProviderSummary>> {
    current.require_password_changed()?;
    let provider = sqlx::query_as::<_, ProviderSummary>(PROVIDER_SELECT)
        .bind(current.id)
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(provider))
}

async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<CreateProviderRequest>,
) -> AppResult<(StatusCode, Json<ProviderSummary>)> {
    current.require_password_changed()?;
    validate_provider_key(&input.provider_key)?;
    validate_provider_type(&input.provider_type)?;
    validate_display_name(&input.display_name)?;
    let base_url = validate_base_url(&input.base_url, &state.settings)?;
    if !input.config.is_object() {
        return Err(AppError::Validation(
            "provider config must be a JSON object".to_owned(),
        ));
    }
    reject_sensitive_config(&input.config)?;
    let encrypted = input
        .api_key
        .filter(|value| !value.trim().is_empty())
        .map(|value| state.credential_cipher.encrypt(&SecretString::from(value)))
        .transpose()
        .map_err(AppError::Internal)?;
    let (ciphertext, nonce, key_version) = encrypted
        .map(|value| {
            (
                Some(value.ciphertext),
                Some(value.nonce),
                Some(value.key_version),
            )
        })
        .unwrap_or((None, None, None));

    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO providers (
            owner_id, provider_key, provider_type, display_name, base_url, config_json,
            credential_ciphertext, credential_nonce, credential_key_version
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id
        "#,
    )
    .bind(current.id)
    .bind(input.provider_key.trim())
    .bind(input.provider_type)
    .bind(input.display_name.trim())
    .bind(base_url.as_str().trim_end_matches('/'))
    .bind(input.config)
    .bind(ciphertext)
    .bind(nonce)
    .bind(key_version)
    .fetch_one(&state.db)
    .await
    .map_err(map_unique_conflict)?;

    let provider = sqlx::query_as::<_, ProviderSummary>(PROVIDER_SELECT)
        .bind(current.id)
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok((StatusCode::CREATED, Json(provider)))
}

async fn update(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateProviderRequest>,
) -> AppResult<Json<ProviderSummary>> {
    current.require_password_changed()?;
    if let Some(display_name) = input.display_name.as_deref() {
        validate_display_name(display_name)?;
    }
    let base_url = input
        .base_url
        .as_deref()
        .map(|value| validate_base_url(value, &state.settings))
        .transpose()?;
    if matches!(&input.config, Some(value) if !value.is_object()) {
        return Err(AppError::Validation(
            "provider config must be a JSON object".to_owned(),
        ));
    }
    if let Some(config) = &input.config {
        reject_sensitive_config(config)?;
    }
    let encrypted = input
        .api_key
        .filter(|value| !value.trim().is_empty())
        .map(|value| state.credential_cipher.encrypt(&SecretString::from(value)))
        .transpose()
        .map_err(AppError::Internal)?;
    let rotate_credential = encrypted.is_some();
    let (ciphertext, nonce, key_version) = encrypted
        .map(|value| {
            (
                Some(value.ciphertext),
                Some(value.nonce),
                Some(value.key_version),
            )
        })
        .unwrap_or((None, None, None));

    let changed = sqlx::query(
        r#"
        UPDATE providers
        SET display_name = COALESCE($1, display_name),
            base_url = COALESCE($2, base_url),
            enabled = COALESCE($3, enabled),
            config_json = COALESCE($4, config_json),
            credential_ciphertext = CASE WHEN $5 THEN $6 ELSE credential_ciphertext END,
            credential_nonce = CASE WHEN $5 THEN $7 ELSE credential_nonce END,
            credential_key_version = CASE WHEN $5 THEN $8 ELSE credential_key_version END,
            updated_at = NOW()
        WHERE id = $9 AND owner_id = $10 AND deleted_at IS NULL
        "#,
    )
    .bind(input.display_name.map(|value| value.trim().to_owned()))
    .bind(base_url.map(|value| value.as_str().trim_end_matches('/').to_owned()))
    .bind(input.enabled)
    .bind(input.config)
    .bind(rotate_credential)
    .bind(ciphertext)
    .bind(nonce)
    .bind(key_version)
    .bind(id)
    .bind(current.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    get_one(State(state), current, Path(id)).await
}

async fn remove(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    current.require_password_changed()?;
    let mut tx = state.db.begin().await?;
    let changed = sqlx::query(
        r#"
        UPDATE providers
        SET enabled = FALSE, deleted_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(id)
    .bind(current.id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    sqlx::query(
        "UPDATE models SET enabled = FALSE, deleted_at = NOW(), updated_at = NOW() WHERE provider_id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderHealthResult {
    status: &'static str,
    model_count: usize,
    latency_ms: u64,
    checked_at: chrono::DateTime<chrono::Utc>,
}

async fn test_connection(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ProviderHealthResult>> {
    current.require_password_changed()?;
    let provider = credential_row(&state, current.id, id).await?;
    let (ciphertext, nonce) = match provider
        .credential_ciphertext
        .zip(provider.credential_nonce)
    {
        Some(value) => value,
        None => {
            update_health(
                &state,
                id,
                "unhealthy",
                Some("provider API key is not configured"),
            )
            .await?;
            return Err(AppError::Validation(
                "provider API key is not configured".to_owned(),
            ));
        }
    };
    let credential = state
        .credential_cipher
        .decrypt(&ciphertext, &nonce)
        .map_err(AppError::Internal)?;
    let started = std::time::Instant::now();
    let models = match crate::provider_adapters::list_models(
        &provider.provider_type,
        &state.http_client,
        &provider.base_url,
        &credential,
    )
    .await
    {
        Ok(models) => models,
        Err(error) => {
            let summary = error.to_string();
            update_health(&state, id, "unhealthy", Some(&summary)).await?;
            return Err(AppError::Upstream(summary));
        }
    };
    let checked_at = chrono::Utc::now();
    update_health(&state, id, "healthy", None).await?;
    Ok(Json(ProviderHealthResult {
        status: "healthy",
        model_count: models.len(),
        latency_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        checked_at,
    }))
}

pub(crate) async fn update_health(
    state: &AppState,
    provider_id: Uuid,
    status: &str,
    error: Option<&str>,
) -> AppResult<()> {
    let error = error.map(|value| value.chars().take(1000).collect::<String>());
    sqlx::query(
        r#"
        UPDATE providers
        SET health_status = $1, last_health_checked_at = NOW(),
            last_health_error = $2, updated_at = NOW()
        WHERE id = $3 AND deleted_at IS NULL
        "#,
    )
    .bind(status)
    .bind(error)
    .bind(provider_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

pub(crate) async fn credential_row(
    state: &AppState,
    owner_id: Uuid,
    provider_id: Uuid,
) -> AppResult<ProviderCredentialRow> {
    sqlx::query_as::<_, ProviderCredentialRow>(
        r#"
        SELECT id, provider_type, base_url, credential_ciphertext, credential_nonce
        FROM providers
        WHERE id = $1 AND owner_id = $2 AND enabled AND deleted_at IS NULL
        "#,
    )
    .bind(provider_id)
    .bind(owner_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

const PROVIDER_SELECT: &str = r#"
    SELECT p.id, p.provider_key, p.provider_type, p.display_name, p.base_url,
           p.enabled, p.config_json,
           (p.credential_ciphertext IS NOT NULL) AS credential_configured,
           COUNT(m.id)::BIGINT AS model_count,
           p.health_status, p.last_health_checked_at, p.last_health_error,
           p.created_at, p.updated_at
    FROM providers p
    LEFT JOIN models m ON m.provider_id = p.id AND m.deleted_at IS NULL
    WHERE p.owner_id = $1 AND p.deleted_at IS NULL AND ($2::UUID IS NULL OR p.id = $2)
    GROUP BY p.id
    ORDER BY p.created_at DESC
"#;

fn validate_provider_key(value: &str) -> AppResult<()> {
    let valid = (2..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(AppError::Validation(
            "providerKey must contain 2 to 64 letters, digits, dashes or underscores".to_owned(),
        ))
    }
}

fn validate_provider_type(value: &str) -> AppResult<()> {
    if matches!(
        value,
        "openai-compatible" | "gemini" | "grok" | "flux" | "comfyui" | "custom"
    ) {
        Ok(())
    } else {
        Err(AppError::Validation("unsupported provider type".to_owned()))
    }
}

fn validate_display_name(value: &str) -> AppResult<()> {
    if value.trim().is_empty() || value.len() > 128 {
        Err(AppError::Validation(
            "displayName must contain 1 to 128 characters".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn validate_base_url(value: &str, settings: &crate::Settings) -> AppResult<Url> {
    let parsed = Url::parse(value)
        .map_err(|_| AppError::Validation("baseUrl must be a valid absolute URL".to_owned()))?;
    if parsed.scheme() != "https"
        && !(settings.allow_private_provider_hosts && parsed.scheme() == "http")
    {
        return Err(AppError::Validation("baseUrl must use HTTPS".to_owned()));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some() {
        return Err(AppError::Validation(
            "baseUrl must not contain credentials or query parameters".to_owned(),
        ));
    }
    if let Some(Host::Ipv4(ip)) = parsed.host()
        && (ip.is_loopback() || ip.is_private() || ip.is_link_local())
        && !settings.allow_private_provider_hosts
    {
        return Err(AppError::Validation(
            "private baseUrl is disabled".to_owned(),
        ));
    }
    if let Some(Host::Ipv6(ip)) = parsed.host()
        && (ip.is_loopback() || ip.is_unspecified())
        && !settings.allow_private_provider_hosts
    {
        return Err(AppError::Validation(
            "private baseUrl is disabled".to_owned(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::Validation("baseUrl must contain a host".to_owned()))?
        .to_ascii_lowercase();
    if !settings.allow_custom_base_url
        && !settings
            .allowed_provider_hosts
            .iter()
            .any(|item| item == &host)
    {
        return Err(AppError::Validation(
            "custom Provider hosts are disabled by the administrator".to_owned(),
        ));
    }
    Ok(parsed)
}

fn map_unique_conflict(error: sqlx::Error) -> AppError {
    if matches!(&error, sqlx::Error::Database(db_error) if db_error.is_unique_violation()) {
        AppError::Conflict("provider key already exists".to_owned())
    } else {
        AppError::Database(error)
    }
}

fn reject_sensitive_config(value: &Value) -> AppResult<()> {
    fn contains_secret(value: &Value) -> bool {
        match value {
            Value::Object(object) => object.iter().any(|(key, child)| {
                let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
                matches!(
                    normalized.as_str(),
                    "apikey" | "key" | "token" | "secret" | "authorization" | "credential"
                ) || contains_secret(child)
            }),
            Value::Array(values) => values.iter().any(contains_secret),
            _ => false,
        }
    }
    if contains_secret(value) {
        Err(AppError::Validation(
            "provider config must not contain credentials; use apiKey instead".to_owned(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_config_rejects_nested_secret() {
        assert!(
            reject_sensitive_config(&serde_json::json!({ "headers": { "api_key": "secret" } }))
                .is_err()
        );
    }
}
