use std::{env, fs, net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::{Context, bail};
use secrecy::SecretString;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageDriver {
    Local,
    S3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskExecutionMode {
    Inline,
    Redis,
}

impl FromStr for TaskExecutionMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "inline" => Ok(Self::Inline),
            "redis" => Ok(Self::Redis),
            _ => bail!("TASK_EXECUTION_MODE must be inline or redis"),
        }
    }
}

impl FromStr for StorageDriver {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "s3" => Ok(Self::S3),
            _ => bail!("STORAGE_DRIVER must be local or s3"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub app_name: String,
    pub app_env: String,
    pub app_version: String,
    pub app_image_reference: String,
    pub listen_addr: SocketAddr,
    pub database_url: String,
    pub database_max_connections: u32,
    pub static_dir: PathBuf,
    pub bootstrap_admin_enabled: bool,
    pub bootstrap_admin_username: String,
    pub bootstrap_admin_password: SecretString,
    pub bootstrap_admin_force_password_change: bool,
    pub session_secret: SecretString,
    pub session_cookie_secure: bool,
    pub credential_master_key: SecretString,
    pub session_ttl_seconds: i64,
    pub storage_driver: StorageDriver,
    pub storage_local_path: PathBuf,
    pub storage_s3_enabled: bool,
    pub storage_s3_bucket: Option<String>,
    pub storage_s3_region: String,
    pub storage_s3_endpoint: Option<String>,
    pub storage_s3_prefix: String,
    pub storage_s3_access_key_id: Option<SecretString>,
    pub storage_s3_secret_access_key: Option<SecretString>,
    pub storage_s3_force_path_style: bool,
    pub storage_consistency_scan_enabled: bool,
    pub storage_consistency_scan_interval_seconds: u64,
    pub storage_orphan_grace_seconds: u64,
    pub http_ca_cert_file: Option<PathBuf>,
    pub request_timeout_seconds: u64,
    pub connect_timeout_seconds: u64,
    pub max_upload_size_mb: usize,
    pub max_provider_image_size_mb: usize,
    pub task_stream_heartbeat_seconds: u64,
    pub task_execution_mode: TaskExecutionMode,
    pub redis_url: Option<SecretString>,
    pub task_queue_key: String,
    pub task_max_retries: i32,
    pub task_retry_delay_seconds: u64,
    pub rate_limit_enabled: bool,
    pub rate_limit_window_seconds: u64,
    pub rate_limit_ip_requests: u32,
    pub rate_limit_session_requests: u32,
    pub rate_limit_user_requests: u32,
    pub update_channel: String,
    pub update_manifest_url: Option<String>,
    pub update_manifest_token: Option<SecretString>,
    pub host_updater_url: Option<String>,
    pub host_updater_socket: Option<PathBuf>,
    pub host_updater_token: Option<SecretString>,
    pub keep_previous_releases: i64,
    pub allow_custom_base_url: bool,
    pub allow_private_provider_hosts: bool,
    pub allowed_provider_hosts: Vec<String>,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        let storage_driver = env_or("STORAGE_DRIVER", "local").parse()?;
        let host_updater_token = optional_secret("HOST_UPDATER_TOKEN", "HOST_UPDATER_TOKEN_FILE")?;
        let settings = Self {
            app_name: env_or("APP_NAME", "AI Image Studio"),
            app_env: env_or("APP_ENV", "development"),
            app_version: env_or(
                "IMAGE_APP_VERSION",
                &env_or("APP_VERSION", env!("CARGO_PKG_VERSION")),
            ),
            app_image_reference: env_or(
                "IMAGE_APP_REFERENCE",
                &env_or("APP_IMAGE_REFERENCE", "ai-image-studio:local"),
            ),
            listen_addr: env_or("LISTEN_ADDR", "0.0.0.0:3000")
                .parse()
                .context("invalid LISTEN_ADDR")?,
            database_url: required("DATABASE_URL")?,
            database_max_connections: parse_or("DATABASE_MAX_CONNECTIONS", 10)?,
            static_dir: PathBuf::from(env_or("STATIC_DIR", "./static")),
            bootstrap_admin_enabled: parse_or("BOOTSTRAP_ADMIN_ENABLED", true)?,
            bootstrap_admin_username: env_or("BOOTSTRAP_ADMIN_USERNAME", "admin"),
            bootstrap_admin_password: SecretString::from(env_or(
                "BOOTSTRAP_ADMIN_PASSWORD",
                "123456",
            )),
            bootstrap_admin_force_password_change: parse_or(
                "BOOTSTRAP_ADMIN_FORCE_PASSWORD_CHANGE",
                true,
            )?,
            session_secret: SecretString::from(required("SESSION_SECRET")?),
            session_cookie_secure: parse_or("SESSION_COOKIE_SECURE", false)?,
            credential_master_key: SecretString::from(required("CREDENTIAL_MASTER_KEY")?),
            session_ttl_seconds: parse_or("SESSION_TTL_SECONDS", 86_400)?,
            storage_driver,
            storage_local_path: PathBuf::from(env_or("STORAGE_LOCAL_PATH", "./data/images")),
            storage_s3_enabled: parse_or("STORAGE_S3_ENABLED", false)?,
            storage_s3_bucket: optional("STORAGE_S3_BUCKET"),
            storage_s3_region: env_or("STORAGE_S3_REGION", "auto"),
            storage_s3_endpoint: optional("STORAGE_S3_ENDPOINT"),
            storage_s3_prefix: env_or("STORAGE_S3_PREFIX", "ai-image-studio/"),
            storage_s3_access_key_id: optional("STORAGE_S3_ACCESS_KEY_ID").map(SecretString::from),
            storage_s3_secret_access_key: optional("STORAGE_S3_SECRET_ACCESS_KEY")
                .map(SecretString::from),
            storage_s3_force_path_style: parse_or("STORAGE_S3_FORCE_PATH_STYLE", false)?,
            storage_consistency_scan_enabled: parse_or("STORAGE_CONSISTENCY_SCAN_ENABLED", true)?,
            storage_consistency_scan_interval_seconds: parse_or(
                "STORAGE_CONSISTENCY_SCAN_INTERVAL_SECONDS",
                86_400,
            )?,
            storage_orphan_grace_seconds: parse_or("STORAGE_ORPHAN_GRACE_SECONDS", 86_400)?,
            http_ca_cert_file: optional("HTTP_CA_CERT_FILE").map(PathBuf::from),
            request_timeout_seconds: parse_or("REQUEST_TIMEOUT_SECONDS", 600)?,
            connect_timeout_seconds: parse_or("CONNECT_TIMEOUT_SECONDS", 15)?,
            max_upload_size_mb: parse_or("MAX_UPLOAD_SIZE_MB", 25)?,
            max_provider_image_size_mb: parse_or("MAX_PROVIDER_IMAGE_SIZE_MB", 50)?,
            task_stream_heartbeat_seconds: parse_or("TASK_STREAM_HEARTBEAT_SECONDS", 15)?,
            task_execution_mode: env_or("TASK_EXECUTION_MODE", "inline").parse()?,
            redis_url: optional("REDIS_URL").map(SecretString::from),
            task_queue_key: env_or("TASK_QUEUE_KEY", "ai-image-studio:tasks"),
            task_max_retries: parse_or("TASK_MAX_RETRIES", 2)?,
            task_retry_delay_seconds: parse_or("TASK_RETRY_DELAY_SECONDS", 3)?,
            rate_limit_enabled: parse_or("RATE_LIMIT_ENABLED", true)?,
            rate_limit_window_seconds: parse_or("RATE_LIMIT_WINDOW_SECONDS", 60)?,
            rate_limit_ip_requests: parse_or("RATE_LIMIT_IP_REQUESTS", 240)?,
            rate_limit_session_requests: parse_or("RATE_LIMIT_SESSION_REQUESTS", 180)?,
            rate_limit_user_requests: parse_or("RATE_LIMIT_USER_REQUESTS", 120)?,
            update_channel: env_or("UPDATE_CHANNEL", "stable"),
            update_manifest_url: optional("UPDATE_MANIFEST_URL"),
            update_manifest_token: optional("UPDATE_MANIFEST_TOKEN").map(SecretString::from),
            host_updater_url: optional("HOST_UPDATER_URL"),
            host_updater_socket: optional("HOST_UPDATER_SOCKET").map(PathBuf::from),
            host_updater_token,
            keep_previous_releases: parse_or("KEEP_PREVIOUS_RELEASES", 3)?,
            allow_custom_base_url: parse_or("ALLOW_CUSTOM_BASE_URL", true)?,
            allow_private_provider_hosts: parse_or("ALLOW_PRIVATE_PROVIDER_HOSTS", false)?,
            allowed_provider_hosts: env_or(
                "ALLOWED_PROVIDER_HOSTS",
                "api.openai.com,api.codechicken.top,generativelanguage.googleapis.com,api.x.ai",
            )
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .collect(),
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn is_production(&self) -> bool {
        self.app_env.eq_ignore_ascii_case("production")
    }

    pub fn healthcheck_addr(&self) -> SocketAddr {
        if self.listen_addr.ip().is_unspecified() {
            SocketAddr::from(([127, 0, 0, 1], self.listen_addr.port()))
        } else {
            self.listen_addr
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.session_secret.expose_secret().len() < 32 {
            bail!("SESSION_SECRET must contain at least 32 characters");
        }
        let decoded = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            self.credential_master_key.expose_secret(),
        )
        .context("CREDENTIAL_MASTER_KEY must be base64")?;
        if decoded.len() != 32 {
            bail!("CREDENTIAL_MASTER_KEY must decode to exactly 32 bytes");
        }
        if self.storage_driver == StorageDriver::S3 {
            if !self.storage_s3_enabled {
                bail!("STORAGE_S3_ENABLED must be true when STORAGE_DRIVER=s3");
            }
            if self.storage_s3_bucket.is_none()
                || self.storage_s3_access_key_id.is_none()
                || self.storage_s3_secret_access_key.is_none()
            {
                bail!("S3 bucket and credentials are required when STORAGE_DRIVER=s3");
            }
        }
        if self.storage_consistency_scan_interval_seconds == 0
            || self.storage_orphan_grace_seconds == 0
        {
            bail!("storage consistency interval and orphan grace must be greater than zero");
        }
        if self
            .http_ca_cert_file
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        {
            bail!("HTTP_CA_CERT_FILE must be an absolute path");
        }
        if self.task_execution_mode == TaskExecutionMode::Redis && self.redis_url.is_none() {
            bail!("REDIS_URL is required when TASK_EXECUTION_MODE=redis");
        }
        if self.task_queue_key.trim().is_empty() {
            bail!("TASK_QUEUE_KEY must not be empty");
        }
        if !(0..=10).contains(&self.task_max_retries) {
            bail!("TASK_MAX_RETRIES must be between 0 and 10");
        }
        if self.rate_limit_window_seconds == 0
            || self.rate_limit_ip_requests == 0
            || self.rate_limit_session_requests == 0
            || self.rate_limit_user_requests == 0
        {
            bail!("rate limit window and request limits must be greater than zero");
        }
        if !matches!(self.update_channel.as_str(), "stable" | "beta" | "nightly") {
            bail!("UPDATE_CHANNEL must be stable, beta or nightly");
        }
        if !(1..=3).contains(&self.keep_previous_releases) {
            bail!("KEEP_PREVIOUS_RELEASES must be between 1 and 3");
        }
        let updater_endpoint_configured =
            self.host_updater_url.is_some() || self.host_updater_socket.is_some();
        if updater_endpoint_configured != self.host_updater_token.is_some() {
            bail!("HOST_UPDATER_TOKEN is required with HOST_UPDATER_URL or HOST_UPDATER_SOCKET");
        }
        if self
            .host_updater_socket
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        {
            bail!("HOST_UPDATER_SOCKET must be an absolute path");
        }
        #[cfg(not(unix))]
        if self.host_updater_socket.is_some() {
            bail!("HOST_UPDATER_SOCKET is only supported on Unix");
        }
        if self
            .host_updater_token
            .as_ref()
            .is_some_and(|token| token.expose_secret().len() < 32)
        {
            bail!("HOST_UPDATER_TOKEN must contain at least 32 bytes");
        }
        if self.is_production() && self.bootstrap_admin_password.expose_secret() == "123456" {
            tracing::warn!("default bootstrap administrator password is still configured");
        }
        if self.is_production() && !self.session_cookie_secure {
            tracing::warn!(
                "SESSION_COOKIE_SECURE is false; enable it when the application is served over HTTPS"
            );
        }
        Ok(())
    }
}

use secrecy::ExposeSecret;

fn env_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn optional(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn required(name: &str) -> anyhow::Result<String> {
    optional(name).with_context(|| format!("{name} is required"))
}

fn optional_secret(value_name: &str, file_name: &str) -> anyhow::Result<Option<SecretString>> {
    if let Some(value) = optional(value_name) {
        if optional(file_name).is_some() {
            bail!("{value_name} and {file_name} cannot both be configured");
        }
        return Ok(Some(SecretString::from(value)));
    }
    let Some(path) = optional(file_name).map(PathBuf::from) else {
        return Ok(None);
    };
    if !path.is_absolute() {
        bail!("{file_name} must be an absolute path");
    }
    let value = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {file_name} from {}", path.display()))?;
    let value = value.trim();
    if value.is_empty() {
        bail!("{file_name} must not be empty");
    }
    Ok(Some(SecretString::from(value.to_owned())))
}

fn parse_or<T>(name: &str, default: T) -> anyhow::Result<T>
where
    T: FromStr + ToString,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .parse()
        .with_context(|| format!("invalid {name}"))
}
