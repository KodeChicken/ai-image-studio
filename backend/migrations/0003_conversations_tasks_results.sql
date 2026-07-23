CREATE TABLE conversations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    title VARCHAR(256) NOT NULL DEFAULT '新会话',
    status VARCHAR(32) NOT NULL DEFAULT 'active',
    default_provider_id UUID REFERENCES providers(id) ON DELETE RESTRICT,
    default_model_id UUID,
    context_summary TEXT,
    sort_order BIGINT NOT NULL DEFAULT 0,
    last_message_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (default_provider_id, default_model_id)
        REFERENCES models(provider_id, id) ON DELETE RESTRICT,
    CHECK (status IN ('active', 'archived'))
);

CREATE INDEX ix_conversations_user_order
    ON conversations(user_id, sort_order, last_message_at DESC);

CREATE TABLE conversation_messages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    parent_message_id UUID,
    role VARCHAR(16) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'completed',
    sequence_no BIGINT NOT NULL,
    content TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (conversation_id, sequence_no),
    UNIQUE (conversation_id, id),
    FOREIGN KEY (conversation_id, parent_message_id)
        REFERENCES conversation_messages(conversation_id, id) ON DELETE SET NULL (parent_message_id),
    CHECK (role IN ('system', 'user', 'assistant')),
    CHECK (status IN ('pending', 'streaming', 'completed', 'failed'))
);

CREATE INDEX ix_conversation_messages_conversation_created
    ON conversation_messages(conversation_id, created_at);

CREATE TABLE image_tasks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_message_id UUID NOT NULL,
    assistant_message_id UUID NOT NULL UNIQUE,
    model_id UUID NOT NULL,
    provider_id UUID NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
    operation VARCHAR(32) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'pending',
    prompt TEXT NOT NULL,
    negative_prompt TEXT,
    request_params JSONB NOT NULL DEFAULT '{}'::jsonb,
    response_summary JSONB,
    upstream_request_id VARCHAR(256),
    trace_id VARCHAR(64) NOT NULL,
    estimated_cost NUMERIC(18, 6),
    actual_cost NUMERIC(18, 6),
    error_code VARCHAR(128),
    error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (conversation_id, user_message_id)
        REFERENCES conversation_messages(conversation_id, id) ON DELETE CASCADE,
    FOREIGN KEY (conversation_id, assistant_message_id)
        REFERENCES conversation_messages(conversation_id, id) ON DELETE CASCADE,
    FOREIGN KEY (provider_id, model_id)
        REFERENCES models(provider_id, id) ON DELETE RESTRICT,
    CHECK (operation IN ('generation', 'edit')),
    CHECK (status IN ('pending', 'processing', 'succeeded', 'failed', 'cancelled', 'retrying')),
    CHECK (retry_count >= 0),
    CHECK (estimated_cost IS NULL OR estimated_cost >= 0),
    CHECK (actual_cost IS NULL OR actual_cost >= 0)
);

CREATE INDEX ix_image_tasks_user_created ON image_tasks(user_id, created_at DESC);
CREATE INDEX ix_image_tasks_status ON image_tasks(status);
CREATE INDEX ix_image_tasks_provider_model ON image_tasks(provider_id, model_id);
CREATE INDEX ix_image_tasks_conversation_created ON image_tasks(conversation_id, created_at);

CREATE TABLE image_assets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    storage_driver VARCHAR(32) NOT NULL,
    storage_container VARCHAR(255) NOT NULL DEFAULT 'default',
    storage_key TEXT NOT NULL,
    original_filename TEXT,
    mime_type VARCHAR(64) NOT NULL,
    width INTEGER,
    height INTEGER,
    file_size_bytes BIGINT NOT NULL,
    sha256 VARCHAR(64) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (storage_driver, storage_container, storage_key),
    CHECK (storage_driver IN ('local', 's3')),
    CHECK (file_size_bytes > 0),
    CHECK (width IS NULL OR width > 0),
    CHECK (height IS NULL OR height > 0)
);

CREATE INDEX ix_image_assets_owner_created ON image_assets(owner_id, created_at DESC);
CREATE INDEX ix_image_assets_sha256 ON image_assets(sha256);

CREATE TABLE message_image_assets (
    message_id UUID NOT NULL REFERENCES conversation_messages(id) ON DELETE CASCADE,
    asset_id UUID NOT NULL REFERENCES image_assets(id) ON DELETE RESTRICT,
    relation_type VARCHAR(32) NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (message_id, asset_id, relation_type),
    CHECK (relation_type IN ('attachment', 'reference', 'generated')),
    CHECK (sort_order >= 0)
);

CREATE INDEX ix_message_image_assets_asset_id ON message_image_assets(asset_id);

CREATE TABLE task_input_images (
    task_id UUID NOT NULL REFERENCES image_tasks(id) ON DELETE CASCADE,
    asset_id UUID NOT NULL REFERENCES image_assets(id) ON DELETE RESTRICT,
    input_index INTEGER NOT NULL DEFAULT 0,
    input_role VARCHAR(32) NOT NULL DEFAULT 'reference',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (task_id, input_role, input_index),
    CHECK (input_index >= 0),
    CHECK (input_role IN ('source', 'reference', 'mask'))
);

CREATE INDEX ix_task_input_images_asset_id ON task_input_images(asset_id);

CREATE TABLE image_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id UUID NOT NULL REFERENCES image_tasks(id) ON DELETE CASCADE,
    asset_id UUID NOT NULL UNIQUE REFERENCES image_assets(id) ON DELETE RESTRICT,
    result_index INTEGER NOT NULL DEFAULT 0,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (task_id, result_index),
    CHECK (result_index >= 0)
);

CREATE INDEX ix_image_results_task_id ON image_results(task_id);

CREATE TABLE task_events (
    id BIGSERIAL PRIMARY KEY,
    task_id UUID NOT NULL REFERENCES image_tasks(id) ON DELETE CASCADE,
    event_type VARCHAR(64) NOT NULL,
    from_status VARCHAR(32),
    to_status VARCHAR(32),
    event_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (from_status IS NULL OR from_status IN ('pending', 'processing', 'succeeded', 'failed', 'cancelled', 'retrying')),
    CHECK (to_status IS NULL OR to_status IN ('pending', 'processing', 'succeeded', 'failed', 'cancelled', 'retrying'))
);

CREATE INDEX ix_task_events_task_id_id ON task_events(task_id, id);
