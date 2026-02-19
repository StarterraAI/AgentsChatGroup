use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use chrono::Utc;
use db::models::{
    chat_agent::ChatAgent,
    chat_message::{ChatMessage, ChatSenderType, CreateChatMessage},
    chat_session::{ChatSession, ChatSessionStatus},
    chat_session_agent::ChatSessionAgent,
};
use executors::{
    approvals::NoopExecutorApprovalService,
    env::{ExecutionEnv, RepoContext},
    executors::{BaseCodingAgent, StandardCodingAgentExecutor},
    logs::{NormalizedEntryType, utils::patch::extract_normalized_entry_from_patch},
    profile::{ExecutorConfigs, ExecutorProfileId},
};
use futures::StreamExt;
use moka::sync::Cache;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use thiserror::Error;
use tokio::{fs, io::AsyncWriteExt};
use tokio_util::io::ReaderStream;
use utils::{log_msg::LogMsg, msg_store::MsgStore};
use uuid::Uuid;

/// Maximum number of messages to include in context
const MAX_CONTEXT_MESSAGES: usize = 30;
/// Number of recent messages to keep in full (uncompressed)
const RECENT_MESSAGES_FULL: usize = 5;
/// Target compression ratio for older messages (keep ~40% of content)
const COMPRESSION_TARGET_RATIO: f64 = 0.4;

/// Threshold for triggering LLM-based compression (> 20 messages)
const LLM_COMPRESSION_THRESHOLD: usize = 20;
/// Number of oldest messages to compress via LLM
const LLM_COMPRESSION_BATCH_SIZE: usize = 5;
/// Size of context window to take from recent messages
const CONTEXT_WINDOW_SIZE: usize = 20;

/// Cache for LLM-generated summaries to avoid repeated API calls
/// Key: hash of message IDs being compressed
/// Value: the generated summary text
static SUMMARY_CACHE: Lazy<Cache<String, String>> = Lazy::new(|| {
    Cache::builder()
        .time_to_live(Duration::from_secs(3600)) // 1 hour TTL
        .max_capacity(100) // Max 100 cached summaries
        .build()
});

/// Generate cache key from message IDs
fn summary_cache_key(message_ids: &[Uuid]) -> String {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };
    let mut hasher = DefaultHasher::new();
    for id in message_ids {
        id.hash(&mut hasher);
    }
    format!("summary_{}", hasher.finish())
}

