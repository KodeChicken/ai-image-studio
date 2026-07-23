CREATE TABLE providers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    provider_key VARCHAR(64) NOT NULL,
    provider_type VARCHAR(32) NOT NULL,
    display_name VARCHAR(128) NOT NULL,
    base_url TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    config_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    credential_ciphertext BYTEA,
    credential_nonce BYTEA,
    credential_key_version INTEGER,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (provider_type IN ('openai-compatible', 'gemini', 'grok', 'flux', 'comfyui', 'custom')),
    CHECK (
        (credential_ciphertext IS NULL AND credential_nonce IS NULL AND credential_key_version IS NULL)
        OR
        (credential_ciphertext IS NOT NULL AND credential_nonce IS NOT NULL AND credential_key_version IS NOT NULL)
    )
);

CREATE UNIQUE INDEX ux_providers_owner_key_active
    ON providers(owner_id, provider_key)
    WHERE deleted_at IS NULL;

CREATE INDEX ix_providers_owner_id
    ON providers(owner_id)
    WHERE deleted_at IS NULL;

CREATE TABLE models (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider_id UUID NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
    model_key VARCHAR(128) NOT NULL,
    upstream_model_id VARCHAR(256) NOT NULL,
    display_name VARCHAR(128) NOT NULL,
    capabilities JSONB NOT NULL DEFAULT '{}'::jsonb,
    parameter_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
    availability_status VARCHAR(32) NOT NULL DEFAULT 'discovered',
    discovery_source VARCHAR(32) NOT NULL DEFAULT 'upstream_list',
    capability_source VARCHAR(32) NOT NULL DEFAULT 'official_catalog',
    upstream_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_discovered_at TIMESTAMPTZ,
    last_verified_at TIMESTAMPTZ,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    sort_order INTEGER NOT NULL DEFAULT 0,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (provider_id, id),
    CHECK (availability_status IN ('discovered', 'verified', 'unsupported', 'unavailable')),
    CHECK (discovery_source IN ('upstream_list', 'official_catalog', 'manual')),
    CHECK (capability_source IN ('official_catalog', 'provider_metadata', 'manual_override', 'probe'))
);

CREATE UNIQUE INDEX ux_models_provider_key_active
    ON models(provider_id, model_key)
    WHERE deleted_at IS NULL;

CREATE UNIQUE INDEX ux_models_provider_upstream_active
    ON models(provider_id, upstream_model_id)
    WHERE deleted_at IS NULL;

CREATE INDEX ix_models_provider_id
    ON models(provider_id)
    WHERE deleted_at IS NULL;

CREATE TABLE model_pricing (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id UUID NOT NULL REFERENCES models(id) ON DELETE RESTRICT,
    pricing_type VARCHAR(32) NOT NULL,
    dimension_key VARCHAR(64) NOT NULL,
    price NUMERIC(18, 6) NOT NULL,
    currency VARCHAR(16) NOT NULL DEFAULT 'USD',
    effective_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    effective_to TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (price >= 0),
    CHECK (effective_to IS NULL OR effective_to > effective_from),
    EXCLUDE USING gist (
        model_id WITH =,
        pricing_type WITH =,
        dimension_key WITH =,
        tstzrange(effective_from, effective_to, '[)') WITH &&
    )
);

CREATE INDEX ix_model_pricing_model_id ON model_pricing(model_id);

