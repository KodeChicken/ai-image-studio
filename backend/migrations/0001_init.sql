CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS btree_gist;

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username VARCHAR(64) NOT NULL UNIQUE,
    external_user_id VARCHAR(128),
    password_hash TEXT,
    display_name VARCHAR(128),
    role VARCHAR(32) NOT NULL DEFAULT 'user',
    status VARCHAR(32) NOT NULL DEFAULT 'active',
    must_change_password BOOLEAN NOT NULL DEFAULT FALSE,
    theme_preference VARCHAR(16) NOT NULL DEFAULT 'system',
    session_version BIGINT NOT NULL DEFAULT 1,
    last_login_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (role IN ('admin', 'user')),
    CHECK (status IN ('active', 'disabled')),
    CHECK (theme_preference IN ('light', 'dark', 'system')),
    CHECK (session_version > 0)
);

CREATE UNIQUE INDEX ux_users_external_user_id
    ON users(external_user_id)
    WHERE external_user_id IS NOT NULL;

