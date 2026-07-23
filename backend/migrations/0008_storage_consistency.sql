CREATE TABLE storage_consistency_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    status VARCHAR(32) NOT NULL DEFAULT 'running',
    delete_orphans BOOLEAN NOT NULL DEFAULT FALSE,
    grace_seconds BIGINT NOT NULL,
    database_assets BIGINT NOT NULL DEFAULT 0,
    storage_objects BIGINT NOT NULL DEFAULT 0,
    missing_objects BIGINT NOT NULL DEFAULT 0,
    orphan_objects BIGINT NOT NULL DEFAULT 0,
    eligible_orphans BIGINT NOT NULL DEFAULT 0,
    deleted_orphans BIGINT NOT NULL DEFAULT 0,
    error_message TEXT,
    requested_by UUID REFERENCES users(id) ON DELETE SET NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ,
    CHECK (status IN ('running', 'succeeded', 'failed')),
    CHECK (grace_seconds > 0),
    CHECK (database_assets >= 0),
    CHECK (storage_objects >= 0),
    CHECK (missing_objects >= 0),
    CHECK (orphan_objects >= 0),
    CHECK (eligible_orphans >= 0),
    CHECK (deleted_orphans >= 0)
);

CREATE INDEX ix_storage_consistency_runs_started_at
    ON storage_consistency_runs(started_at DESC);
