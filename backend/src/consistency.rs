use std::{collections::HashSet, time::Duration};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{AppError, AppResult},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/storage/consistency", get(list_runs))
        .route("/api/v1/admin/storage/consistency/scan", post(scan_now))
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ConsistencyRun {
    id: Uuid,
    status: String,
    delete_orphans: bool,
    grace_seconds: i64,
    database_assets: i64,
    storage_objects: i64,
    missing_objects: i64,
    orphan_objects: i64,
    eligible_orphans: i64,
    deleted_orphans: i64,
    error_message: Option<String>,
    requested_by: Option<Uuid>,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

async fn list_runs(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<Json<Vec<ConsistencyRun>>> {
    current.require_password_changed()?;
    current.require_admin()?;
    let runs = sqlx::query_as::<_, ConsistencyRun>(
        r#"
        SELECT id, status, delete_orphans, grace_seconds, database_assets,
               storage_objects, missing_objects, orphan_objects, eligible_orphans,
               deleted_orphans, error_message, requested_by, started_at, finished_at
        FROM storage_consistency_runs
        ORDER BY started_at DESC
        LIMIT 20
        "#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(runs))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScanRequest {
    #[serde(default)]
    delete_orphans: bool,
}

async fn scan_now(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<ScanRequest>,
) -> AppResult<Json<ConsistencyRun>> {
    current.require_password_changed()?;
    current.require_admin()?;
    Ok(Json(
        run_scan(&state, Some(current.id), input.delete_orphans).await?,
    ))
}

pub async fn run_periodic(state: AppState) {
    if !state.settings.storage_consistency_scan_enabled {
        return;
    }
    let interval = Duration::from_secs(state.settings.storage_consistency_scan_interval_seconds);
    loop {
        tokio::time::sleep(interval).await;
        if let Err(error) = run_scan(&state, None, true).await {
            tracing::error!(error = %error, "scheduled storage consistency scan failed");
        }
    }
}

pub async fn run_scan(
    state: &AppState,
    requested_by: Option<Uuid>,
    delete_orphans: bool,
) -> AppResult<ConsistencyRun> {
    let _guard = state.maintenance_lock.try_lock().map_err(|_| {
        AppError::Conflict("a storage consistency scan is already running".to_owned())
    })?;
    let grace_seconds = i64::try_from(state.settings.storage_orphan_grace_seconds)
        .map_err(|_| AppError::Validation("storage orphan grace is too large".to_owned()))?;
    let run_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO storage_consistency_runs (delete_orphans, grace_seconds, requested_by)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(delete_orphans)
    .bind(grace_seconds)
    .bind(requested_by)
    .fetch_one(&state.db)
    .await?;

    match scan_storage(state, delete_orphans, grace_seconds).await {
        Ok(counts) => {
            sqlx::query(
                r#"
                UPDATE storage_consistency_runs
                SET status = 'succeeded', database_assets = $1, storage_objects = $2,
                    missing_objects = $3, orphan_objects = $4, eligible_orphans = $5,
                    deleted_orphans = $6, finished_at = NOW()
                WHERE id = $7
                "#,
            )
            .bind(counts.database_assets)
            .bind(counts.storage_objects)
            .bind(counts.missing_objects)
            .bind(counts.orphan_objects)
            .bind(counts.eligible_orphans)
            .bind(counts.deleted_orphans)
            .bind(run_id)
            .execute(&state.db)
            .await?;
        }
        Err(error) => {
            let summary = error.to_string().chars().take(2000).collect::<String>();
            sqlx::query(
                "UPDATE storage_consistency_runs SET status = 'failed', error_message = $1, finished_at = NOW() WHERE id = $2",
            )
            .bind(&summary)
            .bind(run_id)
            .execute(&state.db)
            .await?;
            return Err(AppError::Internal(error));
        }
    }
    find_run(state, run_id).await
}

#[derive(FromRow)]
struct AssetLocation {
    storage_driver: String,
    storage_container: String,
    storage_key: String,
}

struct ScanCounts {
    database_assets: i64,
    storage_objects: i64,
    missing_objects: i64,
    orphan_objects: i64,
    eligible_orphans: i64,
    deleted_orphans: i64,
}

async fn scan_storage(
    state: &AppState,
    delete_orphans: bool,
    grace_seconds: i64,
) -> anyhow::Result<ScanCounts> {
    let assets = sqlx::query_as::<_, AssetLocation>(
        "SELECT storage_driver, storage_container, storage_key FROM image_assets",
    )
    .fetch_all(&state.db)
    .await?;
    let database_keys: HashSet<_> = assets
        .iter()
        .map(|asset| {
            (
                asset.storage_driver.clone(),
                asset.storage_container.clone(),
                asset.storage_key.clone(),
            )
        })
        .collect();
    let objects: Vec<_> = state
        .storage
        .list_all()
        .await?
        .into_iter()
        .filter(|object| is_managed_key(&object.key))
        .collect();
    let object_keys: HashSet<_> = objects
        .iter()
        .map(|object| {
            (
                object.driver.to_owned(),
                object.container.clone(),
                object.key.clone(),
            )
        })
        .collect();
    let missing_objects = assets
        .iter()
        .filter(|asset| {
            state
                .storage
                .can_scan(&asset.storage_driver, &asset.storage_container)
                && !object_keys.contains(&(
                    asset.storage_driver.clone(),
                    asset.storage_container.clone(),
                    asset.storage_key.clone(),
                ))
        })
        .count() as i64;
    let cutoff = Utc::now()
        .checked_sub_signed(chrono::Duration::seconds(grace_seconds))
        .context("storage orphan grace is outside the supported range")?;
    let mut orphan_objects = 0_i64;
    let mut eligible_orphans = 0_i64;
    let mut deleted_orphans = 0_i64;
    for object in &objects {
        let key = (
            object.driver.to_owned(),
            object.container.clone(),
            object.key.clone(),
        );
        if database_keys.contains(&key) {
            continue;
        }
        orphan_objects += 1;
        if object.last_modified > cutoff {
            continue;
        }
        eligible_orphans += 1;
        if delete_orphans {
            state
                .storage
                .delete(object.driver, &object.container, &object.key)
                .await
                .with_context(|| format!("failed to delete orphan object {}", object.key))?;
            deleted_orphans += 1;
        }
    }
    Ok(ScanCounts {
        database_assets: assets.len() as i64,
        storage_objects: objects.len() as i64,
        missing_objects,
        orphan_objects,
        eligible_orphans,
        deleted_orphans,
    })
}

async fn find_run(state: &AppState, run_id: Uuid) -> AppResult<ConsistencyRun> {
    sqlx::query_as::<_, ConsistencyRun>(
        r#"
        SELECT id, status, delete_orphans, grace_seconds, database_assets,
               storage_objects, missing_objects, orphan_objects, eligible_orphans,
               deleted_orphans, error_message, requested_by, started_at, finished_at
        FROM storage_consistency_runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::Database)
}

fn is_managed_key(key: &str) -> bool {
    let parts: Vec<_> = key.split('/').collect();
    if parts.len() != 4
        || parts[0].len() != 4
        || !parts[0].bytes().all(|byte| byte.is_ascii_digit())
        || parts[1]
            .parse::<u8>()
            .map_or(true, |month| !(1..=12).contains(&month))
        || Uuid::parse_str(parts[2]).is_err()
    {
        return false;
    }
    let Some((asset_id, extension)) = parts[3].rsplit_once('.') else {
        return false;
    };
    Uuid::parse_str(asset_id).is_ok() && matches!(extension, "png" | "jpg" | "webp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_recognizes_platform_owned_asset_keys() {
        let owner = Uuid::new_v4();
        let asset = Uuid::new_v4();
        assert!(is_managed_key(&format!("2026/07/{owner}/{asset}.png")));
        assert!(!is_managed_key("unrelated/customer-file.png"));
        assert!(!is_managed_key(&format!("2026/13/{owner}/{asset}.png")));
        assert!(!is_managed_key(&format!("2026/07/{owner}/{asset}.txt")));
    }
}
