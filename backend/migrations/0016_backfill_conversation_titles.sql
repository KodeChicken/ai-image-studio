WITH first_prompts AS (
    SELECT DISTINCT ON (conversation_id)
        conversation_id,
        regexp_replace(btrim(content), '[[:space:]]+', ' ', 'g') AS prompt
    FROM conversation_messages
    WHERE role = 'user'
      AND content IS NOT NULL
      AND btrim(content) <> ''
    ORDER BY conversation_id, sequence_no ASC, created_at ASC
)
UPDATE conversations AS conversation
SET title = CASE
        WHEN char_length(first_prompts.prompt) > 30
            THEN left(first_prompts.prompt, 30) || '…'
        ELSE first_prompts.prompt
    END,
    updated_at = NOW()
FROM first_prompts
WHERE conversation.id = first_prompts.conversation_id
  AND conversation.title IN ('新会话', '新生图会话');
