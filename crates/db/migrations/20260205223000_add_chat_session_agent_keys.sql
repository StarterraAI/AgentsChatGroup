PRAGMA foreign_keys = ON;

ALTER TABLE chat_session_agents ADD COLUMN agent_session_id TEXT;
ALTER TABLE chat_session_agents ADD COLUMN agent_message_id TEXT;
