use std::{
    collections::{HashMap, HashSet},
    path::Path,
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
    executors::{BaseCodingAgent, ExecutorError, ExecutorExitResult, StandardCodingAgentExecutor},
    logs::{NormalizedEntryType, utils::patch::extract_normalized_entry_from_patch},
    profile::{ExecutorConfigs, ExecutorProfileId, canonical_variant_key},
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use thiserror::Error;
use tokio::{fs, io::AsyncWriteExt};
use tokio_util::io::ReaderStream;
use ts_rs::TS;
use utils::{assets::config_path, log_msg::LogMsg, msg_store::MsgStore};
use uuid::Uuid;

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

/// Default token threshold for compression (10,000,000 tokens)
pub const DEFAULT_TOKEN_THRESHOLD: u32 = 10_000_000;
/// Default percentage of messages to compress (25%)
pub const DEFAULT_COMPRESSION_PERCENTAGE: u8 = 25;
const SUMMARY_EXECUTION_TIMEOUT: Duration = Duration::from_secs(120);
const SUMMARY_DRAIN_TIMEOUT: Duration = Duration::from_millis(350);
const EXECUTOR_PROFILE_VARIANT_KEY: &str = "executor_profile_variant";

/// Result of the message compression process
#[derive(Debug, Clone)]
pub struct CompressionResult {
    /// The messages after compression (either with summary or truncated)
    pub messages: Vec<super::chat_history_file::SimplifiedMessage>,
    /// Type of compression that was applied
    pub compression_type: CompressionType,
    /// Warning if compression failed and fallback was used
    pub warning: Option<CompressionWarning>,
}

/// Type of compression that was applied to messages
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompressionType {
    /// No compression needed, messages were under threshold
    None,
    /// AI summarization was successful
    AiSummarized,
    /// All AI agents failed, messages were truncated to split file
    Truncated,
}

/// Warning generated when compression falls back to truncation
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CompressionWarning {
    /// Warning code for programmatic handling
    pub code: String,
    /// Human-readable warning message
    pub message: String,
    /// Path to the split file containing archived messages
    pub split_file_path: String,
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
            if prev.is_alphanumeric() || prev == '_' || prev == '-' || prev == '.' {
                continue;
            }
        }

        let mut name = String::new();
        let mut j = i + 1;
        while j < chars.len() {
            let c = chars[j];
            if c.is_alphanumeric() || c == '_' || c == '-' {
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
    /// Whether context compression has been applied
    pub context_compacted: bool,
    /// Warning if compression fell back to truncation
    pub compression_warning: Option<CompressionWarning>,
}

async fn load_chat_compression_settings() -> (u32, u8) {
    let config = super::config::load_config_from_file(&config_path()).await;
    let threshold = config.chat_compression.token_threshold.max(1);
    let percentage = config.chat_compression.compression_percentage.clamp(1, 100);
    (threshold, percentage)
}

fn simplified_to_context_value(message: &SimplifiedMessage) -> Value {
    let time = chrono::DateTime::parse_from_rfc3339(&message.timestamp)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|_| message.timestamp.clone());

    serde_json::json!({
        "sender": message.sender,
        "content": message.content,
        "time": time,
    })
}

/// Build compacted context with token-threshold based compression only.
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
    workspace_path: Option<&std::path::Path>,
) -> Result<CompactedContext, ChatServiceError> {
    // Fetch all messages for the session
    let all_messages = ChatMessage::find_by_session_id(pool, session_id, None).await?;
    let agents = ChatAgent::find_all(pool).await?;
    let agent_map: HashMap<Uuid, String> = agents
        .into_iter()
        .map(|agent| (agent.id, agent.name))
        .collect();

    let simplified_messages: Vec<SimplifiedMessage> = all_messages
        .iter()
        .map(|message| to_simplified_message(message, &agent_map))
        .collect();
    let session_agents = ChatSessionAgent::find_all_for_session(pool, session_id).await?;
    let (token_threshold, compression_percentage) = load_chat_compression_settings().await;
    let workspace_path = workspace_path.unwrap_or(std::path::Path::new("."));

    let compression_result = compress_messages_if_needed(
        pool,
        session_id,
        simplified_messages,
        token_threshold,
        compression_percentage,
        &session_agents,
        workspace_path,
    )
    .await?;

    let messages: Vec<Value> = compression_result
        .messages
        .iter()
        .map(simplified_to_context_value)
        .collect();
    let jsonl = messages
        .iter()
        .filter_map(|msg| serde_json::to_string(msg).ok())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    Ok(CompactedContext {
        messages,
        jsonl,
        context_compacted: compression_result.compression_type != CompressionType::None,
        compression_warning: compression_result.warning,
    })
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

// ==========================================
// New Token-Based Compression System
// ==========================================

use super::chat_history_file::{SimplifiedMessage, append_to_split_file, estimate_token_count};

/// Convert ChatMessage to SimplifiedMessage format (sender + content only)
pub fn to_simplified_message(
    message: &ChatMessage,
    agent_map: &HashMap<Uuid, String>,
) -> SimplifiedMessage {
    let sender_handle = message
        .meta
        .0
        .get("sender_handle")
        .and_then(|value| value.as_str());
    let sender_name = message.sender_id.and_then(|id| agent_map.get(&id).cloned());

    let sender = match message.sender_type {
        ChatSenderType::User => format!("user:{}", sender_handle.unwrap_or("user")),
        ChatSenderType::Agent => format!(
            "agent:{}",
            sender_name.unwrap_or_else(|| "agent".to_string())
        ),
        ChatSenderType::System => "system".to_string(),
    };

    SimplifiedMessage {
        sender,
        content: message.content.clone(),
        timestamp: message.created_at.to_rfc3339(),
    }
}

/// Convert all messages in a session to SimplifiedMessage format
pub async fn build_simplified_messages(
    pool: &SqlitePool,
    session_id: Uuid,
) -> Result<Vec<SimplifiedMessage>, ChatServiceError> {
    let messages = ChatMessage::find_by_session_id(pool, session_id, None).await?;
    let agents = ChatAgent::find_all(pool).await?;
    let agent_map: HashMap<Uuid, String> = agents
        .into_iter()
        .map(|agent| (agent.id, agent.name))
        .collect();

    Ok(messages
        .iter()
        .map(|msg| to_simplified_message(msg, &agent_map))
        .collect())
}

/// Build the prompt for AI summarization
fn build_summarization_prompt(messages_to_compress: &[SimplifiedMessage]) -> String {
    let mut prompt = String::from(
        "Summarize the following chat history while preserving key tasks, decisions, \
constraints, and references. Keep the summary concise (under 500 words).\n\
Return only the summary body. Do not ask follow-up questions. Do not run any tools or shell commands.\n\nMessages:\n",
    );

    for msg in messages_to_compress {
        prompt.push_str(&format!("{}: {}\n", msg.sender, msg.content));
    }

    prompt
}

/// Try to summarize messages using available AI agents
/// Returns Some(summary) if any agent succeeds, None if all fail
async fn try_summarize_with_agents(
    pool: &SqlitePool,
    session_id: Uuid,
    session_agents: &[ChatSessionAgent],
    messages_to_compress: &[SimplifiedMessage],
    workspace_path: &Path,
) -> Option<String> {
    let summarize_prompt = build_summarization_prompt(messages_to_compress);

    for session_agent in session_agents {
        // Get the agent details
        let agent = match ChatAgent::find_by_id(pool, session_agent.agent_id).await {
            Ok(Some(agent)) => agent,
            _ => continue,
        };

        tracing::debug!(
            "Attempting to summarize with agent: {} ({})",
            agent.name,
            agent.id
        );

        let workspace_override = session_agent.workspace_path.as_deref().map(Path::new);
        let effective_workspace_path = workspace_override.unwrap_or(workspace_path);

        // Try to call the agent for summarization
        match call_agent_for_summary(&agent, &summarize_prompt, effective_workspace_path).await {
            Ok(summary) => {
                tracing::info!(
                    session_id = %session_id,
                    agent = %agent.name,
                    "AI summarization successful"
                );
                return Some(summary);
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    agent = %agent.name,
                    error = %e,
                    "Agent failed to summarize, trying next agent"
                );
                continue;
            }
        }
    }

    tracing::warn!(
        session_id = %session_id,
        "All agents failed to summarize messages"
    );
    None
}

