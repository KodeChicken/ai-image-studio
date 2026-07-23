ALTER TABLE providers
    ADD COLUMN health_status VARCHAR(32) NOT NULL DEFAULT 'unknown',
    ADD COLUMN last_health_checked_at TIMESTAMPTZ,
    ADD COLUMN last_health_error TEXT,
    ADD CONSTRAINT providers_health_status_check
        CHECK (health_status IN ('unknown', 'healthy', 'unhealthy'));

