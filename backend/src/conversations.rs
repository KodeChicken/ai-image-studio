use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{AppError, AppResult},
    images::{self, ImageAssetSummary},
    tasks::{self, NewTaskRequest},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/conversations", get(list).post(create))
        .route("/api/v1/conversations/order", put(reorder))
        .route(
            "/api/v1/conversations/{id}",
            get(detail).patch(update).delete(remove),
        )
        .route("/api/v1/conversations/{id}/messages", post(send_message))
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: Uuid,
    pub title: String,
    pub status: String,
    pub default_provider_id: Option<Uuid>,
    pub default_model_id: Option<Uuid>,
    pub sort_order: i64,
    pub last_message_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDetail {
    #[serde(flatten)]
    pub conversation: ConversationSummary,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub parent_message_id: Option<Uuid>,
    pub role: String,
    pub status: String,
    pub sequence_no: i64,
    pub content: Option<String>,
    pub metadata: Value,
    pub task_id: Option<Uuid>,
    pub task_error_code: Option<String>,
    pub task_error_message: Option<String>,
    pub task_retry_count: Option<i32>,
    pub task_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub task_finished_at: Option<chrono::DateTime<chrono::Utc>>,
    #[sqlx(skip)]
    pub assets: Vec<MessageImageAssetSummary>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageImageAssetSummary {
    #[serde(flatten)]
    pub asset: ImageAssetSummary,
    pub relation_type: String,
}

async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
) -> AppResult<Json<Vec<ConversationSummary>>> {
    current.require_password_changed()?;
    let conversations = sqlx::query_as::<_, ConversationSummary>(CONVERSATION_SELECT)
        .bind(current.id)
        .bind(None::<Uuid>)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(conversations))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateConversationRequest {
    title: Option<String>,
    default_provider_id: Option<Uuid>,
    default_model_id: Option<Uuid>,
}

async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<CreateConversationRequest>,
) -> AppResult<(StatusCode, Json<ConversationSummary>)> {
    current.require_password_changed()?;
    let title = normalized_title(input.title.as_deref().unwrap_or("新会话"))?;
    validate_provider_model(
        &state,
        current.id,
        input.default_provider_id,
        input.default_model_id,
    )
    .await?;
    let sort_order = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(sort_order), 0) + 1024 FROM conversations WHERE user_id = $1",
    )
    .bind(current.id)
    .fetch_one(&state.db)
    .await?;
    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO conversations (
            user_id, title, default_provider_id, default_model_id, sort_order
        ) VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(current.id)
    .bind(title)
    .bind(input.default_provider_id)
    .bind(input.default_model_id)
    .bind(sort_order)
    .fetch_one(&state.db)
    .await?;
    let conversation = find_summary(&state, current.id, id).await?;
    Ok((StatusCode::CREATED, Json(conversation)))
}

