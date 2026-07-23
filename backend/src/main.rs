use std::process::ExitCode;

use ai_image_studio::{AppState, Settings, build_router};
use anyhow::Context;
use clap::{Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "ai-image-studio", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve,
    Worker,
    Migrate,
    Healthcheck,
}

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();
    init_tracing();

    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "application terminated");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let settings = Settings::from_env()?;

    if matches!(cli.command, Some(Command::Healthcheck)) {
        let response = reqwest::get(format!(
            "http://{}/api/v1/health",
            settings.healthcheck_addr()
        ))
        .await
        .context("healthcheck request failed")?;
        anyhow::ensure!(
            response.status().is_success(),
            "healthcheck returned {}",
            response.status()
        );
        return Ok(());
    }

    let pool = PgPoolOptions::new()
        .max_connections(settings.database_max_connections)
        .connect(&settings.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    if matches!(cli.command, Some(Command::Migrate)) {
        sqlx::migrate!("./migrations").run(&pool).await?;
        return Ok(());
    }

    sqlx::migrate!("./migrations").run(&pool).await?;
    let state = AppState::initialize(settings.clone(), pool).await?;
    if matches!(cli.command, Some(Command::Worker)) {
        ai_image_studio::tasks::run_worker(state).await?;
        return Ok(());
    }
    tokio::spawn(ai_image_studio::consistency::run_periodic(state.clone()));
    let listener = TcpListener::bind(&settings.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", settings.listen_addr))?;
    info!(address = %settings.listen_addr, "AI Image Studio started");
    axum::serve(
        listener,
        build_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("ai_image_studio=info,tower_http=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();
}
