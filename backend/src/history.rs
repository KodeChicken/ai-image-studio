use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
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
        .route("/api/v1/history", get(list))
        .route("/api/v1/history/{id}", axum::routing::delete(remove))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryQuery {
    conversation_id: Option<Uuid>,
    provider_id: Option<Uuid>,
    model_id: Option<Uuid>,
    created_from: Option<chrono::DateTime<chrono::Utc>>,
    created_to: Option<chrono::DateTime<chrono::Utc>>,
    width: Option<i32>,
    height: Option<i32>,
    #[serde(default = "default_limit")]
    limit: i64,
}

#[derive(Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct HistoryItem {
    task_id: Uuid,
    conversation_id: Uuid,
    conversation_title: String,
    asset_id: Uuid,
    content_url: String,
    model_id: Uuid,
    model_name: String,
    provider_id: Uuid,
    provider_name: String,
    prompt: String,
    mime_type: String,
    width: Option<i32>,
    height: Option<i32>,
    file_size_bytes: i64,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<HistoryQuery>,
) -> AppResult<Json<Vec<HistoryItem>>> {
    current.require_password_changed()?;
    validate_history_query(&query)?;
    let limit = query.limit.clamp(1, 100);
    let mut items = sqlx::query_as::<_, HistoryItem>(
        r#"
        SELECT t.id AS task_id, t.conversation_id, c.title AS conversation_title,
               a.id AS asset_id, ''::TEXT AS content_url,
               t.model_id, m.display_name AS model_name,
               t.provider_id, p.display_name AS provider_name,
               t.prompt, a.mime_type, a.width, a.height, a.file_size_bytes, r.created_at
        FROM image_results r
        JOIN image_tasks t ON t.id = r.task_id
        JOIN image_assets a ON a.id = r.asset_id
        JOIN conversations c ON c.id = t.conversation_id
        JOIN models m ON m.id = t.model_id
        JOIN providers p ON p.id = t.provider_id
        WHERE t.user_id = $1 AND t.status = 'succeeded'
          AND ($2::UUID IS NULL OR t.conversation_id = $2)
          AND ($3::UUID IS NULL OR t.provider_id = $3)
          AND ($4::UUID IS NULL OR t.model_id = $4)
          AND ($5::TIMESTAMPTZ IS NULL OR r.created_at >= $5)
          AND ($6::TIMESTAMPTZ IS NULL OR r.created_at < $6)
          AND ($7::INTEGER IS NULL OR a.width = $7)
          AND ($8::INTEGER IS NULL OR a.height = $8)
        ORDER BY r.created_at DESC
        LIMIT $9
        "#,
    )
    .bind(current.id)
    .bind(query.conversation_id)
    .bind(query.provider_id)
    .bind(query.model_id)
    .bind(query.created_from)
    .bind(query.created_to)
    .bind(query.width)
    .bind(query.height)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    for item in &mut items {
        item.content_url = format!("/api/v1/image-assets/{}/content", item.asset_id);
    }
    Ok(Json(items))
}

fn validate_history_query(query: &HistoryQuery) -> AppResult<()> {
    if let (Some(from), Some(to)) = (query.created_from, query.created_to)
        && from >= to
    {
        return Err(AppError::Validation(
            "history createdFrom must be before createdTo".to_owned(),
        ));
    }
    match (query.width, query.height) {
        (None, None) => Ok(()),
        (Some(width), Some(height))
            if (1..=100_000).contains(&width) && (1..=100_000).contains(&height) =>
        {
            Ok(())
        }
        (Some(_), Some(_)) => Err(AppError::Validation(
            "history width and height must be between 1 and 100000".to_owned(),
        )),
        _ => Err(AppError::Validation(
            "history width and height must be provided together".to_owned(),
        )),
    }
}

#[derive(FromRow)]
struct TaskMessages {
    conversation_id: Uuid,
    user_message_id: Uuid,
    assistant_message_id: Uuid,
}

async fn remove(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(task_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    current.require_password_changed()?;
    let mut tx = state.db.begin().await?;
    let task = sqlx::query_as::<_, TaskMessages>(
        r#"
        SELECT conversation_id, user_message_id, assistant_message_id
        FROM image_tasks
        WHERE id = $1 AND user_id = $2
        FOR UPDATE
        "#,
    )
    .bind(task_id)
    .bind(current.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let message_ids = [task.user_message_id, task.assistant_message_id];
    let candidate_assets = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT asset_id FROM task_input_images WHERE task_id = $1
        UNION
        SELECT asset_id FROM image_results WHERE task_id = $1
        UNION
        SELECT asset_id FROM message_image_assets WHERE message_id = ANY($2)
        "#,
    )
    .bind(task_id)
    .bind(&message_ids[..])
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM image_tasks WHERE id = $1 AND user_id = $2")
        .bind(task_id)
        .bind(current.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = $1 AND id = ANY($2)")
        .bind(task.conversation_id)
        .bind(&message_ids[..])
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        UPDATE conversations c
        SET last_message_at = COALESCE(
                (SELECT MAX(cm.created_at) FROM conversation_messages cm WHERE cm.conversation_id = c.id),
                c.created_at
            ),
            updated_at = NOW()
        WHERE c.id = $1 AND c.user_id = $2
        "#,
    )
    .bind(task.conversation_id)
    .bind(current.id)
    .execute(&mut *tx)
    .await?;
    let storage_deletes =
        crate::images::delete_unreferenced_assets(&mut tx, current.id, &candidate_assets).await?;
    tx.commit().await?;
    crate::images::delete_storage_files(&state, &storage_deletes).await;
    Ok(StatusCode::NO_CONTENT)
}

fn default_limit() -> i64 {
    50
}