#[derive(Debug, Error)]
pub enum ChatServiceError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Chat session not found")]
    SessionNotFound,
    #[error("Chat session is archived")]
    SessionArchived,
    #[error("Validation error: {0}")]
    Validation(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAttachmentMeta {
    pub id: Uuid,
    pub name: String,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub kind: String,
    pub relative_path: String,
}

pub fn extract_attachments(meta: &Value) -> Vec<ChatAttachmentMeta> {
    meta.get("attachments")
        .and_then(|value| serde_json::from_value::<Vec<ChatAttachmentMeta>>(value.clone()).ok())
        .unwrap_or_default()
}

pub fn has_attachments(meta: &Value) -> bool {
    !extract_attachments(meta).is_empty()
}

pub fn extract_reference_message_id(meta: &Value) -> Option<Uuid> {
    let id = meta
        .get("reference")
        .and_then(|value| value.get("message_id"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            meta.get("reference_message_id")
                .and_then(|value| value.as_str())
        });
    id.and_then(|value| Uuid::parse_str(value).ok())
}

/// Compress message content to reduce token usage
/// Keeps the first part of content and truncates with "..." if too long
fn compress_content(content: &str, max_chars: usize) -> String {
    let content = content.trim();
    if content.chars().count() <= max_chars {
        return content.to_string();
    }

    // Find a good break point (end of sentence or word)
    let chars: Vec<char> = content.chars().collect();
    let mut break_point = max_chars;

    // Try to find sentence end (. ! ?) within the range
    for i in (max_chars / 2..max_chars).rev() {
        if i < chars.len() && matches!(chars[i], '.' | '!' | '?' | '。' | '！' | '？') {
            break_point = i + 1;
            break;
        }
    }

    // If no sentence end, find word boundary
    if break_point == max_chars {
        for i in (max_chars / 2..max_chars).rev() {
            if i < chars.len() && (chars[i].is_whitespace() || chars[i] == ',') {
                break_point = i;
                break;
            }
        }
    }

    let truncated: String = chars[..break_point.min(chars.len())].iter().collect();
    format!("{}...[truncated]", truncated.trim())
}

/// Build structured messages with compression for older messages
/// - Keeps the most recent RECENT_MESSAGES_FULL messages in full
/// - Compresses older messages (from index 6 to the oldest)
/// - Limits total messages to MAX_CONTEXT_MESSAGES
pub async fn build_structured_messages_with_compression(
    pool: &SqlitePool,
    session_id: Uuid,
) -> Result<Vec<Value>, ChatServiceError> {
    let messages = ChatMessage::find_by_session_id(pool, session_id, None).await?;
    let agents = ChatAgent::find_all(pool).await?;
    let agent_map: HashMap<Uuid, String> = agents
        .into_iter()
        .map(|agent| (agent.id, agent.name))
        .collect();

    let total_messages = messages.len();
    // Apply message limit - take the most recent MAX_CONTEXT_MESSAGES
    let messages: Vec<_> = if total_messages > MAX_CONTEXT_MESSAGES {
        messages
            .into_iter()
            .skip(total_messages - MAX_CONTEXT_MESSAGES)
            .collect()
    } else {
        messages
    };

    let message_count = messages.len();
    let mut result = Vec::with_capacity(message_count);

    for (idx, message) in messages.into_iter().enumerate() {
        let sender_handle = message
            .meta
            .0
            .get("sender_handle")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let sender_name = message.sender_id.and_then(|id| agent_map.get(&id).cloned());
        let sender_label = match message.sender_type {
            ChatSenderType::User => sender_handle.clone().unwrap_or_else(|| "user".to_string()),
            ChatSenderType::Agent => sender_name
                .clone()
                .or_else(|| message.sender_id.map(|id| id.to_string()))
                .unwrap_or_else(|| "agent".to_string()),
            ChatSenderType::System => "system".to_string(),
        };

        let sender = serde_json::json!({
            "type": message.sender_type,
            "id": message.sender_id,
            "handle": sender_handle,
            "name": sender_name,
            "label": sender_label,
        });

        // Determine if this message should be compressed
        // Recent messages (last RECENT_MESSAGES_FULL) are kept in full
        let is_recent = idx >= message_count.saturating_sub(RECENT_MESSAGES_FULL);

        let content = if is_recent {
            message.content.clone()
        } else {
            // Compress older messages - target ~40% of original length
            let original_len = message.content.chars().count();
            let target_len = (original_len as f64 * COMPRESSION_TARGET_RATIO) as usize;
            let max_chars = target_len.max(100).min(500); // At least 100 chars, max 500
            compress_content(&message.content, max_chars)
        };

        // For compressed messages, strip meta to save tokens
        let meta = if is_recent {
            message.meta.0.clone()
        } else {
            // Keep only essential meta for compressed messages
            let mut minimal_meta = serde_json::json!({});
            if let Some(sender_info) = message.meta.0.get("sender") {
                minimal_meta["sender"] = sender_info.clone();
            }
            minimal_meta
        };

        result.push(serde_json::json!({
            "id": message.id,
            "session_id": message.session_id,
            "created_at": message.created_at,
            "sender": sender,
            "content": content,
            "mentions": message.mentions.0,
            "meta": meta,
            "compressed": !is_recent,
        }));
    }

    Ok(result)
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
    create_message_with_id(
        pool,
        session_id,
        sender_type,
        sender_id,
        content,
        meta,
        Uuid::new_v4(),
    )
    .await
}

pub async fn create_message_with_id(
    pool: &SqlitePool,
    session_id: Uuid,
    sender_type: ChatSenderType,
    sender_id: Option<Uuid>,
    content: String,
    meta: Option<Value>,
    message_id: Uuid,
) -> Result<ChatMessage, ChatServiceError> {
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
    let mut meta = meta.unwrap_or_else(|| serde_json::json!({}));
    if !meta.is_object() {
        meta = serde_json::json!({ "raw_meta": meta });
    }
    if content.trim().is_empty() && !has_attachments(&meta) {
        return Err(ChatServiceError::Validation(
            "content cannot be empty".to_string(),
        ));
    }

    let sender_handle = meta
        .get("sender_handle")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let sender_name = if matches!(sender_type, ChatSenderType::Agent) {
        if let Some(agent_id) = sender_id {
            ChatAgent::find_by_id(pool, agent_id)
                .await?
                .map(|agent| agent.name)
        } else {
            None
        }
    } else {
        None
    };

    let sender_label = match sender_type {
        ChatSenderType::User => sender_handle.clone().unwrap_or_else(|| "user".to_string()),
        ChatSenderType::Agent => sender_name
            .clone()
            .or_else(|| sender_id.map(|id| id.to_string()))
            .unwrap_or_else(|| "agent".to_string()),
        ChatSenderType::System => "system".to_string(),
    };

    if meta.get("sender").is_none() {
        meta["sender"] = serde_json::json!({
            "type": sender_type,
            "id": sender_id,
            "handle": sender_handle,
            "name": sender_name,
            "label": sender_label,
        });
    }

    meta["structured"] = serde_json::json!({
        "sender_type": sender_type,
        "sender_id": sender_id,
        "sender_handle": sender_handle,
        "sender_label": sender_label,
        "content": content.clone(),
        "mentions": mentions.clone(),
        "created_at": Utc::now().to_rfc3339(),
    });

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
        message_id,
    )
    .await?;

    ChatSession::touch(pool, session_id).await?;

    Ok(message)
}