/// Call an agent to generate a summary
/// This spawns a temporary agent process to summarize messages
async fn call_agent_for_summary(
    agent: &ChatAgent,
    prompt: &str,
    workspace_path: &Path,
) -> Result<String, ChatServiceError> {
    let executor_profile_id = parse_executor_profile_id(agent)?;
    let mut executor =
        ExecutorConfigs::get_cached().get_coding_agent_or_default(&executor_profile_id);
    executor.use_approvals(Arc::new(NoopExecutorApprovalService));

    let repo_context = RepoContext::new(workspace_path.to_path_buf(), Vec::new());
    let env = ExecutionEnv::new(repo_context, false, String::new());
    let mut spawned = executor
        .spawn(workspace_path, prompt, &env)
        .await
        .map_err(map_executor_error)?;

    let msg_store = Arc::new(MsgStore::new());
    spawn_summary_log_forwarders(&mut spawned.child, msg_store.clone())?;
    executor.normalize_logs(msg_store.clone(), workspace_path);

    let status = match tokio::time::timeout(SUMMARY_EXECUTION_TIMEOUT, spawned.child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => return Err(ChatServiceError::Io(err)),
        Err(_) => {
            if let Some(cancel) = spawned.cancel.take() {
                cancel.cancel();
            }
            let _ = spawned.child.kill().await;
            return Err(ChatServiceError::Validation(format!(
                "AI summarization timed out for agent {} after {} seconds",
                agent.name,
                SUMMARY_EXECUTION_TIMEOUT.as_secs()
            )));
        }
    };

    let mut failed_by_signal = false;
    if let Some(exit_signal) = spawned.exit_signal.take() {
        match tokio::time::timeout(Duration::from_millis(250), exit_signal).await {
            Ok(Ok(ExecutorExitResult::Failure)) => failed_by_signal = true,
            Ok(Ok(ExecutorExitResult::Success)) => {}
            Ok(Err(err)) => {
                tracing::debug!(
                    agent_name = %agent.name,
                    error = %err,
                    "Failed to receive summarization exit signal"
                );
            }
            Err(_) => {}
        }
    }

    msg_store.push_finished();
    tokio::time::sleep(SUMMARY_DRAIN_TIMEOUT).await;

    if !status.success() || failed_by_signal {
        return Err(ChatServiceError::Validation(format!(
            "AI summarization process failed for agent {}",
            agent.name
        )));
    }

    extract_latest_assistant_from_history(&msg_store.get_history()).ok_or_else(|| {
        ChatServiceError::Validation(format!(
            "No assistant summary output generated by agent {}",
            agent.name
        ))
    })
}

