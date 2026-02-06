use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ChatRun {
    pub id: Uuid,
    pub session_id: Uuid,
    pub session_agent_id: Uuid,
    pub run_index: i64,
    pub run_dir: String,
    pub input_path: Option<String>,
    pub output_path: Option<String>,
    pub raw_log_path: Option<String>,
    pub meta_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChatRun {
    pub session_id: Uuid,
    pub session_agent_id: Uuid,
    pub run_index: i64,
    pub run_dir: String,
    pub input_path: Option<String>,
    pub output_path: Option<String>,
    pub raw_log_path: Option<String>,
    pub meta_path: Option<String>,
}

impl ChatRun {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            ChatRun,
            r#"SELECT id as "id!: Uuid",
                      session_id as "session_id!: Uuid",
                      session_agent_id as "session_agent_id!: Uuid",
                      run_index,
                      run_dir,
                      input_path,
                      output_path,
                      raw_log_path,
                      meta_path,
                      created_at as "created_at!: DateTime<Utc>"
               FROM chat_runs
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_latest_for_session_agent(
        pool: &SqlitePool,
        session_agent_id: Uuid,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            ChatRun,
            r#"SELECT id as "id!: Uuid",
                      session_id as "session_id!: Uuid",
                      session_agent_id as "session_agent_id!: Uuid",
                      run_index,
                      run_dir,
                      input_path,
                      output_path,
                      raw_log_path,
                      meta_path,
                      created_at as "created_at!: DateTime<Utc>"
               FROM chat_runs
               WHERE session_agent_id = $1
               ORDER BY run_index DESC
               LIMIT 1"#,
            session_agent_id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn next_run_index(
        pool: &SqlitePool,
        session_agent_id: Uuid,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query!(
            r#"SELECT COALESCE(MAX(run_index), 0) as "max_index!: i64"
               FROM chat_runs
               WHERE session_agent_id = $1"#,
            session_agent_id
        )
        .fetch_one(pool)
        .await?;

        Ok(row.max_index.saturating_add(1))
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateChatRun,
        id: Uuid,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as!(
            ChatRun,
            r#"INSERT INTO chat_runs
               (id, session_id, session_agent_id, run_index, run_dir, input_path, output_path, raw_log_path, meta_path)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id as "id!: Uuid",
                         session_id as "session_id!: Uuid",
                         session_agent_id as "session_agent_id!: Uuid",
                         run_index,
                         run_dir,
                         input_path,
                         output_path,
                         raw_log_path,
                         meta_path,
                         created_at as "created_at!: DateTime<Utc>""#,
            id,
            data.session_id,
            data.session_agent_id,
            data.run_index,
            data.run_dir,
            data.input_path,
            data.output_path,
            data.raw_log_path,
            data.meta_path
        )
        .fetch_one(pool)
        .await
    }
}
