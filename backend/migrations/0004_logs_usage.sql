CREATE TABLE request_logs (
    id BIGSERIAL PRIMARY KEY,
    task_id UUID REFERENCES image_tasks(id) ON DELETE SET NULL,
    trace_id VARCHAR(64) NOT NULL,
    route VARCHAR(256) NOT NULL,
    method VARCHAR(16) NOT NULL,
    provider_type VARCHAR(32),
    model_key VARCHAR(128),
    status_code INTEGER,
    latency_ms BIGINT,
    ip_hash VARCHAR(128),
    user_agent TEXT,
    error_code VARCHAR(128),
    error_summary TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (status_code IS NULL OR status_code BETWEEN 100 AND 599),
    CHECK (latency_ms IS NULL OR latency_ms >= 0)
);

CREATE INDEX ix_request_logs_trace_id ON request_logs(trace_id);
CREATE INDEX ix_request_logs_created_at ON request_logs(created_at DESC);
CREATE INDEX ix_request_logs_task_id ON request_logs(task_id);

CREATE TABLE usage_records (
    id BIGSERIAL PRIMARY KEY,
    task_id UUID REFERENCES image_tasks(id) ON DELETE SET NULL,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    provider_id UUID NOT NULL,
    model_id UUID NOT NULL,
    quantity NUMERIC(18, 6) NOT NULL DEFAULT 1,
    unit VARCHAR(32) NOT NULL,
    cost NUMERIC(18, 6),
    currency VARCHAR(16) NOT NULL DEFAULT 'USD',
    pricing_snapshot JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (provider_id, model_id)
        REFERENCES models(provider_id, id) ON DELETE RESTRICT,
    CHECK (quantity >= 0),
    CHECK (cost IS NULL OR cost >= 0)
);

CREATE INDEX ix_usage_records_user_created ON usage_records(user_id, created_at DESC);
CREATE INDEX ix_usage_records_task_id ON usage_records(task_id);
