use std::{
    collections::HashMap,
    env,
    fs::File,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use async_trait::async_trait;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path as AxumPath, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    sync::{Mutex, RwLock, mpsc},
    time::timeout,
};
use tower_http::{
    limit::RequestBodyLimitLayer, sensitive_headers::SetSensitiveRequestHeadersLayer,
    trace::TraceLayer,
};
use uuid::Uuid;

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_CLOCK_SKEW_SECONDS: u64 = 60;
const MAX_ERROR_BYTES: usize = 16 * 1024;
const TIMESTAMP_HEADER: &str = "x-ai-studio-timestamp";
const SIGNATURE_HEADER: &str = "x-ai-studio-signature";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug)]
pub struct Settings {
    pub listen_addr: SocketAddr,
    pub unix_socket: Option<PathBuf>,
    pub socket_gid: Option<u32>,
    pub token: SecretString,
    pub state_dir: PathBuf,
    pub executor_path: PathBuf,
    pub executor_config_path: PathBuf,
    pub executor_sha256: Option<String>,
    pub job_timeout: Duration,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        let listen_addr = env_or("HOST_UPDATER_LISTEN_ADDR", "127.0.0.1:3199")
            .parse::<SocketAddr>()
            .context("HOST_UPDATER_LISTEN_ADDR is invalid")?;
        if !listen_addr.ip().is_loopback() {
            bail!("HOST_UPDATER_LISTEN_ADDR must use a loopback address");
        }
        let unix_socket = optional("HOST_UPDATER_UNIX_SOCKET").map(PathBuf::from);
        if unix_socket.as_ref().is_some_and(|path| !path.is_absolute()) {
            bail!("HOST_UPDATER_UNIX_SOCKET must be an absolute path");
        }
        #[cfg(not(unix))]
        if unix_socket.is_some() {
            bail!("HOST_UPDATER_UNIX_SOCKET is only supported on Unix hosts");
        }
        let socket_gid = optional("HOST_UPDATER_SOCKET_GID")
            .map(|value| {
                value
                    .parse::<u32>()
                    .context("HOST_UPDATER_SOCKET_GID is invalid")
            })
            .transpose()?;
        if unix_socket.is_some() != socket_gid.is_some() {
            bail!(
                "HOST_UPDATER_UNIX_SOCKET and HOST_UPDATER_SOCKET_GID must be configured together"
            );
        }
        let token = required("HOST_UPDATER_TOKEN")?;
        if token.len() < 32 {
            bail!("HOST_UPDATER_TOKEN must contain at least 32 bytes");
        }
        let state_dir = absolute_path("HOST_UPDATER_STATE_DIR")?;
        let executor_path = absolute_path("HOST_UPDATER_EXECUTOR_PATH")?;
        let executor_config_path = absolute_path("HOST_UPDATER_EXECUTOR_CONFIG")?;
        let executor_sha256 = optional("HOST_UPDATER_EXECUTOR_SHA256");
        if executor_sha256.as_deref().is_some_and(|value| {
            value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            bail!("HOST_UPDATER_EXECUTOR_SHA256 must be a 64-character SHA-256 hex digest");
        }
        let job_timeout_seconds = env_or("HOST_UPDATER_JOB_TIMEOUT_SECONDS", "1800")
            .parse::<u64>()
            .context("HOST_UPDATER_JOB_TIMEOUT_SECONDS is invalid")?;
        if !(60..=86_400).contains(&job_timeout_seconds) {
            bail!("HOST_UPDATER_JOB_TIMEOUT_SECONDS must be between 60 and 86400");
        }
        Ok(Self {
            listen_addr,
            unix_socket,
            socket_gid,
            token: SecretString::from(token),
            state_dir,
            executor_path,
            executor_config_path,
            executor_sha256: executor_sha256.map(|value| value.to_ascii_lowercase()),
            job_timeout: Duration::from_secs(job_timeout_seconds),
        })
    }
}

