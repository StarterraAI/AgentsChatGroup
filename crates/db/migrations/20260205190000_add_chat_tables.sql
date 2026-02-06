PRAGMA foreign_keys = ON;

CREATE TABLE chat_sessions (
    id            BLOB PRIMARY KEY,
    title         TEXT,
    status        TEXT NOT NULL DEFAULT 'active'
                   CHECK (status IN ('active','archived')),
    summary_text  TEXT,
    archive_ref   TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    archived_at   TEXT
);

CREATE INDEX idx_chat_sessions_status ON chat_sessions(status);
CREATE INDEX idx_chat_sessions_created_at ON chat_sessions(created_at);

CREATE TABLE chat_agents (
    id            BLOB PRIMARY KEY,
    name          TEXT NOT NULL,
    runner_type   TEXT NOT NULL,
    system_prompt TEXT NOT NULL DEFAULT '',
    tools_enabled TEXT NOT NULL DEFAULT '{}',
    created_at    TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE UNIQUE INDEX idx_chat_agents_name ON chat_agents(name);
CREATE INDEX idx_chat_agents_runner_type ON chat_agents(runner_type);

CREATE TABLE chat_session_agents (
    id              BLOB PRIMARY KEY,
    session_id      BLOB NOT NULL,
    agent_id        BLOB NOT NULL,
    state           TEXT NOT NULL DEFAULT 'idle'
                       CHECK (state IN ('idle','running','waiting_approval','dead')),
    workspace_path  TEXT,
    pty_session_key TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (agent_id) REFERENCES chat_agents(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_chat_session_agents_unique
    ON chat_session_agents(session_id, agent_id);
CREATE INDEX idx_chat_session_agents_session_id
    ON chat_session_agents(session_id);
CREATE INDEX idx_chat_session_agents_agent_id
    ON chat_session_agents(agent_id);

CREATE TABLE chat_messages (
    id          BLOB PRIMARY KEY,
    session_id  BLOB NOT NULL,
    sender_type TEXT NOT NULL
                   CHECK (sender_type IN ('user','agent','system')),
    sender_id   BLOB,
    content     TEXT NOT NULL,
    mentions    TEXT NOT NULL DEFAULT '[]',
    meta        TEXT NOT NULL DEFAULT '{}',
    created_at  TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE
);

CREATE INDEX idx_chat_messages_session_created_at
    ON chat_messages(session_id, created_at);
CREATE INDEX idx_chat_messages_sender_id
    ON chat_messages(sender_id);

CREATE TABLE chat_permissions (
    id               BLOB PRIMARY KEY,
    session_id       BLOB NOT NULL,
    session_agent_id BLOB NOT NULL,
    capability       TEXT NOT NULL,
    scope            TEXT NOT NULL DEFAULT '{}',
    ttl_type         TEXT NOT NULL DEFAULT 'once'
                       CHECK (ttl_type IN ('once','time','session')),
    expires_at       TEXT,
    granted_by       TEXT,
    created_at       TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (session_agent_id) REFERENCES chat_session_agents(id) ON DELETE CASCADE
);

CREATE INDEX idx_chat_permissions_session_id
    ON chat_permissions(session_id);
CREATE INDEX idx_chat_permissions_session_agent_id
    ON chat_permissions(session_agent_id);

CREATE TABLE chat_artifacts (
    id         BLOB PRIMARY KEY,
    session_id BLOB NOT NULL,
    name       TEXT NOT NULL,
    path       TEXT NOT NULL,
    type       TEXT NOT NULL,
    created_by BLOB,
    pinned     INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE
);

CREATE INDEX idx_chat_artifacts_session_id
    ON chat_artifacts(session_id);

CREATE TABLE chat_runs (
    id               BLOB PRIMARY KEY,
    session_id       BLOB NOT NULL,
    session_agent_id BLOB NOT NULL,
    run_index        INTEGER NOT NULL,
    run_dir          TEXT NOT NULL,
    input_path       TEXT,
    output_path      TEXT,
    raw_log_path     TEXT,
    meta_path        TEXT,
    created_at       TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (session_agent_id) REFERENCES chat_session_agents(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_chat_runs_unique
    ON chat_runs(session_agent_id, run_index);
CREATE INDEX idx_chat_runs_session_id
    ON chat_runs(session_id);
CREATE INDEX idx_chat_runs_session_agent_id
    ON chat_runs(session_agent_id);
