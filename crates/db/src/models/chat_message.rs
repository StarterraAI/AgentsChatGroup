use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool, Type};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Type, Serialize, Deserialize, PartialEq, TS)]
#[sqlx(type_name = "chat_sender_type", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
#[ts(use_ts_enum)]
pub enum ChatSenderType {
    User,
    Agent,
    System,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ChatMessage {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sender_type: ChatSenderType,
    pub sender_id: Option<Uuid>,
    pub content: String,
    #[ts(type = "string[]")]
    pub mentions: sqlx::types::Json<Vec<String>>,
    #[ts(type = "JsonValue")]
    pub meta: sqlx::types::Json<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateChatMessage {
    pub session_id: Uuid,
    pub sender_type: ChatSenderType,
    pub sender_id: Option<Uuid>,
    pub content: String,
    pub mentions: Vec<String>,
    pub meta: serde_json::Value,
}

impl ChatMessage {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            ChatMessage,
            r#"SELECT id as "id!: Uuid",
                      session_id as "session_id!: Uuid",
                      sender_type as "sender_type!: ChatSenderType",
                      sender_id as "sender_id: Uuid",
                      content,
                      mentions as "mentions!: sqlx::types::Json<Vec<String>>",
                      meta as "meta!: sqlx::types::Json<serde_json::Value>",
                      created_at as "created_at!: DateTime<Utc>"
               FROM chat_messages
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_by_session_id(
        pool: &SqlitePool,
        session_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<Self>, sqlx::Error> {
        if let Some(limit) = limit {
            sqlx::query_as!(
                ChatMessage,
                r#"SELECT id as "id!: Uuid",
                          session_id as "session_id!: Uuid",
                          sender_type as "sender_type!: ChatSenderType",
                          sender_id as "sender_id: Uuid",
                          content,
                          mentions as "mentions!: sqlx::types::Json<Vec<String>>",
                          meta as "meta!: sqlx::types::Json<serde_json::Value>",
                          created_at as "created_at!: DateTime<Utc>"
                   FROM chat_messages
                   WHERE session_id = $1
                   ORDER BY created_at ASC
                   LIMIT $2"#,
                session_id,
                limit
            )
            .fetch_all(pool)
            .await
        } else {
            sqlx::query_as!(
                ChatMessage,
                r#"SELECT id as "id!: Uuid",
                          session_id as "session_id!: Uuid",
                          sender_type as "sender_type!: ChatSenderType",
                          sender_id as "sender_id: Uuid",
                          content,
                          mentions as "mentions!: sqlx::types::Json<Vec<String>>",
                          meta as "meta!: sqlx::types::Json<serde_json::Value>",
                          created_at as "created_at!: DateTime<Utc>"
                   FROM chat_messages
                   WHERE session_id = $1
                   ORDER BY created_at ASC"#,
                session_id
            )
            .fetch_all(pool)
            .await
        }
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateChatMessage,
        id: Uuid,
    ) -> Result<Self, sqlx::Error> {
        let mentions_json = sqlx::types::Json(data.mentions.clone());
        let meta_json = sqlx::types::Json(data.meta.clone());

        sqlx::query_as!(
            ChatMessage,
            r#"INSERT INTO chat_messages (id, session_id, sender_type, sender_id, content, mentions, meta)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING id as "id!: Uuid",
                         session_id as "session_id!: Uuid",
                         sender_type as "sender_type!: ChatSenderType",
                         sender_id as "sender_id: Uuid",
                         content,
                         mentions as "mentions!: sqlx::types::Json<Vec<String>>",
                         meta as "meta!: sqlx::types::Json<serde_json::Value>",
                         created_at as "created_at!: DateTime<Utc>""#,
            id,
            data.session_id,
            data.sender_type,
            data.sender_id,
            data.content,
            mentions_json,
            meta_json
        )
        .fetch_one(pool)
        .await
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM chat_messages WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}