pub async fn build_structured_messages(
    pool: &SqlitePool,
    session_id: Uuid,
) -> Result<Vec<Value>, ChatServiceError> {
    let messages = ChatMessage::find_by_session_id(pool, session_id, None).await?;
    let agents = ChatAgent::find_all(pool).await?;
    let agent_map: HashMap<Uuid, String> = agents
        .into_iter()
        .map(|agent| (agent.id, agent.name))
        .collect();

    let mut result = Vec::with_capacity(messages.len());

    for message in messages {
        let sender_handle = message
            .meta
            .0
            .get("sender_handle")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let sender_name = message.sender_id.and_then(|id| agent_map.get(&id).cloned());
        let sender_label = match message.sender_type {
            ChatSenderType::User => sender_handle.clone().unwrap_or_else(|| "user".to_string()),
            ChatSenderType::Agent => sender_name
                .clone()
                .or_else(|| message.sender_id.map(|id| id.to_string()))
                .unwrap_or_else(|| "agent".to_string()),
            ChatSenderType::System => "system".to_string(),
        };

        let sender = serde_json::json!({
            "type": message.sender_type,
            "id": message.sender_id,
            "handle": sender_handle,
            "name": sender_name,
            "label": sender_label,
        });

        result.push(serde_json::json!({
            "id": message.id,
            "session_id": message.session_id,
            "created_at": message.created_at,
            "sender": sender,
            "content": message.content,
            "mentions": message.mentions.0,
            "meta": message.meta.0,
        }));
    }

    Ok(result)
}

/// Context with LLM-compressed summary message included
pub struct CompactedContext {
    /// The compacted messages (summary + recent messages)
    pub messages: Vec<Value>,
    /// Raw JSONL string for prompt injection
    pub jsonl: String,
}

