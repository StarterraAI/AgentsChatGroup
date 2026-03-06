use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    io,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use codex_app_server_protocol::{
    ApplyPatchApprovalResponse, ClientInfo, ClientNotification, ClientRequest,
    CommandExecutionApprovalDecision, CommandExecutionRequestApprovalResponse,
    ExecCommandApprovalResponse, FileChangeApprovalDecision, FileChangeRequestApprovalResponse,
    GetAuthStatusParams, GetAuthStatusResponse, InitializeParams, InitializeResponse, JSONRPCError,
    JSONRPCNotification, JSONRPCRequest, JSONRPCResponse, ListMcpServerStatusParams,
    ListMcpServerStatusResponse, RequestId, ReviewStartParams, ReviewStartResponse, ReviewTarget,
    ServerNotification, ServerRequest, ThreadItem, ThreadResumeParams, ThreadResumeResponse,
    ThreadStartParams, ThreadStartResponse, TurnCompletedNotification, TurnStartParams,
    TurnStartResponse, TurnStatus, UserInput,
};
use codex_protocol::protocol::ReviewDecision;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{self, Map, Value};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt, BufWriter},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
use workspace_utils::approvals::ApprovalStatus;

use super::jsonrpc::{JsonRpcCallbacks, JsonRpcPeer};
use crate::{
    approvals::{ExecutorApprovalError, ExecutorApprovalService},
    env::RepoContext,
    executors::{ExecutorError, codex::normalize_logs::Approval},
};

pub struct AppServerClient {
    rpc: OnceLock<JsonRpcPeer>,
    log_writer: LogWriter,
    approvals: Option<Arc<dyn ExecutorApprovalService>>,
    thread_id: Mutex<Option<String>>,
    items_by_id: Mutex<HashMap<String, ThreadItem>>,
    pending_feedback: Mutex<VecDeque<String>>,
    auto_approve: bool,
    repo_context: RepoContext,
    commit_reminder: bool,
    commit_reminder_prompt: String,
    commit_reminder_sent: AtomicBool,
    cancel: CancellationToken,
}

