use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool, Type};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Type, Serialize, Deserialize, PartialEq, TS)]
#[sqlx(type_name = "chat_session_agent_state", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
#[ts(use_ts_enum)]
pub enum ChatSessionAgentState {
    Idle,
    Running,
    WaitingApproval,
    Dead,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ChatSessionAgent {
    pub id: Uuid,
    pub session_id: Uuid,
    pub agent_id: Uuid,
    pub state: ChatSessionAgentState,
    pub workspace_path: Option<String>,
    pub pty_session_key: Option<String>,
    pub agent_session_id: Option<String>,
    pub agent_message_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChatSessionAgent {
    pub session_id: Uuid,
    pub agent_id: Uuid,
    pub workspace_path: Option<String>,
}

impl ChatSessionAgent {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            ChatSessionAgent,
            r#"SELECT id as "id!: Uuid",
                      session_id as "session_id!: Uuid",
                      agent_id as "agent_id!: Uuid",
                      state as "state!: ChatSessionAgentState",
                      workspace_path,
                      pty_session_key,
                      agent_session_id,
                      agent_message_id,
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM chat_session_agents
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_by_session_and_agent(
        pool: &SqlitePool,
        session_id: Uuid,
        agent_id: Uuid,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            ChatSessionAgent,
            r#"SELECT id as "id!: Uuid",
                      session_id as "session_id!: Uuid",
                      agent_id as "agent_id!: Uuid",
                      state as "state!: ChatSessionAgentState",
                      workspace_path,
                      pty_session_key,
                      agent_session_id,
                      agent_message_id,
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM chat_session_agents
               WHERE session_id = $1 AND agent_id = $2"#,
            session_id,
            agent_id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_all_for_session(
        pool: &SqlitePool,
        session_id: Uuid,
    ) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            ChatSessionAgent,
            r#"SELECT id as "id!: Uuid",
                      session_id as "session_id!: Uuid",
                      agent_id as "agent_id!: Uuid",
                      state as "state!: ChatSessionAgentState",
                      workspace_path,
                      pty_session_key,
                      agent_session_id,
                      agent_message_id,
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM chat_session_agents
               WHERE session_id = $1
               ORDER BY created_at ASC"#,
            session_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateChatSessionAgent,
        id: Uuid,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as!(
            ChatSessionAgent,
            r#"INSERT INTO chat_session_agents (id, session_id, agent_id, workspace_path, state)
               VALUES ($1, $2, $3, $4, 'idle')
               RETURNING id as "id!: Uuid",
                         session_id as "session_id!: Uuid",
                         agent_id as "agent_id!: Uuid",
                         state as "state!: ChatSessionAgentState",
                         workspace_path,
                         pty_session_key,
                         agent_session_id,
                         agent_message_id,
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            data.session_id,
            data.agent_id,
            data.workspace_path
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update_state(
        pool: &SqlitePool,
        id: Uuid,
        state: ChatSessionAgentState,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as!(
            ChatSessionAgent,
            r#"UPDATE chat_session_agents
               SET state = $2,
                   updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id as "id!: Uuid",
                         session_id as "session_id!: Uuid",
                         agent_id as "agent_id!: Uuid",
                         state as "state!: ChatSessionAgentState",
                         workspace_path,
                         pty_session_key,
                         agent_session_id,
                         agent_message_id,
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            state
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update_workspace_path(
        pool: &SqlitePool,
        id: Uuid,
        workspace_path: Option<String>,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as!(
            ChatSessionAgent,
            r#"UPDATE chat_session_agents
               SET workspace_path = $2,
                   updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id as "id!: Uuid",
                         session_id as "session_id!: Uuid",
                         agent_id as "agent_id!: Uuid",
                         state as "state!: ChatSessionAgentState",
                         workspace_path,
                         pty_session_key,
                         agent_session_id,
                         agent_message_id,
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            workspace_path
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update_agent_session_id(
        pool: &SqlitePool,
        id: Uuid,
        agent_session_id: Option<String>,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as!(
            ChatSessionAgent,
            r#"UPDATE chat_session_agents
               SET agent_session_id = $2,
                   updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id as "id!: Uuid",
                         session_id as "session_id!: Uuid",
                         agent_id as "agent_id!: Uuid",
                         state as "state!: ChatSessionAgentState",
                         workspace_path,
                         pty_session_key,
                         agent_session_id,
                         agent_message_id,
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            agent_session_id
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update_agent_message_id(
        pool: &SqlitePool,
        id: Uuid,
        agent_message_id: Option<String>,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as!(
            ChatSessionAgent,
            r#"UPDATE chat_session_agents
               SET agent_message_id = $2,
                   updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id as "id!: Uuid",
                         session_id as "session_id!: Uuid",
                         agent_id as "agent_id!: Uuid",
                         state as "state!: ChatSessionAgentState",
                         workspace_path,
                         pty_session_key,
                         agent_session_id,
                         agent_message_id,
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            agent_message_id
        )
        .fetch_one(pool)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            r#"DELETE FROM chat_session_agents WHERE id = $1"#,
            id
        )
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Clear agent_session_id and agent_message_id for all session agents using a specific agent.
    /// This should be called when the agent's runner_type changes, as the old session IDs
    /// are no longer valid for the new model.
    pub async fn clear_session_ids_for_agent(
        pool: &SqlitePool,
        agent_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            r#"UPDATE chat_session_agents
               SET agent_session_id = NULL,
                   agent_message_id = NULL,
                   updated_at = datetime('now', 'subsec')
               WHERE agent_id = $1
                 AND (agent_session_id IS NOT NULL OR agent_message_id IS NOT NULL)"#,
            agent_id
        )
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }
}