fn parse_runner_type(agent: &ChatAgent) -> Result<BaseCodingAgent, ChatServiceError> {
    let raw = agent.runner_type.trim();
    let normalized = raw.replace(['-', ' '], "_").to_ascii_uppercase();
    BaseCodingAgent::from_str(&normalized)
        .map_err(|_| ChatServiceError::Validation(format!("unknown runner type: {raw}")))
}

fn extract_executor_profile_variant(tools_enabled: &serde_json::Value) -> Option<String> {
    let variant = tools_enabled
        .as_object()
        .and_then(|value| value.get(EXECUTOR_PROFILE_VARIANT_KEY))
        .and_then(serde_json::Value::as_str)?
        .trim();
    if variant.is_empty() || variant.eq_ignore_ascii_case("DEFAULT") {
        return None;
    }
    Some(canonical_variant_key(variant))
}

fn parse_executor_profile_id(agent: &ChatAgent) -> Result<ExecutorProfileId, ChatServiceError> {
    let executor = parse_runner_type(agent)?;
    let variant = extract_executor_profile_variant(&agent.tools_enabled.0);
    Ok(match variant {
        Some(variant) => ExecutorProfileId::with_variant(executor, variant),
        None => ExecutorProfileId::new(executor),
    })
}

fn map_executor_error(err: ExecutorError) -> ChatServiceError {
    ChatServiceError::Validation(format!("executor error: {err}"))
}