impl AppServerClient {
    pub fn new(
        log_writer: LogWriter,
        approvals: Option<Arc<dyn ExecutorApprovalService>>,
        auto_approve: bool,
        repo_context: RepoContext,
        commit_reminder: bool,
        commit_reminder_prompt: String,
        cancel: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            rpc: OnceLock::new(),
            log_writer,
            approvals,
            auto_approve,
            thread_id: Mutex::new(None),
            items_by_id: Mutex::new(HashMap::new()),
            pending_feedback: Mutex::new(VecDeque::new()),
            repo_context,
            commit_reminder,
            commit_reminder_prompt,
            commit_reminder_sent: AtomicBool::new(false),
            cancel,
        })
    }

    pub fn connect(&self, peer: JsonRpcPeer) {
        let _ = self.rpc.set(peer);
    }

    fn rpc(&self) -> &JsonRpcPeer {
        self.rpc.get().expect("Codex RPC peer not attached")
    }

    pub fn log_writer(&self) -> &LogWriter {
        &self.log_writer
    }

    pub async fn initialize(&self) -> Result<(), ExecutorError> {
        let request = ClientRequest::Initialize {
            request_id: self.next_request_id(),
            params: InitializeParams {
                client_info: ClientInfo {
                    name: "vibe-codex-executor".to_string(),
                    title: None,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
            },
        };

        self.send_request::<InitializeResponse>(request, "initialize")
            .await?;
        self.send_message(&ClientNotification::Initialized).await
    }

    pub async fn start_thread(
        &self,
        params: ThreadStartParams,
    ) -> Result<ThreadStartResponse, ExecutorError> {
        let request = ClientRequest::ThreadStart {
            request_id: self.next_request_id(),
            params,
        };
        self.send_request(request, "thread/start").await
    }

    pub async fn resume_thread(
        &self,
        params: ThreadResumeParams,
    ) -> Result<ThreadResumeResponse, ExecutorError> {
        let request = ClientRequest::ThreadResume {
            request_id: self.next_request_id(),
            params,
        };
        self.send_request(request, "thread/resume").await
    }

    pub async fn start_turn(
        &self,
        thread_id: String,
        message: String,
    ) -> Result<TurnStartResponse, ExecutorError> {
        let request = ClientRequest::TurnStart {
            request_id: self.next_request_id(),
            params: TurnStartParams {
                thread_id,
                input: vec![UserInput::Text {
                    text: message,
                    text_elements: Vec::new(),
                }],
                cwd: None,
                approval_policy: None,
                sandbox_policy: None,
                model: None,
                effort: None,
                summary: None,
                output_schema: None,
            },
        };
        self.send_request(request, "turn/start").await
    }

    pub async fn get_auth_status(&self) -> Result<GetAuthStatusResponse, ExecutorError> {
        let request = ClientRequest::GetAuthStatus {
            request_id: self.next_request_id(),
            params: GetAuthStatusParams {
                include_token: Some(true),
                refresh_token: Some(false),
            },
        };
        self.send_request(request, "getAuthStatus").await
    }

    pub async fn start_review(
        &self,
        thread_id: String,
        target: ReviewTarget,
    ) -> Result<ReviewStartResponse, ExecutorError> {
        let request = ClientRequest::ReviewStart {
            request_id: self.next_request_id(),
            params: ReviewStartParams {
                thread_id,
                target,
                delivery: None,
            },
        };
        self.send_request(request, "review/start").await
    }

    pub async fn list_mcp_server_status(
        &self,
        cursor: Option<String>,
    ) -> Result<ListMcpServerStatusResponse, ExecutorError> {
        let request = ClientRequest::McpServerStatusList {
            request_id: self.next_request_id(),
            params: ListMcpServerStatusParams {
                cursor,
                limit: None,
            },
        };
        self.send_request(request, "mcpServerStatus/list").await
    }

    async fn handle_server_request(
        &self,
        peer: &JsonRpcPeer,
        request: ServerRequest,
    ) -> Result<(), ExecutorError> {
        match request {
            ServerRequest::ApplyPatchApproval { request_id, params } => {
                let input = serde_json::to_value(&params)
                    .map_err(|err| ExecutorError::Io(io::Error::other(err.to_string())))?;
                let status = self
                    .request_tool_approval("edit", input, &params.call_id)
                    .await
                    .map_err(|err| {
                        if !matches!(
                            err,
                            ExecutorError::ExecutorApprovalError(ExecutorApprovalError::Cancelled)
                        ) {
                            tracing::error!(
                                "Codex apply_patch approval failed for call_id={}: {err}",
                                params.call_id
                            );
                        }
                        err
                    })?;
                self.log_writer
                    .log_raw(
                        &Approval::approval_response(
                            params.call_id,
                            "codex.apply_patch".to_string(),
                            status.clone(),
                        )
                        .raw(),
                    )
                    .await?;
                let (decision, feedback) = self.legacy_review_decision(&status);
                let response = ApplyPatchApprovalResponse { decision };
                send_server_response(peer, request_id, response).await?;
                if let Some(message) = feedback {
                    tracing::debug!("queueing patch denial feedback: {message}");
                    self.enqueue_feedback(message).await;
                }
                Ok(())
            }
            ServerRequest::ExecCommandApproval { request_id, params } => {
                let input = serde_json::to_value(&params)
                    .map_err(|err| ExecutorError::Io(io::Error::other(err.to_string())))?;
                let status = self
                    .request_tool_approval("bash", input, &params.call_id)
                    .await
                    .map_err(|err| {
                        tracing::error!(
                            "Codex exec_command approval failed for call_id={}: {err}",
                            params.call_id
                        );
                        err
                    })?;
                self.log_writer
                    .log_raw(
                        &Approval::approval_response(
                            params.call_id,
                            "codex.exec_command".to_string(),
                            status.clone(),
                        )
                        .raw(),
                    )
                    .await?;

                let (decision, feedback) = self.legacy_review_decision(&status);
                let response = ExecCommandApprovalResponse { decision };
                send_server_response(peer, request_id, response).await?;
                if let Some(message) = feedback {
                    tracing::debug!("queueing exec denial feedback: {message}");
                    self.enqueue_feedback(message).await;
                }
                Ok(())
            }
            ServerRequest::CommandExecutionRequestApproval { request_id, params } => {
                let input = self
                    .approval_input("commandExecution", &params.item_id, &params)
                    .await?;
                let status = self
                    .request_tool_approval("bash", input, &params.item_id)
                    .await
                    .map_err(|err| {
                        tracing::error!(
                            "Codex command approval failed for item_id={}: {err}",
                            params.item_id
                        );
                        err
                    })?;
                self.log_writer
                    .log_raw(
                        &Approval::approval_response(
                            params.item_id.clone(),
                            "codex.exec_command".to_string(),
                            status.clone(),
                        )
                        .raw(),
                    )
                    .await?;

                let (decision, feedback) = self.command_approval_decision(&status);
                let response = CommandExecutionRequestApprovalResponse { decision };
                send_server_response(peer, request_id, response).await?;
                if let Some(message) = feedback {
                    tracing::debug!("queueing command denial feedback: {message}");
                    self.enqueue_feedback(message).await;
                }
                Ok(())
            }
            ServerRequest::FileChangeRequestApproval { request_id, params } => {
                let input = self
                    .approval_input("fileChange", &params.item_id, &params)
                    .await?;
                let status = self
                    .request_tool_approval("edit", input, &params.item_id)
                    .await
                    .map_err(|err| {
                        tracing::error!(
                            "Codex file change approval failed for item_id={}: {err}",
                            params.item_id
                        );
                        err
                    })?;
                self.log_writer
                    .log_raw(
                        &Approval::approval_response(
                            params.item_id.clone(),
                            "codex.apply_patch".to_string(),
                            status.clone(),
                        )
                        .raw(),
                    )
                    .await?;

                let (decision, feedback) = self.file_change_approval_decision(&status);
                let response = FileChangeRequestApprovalResponse { decision };
                send_server_response(peer, request_id, response).await?;
                if let Some(message) = feedback {
                    tracing::debug!("queueing file change denial feedback: {message}");
                    self.enqueue_feedback(message).await;
                }
                Ok(())
            }
        }
    }

    async fn request_tool_approval(
        &self,
        tool_name: &str,
        tool_input: Value,
        tool_call_id: &str,
    ) -> Result<ApprovalStatus, ExecutorError> {
        if self.auto_approve {
            return Ok(ApprovalStatus::Approved);
        }
        let approval_service = self
            .approvals
            .as_ref()
            .ok_or(ExecutorApprovalError::ServiceUnavailable)?;

        Ok(approval_service
            .request_tool_approval(tool_name, tool_input, tool_call_id, self.cancel.clone())
            .await?)
    }

    pub async fn register_session(&self, thread_id: &str) -> Result<(), ExecutorError> {
        {
            let mut guard = self.thread_id.lock().await;
            guard.replace(thread_id.to_string());
        }
        self.flush_pending_feedback().await;
        Ok(())
    }

    async fn send_message<M>(&self, message: &M) -> Result<(), ExecutorError>
    where
        M: Serialize + Sync,
    {
        self.rpc().send(message).await
    }

    async fn send_request<R>(&self, request: ClientRequest, label: &str) -> Result<R, ExecutorError>
    where
        R: DeserializeOwned + std::fmt::Debug,
    {
        let request_id = request_id(&request);
        self.rpc()
            .request(request_id, &request, label, self.cancel.clone())
            .await
    }

    fn next_request_id(&self) -> RequestId {
        self.rpc().next_request_id()
    }

    async fn enqueue_feedback(&self, message: String) {
        if message.trim().is_empty() {
            return;
        }
        let mut guard = self.pending_feedback.lock().await;
        guard.push_back(message);
    }

    async fn flush_pending_feedback(&self) -> bool {
        let messages: Vec<String> = {
            let mut guard = self.pending_feedback.lock().await;
            guard.drain(..).collect()
        };

        if messages.is_empty() {
            return false;
        }

        let Some(thread_id) = self.thread_id.lock().await.clone() else {
            tracing::warn!(
                "pending Codex feedback but thread id unavailable; dropping {} messages",
                messages.len()
            );
            return false;
        };

        let mut sent = false;
        for message in messages {
            let trimmed = message.trim();
            if trimmed.is_empty() {
                continue;
            }
            self.spawn_user_message(thread_id.clone(), format!("User feedback: {trimmed}"));
            sent = true;
        }
        sent
    }

    fn spawn_user_message(&self, thread_id: String, message: String) {
        let peer = self.rpc().clone();
        let cancel = self.cancel.clone();
        let request = ClientRequest::TurnStart {
            request_id: peer.next_request_id(),
            params: TurnStartParams {
                thread_id,
                input: vec![UserInput::Text {
                    text: message,
                    text_elements: Vec::new(),
                }],
                cwd: None,
                approval_policy: None,
                sandbox_policy: None,
                model: None,
                effort: None,
                summary: None,
                output_schema: None,
            },
        };
        tokio::spawn(async move {
            if let Err(err) = peer
                .request::<TurnStartResponse, _>(
                    request_id(&request),
                    &request,
                    "turn/start",
                    cancel,
                )
                .await
            {
                tracing::error!("failed to send follow-up turn: {err}");
            }
        });
    }

    async fn approval_input<T: Serialize>(
        &self,
        request_type: &str,
        item_id: &str,
        params: &T,
    ) -> Result<Value, ExecutorError> {
        let mut input = Map::new();
        input.insert(
            "requestType".to_string(),
            Value::String(request_type.to_string()),
        );
        input.insert(
            "params".to_string(),
            serde_json::to_value(params)
                .map_err(|err| ExecutorError::Io(io::Error::other(err.to_string())))?,
        );
        if let Some(item) = self.items_by_id.lock().await.get(item_id).cloned() {
            input.insert(
                "item".to_string(),
                serde_json::to_value(item)
                    .map_err(|err| ExecutorError::Io(io::Error::other(err.to_string())))?,
            );
        }
        Ok(Value::Object(input))
    }

    fn command_approval_decision(
        &self,
        status: &ApprovalStatus,
    ) -> (CommandExecutionApprovalDecision, Option<String>) {
        if self.auto_approve {
            return (CommandExecutionApprovalDecision::AcceptForSession, None);
        }

        match status {
            ApprovalStatus::Approved => (CommandExecutionApprovalDecision::Accept, None),
            ApprovalStatus::Denied { reason } => {
                let feedback = reason
                    .as_ref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                if feedback.is_some() {
                    (CommandExecutionApprovalDecision::Cancel, feedback)
                } else {
                    (CommandExecutionApprovalDecision::Decline, None)
                }
            }
            ApprovalStatus::TimedOut | ApprovalStatus::Pending => {
                (CommandExecutionApprovalDecision::Decline, None)
            }
        }
    }

    fn file_change_approval_decision(
        &self,
        status: &ApprovalStatus,
    ) -> (FileChangeApprovalDecision, Option<String>) {
        if self.auto_approve {
            return (FileChangeApprovalDecision::AcceptForSession, None);
        }

        match status {
            ApprovalStatus::Approved => (FileChangeApprovalDecision::Accept, None),
            ApprovalStatus::Denied { reason } => {
                let feedback = reason
                    .as_ref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                if feedback.is_some() {
                    (FileChangeApprovalDecision::Cancel, feedback)
                } else {
                    (FileChangeApprovalDecision::Decline, None)
                }
            }
            ApprovalStatus::TimedOut | ApprovalStatus::Pending => {
                (FileChangeApprovalDecision::Decline, None)
            }
        }
    }

    fn legacy_review_decision(&self, status: &ApprovalStatus) -> (ReviewDecision, Option<String>) {
        if self.auto_approve {
            return (ReviewDecision::ApprovedForSession, None);
        }

        match status {
            ApprovalStatus::Approved => (ReviewDecision::Approved, None),
            ApprovalStatus::Denied { reason } => {
                let feedback = reason
                    .as_ref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                if feedback.is_some() {
                    (ReviewDecision::Abort, feedback)
                } else {
                    (ReviewDecision::Denied, None)
                }
            }
            ApprovalStatus::TimedOut | ApprovalStatus::Pending => (ReviewDecision::Denied, None),
        }
    }

    async fn cache_notification_item(&self, notification: &ServerNotification) {
        let item = match notification {
            ServerNotification::ItemStarted(payload) => Some(payload.item.clone()),
            ServerNotification::ItemCompleted(payload) => Some(payload.item.clone()),
            _ => None,
        };

        let Some(item) = item else {
            return;
        };

        let Some(item_id) = thread_item_id(&item) else {
            return;
        };

        self.items_by_id
            .lock()
            .await
            .insert(item_id.to_string(), item);
    }
}

