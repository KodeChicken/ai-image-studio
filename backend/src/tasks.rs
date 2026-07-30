use std::{
    convert::Infallible,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::{Bytes, BytesMut};
use chrono::{Datelike, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{FromRow, Postgres, Transaction};
use url::{Host, Url};
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{AppError, AppResult},
    images::{self, ImageAssetSummary},
    provider_adapters::{
        self, ProviderImage, ProviderInput, ProviderPartialImage, ProviderRequest,
    },
    providers,
    storage::StoredObject,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/images/generations", post(compatible_generation))
        .route("/api/v1/images/edits", post(compatible_edit))
        .route("/api/v1/tasks/{id}", get(get_task))
        .route("/api/v1/tasks/{id}/events", get(events))
        .route(
            "/api/v1/tasks/{task_id}/partials/{event_id}",
            get(partial_content),
        )
        .route("/api/v1/tasks/{id}/cancel", post(cancel))
        .route("/api/v1/tasks/{id}/retry", post(retry))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompatibleImageRequest {
    prompt: String,
    conversation_id: Option<Uuid>,
    provider_id: Option<Uuid>,
    model_id: Option<Uuid>,
    model: Option<String>,
    #[serde(default)]
    input_asset_ids: Vec<Uuid>,
    style_prompt: Option<String>,
    stream: Option<bool>,
    #[serde(flatten)]
    parameters: Map<String, Value>,
}

async fn compatible_generation(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(mut input): Json<CompatibleImageRequest>,
) -> AppResult<Response> {
    input.input_asset_ids.clear();
    compatible_image_request(state, current, input, false).await
}

async fn compatible_edit(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(input): Json<CompatibleImageRequest>,
) -> AppResult<Response> {
    if input.input_asset_ids.is_empty() {
        return Err(AppError::Validation(
            "image edits require at least one inputAssetId".to_owned(),
        ));
    }
    compatible_image_request(state, current, input, true).await
}

async fn compatible_image_request(
    state: AppState,
    current: CurrentUser,
    input: CompatibleImageRequest,
    edit: bool,
) -> AppResult<Response> {
    current.require_password_changed()?;
    if input.prompt.trim().is_empty() {
        return Err(AppError::Validation("prompt is required".to_owned()));
    }
    let (provider_id, model_id) = resolve_compatible_model(
        &state,
        current.id,
        input.provider_id,
        input.model_id,
        input.model.as_deref(),
    )
    .await?;
    let conversation_id = match input.conversation_id {
        Some(id) => id,
        None => create_implicit_conversation(&state, current.id, provider_id, model_id).await?,
    };
    let stream = input.stream.unwrap_or(true);
    let created = create_task(
        &state,
        current.id,
        NewTaskRequest {
            conversation_id,
            content: input.prompt.trim().to_owned(),
            parent_message_id: None,
            provider_id,
            model_id,
            parameters: Value::Object(input.parameters),
            input_asset_ids: if edit {
                input.input_asset_ids
            } else {
                Vec::new()
            },
            style_prompt: input.style_prompt,
        },
    )
    .await?;
    dispatch_processing(state.clone(), created.task_id).await?;
    if stream {
        event_stream_response(state, current, created.task_id, 0).await
    } else {
        Ok((StatusCode::ACCEPTED, Json(created)).into_response())
    }
}

async fn resolve_compatible_model(
    state: &AppState,
    user_id: Uuid,
    provider_id: Option<Uuid>,
    model_id: Option<Uuid>,
    upstream_model_id: Option<&str>,
) -> AppResult<(Option<Uuid>, Option<Uuid>)> {
    if provider_id.is_some() || model_id.is_some() {
        if provider_id.is_some() && model_id.is_some() {
            return Ok((provider_id, model_id));
        }
        return Err(AppError::Validation(
            "providerId and modelId must be supplied together".to_owned(),
        ));
    }
    let Some(upstream_model_id) = upstream_model_id else {
        return Ok((None, None));
    };
    let selection = sqlx::query_as::<_, ModelSelection>(
        r#"
        SELECT p.id AS provider_id, m.id AS model_id, m.parameter_schema
        FROM providers p JOIN models m ON m.provider_id = p.id
        WHERE p.owner_id = $1 AND m.upstream_model_id = $2
          AND p.enabled AND p.deleted_at IS NULL
          AND m.enabled AND m.deleted_at IS NULL
          AND m.availability_status = 'verified'
        ORDER BY m.sort_order, m.created_at LIMIT 1
        "#,
    )
    .bind(user_id)
    .bind(upstream_model_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Validation("requested model is not available".to_owned()))?;
    Ok((Some(selection.provider_id), Some(selection.model_id)))
}

async fn create_implicit_conversation(
    state: &AppState,
    user_id: Uuid,
    provider_id: Option<Uuid>,
    model_id: Option<Uuid>,
) -> AppResult<Uuid> {
    let sort_order = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(sort_order), 0) + 1024 FROM conversations WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    Ok(sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO conversations (
            user_id, title, default_provider_id, default_model_id, sort_order
        ) VALUES ($1, '新生图会话', $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(provider_id)
    .bind(model_id)
    .bind(sort_order)
    .fetch_one(&state.db)
    .await?)
}

pub struct NewTaskRequest {
    pub conversation_id: Uuid,
    pub content: String,
    pub parent_message_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    pub model_id: Option<Uuid>,
    pub parameters: Value,
    pub input_asset_ids: Vec<Uuid>,
    pub style_prompt: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCreated {
    pub conversation_id: Uuid,
    pub message_id: Uuid,
    pub task_id: Uuid,
}

#[derive(FromRow)]
struct ConversationDefaults {
    default_provider_id: Option<Uuid>,
    default_model_id: Option<Uuid>,
}

#[derive(FromRow)]
struct ModelSelection {
    provider_id: Uuid,
    model_id: Uuid,
    parameter_schema: Value,
}

pub async fn create_task(
    state: &AppState,
    user_id: Uuid,
    request: NewTaskRequest,
) -> AppResult<TaskCreated> {
    let mut tx = state.db.begin().await?;
    let conversation = sqlx::query_as::<_, ConversationDefaults>(
        r#"
        SELECT default_provider_id, default_model_id
        FROM conversations
        WHERE id = $1 AND user_id = $2 AND status = 'active'
        FOR UPDATE
        "#,
    )
    .bind(request.conversation_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;

    let selection = resolve_model_selection(
        &mut tx,
        user_id,
        request.provider_id.or(conversation.default_provider_id),
        request.model_id.or(conversation.default_model_id),
    )
    .await?;
    let parent_message_id =
        resolve_parent(&mut tx, request.conversation_id, request.parent_message_id).await?;
    let mut input_asset_ids =
        validate_explicit_assets(&mut tx, user_id, &request.input_asset_ids).await?;
    let explicit_asset_count = input_asset_ids.len();
    let previous_asset_id =
        latest_generated_asset_for_conversation(&mut tx, request.conversation_id, user_id).await?;
    append_previous_asset(&mut input_asset_ids, previous_asset_id)?;
    let operation = if input_asset_ids.is_empty() {
        "generation"
    } else {
        "edit"
    };
    validate_task_parameters(&request.parameters, &selection.parameter_schema, operation)?;

    let sequence = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(sequence_no), 0) + 1 FROM conversation_messages WHERE conversation_id = $1",
    )
    .bind(request.conversation_id)
    .fetch_one(&mut *tx)
    .await?;
    let user_message_id = Uuid::new_v4();
    let assistant_message_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO conversation_messages (
            id, conversation_id, parent_message_id, role, status, sequence_no, content
        ) VALUES ($1, $2, $3, 'user', 'completed', $4, $5)
        "#,
    )
    .bind(user_message_id)
    .bind(request.conversation_id)
    .bind(parent_message_id)
    .bind(sequence)
    .bind(&request.content)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO conversation_messages (
            id, conversation_id, parent_message_id, role, status, sequence_no, content
        ) VALUES ($1, $2, $3, 'assistant', 'streaming', $4, NULL)
        "#,
    )
    .bind(assistant_message_id)
    .bind(request.conversation_id)
    .bind(user_message_id)
    .bind(sequence + 1)
    .execute(&mut *tx)
    .await?;

    let context = load_text_context(&mut tx, request.conversation_id, parent_message_id).await?;
    let final_prompt = build_prompt(&context, &request.content, request.style_prompt.as_deref())?;
    let task_id = Uuid::new_v4();
    let trace_id = Uuid::new_v4().simple().to_string();
    let mut request_snapshot = request.parameters;
    if let Value::Object(values) = &mut request_snapshot {
        values.insert(
            "context_asset_ids".to_owned(),
            Value::Array(
                input_asset_ids
                    .iter()
                    .copied()
                    .map(|id| json!(id))
                    .collect(),
            ),
        );
        values.insert("context_message_count".to_owned(), json!(context.len()));
    }
    sqlx::query(
        r#"
        INSERT INTO image_tasks (
            id, user_id, conversation_id, user_message_id, assistant_message_id,
            model_id, provider_id, operation, status, prompt, request_params, trace_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9, $10, $11)
        "#,
    )
    .bind(task_id)
    .bind(user_id)
    .bind(request.conversation_id)
    .bind(user_message_id)
    .bind(assistant_message_id)
    .bind(selection.model_id)
    .bind(selection.provider_id)
    .bind(operation)
    .bind(final_prompt)
    .bind(request_snapshot)
    .bind(trace_id)
    .execute(&mut *tx)
    .await?;

    for (index, asset_id) in input_asset_ids.iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO task_input_images (task_id, asset_id, input_index, input_role)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(task_id)
        .bind(asset_id)
        .bind(index as i32)
        .bind(if index < explicit_asset_count {
            "reference"
        } else {
            "source"
        })
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO message_image_assets (message_id, asset_id, relation_type, sort_order)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(user_message_id)
        .bind(asset_id)
        .bind(if index < explicit_asset_count {
            "attachment"
        } else {
            "reference"
        })
        .bind(index as i32)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        r#"
        UPDATE conversations
        SET default_provider_id = $1, default_model_id = $2,
            last_message_at = NOW(), updated_at = NOW()
        WHERE id = $3
        "#,
    )
    .bind(selection.provider_id)
    .bind(selection.model_id)
    .bind(request.conversation_id)
    .execute(&mut *tx)
    .await?;
    insert_event(
        &mut tx,
        task_id,
        "task.created",
        None,
        Some("pending"),
        json!({
            "taskId": task_id,
            "conversationId": request.conversation_id,
            "messageId": assistant_message_id
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(TaskCreated {
        conversation_id: request.conversation_id,
        message_id: assistant_message_id,
        task_id,
    })
}

fn spawn_processing(state: AppState, task_id: Uuid) {
    tokio::spawn(async move {
        run_task(&state, task_id).await;
    });
}

pub async fn dispatch_processing(state: AppState, task_id: Uuid) -> AppResult<()> {
    match state.settings.task_execution_mode {
        crate::config::TaskExecutionMode::Inline => {
            spawn_processing(state, task_id);
            Ok(())
        }
        crate::config::TaskExecutionMode::Redis => {
            if let Err(error) = enqueue_task(&state, task_id).await {
                let message = format!("task queue is unavailable: {error}");
                mark_dispatch_failure(&state, task_id, &message).await?;
                return Err(AppError::Internal(anyhow::anyhow!(message)));
            }
            Ok(())
        }
    }
}

async fn enqueue_task(state: &AppState, task_id: Uuid) -> anyhow::Result<()> {
    let client = state.redis.as_ref().context("Redis is not configured")?;
    let mut connection = client.get_multiplexed_async_connection().await?;
    redis::cmd("LPUSH")
        .arg(&state.settings.task_queue_key)
        .arg(task_id.to_string())
        .query_async::<i64>(&mut connection)
        .await?;
    Ok(())
}

pub async fn run_worker(state: AppState) -> anyhow::Result<()> {
    if state.settings.task_execution_mode != crate::config::TaskExecutionMode::Redis {
        bail!("worker requires TASK_EXECUTION_MODE=redis");
    }
    recover_stale_tasks(&state).await?;
    let client = state
        .redis
        .as_ref()
        .context("Redis is not configured")?
        .clone();
    tracing::info!(queue = %state.settings.task_queue_key, "task worker started");
    loop {
        let mut connection = match client.get_multiplexed_async_connection().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(error = %error, "failed to connect to Redis; retrying");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        loop {
            let queued = redis::cmd("BRPOP")
                .arg(&state.settings.task_queue_key)
                .arg(5)
                .query_async::<Vec<String>>(&mut connection)
                .await;
            let queued = match queued {
                Ok(values) => values.get(1).cloned(),
                Err(error) => {
                    tracing::warn!(error = %error, "Redis task dequeue failed; reconnecting");
                    break;
                }
            };
            let task_id = queued
                .as_deref()
                .and_then(|value| Uuid::parse_str(value).ok())
                .or(next_pending_task(&state).await?);
            if let Some(task_id) = task_id {
                run_task(&state, task_id).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn next_pending_task(state: &AppState) -> anyhow::Result<Option<Uuid>> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT id FROM image_tasks
        WHERE status IN ('pending', 'retrying')
        ORDER BY created_at
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.db)
    .await?)
}

async fn recover_stale_tasks(state: &AppState) -> anyhow::Result<()> {
    let stale_after = state.settings.request_timeout_seconds.saturating_add(60) as f64;
    let recovered = sqlx::query_scalar::<_, Uuid>(
        r#"
        UPDATE image_tasks
        SET status = 'retrying', retry_count = retry_count + 1,
            error_code = 'WORKER_INTERRUPTED',
            error_message = 'worker stopped before the task completed',
            finished_at = NULL, updated_at = NOW()
        WHERE status = 'processing'
          AND started_at < NOW() - make_interval(secs => $1)
          AND retry_count < $2
        RETURNING id
        "#,
    )
    .bind(stale_after)
    .bind(state.settings.task_max_retries)
    .fetch_all(&state.db)
    .await?;
    if !recovered.is_empty() {
        tracing::warn!(count = recovered.len(), "recovered stale image tasks");
    }
    Ok(())
}

async fn run_task(state: &AppState, task_id: Uuid) {
    if let Err(error) = process_task(state, task_id).await {
        tracing::error!(task_id = %task_id, error = %error, "image task failed");
        match fail_task(state, task_id, &error).await {
            Ok(true) => schedule_retry(state.clone(), task_id),
            Ok(false) => {}
            Err(record_error) => {
                tracing::error!(task_id = %task_id, error = %record_error, "failed to record task failure");
            }
        }
    }
}

fn schedule_retry(state: AppState, task_id: Uuid) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(state.settings.task_retry_delay_seconds)).await;
        match state.settings.task_execution_mode {
            crate::config::TaskExecutionMode::Inline => spawn_processing(state, task_id),
            crate::config::TaskExecutionMode::Redis => {
                if let Err(error) = enqueue_task(&state, task_id).await {
                    tracing::warn!(task_id = %task_id, error = %error, "failed to enqueue automatic retry; worker database polling will recover it");
                }
            }
        }
    });
}

