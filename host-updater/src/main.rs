use std::sync::Arc;

use ai_image_studio_host_updater::{ProcessExecutor, Settings, build_router};
use anyhow::Context;
use tokio::net::TcpListener;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ai_image_studio_host_updater=info")),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    let settings = Settings::from_env()?;
    let executor = Arc::new(ProcessExecutor::from_settings(&settings).await?);
    let app = build_router(&settings, executor).await?;
    #[cfg(unix)]
    if let Some(socket_path) = settings.unix_socket.as_deref() {
        return serve_unix_socket(
            socket_path,
            settings.socket_gid.context("socket GID is required")?,
            app,
        )
        .await;
    }
    let listener = TcpListener::bind(settings.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", settings.listen_addr))?;
    tracing::info!(address = %settings.listen_addr, "Host Updater listening on loopback");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

#[cfg(unix)]
async fn serve_unix_socket(
    socket_path: &std::path::Path,
    socket_gid: u32,
    app: axum::Router,
) -> anyhow::Result<()> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt, chown};

    use tokio::net::UnixListener;

    let parent = socket_path
        .parent()
        .context("Unix socket has no parent directory")?;
    tokio::fs::create_dir_all(parent).await?;
    tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o770)).await?;
    chown(parent, None, Some(socket_gid))?;
    if tokio::fs::try_exists(socket_path).await? {
        let metadata = tokio::fs::symlink_metadata(socket_path).await?;
        if !metadata.file_type().is_socket() {
            anyhow::bail!(
                "refusing to replace non-socket path {}",
                socket_path.display()
            );
        }
        tokio::fs::remove_file(socket_path).await?;
    }
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("failed to bind {}", socket_path.display()))?;
    tokio::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660)).await?;
    chown(socket_path, None, Some(socket_gid))?;
    tracing::info!(path = %socket_path.display(), "Host Updater listening on Unix socket");
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    let _ = tokio::fs::remove_file(socket_path).await;
    result.map_err(Into::into)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
