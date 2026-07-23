CREATE TABLE update_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    action VARCHAR(32) NOT NULL,
    from_version VARCHAR(64),
    target_version VARCHAR(64) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'pending',
    progress INTEGER NOT NULL DEFAULT 0,
    current_step VARCHAR(128),
    error_message TEXT,
    requested_by UUID REFERENCES users(id) ON DELETE SET NULL,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (action IN ('upgrade', 'rollback')),
    CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'cancelled')),
    CHECK (progress BETWEEN 0 AND 100)
);

CREATE INDEX ix_update_jobs_created_at ON update_jobs(created_at DESC);

CREATE TABLE deployment_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_version VARCHAR(64) NOT NULL,
    image_reference TEXT NOT NULL,
    image_digest VARCHAR(128),
    schema_version BIGINT NOT NULL,
    backup_reference TEXT,
    deployment_status VARCHAR(32) NOT NULL,
    deployed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rolled_back_at TIMESTAMPTZ,
    CHECK (deployment_status IN ('active', 'superseded', 'failed', 'rolled_back')),
    CHECK (schema_version >= 0)
);

CREATE INDEX ix_deployment_history_deployed_at
    ON deployment_history(deployed_at DESC);

