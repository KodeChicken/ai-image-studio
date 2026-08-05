CREATE TABLE image_edit_documents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    source_asset_id UUID NOT NULL REFERENCES image_assets(id) ON DELETE RESTRICT,
    title VARCHAR(256) NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    document_json JSONB NOT NULL,
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (schema_version > 0),
    CHECK (version > 0)
);

CREATE INDEX ix_image_edit_documents_owner_updated
    ON image_edit_documents(owner_id, updated_at DESC);
CREATE INDEX ix_image_edit_documents_source
    ON image_edit_documents(source_asset_id);

ALTER TABLE image_assets
    ADD COLUMN parent_asset_id UUID REFERENCES image_assets(id) ON DELETE RESTRICT,
    ADD COLUMN edit_document_id UUID REFERENCES image_edit_documents(id) ON DELETE SET NULL,
    ADD COLUMN asset_origin VARCHAR(32) NOT NULL DEFAULT 'uploaded',
    ADD CONSTRAINT ck_image_assets_origin
        CHECK (asset_origin IN ('generated', 'uploaded', 'edited', 'ai_edited')),
    ADD CONSTRAINT ck_image_assets_not_own_parent
        CHECK (parent_asset_id IS NULL OR parent_asset_id <> id);

UPDATE image_assets a
SET asset_origin = 'generated'
WHERE EXISTS (SELECT 1 FROM image_results r WHERE r.asset_id = a.id);

CREATE INDEX ix_image_assets_parent ON image_assets(parent_asset_id)
    WHERE parent_asset_id IS NOT NULL;
CREATE INDEX ix_image_assets_edit_document ON image_assets(edit_document_id)
    WHERE edit_document_id IS NOT NULL;

ALTER TABLE image_tasks
    ALTER COLUMN conversation_id DROP NOT NULL,
    ALTER COLUMN user_message_id DROP NOT NULL,
    ALTER COLUMN assistant_message_id DROP NOT NULL,
    ADD COLUMN edit_document_id UUID REFERENCES image_edit_documents(id) ON DELETE CASCADE,
    ADD CONSTRAINT ck_image_tasks_context CHECK (
        (
            edit_document_id IS NULL
            AND conversation_id IS NOT NULL
            AND user_message_id IS NOT NULL
            AND assistant_message_id IS NOT NULL
        ) OR (
            edit_document_id IS NOT NULL
            AND conversation_id IS NULL
            AND user_message_id IS NULL
            AND assistant_message_id IS NULL
        )
    );

CREATE INDEX ix_image_tasks_edit_document_created
    ON image_tasks(edit_document_id, created_at DESC)
    WHERE edit_document_id IS NOT NULL;

UPDATE models m
SET capabilities = m.capabilities || jsonb_build_object(
        'image_edit_capability',
        jsonb_build_object(
            'supportsImageEdit', TRUE,
            'supportsMask', TRUE,
            'supportsOutpaint', TRUE,
            'supportedInputMimeTypes', '["image/png", "image/jpeg", "image/webp"]'::JSONB,
            'supportedOutputSizes', CASE
                WHEN LOWER(m.upstream_model_id) LIKE 'gpt-image-2%' THEN '"custom"'::JSONB
                ELSE COALESCE(m.capabilities->'sizes', '["auto"]'::JSONB)
            END,
            'maxInputImages', CASE
                WHEN jsonb_typeof(m.capabilities->'max_images_per_request') = 'number'
                    THEN (m.capabilities->>'max_images_per_request')::INTEGER
                ELSE 1
            END,
            'maxDimension', CASE
                WHEN LOWER(m.upstream_model_id) LIKE 'gpt-image-2%' THEN 3840
                ELSE 1536
            END
        )
    ),
    updated_at = NOW()
FROM providers p
WHERE p.id = m.provider_id
  AND p.provider_type = 'openai-compatible'
  AND LOWER(m.upstream_model_id) LIKE 'gpt-image-%';

UPDATE models m
SET capabilities = m.capabilities || jsonb_build_object(
        'image_edit_capability',
        jsonb_build_object(
            'supportsImageEdit', TRUE,
            'supportsMask', TRUE,
            'supportsOutpaint', TRUE,
            'supportedInputMimeTypes', '["image/png"]'::JSONB,
            'supportedOutputSizes', '["auto", "256x256", "512x512", "1024x1024"]'::JSONB,
            'maxInputImages', 1,
            'maxDimension', 1024
        )
    ),
    updated_at = NOW()
FROM providers p
WHERE p.id = m.provider_id
  AND p.provider_type = 'openai-compatible'
  AND LOWER(m.upstream_model_id) = 'dall-e-2';
