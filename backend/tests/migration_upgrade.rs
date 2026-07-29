use std::borrow::Cow;

use anyhow::Context;
use sqlx::{PgPool, postgres::PgPoolOptions};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[tokio::test]
async fn upgrades_three_predecessor_schemas_without_losing_business_rows() -> anyhow::Result<()> {
    if std::env::var("TEST_MIGRATION_UPGRADE").as_deref() != Ok("1") {
        return Ok(());
    }
    let base_url = std::env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&base_url)
        .await?;
    for predecessor in [10_i64, 11, 12] {
        exercise_upgrade(&admin, &base_url, predecessor).await?;
    }
    Ok(())
}

async fn exercise_upgrade(admin: &PgPool, base_url: &str, predecessor: i64) -> anyhow::Result<()> {
    let database_name = format!(
        "migration_upgrade_{}_{}",
        predecessor,
        uuid::Uuid::new_v4().simple()
    );
    sqlx::query(&format!(r#"CREATE DATABASE "{database_name}""#))
        .execute(admin)
        .await?;

    let result = run_upgrade_case(base_url, &database_name, predecessor).await;
    sqlx::query(&format!(r#"DROP DATABASE "{database_name}" WITH (FORCE)"#))
        .execute(admin)
        .await?;
    result
}

async fn run_upgrade_case(
    base_url: &str,
    database_name: &str,
    predecessor: i64,
) -> anyhow::Result<()> {
    let mut url = url::Url::parse(base_url)?;
    url.set_path(&format!("/{database_name}"));
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(url.as_str())
        .await?;
    let old_migrator = sqlx::migrate::Migrator {
        migrations: Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|migration| migration.version <= predecessor)
                .cloned()
                .collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    old_migrator.run(&pool).await?;

    let user_id = uuid::Uuid::new_v4();
    let deployment_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, username, role, status) VALUES ($1, $2, 'user', 'active')")
        .bind(user_id)
        .bind(format!("migration_user_{predecessor}"))
        .execute(&pool)
        .await?;
    sqlx::query(
        r#"
        INSERT INTO deployment_history (
            id, app_version, image_reference, image_digest,
            schema_version, deployment_status
        ) VALUES ($1, '0.0.1', 'example/image:v0.0.1', $2, $3, 'superseded')
        "#,
    )
    .bind(deployment_id)
    .bind(format!("sha256:{}", "a".repeat(64)))
    .bind(predecessor)
    .execute(&pool)
    .await?;

    MIGRATOR.run(&pool).await?;
    let applied =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::BIGINT FROM _sqlx_migrations WHERE success")
            .fetch_one(&pool)
            .await?;
    let expected_migrations = MIGRATOR.iter().count() as i64;
    anyhow::ensure!(
        applied == expected_migrations,
        "expected {expected_migrations} applied migrations, got {applied}"
    );
    let user_exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
            .bind(user_id)
            .fetch_one(&pool)
            .await?;
    anyhow::ensure!(
        user_exists,
        "user row was lost while upgrading from {predecessor}"
    );
    let source_job_id = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT source_job_id FROM deployment_history WHERE id = $1",
    )
    .bind(deployment_id)
    .fetch_one(&pool)
    .await?;
    anyhow::ensure!(source_job_id.is_none());
    let consistency_table = sqlx::query_scalar::<_, bool>(
        "SELECT to_regclass('public.storage_consistency_runs') IS NOT NULL",
    )
    .fetch_one(&pool)
    .await?;
    anyhow::ensure!(consistency_table);
    let public_templates = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::BIGINT FROM prompt_templates WHERE owner_id IS NULL AND is_public",
    )
    .fetch_one(&pool)
    .await?;
    anyhow::ensure!(public_templates == 4);
    let digital_internet_template = sqlx::query_scalar::<_, String>(
        "SELECT prompt FROM prompt_templates WHERE id = '10000000-0000-4000-8000-000000000004'",
    )
    .fetch_one(&pool)
    .await?;
    anyhow::ensure!(digital_internet_template.contains("科幻粒子"));
    let message_status_constraint = sqlx::query_scalar::<_, String>(
        r#"
        SELECT pg_get_constraintdef(oid)
        FROM pg_constraint
        WHERE conrelid = 'conversation_messages'::regclass
          AND conname = 'conversation_messages_status_check'
        "#,
    )
    .fetch_one(&pool)
    .await?;
    anyhow::ensure!(
        message_status_constraint.contains("cancelled"),
        "cancelled conversation message status is missing after upgrade"
    );
    pool.close().await;
    Ok(())
}