/// Build summary prompt for LLM compression
/// This function is used when implementing LLM-based summarization
#[allow(dead_code)]
fn build_summary_prompt(messages_to_summarize: &[Value]) -> String {
    let mut prompt = String::from(
        "Summarize the following chat messages into a single concise summary. \
        Preserve key information including:\n\
        - Tasks/decisions made\n\
        - Constraints and requirements mentioned\n\
        - Names of people/agents mentioned\n\
        - Important references or quotes\n\n\
        Keep the summary under 500 characters. Output plain text only, no formatting.\n\n\
        Messages to summarize:\n",
    );

    for msg in messages_to_summarize {
        // sender is now a string (label) directly, not an object
        let sender_label = msg
            .get("sender")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");
        let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
        prompt.push_str(&format!("{}: {}\n", sender_label, content));
    }

    prompt
}

/// Create a summary message from compressed messages
fn create_summary_message(
    summary_text: &str,
    _compressed_message_ids: Vec<Uuid>,
    earliest_created_at: &str,
) -> Value {
    // Parse the RFC3339 timestamp and format as simple datetime
    let time = chrono::DateTime::parse_from_rfc3339(earliest_created_at)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|_| earliest_created_at.to_string());

    // Only include essential fields: sender, content, compressed, time
    serde_json::json!({
        "sender": "summary",
        "content": summary_text,
        "compressed": true,
        "time": time,
    })
}

/// Simple fallback compression by concatenating messages
fn create_fallback_summary(messages_to_summarize: &[Value]) -> String {
    let mut summary = String::from("[Context Summary] ");
    for msg in messages_to_summarize
        .iter()
        .take(LLM_COMPRESSION_BATCH_SIZE)
    {
        // sender is now a string (label) directly, not an object
        let sender_label = msg
            .get("sender")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");
        let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
        // Keep only first 100 chars per message
        let truncated = if content.len() > 100 {
            format!("{}...", &content[..100])
        } else {
            content.to_string()
        };
        summary.push_str(&format!("{}:{} | ", sender_label, truncated));
    }
    if summary.len() > 500 {
        summary.truncate(500);
        summary.push_str("...");
    }
    summary
}

/// Generate summary using an LLM agent in the background
/// This function spawns a background task and returns immediately
/// The result is stored in the cache and will be available on subsequent calls
async fn generate_llm_summary_background(
    pool: SqlitePool,
    session_id: Uuid,
    message_ids: Vec<Uuid>,
    messages_content: Vec<Value>,
    workspace_path: PathBuf,
) {
    let cache_key = summary_cache_key(&message_ids);

    // Check if already cached
    if SUMMARY_CACHE.get(&cache_key).is_some() {
        return;
    }

    // Spawn background task for LLM summarization
    tokio::spawn(async move {
        match generate_llm_summary_inner(&pool, session_id, &messages_content, &workspace_path)
            .await
        {
            Ok(summary) => {
                tracing::info!(
                    session_id = %session_id,
                    "LLM summary generated successfully, caching result"
                );
                SUMMARY_CACHE.insert(cache_key, summary);
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "Failed to generate LLM summary, will use fallback on next request"
                );
            }
        }
    });
}