#[derive(Debug, FromRow)]
struct ProcessingTask {
    id: Uuid,
    user_id: Uuid,
    assistant_message_id: Uuid,
    provider_id: Uuid,
    model_id: Uuid,
    provider_type: String,
    model_key: String,
    operation: String,
    prompt: String,
    request_params: Value,
    upstream_model_id: String,
    trace_id: String,
}

#[derive(Debug, FromRow)]
struct TaskInput {
    id: Uuid,
    storage_driver: String,
    storage_container: String,
    storage_key: String,
    mime_type: String,
}

async fn process_task(state: &AppState, task_id: Uuid) -> anyhow::Result<()> {
    let task = sqlx::query_as::<_, ProcessingTask>(
        r#"
        SELECT t.id, t.user_id, t.assistant_message_id, t.provider_id, t.model_id,
               p.provider_type, m.model_key, t.operation, t.prompt, t.request_params,
               m.upstream_model_id, t.trace_id
        FROM image_tasks t
        JOIN models m ON m.id = t.model_id AND m.provider_id = t.provider_id
        JOIN providers p ON p.id = t.provider_id
        WHERE t.id = $1 AND t.status IN ('pending', 'retrying')
        "#,
    )
    .bind(task_id)
    .fetch_optional(&state.db)
    .await?
    .context("task is not available for processing")?;
    if !transition_task(
        state,
        task_id,
        &["pending", "retrying"],
        "processing",
        "task.progress",
        json!({ "taskId": task_id, "stage": "provider.processing" }),
    )
    .await?
    {
        return Ok(());
    }
    let inputs = sqlx::query_as::<_, TaskInput>(
        r#"
        SELECT a.id, a.storage_driver, a.storage_container, a.storage_key, a.mime_type
        FROM task_input_images i
        JOIN image_assets a ON a.id = i.asset_id
        WHERE i.task_id = $1
        ORDER BY CASE i.input_role
            WHEN 'source' THEN 0
            WHEN 'reference' THEN 1
            ELSE 2
        END, i.input_index
        "#,
    )
    .bind(task_id)
    .fetch_all(&state.db)
    .await?;
    let provider_started = Instant::now();
    let outputs = match call_provider_until_cancelled(state, &task, &inputs).await {
        Ok(Some(outputs)) => {
            record_provider_request(state, &task, provider_started.elapsed(), 200, None, None)
                .await;
            outputs
        }
        Ok(None) => {
            record_provider_request(
                state,
                &task,
                provider_started.elapsed(),
                499,
                Some("TASK_CANCELLED"),
                Some("provider request cancelled by user".to_owned()),
            )
            .await;
            return Ok(());
        }
        Err(error) => {
            let status_code = provider_adapters::provider_error_status(&error)
                .map(i32::from)
                .unwrap_or(502);
            let error_code = provider_adapters::provider_error_code(&error)
                .unwrap_or("UPSTREAM_ERROR")
                .to_owned();
            record_provider_request(
                state,
                &task,
                provider_started.elapsed(),
                status_code,
                Some(&error_code),
                Some(error.to_string()),
            )
            .await;
            return Err(error);
        }
    };
    transition_event(
        state,
        task_id,
        "task.progress",
        json!({ "taskId": task_id, "stage": "storage.validating" }),
    )
    .await?;

    let current_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM image_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(&state.db)
            .await?;
    if current_status == "cancelled" {
        return Ok(());
    }

    let mut completed_assets = Vec::new();
    for (index, output) in outputs.into_iter().enumerate() {
        let bytes = match provider_image_bytes(state, output).await {
            Ok(bytes) => bytes,
            Err(error) => {
                cleanup_task_assets(state, &task, &completed_assets).await;
                return Err(error);
            }
        };
        let validated = match images::validate_image(bytes) {
            Ok(validated) => validated,
            Err(error) => {
                cleanup_task_assets(state, &task, &completed_assets).await;
                return Err(anyhow::anyhow!(error.to_string()));
            }
        };
        let (validated, result_metadata) = match normalize_result_size(&task, validated) {
            Ok(result) => result,
            Err(error) => {
                cleanup_task_assets(state, &task, &completed_assets).await;
                return Err(error);
            }
        };
        let asset = match images::persist_asset(state, task.user_id, None, validated).await {
            Ok(asset) => asset,
            Err(error) => {
                cleanup_task_assets(state, &task, &completed_assets).await;
                return Err(anyhow::anyhow!(error.to_string()));
            }
        };
        if let Err(error) = link_result(state, &task, &asset, index as i32, result_metadata).await {
            compensate_asset(state, task.user_id, asset.id).await;
            cleanup_task_assets(state, &task, &completed_assets).await;
            return Err(error);
        }
        completed_assets.push(asset);
    }
    if completed_assets.is_empty() {
        bail!("provider returned no images");
    }

    let mut tx = state.db.begin().await?;
    record_usage(&mut tx, &task, completed_assets.len()).await?;
    let changed = sqlx::query(
        r#"
        UPDATE image_tasks
        SET status = 'succeeded', response_summary = $1,
            actual_cost = (SELECT cost FROM usage_records WHERE task_id = $2 ORDER BY id DESC LIMIT 1),
            finished_at = NOW(), updated_at = NOW()
        WHERE id = $2 AND status = 'processing'
        "#,
    )
    .bind(json!({ "imageCount": completed_assets.len() }))
    .bind(task_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        tx.rollback().await?;
        cleanup_task_assets(state, &task, &completed_assets).await;
        return Ok(());
    }
    sqlx::query(
        "UPDATE conversation_messages SET status = 'completed', content = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(format!("已生成 {} 张图片", completed_assets.len()))
    .bind(task.assistant_message_id)
    .execute(&mut *tx)
    .await?;
    insert_event(
        &mut tx,
        task_id,
        "task.completed",
        Some("processing"),
        Some("succeeded"),
        json!({ "taskId": task_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn record_usage(
    tx: &mut Transaction<'_, Postgres>,
    task: &ProcessingTask,
    image_count: usize,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        WITH selected_price AS (
            SELECT id, pricing_type, dimension_key, price, currency,
                   effective_from, effective_to
            FROM model_pricing
            WHERE model_id = $4
              AND pricing_type = 'image'
              AND effective_from <= NOW()
              AND (effective_to IS NULL OR effective_to > NOW())
              AND dimension_key IN ('image', 'default')
            ORDER BY CASE dimension_key WHEN 'image' THEN 0 ELSE 1 END, effective_from DESC
            LIMIT 1
        )
        INSERT INTO usage_records (
            task_id, user_id, provider_id, model_id, quantity, unit,
            cost, currency, pricing_snapshot
        )
        SELECT $1, $2, $3, $4, $5::NUMERIC, 'image',
               selected_price.price * $5::NUMERIC,
               COALESCE(selected_price.currency, 'USD'),
               CASE WHEN selected_price.id IS NULL THEN '{}'::JSONB ELSE jsonb_build_object(
                   'pricingId', selected_price.id,
                   'pricingType', selected_price.pricing_type,
                   'dimensionKey', selected_price.dimension_key,
                   'unitPrice', selected_price.price,
                   'effectiveFrom', selected_price.effective_from,
                   'effectiveTo', selected_price.effective_to
               ) END
        FROM (SELECT 1) AS seed
        LEFT JOIN selected_price ON TRUE
        "#,
    )
    .bind(task.id)
    .bind(task.user_id)
    .bind(task.provider_id)
    .bind(task.model_id)
    .bind(image_count as i64)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn record_provider_request(
    state: &AppState,
    task: &ProcessingTask,
    elapsed: Duration,
    status_code: i32,
    error_code: Option<&str>,
    error: Option<String>,
) {
    let result = sqlx::query(
        r#"
        INSERT INTO request_logs (
            task_id, trace_id, route, method, provider_type, model_key,
            status_code, latency_ms, error_code, error_summary
        ) VALUES ($1, $2, $3, 'POST', $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(task.id)
    .bind(&task.trace_id)
    .bind(if task.operation == "edit" {
        "provider.images.edits"
    } else {
        "provider.images.generations"
    })
    .bind(&task.provider_type)
    .bind(&task.model_key)
    .bind(status_code)
    .bind(elapsed.as_millis().min(i64::MAX as u128) as i64)
    .bind(error_code)
    .bind(error.map(|value| value.chars().take(1000).collect::<String>()))
    .execute(&state.db)
    .await;
    if let Err(error) = result {
        tracing::warn!(task_id = %task.id, error = %error, "failed to persist provider request log");
    }
}

async fn call_provider(
    state: &AppState,
    task: &ProcessingTask,
    inputs: &[TaskInput],
    partial_sender: Option<tokio::sync::mpsc::UnboundedSender<ProviderPartialImage>>,
) -> anyhow::Result<Vec<ProviderImage>> {
    let provider = providers::credential_row(state, task.user_id, task.provider_id)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let (ciphertext, nonce) = provider
        .credential_ciphertext
        .zip(provider.credential_nonce)
        .context("provider API key is not configured")?;
    let credential = state.credential_cipher.decrypt(&ciphertext, &nonce)?;
    let mut provider_inputs = Vec::with_capacity(inputs.len());
    for input in inputs {
        let bytes = state
            .storage
            .get(
                &input.storage_driver,
                &input.storage_container,
                &input.storage_key,
            )
            .await?;
        provider_inputs.push(ProviderInput {
            filename: format!("{}{}", input.id, extension_for_mime(&input.mime_type)),
            mime_type: input.mime_type.clone(),
            bytes,
        });
    }
    provider_adapters::create_images_with_partials(
        &provider.provider_type,
        &task.operation,
        &state.http_client,
        &provider.base_url,
        &credential,
        ProviderRequest {
            model: task.upstream_model_id.clone(),
            prompt: task.prompt.clone(),
            parameters: task.request_params.clone(),
            inputs: provider_inputs,
            max_image_bytes: state.settings.max_provider_image_size_mb * 1024 * 1024,
        },
        partial_sender,
    )
    .await
}

async fn call_provider_until_cancelled(
    state: &AppState,
    task: &ProcessingTask,
    inputs: &[TaskInput],
) -> anyhow::Result<Option<Vec<ProviderImage>>> {
    let (partial_sender, mut partial_receiver) = tokio::sync::mpsc::unbounded_channel();
    let provider_call = call_provider(state, task, inputs, Some(partial_sender));
    tokio::pin!(provider_call);
    let mut cancellation_check = tokio::time::interval(Duration::from_millis(200));
    cancellation_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut partial_channel_open = true;
    let mut partials = Vec::new();
    let result = loop {
        tokio::select! {
            result = &mut provider_call => {
                while let Ok(partial) = partial_receiver.try_recv() {
                    record_partial_preview(state, task, partial, &mut partials).await;
                }
                break result.map(Some);
            },
            partial = partial_receiver.recv(), if partial_channel_open => {
                match partial {
                    Some(partial) => record_partial_preview(state, task, partial, &mut partials).await,
                    None => partial_channel_open = false,
                }
            },
            _ = cancellation_check.tick() => {
                let status = sqlx::query_scalar::<_, String>(
                    "SELECT status FROM image_tasks WHERE id = $1",
                )
                .bind(task.id)
                .fetch_optional(&state.db)
                .await;
                let status = match status {
                    Ok(status) => status,
                    Err(error) => break Err(error.into()),
                };
                if status.as_deref() == Some("cancelled") {
                    break Ok(None);
                }
            }
        }
    };
    schedule_partial_cleanup(state.clone(), partials);
    result
}

struct StoredPartialPreview {
    event_id: i64,
    object: StoredObject,
}

async fn record_partial_preview(
    state: &AppState,
    task: &ProcessingTask,
    partial: ProviderPartialImage,
    partials: &mut Vec<StoredPartialPreview>,
) {
    match persist_partial_preview(state, task, partial).await {
        Ok(preview) => partials.push(preview),
        Err(error) => tracing::warn!(
            task_id = %task.id,
            error = %error,
            "failed to persist provider partial preview"
        ),
    }
}

async fn persist_partial_preview(
    state: &AppState,
    task: &ProcessingTask,
    partial: ProviderPartialImage,
) -> anyhow::Result<StoredPartialPreview> {
    let bytes = provider_image_bytes(
        state,
        ProviderImage {
            url: None,
            b64_json: Some(partial.b64_json),
        },
    )
    .await?;
    let image =
        images::validate_image(bytes).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let now = Utc::now();
    let storage_key = format!(
        "{}/{:02}/{}/{}.{}",
        now.year(),
        now.month(),
        task.user_id,
        Uuid::new_v4(),
        image.extension
    );
    let object = state.storage.put(&storage_key, image.bytes.clone()).await?;
    let event_result = async {
        let mut tx = state.db.begin().await?;
        let mut event_data = json!({
            "taskId": task.id,
            "partialIndex": partial.index,
            "storageDriver": object.driver,
            "storageContainer": object.container,
            "storageKey": object.key,
            "mimeType": image.mime_type,
            "width": image.width,
            "height": image.height,
            "fileSizeBytes": image.bytes.len(),
            "expired": false
        });
        let event_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO task_events (task_id, event_type, event_data) VALUES ($1, 'image.partial', $2) RETURNING id",
        )
        .bind(task.id)
        .bind(&event_data)
        .fetch_one(&mut *tx)
        .await?;
        event_data["contentUrl"] = json!(format!(
            "/api/v1/tasks/{}/partials/{event_id}",
            task.id
        ));
        sqlx::query("UPDATE task_events SET event_data = $1 WHERE id = $2")
            .bind(event_data)
            .bind(event_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok::<_, anyhow::Error>(event_id)
    }
    .await;
    match event_result {
        Ok(event_id) => Ok(StoredPartialPreview { event_id, object }),
        Err(error) => {
            let _ = state
                .storage
                .delete(object.driver, &object.container, &object.key)
                .await;
            Err(error)
        }
    }
}

fn schedule_partial_cleanup(state: AppState, partials: Vec<StoredPartialPreview>) {
    if partials.is_empty() {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(300)).await;
        for partial in partials {
            match state
                .storage
                .delete(
                    partial.object.driver,
                    &partial.object.container,
                    &partial.object.key,
                )
                .await
            {
                Ok(()) => {
                    if let Err(error) = sqlx::query(
                        "UPDATE task_events SET event_data = event_data || '{\"expired\":true}'::jsonb WHERE id = $1",
                    )
                    .bind(partial.event_id)
                    .execute(&state.db)
                    .await
                    {
                        tracing::warn!(event_id = partial.event_id, error = %error, "failed to expire partial preview event");
                    }
                }
                Err(error) => tracing::warn!(
                    event_id = partial.event_id,
                    error = %error,
                    "failed to delete partial preview; consistency cleanup must recover it"
                ),
            }
        }
    });
}

pub(crate) fn validate_task_parameters(
    parameters: &Value,
    schema: &Value,
    operation: &str,
) -> AppResult<()> {
    let values = parameters
        .as_object()
        .ok_or_else(|| AppError::Validation("parameters must be a JSON object".to_owned()))?;
    let definitions = schema
        .get("parameters")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (name, value) in values {
        if name == "aspect_ratio" && !definitions.contains_key(name) {
            if !matches!(
                value.as_str(),
                Some("auto" | "1:1" | "3:2" | "2:3" | "16:9" | "9:16")
            ) {
                return Err(AppError::Validation("unsupported aspect_ratio".to_owned()));
            }
            continue;
        }
        let definition = definitions.get(name).ok_or_else(|| {
            AppError::Validation(format!("parameter '{name}' is not supported by this model"))
        })?;
        if definition.get("supported").and_then(Value::as_bool) == Some(false) {
            return Err(AppError::Validation(format!(
                "parameter '{name}' is disabled for this model"
            )));
        }
        if definition
            .get("operations")
            .and_then(Value::as_array)
            .is_some_and(|operations| !operations.contains(&json!(operation)))
        {
            return Err(AppError::Validation(format!(
                "parameter '{name}' is not supported for {operation}"
            )));
        }
        if definition
            .get("visible_when")
            .and_then(Value::as_object)
            .is_some_and(|conditions| {
                conditions.iter().any(|(dependency, expected)| {
                    let actual = if dependency == "stream" {
                        Some(json!(true))
                    } else {
                        values.get(dependency).cloned().or_else(|| {
                            definitions
                                .get(dependency)
                                .and_then(|item| item.get("default"))
                                .cloned()
                        })
                    };
                    match (actual, expected.as_array()) {
                        (Some(actual), Some(options)) => !options.contains(&actual),
                        (Some(actual), None) => &actual != expected,
                        (None, _) => true,
                    }
                })
            })
        {
            return Err(AppError::Validation(format!(
                "parameter '{name}' is not available with the selected settings"
            )));
        }
        let kind = definition
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("string");
        let type_matches = match kind {
            "boolean" => value.is_boolean(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.is_number(),
            "enum" | "string" => value.is_string(),
            _ => false,
        };
        if !type_matches {
            return Err(AppError::Validation(format!(
                "parameter '{name}' has an invalid type"
            )));
        }
        if let Some(options) = definition.get("options").and_then(Value::as_array)
            && !options.contains(value)
            && !(name == "size"
                && definition.get("allow_custom").and_then(Value::as_bool) == Some(true)
                && value
                    .as_str()
                    .is_some_and(|value| valid_custom_image_size(value, definition)))
        {
            return Err(AppError::Validation(format!(
                "parameter '{name}' is outside the supported options"
            )));
        }
        if let Some(number) = value.as_f64()
            && (definition
                .get("min")
                .and_then(Value::as_f64)
                .is_some_and(|minimum| number < minimum)
                || definition
                    .get("max")
                    .and_then(Value::as_f64)
                    .is_some_and(|maximum| number > maximum))
        {
            return Err(AppError::Validation(format!(
                "parameter '{name}' is outside the supported range"
            )));
        }
    }
    validate_aspect_ratio_matches_size(values)?;
    Ok(())
}

fn validate_aspect_ratio_matches_size(values: &Map<String, Value>) -> AppResult<()> {
    let Some(aspect_ratio) = values.get("aspect_ratio").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(size) = values.get("size").and_then(Value::as_str) else {
        return Ok(());
    };
    if aspect_ratio.eq_ignore_ascii_case("auto") || size.eq_ignore_ascii_case("auto") {
        return Ok(());
    }
    let Some((aspect_width, aspect_height)) =
        aspect_ratio.split_once(':').and_then(|(width, height)| {
            Some((width.parse::<u64>().ok()?, height.parse::<u64>().ok()?))
        })
    else {
        return Ok(());
    };
    let Some((width, height)) = size.split_once('x').and_then(|(width, height)| {
        Some((width.parse::<u64>().ok()?, height.parse::<u64>().ok()?))
    }) else {
        return Ok(());
    };
    let matches = width
        .checked_mul(aspect_height)
        .zip(height.checked_mul(aspect_width))
        .is_some_and(|(left, right)| left == right);
    if matches {
        Ok(())
    } else {
        Err(AppError::Validation(
            "aspect_ratio does not match size".to_owned(),
        ))
    }
}

fn valid_custom_image_size(value: &str, definition: &Value) -> bool {
    let Some((width, height)) = value.split_once('x').and_then(|(width, height)| {
        Some((width.parse::<u64>().ok()?, height.parse::<u64>().ok()?))
    }) else {
        return false;
    };
    let constraints = definition.get("constraints").unwrap_or(&Value::Null);
    let edge_multiple = constraints
        .get("edgeMultiple")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1);
    let max_edge = constraints
        .get("maxEdge")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let min_pixels = constraints
        .get("minPixels")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let max_pixels = constraints
        .get("maxPixels")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let max_aspect_ratio = constraints
        .get("maxAspectRatio")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX)
        .max(1);
    let Some(pixels) = width.checked_mul(height) else {
        return false;
    };
    let long_edge = width.max(height);
    let short_edge = width.min(height);
    width > 0
        && height > 0
        && width % edge_multiple == 0
        && height % edge_multiple == 0
        && long_edge <= max_edge
        && pixels >= min_pixels
        && pixels <= max_pixels
        && short_edge
            .checked_mul(max_aspect_ratio)
            .is_some_and(|maximum| long_edge <= maximum)
}

fn extension_for_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => ".jpg",
        "image/webp" => ".webp",
        _ => ".png",
    }
}

pub(crate) async fn provider_image_bytes(
    state: &AppState,
    image: ProviderImage,
) -> anyhow::Result<Bytes> {
    let max = state.settings.max_provider_image_size_mb * 1024 * 1024;
    if let Some(encoded) = image.b64_json {
        if encoded.len() > max.saturating_mul(4) / 3 + 16 {
            bail!("provider image exceeds configured limit");
        }
        let value = encoded
            .split_once(',')
            .map(|(_, data)| data)
            .unwrap_or(&encoded);
        let bytes = STANDARD
            .decode(value)
            .context("provider returned invalid base64 image")?;
        if bytes.len() > max {
            bail!("provider image exceeds configured limit");
        }
        return Ok(Bytes::from(bytes));
    }
    let raw_url = image
        .url
        .context("provider image has neither URL nor base64 data")?;
    let validated_url = validate_provider_image_url(&raw_url).await?;
    let response = state.http_client.get(validated_url).send().await?;
    if !response.status().is_success() {
        bail!(
            "provider image download returned HTTP {}",
            response.status()
        );
    }
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        bail!("provider image exceeds configured limit");
    }
    let mut output = BytesMut::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if output.len() + chunk.len() > max {
            bail!("provider image exceeds configured limit");
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output.freeze())
}

async fn validate_provider_image_url(value: &str) -> anyhow::Result<Url> {
    let url = Url::parse(value).context("provider returned invalid image URL")?;
    if url.scheme() != "https" {
        bail!("provider image URL must use HTTPS");
    }
    match url.host() {
        Some(Host::Ipv4(ip)) if ip.is_loopback() || ip.is_private() || ip.is_link_local() => {
            bail!("provider image URL resolves to a disallowed address")
        }
        Some(Host::Ipv6(ip)) if ip.is_loopback() || ip.is_unspecified() => {
            bail!("provider image URL resolves to a disallowed address")
        }
        None => bail!("provider image URL has no host"),
        _ => {}
    }
    let host = url.host_str().context("provider image URL has no host")?;
    let port = url
        .port_or_known_default()
        .context("provider image URL has no port")?;
    let addresses: Vec<_> = tokio::net::lookup_host((host, port)).await?.collect();
    if addresses.is_empty() {
        bail!("provider image URL host did not resolve");
    }
    if addresses
        .iter()
        .any(|address| is_disallowed_ip(address.ip()))
    {
        bail!("provider image URL resolves to a disallowed address");
    }
    Ok(url)
}

fn is_disallowed_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        std::net::IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

async fn link_result(
    state: &AppState,
    task: &ProcessingTask,
    asset: &ImageAssetSummary,
    index: i32,
    metadata: Value,
) -> anyhow::Result<()> {
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO image_results (task_id, asset_id, result_index, metadata) VALUES ($1, $2, $3, $4)",
    )
        .bind(task.id)
        .bind(asset.id)
        .bind(index)
        .bind(&metadata)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        INSERT INTO message_image_assets (message_id, asset_id, relation_type, sort_order)
        VALUES ($1, $2, 'generated', $3)
        "#,
    )
    .bind(task.assistant_message_id)
    .bind(asset.id)
    .bind(index)
    .execute(&mut *tx)
    .await?;
    insert_event(
        &mut tx,
        task.id,
        "image.completed",
        None,
        None,
        json!({ "taskId": task.id, "asset": asset, "metadata": metadata }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

fn normalize_result_size(
    task: &ProcessingTask,
    image: images::ValidatedImage,
) -> anyhow::Result<(images::ValidatedImage, Value)> {
    let Some((target_width, target_height)) = exact_size_target(task) else {
        return Ok((image, json!({})));
    };
    if image.width == target_width as i32 && image.height == target_height as i32 {
        return Ok((image, json!({})));
    }

    let source_width = image.width;
    let source_height = image.height;
    let resized = images::resize_exact(image, target_width, target_height)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok((
        resized,
        json!({
            "resized": true,
            "resizeMethod": "lanczos3-center-crop",
            "sourceWidth": source_width,
            "sourceHeight": source_height,
            "requestedSize": format!("{target_width}x{target_height}")
        }),
    ))
}

fn exact_size_target(task: &ProcessingTask) -> Option<(u32, u32)> {
    if task.provider_type != "openai-compatible"
        || !task.upstream_model_id.eq_ignore_ascii_case("gpt-image-2")
    {
        return None;
    }
    provider_adapters::effective_openai_dimensions(&task.request_params, &task.upstream_model_id)
}

async fn compensate_asset(state: &AppState, owner_id: Uuid, asset_id: Uuid) {
    let asset = images::load_owned_asset(state, owner_id, asset_id).await;
    if let Ok(asset) = asset
        && sqlx::query("DELETE FROM image_assets WHERE id = $1")
            .bind(asset_id)
            .execute(&state.db)
            .await
            .is_ok()
    {
        let _ = state
            .storage
            .delete(
                &asset.storage_driver,
                &asset.storage_container,
                &asset.storage_key,
            )
            .await;
    }
}

async fn cleanup_task_assets(
    state: &AppState,
    task: &ProcessingTask,
    assets: &[ImageAssetSummary],
) {
    if assets.is_empty() {
        return;
    }
    let asset_ids: Vec<_> = assets.iter().map(|asset| asset.id).collect();
    let cleanup = async {
        let mut tx = state.db.begin().await?;
        sqlx::query(
            "DELETE FROM message_image_assets WHERE message_id = $1 AND asset_id = ANY($2)",
        )
        .bind(task.assistant_message_id)
        .bind(&asset_ids)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM image_results WHERE task_id = $1 AND asset_id = ANY($2)")
            .bind(task.id)
            .bind(&asset_ids)
            .execute(&mut *tx)
            .await?;
        let storage_deletes =
            images::delete_unreferenced_assets(&mut tx, task.user_id, &asset_ids).await?;
        tx.commit().await?;
        Ok::<_, AppError>(storage_deletes)
    }
    .await;
    match cleanup {
        Ok(storage_deletes) => images::delete_storage_files(state, &storage_deletes).await,
        Err(error) => tracing::error!(
            task_id = %task.id,
            error = %error,
            "failed to clean partial task assets; consistency cleanup must recover them"
        ),
    }
}

#[derive(FromRow)]
struct FailureUpdate {
    assistant_message_id: Uuid,
    status: String,
    retry_count: i32,
}

async fn fail_task(state: &AppState, task_id: Uuid, error: &anyhow::Error) -> anyhow::Result<bool> {
    let retryable = provider_adapters::provider_error_is_retryable(error);
    let terminal_error_code = provider_adapters::provider_error_code(error)
        .unwrap_or("TASK_FAILED")
        .to_owned();
    let user_message = provider_adapters::provider_error_user_message(error)
        .unwrap_or_else(|| truncate_error(error));
    let mut tx = state.db.begin().await?;
    let update = sqlx::query_as::<_, FailureUpdate>(
        r#"
        UPDATE image_tasks
        SET status = CASE WHEN $4 AND retry_count < $3 THEN 'retrying' ELSE 'failed' END,
            retry_count = CASE WHEN $4 AND retry_count < $3 THEN retry_count + 1 ELSE retry_count END,
            error_code = CASE WHEN $4 AND retry_count < $3 THEN 'TASK_RETRYING' ELSE $5 END,
            error_message = $1,
            finished_at = CASE WHEN $4 AND retry_count < $3 THEN NULL ELSE NOW() END,
            updated_at = NOW()
        WHERE id = $2 AND status IN ('pending', 'processing', 'retrying')
        RETURNING assistant_message_id, status, retry_count
        "#,
    )
    .bind(&user_message)
    .bind(task_id)
    .bind(state.settings.task_max_retries)
    .bind(retryable)
    .bind(&terminal_error_code)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(update) = update {
        let retrying = update.status == "retrying";
        sqlx::query(
            "UPDATE conversation_messages SET status = $1, content = $2, updated_at = NOW() WHERE id = $3",
        )
        .bind(if retrying { "streaming" } else { "failed" })
        .bind(if retrying {
            format!(
                "正在自动重试（{}/{}）",
                update.retry_count, state.settings.task_max_retries
            )
        } else {
            user_message.clone()
        })
        .bind(update.assistant_message_id)
        .execute(&mut *tx)
        .await?;
        insert_event(
            &mut tx,
            task_id,
            if retrying {
                "task.progress"
            } else {
                "task.failed"
            },
            Some("processing"),
            Some(if retrying { "retrying" } else { "failed" }),
            if retrying {
                json!({
                    "taskId": task_id,
                    "stage": "automatic_retry",
                    "retryCount": update.retry_count,
                    "maxRetries": state.settings.task_max_retries
                })
            } else {
                json!({ "taskId": task_id, "errorCode": terminal_error_code })
            },
        )
        .await?;
        tx.commit().await?;
        return Ok(retrying);
    }
    tx.commit().await?;
    Ok(false)
}

async fn mark_dispatch_failure(state: &AppState, task_id: Uuid, message: &str) -> AppResult<()> {
    let mut tx = state.db.begin().await?;
    let task = sqlx::query_as::<_, (Uuid, String)>(
        r#"
        SELECT assistant_message_id, status
        FROM image_tasks
        WHERE id = $1 AND status IN ('pending', 'retrying')
        FOR UPDATE
        "#,
    )
    .bind(task_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((assistant_message_id, from_status)) = task else {
        tx.rollback().await?;
        return Ok(());
    };
    sqlx::query(
        r#"
        UPDATE image_tasks
        SET status = 'failed', error_code = 'QUEUE_UNAVAILABLE', error_message = $1,
            finished_at = NOW(), updated_at = NOW()
        WHERE id = $2 AND status = $3
        "#,
    )
    .bind(message.chars().take(1000).collect::<String>())
    .bind(task_id)
    .bind(&from_status)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE conversation_messages SET status = 'failed', content = '任务队列暂不可用', updated_at = NOW() WHERE id = $1",
    )
    .bind(assistant_message_id)
    .execute(&mut *tx)
    .await?;
    insert_event(
        &mut tx,
        task_id,
        "task.failed",
        Some(&from_status),
        Some("failed"),
        json!({ "taskId": task_id, "errorCode": "QUEUE_UNAVAILABLE" }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

fn truncate_error(error: &anyhow::Error) -> String {
    error.to_string().chars().take(1000).collect()
}

async fn transition_task(
    state: &AppState,
    task_id: Uuid,
    from: &[&str],
    to: &str,
    event_type: &str,
    data: Value,
) -> anyhow::Result<bool> {
    let mut tx = state.db.begin().await?;
    let changed = sqlx::query(
        "UPDATE image_tasks SET status = $1, started_at = CASE WHEN status = 'retrying' THEN NOW() ELSE COALESCE(started_at, NOW()) END, updated_at = NOW() WHERE id = $2 AND status = ANY($3)",
    )
    .bind(to)
    .bind(task_id)
    .bind(from)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    insert_event(&mut tx, task_id, event_type, None, Some(to), data).await?;
    tx.commit().await?;
    Ok(true)
}

async fn transition_event(
    state: &AppState,
    task_id: Uuid,
    event_type: &str,
    data: Value,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO task_events (task_id, event_type, event_data) VALUES ($1, $2, $3)")
        .bind(task_id)
        .bind(event_type)
        .bind(data)
        .execute(&state.db)
        .await?;
    Ok(())
}

async fn insert_event(
    tx: &mut Transaction<'_, Postgres>,
    task_id: Uuid,
    event_type: &str,
    from_status: Option<&str>,
    to_status: Option<&str>,
    data: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO task_events (task_id, event_type, from_status, to_status, event_data)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(task_id)
    .bind(event_type)
    .bind(from_status)
    .bind(to_status)
    .bind(data)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn resolve_model_selection(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    provider_id: Option<Uuid>,
    model_id: Option<Uuid>,
) -> AppResult<ModelSelection> {
    let selection = match (provider_id, model_id) {
        (Some(provider_id), Some(model_id)) => {
            sqlx::query_as::<_, ModelSelection>(
                r#"
            SELECT p.id AS provider_id, m.id AS model_id, m.parameter_schema
            FROM providers p JOIN models m ON m.provider_id = p.id
            WHERE p.id = $1 AND m.id = $2 AND p.owner_id = $3
              AND p.enabled AND p.deleted_at IS NULL
              AND m.enabled AND m.deleted_at IS NULL
              AND m.availability_status = 'verified'
            "#,
            )
            .bind(provider_id)
            .bind(model_id)
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, ModelSelection>(
                r#"
            SELECT p.id AS provider_id, m.id AS model_id, m.parameter_schema
            FROM providers p JOIN models m ON m.provider_id = p.id
            WHERE p.owner_id = $1 AND p.enabled AND p.deleted_at IS NULL
              AND m.enabled AND m.deleted_at IS NULL
              AND m.availability_status = 'verified'
            ORDER BY m.sort_order, m.created_at
            LIMIT 1
            "#,
            )
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await?
        }
        _ => {
            return Err(AppError::Validation(
                "provider and model must be selected together".to_owned(),
            ));
        }
    };
    selection.ok_or_else(|| AppError::Validation("no verified image model is available".to_owned()))
}

async fn resolve_parent(
    tx: &mut Transaction<'_, Postgres>,
    conversation_id: Uuid,
    requested: Option<Uuid>,
) -> AppResult<Option<Uuid>> {
    if let Some(parent_id) = requested {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE id = $1 AND conversation_id = $2)",
        )
        .bind(parent_id)
        .bind(conversation_id)
        .fetch_one(&mut **tx)
        .await?;
        return if exists {
            Ok(Some(parent_id))
        } else {
            Err(AppError::Validation(
                "parent message does not belong to this conversation".to_owned(),
            ))
        };
    }
    Ok(sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT id FROM conversation_messages
        WHERE conversation_id = $1 AND role = 'assistant'
        ORDER BY sequence_no DESC LIMIT 1
        "#,
    )
    .bind(conversation_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn validate_explicit_assets(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    asset_ids: &[Uuid],
) -> AppResult<Vec<Uuid>> {
    let unique: std::collections::HashSet<_> = asset_ids.iter().copied().collect();
    if unique.len() != asset_ids.len() || asset_ids.len() > 10 {
        return Err(AppError::Validation(
            "input assets must be unique and contain at most 10 items".to_owned(),
        ));
    }
    let owned = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM image_assets WHERE owner_id = $1 AND id = ANY($2) ORDER BY id FOR SHARE",
    )
    .bind(user_id)
    .bind(asset_ids)
    .fetch_all(&mut **tx)
    .await?;
    if owned.len() != asset_ids.len() {
        return Err(AppError::Validation(
            "one or more input assets are unavailable".to_owned(),
        ));
    }
    Ok(asset_ids.to_vec())
}

async fn latest_generated_asset_for_conversation(
    tx: &mut Transaction<'_, Postgres>,
    conversation_id: Uuid,
    user_id: Uuid,
) -> AppResult<Option<Uuid>> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT ir.asset_id
        FROM image_tasks t
        JOIN image_results ir ON ir.task_id = t.id
        JOIN image_assets a ON a.id = ir.asset_id
        WHERE t.conversation_id = $1
          AND t.user_id = $2
          AND t.status = 'succeeded'
          AND a.owner_id = $2
        ORDER BY t.created_at DESC, ir.result_index ASC
        LIMIT 1
        "#,
    )
    .bind(conversation_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?)
}

fn append_previous_asset(
    input_asset_ids: &mut Vec<Uuid>,
    previous_asset_id: Option<Uuid>,
) -> AppResult<()> {
    let Some(previous_asset_id) = previous_asset_id else {
        return Ok(());
    };
    if input_asset_ids.contains(&previous_asset_id) {
        return Ok(());
    }
    if input_asset_ids.len() >= 10 {
        return Err(AppError::Validation(
            "input assets plus the previous conversation image must contain at most 10 items"
                .to_owned(),
        ));
    }
    input_asset_ids.push(previous_asset_id);
    Ok(())
}

async fn load_text_context(
    tx: &mut Transaction<'_, Postgres>,
    conversation_id: Uuid,
    parent_message_id: Option<Uuid>,
) -> AppResult<Vec<(String, String)>> {
    #[derive(FromRow)]
    struct ContextRow {
        role: String,
        content: String,
        sequence_no: i64,
    }
    let Some(parent_id) = parent_message_id else {
        return Ok(Vec::new());
    };
    let mut rows = sqlx::query_as::<_, ContextRow>(
        r#"
        WITH RECURSIVE branch AS (
            SELECT id, parent_message_id, role, content, sequence_no
            FROM conversation_messages
            WHERE id = $1 AND conversation_id = $2
            UNION ALL
            SELECT parent.id, parent.parent_message_id, parent.role, parent.content, parent.sequence_no
            FROM conversation_messages parent
            JOIN branch child ON child.parent_message_id = parent.id
            WHERE parent.conversation_id = $2
        )
        SELECT role, content, sequence_no
        FROM branch
        WHERE content IS NOT NULL AND role IN ('user', 'assistant')
        ORDER BY sequence_no DESC
        LIMIT 8
        "#,
    )
    .bind(parent_id)
    .bind(conversation_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.sort_by_key(|row| row.sequence_no);
    Ok(rows
        .into_iter()
        .map(|row| (row.role, row.content))
        .collect())
}

fn build_prompt(
    context: &[(String, String)],
    current: &str,
    style: Option<&str>,
) -> AppResult<String> {
    if style.is_some_and(|value| value.len() > 4000) {
        return Err(AppError::Validation("style prompt is too long".to_owned()));
    }
    let mut prompt = String::new();
    if !context.is_empty() {
        prompt.push_str("Conversation context:\n");
        for (role, content) in context {
            prompt.push_str(if role == "user" {
                "User: "
            } else {
                "Assistant: "
            });
            prompt.push_str(content);
            prompt.push('\n');
        }
        prompt.push_str("Current request: ");
    }
    prompt.push_str(current);
    if let Some(style) = style.map(str::trim).filter(|value| !value.is_empty()) {
        prompt.push_str("\nStyle guidance: ");
        prompt.push_str(style);
    }
    Ok(prompt)
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct TaskView {
    id: Uuid,
    conversation_id: Uuid,
    user_message_id: Uuid,
    assistant_message_id: Uuid,
    model_id: Uuid,
    provider_id: Uuid,
    operation: String,
    status: String,
    request_params: Value,
    error_code: Option<String>,
    error_message: Option<String>,
    retry_count: i32,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    #[sqlx(skip)]
    results: Vec<ImageAssetSummary>,
}

async fn get_task(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(task_id): Path<Uuid>,
) -> AppResult<Json<TaskView>> {
    current.require_password_changed()?;
    let mut task = sqlx::query_as::<_, TaskView>(
        r#"
        SELECT id, conversation_id, user_message_id, assistant_message_id, model_id,
               provider_id, operation, status, request_params, error_code, error_message,
               retry_count, started_at, finished_at, created_at
        FROM image_tasks WHERE id = $1 AND user_id = $2
        "#,
    )
    .bind(task_id)
    .bind(current.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    task.results = task_results(&state, task_id).await?;
    Ok(Json(task))
}

async fn task_results(state: &AppState, task_id: Uuid) -> AppResult<Vec<ImageAssetSummary>> {
    #[derive(FromRow)]
    struct ResultRow {
        id: Uuid,
        mime_type: String,
        width: Option<i32>,
        height: Option<i32>,
        file_size_bytes: i64,
    }
    Ok(sqlx::query_as::<_, ResultRow>(
        r#"
        SELECT a.id, a.mime_type, a.width, a.height, a.file_size_bytes
        FROM image_results r JOIN image_assets a ON a.id = r.asset_id
        WHERE r.task_id = $1 ORDER BY r.result_index
        "#,
    )
    .bind(task_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| ImageAssetSummary {
        id: row.id,
        content_url: format!("/api/v1/image-assets/{}/content", row.id),
        mime_type: row.mime_type,
        width: row.width,
        height: row.height,
        file_size_bytes: row.file_size_bytes,
    })
    .collect())
}

#[derive(Debug, FromRow)]
struct TaskEventRow {
    id: i64,
    event_type: String,
    event_data: Value,
}

#[derive(Debug, FromRow)]
struct PartialImageContent {
    storage_driver: String,
    storage_container: String,
    storage_key: String,
    mime_type: String,
}

async fn partial_content(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((task_id, event_id)): Path<(Uuid, i64)>,
) -> AppResult<Response> {
    current.require_password_changed()?;
    let partial = sqlx::query_as::<_, PartialImageContent>(
        r#"
        SELECT e.event_data->>'storageDriver' AS storage_driver,
               e.event_data->>'storageContainer' AS storage_container,
               e.event_data->>'storageKey' AS storage_key,
               e.event_data->>'mimeType' AS mime_type
        FROM task_events e
        JOIN image_tasks t ON t.id = e.task_id
        WHERE e.id = $1 AND e.task_id = $2 AND t.user_id = $3
          AND e.event_type = 'image.partial'
          AND e.event_data->>'expired' IS DISTINCT FROM 'true'
        "#,
    )
    .bind(event_id)
    .bind(task_id)
    .bind(current.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let bytes = state
        .storage
        .get(
            &partial.storage_driver,
            &partial.storage_container,
            &partial.storage_key,
        )
        .await
        .map_err(|error| {
            tracing::warn!(task_id = %task_id, event_id, error = %error, "partial preview is unavailable");
            AppError::NotFound
        })?;
    let mut response = Body::from(bytes).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&partial.mime_type)
            .map_err(|error| AppError::Internal(error.into()))?,
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn events(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(task_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Response> {
    current.require_password_changed()?;
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    event_stream_response(state, current, task_id, last_event_id).await
}

pub async fn event_stream_response(
    state: AppState,
    current: CurrentUser,
    task_id: Uuid,
    last_event_id: i64,
) -> AppResult<Response> {
    let owned = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM image_tasks WHERE id = $1 AND user_id = $2)",
    )
    .bind(task_id)
    .bind(current.id)
    .fetch_one(&state.db)
    .await?;
    if !owned {
        return Err(AppError::NotFound);
    }
    let pool = state.db.clone();
    let stream = async_stream::stream! {
        let mut cursor = last_event_id;
        loop {
            let rows = sqlx::query_as::<_, TaskEventRow>(
                "SELECT id, event_type, event_data FROM task_events WHERE task_id = $1 AND id > $2 ORDER BY id LIMIT 100",
            )
            .bind(task_id)
            .bind(cursor)
            .fetch_all(&pool)
            .await;
            match rows {
                Ok(rows) => {
                    let mut terminal = false;
                    for row in rows {
                        cursor = row.id;
                        terminal = matches!(row.event_type.as_str(), "task.completed" | "task.failed" | "task.cancelled");
                        let event = Event::default()
                            .id(row.id.to_string())
                            .event(row.event_type)
                            .json_data(row.event_data)
                            .expect("JSON event data is serializable");
                        yield Ok::<Event, Infallible>(event);
                    }
                    if terminal {
                        break;
                    }
                }
                Err(error) => {
                    tracing::error!(task_id = %task_id, error = %error, "failed to read task events");
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(
                    state.settings.task_stream_heartbeat_seconds,
                ))
                .text("heartbeat"),
        )
        .into_response())
}

async fn cancel(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(task_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    current.require_password_changed()?;
    let mut tx = state.db.begin().await?;
    let assistant_message_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        UPDATE image_tasks
        SET status = 'cancelled', finished_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND user_id = $2 AND status IN ('pending', 'processing', 'retrying')
        RETURNING assistant_message_id
        "#,
    )
    .bind(task_id)
    .bind(current.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(assistant_message_id) = assistant_message_id else {
        return Err(AppError::Conflict(
            "task cannot be cancelled in its current state".to_owned(),
        ));
    };
    sqlx::query(
        r#"
        UPDATE conversation_messages
        SET status = 'cancelled', content = '生成已取消', updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(assistant_message_id)
    .execute(&mut *tx)
    .await?;
    insert_event(
        &mut tx,
        task_id,
        "task.cancelled",
        None,
        Some("cancelled"),
        json!({ "taskId": task_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn retry(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(task_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    current.require_password_changed()?;
    let mut tx = state.db.begin().await?;
    let changed = sqlx::query(
        r#"
        UPDATE image_tasks
        SET status = 'retrying', retry_count = retry_count + 1,
            error_code = NULL, error_message = NULL, finished_at = NULL, updated_at = NOW()
        WHERE id = $1 AND user_id = $2 AND status = 'failed'
        "#,
    )
    .bind(task_id)
    .bind(current.id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::Conflict(
            "only failed tasks can be retried".to_owned(),
        ));
    }
    sqlx::query(
        "UPDATE conversation_messages SET status = 'streaming', content = NULL, updated_at = NOW() WHERE id = (SELECT assistant_message_id FROM image_tasks WHERE id = $1)",
    )
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    insert_event(
        &mut tx,
        task_id,
        "task.progress",
        Some("failed"),
        Some("retrying"),
        json!({ "taskId": task_id, "stage": "retrying" }),
    )
    .await?;
    let last_event_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM task_events WHERE task_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    dispatch_processing(state, task_id).await?;
    Ok(Json(
        json!({ "taskId": task_id, "lastEventId": last_event_id }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_the_previous_conversation_image_after_explicit_references() {
        let explicit = Uuid::new_v4();
        let previous = Uuid::new_v4();
        let mut ids = vec![explicit];
        append_previous_asset(&mut ids, Some(previous)).unwrap();
        assert_eq!(ids, vec![explicit, previous]);
    }

    #[test]
    fn does_not_duplicate_an_explicitly_selected_previous_image() {
        let previous = Uuid::new_v4();
        let mut ids = vec![previous];
        append_previous_asset(&mut ids, Some(previous)).unwrap();
        assert_eq!(ids, vec![previous]);
    }

    #[test]
    fn prompt_includes_context_and_style() {
        let prompt = build_prompt(
            &[("user".to_owned(), "画一只猫".to_owned())],
            "改成夜景",
            Some("电影感"),
        )
        .unwrap();
        assert!(prompt.contains("画一只猫"));
        assert!(prompt.contains("电影感"));
    }

    #[test]
    fn rejects_parameter_outside_model_schema() {
        let schema = json!({
            "parameters": {
                "quality": { "type": "enum", "options": ["auto", "high"] }
            }
        });
        assert!(
            validate_task_parameters(&json!({ "quality": "ultra" }), &schema, "generation")
                .is_err()
        );
        assert!(validate_task_parameters(&json!({ "seed": 1 }), &schema, "generation").is_err());
    }

    #[test]
    fn validates_provider_specific_aspect_ratios_from_the_model_schema() {
        let schema = json!({
            "parameters": {
                "aspect_ratio": {
                    "type": "enum",
                    "default": "auto",
                    "options": ["auto", "1:1", "21:9"]
                }
            }
        });
        assert!(
            validate_task_parameters(&json!({ "aspect_ratio": "21:9" }), &schema, "generation")
                .is_ok()
        );
        assert!(
            validate_task_parameters(&json!({ "aspect_ratio": "9:16" }), &schema, "generation")
                .is_err()
        );
    }

    #[test]
    fn enforces_operation_and_parameter_visibility_rules() {
        let schema = json!({
            "parameters": {
                "output_format": { "type": "enum", "default": "png", "options": ["png", "jpeg"] },
                "output_compression": {
                    "type": "integer", "min": 0, "max": 100,
                    "visible_when": { "output_format": ["jpeg"] }
                },
                "input_fidelity": {
                    "type": "enum", "options": ["low", "high"],
                    "operations": ["edit"]
                },
                "partial_images": {
                    "type": "integer", "min": 0, "max": 3,
                    "visible_when": { "stream": true }
                }
            }
        });
        assert!(
            validate_task_parameters(
                &json!({ "output_format": "png", "output_compression": 80 }),
                &schema,
                "generation"
            )
            .is_err()
        );
        assert!(
            validate_task_parameters(&json!({ "input_fidelity": "high" }), &schema, "generation")
                .is_err()
        );
        assert!(
            validate_task_parameters(&json!({ "input_fidelity": "high" }), &schema, "edit").is_ok()
        );
        assert!(
            validate_task_parameters(&json!({ "partial_images": 2 }), &schema, "generation")
                .is_ok()
        );
    }

    #[test]
    fn validates_gpt_image_2_custom_size_constraints() {
        let schema = json!({
            "parameters": {
                "size": {
                    "type": "enum",
                    "options": ["auto", "1024x1024"],
                    "allow_custom": true,
                    "constraints": {
                        "edgeMultiple": 16,
                        "maxEdge": 3840,
                        "minPixels": 655360,
                        "maxPixels": 8294400,
                        "maxAspectRatio": 3
                    }
                }
            }
        });
        assert!(
            validate_task_parameters(&json!({ "size": "1536x864" }), &schema, "generation").is_ok()
        );
        assert!(
            validate_task_parameters(&json!({ "size": "1537x864" }), &schema, "generation")
                .is_err()
        );
        assert!(
            validate_task_parameters(&json!({ "size": "3840x3840" }), &schema, "generation")
                .is_err()
        );
    }

    #[test]
    fn rejects_aspect_ratio_and_size_mismatches() {
        let schema = json!({
            "parameters": {
                "aspect_ratio": {
                    "type": "enum",
                    "options": ["auto", "16:9", "9:16"]
                },
                "size": {
                    "type": "enum",
                    "options": ["auto", "3840x2160", "2160x3840"]
                }
            }
        });
        assert!(
            validate_task_parameters(
                &json!({ "aspect_ratio": "9:16", "size": "2160x3840" }),
                &schema,
                "generation"
            )
            .is_ok()
        );
        assert!(
            validate_task_parameters(
                &json!({ "aspect_ratio": "16:9", "size": "2160x3840" }),
                &schema,
                "generation"
            )
            .is_err()
        );
        assert!(
            validate_task_parameters(
                &json!({ "aspect_ratio": "auto", "size": "2160x3840" }),
                &schema,
                "generation"
            )
            .is_ok()
        );
        assert!(
            validate_task_parameters(
                &json!({ "aspect_ratio": "16:9", "size": "4k" }),
                &json!({
                    "parameters": {
                        "aspect_ratio": { "type": "enum", "options": ["16:9"] },
                        "size": { "type": "enum", "options": ["4k"] }
                    }
                }),
                "generation"
            )
            .is_ok()
        );
    }

    #[test]
    fn exact_size_target_uses_explicit_or_aspect_ratio_gpt_image_2_sizes() {
        let mut task = ProcessingTask {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            assistant_message_id: Uuid::new_v4(),
            provider_id: Uuid::new_v4(),
            model_id: Uuid::new_v4(),
            provider_type: "openai-compatible".to_owned(),
            model_key: "gpt-image-2".to_owned(),
            operation: "generation".to_owned(),
            prompt: "test".to_owned(),
            request_params: json!({ "size": "3840x2160" }),
            upstream_model_id: "gpt-image-2".to_owned(),
            trace_id: "trace".to_owned(),
        };
        assert_eq!(exact_size_target(&task), Some((3840, 2160)));

        task.request_params = json!({ "size": "auto", "aspect_ratio": "16:9" });
        assert_eq!(exact_size_target(&task), Some((1536, 864)));

        task.request_params = json!({ "size": "auto" });
        assert_eq!(exact_size_target(&task), None);
    }

    #[test]
    fn normalizes_an_incorrect_upstream_orientation_to_the_effective_size() {
        let task = ProcessingTask {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            assistant_message_id: Uuid::new_v4(),
            provider_id: Uuid::new_v4(),
            model_id: Uuid::new_v4(),
            provider_type: "openai-compatible".to_owned(),
            model_key: "gpt-image-2".to_owned(),
            operation: "generation".to_owned(),
            prompt: "test".to_owned(),
            request_params: json!({ "size": "auto", "aspect_ratio": "16:9" }),
            upstream_model_id: "gpt-image-2".to_owned(),
            trace_id: "trace".to_owned(),
        };
        let source = image::DynamicImage::new_rgb8(16, 24);
        let mut encoded = std::io::Cursor::new(Vec::new());
        source
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let validated = images::validate_image(bytes::Bytes::from(encoded.into_inner())).unwrap();

        let (normalized, metadata) = normalize_result_size(&task, validated).unwrap();

        assert_eq!((normalized.width, normalized.height), (1536, 864));
        assert_eq!(metadata["sourceWidth"], json!(16));
        assert_eq!(metadata["sourceHeight"], json!(24));
        assert_eq!(metadata["requestedSize"], json!("1536x864"));
    }
}
