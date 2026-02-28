PRAGMA foreign_keys = ON;

CREATE TABLE chat_session_compression_states (
    session_id              BLOB PRIMARY KEY,
    source_fingerprint      TEXT NOT NULL,
    source_message_count    INTEGER NOT NULL,
    token_threshold         INTEGER NOT NULL,
    compression_percentage  INTEGER NOT NULL,
    source_token_count      INTEGER NOT NULL,
    effective_token_count   INTEGER NOT NULL,
    compression_type        TEXT NOT NULL
                             CHECK (compression_type IN ('none','ai_summarized','truncated')),
    warning_json            TEXT,
    result_messages_json    TEXT NOT NULL,
    created_at              TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at              TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE
);

CREATE INDEX idx_chat_session_compression_states_updated_at
    ON chat_session_compression_states(updated_at);
