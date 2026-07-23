use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{AppError, AppResult},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/usage", get(usage_overview))
        .route("/api/v1/admin/analytics", get(admin_analytics))
        .route("/api/v1/admin/request-logs", get(request_logs))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PeriodQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    before_id: Option<i64>,
    #[serde(default = "default_usage_limit")]
    limit: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PeriodView {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}

fn period(
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> AppResult<(DateTime<Utc>, DateTime<Utc>)> {
    let to = to.unwrap_or_else(Utc::now);
    let from = from.unwrap_or_else(|| to - Duration::days(30));
    if from >= to {
        return Err(AppError::Validation(
            "from must be earlier than to".to_owned(),
        ));
    }
    if to - from > Duration::days(366) {
        return Err(AppError::Validation(
            "analytics period cannot exceed 366 days".to_owned(),
        ));
    }
    Ok((from, to))
}

fn default_usage_limit() -> i64 {
    50
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct UsageTotals {
    task_count: i64,
    image_count: f64,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct CurrencyTotal {
    currency: String,
    total_cost: f64,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct UsageByModel {
    provider_id: uuid::Uuid,
    model_id: uuid::Uuid,
    provider_name: String,
    model_name: String,
    task_count: i64,
    image_count: f64,
    total_cost: Option<f64>,
    currency: String,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct UsageRecordView {
    id: i64,
    task_id: Option<uuid::Uuid>,
    provider_name: String,
    model_name: String,
    quantity: f64,
    unit: String,
    cost: Option<f64>,
    currency: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageOverview {
    period: PeriodView,
    totals: UsageTotals,
    costs: Vec<CurrencyTotal>,
    by_model: Vec<UsageByModel>,
    recent: Vec<UsageRecordView>,
    next_before_id: Option<i64>,
}

async fn usage_overview(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<UsageQuery>,
) -> AppResult<Json<UsageOverview>> {
    current.require_password_changed()?;
    if !(1..=200).contains(&query.limit) {
        return Err(AppError::Validation(
            "limit must be between 1 and 200".to_owned(),
        ));
    }
    let (from, to) = period(query.from, query.to)?;
    let totals = sqlx::query_as::<_, UsageTotals>(
        r#"
        SELECT COUNT(DISTINCT COALESCE(task_id::TEXT, 'deleted:' || id::TEXT))::BIGINT AS task_count,
               COALESCE(SUM(quantity), 0)::DOUBLE PRECISION AS image_count
        FROM usage_records
        WHERE user_id = $1 AND created_at >= $2 AND created_at < $3
        "#,
    )
    .bind(current.id)
    .bind(from)
    .bind(to)
    .fetch_one(&state.db)
    .await?;
    let costs = sqlx::query_as::<_, CurrencyTotal>(
        r#"
        SELECT currency, SUM(cost)::DOUBLE PRECISION AS total_cost
        FROM usage_records
        WHERE user_id = $1 AND created_at >= $2 AND created_at < $3 AND cost IS NOT NULL
        GROUP BY currency ORDER BY currency
        "#,
    )
    .bind(current.id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    let by_model = sqlx::query_as::<_, UsageByModel>(
        r#"
        SELECT p.id AS provider_id, m.id AS model_id,
               p.display_name AS provider_name, m.display_name AS model_name,
               COUNT(DISTINCT COALESCE(u.task_id::TEXT, 'deleted:' || u.id::TEXT))::BIGINT AS task_count,
               COALESCE(SUM(u.quantity), 0)::DOUBLE PRECISION AS image_count,
               SUM(u.cost)::DOUBLE PRECISION AS total_cost,
               u.currency
        FROM usage_records u
        JOIN providers p ON p.id = u.provider_id
        JOIN models m ON m.id = u.model_id
        WHERE u.user_id = $1 AND u.created_at >= $2 AND u.created_at < $3
        GROUP BY p.id, m.id, p.display_name, m.display_name, u.currency
        ORDER BY image_count DESC, provider_name, model_name
        "#,
    )
    .bind(current.id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    let recent = sqlx::query_as::<_, UsageRecordView>(
        r#"
        SELECT u.id, u.task_id, p.display_name AS provider_name,
               m.display_name AS model_name, u.quantity::DOUBLE PRECISION AS quantity,
               u.unit, u.cost::DOUBLE PRECISION AS cost, u.currency, u.created_at
        FROM usage_records u
        JOIN providers p ON p.id = u.provider_id
        JOIN models m ON m.id = u.model_id
        WHERE u.user_id = $1 AND u.created_at >= $2 AND u.created_at < $3
          AND ($4::BIGINT IS NULL OR u.id < $4)
        ORDER BY u.id DESC
        LIMIT $5
        "#,
    )
    .bind(current.id)
    .bind(from)
    .bind(to)
    .bind(query.before_id)
    .bind(query.limit)
    .fetch_all(&state.db)
    .await?;
    let next_before_id = (recent.len() == query.limit as usize)
        .then(|| recent.last().map(|item| item.id))
        .flatten();
    Ok(Json(UsageOverview {
        period: PeriodView { from, to },
        totals,
        costs,
        by_model,
        recent,
        next_before_id,
    }))
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct AdminTaskTotals {
    total_tasks: i64,
    succeeded_tasks: i64,
    failed_tasks: i64,
    active_tasks: i64,
    retry_count: i64,
    generated_images: i64,
    success_rate: f64,
    p50_latency_ms: Option<f64>,
    p95_latency_ms: Option<f64>,
    p99_latency_ms: Option<f64>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct ProviderMetric {
    provider_id: uuid::Uuid,
    provider_name: String,
    provider_type: String,
    task_count: i64,
    succeeded_tasks: i64,
    failed_tasks: i64,
    average_latency_ms: Option<f64>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct DailyMetric {
    day: String,
    task_count: i64,
    succeeded_tasks: i64,
    failed_tasks: i64,
    image_count: i64,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct StorageMetric {
    driver: String,
    asset_count: i64,
    file_size_bytes: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminAnalytics {
    period: PeriodView,
    totals: AdminTaskTotals,
    providers: Vec<ProviderMetric>,
    daily: Vec<DailyMetric>,
    storage: Vec<StorageMetric>,
    costs: Vec<CurrencyTotal>,
}

async fn admin_analytics(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<PeriodQuery>,
) -> AppResult<Json<AdminAnalytics>> {
    current.require_admin()?;
    current.require_password_changed()?;
    let (from, to) = period(query.from, query.to)?;
    let totals = sqlx::query_as::<_, AdminTaskTotals>(
        r#"
        WITH filtered AS (
            SELECT * FROM image_tasks WHERE created_at >= $1 AND created_at < $2
        )
        SELECT COUNT(*)::BIGINT AS total_tasks,
               COUNT(*) FILTER (WHERE status = 'succeeded')::BIGINT AS succeeded_tasks,
               COUNT(*) FILTER (WHERE status = 'failed')::BIGINT AS failed_tasks,
               COUNT(*) FILTER (WHERE status IN ('pending', 'processing', 'retrying'))::BIGINT AS active_tasks,
               COALESCE(SUM(retry_count), 0)::BIGINT AS retry_count,
               (SELECT COUNT(*)::BIGINT FROM image_results r JOIN filtered f ON f.id = r.task_id) AS generated_images,
               CASE WHEN COUNT(*) = 0 THEN 0::DOUBLE PRECISION
                    ELSE COUNT(*) FILTER (WHERE status = 'succeeded')::DOUBLE PRECISION / COUNT(*) END AS success_rate,
               PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM (finished_at - started_at)) * 1000)
                   FILTER (WHERE finished_at IS NOT NULL AND started_at IS NOT NULL)::DOUBLE PRECISION AS p50_latency_ms,
               PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM (finished_at - started_at)) * 1000)
                   FILTER (WHERE finished_at IS NOT NULL AND started_at IS NOT NULL)::DOUBLE PRECISION AS p95_latency_ms,
               PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM (finished_at - started_at)) * 1000)
                   FILTER (WHERE finished_at IS NOT NULL AND started_at IS NOT NULL)::DOUBLE PRECISION AS p99_latency_ms
        FROM filtered
        "#,
    )
    .bind(from)
    .bind(to)
    .fetch_one(&state.db)
    .await?;
    let providers = sqlx::query_as::<_, ProviderMetric>(
        r#"
        SELECT p.id AS provider_id, p.display_name AS provider_name, p.provider_type,
               COUNT(*)::BIGINT AS task_count,
               COUNT(*) FILTER (WHERE t.status = 'succeeded')::BIGINT AS succeeded_tasks,
               COUNT(*) FILTER (WHERE t.status = 'failed')::BIGINT AS failed_tasks,
               AVG(EXTRACT(EPOCH FROM (t.finished_at - t.started_at)) * 1000)
                   FILTER (WHERE t.finished_at IS NOT NULL AND t.started_at IS NOT NULL)::DOUBLE PRECISION AS average_latency_ms
        FROM image_tasks t JOIN providers p ON p.id = t.provider_id
        WHERE t.created_at >= $1 AND t.created_at < $2
        GROUP BY p.id, p.display_name, p.provider_type
        ORDER BY task_count DESC, provider_name
        "#,
    )
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    let costs = sqlx::query_as::<_, CurrencyTotal>(
        r#"
        SELECT currency, SUM(cost)::DOUBLE PRECISION AS total_cost
        FROM usage_records
        WHERE created_at >= $1 AND created_at < $2 AND cost IS NOT NULL
        GROUP BY currency ORDER BY currency
        "#,
    )
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    let daily = sqlx::query_as::<_, DailyMetric>(
        r#"
        SELECT DATE_TRUNC('day', t.created_at)::DATE::TEXT AS day,
               COUNT(DISTINCT t.id)::BIGINT AS task_count,
               COUNT(DISTINCT t.id) FILTER (WHERE t.status = 'succeeded')::BIGINT AS succeeded_tasks,
               COUNT(DISTINCT t.id) FILTER (WHERE t.status = 'failed')::BIGINT AS failed_tasks,
               COUNT(r.id)::BIGINT AS image_count
        FROM image_tasks t LEFT JOIN image_results r ON r.task_id = t.id
        WHERE t.created_at >= $1 AND t.created_at < $2
        GROUP BY DATE_TRUNC('day', t.created_at)::DATE
        ORDER BY DATE_TRUNC('day', t.created_at)::DATE
        "#,
    )
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    let storage = sqlx::query_as::<_, StorageMetric>(
        r#"
        SELECT storage_driver AS driver, COUNT(*)::BIGINT AS asset_count,
               COALESCE(SUM(file_size_bytes), 0)::BIGINT AS file_size_bytes
        FROM image_assets GROUP BY storage_driver ORDER BY storage_driver
        "#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(AdminAnalytics {
        period: PeriodView { from, to },
        totals,
        providers,
        daily,
        storage,
        costs,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogQuery {
    before_id: Option<i64>,
    trace_id: Option<String>,
    #[serde(default = "default_log_limit")]
    limit: i64,
}

fn default_log_limit() -> i64 {
    50
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct RequestLogView {
    id: i64,
    task_id: Option<uuid::Uuid>,
    trace_id: String,
    route: String,
    method: String,
    provider_type: Option<String>,
    model_key: Option<String>,
    status_code: Option<i32>,
    latency_ms: Option<i64>,
    error_code: Option<String>,
    error_summary: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestLogPage {
    items: Vec<RequestLogView>,
    next_before_id: Option<i64>,
}

async fn request_logs(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<LogQuery>,
) -> AppResult<Json<RequestLogPage>> {
    current.require_admin()?;
    current.require_password_changed()?;
    if !(1..=200).contains(&query.limit) {
        return Err(AppError::Validation(
            "limit must be between 1 and 200".to_owned(),
        ));
    }
    let trace_id = query
        .trace_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let items = sqlx::query_as::<_, RequestLogView>(
        r#"
        SELECT id, task_id, trace_id, route, method, provider_type, model_key,
               status_code, latency_ms, error_code, error_summary, created_at
        FROM request_logs
        WHERE ($1::BIGINT IS NULL OR id < $1)
          AND ($2::TEXT IS NULL OR trace_id = $2)
        ORDER BY id DESC
        LIMIT $3
        "#,
    )
    .bind(query.before_id)
    .bind(trace_id)
    .bind(query.limit)
    .fetch_all(&state.db)
    .await?;
    let next_before_id = (items.len() == query.limit as usize)
        .then(|| items.last().map(|item| item.id))
        .flatten();
    Ok(Json(RequestLogPage {
        items,
        next_before_id,
    }))
}
