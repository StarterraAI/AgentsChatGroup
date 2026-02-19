use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ChatAgent {
    pub id: Uuid,
    pub name: String,
    pub runner_type: String,
    pub system_prompt: String,
    #[ts(type = "JsonValue")]
    pub tools_enabled: sqlx::types::Json<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateChatAgent {
    pub name: String,
    pub runner_type: String,
    pub system_prompt: Option<String>,
    pub tools_enabled: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, TS)]
pub struct UpdateChatAgent {
    pub name: Option<String>,
    pub runner_type: Option<String>,
    pub system_prompt: Option<String>,
    pub tools_enabled: Option<serde_json::Value>,
}

impl ChatAgent {
    pub async fn find_all(pool: &SqlitePool) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            ChatAgent,
            r#"SELECT id as "id!: Uuid",
                      name,
                      runner_type,
                      system_prompt,
                      tools_enabled as "tools_enabled!: sqlx::types::Json<serde_json::Value>",
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM chat_agents
               ORDER BY name ASC"#
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            ChatAgent,
            r#"SELECT id as "id!: Uuid",
                      name,
                      runner_type,
                      system_prompt,
                      tools_enabled as "tools_enabled!: sqlx::types::Json<serde_json::Value>",
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM chat_agents
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_by_name(pool: &SqlitePool, name: &str) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            ChatAgent,
            r#"SELECT id as "id!: Uuid",
                      name,
                      runner_type,
                      system_prompt,
                      tools_enabled as "tools_enabled!: sqlx::types::Json<serde_json::Value>",
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM chat_agents
               WHERE lower(name) = lower($1)"#,
            name
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateChatAgent,
        id: Uuid,
    ) -> Result<Self, sqlx::Error> {
        let system_prompt = data.system_prompt.clone().unwrap_or_default();
        let tools_enabled = data
            .tools_enabled
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));

        let tools_enabled_json = sqlx::types::Json(tools_enabled);

        sqlx::query_as!(
            ChatAgent,
            r#"INSERT INTO chat_agents (id, name, runner_type, system_prompt, tools_enabled)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id as "id!: Uuid",
                         name,
                         runner_type,
                         system_prompt,
                         tools_enabled as "tools_enabled!: sqlx::types::Json<serde_json::Value>",
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            data.name,
            data.runner_type,
            system_prompt,
            tools_enabled_json
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateChatAgent,
    ) -> Result<Self, sqlx::Error> {
        let existing = Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;

        let name = data.name.clone().unwrap_or(existing.name);
        let runner_type = data.runner_type.clone().unwrap_or(existing.runner_type);
        let system_prompt = data.system_prompt.clone().unwrap_or(existing.system_prompt);
        let tools_enabled = data
            .tools_enabled
            .clone()
            .unwrap_or(existing.tools_enabled.0);

        let tools_enabled_json = sqlx::types::Json(tools_enabled);

        sqlx::query_as!(
            ChatAgent,
            r#"UPDATE chat_agents
               SET name = $2,
                   runner_type = $3,
                   system_prompt = $4,
                   tools_enabled = $5,
                   updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id as "id!: Uuid",
                         name,
                         runner_type,
                         system_prompt,
                         tools_enabled as "tools_enabled!: sqlx::types::Json<serde_json::Value>",
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>""#,
            id,
            name,
            runner_type,
            system_prompt,
            tools_enabled_json
        )
        .fetch_one(pool)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM chat_agents WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}
