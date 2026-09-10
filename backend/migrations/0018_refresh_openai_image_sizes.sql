UPDATE models
SET capabilities = capabilities || jsonb_build_object(
        'sizes', '["auto","1024x1024","1536x1024","1024x1536","1536x864","864x1536","2048x2048","2048x1152","1152x2048","3840x2160","2160x3840"]'::jsonb,
        'aspect_ratios', '["auto","1:1","3:2","2:3","16:9","9:16"]'::jsonb,
        'image_edit_capability', COALESCE(capabilities->'image_edit_capability', '{}'::jsonb)
            || '{"supportedOutputSizes":"custom","maxDimension":3840}'::jsonb
    ),
    parameter_schema = parameter_schema || jsonb_build_object(
        'meta', COALESCE(parameter_schema->'meta', '{}'::jsonb) || jsonb_build_object(
            'source', 'official_catalog',
            'modelFamily', 'gpt-image',
            'requestProfile', 'openai-gpt-image-v2',
            'schemaVersion', '2026-09-08',
            'reference', 'https://developers.openai.com/api/docs/guides/image-generation'
        ),
        'parameters', COALESCE(parameter_schema->'parameters', '{}'::jsonb) || jsonb_build_object(
            'aspect_ratio', '{"type":"enum","default":"auto","options":["auto","1:1","3:2","2:3","16:9","9:16"]}'::jsonb,
            'size', '{"type":"enum","default":"auto","options":["auto","1024x1024","1536x1024","1024x1536","1536x864","864x1536","2048x2048","2048x1152","1152x2048","3840x2160","2160x3840"],"allow_custom":true,"constraints":{"edgeMultiple":16,"maxEdge":3840,"minPixels":655360,"maxPixels":8294400,"maxAspectRatio":3}}'::jsonb
        )
    ),
    updated_at = NOW()
WHERE lower(upstream_model_id) ~ '^gpt-image-2($|[.-])'
  AND capability_source <> 'manual_override';