fn spawn_summary_log_forwarders(
    child: &mut command_group::AsyncGroupChild,
    msg_store: Arc<MsgStore>,
) -> Result<(), ChatServiceError> {
    let stdout = child.inner().stdout.take().ok_or_else(|| {
        ChatServiceError::Validation("summarization child missing stdout".to_string())
    })?;
    let stderr = child.inner().stderr.take().ok_or_else(|| {
        ChatServiceError::Validation("summarization child missing stderr".to_string())
    })?;

    let stdout_store = msg_store.clone();
    tokio::spawn(async move {
        let mut stream = ReaderStream::new(stdout);
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    stdout_store.push(LogMsg::Stdout(text));
                }
                Err(err) => {
                    stdout_store.push(LogMsg::Stderr(format!("stdout error: {err}")));
                }
            }
        }
    });

    let stderr_store = msg_store;
    tokio::spawn(async move {
        let mut stream = ReaderStream::new(stderr);
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    stderr_store.push(LogMsg::Stderr(text));
                }
                Err(err) => {
                    stderr_store.push(LogMsg::Stderr(format!("stderr error: {err}")));
                }
            }
        }
    });

    Ok(())
}

fn extract_latest_assistant_from_history(history: &[LogMsg]) -> Option<String> {
    let mut assistant_entries: HashMap<usize, String> = HashMap::new();

    for message in history {
        let LogMsg::JsonPatch(patch) = message else {
            continue;
        };

        let Some((index, entry)) = extract_normalized_entry_from_patch(patch) else {
            continue;
        };

        if matches!(entry.entry_type, NormalizedEntryType::AssistantMessage) {
            assistant_entries.insert(index, entry.content);
        }
    }

    assistant_entries
        .into_iter()
        .max_by_key(|(index, _)| *index)
        .map(|(_, content)| content.trim().to_string())
        .filter(|content| !content.is_empty())
}

