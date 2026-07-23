CREATE TABLE prompt_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id UUID REFERENCES users(id) ON DELETE CASCADE,
    template_type VARCHAR(32) NOT NULL DEFAULT 'general',
    title VARCHAR(256) NOT NULL,
    prompt TEXT NOT NULL,
    negative_prompt TEXT,
    tags TEXT[] NOT NULL DEFAULT '{}',
    is_public BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (template_type IN ('general', 'style')),
    CHECK (owner_id IS NOT NULL OR is_public)
);

CREATE INDEX ix_prompt_templates_owner_id ON prompt_templates(owner_id);

INSERT INTO prompt_templates (
    id, owner_id, template_type, title, prompt, tags, is_public
) VALUES
    (
        '10000000-0000-4000-8000-000000000001', NULL, 'style', '电影感',
        'cinematic lighting, dramatic composition, subtle film grain, rich color grading',
        ARRAY['cinematic'], TRUE
    ),
    (
        '10000000-0000-4000-8000-000000000002', NULL, 'style', '摄影',
        'professional photography, natural light, realistic detail, balanced exposure',
        ARRAY['photography'], TRUE
    ),
    (
        '10000000-0000-4000-8000-000000000003', NULL, 'style', '插画',
        'editorial illustration, expressive shapes, refined color palette, clean details',
        ARRAY['illustration'], TRUE
    )
ON CONFLICT (id) DO NOTHING;

CREATE TABLE system_settings (
    setting_key VARCHAR(128) PRIMARY KEY,
    value_json JSONB NOT NULL,
    description TEXT,
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