#[async_trait]
impl JsonRpcCallbacks for AppServerClient {
    async fn on_request(
        &self,
        peer: &JsonRpcPeer,
        raw: &str,
        request: JSONRPCRequest,
    ) -> Result<(), ExecutorError> {
        self.log_writer.log_raw(raw).await?;
        match ServerRequest::try_from(request.clone()) {
            Ok(server_request) => self.handle_server_request(peer, server_request).await,
            Err(err) => {
                tracing::debug!("Unhandled server request `{}`: {err}", request.method);
                let response = JSONRPCResponse {
                    id: request.id,
                    result: Value::Null,
                };
                peer.send(&response).await
            }
        }
    }

    async fn on_response(
        &self,
        _peer: &JsonRpcPeer,
        raw: &str,
        _response: &JSONRPCResponse,
    ) -> Result<(), ExecutorError> {
        self.log_writer.log_raw(raw).await
    }

    async fn on_error(
        &self,
        _peer: &JsonRpcPeer,
        raw: &str,
        _error: &JSONRPCError,
    ) -> Result<(), ExecutorError> {
        self.log_writer.log_raw(raw).await
    }

    async fn on_notification(
        &self,
        _peer: &JsonRpcPeer,
        raw: &str,
        notification: JSONRPCNotification,
    ) -> Result<bool, ExecutorError> {
        let parsed_notification = serde_json::from_str::<ServerNotification>(raw).ok();
        if let Some(server_notification) = parsed_notification.as_ref() {
            self.cache_notification_item(server_notification).await;
        }

        let raw = if let Some(mut server_notification) = parsed_notification.clone() {
            if let ServerNotification::SessionConfigured(session_configured) =
                &mut server_notification
            {
                // history can be large, which might get truncated during transmission, corrupting the JSON line and losing valuable session and model information.
                session_configured.initial_messages = None;
                Cow::Owned(serde_json::to_string(&server_notification)?)
            } else {
                Cow::Borrowed(raw)
            }
        } else {
            Cow::Borrowed(raw)
        };
        self.log_writer.log_raw(&raw).await?;

        if let Some(server_notification) = parsed_notification {
            if let ServerNotification::TurnCompleted(TurnCompletedNotification {
                thread_id,
                turn,
            }) = server_notification
            {
                let has_finished = matches!(
                    turn.status,
                    TurnStatus::Completed | TurnStatus::Interrupted | TurnStatus::Failed
                );

                if has_finished
                    && matches!(turn.status, TurnStatus::Completed)
                    && self.commit_reminder
                    && !self.commit_reminder_sent.swap(true, Ordering::SeqCst)
                    && let status = self.repo_context.check_uncommitted_changes().await
                    && !status.is_empty()
                {
                    let prompt = format!("{}\n{}", self.commit_reminder_prompt, status);
                    self.spawn_user_message(thread_id, prompt);
                    return Ok(false);
                }

                if self.flush_pending_feedback().await {
                    return Ok(false);
                }

                return Ok(has_finished);
            }

            return Ok(false);
        }

        let method = notification.method.as_str();
        if !method.starts_with("codex/event") {
            return Ok(false);
        }

        if method.ends_with("turn_aborted") {
            tracing::debug!("codex turn aborted; flushing feedback queue");
            self.flush_pending_feedback().await;
            return Ok(false);
        }

        let has_finished = method
            .strip_prefix("codex/event/")
            .is_some_and(|suffix| suffix == "task_complete");

        if has_finished
            && self.commit_reminder
            && !self.commit_reminder_sent.swap(true, Ordering::SeqCst)
            && let status = self.repo_context.check_uncommitted_changes().await
            && !status.is_empty()
            && let Some(thread_id) = self.thread_id.lock().await.clone()
        {
            let prompt = format!("{}\n{}", self.commit_reminder_prompt, status);
            self.spawn_user_message(thread_id, prompt);
            return Ok(false);
        }

        Ok(has_finished)
    }