/// Compress messages if they exceed the token threshold
///
/// This function implements the compression strategy:
/// 1. Calculate total token count using tiktoken
/// 2. If under threshold, return messages unchanged
/// 3. If over threshold:
///    - Calculate 25% of messages to compress
///    - Try AI summarization with each session agent
///    - If all agents fail, truncate to split file and return warning
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `session_id` - Chat session ID
/// * `messages` - Messages to potentially compress
/// * `token_threshold` - Token count that triggers compression
/// * `compression_percentage` - Percentage of messages to compress (default 25)
/// * `session_agents` - AI agents in the session for summarization
/// * `workspace_path` - Workspace path for running agents
pub async fn compress_messages_if_needed(
    pool: &SqlitePool,
    session_id: Uuid,
    messages: Vec<SimplifiedMessage>,
    token_threshold: u32,
    compression_percentage: u8,
    session_agents: &[ChatSessionAgent],
    workspace_path: &Path,
) -> Result<CompressionResult, ChatServiceError> {
    // Calculate token count
    let token_count = estimate_token_count(&messages);

    tracing::debug!(
        session_id = %session_id,
        token_count = token_count,
        threshold = token_threshold,
        "Checking if compression is needed"
    );

    // If under threshold, no compression needed
    if token_count <= token_threshold {
        return Ok(CompressionResult {
            messages,
            compression_type: CompressionType::None,
            warning: None,
        });
    }

    let total_messages = messages.len();
    if total_messages == 0 {
        return Ok(CompressionResult {
            messages,
            compression_type: CompressionType::None,
            warning: None,
        });
    }

    // Calculate number of messages to compress (default 25%)
    let messages_to_compress_count =
        ((total_messages as f64) * (compression_percentage as f64 / 100.0)).ceil() as usize;
    let messages_to_compress_count = messages_to_compress_count.max(1).min(total_messages);

    let (messages_to_compress, messages_to_keep) = messages.split_at(messages_to_compress_count);

    tracing::info!(
        session_id = %session_id,
        total = total_messages,
        to_compress = messages_to_compress_count,
        to_keep = messages_to_keep.len(),
        "Compressing messages"
    );

    // Try AI summarization with available agents
    if !session_agents.is_empty() {
        if let Some(summary) = try_summarize_with_agents(
            pool,
            session_id,
            session_agents,
            messages_to_compress,
            workspace_path,
        )
        .await
        {
            // Create summary message and prepend to kept messages
            let summary_message = SimplifiedMessage {
                sender: "system:summary".to_string(),
                content: format!("[History Summary]\n{}", summary),
                timestamp: Utc::now().to_rfc3339(),
            };

            let mut result_messages = vec![summary_message];
            result_messages.extend(messages_to_keep.to_vec());

            return Ok(CompressionResult {
                messages: result_messages,
                compression_type: CompressionType::AiSummarized,
                warning: None,
            });
        }
    }

    // All agents failed - fallback to truncation
    tracing::warn!(
        session_id = %session_id,
        "AI summarization failed, falling back to truncation"
    );

    // Write messages to split file
    let split_path = append_to_split_file(session_id, messages_to_compress)
        .await
        .map_err(|e| {
            ChatServiceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create split file: {}", e),
            ))
        })?;

    let split_path_str = split_path.to_string_lossy().to_string();

    // Return remaining messages with warning
    Ok(CompressionResult {
        messages: messages_to_keep.to_vec(),
        compression_type: CompressionType::Truncated,
        warning: Some(CompressionWarning {
            code: "COMPRESSION_FALLBACK".to_string(),
            message: format!(
                "AI summarization failed; archived {} messages to split file",
                messages_to_compress_count
            ),
            split_file_path: split_path_str,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CompressionType, SimplifiedMessage, compress_messages_if_needed, parse_mentions,
    };
    use sqlx::SqlitePool;
    use uuid::Uuid;

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

    #[test]
    fn parses_mentions_with_unicode_names() {
        let mentions = parse_mentions(
            "@\u{5C0F}\u{660E} please check @\u{30C6}\u{30B9}\u{30C8}-agent and @\u{0645}\u{0637}\u{0648}\u{0631}_1",
        );
        assert_eq!(
            mentions,
            vec![
                "\u{5C0F}\u{660E}",
                "\u{30C6}\u{30B9}\u{30C8}-agent",
                "\u{0645}\u{0637}\u{0648}\u{0631}_1",
            ]
        );
    }

    #[tokio::test]
    async fn compress_messages_falls_back_to_truncation_without_agents() {
        if dirs::data_dir().is_none() {
            return;
        }

        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("create sqlite memory pool");
        let session_id = Uuid::new_v4();
        let workspace = std::path::Path::new(".");
        let messages = vec![
            SimplifiedMessage {
                sender: "user:alice".to_string(),
                content: "A very long message that should exceed tiny threshold quickly".repeat(8),
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
            SimplifiedMessage {
                sender: "agent:bot".to_string(),
                content: "Second long message for compression coverage".repeat(8),
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
            SimplifiedMessage {
                sender: "user:bob".to_string(),
                content: "Recent message to keep".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
            SimplifiedMessage {
                sender: "agent:bot".to_string(),
                content: "Another recent message to keep".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
        ];

        let result = compress_messages_if_needed(
            &pool,
            session_id,
            messages.clone(),
            1,   // force compression
            50,  // compress half
            &[], // no agents available
            workspace,
        )
        .await
        .expect("compression should succeed with fallback");

        assert_eq!(result.compression_type, CompressionType::Truncated);
        assert_eq!(result.messages.len(), 2);

        let warning = result.warning.expect("fallback should include warning");
        assert_eq!(warning.code, "COMPRESSION_FALLBACK");
        assert!(
            std::path::Path::new(&warning.split_file_path).exists(),
            "split file should be created"
        );

        let _ = tokio::fs::remove_file(&warning.split_file_path).await;
    }

    #[tokio::test]
    async fn compress_messages_keeps_original_when_under_threshold() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("create sqlite memory pool");
        let session_id = Uuid::new_v4();
        let workspace = std::path::Path::new(".");
        let messages = vec![
            SimplifiedMessage {
                sender: "user:alice".to_string(),
                content: "short message".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
            SimplifiedMessage {
                sender: "agent:bot".to_string(),
                content: "another short one".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
        ];

        let result = compress_messages_if_needed(
            &pool,
            session_id,
            messages.clone(),
            u32::MAX, // never trigger compression
            25,
            &[],
            workspace,
        )
        .await
        .expect("compression should pass");

        assert_eq!(result.compression_type, CompressionType::None);
        assert_eq!(result.messages.len(), messages.len());
        assert!(result.warning.is_none());
    }
}
