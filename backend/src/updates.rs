use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, Postgres, Transaction};
use std::time::{SystemTime, UNIX_EPOCH};
use url::{Host, Url};
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{AppError, AppResult},
    security::verify_password,
};

const ACTION_HEADER: &str = "x-ai-studio-action";
const ACTION_HEADER_VALUE: &str = "update";
const MANIFEST_LIMIT: usize = 1024 * 1024;
const UPDATER_TIMESTAMP_HEADER: &str = "x-ai-studio-timestamp";
const UPDATER_SIGNATURE_HEADER: &str = "x-ai-studio-signature";

type HmacSha256 = Hmac<Sha256>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/updates/status", get(status))
        .route("/api/v1/admin/updates/check", post(check))
        .route("/api/v1/admin/updates/jobs", post(create_job))
        .route("/api/v1/admin/updates/jobs/{id}", get(get_job))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseManifest {
    version: String,
    image: String,
    #[serde(alias = "image_digest")]
    image_digest: String,
    #[serde(alias = "schema_target")]
    schema_target: i64,
    #[serde(alias = "schema_min_supported")]
    schema_min_supported: i64,
    #[serde(alias = "schema_max_supported")]
    schema_max_supported: i64,
    #[serde(alias = "rollback_compatible_to")]
    rollback_compatible_to: String,
    #[serde(alias = "requires_backup")]
    requires_backup: bool,
    #[serde(alias = "destructive_migration")]
    destructive_migration: bool,
    #[serde(alias = "minimum_updater_version")]
    minimum_updater_version: String,
    #[serde(default, alias = "release_notes")]
    release_notes: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCheck {
    manifest: ReleaseManifest,
    has_update: bool,
    schema_compatible: bool,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct UpdateJobView {
    id: Uuid,
    action: String,
    from_version: Option<String>,
    target_version: String,
    status: String,
    progress: i32,
    current_step: Option<String>,
    error_message: Option<String>,
    requested_by: Option<Uuid>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct DeploymentView {
    id: Uuid,
    app_version: String,
    image_reference: String,
    image_digest: Option<String>,
    schema_version: i64,
    backup_reference: Option<String>,
    deployment_status: String,
    deployed_at: chrono::DateTime<chrono::Utc>,
    rolled_back_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStatus {
    current_version: String,
    current_image: String,
    schema_version: i64,
    channel: String,
    manifest_configured: bool,
    updater_configured: bool,
    keep_previous_releases: i64,
    jobs: Vec<UpdateJobView>,
    deployments: Vec<DeploymentView>,
}

async fn status(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<Json<UpdateStatus>> {
    require_admin(&current)?;
    let schema_version = schema_version(&state).await?;
    let jobs = sqlx::query_as::<_, UpdateJobView>(
        r#"
        SELECT id, action, from_version, target_version, status, progress,
               current_step, error_message, requested_by, started_at, finished_at, created_at
        FROM update_jobs ORDER BY created_at DESC LIMIT 20
        "#,
    )
    .fetch_all(&state.db)
    .await?;
    let deployments = sqlx::query_as::<_, DeploymentView>(
        r#"
        SELECT id, app_version, image_reference, image_digest, schema_version,
               backup_reference, deployment_status, deployed_at, rolled_back_at
        FROM deployment_history
        ORDER BY deployed_at DESC
        LIMIT $1
        "#,
    )
    .bind(state.settings.keep_previous_releases + 1)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(UpdateStatus {
        current_version: state.settings.app_version.clone(),
        current_image: state.settings.app_image_reference.clone(),
        schema_version,
        channel: state.settings.update_channel.clone(),
        manifest_configured: state.settings.update_manifest_url.is_some(),
        updater_configured: state.settings.host_updater_token.is_some()
            && (state.settings.host_updater_url.is_some()
                || state.settings.host_updater_socket.is_some()),
        keep_previous_releases: state.settings.keep_previous_releases,
        jobs,
        deployments,
    }))
}

async fn check(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<Json<UpdateCheck>> {
    require_admin(&current)?;
    let manifest = fetch_manifest(&state).await?;
    let current_schema = schema_version(&state).await?;
    let current = parse_version(&state.settings.app_version)?;
    let target = parse_version(&manifest.version)?;
    Ok(Json(UpdateCheck {
        has_update: target > current,
        schema_compatible: current_schema >= manifest.schema_min_supported
            && current_schema <= manifest.schema_max_supported,
        manifest,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateJobRequest {
    action: String,
    target_version: String,
    current_password: String,
    #[serde(default)]
    confirm_destructive_migration: bool,
}

async fn create_job(
    State(state): State<AppState>,
    current: CurrentUser,
    headers: HeaderMap,
    Json(input): Json<CreateJobRequest>,
) -> AppResult<Json<UpdateJobView>> {
    require_admin(&current)?;
    require_action_header(&headers)?;
    verify_current_password(&state, current.id, input.current_password).await?;
    if !matches!(input.action.as_str(), "upgrade" | "rollback") {
        return Err(AppError::Validation(
            "action must be upgrade or rollback".to_owned(),
        ));
    }
    parse_version(&input.target_version)?;
    updater_config(&state)?;
    if input.action == "upgrade" {
        let manifest = fetch_manifest(&state).await?;
        if manifest.version != input.target_version {
            return Err(AppError::Validation(
                "target version does not match the current release manifest".to_owned(),
            ));
        }
        let current_schema = schema_version(&state).await?;
        if current_schema < manifest.schema_min_supported
            || current_schema > manifest.schema_max_supported
        {
            return Err(AppError::Conflict(
                "current schema is not supported by the target release".to_owned(),
            ));
        }
        if manifest.destructive_migration && !input.confirm_destructive_migration {
            return Err(AppError::Validation(
                "destructive migration requires explicit confirmation".to_owned(),
            ));
        }
    } else {
        validate_rollback_target(&state, &input.target_version).await?;
    }

    let mut tx = state.db.begin().await?;
    acquire_update_lock(&mut tx).await?;
    let active = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM update_jobs WHERE status IN ('pending', 'running'))",
    )
    .fetch_one(&mut *tx)
    .await?;
    if active {
        return Err(AppError::Conflict(
            "another update or rollback job is already active".to_owned(),
        ));
    }
    let job_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO update_jobs (
            action, from_version, target_version, status, progress, current_step, requested_by
        ) VALUES ($1, $2, $3, 'pending', 0, 'waiting_for_host_updater', $4)
        RETURNING id
        "#,
    )
    .bind(&input.action)
    .bind(&state.settings.app_version)
    .bind(&input.target_version)
    .bind(current.id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    if let Err(error) = submit_to_updater(
        &state,
        job_id,
        &input.action,
        &input.target_version,
        input.confirm_destructive_migration,
    )
    .await
    {
        sqlx::query(
            r#"
            UPDATE update_jobs SET status = 'failed', current_step = 'host_updater_rejected',
                error_message = $1, finished_at = NOW()
            WHERE id = $2
            "#,
        )
        .bind(error.to_string().chars().take(1000).collect::<String>())
        .bind(job_id)
        .execute(&state.db)
        .await?;
        return Err(AppError::Upstream(error.to_string()));
    }
    sqlx::query(
        r#"
        UPDATE update_jobs SET status = 'running', progress = 1,
            current_step = 'accepted_by_host_updater', started_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(job_id)
    .execute(&state.db)
    .await?;
    Ok(Json(load_job(&state, job_id).await?))
}

async fn get_job(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(job_id): Path<Uuid>,
) -> AppResult<Json<UpdateJobView>> {
    require_admin(&current)?;
    let job = load_job(&state, job_id).await?;
    if matches!(job.status.as_str(), "pending" | "running")
        && (state.settings.host_updater_url.is_some()
            || state.settings.host_updater_socket.is_some())
        && let Err(error) = sync_from_updater(&state, job_id).await
    {
        tracing::warn!(job_id = %job_id, error = %error, "failed to sync Host Updater job");
    }
    Ok(Json(load_job(&state, job_id).await?))
}

async fn load_job(state: &AppState, job_id: Uuid) -> AppResult<UpdateJobView> {
    sqlx::query_as::<_, UpdateJobView>(
        r#"
        SELECT id, action, from_version, target_version, status, progress,
               current_step, error_message, requested_by, started_at, finished_at, created_at
        FROM update_jobs WHERE id = $1
        "#,
    )
    .bind(job_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

fn require_admin(current: &CurrentUser) -> AppResult<()> {
    current.require_admin()?;
    current.require_password_changed()
}

fn require_action_header(headers: &HeaderMap) -> AppResult<()> {
    if headers
        .get(ACTION_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some(ACTION_HEADER_VALUE)
    {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

async fn verify_current_password(
    state: &AppState,
    user_id: Uuid,
    password: String,
) -> AppResult<()> {
    let hash = sqlx::query_scalar::<_, Option<String>>(
        "SELECT password_hash FROM users WHERE id = $1 AND status = 'active'",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .flatten()
    .ok_or(AppError::Unauthorized)?;
    if verify_password(SecretString::from(password), hash)
        .await
        .map_err(AppError::Internal)?
    {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

async fn schema_version(state: &AppState) -> AppResult<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(version), 0)::BIGINT FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&state.db)
    .await?)
}

fn parse_version(value: &str) -> AppResult<Version> {
    Version::parse(value.trim_start_matches('v'))
        .map_err(|_| AppError::Validation("version must use semantic versioning".to_owned()))
}

async fn fetch_manifest(state: &AppState) -> AppResult<ReleaseManifest> {
    let raw_url = state
        .settings
        .update_manifest_url
        .as_deref()
        .ok_or_else(|| AppError::Validation("UPDATE_MANIFEST_URL is not configured".to_owned()))?;
    let url = Url::parse(raw_url)
        .map_err(|_| AppError::Validation("UPDATE_MANIFEST_URL is invalid".to_owned()))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Validation(
            "UPDATE_MANIFEST_URL must use HTTPS and contain no credentials".to_owned(),
        ));
    }
    let response = state
        .http_client
        .get(url)
        .send()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;
    if !response.status().is_success() {
        return Err(AppError::Upstream(format!(
            "release manifest returned HTTP {}",
            response.status()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MANIFEST_LIMIT as u64)
    {
        return Err(AppError::Upstream(
            "release manifest is too large".to_owned(),
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;
    if bytes.len() > MANIFEST_LIMIT {
        return Err(AppError::Upstream(
            "release manifest is too large".to_owned(),
        ));
    }
    let manifest: ReleaseManifest = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::Upstream(format!("invalid release manifest: {error}")))?;
    parse_version(&manifest.version)?;
    if !manifest.image_digest.starts_with("sha256:")
        || manifest.image.trim().is_empty()
        || manifest.schema_target < 0
        || manifest.schema_min_supported < 0
        || manifest.schema_max_supported < manifest.schema_target
        || manifest.schema_target < manifest.schema_min_supported
    {
        return Err(AppError::Upstream(
            "release manifest failed validation".to_owned(),
        ));
    }
    Ok(manifest)
}

async fn validate_rollback_target(state: &AppState, target: &str) -> AppResult<()> {
    let available = sqlx::query_scalar::<_, String>(
        r#"
        SELECT app_version FROM deployment_history
        WHERE deployment_status IN ('active', 'superseded') AND app_version <> $1
        ORDER BY deployed_at DESC
        LIMIT $2
        "#,
    )
    .bind(&state.settings.app_version)
    .bind(state.settings.keep_previous_releases)
    .fetch_all(&state.db)
    .await?;
    if available.iter().any(|version| version == target) {
        Ok(())
    } else {
        Err(AppError::Validation(
            "target is not one of the retained rollback versions".to_owned(),
        ))
    }
}

async fn acquire_update_lock(tx: &mut Transaction<'_, Postgres>) -> AppResult<()> {
    let locked = sqlx::query_scalar::<_, bool>(
        "SELECT pg_try_advisory_xact_lock(hashtext('ai-image-studio:update'))",
    )
    .fetch_one(&mut **tx)
    .await?;
    if locked {
        Ok(())
    } else {
        Err(AppError::Conflict(
            "another update request is being created".to_owned(),
        ))
    }
}

struct UpdaterConnection<'a> {
    base: Url,
    token: &'a SecretString,
    client: reqwest::Client,
}

fn updater_config(state: &AppState) -> AppResult<UpdaterConnection<'_>> {
    let raw_url = state
        .settings
        .host_updater_url
        .as_deref()
        .unwrap_or("http://localhost/");
    let url = Url::parse(raw_url)
        .map_err(|_| AppError::Validation("HOST_UPDATER_URL is invalid".to_owned()))?;
    let token =
        state.settings.host_updater_token.as_ref().ok_or_else(|| {
            AppError::Validation("HOST_UPDATER_TOKEN is not configured".to_owned())
        })?;
    let client = if let Some(socket) = &state.settings.host_updater_socket {
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        {
            return Err(AppError::Validation(
                "Unix-socket Host Updater uses an http://localhost base URL".to_owned(),
            ));
        }
        #[cfg(unix)]
        {
            reqwest::Client::builder()
                .unix_socket(socket.as_path())
                .connect_timeout(std::time::Duration::from_secs(
                    state.settings.connect_timeout_seconds,
                ))
                .timeout(std::time::Duration::from_secs(
                    state.settings.request_timeout_seconds,
                ))
                .build()
                .map_err(|error| AppError::Internal(error.into()))?
        }
        #[cfg(not(unix))]
        {
            let _ = socket;
            return Err(AppError::Validation(
                "HOST_UPDATER_SOCKET is only supported on Unix".to_owned(),
            ));
        }
    } else {
        let local_http = url.scheme() == "http"
            && match url.host() {
                Some(Host::Ipv4(ip)) => ip.is_loopback(),
                Some(Host::Ipv6(ip)) => ip.is_loopback(),
                _ => false,
            };
        let trusted_service = url.scheme() == "http" && matches!(url.host_str(), Some("localhost"));
        if url.scheme() != "https" && !local_http && !trusted_service {
            return Err(AppError::Validation(
                "HOST_UPDATER_URL must use HTTPS or a loopback address".to_owned(),
            ));
        }
        state.http_client.clone()
    };
    Ok(UpdaterConnection {
        base: url,
        token,
        client,
    })
}

async fn submit_to_updater(
    state: &AppState,
    job_id: Uuid,
    action: &str,
    target_version: &str,
    confirm_destructive_migration: bool,
) -> anyhow::Result<()> {
    let connection = updater_config(state).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let endpoint = connection.base.join("v1/jobs")?;
    let body = serde_json::to_vec(&serde_json::json!({
        "jobId": job_id,
        "action": action,
        "targetVersion": target_version,
        "confirmDestructiveMigration": confirm_destructive_migration
    }))?;
    let auth_headers = updater_auth_headers("POST", endpoint.path(), &body, connection.token)?;
    let response = connection
        .client
        .post(endpoint)
        .headers(auth_headers)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("Host Updater returned HTTP {}", response.status());
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdaterJob {
    status: String,
    progress: i32,
    current_step: Option<String>,
    error_message: Option<String>,
    deployment: Option<UpdaterDeployment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdaterDeployment {
    app_version: String,
    image_reference: String,
    image_digest: String,
    schema_version: i64,
    backup_reference: String,
}

async fn sync_from_updater(state: &AppState, job_id: Uuid) -> anyhow::Result<()> {
    let connection = updater_config(state).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let endpoint = connection.base.join(&format!("v1/jobs/{job_id}"))?;
    let auth_headers = updater_auth_headers("GET", endpoint.path(), &[], connection.token)?;
    let response = connection
        .client
        .get(endpoint)
        .headers(auth_headers)
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("Host Updater returned HTTP {}", response.status());
    }
    let job = response.json::<UpdaterJob>().await?;
    if !matches!(
        job.status.as_str(),
        "pending" | "running" | "succeeded" | "failed" | "cancelled"
    ) || !(0..=100).contains(&job.progress)
    {
        anyhow::bail!("Host Updater returned an invalid job state");
    }
    if job.status == "succeeded" && job.deployment.is_none() {
        anyhow::bail!("Host Updater completed without deployment evidence");
    }
    if let Some(deployment) = &job.deployment {
        validate_updater_deployment(deployment)?;
    }
    let target_version =
        sqlx::query_scalar::<_, String>("SELECT target_version FROM update_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(&state.db)
            .await?
            .context("update job no longer exists")?;
    if job
        .deployment
        .as_ref()
        .is_some_and(|deployment| deployment.app_version != target_version)
    {
        anyhow::bail!("Host Updater deployment version does not match the requested target");
    }
    let mut tx = state.db.begin().await?;
    sqlx::query(
        r#"
        UPDATE update_jobs
        SET status = $1, progress = $2, current_step = $3, error_message = $4,
            started_at = CASE WHEN $1 = 'running' THEN COALESCE(started_at, NOW()) ELSE started_at END,
            finished_at = CASE WHEN $1 IN ('succeeded', 'failed', 'cancelled') THEN NOW() ELSE NULL END
        WHERE id = $5
        "#,
    )
    .bind(job.status)
    .bind(job.progress)
    .bind(job.current_step)
    .bind(job.error_message.map(|value| value.chars().take(1000).collect::<String>()))
    .bind(job_id)
    .execute(&mut *tx)
    .await?;
    if let Some(deployment) = job.deployment {
        sqlx::query(
            r#"
            UPDATE deployment_history
            SET deployment_status = CASE
                WHEN deployment_status = 'active' THEN 'superseded'
                ELSE deployment_status
            END
            WHERE source_job_id IS DISTINCT FROM $1
            "#,
        )
        .bind(job_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO deployment_history (
                app_version, image_reference, image_digest, schema_version,
                backup_reference, deployment_status, source_job_id
            ) VALUES ($1, $2, $3, $4, $5, 'active', $6)
            ON CONFLICT (source_job_id) DO UPDATE SET
                app_version = EXCLUDED.app_version,
                image_reference = EXCLUDED.image_reference,
                image_digest = EXCLUDED.image_digest,
                schema_version = EXCLUDED.schema_version,
                backup_reference = EXCLUDED.backup_reference,
                deployment_status = 'active'
            "#,
        )
        .bind(deployment.app_version)
        .bind(deployment.image_reference)
        .bind(deployment.image_digest)
        .bind(deployment.schema_version)
        .bind(deployment.backup_reference)
        .bind(job_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn updater_auth_headers(
    method: &str,
    path: &str,
    body: &[u8],
    token: &SecretString,
) -> anyhow::Result<HeaderMap> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let signature = updater_signature(timestamp, method, path, body, token)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", token.expose_secret()).parse()?,
    );
    headers.insert(UPDATER_TIMESTAMP_HEADER, timestamp.to_string().parse()?);
    headers.insert(UPDATER_SIGNATURE_HEADER, signature.parse()?);
    Ok(headers)
}

fn updater_signature(
    timestamp: u64,
    method: &str,
    path: &str,
    body: &[u8],
    token: &SecretString,
) -> anyhow::Result<String> {
    let body_hash = hex::encode(Sha256::digest(body));
    let payload = format!("{timestamp}\n{method}\n{path}\n{body_hash}");
    let mut mac = HmacSha256::new_from_slice(token.expose_secret().as_bytes())?;
    mac.update(payload.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn validate_updater_deployment(deployment: &UpdaterDeployment) -> anyhow::Result<()> {
    parse_version(&deployment.app_version).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    if deployment.image_reference.trim().is_empty()
        || deployment.backup_reference.trim().is_empty()
        || deployment.schema_version < 0
        || deployment.image_digest.len() != 71
        || !deployment.image_digest.starts_with("sha256:")
        || !deployment.image_digest[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        anyhow::bail!("Host Updater returned invalid deployment evidence");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_versions_accept_optional_v_prefix() {
        assert_eq!(parse_version("v1.2.3").unwrap(), Version::new(1, 2, 3));
        assert!(parse_version("latest").is_err());
    }

    #[test]
    fn release_manifest_accepts_the_snake_case_updater_contract() {
        let manifest: ReleaseManifest = serde_json::from_value(serde_json::json!({
            "version": "1.2.3",
            "image": "ghcr.io/example/app:v1.2.3",
            "image_digest": format!("sha256:{}", "a".repeat(64)),
            "schema_target": 10,
            "schema_min_supported": 7,
            "schema_max_supported": 10,
            "rollback_compatible_to": "1.1.0",
            "requires_backup": true,
            "destructive_migration": false,
            "minimum_updater_version": "0.1.0",
            "release_notes": "test"
        }))
        .unwrap();

        assert_eq!(manifest.schema_target, 10);
        assert_eq!(manifest.minimum_updater_version, "0.1.0");
        let response = serde_json::to_value(manifest).unwrap();
        assert_eq!(response["schemaTarget"], 10);
        assert!(response.get("schema_target").is_none());
    }

    #[test]
    fn updater_deployment_requires_complete_evidence() {
        let valid = UpdaterDeployment {
            app_version: "1.2.3".to_owned(),
            image_reference: "ghcr.io/example/app:v1.2.3".to_owned(),
            image_digest: format!("sha256:{}", "a".repeat(64)),
            schema_version: 9,
            backup_reference: "/backups/job/backup-manifest.json".to_owned(),
        };
        assert!(validate_updater_deployment(&valid).is_ok());
        let mut invalid = valid;
        invalid.image_digest = "sha256:abc".to_owned();
        assert!(validate_updater_deployment(&invalid).is_err());
    }

    #[test]
    fn updater_signature_matches_the_protocol_test_vector() {
        let signature = updater_signature(
            1_700_000_000,
            "POST",
            "/v1/jobs",
            br#"{"a":1}"#,
            &SecretString::from("0123456789abcdef0123456789abcdef"),
        )
        .unwrap();
        assert_eq!(
            signature,
            "cabf9479aa68b213187b0f53b471c48bfcc831e4a568ba7db39bb769768aa7b7"
        );
    }
}
