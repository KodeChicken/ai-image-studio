ALTER TABLE conversation_messages
    DROP CONSTRAINT conversation_messages_status_check,
    ADD CONSTRAINT conversation_messages_status_check
        CHECK (status IN ('pending', 'streaming', 'completed', 'failed', 'cancelled'));
