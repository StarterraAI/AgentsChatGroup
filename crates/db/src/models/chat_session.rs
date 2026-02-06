use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool, Type};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Type, Serialize, Deserialize, PartialEq, TS)]
#[sqlx(type_name = "chat_session_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
#[ts(use_ts_enum)]
pub enum ChatSessionStatus {
    Active,
    Archived,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ChatSession {
    pub id: Uuid,
    pub title: Option<String>,
    pub status: ChatSessionStatus,
    pub summary_text: Option<String>,
    pub archive_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateChatSession {
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
pub struct UpdateChatSession {
    pub title: Option<String>,
    pub status: Option<ChatSessionStatus>,
    pub summary_text: Option<String>,
    pub archive_ref: Option<String>,
}

impl ChatSession {
    pub async fn find_all(
        pool: &SqlitePool,
        status: Option<ChatSessionStatus>,
    ) -> Result<Vec<Self>, sqlx::Error> {
        let sessions = if let Some(status) = status {
            sqlx::query_as!(
                ChatSession,
                r#"SELECT id as "id!: Uuid",
                          title,
                          status as "status!: ChatSessionStatus",
                          summary_text,
                          archive_ref,
                          created_at as "created_at!: DateTime<Utc>",
                          updated_at as "updated_at!: DateTime<Utc>",
                          archived_at as "archived_at: DateTime<Utc>"
                   FROM chat_sessions
                   WHERE status = $1
                   ORDER BY updated_at DESC"#,
                status
            )
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as!(
                ChatSession,
                r#"SELECT id as "id!: Uuid",
                          title,
                          status as "status!: ChatSessionStatus",
                          summary_text,
                          archive_ref,
                          created_at as "created_at!: DateTime<Utc>",
                          updated_at as "updated_at!: DateTime<Utc>",
                          archived_at as "archived_at: DateTime<Utc>"
                   FROM chat_sessions
                   ORDER BY updated_at DESC"#
            )
            .fetch_all(pool)
            .await?
        };

        Ok(sessions)
    }

    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            ChatSession,
            r#"SELECT id as "id!: Uuid",
                      title,
                      status as "status!: ChatSessionStatus",
                      summary_text,
                      archive_ref,
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>",
                      archived_at as "archived_at: DateTime<Utc>"
               FROM chat_sessions
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateChatSession,
        id: Uuid,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as!(
            ChatSession,
            r#"INSERT INTO chat_sessions (id, title, status)
               VALUES ($1, $2, $3)
               RETURNING id as "id!: Uuid",
                         title,
                         status as "status!: ChatSessionStatus",
                         summary_text,
                         archive_ref,
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>",
                         archived_at as "archived_at: DateTime<Utc>""#,
            id,
            data.title,
            ChatSessionStatus::Active
        )
        .fetch_one(pool)
        .await
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        data: &UpdateChatSession,
    ) -> Result<Self, sqlx::Error> {
        let existing = Self::find_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;

        let title = data.title.clone().or(existing.title);
        let status = data.status.clone().unwrap_or(existing.status);
        let summary_text = data.summary_text.clone().or(existing.summary_text);
        let archive_ref = data.archive_ref.clone().or(existing.archive_ref);

        let archived_at = if status == ChatSessionStatus::Archived {
            existing.archived_at.or(Some(Utc::now()))
        } else {
            None
        };

        sqlx::query_as!(
            ChatSession,
            r#"UPDATE chat_sessions
               SET title = $2,
                   status = $3,
                   summary_text = $4,
                   archive_ref = $5,
                   archived_at = $6,
                   updated_at = datetime('now', 'subsec')
               WHERE id = $1
               RETURNING id as "id!: Uuid",
                         title,
                         status as "status!: ChatSessionStatus",
                         summary_text,
                         archive_ref,
                         created_at as "created_at!: DateTime<Utc>",
                         updated_at as "updated_at!: DateTime<Utc>",
                         archived_at as "archived_at: DateTime<Utc>""#,
            id,
            title,
            status,
            summary_text,
            archive_ref,
            archived_at
        )
        .fetch_one(pool)
        .await
    }

    pub async fn touch(pool: &SqlitePool, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE chat_sessions SET updated_at = datetime('now', 'subsec') WHERE id = $1",
            id
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("DELETE FROM chat_sessions WHERE id = $1", id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}