async fn detail(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ConversationDetail>> {
    current.require_password_changed()?;
    let conversation = find_summary(&state, current.id, id).await?;
    let mut messages = sqlx::query_as::<_, ConversationMessage>(
        r#"
        SELECT cm.id, cm.conversation_id, cm.parent_message_id, cm.role, cm.status,
               cm.sequence_no, cm.content, cm.metadata,
               task.id AS task_id, task.error_code AS task_error_code,
               task.error_message AS task_error_message,
               task.retry_count AS task_retry_count,
               task.started_at AS task_started_at, task.finished_at AS task_finished_at,
               cm.created_at, cm.updated_at
        FROM conversation_messages cm
        LEFT JOIN image_tasks task ON task.assistant_message_id = cm.id
        WHERE cm.conversation_id = $1
        ORDER BY cm.sequence_no
        "#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    for message in &mut messages {
        message.assets = load_message_assets(&state, current.id, message.id).await?;
    }
    Ok(Json(ConversationDetail {
        conversation,
        messages,
    }))
}

#[derive(Debug, Deserialize)]
struct UpdateConversationRequest {
    title: String,
}

async fn update(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateConversationRequest>,
) -> AppResult<Json<ConversationSummary>> {
    current.require_password_changed()?;
    let title = normalized_title(&input.title)?;
    let changed = sqlx::query(
        "UPDATE conversations SET title = $1, updated_at = NOW() WHERE id = $2 AND user_id = $3",
    )
    .bind(title)
    .bind(id)
    .bind(current.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    Ok(Json(find_summary(&state, current.id, id).await?))
}

async fn remove(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    current.require_password_changed()?;
    let mut tx = state.db.begin().await?;
    let candidate_assets = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT ma.asset_id
        FROM message_image_assets ma
        JOIN conversation_messages cm ON cm.id = ma.message_id
        WHERE cm.conversation_id = $1
        UNION
        SELECT ti.asset_id
        FROM task_input_images ti
        JOIN image_tasks t ON t.id = ti.task_id
        WHERE t.conversation_id = $1
        UNION
        SELECT ir.asset_id
        FROM image_results ir
        JOIN image_tasks t ON t.id = ir.task_id
        WHERE t.conversation_id = $1
        "#,
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let changed = sqlx::query("DELETE FROM conversations WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(current.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    let storage_deletes =
        crate::images::delete_unreferenced_assets(&mut tx, current.id, &candidate_assets).await?;
    tx.commit().await?;
    crate::images::delete_storage_files(&state, &storage_deletes).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderRequest {
    conversation_ids: Vec<Uuid>,
}

async fn reorder(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<ReorderRequest>,
) -> AppResult<StatusCode> {
    current.require_password_changed()?;
    let unique: HashSet<_> = input.conversation_ids.iter().copied().collect();
    if unique.len() != input.conversation_ids.len() {
        return Err(AppError::Validation(
            "conversation order contains duplicates".to_owned(),
        ));
    }
    let owned = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM conversations WHERE user_id = $1 AND id = ANY($2)",
    )
    .bind(current.id)
    .bind(&input.conversation_ids)
    .fetch_one(&state.db)
    .await?;
    if owned as usize != input.conversation_ids.len() {
        return Err(AppError::Validation(
            "conversation order contains unknown IDs".to_owned(),
        ));
    }
    let mut tx = state.db.begin().await?;
    for (index, id) in input.conversation_ids.into_iter().enumerate() {
        sqlx::query(
            "UPDATE conversations SET sort_order = $1, updated_at = NOW() WHERE id = $2 AND user_id = $3",
        )
        .bind((index as i64 + 1) * 1024)
        .bind(id)
        .bind(current.id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendMessageRequest {
    content: String,
    parent_message_id: Option<Uuid>,
    provider_id: Option<Uuid>,
    model_id: Option<Uuid>,
    #[serde(default)]
    parameters: Value,
    #[serde(default)]
    input_asset_ids: Vec<Uuid>,
    style_prompt: Option<String>,
    stream: Option<bool>,
}

async fn send_message(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(conversation_id): Path<Uuid>,
    Json(input): Json<SendMessageRequest>,
) -> AppResult<Response> {
    current.require_password_changed()?;
    let content = input.content.trim();
    if content.is_empty() || content.len() > 16_000 {
        cleanup_failed_input_assets(&state, current.id, &input.input_asset_ids).await;
        return Err(AppError::Validation(
            "message must contain 1 to 16000 characters".to_owned(),
        ));
    }
    if !input.parameters.is_object() {
        cleanup_failed_input_assets(&state, current.id, &input.input_asset_ids).await;
        return Err(AppError::Validation(
            "parameters must be a JSON object".to_owned(),
        ));
    }
    let cleanup_asset_ids = input.input_asset_ids.clone();
    let created = tasks::create_task(
        &state,
        current.id,
        NewTaskRequest {
            conversation_id,
            content: content.to_owned(),
            parent_message_id: input.parent_message_id,
            provider_id: input.provider_id,
            model_id: input.model_id,
            parameters: input.parameters,
            input_asset_ids: input.input_asset_ids,
            style_prompt: input.style_prompt,
        },
    )
    .await;
    let created = match created {
        Ok(created) => created,
        Err(error) => {
            cleanup_failed_input_assets(&state, current.id, &cleanup_asset_ids).await;
            return Err(error);
        }
    };
    tasks::dispatch_processing(state.clone(), created.task_id).await?;
    if input.stream.unwrap_or(true) {
        Ok(tasks::event_stream_response(state, current, created.task_id, 0).await?)
    } else {
        Ok((StatusCode::ACCEPTED, Json(created)).into_response())
    }
}

async fn cleanup_failed_input_assets(state: &AppState, user_id: Uuid, asset_ids: &[Uuid]) {
    if asset_ids.is_empty() {
        return;
    }
    let result = async {
        let mut tx = state.db.begin().await?;
        let deletes = images::delete_unreferenced_assets(&mut tx, user_id, asset_ids).await?;
        tx.commit().await?;
        images::delete_storage_files(state, &deletes).await;
        Ok::<(), AppError>(())
    }
    .await;
    if let Err(error) = result {
        tracing::error!(user_id = %user_id, error = %error, "failed to compensate unreferenced input assets");
    }
}

async fn find_summary(
    state: &AppState,
    user_id: Uuid,
    conversation_id: Uuid,
) -> AppResult<ConversationSummary> {
    sqlx::query_as::<_, ConversationSummary>(CONVERSATION_SELECT)
        .bind(user_id)
        .bind(conversation_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)
}

async fn load_message_assets(
    state: &AppState,
    owner_id: Uuid,
    message_id: Uuid,
) -> AppResult<Vec<MessageImageAssetSummary>> {
    #[derive(FromRow)]
    struct Row {
        id: Uuid,
        mime_type: String,
        width: Option<i32>,
        height: Option<i32>,
        file_size_bytes: i64,
        relation_type: String,
    }
    let rows = sqlx::query_as::<_, Row>(
        r#"
        SELECT a.id, a.mime_type, a.width, a.height, a.file_size_bytes, ma.relation_type
        FROM message_image_assets ma
        JOIN image_assets a ON a.id = ma.asset_id
        WHERE ma.message_id = $1 AND a.owner_id = $2
        ORDER BY ma.sort_order, a.created_at
        "#,
    )
    .bind(message_id)
    .bind(owner_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| MessageImageAssetSummary {
            asset: ImageAssetSummary {
                id: row.id,
                content_url: format!("/api/v1/image-assets/{}/content", row.id),
                mime_type: row.mime_type,
                width: row.width,
                height: row.height,
                file_size_bytes: row.file_size_bytes,
            },
            relation_type: row.relation_type,
        })
        .collect())
}

async fn validate_provider_model(
    state: &AppState,
    user_id: Uuid,
    provider_id: Option<Uuid>,
    model_id: Option<Uuid>,
) -> AppResult<()> {
    match (provider_id, model_id) {
        (None, None) => Ok(()),
        (Some(provider_id), Some(model_id)) => {
            let valid = sqlx::query_scalar::<_, bool>(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM models m
                    JOIN providers p ON p.id = m.provider_id
                    WHERE p.id = $1 AND m.id = $2 AND p.owner_id = $3
                      AND p.enabled AND p.deleted_at IS NULL
                      AND m.enabled AND m.deleted_at IS NULL
                )
                "#,
            )
            .bind(provider_id)
            .bind(model_id)
            .bind(user_id)
            .fetch_one(&state.db)
            .await?;
            if valid {
                Ok(())
            } else {
                Err(AppError::Validation(
                    "provider and model do not match".to_owned(),
                ))
            }
        }
        _ => Err(AppError::Validation(
            "default provider and model must be selected together".to_owned(),
        )),
    }
}

fn normalized_title(title: &str) -> AppResult<String> {
    let title = title.trim();
    if title.is_empty() || title.len() > 256 {
        Err(AppError::Validation(
            "title must contain 1 to 256 characters".to_owned(),
        ))
    } else {
        Ok(title.to_owned())
    }
}

const CONVERSATION_SELECT: &str = r#"
    SELECT id, title, status, default_provider_id, default_model_id,
           sort_order, last_message_at, created_at, updated_at
    FROM conversations
    WHERE user_id = $1 AND status = 'active' AND ($2::UUID IS NULL OR id = $2)
    ORDER BY sort_order, last_message_at DESC
"#;