/// Inner function to actually call the LLM for summarization
async fn generate_llm_summary_inner(
    pool: &SqlitePool,
    session_id: Uuid,
    messages_content: &[Value],
    workspace_path: &Path,
) -> Result<String, ChatServiceError> {
    // Get the first session agent to use for summarization
    let session_agents = ChatSessionAgent::find_all_for_session(pool, session_id).await?;
    let first_agent = session_agents.first().ok_or_else(|| {
        ChatServiceError::Validation("No agents configured for session".to_string())
    })?;

    // Get the agent details
    let agent = ChatAgent::find_by_id(pool, first_agent.agent_id)
        .await?
        .ok_or_else(|| ChatServiceError::Validation("Agent not found".to_string()))?;

    // Parse runner type
    let runner_type_str = agent.runner_type.trim();
    let normalized = runner_type_str
        .replace('-', "_")
        .replace(' ', "_")
        .to_ascii_uppercase();
    let base_agent = BaseCodingAgent::from_str(&normalized).map_err(|_| {
        ChatServiceError::Validation(format!("Unknown runner type: {}", runner_type_str))
    })?;

    // Build the summary prompt
    let prompt = build_summary_prompt(messages_content);

    // Get executor configuration
    let executor_profile_id = ExecutorProfileId::new(base_agent);
    let mut executor =
        ExecutorConfigs::get_cached().get_coding_agent_or_default(&executor_profile_id);
    executor.use_approvals(Arc::new(NoopExecutorApprovalService::default()));

    // Set up execution environment
    let repo_context = RepoContext::new(workspace_path.to_path_buf(), Vec::new());
    let env = ExecutionEnv::new(repo_context, false, String::new());

    // Spawn the executor
    let mut spawned = executor
        .spawn(workspace_path, &prompt, &env)
        .await
        .map_err(|e| ChatServiceError::Io(std::io::Error::other(e.to_string())))?;

    // Collect output using MsgStore
    let msg_store = Arc::new(MsgStore::new());

    // Set up log forwarders
    if let Some(stdout) = spawned.child.inner().stdout.take() {
        let store_clone = msg_store.clone();
        let reader = ReaderStream::new(stdout);
        tokio::spawn(async move {
            let mut reader = reader;
            while let Some(chunk) = reader.next().await {
                if let Ok(bytes) = chunk {
                    store_clone.push(LogMsg::Stdout(String::from_utf8_lossy(&bytes).to_string()));
                }
            }
        });
    }

    if let Some(stderr) = spawned.child.inner().stderr.take() {
        let store_clone = msg_store.clone();
        let reader = ReaderStream::new(stderr);
        tokio::spawn(async move {
            let mut reader = reader;
            while let Some(chunk) = reader.next().await {
                if let Ok(bytes) = chunk {
                    store_clone.push(LogMsg::Stderr(String::from_utf8_lossy(&bytes).to_string()));
                }
            }
        });
    }

    // Normalize logs to extract assistant messages
    executor.normalize_logs(msg_store.clone(), workspace_path);

    // Wait for process to complete with timeout
    let timeout_duration = Duration::from_secs(60);
    let exit_status = tokio::time::timeout(timeout_duration, spawned.child.wait())
        .await
        .map_err(|_| ChatServiceError::Io(std::io::Error::other("LLM summarization timed out")))?
        .map_err(|e| ChatServiceError::Io(e))?;

    if !exit_status.success() {
        return Err(ChatServiceError::Io(std::io::Error::other(format!(
            "LLM process exited with status: {}",
            exit_status
        ))));
    }

    // Give a moment for logs to be processed
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Extract the last assistant message from the history
    let history = msg_store.get_history();
    for msg in history.iter().rev() {
        if let LogMsg::JsonPatch(patch) = msg {
            if let Some((_, entry)) = extract_normalized_entry_from_patch(patch) {
                if matches!(entry.entry_type, NormalizedEntryType::AssistantMessage) {
                    let content = entry.content.trim();
                    if !content.is_empty() {
                        // Truncate if too long
                        let summary = if content.len() > 500 {
                            format!("{}...", &content[..500])
                        } else {
                            content.to_string()
                        };
                        return Ok(summary);
                    }
                }
            }
        }
    }

    Err(ChatServiceError::Validation(
        "No summary generated from LLM".to_string(),
    ))
}

/// Try to get cached summary or return None
fn get_cached_summary(message_ids: &[Uuid]) -> Option<String> {
    let cache_key = summary_cache_key(message_ids);
    SUMMARY_CACHE.get(&cache_key)
}

