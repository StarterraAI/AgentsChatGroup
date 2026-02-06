use std::{
    collections::HashMap,
    path::PathBuf,
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
        chat_session_agent::{
            ChatSessionAgent, ChatSessionAgentState,
        },
    },
};
use executors::{
    approvals::NoopExecutorApprovalService,
    env::{ExecutionEnv, RepoContext},
    executors::{BaseCodingAgent, ExecutorError, StandardCodingAgentExecutor},
    logs::{NormalizedEntryType, utils::patch::extract_normalized_entry_from_patch},
    profile::{ExecutorConfigs, ExecutorProfileId},
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    fs,
    io::AsyncWriteExt,
    process::Command,
    sync::{broadcast, Mutex},
};
use tokio_util::io::ReaderStream;
use ts_rs::TS;
use utils::{assets::asset_dir, log_msg::LogMsg, msg_store::MsgStore};
use uuid::Uuid;
use crate::services::chat::ChatServiceError;

const DIFF_PREVIEW_LIMIT: usize = 4000;
const UNTRACKED_FILE_LIMIT: u64 = 1024 * 1024;

struct DiffInfo {
    preview: String,
    truncated: bool,
}

struct ContextSnapshot {
    jsonl: String,
    workspace_path: PathBuf,
    run_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ChatStreamEvent {
    MessageNew { message: ChatMessage },
    AgentDelta {
        session_id: Uuid,
        session_agent_id: Uuid,
        agent_id: Uuid,
        run_id: Uuid,
        content: String,
        delta: bool,
        is_final: bool,
    },
    AgentState {
        session_agent_id: Uuid,
        agent_id: Uuid,
        state: ChatSessionAgentState,
    },
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

#[derive(Clone)]
pub struct ChatRunner {
    db: DBService,
    streams: Arc<DashMap<Uuid, broadcast::Sender<ChatStreamEvent>>>,
}

impl ChatRunner {
    pub fn new(db: DBService) -> Self {
        Self {
            db,
            streams: Arc::new(DashMap::new()),
        }
    }

    pub fn subscribe(&self, session_id: Uuid) -> broadcast::Receiver<ChatStreamEvent> {
        self.sender_for(session_id).subscribe()
    }

    pub fn emit_message_new(&self, session_id: Uuid, message: ChatMessage) {
        self.emit(session_id, ChatStreamEvent::MessageNew { message });
    }

    pub async fn handle_message(&self, session: &ChatSession, message: &ChatMessage) {
        self.emit_message_new(session.id, message.clone());

        if message.sender_type != ChatSenderType::User {
            return;
        }

        let session_id = session.id;
        let mentions = message.mentions.0.clone();
        for mention in mentions {
            let runner = self.clone();
            let message_clone = message.clone();
            tokio::spawn(async move {
                if let Err(err) =
                    runner.run_agent_for_mention(session_id, &mention, &message_clone).await
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

    async fn run_agent_for_mention(
        &self,
        session_id: Uuid,
        mention: &str,
        source_message: &ChatMessage,
    ) -> Result<(), ChatRunnerError> {
        let Some(agent) = ChatAgent::find_by_name(&self.db.pool, mention).await? else {
            return Err(ChatRunnerError::AgentNotFound(mention.to_string()));
        };

        let Some(session_agent) = self.get_session_agent(session_id, &agent).await? else {
            tracing::debug!(
                session_id = %session_id,
                agent_id = %agent.id,
                "chat session agent not configured; skipping mention"
            );
            return Ok(());
        };

        if session_agent.state == ChatSessionAgentState::Running {
            tracing::debug!(
                session_agent_id = %session_agent.id,
                agent_id = %agent.id,
                "chat session agent already running; skipping new run"
            );
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
            },
        );

        let session_agent_id = session_agent.id;
        let agent_id = agent.id;

        let reply_handle = self.resolve_reply_handle(source_message);

        let result = async {
            let workspace_path = session_agent
                .workspace_path
                .clone()
                .unwrap_or_else(|| self.build_workspace_path(session_id, agent_id));
            fs::create_dir_all(&workspace_path).await?;
            fs::create_dir_all(PathBuf::from(&workspace_path).join("runs")).await?;

            let run_index = ChatRun::next_run_index(&self.db.pool, session_agent_id).await?;
            let run_id = Uuid::new_v4();
            let run_dir = PathBuf::from(&workspace_path)
                .join("runs")
                .join(format!("run_{:04}", run_index));
            fs::create_dir_all(&run_dir).await?;

            let input_path = run_dir.join("input.md");
            let output_path = run_dir.join("output.md");
            let raw_log_path = run_dir.join("raw.log");
            let meta_path = run_dir.join("meta.json");

            let context_snapshot = self
                .build_context_snapshot(session_id, &workspace_path, &run_dir)
                .await?;
            let prompt = self.build_prompt(
                &agent,
                source_message,
                &context_snapshot.jsonl,
                &context_snapshot.workspace_path,
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

            let executor_profile_id = ExecutorProfileId::new(self.parse_runner_type(&agent)?);
            let mut executor = ExecutorConfigs::get_cached()
                .get_coding_agent_or_default(&executor_profile_id);
            executor.use_approvals(Arc::new(NoopExecutorApprovalService::default()));

            let repo_context = RepoContext::new(PathBuf::from(&workspace_path), Vec::new());
            let mut env = ExecutionEnv::new(repo_context, false, String::new());
            env.insert("VK_CHAT_SESSION_ID", session_id.to_string());
            env.insert("VK_CHAT_AGENT_ID", agent_id.to_string());
            env.insert("VK_CHAT_SESSION_AGENT_ID", session_agent_id.to_string());
            env.insert("VK_CHAT_RUN_ID", run_id.to_string());
            env.insert(
                "VK_CHAT_CONTEXT_PATH",
                context_snapshot.workspace_path.to_string_lossy().to_string(),
            );
            env.insert(
                "VK_CHAT_CONTEXT_RUN_PATH",
                context_snapshot.run_path.to_string_lossy().to_string(),
            );

            let mut spawned = if let Some(agent_session_id) =
                session_agent.agent_session_id.as_deref()
            {
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
            );

            self.spawn_exit_watcher(spawned.child, msg_store, failed_flag);

            Ok::<(), ChatRunnerError>(())
        }
        .await;

        if result.is_err() {
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

    async fn get_session_agent(
        &self,
        session_id: Uuid,
        agent: &ChatAgent,
    ) -> Result<Option<ChatSessionAgent>, ChatRunnerError> {
        if let Some(existing) =
            ChatSessionAgent::find_by_session_and_agent(&self.db.pool, session_id, agent.id)
                .await?
        {
            if existing.workspace_path.is_none() {
                let workspace_path = self.build_workspace_path(session_id, agent.id);
                return Ok(Some(
                    ChatSessionAgent::update_workspace_path(
                        &self.db.pool,
                        existing.id,
                        Some(workspace_path),
                    )
                    .await?,
                ));
            }
            return Ok(Some(existing));
        }

        Ok(None)
    }

    fn parse_runner_type(&self, agent: &ChatAgent) -> Result<BaseCodingAgent, ChatRunnerError> {
        let raw = agent.runner_type.trim();
        let normalized = raw
            .replace('-', "_")
            .replace(' ', "_")
            .to_ascii_uppercase();
        BaseCodingAgent::from_str(&normalized)
            .map_err(|_| ChatRunnerError::UnknownRunnerType(raw.to_string()))
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
        let Some(handle) = handle else {
            return content.to_string();
        };
        let prefix = format!("@{handle} ");
        let trimmed = content.trim_start();
        if trimmed.starts_with(&prefix) {
            content.to_string()
        } else {
            format!("{prefix}{trimmed}")
        }
    }

    async fn capture_git_diff(
        workspace_path: &PathBuf,
        run_dir: &PathBuf,
    ) -> Option<DiffInfo> {
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

        let truncated = diff.len() > DIFF_PREVIEW_LIMIT;
        let preview = diff.chars().take(DIFF_PREVIEW_LIMIT).collect::<String>();

        Some(DiffInfo { preview, truncated })
    }

    async fn capture_untracked_files(
        workspace_path: &PathBuf,
        run_dir: &PathBuf,
    ) -> Vec<String> {
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

            if let Some(parent) = dest.parent() {
                if let Err(err) = fs::create_dir_all(parent).await {
                    tracing::warn!("Failed to create untracked dir: {}", err);
                    continue;
                }
            }

            match fs::metadata(&src).await {
                Ok(metadata) => {
                    if metadata.len() > UNTRACKED_FILE_LIMIT {
                        let placeholder = format!(
                            "File too large to display ({} bytes).",
                            metadata.len()
                        );
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
        run_dir: &PathBuf,
    ) -> Result<ContextSnapshot, ChatRunnerError> {
        let messages =
            crate::services::chat::build_structured_messages(&self.db.pool, session_id).await?;
        let mut jsonl = String::new();
        for message in messages {
            let line = serde_json::to_string(&message).unwrap_or_default();
            jsonl.push_str(&line);
            jsonl.push('\n');
        }

        let context_dir = PathBuf::from(workspace_path).join("context");
        fs::create_dir_all(&context_dir).await?;
        let context_path = context_dir.join("messages.jsonl");
        fs::write(&context_path, jsonl.as_bytes()).await?;

        let run_context_path = run_dir.join("messages.jsonl");
        fs::write(&run_context_path, jsonl.as_bytes()).await?;

        Ok(ContextSnapshot {
            jsonl,
            workspace_path: context_path,
            run_path: run_context_path,
        })
    }

    fn build_prompt(
        &self,
        agent: &ChatAgent,
        message: &ChatMessage,
        context_jsonl: &str,
        context_path: &PathBuf,
    ) -> String {
        let mut prompt = String::new();
        prompt.push_str("[ENVELOPE]\n");
        prompt.push_str(&format!("session_id={}\n", message.session_id));
        let sender_handle = self.resolve_reply_handle(message);
        prompt.push_str(&format!("from=user:{}\n", sender_handle));
        prompt.push_str(&format!("to=agent:{}\n", agent.name));
        prompt.push_str(&format!("message_id={}\n", message.id));
        prompt.push_str(&format!("timestamp={}\n", message.created_at));
        prompt.push_str("[/ENVELOPE]\n\n");

        if !agent.system_prompt.trim().is_empty() {
            prompt.push_str("[AGENT_ROLE]\n");
            prompt.push_str(agent.system_prompt.trim());
            prompt.push_str("\n[/AGENT_ROLE]\n\n");
        }

        prompt.push_str("[GROUP_CONTEXT]\n");
        prompt.push_str("All group messages in JSONL format (oldest to newest).\n");
        prompt.push_str(&format!(
            "context_path={}\n",
            context_path.to_string_lossy()
        ));
        prompt.push_str(context_jsonl);
        prompt.push_str("\n[/GROUP_CONTEXT]\n\n");

        prompt.push_str("[USER_MESSAGE]\n");
        prompt.push_str(message.content.trim());
        prompt.push_str("\n[/USER_MESSAGE]\n");

        prompt
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
    ) {
        let db = self.db.clone();
        let sender = self.sender_for(session_id);

        tokio::spawn(async move {
            let mut stream = msg_store.history_plus_stream();
            let mut last_content: HashMap<usize, String> = HashMap::new();
            let mut latest_assistant = String::new();
            let mut agent_session_id: Option<String> = None;
            let mut agent_message_id: Option<String> = None;

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
                    Ok(LogMsg::JsonPatch(patch)) => {
                        if let Some((index, entry)) = extract_normalized_entry_from_patch(&patch) {
                            if matches!(entry.entry_type, NormalizedEntryType::AssistantMessage) {
                                let current = entry.content;
                                let previous = last_content.get(&index).cloned().unwrap_or_default();
                                let (delta, is_delta) = if current.starts_with(&previous) {
                                    (current[previous.len()..].to_string(), true)
                                } else {
                                    (current.clone(), false)
                                };

                                last_content.insert(index, current.clone());
                                latest_assistant = current.clone();

                                if !delta.is_empty() {
                                    let _ = sender.send(ChatStreamEvent::AgentDelta {
                                        session_id,
                                        session_agent_id,
                                        agent_id,
                                        run_id,
                                        content: delta,
                                        delta: is_delta,
                                        is_final: false,
                                    });
                                }
                            }
                        }
                    }
                    Ok(LogMsg::Finished) => {
                        let _ = fs::write(&output_path, &latest_assistant).await;

                        let diff_info =
                            ChatRunner::capture_git_diff(&workspace_path, &run_dir).await;
                        let untracked_files =
                            ChatRunner::capture_untracked_files(&workspace_path, &run_dir).await;

                        let mut meta = serde_json::json!({
                            "run_id": run_id,
                            "session_id": session_id,
                            "session_agent_id": session_agent_id,
                            "agent_id": agent_id,
                            "agent_session_id": agent_session_id,
                            "agent_message_id": agent_message_id,
                            "finished_at": Utc::now().to_rfc3339(),
                        });

                        if let Some(diff) = diff_info.as_ref() {
                            meta["diff_available"] = true.into();
                            meta["diff_preview"] = diff.preview.clone().into();
                            meta["diff_truncated"] = diff.truncated.into();
                        }

                        if !untracked_files.is_empty() {
                            meta["untracked_files"] =
                                serde_json::to_value(&untracked_files).unwrap_or_default();
                        }

                        let _ = fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap())
                            .await;

                        let final_content =
                            ChatRunner::apply_reply_prefix(&latest_assistant, reply_handle.as_deref());

                        if !final_content.trim().is_empty() {
                            if let Ok(message) = crate::services::chat::create_message(
                                &db.pool,
                                session_id,
                                ChatSenderType::Agent,
                                Some(agent_id),
                                final_content.clone(),
                                Some(meta.clone()),
                            )
                            .await
                            {
                                let _ = sender.send(ChatStreamEvent::MessageNew { message });
                            }
                        }

                        let _ = sender.send(ChatStreamEvent::AgentDelta {
                            session_id,
                            session_agent_id,
                            agent_id,
                            run_id,
                            content: latest_assistant.clone(),
                            delta: false,
                            is_final: true,
                        });

                        let failed = failed_flag.load(Ordering::Relaxed);
                        let final_state = if failed {
                            ChatSessionAgentState::Dead
                        } else {
                            ChatSessionAgentState::Idle
                        };

                        let _ =
                            ChatSessionAgent::update_state(
                                &db.pool,
                                session_agent_id,
                                final_state.clone(),
                            )
                            .await;

                        let _ = sender.send(ChatStreamEvent::AgentState {
                            session_agent_id,
                            agent_id,
                            state: final_state,
                        });
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
        msg_store: Arc<MsgStore>,
        failed_flag: Arc<AtomicBool>,
    ) {
        tokio::spawn(async move {
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            failed_flag.store(true, Ordering::Relaxed);
                        }
                        msg_store.push_finished();
                        break;
                    }
                    Ok(None) => {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    }
                    Err(err) => {
                        msg_store.push(LogMsg::Stderr(format!("process wait error: {err}")));
                        failed_flag.store(true, Ordering::Relaxed);
                        msg_store.push_finished();
                        break;
                    }
                }
            }
        });
    }
}
