use std::collections::HashSet;

use db::models::{
    chat_message::{ChatMessage, ChatSenderType, CreateChatMessage},
    chat_session::{ChatSession, ChatSessionStatus},
};
use serde_json::Value;
use sqlx::SqlitePool;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ChatServiceError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("Chat session not found")]
    SessionNotFound,
    #[error("Chat session is archived")]
    SessionArchived,
    #[error("Validation error: {0}")]
    Validation(String),
}

pub fn parse_mentions(content: &str) -> Vec<String> {
    let chars: Vec<char> = content.chars().collect();
    let mut mentions = Vec::new();
    let mut seen = HashSet::new();

    for i in 0..chars.len() {
        if chars[i] != '@' {
            continue;
        }

        if i > 0 {
            let prev = chars[i - 1];
            if prev.is_ascii_alphanumeric() || prev == '_' || prev == '-' || prev == '.' {
                continue;
            }
        }

        let mut name = String::new();
        let mut j = i + 1;
        while j < chars.len() {
            let c = chars[j];
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                name.push(c);
                j += 1;
            } else {
                break;
            }
        }

        if !name.is_empty() && seen.insert(name.clone()) {
            mentions.push(name);
        }
    }

    mentions
}

pub async fn create_message(
    pool: &SqlitePool,
    session_id: Uuid,
    sender_type: ChatSenderType,
    sender_id: Option<Uuid>,
    content: String,
    meta: Option<Value>,
) -> Result<ChatMessage, ChatServiceError> {
    if content.trim().is_empty() {
        return Err(ChatServiceError::Validation(
            "content cannot be empty".to_string(),
        ));
    }

    if matches!(sender_type, ChatSenderType::Agent) && sender_id.is_none() {
        return Err(ChatServiceError::Validation(
            "sender_id is required for agent messages".to_string(),
        ));
    }

    let session = ChatSession::find_by_id(pool, session_id)
        .await?
        .ok_or(ChatServiceError::SessionNotFound)?;

    if session.status != ChatSessionStatus::Active {
        return Err(ChatServiceError::SessionArchived);
    }

    let mentions = parse_mentions(&content);
    let meta = meta.unwrap_or_else(|| serde_json::json!({}));

    let message = ChatMessage::create(
        pool,
        &CreateChatMessage {
            session_id,
            sender_type,
            sender_id,
            content,
            mentions,
            meta,
        },
        Uuid::new_v4(),
    )
    .await?;

    ChatSession::touch(pool, session_id).await?;

    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::parse_mentions;

    #[test]
    fn parses_mentions_with_basic_tokens() {
        let mentions = parse_mentions("@coder please check @planner");
        assert_eq!(mentions, vec!["coder", "planner"]);
    }

    #[test]
    fn ignores_email_addresses() {
        let mentions = parse_mentions("email me at test@example.com");
        assert!(mentions.is_empty());
    }

    #[test]
    fn de_dupes_mentions_in_order() {
        let mentions = parse_mentions("@a @a @b");
        assert_eq!(mentions, vec!["a", "b"]);
    }
}