    async fn on_non_json(&self, raw: &str) -> Result<(), ExecutorError> {
        self.log_writer.log_raw(raw).await?;
        Ok(())
    }
}

async fn send_server_response<T>(
    peer: &JsonRpcPeer,
    request_id: RequestId,
    response: T,
) -> Result<(), ExecutorError>
where
    T: Serialize,
{
    let payload = JSONRPCResponse {
        id: request_id,
        result: serde_json::to_value(response)
            .map_err(|err| ExecutorError::Io(io::Error::other(err.to_string())))?,
    };

    peer.send(&payload).await
}

fn request_id(request: &ClientRequest) -> RequestId {
    match request {
        ClientRequest::Initialize { request_id, .. }
        | ClientRequest::ThreadStart { request_id, .. }
        | ClientRequest::GetAuthStatus { request_id, .. }
        | ClientRequest::ThreadResume { request_id, .. }
        | ClientRequest::TurnStart { request_id, .. }
        | ClientRequest::ReviewStart { request_id, .. }
        | ClientRequest::McpServerStatusList { request_id, .. } => request_id.clone(),
        _ => unreachable!("request_id called for unsupported request variant"),
    }
}

fn thread_item_id(item: &ThreadItem) -> Option<&str> {
    match item {
        ThreadItem::UserMessage { id, .. }
        | ThreadItem::AgentMessage { id, .. }
        | ThreadItem::Reasoning { id, .. }
        | ThreadItem::CommandExecution { id, .. }
        | ThreadItem::FileChange { id, .. }
        | ThreadItem::McpToolCall { id, .. }
        | ThreadItem::CollabAgentToolCall { id, .. }
        | ThreadItem::WebSearch { id, .. }
        | ThreadItem::ImageView { id, .. }
        | ThreadItem::EnteredReviewMode { id, .. }
        | ThreadItem::ExitedReviewMode { id, .. } => Some(id.as_str()),
    }
}

#[derive(Clone)]
pub struct LogWriter {
    writer: Arc<Mutex<BufWriter<Box<dyn AsyncWrite + Send + Unpin>>>>,
}

impl LogWriter {
    pub fn new(writer: impl AsyncWrite + Send + Unpin + 'static) -> Self {
        Self {
            writer: Arc::new(Mutex::new(BufWriter::new(Box::new(writer)))),
        }
    }

    pub async fn log_raw(&self, raw: &str) -> Result<(), ExecutorError> {
        let mut guard = self.writer.lock().await;
        guard
            .write_all(raw.as_bytes())
            .await
            .map_err(ExecutorError::Io)?;
        guard.write_all(b"\n").await.map_err(ExecutorError::Io)?;
        guard.flush().await.map_err(ExecutorError::Io)?;
        Ok(())
    }
}