fn env_or(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn required(name: &str) -> anyhow::Result<String> {
    optional(name).with_context(|| format!("{name} is required"))
}

fn optional(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn absolute_path(name: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(required(name)?);
    if !path.is_absolute() {
        bail!("{name} must be an absolute path");
    }
    Ok(path)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobRequest {
    pub job_id: Uuid,
    pub action: String,
    pub target_version: String,
    #[serde(default)]
    pub confirm_destructive_migration: bool,
}

impl JobRequest {
    fn validate(&self) -> Result<(), ApiError> {
        if !matches!(self.action.as_str(), "upgrade" | "rollback") {
            return Err(ApiError::validation("action must be upgrade or rollback"));
        }
        Version::parse(self.target_version.trim_start_matches('v'))
            .map_err(|_| ApiError::validation("targetVersion must use semantic versioning"))?;
        if self.action == "rollback" && self.confirm_destructive_migration {
            return Err(ApiError::validation(
                "confirmDestructiveMigration only applies to upgrades",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentResult {
    pub app_version: String,
    pub image_reference: String,
    pub image_digest: String,
    pub schema_version: i64,
    pub backup_reference: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub job_id: Uuid,
    pub action: String,
    pub target_version: String,
    pub confirm_destructive_migration: bool,
    pub status: String,
    pub progress: i32,
    pub current_step: Option<String>,
    pub error_message: Option<String>,
    pub deployment: Option<DeploymentResult>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl JobView {
    fn pending(request: &JobRequest) -> Self {
        Self {
            job_id: request.job_id,
            action: request.action.clone(),
            target_version: request.target_version.clone(),
            confirm_destructive_migration: request.confirm_destructive_migration,
            status: "pending".to_owned(),
            progress: 0,
            current_step: Some("queued".to_owned()),
            error_message: None,
            deployment: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ExecutorMessage {
    Progress {
        progress: i32,
        #[serde(rename = "currentStep", alias = "current_step")]
        current_step: String,
    },
    Result {
        #[serde(flatten)]
        deployment: DeploymentResult,
    },
}

#[derive(Clone, Debug)]
pub struct ProgressUpdate {
    pub progress: i32,
    pub current_step: String,
}

#[async_trait]
pub trait Executor: Send + Sync + 'static {
    async fn execute(
        &self,
        request: JobRequest,
        progress: mpsc::UnboundedSender<ProgressUpdate>,
    ) -> anyhow::Result<DeploymentResult>;
}

pub struct ProcessExecutor {
    executable: PathBuf,
    config_path: PathBuf,
    timeout: Duration,
}

impl ProcessExecutor {
    pub async fn from_settings(settings: &Settings) -> anyhow::Result<Self> {
        let executable = tokio::fs::canonicalize(&settings.executor_path)
            .await
            .with_context(|| {
                format!(
                    "failed to resolve executor {}",
                    settings.executor_path.display()
                )
            })?;
        let config_path = tokio::fs::canonicalize(&settings.executor_config_path)
            .await
            .with_context(|| {
                format!(
                    "failed to resolve executor config {}",
                    settings.executor_config_path.display()
                )
            })?;
        if let Some(expected) = &settings.executor_sha256 {
            let bytes = tokio::fs::read(&executable).await?;
            let actual = hex::encode(Sha256::digest(bytes));
            if &actual != expected {
                bail!("Host Updater executor SHA-256 does not match configuration");
            }
        }
        Ok(Self {
            executable,
            config_path,
            timeout: settings.job_timeout,
        })
    }

    async fn execute_inner(
        &self,
        request: JobRequest,
        progress: mpsc::UnboundedSender<ProgressUpdate>,
    ) -> anyhow::Result<DeploymentResult> {
        let mut child = Command::new(&self.executable)
            .arg("--config")
            .arg(&self.config_path)
            .arg("--job-id")
            .arg(request.job_id.to_string())
            .arg("--action")
            .arg(&request.action)
            .arg("--target-version")
            .arg(&request.target_version)
            .args(
                request
                    .confirm_destructive_migration
                    .then_some("--confirm-destructive-migration"),
            )
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("failed to start the fixed Host Updater executor")?;
        let stdout = child
            .stdout
            .take()
            .context("executor stdout is unavailable")?;
        let stderr = child
            .stderr
            .take()
            .context("executor stderr is unavailable")?;
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).take(MAX_ERROR_BYTES as u64);
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await?;
            anyhow::Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
        });
        let mut lines = BufReader::new(stdout).lines();
        let mut result = None;
        while let Some(line) = lines.next_line().await? {
            let message: ExecutorMessage = serde_json::from_str(&line)
                .with_context(|| format!("executor emitted an invalid JSON line: {line}"))?;
            match message {
                ExecutorMessage::Progress {
                    progress: value,
                    current_step,
                } => {
                    if !(1..=99).contains(&value) || current_step.trim().is_empty() {
                        bail!("executor emitted an invalid progress update");
                    }
                    let _ = progress.send(ProgressUpdate {
                        progress: value,
                        current_step,
                    });
                }
                ExecutorMessage::Result { deployment } => result = Some(deployment),
            }
        }
        let status = child.wait().await?;
        let stderr = stderr_task.await.context("executor stderr task failed")??;
        if !status.success() {
            bail!(
                "executor failed with {}{}",
                status,
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                }
            );
        }
        result.context("executor completed without a deployment result")
    }
}

#[async_trait]
impl Executor for ProcessExecutor {
    async fn execute(
        &self,
        request: JobRequest,
        progress: mpsc::UnboundedSender<ProgressUpdate>,
    ) -> anyhow::Result<DeploymentResult> {
        timeout(self.timeout, self.execute_inner(request, progress))
            .await
            .context("Host Updater executor timed out")?
    }
}

struct Store {
    jobs_dir: PathBuf,
    jobs: RwLock<HashMap<Uuid, JobView>>,
    active_job: Mutex<Option<Uuid>>,
    _process_lock: File,
}

impl Store {
    async fn open(state_dir: &Path) -> anyhow::Result<Self> {
        let jobs_dir = state_dir.join("jobs");
        tokio::fs::create_dir_all(&jobs_dir).await?;
        let lock_path = state_dir.join("update.lock");
        let process_lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open {}", lock_path.display()))?;
        process_lock
            .try_lock_exclusive()
            .context("another Host Updater process already owns update.lock")?;

        let mut jobs = HashMap::new();
        let mut entries = tokio::fs::read_dir(&jobs_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes = tokio::fs::read(entry.path()).await?;
            let mut job: JobView = serde_json::from_slice(&bytes)
                .with_context(|| format!("invalid job state {}", entry.path().display()))?;
            if matches!(job.status.as_str(), "pending" | "running") {
                job.status = "failed".to_owned();
                job.current_step = Some("updater_restarted".to_owned());
                job.error_message =
                    Some("Host Updater restarted before the job completed".to_owned());
                job.finished_at = Some(Utc::now());
                persist_job(&jobs_dir, &job).await?;
            }
            jobs.insert(job.job_id, job);
        }
        Ok(Self {
            jobs_dir,
            jobs: RwLock::new(jobs),
            active_job: Mutex::new(None),
            _process_lock: process_lock,
        })
    }

    async fn get(&self, id: Uuid) -> Option<JobView> {
        self.jobs.read().await.get(&id).cloned()
    }

    async fn insert(&self, job: JobView) -> anyhow::Result<()> {
        persist_job(&self.jobs_dir, &job).await?;
        self.jobs.write().await.insert(job.job_id, job);
        Ok(())
    }

    async fn update(&self, id: Uuid, update: impl FnOnce(&mut JobView)) -> anyhow::Result<JobView> {
        let job = {
            let mut jobs = self.jobs.write().await;
            let job = jobs.get_mut(&id).context("job state disappeared")?;
            update(job);
            job.clone()
        };
        persist_job(&self.jobs_dir, &job).await?;
        Ok(job)
    }
}

async fn persist_job(jobs_dir: &Path, job: &JobView) -> anyhow::Result<()> {
    let path = jobs_dir.join(format!("{}.json", job.job_id));
    let temporary = jobs_dir.join(format!(".{}.{}.tmp", job.job_id, Uuid::new_v4()));
    tokio::fs::write(&temporary, serde_json::to_vec_pretty(job)?).await?;
    if cfg!(windows) && tokio::fs::try_exists(&path).await? {
        tokio::fs::remove_file(&path).await?;
    }
    tokio::fs::rename(&temporary, &path).await?;
    Ok(())
}

#[derive(Clone)]
struct AppState {
    token: SecretString,
    store: Arc<Store>,
    executor: Arc<dyn Executor>,
}

pub async fn build_router(
    settings: &Settings,
    executor: Arc<dyn Executor>,
) -> anyhow::Result<Router> {
    let state = AppState {
        token: settings.token.clone(),
        store: Arc::new(Store::open(&settings.state_dir).await?),
        executor,
    };
    Ok(Router::new()
        .route("/health", get(health))
        .route("/v1/jobs", post(create_job))
        .route("/v1/jobs/{id}", get(get_job))
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BYTES))
        .layer(SetSensitiveRequestHeadersLayer::new(std::iter::once(
            header::AUTHORIZATION,
        )))
        .layer(TraceLayer::new_for_http())
        .with_state(state))
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn create_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<JobView>), ApiError> {
    verify_request(&headers, "POST", "/v1/jobs", &body, &state.token)?;
    let request: JobRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::validation("request body is invalid"))?;
    request.validate()?;

    let mut active = state.store.active_job.lock().await;
    if let Some(existing) = state.store.get(request.job_id).await {
        if existing.action == request.action
            && existing.target_version == request.target_version
            && existing.confirm_destructive_migration == request.confirm_destructive_migration
        {
            return Ok((StatusCode::OK, Json(existing)));
        }
        return Err(ApiError::conflict(
            "jobId already exists with different parameters",
        ));
    }
    if active.is_some() {
        return Err(ApiError::conflict("another update job is already active"));
    }
    let job = JobView::pending(&request);
    state
        .store
        .insert(job.clone())
        .await
        .map_err(ApiError::internal)?;
    *active = Some(request.job_id);
    drop(active);

    tokio::spawn(run_job(state, request));
    Ok((StatusCode::ACCEPTED, Json(job)))
}

async fn get_job(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<Uuid>,
    headers: HeaderMap,
) -> Result<Json<JobView>, ApiError> {
    verify_request(
        &headers,
        "GET",
        &format!("/v1/jobs/{id}"),
        &[],
        &state.token,
    )?;
    state
        .store
        .get(id)
        .await
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

async fn run_job(state: AppState, request: JobRequest) {
    let job_id = request.job_id;
    let target_version = request.target_version.clone();
    if let Err(error) = state
        .store
        .update(job_id, |job| {
            job.status = "running".to_owned();
            job.progress = 1;
            job.current_step = Some("executor_started".to_owned());
            job.started_at = Some(Utc::now());
        })
        .await
    {
        tracing::error!(%job_id, %error, "failed to persist running updater job");
    }

    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();
    let execution = state.executor.execute(request, progress_tx);
    tokio::pin!(execution);
    let result = loop {
        tokio::select! {
            result = &mut execution => break result,
            progress = progress_rx.recv() => {
                let Some(progress) = progress else { continue };
                if let Err(error) = state.store.update(job_id, |job| {
                    if progress.progress > job.progress {
                        job.progress = progress.progress;
                        job.current_step = Some(progress.current_step);
                    }
                }).await {
                    tracing::error!(%job_id, %error, "failed to persist updater progress");
                }
            }
        }
    };

    let result = result.and_then(|deployment| {
        validate_deployment_result(&deployment, &target_version)?;
        Ok(deployment)
    });
    let finished_at = Utc::now();
    let update = state
        .store
        .update(job_id, |job| match result {
            Ok(deployment) => {
                job.status = "succeeded".to_owned();
                job.progress = 100;
                job.current_step = Some("completed".to_owned());
                job.error_message = None;
                job.deployment = Some(deployment);
                job.finished_at = Some(finished_at);
            }
            Err(error) => {
                job.status = "failed".to_owned();
                job.current_step = Some("failed_and_recovery_attempted".to_owned());
                job.error_message = Some(error.to_string().chars().take(1000).collect());
                job.finished_at = Some(finished_at);
            }
        })
        .await;
    if let Err(error) = update {
        tracing::error!(%job_id, %error, "failed to persist terminal updater job");
    }
    let mut active = state.store.active_job.lock().await;
    if *active == Some(job_id) {
        *active = None;
    }
}

fn validate_deployment_result(
    deployment: &DeploymentResult,
    target_version: &str,
) -> anyhow::Result<()> {
    if deployment.app_version != target_version
        || deployment.image_reference.trim().is_empty()
        || deployment.backup_reference.trim().is_empty()
        || deployment.schema_version < 0
        || deployment.image_digest.len() != 71
        || !deployment.image_digest.starts_with("sha256:")
        || !deployment.image_digest[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("executor returned invalid deployment evidence");
    }
    Ok(())
}

fn verify_request(
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: &[u8],
    token: &SecretString,
) -> Result<(), ApiError> {
    let expected_bearer = format!("Bearer {}", token.expose_secret());
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        != Some(expected_bearer.as_str())
    {
        return Err(ApiError::unauthorized());
    }
    let timestamp = headers
        .get(TIMESTAMP_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(ApiError::unauthorized)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::unauthorized())?
        .as_secs();
    if now.abs_diff(timestamp) > MAX_CLOCK_SKEW_SECONDS {
        return Err(ApiError::unauthorized());
    }
    let signature = headers
        .get(SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| hex::decode(value).ok())
        .ok_or_else(ApiError::unauthorized)?;
    let payload = signature_payload(timestamp, method, path, body);
    let mut mac = HmacSha256::new_from_slice(token.expose_secret().as_bytes())
        .map_err(|_| ApiError::unauthorized())?;
    mac.update(&payload);
    mac.verify_slice(&signature)
        .map_err(|_| ApiError::unauthorized())
}

fn signature_payload(timestamp: u64, method: &str, path: &str, body: &[u8]) -> Vec<u8> {
    let body_hash = hex::encode(Sha256::digest(body));
    format!("{timestamp}\n{method}\n{path}\n{body_hash}").into_bytes()
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "UNAUTHORIZED",
            message: "request authentication failed".to_owned(),
        }
    }

    fn validation(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "VALIDATION_ERROR",
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "CONFLICT",
            message: message.into(),
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND",
            message: "job not found".to_owned(),
        }
    }

    fn internal(error: anyhow::Error) -> Self {
        tracing::error!(%error, "Host Updater internal error");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            message: "internal server error".to_owned(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: ErrorDetail {
                    code: self.code,
                    message: self.message,
                },
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    #[derive(Default)]
    struct FakeExecutor {
        calls: AtomicUsize,
    }

    struct FailingExecutor;

    #[async_trait]
    impl Executor for FakeExecutor {
        async fn execute(
            &self,
            request: JobRequest,
            progress: mpsc::UnboundedSender<ProgressUpdate>,
        ) -> anyhow::Result<DeploymentResult> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            progress
                .send(ProgressUpdate {
                    progress: 50,
                    current_step: "health_check".to_owned(),
                })
                .unwrap();
            Ok(DeploymentResult {
                app_version: request.target_version,
                image_reference: "ghcr.io/example/app:v1.2.3".to_owned(),
                image_digest: format!("sha256:{}", "a".repeat(64)),
                schema_version: 9,
                backup_reference: "/backups/job/backup-manifest.json".to_owned(),
            })
        }
    }

    #[async_trait]
    impl Executor for FailingExecutor {
        async fn execute(
            &self,
            _request: JobRequest,
            progress: mpsc::UnboundedSender<ProgressUpdate>,
        ) -> anyhow::Result<DeploymentResult> {
            progress
                .send(ProgressUpdate {
                    progress: 75,
                    current_step: "candidate_health_check".to_owned(),
                })
                .unwrap();
            anyhow::bail!("simulated candidate failure after recovery")
        }
    }

    fn test_settings(root: &Path) -> Settings {
        Settings {
            listen_addr: "127.0.0.1:3199".parse().unwrap(),
            unix_socket: None,
            socket_gid: None,
            token: SecretString::from("test-token-with-at-least-thirty-two-bytes"),
            state_dir: root.to_path_buf(),
            executor_path: root.join("executor"),
            executor_config_path: root.join("config"),
            executor_sha256: None,
            job_timeout: Duration::from_secs(60),
        }
    }

    #[test]
    fn executor_progress_accepts_camel_case_wire_format() {
        let message: ExecutorMessage = serde_json::from_str(
            r#"{"type":"progress","progress":3,"currentStep":"validating_environment"}"#,
        )
        .unwrap();

        match message {
            ExecutorMessage::Progress {
                progress,
                current_step,
            } => {
                assert_eq!(progress, 3);
                assert_eq!(current_step, "validating_environment");
            }
            ExecutorMessage::Result { .. } => panic!("expected a progress message"),
        }
    }

    fn signed_request(
        token: &SecretString,
        method: &str,
        path: &str,
        body: Vec<u8>,
    ) -> Request<Body> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let payload = signature_payload(timestamp, method, path, &body);
        let mut mac = HmacSha256::new_from_slice(token.expose_secret().as_bytes()).unwrap();
        mac.update(&payload);
        Request::builder()
            .method(method)
            .uri(path)
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", token.expose_secret()),
            )
            .header(TIMESTAMP_HEADER, timestamp)
            .header(SIGNATURE_HEADER, hex::encode(mac.finalize().into_bytes()))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn rejects_unsigned_requests() {
        let root = tempfile::tempdir().unwrap();
        let settings = test_settings(root.path());
        let app = build_router(&settings, Arc::new(FakeExecutor::default()))
            .await
            .unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn executes_signed_job_and_persists_terminal_state() {
        let root = tempfile::tempdir().unwrap();
        let settings = test_settings(root.path());
        let executor = Arc::new(FakeExecutor::default());
        let app = build_router(&settings, executor.clone()).await.unwrap();
        let job_id = Uuid::new_v4();
        let body = serde_json::to_vec(&JobRequest {
            job_id,
            action: "upgrade".to_owned(),
            target_version: "1.2.3".to_owned(),
            confirm_destructive_migration: false,
        })
        .unwrap();
        let response = app
            .clone()
            .oneshot(signed_request(&settings.token, "POST", "/v1/jobs", body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let mut terminal = None;
        for _ in 0..50 {
            let path = format!("/v1/jobs/{job_id}");
            let response = app
                .clone()
                .oneshot(signed_request(&settings.token, "GET", &path, Vec::new()))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let job: JobView = serde_json::from_slice(&bytes).unwrap();
            if job.status == "succeeded" {
                terminal = Some(job);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let job = terminal.expect("job did not finish");
        assert_eq!(job.progress, 100);
        assert_eq!(job.deployment.unwrap().schema_version, 9);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
        let persisted = tokio::fs::read(root.path().join("jobs").join(format!("{job_id}.json")))
            .await
            .unwrap();
        let persisted: JobView = serde_json::from_slice(&persisted).unwrap();
        assert_eq!(persisted.status, "succeeded");
    }

    #[tokio::test]
    async fn persists_executor_failure_and_releases_the_active_job_slot() {
        let root = tempfile::tempdir().unwrap();
        let settings = test_settings(root.path());
        let app = build_router(&settings, Arc::new(FailingExecutor))
            .await
            .unwrap();
        let first_id = Uuid::new_v4();
        let body = serde_json::to_vec(&JobRequest {
            job_id: first_id,
            action: "upgrade".to_owned(),
            target_version: "1.2.3".to_owned(),
            confirm_destructive_migration: false,
        })
        .unwrap();
        let response = app
            .clone()
            .oneshot(signed_request(&settings.token, "POST", "/v1/jobs", body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        for _ in 0..50 {
            let path = format!("/v1/jobs/{first_id}");
            let response = app
                .clone()
                .oneshot(signed_request(&settings.token, "GET", &path, Vec::new()))
                .await
                .unwrap();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let job: JobView = serde_json::from_slice(&bytes).unwrap();
            if job.status == "failed" {
                assert_eq!(
                    job.current_step.as_deref(),
                    Some("failed_and_recovery_attempted")
                );
                assert!(
                    job.error_message
                        .unwrap()
                        .contains("simulated candidate failure")
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let second = JobRequest {
            job_id: Uuid::new_v4(),
            action: "upgrade".to_owned(),
            target_version: "1.2.4".to_owned(),
            confirm_destructive_migration: false,
        };
        let response = app
            .oneshot(signed_request(
                &settings.token,
                "POST",
                "/v1/jobs",
                serde_json::to_vec(&second).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[test]
    fn rejects_incomplete_deployment_evidence() {
        let deployment = DeploymentResult {
            app_version: "1.2.3".to_owned(),
            image_reference: "ghcr.io/example/app:v1.2.3".to_owned(),
            image_digest: "sha256:abc".to_owned(),
            schema_version: 9,
            backup_reference: "/backups/job/backup-manifest.json".to_owned(),
        };
        assert!(validate_deployment_result(&deployment, "1.2.3").is_err());
    }

    #[test]
    fn request_signature_matches_the_web_protocol_test_vector() {
        let payload = signature_payload(1_700_000_000, "POST", "/v1/jobs", br#"{"a":1}"#);
        let mut mac = HmacSha256::new_from_slice(
            SecretString::from("0123456789abcdef0123456789abcdef")
                .expose_secret()
                .as_bytes(),
        )
        .unwrap();
        mac.update(&payload);
        assert_eq!(
            hex::encode(mac.finalize().into_bytes()),
            "cabf9479aa68b213187b0f53b471c48bfcc831e4a568ba7db39bb769768aa7b7"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn accepts_signed_jobs_over_a_real_unix_socket() {
        use tokio::net::UnixListener;

        let root = tempfile::tempdir().unwrap();
        let settings = test_settings(root.path());
        let app = build_router(&settings, Arc::new(FakeExecutor::default()))
            .await
            .unwrap();
        let socket = root.path().join("updater.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder()
            .unix_socket(socket)
            .build()
            .unwrap();
        let request = JobRequest {
            job_id: Uuid::new_v4(),
            action: "upgrade".to_owned(),
            target_version: "1.2.3".to_owned(),
            confirm_destructive_migration: false,
        };
        let body = serde_json::to_vec(&request).unwrap();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let payload = signature_payload(timestamp, "POST", "/v1/jobs", &body);
        let mut mac =
            HmacSha256::new_from_slice(settings.token.expose_secret().as_bytes()).unwrap();
        mac.update(&payload);
        let response = client
            .post("http://localhost/v1/jobs")
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", settings.token.expose_secret()),
            )
            .header(TIMESTAMP_HEADER, timestamp)
            .header(SIGNATURE_HEADER, hex::encode(mac.finalize().into_bytes()))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        server.abort();
    }
}
