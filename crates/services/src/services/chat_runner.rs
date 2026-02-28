use std::{
    collections::{HashMap, VecDeque},
    path::{Component, Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::Utc;
use dashmap::DashMap;
use db::{
    DBService,
    models::{
        chat_agent::ChatAgent,
        chat_message::{ChatMessage, ChatSenderType},
        chat_run::{ChatRun, CreateChatRun},
        chat_session::ChatSession,
        chat_session_agent::{ChatSessionAgent, ChatSessionAgentState},
    },
};
use executors::{
    approvals::NoopExecutorApprovalService,
    env::{ExecutionEnv, RepoContext},
    executors::{
        BaseCodingAgent, CancellationToken, ExecutorError, ExecutorExitSignal,
        StandardCodingAgentExecutor,
    },
    logs::{
        NormalizedEntryType, TokenUsageInfo, utils::patch::extract_normalized_entry_from_patch,
    },
    profile::{ExecutorConfigs, ExecutorProfileId, canonical_variant_key},
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    fs,
    io::AsyncWriteExt,
    process::Command,
    sync::{Mutex, broadcast},
};
use tokio_util::io::ReaderStream;
use ts_rs::TS;
use utils::{assets::asset_dir, log_msg::LogMsg, msg_store::MsgStore};
use uuid::Uuid;

use crate::services::chat::{self, ChatServiceError};

const UNTRACKED_FILE_LIMIT: u64 = 1024 * 1024;
const MAX_AGENT_CHAIN_DEPTH: u32 = 5;
const AGENTS_CHATGROUP_HOME_DIR: &str = ".agents-chatgroup";
const AGENTS_CHATGROUP_WORKSPACE_DIR: &str = ".agents_chatgroup";
const RUNS_DIR_NAME: &str = "runs";
const CONTEXT_DIR_NAME: &str = "context";
const LEGACY_COMPACTED_CONTEXT_FILE_NAME: &str = "messages_compacted.background.jsonl";
const RUN_RECORDS_DIR_NAME: &str = "run_records";
const RESERVED_USER_HANDLE: &str = "you";
const EXECUTOR_PROFILE_VARIANT_KEY: &str = "executor_profile_variant";

struct DiffInfo {
    truncated: bool,
}

struct ContextSnapshot {
    workspace_path: PathBuf,
    run_path: PathBuf,
    context_compacted: bool,
    compression_warning: Option<chat::CompressionWarning>,
}

struct ReferenceAttachment {
    name: String,
    mime_type: Option<String>,
    size_bytes: i64,
    kind: String,
    local_path: String,
}

struct ReferenceContext {
    message_id: Uuid,
    sender_label: String,
    sender_type: ChatSenderType,
    created_at: String,
    content: String,
    attachments: Vec<ReferenceAttachment>,
}

struct MessageAttachmentContext {
    message_id: Uuid,
    attachments: Vec<ReferenceAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CompressionWarning {
    pub code: String,
    pub message: String,
    pub split_file_path: String,
}

impl From<chat::CompressionWarning> for CompressionWarning {
    fn from(value: chat::CompressionWarning) -> Self {
        Self {
            code: value.code,
            message: value.message,
            split_file_path: value.split_file_path,
        }
    }
}

#[derive(Debug, Serialize)]
struct SessionAgentSummary {
    session_agent_id: Uuid,
    agent_id: Uuid,
    name: String,
    runner_type: String,
    state: ChatSessionAgentState,
    /// Description of the agent for GROUP_MEMBERS display
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
    tools_enabled: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MentionStatus {
    Received, // Message queued, waiting for agent to be available
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ChatStreamEvent {
    MessageNew {
        message: ChatMessage,
    },
    AgentDelta {
        session_id: Uuid,
        session_agent_id: Uuid,
        agent_id: Uuid,
        run_id: Uuid,
        stream_type: ChatStreamDeltaType,
        content: String,
        delta: bool,
        is_final: bool,
    },
    AgentState {
        session_agent_id: Uuid,
        agent_id: Uuid,
        state: ChatSessionAgentState,
        started_at: Option<chrono::DateTime<Utc>>,
    },
    MentionAcknowledged {
        session_id: Uuid,
        message_id: Uuid,
        mentioned_agent: String,
        agent_id: Uuid,
        status: MentionStatus,
    },
    CompressionWarning {
        session_id: Uuid,
        warning: CompressionWarning,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChatStreamDeltaType {
    Assistant,
    Thinking,
}

#[derive(Debug, Error)]
pub enum ChatRunnerError {
    #[error("chat agent not found: {0}")]
    AgentNotFound(String),
    #[error("unknown runner type: {0}")]
    UnknownRunnerType(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Executor(#[from] ExecutorError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    ChatService(#[from] ChatServiceError),
}

/// Pending message to be processed by an agent
#[derive(Clone, Debug)]
struct PendingMessage {
    session_id: Uuid,
    agent_id: Uuid,
    agent_name: String,
    message: ChatMessage,
}

#[derive(Clone)]
pub struct ChatRunner {
    db: DBService,
    streams: Arc<DashMap<Uuid, broadcast::Sender<ChatStreamEvent>>>,
    // Store cancellation tokens for graceful shutdown, key = session_agent_id
    cancellation_tokens: Arc<DashMap<Uuid, CancellationToken>>,
    // Message queue for each session_agent, keyed by session_agent_id
    // When an agent is running, new messages are queued here and processed after completion
    pending_messages: Arc<DashMap<Uuid, VecDeque<PendingMessage>>>,
    // Session-level background context compaction dedupe.
    // At most one compaction task per session is allowed at a time.
    background_compaction_inflight: Arc<DashMap<Uuid, ()>>,
}

impl ChatRunner {
    pub fn new(db: DBService) -> Self {
        Self {
            db,
            streams: Arc::new(DashMap::new()),
            cancellation_tokens: Arc::new(DashMap::new()),
            pending_messages: Arc::new(DashMap::new()),
            background_compaction_inflight: Arc::new(DashMap::new()),
        }
    }

    pub fn subscribe(&self, session_id: Uuid) -> broadcast::Receiver<ChatStreamEvent> {
        self.sender_for(session_id).subscribe()
    }

    pub fn emit_message_new(&self, session_id: Uuid, message: ChatMessage) {
        self.emit(session_id, ChatStreamEvent::MessageNew { message });
    }

    /// Update the mention_statuses field in a message's meta
    async fn update_mention_status(&self, message_id: Uuid, agent_name: &str, status: &str) {
        // Fetch the current message
        let Ok(Some(message)) = ChatMessage::find_by_id(&self.db.pool, message_id).await else {
            tracing::warn!(
                message_id = %message_id,
                "failed to fetch message for mention status update"
            );
            return;
        };

        // Update the meta with new mention status
        let mut meta = message.meta.0.clone();
        let mention_statuses = meta
            .get_mut("mention_statuses")
            .and_then(|v| v.as_object_mut());

        if let Some(statuses) = mention_statuses {
            statuses.insert(agent_name.to_string(), serde_json::json!(status));
        } else {
            let mut new_statuses = serde_json::Map::new();
            new_statuses.insert(agent_name.to_string(), serde_json::json!(status));
            meta["mention_statuses"] = serde_json::Value::Object(new_statuses);
        }

        // Persist the updated meta
        if let Err(err) = ChatMessage::update_meta(&self.db.pool, message_id, meta).await {
            tracing::warn!(
                message_id = %message_id,
                error = %err,
                "failed to update message mention status"
            );
        }
    }

    fn mention_status_as_str(status: &MentionStatus) -> &'static str {
        match status {
            MentionStatus::Received => "received",
            MentionStatus::Running => "running",
            MentionStatus::Completed => "completed",
            MentionStatus::Failed => "failed",
        }
    }

    async fn set_mention_status(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        agent_name: &str,
        agent_id: Option<Uuid>,
        status: MentionStatus,
    ) {
        self.update_mention_status(message_id, agent_name, Self::mention_status_as_str(&status))
            .await;

        if let Some(agent_id) = agent_id {
            self.emit(
                session_id,
                ChatStreamEvent::MentionAcknowledged {
                    session_id,
                    message_id,
                    mentioned_agent: agent_name.to_string(),
                    agent_id,
                    status,
                },
            );
        }
    }

    async fn report_mention_failure(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        agent_name: &str,
        agent_id: Option<Uuid>,
        reason: impl Into<String>,
    ) {
        let reason = reason.into();
        let compact_reason = reason
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let compact_reason = if compact_reason.is_empty() {
            "Unknown error".to_string()
        } else {
            compact_reason
        };

        self.set_mention_status(
            session_id,
            message_id,
            agent_name,
            agent_id,
            MentionStatus::Failed,
        )
        .await;

        let mut failure_meta = serde_json::json!({
            "mention_failure": {
                "source_message_id": message_id,
                "mentioned_agent": agent_name,
                "reason": compact_reason.clone(),
            }
        });

        if let Some(value) = agent_id {
            failure_meta["mention_failure"]["agent_id"] = serde_json::json!(value);
        }

        let system_content = format!(
            "Agent \"{}\" failed to execute this mention: {}",
            agent_name, compact_reason
        );

        match chat::create_message(
            &self.db.pool,
            session_id,
            ChatSenderType::System,
            None,
            system_content,
            Some(failure_meta),
        )
        .await
        {
            Ok(message) => self.emit_message_new(session_id, message),
            Err(err) => {
                tracing::warn!(
                    session_id = %session_id,
                    message_id = %message_id,
                    agent_name = %agent_name,
                    error = %err,
                    "failed to emit mention failure system message"
                );
            }
        }
    }

    pub async fn handle_message(&self, session: &ChatSession, message: &ChatMessage) {
        self.emit_message_new(session.id, message.clone());

        // Check chain depth to prevent infinite loops
        let chain_depth = self.extract_chain_depth(&message.meta);
        if chain_depth >= MAX_AGENT_CHAIN_DEPTH {
            tracing::warn!(
                session_id = %session.id,
                chain_depth = chain_depth,
                "agent chain depth limit reached; not triggering further agents"
            );
            return;
        }

        let session_id = session.id;
        let mentions = message.mentions.0.clone();
        for mention in mentions {
            if message.sender_type == ChatSenderType::Agent
                && mention.eq_ignore_ascii_case(RESERVED_USER_HANDLE)
            {
                tracing::debug!(
                    session_id = %session_id,
                    message_id = %message.id,
                    mention = mention,
                    "skipping reserved user mention in agent message"
                );
                continue;
            }

            let runner = self.clone();
            let message_clone = message.clone();
            tokio::spawn(async move {
                if let Err(err) = runner
                    .run_agent_for_mention(session_id, &mention, &message_clone)
                    .await
                {
                    tracing::warn!(
                        error = %err,
                        mention = mention,
                        session_id = %session_id,
                        "chat runner failed for mention"
                    );
                }
            });
        }
    }

    fn extract_chain_depth(&self, meta: &sqlx::types::Json<serde_json::Value>) -> u32 {
        meta.get("chain_depth")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(0)
    }

    fn emit(&self, session_id: Uuid, event: ChatStreamEvent) {
        let sender = self.sender_for(session_id);
        let _ = sender.send(event);
    }

    fn sender_for(&self, session_id: Uuid) -> broadcast::Sender<ChatStreamEvent> {
        if let Some(entry) = self.streams.get(&session_id) {
            return entry.clone();
        }

        let (sender, _) = broadcast::channel(1024);
        self.streams.insert(session_id, sender.clone());
        sender
    }

    /// Process the next pending message for a session agent after it becomes idle
    async fn process_pending_queue(&self, session_id: Uuid, session_agent_id: Uuid) {
        // Get the next pending message from the queue
        let pending = self
            .pending_messages
            .get_mut(&session_agent_id)
            .and_then(|mut queue| queue.pop_front());

        if let Some(pending_msg) = pending {
            tracing::info!(
                session_agent_id = %session_agent_id,
                message_id = %pending_msg.message.id,
                agent_name = %pending_msg.agent_name,
                "processing queued message for agent"
            );

            // Process the queued message by calling run_agent_for_mention
            // Use the stored agent_name to find the agent (handles rename gracefully)
            if let Err(err) = self
                .run_agent_for_mention(
                    pending_msg.session_id,
                    &pending_msg.agent_name,
                    &pending_msg.message,
                )
                .await
            {
                tracing::warn!(
                    error = %err,
                    agent_name = %pending_msg.agent_name,
                    session_agent_id = %session_agent_id,
                    "failed to process queued message"
                );
                // Continue processing the rest of the queue
                Box::pin(self.process_pending_queue(session_id, session_agent_id)).await;
            }
        } else {
            // Clean up empty queue entry
            self.pending_messages.remove(&session_agent_id);
        }
    }

    /// Clear all pending messages for a session agent and mark them as failed
    /// Called when an agent fails/dies to prevent messages from being stuck
    async fn clear_pending_queue_on_failure(&self, _session_id: Uuid, session_agent_id: Uuid) {
        // Remove and get all pending messages for this agent
        let pending_messages = self.pending_messages.remove(&session_agent_id);

        if let Some((_, messages)) = pending_messages {
            for pending_msg in messages {
                tracing::info!(
                    session_agent_id = %session_agent_id,
                    message_id = %pending_msg.message.id,
                    agent_name = %pending_msg.agent_name,
                    "marking queued message as failed due to agent failure"
                );

                // Update message meta to show failed status
                self.update_mention_status(
                    pending_msg.message.id,
                    &pending_msg.agent_name,
                    "failed",
                )
                .await;

                // Emit failed event
                self.emit(
                    pending_msg.session_id,
                    ChatStreamEvent::MentionAcknowledged {
                        session_id: pending_msg.session_id,
                        message_id: pending_msg.message.id,
                        mentioned_agent: pending_msg.agent_name.clone(),
                        agent_id: pending_msg.agent_id,
                        status: MentionStatus::Failed,
                    },
                );
            }
        }
    }

    async fn resolve_session_agent_for_mention(
        &self,
        session_id: Uuid,
        mention: &str,
    ) -> Result<Option<(ChatSessionAgent, ChatAgent)>, ChatRunnerError> {
        let session_agents =
            ChatSessionAgent::find_all_for_session(&self.db.pool, session_id).await?;
        if session_agents.is_empty() {
            return Ok(None);
        }

        let agents = ChatAgent::find_all(&self.db.pool).await?;
        let agent_map: HashMap<Uuid, ChatAgent> =
            agents.into_iter().map(|agent| (agent.id, agent)).collect();

        let mut exact_match: Option<(ChatSessionAgent, ChatAgent)> = None;
        let mut ci_match: Option<(ChatSessionAgent, ChatAgent)> = None;

        for session_agent in session_agents {
            let Some(agent) = agent_map.get(&session_agent.agent_id) else {
                tracing::warn!(
                    session_agent_id = %session_agent.id,
                    agent_id = %session_agent.agent_id,
                    "chat session agent missing backing agent"
                );
                continue;
            };

            if agent.name == mention {
                exact_match = Some((session_agent, agent.clone()));
                break;
            }

            if agent.name.eq_ignore_ascii_case(mention) {
                if ci_match.is_some() {
                    tracing::warn!(
                        session_id = %session_id,
                        mention = mention,
                        "multiple session agents matched mention; skipping"
                    );
                    return Ok(None);
                }
                ci_match = Some((session_agent, agent.clone()));
            }
        }

        let Some((session_agent, agent)) = exact_match.or(ci_match) else {
            return Ok(None);
        };

        if session_agent.workspace_path.is_none() {
            let workspace_path = self.build_workspace_path(session_id, agent.id);
            let updated = ChatSessionAgent::update_workspace_path(
                &self.db.pool,
                session_agent.id,
                Some(workspace_path),
            )
            .await?;
            return Ok(Some((updated, agent)));
        }

        Ok(Some((session_agent, agent)))
    }

    async fn run_agent_for_mention(
        &self,
        session_id: Uuid,
        mention: &str,
        source_message: &ChatMessage,
    ) -> Result<(), ChatRunnerError> {
        if source_message.sender_type == ChatSenderType::Agent
            && mention.eq_ignore_ascii_case(RESERVED_USER_HANDLE)
        {
            tracing::debug!(
                session_id = %session_id,
                message_id = %source_message.id,
                mention = mention,
                "skipping reserved user mention in agent message"
            );
            return Ok(());
        }

        let resolved = self
            .resolve_session_agent_for_mention(session_id, mention)
            .await;
        let Some((session_agent, agent)) = (match resolved {
            Ok(value) => value,
            Err(err) => {
                self.report_mention_failure(
                    session_id,
                    source_message.id,
                    mention,
                    None,
                    format!("Failed to resolve mentioned agent: {err}"),
                )
                .await;
                return Err(err);
            }
        }) else {
            if let Some(agent) = ChatAgent::find_by_name(&self.db.pool, mention).await? {
                tracing::debug!(
                    session_id = %session_id,
                    agent_id = %agent.id,
                    mention = mention,
                    "chat session agent not configured; marking mention as failed"
                );
                self.report_mention_failure(
                    session_id,
                    source_message.id,
                    &agent.name,
                    Some(agent.id),
                    "Agent is not configured in this session.",
                )
                .await;
                return Err(ChatRunnerError::AgentNotFound(mention.to_string()));
            }
            self.report_mention_failure(
                session_id,
                source_message.id,
                mention,
                None,
                "Mentioned agent was not found.",
            )
            .await;
            return Err(ChatRunnerError::AgentNotFound(mention.to_string()));
        };

        if source_message.sender_type == ChatSenderType::Agent
            && let Some(sender_id) = source_message.sender_id
            && sender_id == agent.id
        {
            tracing::debug!(
                agent_id = %sender_id,
                mention = mention,
                "skipping self-mention by agent"
            );
            return Ok(());
        }

        if session_agent.state == ChatSessionAgentState::Running {
            // Queue the message for later processing instead of skipping
            tracing::debug!(
                session_agent_id = %session_agent.id,
                agent_id = %agent.id,
                message_id = %source_message.id,
                "chat session agent already running; queueing message for later"
            );

            let pending = PendingMessage {
                session_id,
                agent_id: agent.id,
                agent_name: agent.name.clone(),
                message: source_message.clone(),
            };

            self.pending_messages
                .entry(session_agent.id)
                .or_default()
                .push_back(pending);

            // Emit a "received" status to indicate the message is queued
            self.emit(
                session_id,
                ChatStreamEvent::MentionAcknowledged {
                    session_id,
                    message_id: source_message.id,
                    mentioned_agent: agent.name.clone(),
                    agent_id: agent.id,
                    status: MentionStatus::Received,
                },
            );

            // Persist received status to message meta
            self.update_mention_status(source_message.id, &agent.name, "received")
                .await;

            return Ok(());
        }

        let session_agent = if session_agent.state != ChatSessionAgentState::Running {
            ChatSessionAgent::update_state(
                &self.db.pool,
                session_agent.id,
                ChatSessionAgentState::Running,
            )
            .await?
        } else {
            session_agent
        };

        self.emit(
            session_id,
            ChatStreamEvent::AgentState {
                session_agent_id: session_agent.id,
                agent_id: agent.id,
                state: ChatSessionAgentState::Running,
                started_at: Some(session_agent.updated_at),
            },
        );

        // Emit MentionAcknowledged running event
        self.emit(
            session_id,
            ChatStreamEvent::MentionAcknowledged {
                session_id,
                message_id: source_message.id,
                mentioned_agent: agent.name.clone(),
                agent_id: agent.id,
                status: MentionStatus::Running,
            },
        );

        // Persist running status to message meta
        self.update_mention_status(source_message.id, &agent.name, "running")
            .await;

        let session_agent_id = session_agent.id;
        let agent_id = agent.id;

        let reply_handle = self.resolve_reply_handle(source_message);
        let chain_depth = self.extract_chain_depth(&source_message.meta);

        let result = async {
            let workspace_path = session_agent
                .workspace_path
                .clone()
                .unwrap_or_else(|| self.build_workspace_path(session_id, agent_id));
            fs::create_dir_all(&workspace_path).await?;
            let run_records_dir = Self::workspace_run_records_dir(
                PathBuf::from(&workspace_path).as_path(),
                session_id,
            );
            fs::create_dir_all(&run_records_dir).await?;
            tracing::info!(
                session_id = %session_id,
                workspace_path = %workspace_path,
                runs_dir = %run_records_dir.display(),
                "Using workspace runs directory"
            );

            let run_index = ChatRun::next_run_index(&self.db.pool, session_agent_id).await?;
            let run_id = Uuid::new_v4();
            let run_dir =
                run_records_dir.join(Self::run_records_prefix(session_agent_id, run_index));
            fs::create_dir_all(&run_dir).await?;

            let input_path = run_dir.join("input.md");
            let output_path = run_dir.join("output.md");
            let raw_log_path = run_dir.join("raw.log");
            let meta_path = run_dir.join("meta.json");

            let context_snapshot = self
                .build_context_snapshot(session_id, &workspace_path, &run_dir)
                .await?;
            if let Some(warning) = context_snapshot.compression_warning.clone() {
                self.emit(
                    session_id,
                    ChatStreamEvent::CompressionWarning {
                        session_id,
                        warning: warning.into(),
                    },
                );
            }
            let context_dir = context_snapshot
                .workspace_path
                .parent()
                .map(|path| path.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(&workspace_path));
            let reference_context = self
                .build_reference_context(session_id, source_message, &context_dir)
                .await?;
            let message_attachments = self
                .build_message_attachment_context(source_message, &context_dir)
                .await?;
            let session_agents = self.build_session_agent_summaries(session_id).await?;
            let prompt = self.build_prompt(
                &agent,
                source_message,
                &context_snapshot.workspace_path,
                &session_agents,
                message_attachments.as_ref(),
                reference_context.as_ref(),
            );
            fs::write(&input_path, &prompt).await?;

            let _run = ChatRun::create(
                &self.db.pool,
                &CreateChatRun {
                    session_id,
                    session_agent_id,
                    run_index,
                    run_dir: run_dir.to_string_lossy().to_string(),
                    input_path: Some(input_path.to_string_lossy().to_string()),
                    output_path: Some(output_path.to_string_lossy().to_string()),
                    raw_log_path: Some(raw_log_path.to_string_lossy().to_string()),
                    meta_path: Some(meta_path.to_string_lossy().to_string()),
                },
                run_id,
            )
            .await?;

            let executor_profile_id = self.parse_executor_profile_id(&agent)?;
            let mut executor =
                ExecutorConfigs::get_cached().get_coding_agent_or_default(&executor_profile_id);
            executor.use_approvals(Arc::new(NoopExecutorApprovalService));

            let repo_context = RepoContext::new(PathBuf::from(&workspace_path), Vec::new());
            let mut env = ExecutionEnv::new(repo_context, false, String::new());
            env.insert("VK_CHAT_SESSION_ID", session_id.to_string());
            env.insert("VK_CHAT_AGENT_ID", agent_id.to_string());
            env.insert("VK_CHAT_SESSION_AGENT_ID", session_agent_id.to_string());
            env.insert("VK_CHAT_RUN_ID", run_id.to_string());
            env.insert(
                "VK_CHAT_CONTEXT_PATH",
                context_snapshot
                    .workspace_path
                    .to_string_lossy()
                    .to_string(),
            );
            env.insert(
                "VK_CHAT_CONTEXT_RUN_PATH",
                context_snapshot.run_path.to_string_lossy().to_string(),
            );

            let mut spawned = if session_agent.state != ChatSessionAgentState::Dead {
                if let Some(agent_session_id) = session_agent.agent_session_id.as_deref() {
                    executor
                        .spawn_follow_up(
                            PathBuf::from(&workspace_path).as_path(),
                            &prompt,
                            agent_session_id,
                            session_agent.agent_message_id.as_deref(),
                            &env,
                        )
                        .await?
                } else {
                    executor
                        .spawn(PathBuf::from(&workspace_path).as_path(), &prompt, &env)
                        .await?
                }
            } else {
                executor
                    .spawn(PathBuf::from(&workspace_path).as_path(), &prompt, &env)
                    .await?
            };

            let msg_store = Arc::new(MsgStore::new());
            let raw_log_file = Arc::new(Mutex::new(fs::File::create(&raw_log_path).await?));

            self.spawn_log_forwarders(&mut spawned.child, msg_store.clone(), raw_log_file);
            executor.normalize_logs(msg_store.clone(), PathBuf::from(&workspace_path).as_path());

            let failed_flag = Arc::new(AtomicBool::new(false));

            self.spawn_stream_bridge(
                msg_store.clone(),
                session_id,
                agent_id,
                session_agent_id,
                run_id,
                output_path,
                meta_path,
                PathBuf::from(&workspace_path),
                run_dir,
                Some(reply_handle),
                failed_flag.clone(),
                chain_depth,
                context_snapshot.context_compacted,
                context_snapshot.compression_warning.clone(),
                self.clone(),
                source_message.id,
                agent.name.clone(),
            );

            self.spawn_exit_watcher(
                spawned.child,
                spawned.cancel,
                spawned.exit_signal,
                msg_store,
                failed_flag,
                session_agent_id,
            );

            Ok::<(), ChatRunnerError>(())
        }
        .await;

        if result.is_err() {
            if let Err(err) = &result {
                self.report_mention_failure(
                    session_id,
                    source_message.id,
                    &agent.name,
                    Some(agent_id),
                    format!("Failed to start agent run: {err}"),
                )
                .await;
            }
            let _ = ChatSessionAgent::update_state(
                &self.db.pool,
                session_agent_id,
                ChatSessionAgentState::Dead,
            )
            .await;
            self.emit(
                session_id,
                ChatStreamEvent::AgentState {
                    session_agent_id,
                    agent_id,
                    state: ChatSessionAgentState::Dead,
                    started_at: None,
                },
            );
        }

        result
    }

    fn build_workspace_path(&self, session_id: Uuid, agent_id: Uuid) -> String {
        asset_dir()
            .join("chat")
            .join(format!("session_{session_id}"))
            .join("agents")
            .join(agent_id.to_string())
            .to_string_lossy()
            .to_string()
    }

    fn workspace_runs_dir(workspace_path: &Path, session_id: Uuid) -> PathBuf {
        workspace_path
            .join(AGENTS_CHATGROUP_WORKSPACE_DIR)
            .join(RUNS_DIR_NAME)
            .join(session_id.to_string())
    }

    fn workspace_run_records_dir(workspace_path: &Path, session_id: Uuid) -> PathBuf {
        Self::workspace_runs_dir(workspace_path, session_id).join(RUN_RECORDS_DIR_NAME)
    }

    fn run_records_prefix(session_agent_id: Uuid, run_index: i64) -> String {
        format!("session_agent_{session_agent_id}_run_{run_index:04}")
    }

    fn parse_runner_type(&self, agent: &ChatAgent) -> Result<BaseCodingAgent, ChatRunnerError> {
        let raw = agent.runner_type.trim();
        let normalized = raw.replace(['-', ' '], "_").to_ascii_uppercase();
        BaseCodingAgent::from_str(&normalized)
            .map_err(|_| ChatRunnerError::UnknownRunnerType(raw.to_string()))
    }

    fn parse_executor_profile_id(
        &self,
        agent: &ChatAgent,
    ) -> Result<ExecutorProfileId, ChatRunnerError> {
        let executor = self.parse_runner_type(agent)?;
        let variant = Self::extract_executor_profile_variant(&agent.tools_enabled.0);
        Ok(match variant {
            Some(variant) => ExecutorProfileId::with_variant(executor, variant),
            None => ExecutorProfileId::new(executor),
        })
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

    fn resolve_reply_handle(&self, message: &ChatMessage) -> String {
        let handle = message
            .meta
            .0
            .get("sender_handle")
            .and_then(|value| value.as_str())
            .unwrap_or("you");
        let sanitized = handle
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>();
        if sanitized.is_empty() {
            "you".to_string()
        } else {
            sanitized
        }
    }

    fn apply_reply_prefix(content: &str, handle: Option<&str>) -> String {
        let _ = handle;
        content.to_string()
    }

    async fn capture_git_diff(workspace_path: &Path, run_dir: &Path) -> Option<DiffInfo> {
        let check = Command::new("git")
            .arg("-C")
            .arg(workspace_path)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .await
            .ok()?;

        if !check.status.success() {
            return None;
        }

        let status = Command::new("git")
            .arg("-C")
            .arg(workspace_path)
            .args(["status", "--porcelain"])
            .output()
            .await
            .ok()?;

        if !status.status.success() {
            return None;
        }

        let status_text = String::from_utf8_lossy(&status.stdout);
        let has_tracked_changes = status_text.lines().any(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("??")
        });

        if !has_tracked_changes {
            return None;
        }

        let output = Command::new("git")
            .arg("-C")
            .arg(workspace_path)
            .args(["diff", "--no-color"])
            .output()
            .await
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let diff = String::from_utf8_lossy(&output.stdout).to_string();
        if diff.trim().is_empty() {
            return None;
        }

        let diff_path = run_dir.join("diff.patch");
        if let Err(err) = fs::write(&diff_path, &diff).await {
            tracing::warn!("Failed to write diff patch: {}", err);
            return None;
        }

        // Consider diff truncated if it's over 4KB (for UI display purposes)
        let truncated = diff.len() > 4000;

        Some(DiffInfo { truncated })
    }

    async fn capture_untracked_files(workspace_path: &Path, run_dir: &Path) -> Vec<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(workspace_path)
            .args(["ls-files", "--others", "--exclude-standard"])
            .output()
            .await;

        let output = match output {
            Ok(output) if output.status.success() => output,
            _ => return Vec::new(),
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut files = Vec::new();
        let untracked_dir = run_dir.join("untracked");

        for line in stdout.lines() {
            let rel = line.trim();
            if rel.is_empty() {
                continue;
            }
            if rel == AGENTS_CHATGROUP_HOME_DIR
                || rel.starts_with(&format!("{AGENTS_CHATGROUP_HOME_DIR}/"))
                || rel.starts_with(&format!("{AGENTS_CHATGROUP_HOME_DIR}\\"))
                || rel == AGENTS_CHATGROUP_WORKSPACE_DIR
                || rel.starts_with(&format!("{AGENTS_CHATGROUP_WORKSPACE_DIR}/"))
                || rel.starts_with(&format!("{AGENTS_CHATGROUP_WORKSPACE_DIR}\\"))
            {
                // Skip internal runtime artifacts generated by chat context snapshots.
                continue;
            }
            let rel_path = PathBuf::from(rel);
            if rel_path.is_absolute()
                || rel_path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                continue;
            }

            let src = workspace_path.join(&rel_path);
            let dest = untracked_dir.join(&rel_path);

            if let Some(parent) = dest.parent()
                && let Err(err) = fs::create_dir_all(parent).await
            {
                tracing::warn!("Failed to create untracked dir: {}", err);
                continue;
            }

            match fs::metadata(&src).await {
                Ok(metadata) => {
                    if metadata.len() > UNTRACKED_FILE_LIMIT {
                        let placeholder =
                            format!("File too large to display ({} bytes).", metadata.len());
                        let _ = fs::write(&dest, placeholder).await;
                    } else if let Ok(bytes) = fs::read(&src).await {
                        let content = String::from_utf8_lossy(&bytes).to_string();
                        let _ = fs::write(&dest, content).await;
                    }
                }
                Err(err) => {
                    tracing::warn!("Failed to read untracked file {}: {}", rel, err);
                }
            }

            files.push(rel.to_string());
        }

        files
    }

    async fn build_context_snapshot(
        &self,
        session_id: Uuid,
        workspace_path: &str,
        run_dir: &Path,
    ) -> Result<ContextSnapshot, ChatRunnerError> {
        // Create context directory first (needed for cutoff files)
        let context_dir = PathBuf::from(workspace_path)
            .join(AGENTS_CHATGROUP_WORKSPACE_DIR)
            .join(CONTEXT_DIR_NAME)
            .join(session_id.to_string());
        fs::create_dir_all(&context_dir).await?;
        let legacy_compacted_context_path = context_dir.join(LEGACY_COMPACTED_CONTEXT_FILE_NAME);
        if let Err(err) = fs::remove_file(&legacy_compacted_context_path).await
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                session_id = %session_id,
                error = %err,
                path = %legacy_compacted_context_path.display(),
                "Failed to remove legacy background compacted context file"
            );
        }

        // Main path must never block on summarization: always build full context synchronously.
        let full_context =
            crate::services::chat::build_full_context(&self.db.pool, session_id).await?;
        let jsonl = full_context.jsonl;
        let context_path = context_dir.join("messages.jsonl");
        fs::write(&context_path, jsonl.as_bytes()).await?;
        tracing::info!(
            session_id = %session_id,
            workspace_path = %workspace_path,
            context_path = %context_path.display(),
            "Using workspace context (full, non-blocking)"
        );

        // Kick off background compaction for future runs, without blocking current run.
        self.spawn_background_context_compaction(
            session_id,
            workspace_path.to_string(),
            context_dir.clone(),
        );

        fs::create_dir_all(run_dir).await?;
        let run_context_path = run_dir.join("context.jsonl");
        fs::write(&run_context_path, jsonl.as_bytes()).await?;

        Ok(ContextSnapshot {
            workspace_path: context_path,
            run_path: run_context_path,
            context_compacted: false,
            compression_warning: None,
        })
    }

    fn spawn_background_context_compaction(
        &self,
        session_id: Uuid,
        workspace_path: String,
        context_dir: PathBuf,
    ) {
        if self
            .background_compaction_inflight
            .contains_key(&session_id)
        {
            return;
        }
        self.background_compaction_inflight.insert(session_id, ());

        let runner = self.clone();
        tokio::spawn(async move {
            let workspace_path_buf = PathBuf::from(&workspace_path);
            let result = crate::services::chat::build_compacted_context(
                &runner.db.pool,
                session_id,
                None,
                Some(workspace_path_buf.as_path()),
                Some(context_dir.as_path()),
            )
            .await;

            match result {
                Ok(compacted) => {
                    if compacted.context_compacted {
                        let workspace_context_path = context_dir.join("messages.jsonl");
                        if let Err(err) =
                            fs::write(&workspace_context_path, compacted.jsonl.as_bytes()).await
                        {
                            tracing::warn!(
                                session_id = %session_id,
                                error = %err,
                                path = %workspace_context_path.display(),
                                "Failed to update workspace context with compacted history"
                            );
                        } else {
                            tracing::info!(
                                session_id = %session_id,
                                path = %workspace_context_path.display(),
                                compacted_message_count = compacted.messages.len(),
                                "Background context compaction completed and updated workspace context"
                            );
                        }
                    }

                    if let Some(warning) = compacted.compression_warning {
                        runner.emit(
                            session_id,
                            ChatStreamEvent::CompressionWarning {
                                session_id,
                                warning: warning.into(),
                            },
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        session_id = %session_id,
                        error = %err,
                        "Background context compaction failed"
                    );
                }
            }

            runner.background_compaction_inflight.remove(&session_id);
        });
    }

    async fn build_reference_context(
        &self,
        session_id: Uuid,
        source_message: &ChatMessage,
        context_dir: &Path,
    ) -> Result<Option<ReferenceContext>, ChatRunnerError> {
        let Some(reference_id) = chat::extract_reference_message_id(&source_message.meta.0) else {
            return Ok(None);
        };

        let Some(reference) = ChatMessage::find_by_id(&self.db.pool, reference_id).await? else {
            return Ok(None);
        };

        if reference.session_id != session_id {
            return Ok(None);
        }

        let sender_label = reference
            .meta
            .0
            .get("sender")
            .and_then(|value| value.get("label"))
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string();

        let attachments = chat::extract_attachments(&reference.meta.0);
        let mut reference_attachments = Vec::new();

        if !attachments.is_empty() {
            let reference_dir = context_dir
                .join("references")
                .join(reference_id.to_string());
            fs::create_dir_all(&reference_dir).await?;

            for attachment in attachments {
                let relative = PathBuf::from(&attachment.relative_path);
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|component| matches!(component, Component::ParentDir))
                {
                    continue;
                }

                let source_path = asset_dir().join(&relative);
                let file_name = source_path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| attachment.name.clone());
                let dest_path = reference_dir.join(&file_name);
                let local_path = if fs::copy(&source_path, &dest_path).await.is_ok() {
                    dest_path.to_string_lossy().to_string()
                } else {
                    source_path.to_string_lossy().to_string()
                };

                reference_attachments.push(ReferenceAttachment {
                    name: attachment.name,
                    mime_type: attachment.mime_type,
                    size_bytes: attachment.size_bytes,
                    kind: attachment.kind,
                    local_path,
                });
            }
        }

        Ok(Some(ReferenceContext {
            message_id: reference.id,
            sender_label,
            sender_type: reference.sender_type,
            created_at: reference.created_at.to_rfc3339(),
            content: reference.content,
            attachments: reference_attachments,
        }))
    }

    async fn build_message_attachment_context(
        &self,
        source_message: &ChatMessage,
        context_dir: &Path,
    ) -> Result<Option<MessageAttachmentContext>, ChatRunnerError> {
        let attachments = chat::extract_attachments(&source_message.meta.0);
        if attachments.is_empty() {
            return Ok(None);
        }

        let message_dir = context_dir
            .join("attachments")
            .join(source_message.id.to_string());
        fs::create_dir_all(&message_dir).await?;

        let mut message_attachments = Vec::new();
        for attachment in attachments {
            let relative = PathBuf::from(&attachment.relative_path);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|component| matches!(component, Component::ParentDir))
            {
                continue;
            }

            let source_path = asset_dir().join(&relative);
            let file_name = source_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| attachment.name.clone());
            let dest_path = message_dir.join(&file_name);
            let local_path = if fs::copy(&source_path, &dest_path).await.is_ok() {
                dest_path.to_string_lossy().to_string()
            } else {
                source_path.to_string_lossy().to_string()
            };

            message_attachments.push(ReferenceAttachment {
                name: attachment.name,
                mime_type: attachment.mime_type,
                size_bytes: attachment.size_bytes,
                kind: attachment.kind,
                local_path,
            });
        }

        Ok(Some(MessageAttachmentContext {
            message_id: source_message.id,
            attachments: message_attachments,
        }))
    }

    async fn build_session_agent_summaries(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionAgentSummary>, ChatRunnerError> {
        let session_agents =
            ChatSessionAgent::find_all_for_session(&self.db.pool, session_id).await?;
        if session_agents.is_empty() {
            return Ok(Vec::new());
        }

        let agents = ChatAgent::find_all(&self.db.pool).await?;
        let agent_map: HashMap<Uuid, ChatAgent> =
            agents.into_iter().map(|agent| (agent.id, agent)).collect();

        let mut summaries = Vec::with_capacity(session_agents.len());
        for session_agent in session_agents {
            let Some(agent) = agent_map.get(&session_agent.agent_id) else {
                tracing::warn!(
                    session_agent_id = %session_agent.id,
                    agent_id = %session_agent.agent_id,
                    "chat session agent missing backing agent"
                );
                continue;
            };
            let system_prompt = agent.system_prompt.trim();
            // Extract description from first line of system prompt or use agent name
            let description = if !system_prompt.is_empty() {
                system_prompt
                    .lines()
                    .next()
                    .map(|line| line.trim().to_string())
                    .filter(|s| !s.is_empty())
            } else {
                None
            };
            summaries.push(SessionAgentSummary {
                session_agent_id: session_agent.id,
                agent_id: agent.id,
                name: agent.name.clone(),
                runner_type: agent.runner_type.clone(),
                state: session_agent.state,
                description,
                system_prompt: if system_prompt.is_empty() {
                    None
                } else {
                    Some(system_prompt.to_string())
                },
                tools_enabled: agent.tools_enabled.0.clone(),
            });
        }

        Ok(summaries)
    }

    /// Build the system prompt containing agent role, group members, and critical instructions.
    /// This is separated from the user message for potential future API-level system prompt support.
    fn build_system_prompt(
        &self,
        agent: &ChatAgent,
        session_agents: &[SessionAgentSummary],
        chat_history_path: &Path,
    ) -> String {
        let mut system = String::new();

        // 1. Agent role settings
        if !agent.system_prompt.trim().is_empty() {
            system.push_str("[AGENT_ROLE]\n");
            system.push_str(agent.system_prompt.trim());
            system.push_str("\n[/AGENT_ROLE]\n\n");
        }

        // 2. Group members info (separate from AGENT_ROLE)
        system.push_str("[GROUP_MEMBERS]\n");
        system.push_str("Current AI members in this group:\n");
        if session_agents.is_empty() {
            system.push_str("- No other AI members\n");
        } else {
            for member in session_agents {
                let description = member.description.as_deref().unwrap_or("AI assistant");
                system.push_str(&format!(
                    "- {}: {} (state: {:?})\n",
                    member.name, description, member.state
                ));
            }
        }
        system.push_str("[/GROUP_MEMBERS]\n\n");

        // 3. Message routing protocol
        system.push_str("[MESSAGE_ROUTING]\n");
        system.push_str(
            "When you want to forward work to another member, include an explicit routing marker in your reply:\n",
        );
        system.push_str("[sendMessageTo@@member_name]\n");
        system.push_str("Rules:\n");
        system.push_str("- Only this marker triggers forwarding.\n");
        system.push_str("- Both [sendMessageTo@@member_name] and [sendMessageTo@@{member_name}] are accepted.\n");
        system.push_str("- Plain @member text is normal content and never triggers forwarding.\n");
        system.push_str("- member_name must exactly match a name in [GROUP_MEMBERS].\n");
        system.push_str("- Multiple targets are allowed by adding multiple markers.\n");
        system.push_str("[/MESSAGE_ROUTING]\n\n");

        // 4. Critical instruction to read history file
        system.push_str("[CRITICAL_INSTRUCTION]\n");
        system
            .push_str("Before doing any task, you must first read the group chat history file:\n");
        system.push_str(&format!(
            "file_path: {}\n",
            chat_history_path.to_string_lossy()
        ));
        system.push_str("format: JSON, containing sender and content fields\n");
        system.push_str("This is mandatory: understand group context first, then respond.\n");
        system.push_str("[/CRITICAL_INSTRUCTION]\n");

        system
    }

    /// Build the user message prompt (envelope, reference, attachments, message).
    #[allow(clippy::too_many_arguments)]
    fn build_user_prompt(
        &self,
        agent: &ChatAgent,
        message: &ChatMessage,
        message_attachments: Option<&MessageAttachmentContext>,
        reference: Option<&ReferenceContext>,
    ) -> String {
        let mut prompt = String::new();

        // Envelope metadata
        prompt.push_str("[ENVELOPE]\n");
        prompt.push_str(&format!("session_id={}\n", message.session_id));
        let sender_handle = self.resolve_reply_handle(message);
        prompt.push_str(&format!("from=user:{}\n", sender_handle));
        prompt.push_str(&format!("to=agent:{}\n", agent.name));
        prompt.push_str(&format!("message_id={}\n", message.id));
        prompt.push_str(&format!("timestamp={}\n", message.created_at));
        prompt.push_str("[/ENVELOPE]\n\n");

        // Reference message (if any)
        if let Some(reference) = reference {
            prompt.push_str("[REFERENCE_MESSAGE]\n");
            prompt.push_str(
                "User referenced the following historical group chat message. Prioritize it.\n",
            );
            prompt.push_str(&format!("reference_id={}\n", reference.message_id));
            prompt.push_str(&format!("reference_sender={}\n", reference.sender_label));
            prompt.push_str(&format!(
                "reference_sender_type={:?}\n",
                reference.sender_type
            ));
            prompt.push_str(&format!("reference_created_at={}\n", reference.created_at));
            if !reference.attachments.is_empty() {
                prompt.push_str("reference_attachments:\n");
                for attachment in &reference.attachments {
                    prompt.push_str(&format!(
                        "- name={} kind={} size_bytes={} mime_type={} local_path={}\n",
                        attachment.name,
                        attachment.kind,
                        attachment.size_bytes,
                        attachment.mime_type.as_deref().unwrap_or("unknown"),
                        attachment.local_path
                    ));
                }
            }
            prompt.push_str("reference_content:\n");
            prompt.push_str(reference.content.trim());
            prompt.push_str("\n[/REFERENCE_MESSAGE]\n\n");
        }

        // Message attachments (if any)
        if let Some(message_attachments) = message_attachments
            && !message_attachments.attachments.is_empty()
        {
            prompt.push_str("[MESSAGE_ATTACHMENTS]\n");
            prompt.push_str("Attachments included with this message.\n");
            prompt.push_str(&format!("message_id={}\n", message_attachments.message_id));
            for attachment in &message_attachments.attachments {
                prompt.push_str(&format!(
                    "- name={} kind={} size_bytes={} mime_type={} local_path={}\n",
                    attachment.name,
                    attachment.kind,
                    attachment.size_bytes,
                    attachment.mime_type.as_deref().unwrap_or("unknown"),
                    attachment.local_path
                ));
            }
            prompt.push_str("[/MESSAGE_ATTACHMENTS]\n\n");
        }

        // User message (simplified format: sender + content)
        prompt.push_str("[USER_MESSAGE]\n");
        prompt.push_str(&format!("{}: {}\n", sender_handle, message.content.trim()));
        prompt.push_str("[/USER_MESSAGE]\n");

        prompt
    }

    /// Build the full prompt by combining system prompt and user prompt.
    /// This maintains backwards compatibility while allowing future separation.
    #[allow(clippy::too_many_arguments)]
    fn build_prompt(
        &self,
        agent: &ChatAgent,
        message: &ChatMessage,
        context_path: &Path,
        session_agents: &[SessionAgentSummary],
        message_attachments: Option<&MessageAttachmentContext>,
        reference: Option<&ReferenceContext>,
    ) -> String {
        // Build system prompt with agent role, group members, and history file instruction
        let system_prompt = self.build_system_prompt(agent, session_agents, context_path);

        // Build user prompt with envelope, reference, attachments, and message
        let user_prompt = self.build_user_prompt(agent, message, message_attachments, reference);

        // Combine system and user prompts
        let mut full_prompt = system_prompt;
        full_prompt.push('\n');

        full_prompt.push_str(&user_prompt);
        full_prompt
    }

    fn spawn_log_forwarders(
        &self,
        child: &mut command_group::AsyncGroupChild,
        msg_store: Arc<MsgStore>,
        raw_log_file: Arc<Mutex<fs::File>>,
    ) {
        let stdout = child
            .inner()
            .stdout
            .take()
            .expect("chat runner missing stdout");
        let stderr = child
            .inner()
            .stderr
            .take()
            .expect("chat runner missing stderr");

        let stdout_store = msg_store.clone();
        let stdout_log = raw_log_file.clone();
        tokio::spawn(async move {
            let mut stream = ReaderStream::new(stdout);
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        stdout_store.push(LogMsg::Stdout(text.clone()));
                        let mut file = stdout_log.lock().await;
                        let _ = file.write_all(text.as_bytes()).await;
                    }
                    Err(err) => {
                        stdout_store.push(LogMsg::Stderr(format!("stdout error: {err}")));
                    }
                }
            }
        });

        let stderr_store = msg_store.clone();
        let stderr_log = raw_log_file.clone();
        tokio::spawn(async move {
            let mut stream = ReaderStream::new(stderr);
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        stderr_store.push(LogMsg::Stderr(text.clone()));
                        let mut file = stderr_log.lock().await;
                        let _ = file.write_all(text.as_bytes()).await;
                    }
                    Err(err) => {
                        stderr_store.push(LogMsg::Stderr(format!("stderr error: {err}")));
                    }
                }
            }
        });
    }

    fn parse_token_usage_from_stdout_line(line: &str) -> Option<TokenUsageInfo> {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        let value_obj = value.as_object()?;

        if value_obj.get("type").and_then(|v| v.as_str()) == Some("token_usage") {
            let total_tokens = value_obj
                .get("total_tokens")
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok())?;
            let model_context_window = value_obj
                .get("model_context_window")
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok())?;
            return Some(TokenUsageInfo {
                total_tokens,
                model_context_window,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                is_estimated: false,
            });
        }

        if value_obj.get("method").and_then(|v| v.as_str()) != Some("codex/event/token_count") {
            return None;
        }

        let info = value_obj
            .get("params")
            .and_then(|v| v.get("msg"))
            .and_then(|v| v.get("info"))?;

        let total_tokens = info
            .get("last_token_usage")
            .and_then(|v| v.get("total_tokens"))
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())?;
        let model_context_window = info
            .get("model_context_window")
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);

        Some(TokenUsageInfo {
            total_tokens,
            model_context_window,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            is_estimated: false,
        })
    }

    fn update_token_usage_from_stdout_chunk(
        stdout_line_buffer: &mut String,
        last_token_usage: &mut Option<TokenUsageInfo>,
        chunk: &str,
    ) {
        stdout_line_buffer.push_str(chunk);

        while let Some(newline_index) = stdout_line_buffer.find('\n') {
            let mut line: String = stdout_line_buffer.drain(..=newline_index).collect();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            if line.is_empty() {
                continue;
            }
            if let Some(usage) = Self::parse_token_usage_from_stdout_line(&line) {
                *last_token_usage = Some(usage);
            }
        }
    }

    fn flush_token_usage_buffer(
        stdout_line_buffer: &mut String,
        last_token_usage: &mut Option<TokenUsageInfo>,
    ) {
        if stdout_line_buffer.is_empty() {
            return;
        }
        let line = stdout_line_buffer.trim_end_matches(['\n', '\r']);
        if !line.is_empty()
            && let Some(usage) = Self::parse_token_usage_from_stdout_line(line)
        {
            *last_token_usage = Some(usage);
        }
        stdout_line_buffer.clear();
    }

    /// 浣跨敤tiktoken浼扮畻鏂囨湰鐨則oken鏁伴噺
    fn estimate_tokens_with_tiktoken(text: &str) -> u32 {
        use tiktoken_rs::cl100k_base;

        match cl100k_base() {
            Ok(bpe) => bpe.encode_with_special_tokens(text).len() as u32,
            Err(_) => {
                // fallback: 绾?涓瓧绗︿竴涓猼oken
                (text.len() / 4) as u32
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process_stream_patch(
        patch: json_patch::Patch,
        session_id: Uuid,
        session_agent_id: Uuid,
        agent_id: Uuid,
        run_id: Uuid,
        sender: &broadcast::Sender<ChatStreamEvent>,
        last_content: &mut HashMap<usize, String>,
        latest_assistant: &mut String,
        last_token_usage: &mut Option<TokenUsageInfo>,
    ) {
        if let Some((index, entry)) = extract_normalized_entry_from_patch(&patch) {
            let stream_type = match &entry.entry_type {
                NormalizedEntryType::AssistantMessage => Some(ChatStreamDeltaType::Assistant),
                NormalizedEntryType::Thinking => Some(ChatStreamDeltaType::Thinking),
                NormalizedEntryType::TokenUsageInfo(usage) => {
                    *last_token_usage = Some(usage.clone());
                    None
                }
                _ => None,
            };

            if let Some(stream_type) = stream_type {
                let current = entry.content;
                let previous = last_content.get(&index).cloned().unwrap_or_default();
                let (delta, is_delta) = if current.starts_with(&previous) {
                    (current[previous.len()..].to_string(), true)
                } else {
                    (current.clone(), false)
                };

                last_content.insert(index, current.clone());
                if matches!(stream_type, ChatStreamDeltaType::Assistant) {
                    *latest_assistant = current.clone();
                }

                if !delta.is_empty() {
                    let _ = sender.send(ChatStreamEvent::AgentDelta {
                        session_id,
                        session_agent_id,
                        agent_id,
                        run_id,
                        stream_type,
                        content: delta,
                        delta: is_delta,
                        is_final: false,
                    });
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_stream_bridge(
        &self,
        msg_store: Arc<MsgStore>,
        session_id: Uuid,
        agent_id: Uuid,
        session_agent_id: Uuid,
        run_id: Uuid,
        output_path: PathBuf,
        meta_path: PathBuf,
        workspace_path: PathBuf,
        run_dir: PathBuf,
        reply_handle: Option<String>,
        failed_flag: Arc<AtomicBool>,
        chain_depth: u32,
        context_compacted: bool,
        compression_warning: Option<chat::CompressionWarning>,
        runner: ChatRunner,
        source_message_id: Uuid,
        agent_name: String,
    ) {
        let db = self.db.clone();
        let sender = self.sender_for(session_id);

        tokio::spawn(async move {
            let mut stream = msg_store.history_plus_stream();
            let mut last_content: HashMap<usize, String> = HashMap::new();
            let mut latest_assistant = String::new();
            let mut agent_session_id: Option<String> = None;
            let mut agent_message_id: Option<String> = None;
            let mut last_token_usage: Option<TokenUsageInfo> = None;
            let mut stdout_line_buffer = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(LogMsg::SessionId(session_id_value)) => {
                        if agent_session_id.as_deref() != Some(&session_id_value) {
                            agent_session_id = Some(session_id_value.clone());
                            let _ = ChatSessionAgent::update_agent_session_id(
                                &db.pool,
                                session_agent_id,
                                Some(session_id_value),
                            )
                            .await;
                        }
                    }
                    Ok(LogMsg::MessageId(message_id_value)) => {
                        if agent_message_id.as_deref() != Some(&message_id_value) {
                            agent_message_id = Some(message_id_value.clone());
                            let _ = ChatSessionAgent::update_agent_message_id(
                                &db.pool,
                                session_agent_id,
                                Some(message_id_value),
                            )
                            .await;
                        }
                    }
                    Ok(LogMsg::Stdout(chunk)) => {
                        Self::update_token_usage_from_stdout_chunk(
                            &mut stdout_line_buffer,
                            &mut last_token_usage,
                            &chunk,
                        );
                    }
                    Ok(LogMsg::JsonPatch(patch)) => {
                        Self::process_stream_patch(
                            patch,
                            session_id,
                            session_agent_id,
                            agent_id,
                            run_id,
                            &sender,
                            &mut last_content,
                            &mut latest_assistant,
                            &mut last_token_usage,
                        );
                    }
                    Ok(LogMsg::Finished) => {
                        Self::flush_token_usage_buffer(
                            &mut stdout_line_buffer,
                            &mut last_token_usage,
                        );

                        // Drain tail messages briefly to handle out-of-order `Finished` vs stdout/json patches.
                        let drain_deadline =
                            tokio::time::Instant::now() + std::time::Duration::from_millis(350);
                        loop {
                            let now = tokio::time::Instant::now();
                            if now >= drain_deadline {
                                break;
                            }
                            let remaining = drain_deadline.duration_since(now);
                            let Ok(next_item) =
                                tokio::time::timeout(remaining, stream.next()).await
                            else {
                                break;
                            };
                            let Some(next_item) = next_item else {
                                break;
                            };
                            match next_item {
                                Ok(LogMsg::SessionId(session_id_value)) => {
                                    if agent_session_id.as_deref() != Some(&session_id_value) {
                                        agent_session_id = Some(session_id_value.clone());
                                        let _ = ChatSessionAgent::update_agent_session_id(
                                            &db.pool,
                                            session_agent_id,
                                            Some(session_id_value),
                                        )
                                        .await;
                                    }
                                }
                                Ok(LogMsg::MessageId(message_id_value)) => {
                                    if agent_message_id.as_deref() != Some(&message_id_value) {
                                        agent_message_id = Some(message_id_value.clone());
                                        let _ = ChatSessionAgent::update_agent_message_id(
                                            &db.pool,
                                            session_agent_id,
                                            Some(message_id_value),
                                        )
                                        .await;
                                    }
                                }
                                Ok(LogMsg::Stdout(chunk)) => {
                                    Self::update_token_usage_from_stdout_chunk(
                                        &mut stdout_line_buffer,
                                        &mut last_token_usage,
                                        &chunk,
                                    );
                                }
                                Ok(LogMsg::JsonPatch(patch)) => {
                                    Self::process_stream_patch(
                                        patch,
                                        session_id,
                                        session_agent_id,
                                        agent_id,
                                        run_id,
                                        &sender,
                                        &mut last_content,
                                        &mut latest_assistant,
                                        &mut last_token_usage,
                                    );
                                }
                                _ => {}
                            }
                        }

                        Self::flush_token_usage_buffer(
                            &mut stdout_line_buffer,
                            &mut last_token_usage,
                        );

                        let _ = fs::write(&output_path, &latest_assistant).await;

                        let diff_info =
                            ChatRunner::capture_git_diff(&workspace_path, &run_dir).await;
                        let untracked_files =
                            ChatRunner::capture_untracked_files(&workspace_path, &run_dir).await;
                        let failed = failed_flag.load(Ordering::Relaxed);

                        if failed {
                            agent_session_id = None;
                            agent_message_id = None;
                            let _ = ChatSessionAgent::update_agent_session_id(
                                &db.pool,
                                session_agent_id,
                                None,
                            )
                            .await;
                            let _ = ChatSessionAgent::update_agent_message_id(
                                &db.pool,
                                session_agent_id,
                                None,
                            )
                            .await;
                        }

                        let mut meta = serde_json::json!({
                            "run_id": run_id,
                            "session_id": session_id,
                            "session_agent_id": session_agent_id,
                            "agent_id": agent_id,
                            "agent_session_id": agent_session_id,
                            "agent_message_id": agent_message_id,
                            "finished_at": Utc::now().to_rfc3339(),
                            "chain_depth": chain_depth + 1,
                        });

                        // 濡傛灉娌℃湁token_usage锛屼娇鐢╰iktoken浼扮畻
                        let token_usage = if let Some(ref usage) = last_token_usage {
                            usage.clone()
                        } else {
                            // 璇诲彇input prompt杩涜浼扮畻
                            let input_path = run_dir.join("input.txt");
                            let prompt_content =
                                fs::read_to_string(&input_path).await.unwrap_or_default();
                            let estimated_input =
                                Self::estimate_tokens_with_tiktoken(&prompt_content);
                            let estimated_output =
                                Self::estimate_tokens_with_tiktoken(&latest_assistant);
                            TokenUsageInfo {
                                total_tokens: estimated_input + estimated_output,
                                model_context_window: 0,
                                input_tokens: Some(estimated_input),
                                output_tokens: Some(estimated_output),
                                cache_read_tokens: None,
                                is_estimated: true,
                            }
                        };

                        meta["token_usage"] = serde_json::json!({
                            "total_tokens": token_usage.total_tokens,
                            "model_context_window": token_usage.model_context_window,
                            "input_tokens": token_usage.input_tokens,
                            "output_tokens": token_usage.output_tokens,
                            "is_estimated": token_usage.is_estimated,
                        });

                        if context_compacted {
                            meta["context_compacted"] = true.into();
                        }
                        if let Some(warning) = compression_warning.as_ref() {
                            meta["compression_warning"] = serde_json::json!({
                                "code": warning.code,
                                "message": warning.message,
                                "split_file_path": warning.split_file_path,
                            });
                        }

                        if let Some(diff) = diff_info.as_ref() {
                            meta["diff_available"] = true.into();
                            meta["diff_truncated"] = diff.truncated.into();
                        }

                        if !untracked_files.is_empty() {
                            meta["untracked_files"] =
                                serde_json::to_value(&untracked_files).unwrap_or_default();
                        }

                        let _ = fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap())
                            .await;

                        let final_content = ChatRunner::apply_reply_prefix(
                            &latest_assistant,
                            reply_handle.as_deref(),
                        );

                        if !final_content.trim().is_empty()
                            && let Ok(message) = crate::services::chat::create_message(
                                &db.pool,
                                session_id,
                                ChatSenderType::Agent,
                                Some(agent_id),
                                final_content.clone(),
                                Some(meta.clone()),
                            )
                            .await
                        {
                            // Call handle_message to process explicit routing directives
                            // This enables AI-to-AI message forwarding (chain calls)
                            if let Ok(Some(session)) =
                                ChatSession::find_by_id(&db.pool, session_id).await
                            {
                                runner.handle_message(&session, &message).await;
                            } else {
                                // Fallback: emit MessageNew event if session lookup fails
                                let _ = sender.send(ChatStreamEvent::MessageNew { message });
                            }
                        }

                        let _ = sender.send(ChatStreamEvent::AgentDelta {
                            session_id,
                            session_agent_id,
                            agent_id,
                            run_id,
                            stream_type: ChatStreamDeltaType::Assistant,
                            content: latest_assistant.clone(),
                            delta: false,
                            is_final: true,
                        });

                        let final_state = if failed {
                            ChatSessionAgentState::Dead
                        } else {
                            ChatSessionAgentState::Idle
                        };

                        let _ = ChatSessionAgent::update_state(
                            &db.pool,
                            session_agent_id,
                            final_state.clone(),
                        )
                        .await;

                        let _ = sender.send(ChatStreamEvent::AgentState {
                            session_agent_id,
                            agent_id,
                            state: final_state.clone(),
                            started_at: None,
                        });

                        // Emit MentionAcknowledged completed/failed event
                        let mention_status = if final_state == ChatSessionAgentState::Dead {
                            MentionStatus::Failed
                        } else {
                            MentionStatus::Completed
                        };
                        let _ = sender.send(ChatStreamEvent::MentionAcknowledged {
                            session_id,
                            message_id: source_message_id,
                            mentioned_agent: agent_name.clone(),
                            agent_id,
                            status: mention_status.clone(),
                        });

                        // Persist completed/failed status to message meta
                        let status_str = match mention_status {
                            MentionStatus::Completed => "completed",
                            MentionStatus::Failed => "failed",
                            MentionStatus::Running => "running",
                            MentionStatus::Received => "received",
                        };
                        if let Ok(Some(msg)) =
                            ChatMessage::find_by_id(&db.pool, source_message_id).await
                        {
                            let mut meta = msg.meta.0.clone();
                            let mention_statuses = meta
                                .get_mut("mention_statuses")
                                .and_then(|v| v.as_object_mut());

                            if let Some(statuses) = mention_statuses {
                                statuses.insert(agent_name.clone(), serde_json::json!(status_str));
                            } else {
                                let mut new_statuses = serde_json::Map::new();
                                new_statuses
                                    .insert(agent_name.clone(), serde_json::json!(status_str));
                                meta["mention_statuses"] = serde_json::Value::Object(new_statuses);
                            }

                            let _ =
                                ChatMessage::update_meta(&db.pool, source_message_id, meta).await;
                        }

                        // Process any pending messages in the queue for this agent
                        // Only process if the agent completed successfully (not failed/dead)
                        if final_state == ChatSessionAgentState::Idle {
                            runner
                                .process_pending_queue(session_id, session_agent_id)
                                .await;
                        } else {
                            // Agent failed/died - clear pending queue and mark all as failed
                            runner
                                .clear_pending_queue_on_failure(session_id, session_agent_id)
                                .await;
                        }

                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    fn spawn_exit_watcher(
        &self,
        mut child: command_group::AsyncGroupChild,
        cancel_token: Option<CancellationToken>,
        exit_signal: Option<ExecutorExitSignal>,
        msg_store: Arc<MsgStore>,
        failed_flag: Arc<AtomicBool>,
        session_agent_id: Uuid,
    ) {
        // Store the cancellation token for graceful shutdown
        if let Some(ref token) = cancel_token {
            self.cancellation_tokens
                .insert(session_agent_id, token.clone());
        }

        let finished_sent = Arc::new(AtomicBool::new(false));
        let finished_from_exit_signal = Arc::new(AtomicBool::new(false));
        let cancellation_tokens = self.cancellation_tokens.clone();
        let process_finished = finished_sent.clone();
        let process_finished_from_signal = finished_from_exit_signal.clone();
        let process_msg_store = msg_store.clone();
        let process_failed_flag = failed_flag.clone();
        tokio::spawn(async move {
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            process_failed_flag.store(true, Ordering::Relaxed);
                        }
                        if !process_finished.swap(true, Ordering::Relaxed) {
                            process_msg_store.push_finished();
                        }
                        // If completion already came from exit_signal, token was cleaned there.
                        if !process_finished_from_signal.load(Ordering::Relaxed) {
                            cancellation_tokens.remove(&session_agent_id);
                        }
                        break;
                    }
                    Ok(None) => {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    }
                    Err(err) => {
                        process_msg_store
                            .push(LogMsg::Stderr(format!("process wait error: {err}")));
                        process_failed_flag.store(true, Ordering::Relaxed);
                        if !process_finished.swap(true, Ordering::Relaxed) {
                            process_msg_store.push_finished();
                        }
                        if !process_finished_from_signal.load(Ordering::Relaxed) {
                            cancellation_tokens.remove(&session_agent_id);
                        }
                        break;
                    }
                }
            }
        });

        if let Some(exit_signal_rx) = exit_signal {
            let signal_msg_store = msg_store;
            let signal_failed_flag = failed_flag;
            let signal_finished = finished_sent;
            let signal_finished_from_signal = finished_from_exit_signal;
            let signal_cancel_token = cancel_token;
            let signal_cancellation_tokens = self.cancellation_tokens.clone();
            tokio::spawn(async move {
                match exit_signal_rx.await {
                    Ok(exit_result) => {
                        if matches!(
                            exit_result,
                            executors::executors::ExecutorExitResult::Failure
                        ) {
                            signal_failed_flag.store(true, Ordering::Relaxed);
                        }

                        // Ignore completion emitted after manual cancellation (e.g. stop_agent).
                        let manually_cancelled = signal_cancel_token
                            .as_ref()
                            .map(CancellationToken::is_cancelled)
                            .unwrap_or(false);

                        if !manually_cancelled {
                            signal_finished_from_signal.store(true, Ordering::Relaxed);
                            if !signal_finished.swap(true, Ordering::Relaxed) {
                                signal_msg_store.push_finished();
                            }
                            signal_cancellation_tokens.remove(&session_agent_id);
                        }
                    }
                    Err(err) => {
                        signal_msg_store
                            .push(LogMsg::Stderr(format!("exit signal receive error: {err}")));
                    }
                }
            });
        }
    }

    /// Stop a running agent by triggering graceful cancellation via CancellationToken
    pub async fn stop_agent(
        &self,
        session_id: Uuid,
        session_agent_id: Uuid,
    ) -> Result<(), ChatRunnerError> {
        tracing::info!(
            "stop_agent called for session_agent_id: {}",
            session_agent_id
        );

        // Try to cancel the agent via CancellationToken (graceful shutdown)
        let token_found = self.cancellation_tokens.contains_key(&session_agent_id);
        tracing::info!("CancellationToken found: {}", token_found);

        if let Some(token) = self.cancellation_tokens.get(&session_agent_id) {
            tracing::info!(
                "Cancelling agent for session_agent_id: {}",
                session_agent_id
            );
            token.cancel();
        } else {
            tracing::warn!(
                "No CancellationToken found for session_agent_id: {}",
                session_agent_id
            );
        }

        // Update state to Dead
        let session_agent = ChatSessionAgent::update_state(
            &self.db.pool,
            session_agent_id,
            ChatSessionAgentState::Dead,
        )
        .await?;

        // Emit state change event
        self.emit(
            session_id,
            ChatStreamEvent::AgentState {
                session_agent_id,
                agent_id: session_agent.agent_id,
                state: ChatSessionAgentState::Dead,
                started_at: None,
            },
        );

        // Clean up the cancellation token
        self.cancellation_tokens.remove(&session_agent_id);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ChatRunner;

    #[test]
    fn parse_token_usage_from_codex_token_count_line() {
        let line = r#"{"method":"codex/event/token_count","params":{"msg":{"info":{"last_token_usage":{"total_tokens":53002},"model_context_window":258400}}}}"#;
        let usage = ChatRunner::parse_token_usage_from_stdout_line(line).expect("usage");
        assert_eq!(usage.total_tokens, 53002);
        assert_eq!(usage.model_context_window, 258400);
    }

    #[test]
    fn parse_token_usage_from_plain_token_usage_line() {
        let line = r#"{"type":"token_usage","total_tokens":14596,"model_context_window":258400}"#;
        let usage = ChatRunner::parse_token_usage_from_stdout_line(line).expect("usage");
        assert_eq!(usage.total_tokens, 14596);
        assert_eq!(usage.model_context_window, 258400);
    }
}