/// Build compacted context with LLM-based compression for older messages.
///
/// Algorithm:
/// - If total messages > 20, take the most recent 20 as context window
/// - Compress the oldest 5 messages of this window into 1 summary message via LLM
/// - Final context = 1 summary + 15 recent messages = 16 messages
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `session_id` - Chat session ID
/// * `runner_type` - Runner type string for the agent (e.g., "CLAUDE_CODE", "CODEX")
/// * `workspace_path` - Path to workspace for running LLM
///
/// # Returns
/// CompactedContext with messages and JSONL string
pub async fn build_compacted_context(
    pool: &SqlitePool,
    session_id: Uuid,
    _runner_type: Option<&str>,
    _workspace_path: Option<&std::path::Path>,
) -> Result<CompactedContext, ChatServiceError> {
    // Fetch all messages for the session
    let all_messages = ChatMessage::find_by_session_id(pool, session_id, None).await?;
    let agents = ChatAgent::find_all(pool).await?;
    let agent_map: HashMap<Uuid, String> = agents
        .into_iter()
        .map(|agent| (agent.id, agent.name))
        .collect();

    let total_count = all_messages.len();

    // If total messages <= threshold, return all messages without compression
    if total_count <= LLM_COMPRESSION_THRESHOLD {
        let structured = build_structured_messages_internal(&all_messages, &agent_map, false);
        let jsonl = structured
            .iter()
            .filter_map(|msg| serde_json::to_string(msg).ok())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        return Ok(CompactedContext {
            messages: structured,
            jsonl,
        });
    }

    // Take the most recent CONTEXT_WINDOW_SIZE messages as context window
    let context_window: Vec<_> = all_messages
        .into_iter()
        .skip(total_count.saturating_sub(CONTEXT_WINDOW_SIZE))
        .collect();

    // Split into messages to compress (oldest 5) and messages to keep (remaining 15)
    // But first, filter out any messages that are already summaries (sender_type=System with compression meta)
    // This shouldn't happen if summaries aren't persisted to DB, but we check for robustness
    let split_point = LLM_COMPRESSION_BATCH_SIZE.min(context_window.len());
    let (to_compress_raw, messages_to_keep) = context_window.split_at(split_point);

    // Filter: skip messages that are already compressed summaries
    let messages_to_compress: Vec<&ChatMessage> = to_compress_raw
        .iter()
        .filter(|m| {
            // Check if this is a system message with compression meta (already a summary)
            if m.sender_type == ChatSenderType::System {
                if let Some(compression) = m.meta.0.get("compression") {
                    if compression.as_str() == Some("llm") {
                        tracing::debug!("Skipping already compressed message: {}", m.id);
                        return false;
                    }
                }
            }
            true
        })
        .collect();

    // If no messages to compress (all were already summaries), return without new compression
    if messages_to_compress.is_empty() {
        let structured = build_structured_messages_internal(&context_window, &agent_map, false);
        let jsonl = structured
            .iter()
            .filter_map(|msg| serde_json::to_string(msg).ok())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        return Ok(CompactedContext {
            messages: structured,
            jsonl,
        });
    }

    // Convert Vec<&ChatMessage> to Vec<ChatMessage> for build_structured_messages_internal
    let messages_to_compress_owned: Vec<ChatMessage> =
        messages_to_compress.iter().map(|m| (*m).clone()).collect();

    // Build structured messages for the ones to compress
    let structured_to_compress =
        build_structured_messages_internal(&messages_to_compress_owned, &agent_map, false);

    // Extract message IDs for summary metadata
    let compressed_ids: Vec<Uuid> = messages_to_compress.iter().map(|m| m.id).collect();

    // Get earliest timestamp from compressed messages (messages_to_compress is now Vec<&ChatMessage>)
    let earliest_created_at = messages_to_compress
        .first()
        .map(|m| m.created_at.to_rfc3339())
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    // Try to get cached LLM summary first
    let summary_text = if let Some(cached_summary) = get_cached_summary(&compressed_ids) {
        tracing::debug!(session_id = %session_id, "Using cached LLM summary");
        cached_summary
    } else {
        // No cache hit - spawn background LLM summarization for next time
        // and use fallback for this request (non-blocking)
        if let Some(workspace_path) = _workspace_path {
            let pool_clone = pool.clone();
            let workspace_path_buf = workspace_path.to_path_buf();
            let compressed_ids_clone = compressed_ids.clone();
            let structured_clone = structured_to_compress.clone();

            // Spawn background task - this is fire-and-forget
            generate_llm_summary_background(
                pool_clone,
                session_id,
                compressed_ids_clone,
                structured_clone,
                workspace_path_buf,
            )
            .await;

            tracing::debug!(
                session_id = %session_id,
                "LLM summary generation started in background, using fallback for now"
            );
        }

        // Use fallback for immediate response
        create_fallback_summary(&structured_to_compress)
    };

    // Create summary message
    let summary_message =
        create_summary_message(&summary_text, compressed_ids, &earliest_created_at);

    // Build structured messages for remaining messages
    let structured_to_keep =
        build_structured_messages_internal(&messages_to_keep, &agent_map, false);

    // Combine: summary + remaining messages
    let mut final_messages = vec![summary_message];
    final_messages.extend(structured_to_keep);

    // Build JSONL
    let jsonl = final_messages
        .iter()
        .filter_map(|msg| serde_json::to_string(msg).ok())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    Ok(CompactedContext {
        messages: final_messages,
        jsonl,
    })
}

/// Internal helper to build structured messages without compression
fn build_structured_messages_internal(
    messages: &[ChatMessage],
    agent_map: &HashMap<Uuid, String>,
    apply_compression: bool,
) -> Vec<Value> {
    let message_count = messages.len();
    let mut result = Vec::with_capacity(message_count);

    for (idx, message) in messages.iter().enumerate() {
        // Build sender label from message metadata
        let sender_handle = message
            .meta
            .0
            .get("sender_handle")
            .and_then(|value| value.as_str());
        let sender_name = message.sender_id.and_then(|id| agent_map.get(&id).cloned());
        let sender_label = match message.sender_type {
            ChatSenderType::User => sender_handle
                .map(|s| s.to_string())
                .unwrap_or_else(|| "user".to_string()),
            ChatSenderType::Agent => sender_name
                .or_else(|| message.sender_id.map(|id| id.to_string()))
                .unwrap_or_else(|| "agent".to_string()),
            ChatSenderType::System => "system".to_string(),
        };

        // Determine if this message should be compressed (only if apply_compression is true)
        let is_recent =
            !apply_compression || idx >= message_count.saturating_sub(RECENT_MESSAGES_FULL);

        let content = if is_recent {
            message.content.clone()
        } else {
            let original_len = message.content.chars().count();
            let target_len = (original_len as f64 * COMPRESSION_TARGET_RATIO) as usize;
            let max_chars = target_len.max(100).min(500);
            compress_content(&message.content, max_chars)
        };

        // Only include essential fields: sender, content, compressed, time
        result.push(serde_json::json!({
            "sender": sender_label,
            "content": content,
            "compressed": !is_recent,
            "time": message.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        }));
    }

    result
}

pub async fn export_session_archive(
    pool: &SqlitePool,
    session: &ChatSession,
    archive_dir: &Path,
) -> Result<String, ChatServiceError> {
    fs::create_dir_all(archive_dir).await?;

    let messages = build_structured_messages(pool, session.id).await?;
    let export_path = archive_dir.join("messages_export.jsonl");
    let mut file = fs::File::create(&export_path).await?;
    for message in messages {
        let line = serde_json::to_string(&message).unwrap_or_default();
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
    }

    let summary_path = archive_dir.join("session_summary.md");
    let summary = session
        .summary_text
        .clone()
        .unwrap_or_else(|| "No summary available.".to_string());
    fs::write(&summary_path, summary).await?;

    Ok(archive_dir.to_string_lossy().to_string())
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
