pub mod client;
pub mod jsonrpc;
pub mod normalize_logs;
pub mod review;
pub mod session;
pub mod slash_commands;
use std::{
    collections::{BTreeSet, HashMap},
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Returns the Codex home directory.
///
/// Checks the `CODEX_HOME` environment variable first, then falls back to `~/.codex`.
/// This allows users to configure a custom location for Codex configuration and state.
pub fn codex_home() -> Option<PathBuf> {
    if let Ok(codex_home) = env::var("CODEX_HOME")
        && !codex_home.trim().is_empty()
    {
        return Some(PathBuf::from(codex_home));
    }
    dirs::home_dir().map(|home| home.join(".codex"))
}

fn codex_model_cache_paths() -> Vec<PathBuf> {
    codex_home()
        .map(|home| vec![home.join("models_cache.json")])
        .unwrap_or_default()
}

const CODEX_MODEL_FALLBACKS: &[&str] = &[
    "gpt-6-astra",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex-spark",
    "codex-auto-review",
];

use async_trait::async_trait;
use codex_app_server_protocol::{
    AskForApproval as AppServerAskForApproval, ReviewTarget, SandboxMode as AppServerSandboxMode,
    ThreadResumeParams, ThreadStartParams,
};
use command_group::AsyncCommandGroup;
use derivative::Derivative;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use strum_macros::AsRefStr;
use tokio::process::Command;
use ts_rs::TS;
use workspace_utils::msg_store::MsgStore;

use self::{
    client::{AppServerClient, LogWriter},
    jsonrpc::{ExitSignalSender, JsonRpcPeer},
    normalize_logs::{Error, normalize_logs},
    session::SessionHandler,
};
use crate::{
    approvals::ExecutorApprovalService,
    command::{CmdOverrides, CommandBuildError, CommandBuilder, CommandParts, apply_overrides},
    env::ExecutionEnv,
    executors::{
        AppendPrompt, AvailabilityInfo, ExecutorError, ExecutorExitResult, SlashCommandDescription,
        SpawnedChild, StandardCodingAgentExecutor,
        opencode::{FrozenProcessCommand, ProcessOutputRedactor},
        utils::{json_has_nonempty_string, read_json_file},
    },
    logs::utils::patch,
    mcp_config::MemberMcpConfig,
    mcp_run::{McpRunContext, PreparedMcpRun},
    model_discovery::{
        ProviderKind, cli_model_commands, discover_from_sources, model_slugs_from_models_json,
        read_config_value, runner_config_paths,
    },
    skill_config::NativeSkillConfigBackend,
    stdout_dup::create_stdout_pipe_writer,
};

/// Sandbox policy modes for Codex
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum SandboxMode {
    Auto,
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

/// Determines when the user is consulted to approve Codex actions.
///
/// - `UnlessTrusted`: Read-only commands are auto-approved. Everything else will
///   ask the user to approve.
/// - `OnFailure`: All commands run in a restricted sandbox initially. If a
///   command fails, the user is asked to approve execution without the sandbox.
/// - `OnRequest`: The model decides when to ask the user for approval.
/// - `Never`: Commands never ask for approval. Commands that fail in the
///   restricted sandbox are not retried.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum AskForApproval {
    UnlessTrusted,
    OnFailure,
    OnRequest,
    Never,
}

/// Reasoning effort for the underlying model
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
}

/// Model reasoning summary style
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ReasoningSummary {
    Auto,
    Concise,
    Detailed,
    None,
}

/// Format for model reasoning summaries
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ReasoningSummaryFormat {
    None,
    Experimental,
}

enum CodexSessionAction {
    Chat { prompt: String },
    Review { target: ReviewTarget },
}

#[derive(Clone)]
struct CodexMcpRuntimeSnapshot {
    mcp_servers: Value,
    output_redactor: ProcessOutputRedactor,
    process_command: FrozenProcessCommand,
}

impl std::fmt::Debug for CodexMcpRuntimeSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodexMcpRuntimeSnapshot")
            .field(
                "mcp_server_count",
                &self.mcp_servers.as_object().map_or(0, serde_json::Map::len),
            )
            .finish()
    }
}

#[derive(Derivative, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[derivative(Debug, PartialEq)]
pub struct Codex {
    #[serde(default)]
    pub append_prompt: AppendPrompt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_for_approval: Option<AskForApproval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oss: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_reasoning_effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_reasoning_summary: Option<ReasoningSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_reasoning_summary_format: Option<ReasoningSummaryFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_apply_patch_tool: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer_instructions: Option<String>,
    #[serde(flatten)]
    pub cmd: CmdOverrides,

    #[serde(skip)]
    #[ts(skip)]
    #[derivative(Debug = "ignore", PartialEq = "ignore")]
    approvals: Option<Arc<dyn ExecutorApprovalService>>,

    #[serde(skip)]
    #[ts(skip)]
    #[derivative(Debug = "ignore", PartialEq = "ignore")]
    runtime_mcp_snapshot: Option<Arc<CodexMcpRuntimeSnapshot>>,

    #[cfg(test)]
    #[serde(skip)]
    #[ts(skip)]
    #[schemars(skip)]
    #[derivative(Debug = "ignore", PartialEq = "ignore")]
    test_base_command: Option<String>,
}

#[async_trait]
impl StandardCodingAgentExecutor for Codex {
    async fn prepare_mcp_for_run(
        &mut self,
        canonical: &MemberMcpConfig,
        _context: &McpRunContext,
        _env: &mut ExecutionEnv,
    ) -> Result<PreparedMcpRun, ExecutorError> {
        self.runtime_mcp_snapshot = None;
        if self.cmd.base_command_override.is_some() {
            return Err(ExecutorError::Configuration(
                "Codex run-scoped MCP isolation cannot be verified for a custom base command"
                    .to_string(),
            ));
        }
        let mcp_servers = build_codex_mcp_servers(canonical)?;
        let output_redactor = ProcessOutputRedactor::from_config(&serde_json::json!({
            "mcp_servers": mcp_servers
        }));
        let process_command =
            FrozenProcessCommand::resolve(self.build_command_builder()?.build_initial()?).await?;
        self.runtime_mcp_snapshot = Some(Arc::new(CodexMcpRuntimeSnapshot {
            mcp_servers,
            output_redactor,
            process_command,
        }));
        PreparedMcpRun::new(canonical)
    }

    fn is_authenticated(&self, env: &ExecutionEnv) -> bool {
        let env = env.clone().with_profile(&self.cmd);
        let Some(home) = codex_home() else {
            return self.authentication_detected(&env, &["OPENAI_API_KEY"], false);
        };
        let auth = read_json_file(&home.join("auth.json"));
        let oauth_login = auth.as_ref().is_some_and(|value| {
            json_has_nonempty_string(
                value,
                &[
                    "/tokens/access_token",
                    "/tokens/refresh_token",
                    "/access_token",
                    "/refresh_token",
                ],
            )
        });
        let auth_file_key = auth
            .as_ref()
            .is_some_and(|value| json_has_nonempty_string(value, &["/OPENAI_API_KEY", "/api_key"]));
        let provider_configured = self
            .model_provider
            .as_deref()
            .is_some_and(|provider| !provider.trim().is_empty())
            || codex_config_has_provider(&home.join("config.toml"), &env);
        self.authentication_detected(
            &env,
            &["OPENAI_API_KEY", "AZURE_OPENAI_API_KEY"],
            oauth_login || auth_file_key || provider_configured,
        )
    }

    fn use_approvals(&mut self, approvals: Arc<dyn ExecutorApprovalService>) {
        self.approvals = Some(approvals);
    }

    async fn available_slash_commands(
        &self,
        _workdir: &Path,
    ) -> Result<futures::stream::BoxStream<'static, json_patch::Patch>, ExecutorError> {
        let commands = vec![
            SlashCommandDescription {
                name: "compact".to_string(),
                description: Some(
                    "summarize conversation to prevent hitting the context limit".to_string(),
                ),
            },
            SlashCommandDescription {
                name: "init".to_string(),
                description: Some(
                    "create an AGENTS.md file with instructions for Codex".to_string(),
                ),
            },
            SlashCommandDescription {
                name: "status".to_string(),
                description: Some("show current session configuration and token usage".to_string()),
            },
            SlashCommandDescription {
                name: "mcp".to_string(),
                description: Some("list configured MCP tools".to_string()),
            },
        ];
        Ok(Box::pin(futures::stream::once(async move {
            patch::slash_commands(commands, false, None)
        })))
    }

    async fn list_models(
        &self,
        current_dir: &Path,
        env: &ExecutionEnv,
    ) -> Result<Option<Vec<String>>, ExecutorError> {
        let config_paths = runner_config_paths([
            self.default_mcp_config_path(),
            codex_home().map(|home| home.join("config.json")),
        ]);
        let mut models = BTreeSet::new();
        if let Some(discovered) = discover_from_sources(
            current_dir,
            env,
            &self.cmd,
            self.model.as_deref(),
            config_paths,
            cli_model_commands(Self::BASE_COMMAND, &self.cmd),
            &[ProviderKind::OpenAiCompatible],
        )
        .await?
        {
            models.extend(discovered);
        }

        for cache_path in codex_model_cache_paths() {
            match read_config_value(&cache_path).await {
                Ok(Some(value)) => {
                    let slugs = model_slugs_from_models_json(&value);
                    if !slugs.is_empty() {
                        models.extend(slugs);
                        break;
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::debug!(
                        "Failed to read Codex model cache at {}: {err}",
                        cache_path.display()
                    );
                }
            }
        }

        models.extend(
            CODEX_MODEL_FALLBACKS
                .iter()
                .map(|model| (*model).to_string()),
        );

        if models.is_empty() {
            Ok(None)
        } else {
            Ok(Some(models.into_iter().collect()))
        }
    }

    async fn spawn(
        &self,
        current_dir: &Path,
        prompt: &str,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        self.spawn_slash_command(current_dir, prompt, None, env)
            .await
    }

    async fn spawn_follow_up(
        &self,
        current_dir: &Path,
        prompt: &str,
        session_id: &str,
        _reset_to_message_id: Option<&str>,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        self.spawn_slash_command(current_dir, prompt, Some(session_id), env)
            .await
    }

    fn normalize_logs(&self, msg_store: Arc<MsgStore>, worktree_path: &Path) {
        normalize_logs(msg_store, worktree_path);
    }

    fn default_mcp_config_path(&self) -> Option<PathBuf> {
        codex_home().map(|home| home.join("config.toml"))
    }

    fn default_skill_config_path(&self) -> Option<PathBuf> {
        self.default_mcp_config_path()
    }

    fn native_skill_discovery_roots(&self) -> Vec<PathBuf> {
        dirs::home_dir()
            .map(|home| vec![home.join(".agents").join("skills")])
            .unwrap_or_default()
    }

    fn native_skill_config_backend(&self) -> NativeSkillConfigBackend {
        NativeSkillConfigBackend::Codex
    }

    fn get_availability_info(&self) -> AvailabilityInfo {
        if let Some(timestamp) = codex_home()
            .and_then(|home| std::fs::metadata(home.join("auth.json")).ok())
            .and_then(|m| m.modified().ok())
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
        {
            return AvailabilityInfo::LoginDetected {
                last_auth_timestamp: timestamp,
            };
        }

        let mcp_config_found = self
            .default_mcp_config_path()
            .map(|p| p.exists())
            .unwrap_or(false);

        let installation_indicator_found = codex_home()
            .map(|home| home.join("version.json").exists())
            .unwrap_or(false);

        if mcp_config_found || installation_indicator_found {
            AvailabilityInfo::InstallationFound
        } else {
            AvailabilityInfo::NotFound
        }
    }

    async fn spawn_review(
        &self,
        current_dir: &Path,
        prompt: &str,
        session_id: Option<&str>,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        let command_parts = self.build_command_builder()?.build_initial()?;
        let review_target = ReviewTarget::Custom {
            instructions: prompt.to_string(),
        };
        let action = CodexSessionAction::Review {
            target: review_target,
        };
        self.spawn_inner(current_dir, command_parts, action, session_id, env)
            .await
    }
}

impl Codex {
    const BASE_COMMAND: &'static str = "npx -y @openai/codex@0.153.4";

    pub fn base_command() -> &'static str {
        Self::BASE_COMMAND
    }

    fn build_command_builder(&self) -> Result<CommandBuilder, CommandBuildError> {
        #[cfg(test)]
        let base_command = self
            .test_base_command
            .clone()
            .unwrap_or_else(|| Self::base_command().to_string());
        #[cfg(not(test))]
        let base_command = Self::base_command().to_string();

        let mut builder = CommandBuilder::new(base_command);
        builder = builder.extend_params(["app-server"]);
        if self.oss.unwrap_or(false) {
            builder = builder.extend_params(["--oss"]);
        }

        apply_overrides(builder, &self.cmd)
    }

    fn build_thread_start_params(&self, cwd: &Path) -> ThreadStartParams {
        let sandbox = match self.sandbox.as_ref() {
            None | Some(SandboxMode::Auto) => Some(AppServerSandboxMode::WorkspaceWrite), // match the Auto preset in codex
            Some(SandboxMode::ReadOnly) => Some(AppServerSandboxMode::ReadOnly),
            Some(SandboxMode::WorkspaceWrite) => Some(AppServerSandboxMode::WorkspaceWrite),
            Some(SandboxMode::DangerFullAccess) => Some(AppServerSandboxMode::DangerFullAccess),
        };

        let approval_policy = match self.ask_for_approval.as_ref() {
            None if matches!(self.sandbox.as_ref(), None | Some(SandboxMode::Auto)) => {
                // match the Auto preset in codex
                Some(AppServerAskForApproval::OnRequest)
            }
            None => None,
            Some(AskForApproval::UnlessTrusted) => Some(AppServerAskForApproval::UnlessTrusted),
            Some(AskForApproval::OnFailure) => Some(AppServerAskForApproval::Granular {
                sandbox_approval: true,
                rules: false,
                skill_approval: false,
                request_permissions: false,
                mcp_elicitations: false,
            }),
            Some(AskForApproval::OnRequest) => Some(AppServerAskForApproval::OnRequest),
            Some(AskForApproval::Never) => Some(AppServerAskForApproval::Never),
        };

        ThreadStartParams {
            model: self.model.clone(),
            cwd: Some(cwd.to_string_lossy().to_string()),
            approval_policy,
            sandbox,
            config: self.build_config_overrides(),
            base_instructions: self.base_instructions.clone(),
            developer_instructions: self.developer_instructions.clone(),
            model_provider: self.model_provider.clone(),
            ..Default::default()
        }
    }

    fn build_config_overrides(&self) -> Option<HashMap<String, Value>> {
        let mut overrides = HashMap::new();

        overrides.insert(
            "mcp_servers".to_string(),
            self.runtime_mcp_snapshot
                .as_ref()
                .map(|snapshot| snapshot.mcp_servers.clone())
                .unwrap_or_else(|| Value::Object(Map::new())),
        );

        if let Some(effort) = &self.model_reasoning_effort {
            overrides.insert(
                "model_reasoning_effort".to_string(),
                Value::String(effort.as_ref().to_string()),
            );
        }

        let reasoning_summary = self
            .model_reasoning_summary
            .as_ref()
            .unwrap_or(&ReasoningSummary::Auto);
        overrides.insert(
            "model_reasoning_summary".to_string(),
            Value::String(reasoning_summary.as_ref().to_string()),
        );

        if let Some(format) = &self.model_reasoning_summary_format
            && format != &ReasoningSummaryFormat::None
        {
            overrides.insert(
                "model_reasoning_summary_format".to_string(),
                Value::String(format.as_ref().to_string()),
            );
        }

        if let Some(profile) = &self.profile {
            overrides.insert("profile".to_string(), Value::String(profile.clone()));
        }

        if let Some(compact_prompt) = &self.compact_prompt {
            overrides.insert(
                "compact_prompt".to_string(),
                Value::String(compact_prompt.clone()),
            );
        }

        if let Some(include_apply_patch_tool) = self.include_apply_patch_tool {
            overrides.insert(
                "include_apply_patch_tool".to_string(),
                Value::Bool(include_apply_patch_tool),
            );
        }

        Some(overrides)
    }

    fn build_thread_resume_params(
        thread_params: ThreadStartParams,
        session_id: String,
        rollout_path: PathBuf,
    ) -> ThreadResumeParams {
        ThreadResumeParams {
            thread_id: session_id,
            path: Some(rollout_path),
            model: thread_params.model,
            model_provider: thread_params.model_provider,
            cwd: thread_params.cwd,
            approval_policy: thread_params.approval_policy,
            sandbox: thread_params.sandbox,
            config: thread_params.config,
            base_instructions: thread_params.base_instructions,
            developer_instructions: thread_params.developer_instructions,
            ..Default::default()
        }
    }

    async fn spawn_inner(
        &self,
        current_dir: &Path,
        command_parts: CommandParts,
        action: CodexSessionAction,
        resume_session: Option<&str>,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        let params = self.build_thread_start_params(current_dir);
        let resume_session = resume_session.map(|s| s.to_string());

        self.spawn_app_server(
            current_dir,
            command_parts,
            env,
            move |client, _| async move {
                match action {
                    CodexSessionAction::Chat { prompt } => {
                        Self::launch_codex_agent(params, resume_session, prompt, client).await
                    }
                    CodexSessionAction::Review { target } => {
                        review::launch_codex_review(params, resume_session, target, client).await
                    }
                }
            },
        )
        .await
    }

    async fn launch_codex_agent(
        thread_params: ThreadStartParams,
        resume_session: Option<String>,
        combined_prompt: String,
        client: Arc<AppServerClient>,
    ) -> Result<(), ExecutorError> {
        let auth_status = client.get_auth_status().await?;
        if auth_status.requires_openai_auth.unwrap_or(true) && auth_status.auth_method.is_none() {
            return Err(ExecutorError::AuthRequired(
                "Codex authentication required".to_string(),
            ));
        }
        match resume_session {
            None => {
                let response = client.start_thread(thread_params).await?;
                let thread_id = response.thread.id;
                client.register_session(&thread_id).await?;
                client.start_turn(thread_id, combined_prompt).await?;
            }
            Some(session_id) => {
                let (rollout_path, _forked_session_id) =
                    SessionHandler::fork_rollout_file(&session_id)
                        .map_err(|e| ExecutorError::FollowUpNotSupported(e.to_string()))?;
                let params = Self::build_thread_resume_params(
                    thread_params,
                    session_id,
                    rollout_path.clone(),
                );
                let response = client.resume_thread(params).await?;
                tracing::debug!(
                    rollout_path = %rollout_path.display(),
                    thread_id = %response.thread.id,
                    turns = response.thread.turns.len(),
                    model = %response.model,
                    model_provider = %response.model_provider,
                    "resumed session using rollout file"
                );
                let thread_id = response.thread.id;
                client.register_session(&thread_id).await?;
                client.start_turn(thread_id, combined_prompt).await?;
            }
        }
        Ok(())
    }

    /// Common boilerplate for spawning a Codex app server process
    /// Handles process spawning, stdout/stderr piping, exit signal handling, client initialization, and error logging.
    /// Delegates the actual Codex session logic to the provided `task` closure.
    async fn spawn_app_server<F, Fut>(
        &self,
        current_dir: &Path,
        _command_parts: CommandParts,
        env: &ExecutionEnv,
        task: F,
    ) -> Result<SpawnedChild, ExecutorError>
    where
        F: FnOnce(Arc<AppServerClient>, ExitSignalSender) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), ExecutorError>> + Send + 'static,
    {
        let snapshot = self
            .runtime_mcp_snapshot
            .as_ref()
            .ok_or(ExecutorError::McpIsolationNotImplemented)?;
        if self.cmd.base_command_override.is_some() {
            return Err(ExecutorError::Configuration(
                "Codex command changed after run-scoped MCP preparation".to_string(),
            ));
        }
        let output_redactor = snapshot.output_redactor.clone();
        let (program_path, args) = snapshot.process_command.parts();

        let mut process = Command::new(program_path);
        process
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(current_dir)
            .env("NPM_CONFIG_LOGLEVEL", "error")
            .env("NODE_NO_WARNINGS", "1")
            .env("NO_COLOR", "1")
            .env("RUST_LOG", "error")
            .args(args);

        env.clone()
            .with_profile(&self.cmd)
            .apply_to_command(&mut process);

        let mut child = process.group_spawn()?;

        let child_stdout = child.inner().stdout.take().ok_or_else(|| {
            ExecutorError::Io(std::io::Error::other("Codex app server missing stdout"))
        })?;
        let child_stdin = child.inner().stdin.take().ok_or_else(|| {
            ExecutorError::Io(std::io::Error::other("Codex app server missing stdin"))
        })?;

        let new_stdout = create_stdout_pipe_writer(&mut child)?;
        let (exit_signal_tx, exit_signal_rx) = tokio::sync::oneshot::channel();
        let cancel = tokio_util::sync::CancellationToken::new();

        let auto_approve = matches!(
            (&self.sandbox, &self.ask_for_approval),
            (Some(SandboxMode::DangerFullAccess), None)
        );
        let approvals = self.approvals.clone();
        let repo_context = env.repo_context.clone();
        let commit_reminder = env.commit_reminder;
        let commit_reminder_prompt = env.commit_reminder_prompt.clone();
        let cancel_for_task = cancel.clone();

        tokio::spawn(async move {
            let exit_signal_tx = ExitSignalSender::new(exit_signal_tx);
            let log_writer = LogWriter::new_with_redactor(new_stdout, output_redactor.clone());

            // Initialize the AppServerClient
            let client = AppServerClient::new(
                log_writer.clone(),
                approvals,
                auto_approve,
                repo_context,
                commit_reminder,
                commit_reminder_prompt,
                cancel_for_task.clone(),
            );
            let rpc_peer = JsonRpcPeer::spawn(
                child_stdin,
                child_stdout,
                client.clone(),
                exit_signal_tx.clone(),
                cancel_for_task,
            );
            client.connect(rpc_peer);

            let result = async {
                client.initialize().await?;
                task(client, exit_signal_tx.clone()).await
            }
            .await;

            if let Err(err) = result {
                match &err {
                    ExecutorError::Io(io_err)
                        if io_err.kind() == std::io::ErrorKind::BrokenPipe =>
                    {
                        // Broken pipe likely means the parent process exited, so we can ignore it
                        return;
                    }
                    ExecutorError::AuthRequired(message) => {
                        let safe_message = output_redactor.redact(message);
                        log_writer
                            .log_raw(&Error::auth_required(safe_message).raw())
                            .await
                            .ok();
                        exit_signal_tx
                            .send_exit_signal(ExecutorExitResult::Failure)
                            .await;
                        return;
                    }
                    _ => {
                        let safe_error = output_redactor.redact(&err.to_string());
                        tracing::error!("Codex spawn error: {}", safe_error);
                        log_writer
                            .log_raw(&Error::launch_error(safe_error).raw())
                            .await
                            .ok();
                    }
                }
                exit_signal_tx
                    .send_exit_signal(ExecutorExitResult::Failure)
                    .await;
            }
        });

        Ok(SpawnedChild {
            child,
            stdout: None,
            stderr: None,
            exit_signal: Some(exit_signal_rx),
            cancel: Some(cancel),
            cleanup: None,
        })
    }
}

fn build_codex_mcp_servers(canonical: &MemberMcpConfig) -> Result<Value, ExecutorError> {
    canonical.validate("codex").map_err(|_| {
        ExecutorError::Configuration("Codex rejected invalid member MCP configuration".to_string())
    })?;

    let mut servers = Map::new();
    for (name, definition) in &canonical.mcp_servers {
        let server = definition.as_object().ok_or_else(|| {
            ExecutorError::Configuration(
                "Codex rejected invalid member MCP configuration".to_string(),
            )
        })?;
        let is_remote = server.contains_key("url")
            || server.contains_key("httpUrl")
            || matches!(
                server.get("type").and_then(Value::as_str),
                Some("http" | "sse")
            );
        let mut converted = server.clone();
        converted.remove("type");
        converted.remove("disabled");
        if is_remote {
            if server.get("type").and_then(Value::as_str) == Some("sse") {
                return Err(ExecutorError::McpNotSupported);
            }
            let url = match (server.get("url"), server.get("httpUrl")) {
                (Some(url), Some(http_url)) if url != http_url => {
                    return Err(ExecutorError::Configuration(
                        "Codex rejected ambiguous member MCP remote URL fields".to_string(),
                    ));
                }
                (Some(url), _) | (_, Some(url)) => url.clone(),
                (None, None) => return Err(ExecutorError::McpNotSupported),
            };
            if url.as_str().is_none_or(|url| url.trim().is_empty()) {
                return Err(ExecutorError::Configuration(
                    "Codex rejected an empty member MCP remote URL".to_string(),
                ));
            }
            converted.remove("httpUrl");
            converted.insert("url".to_string(), url);
            if let Some(headers) = converted.remove("headers") {
                converted.insert("http_headers".to_string(), headers);
            }
        } else {
            if !matches!(
                server.get("type").and_then(Value::as_str),
                None | Some("stdio" | "local")
            ) {
                return Err(ExecutorError::McpNotSupported);
            }
            let command = server.get("command").cloned().ok_or_else(|| {
                ExecutorError::Configuration(
                    "Codex rejected a member MCP server without a command".to_string(),
                )
            })?;
            if command
                .as_str()
                .is_none_or(|command| command.trim().is_empty())
            {
                return Err(ExecutorError::Configuration(
                    "Codex rejected an empty member MCP command".to_string(),
                ));
            }
            converted.insert("command".to_string(), command);
        }
        if let Some(enabled) = effective_mcp_enabled(server) {
            converted.insert("enabled".to_string(), Value::Bool(enabled));
        }
        servers.insert(name.clone(), Value::Object(converted));
    }
    Ok(Value::Object(servers))
}

fn effective_mcp_enabled(server: &Map<String, Value>) -> Option<bool> {
    match (
        server.get("enabled").and_then(Value::as_bool),
        server.get("disabled").and_then(Value::as_bool),
    ) {
        (_, Some(true)) => Some(false),
        (Some(enabled), _) => Some(enabled),
        (None, Some(false)) => Some(true),
        (None, None) => None,
    }
}

fn codex_config_has_provider(path: &Path, env: &ExecutionEnv) -> bool {
    let Some(value) = std::fs::read_to_string(path)
        .ok()
        .and_then(|content| toml::from_str::<toml::Value>(&content).ok())
    else {
        return false;
    };
    let providers = value.get("model_providers").and_then(toml::Value::as_table);
    let configured_provider = value
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .is_some_and(|provider| !provider.trim().is_empty());
    let provider_entry = providers.is_some_and(|providers| !providers.is_empty());
    let referenced_key = providers
        .into_iter()
        .flat_map(|providers| providers.values())
        .any(|provider| {
            provider
                .get("env_key")
                .and_then(toml::Value::as_str)
                .is_some_and(|key| {
                    env.get(key).is_some_and(|value| !value.trim().is_empty())
                        || std::env::var_os(key).is_some_and(|value| !value.is_empty())
                })
        });
    configured_provider || provider_entry || referenced_key
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        time::Duration,
    };

    use serde_json::json;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{CODEX_MODEL_FALLBACKS, Codex, build_codex_mcp_servers, codex_config_has_provider};
    use crate::{
        command::CmdOverrides,
        env::{ExecutionEnv, RepoContext},
        executors::{ExecutorError, StandardCodingAgentExecutor},
        mcp_config::MemberMcpConfig,
        mcp_run::McpRunContext,
    };

    fn test_codex() -> Codex {
        let mut codex: Codex =
            serde_json::from_value(json!({})).expect("deserialize Codex test config");
        codex.test_base_command = Some(
            std::env::current_exe()
                .expect("resolve current test executable")
                .to_string_lossy()
                .to_string(),
        );
        codex
    }

    fn run_context(workspace: &TempDir) -> McpRunContext {
        McpRunContext::new(workspace.path(), Uuid::new_v4(), Uuid::new_v4())
            .expect("create MCP run context")
    }

    fn execution_env(workspace: &TempDir) -> ExecutionEnv {
        ExecutionEnv::new(
            RepoContext::new(workspace.path().to_path_buf(), Vec::new()),
            false,
            String::new(),
        )
    }

    #[cfg(unix)]
    fn write_executable_script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, body).expect("write fake Codex app server");
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).expect("make fake Codex app server executable");
    }

    #[test]
    fn base_command_uses_codex_0_153_4() {
        assert_eq!(Codex::base_command(), "npx -y @openai/codex@0.153.4");
    }

    #[test]
    fn codex_model_fallbacks_include_gpt_6_and_gpt_5_6_models() {
        assert!(CODEX_MODEL_FALLBACKS.contains(&"gpt-6-astra"));
        assert!(CODEX_MODEL_FALLBACKS.contains(&"gpt-5.6-sol"));
        assert!(CODEX_MODEL_FALLBACKS.contains(&"gpt-5.6-terra"));
        assert!(CODEX_MODEL_FALLBACKS.contains(&"gpt-5.6-luna"));
    }

    #[tokio::test]
    async fn codex_thread_start_and_resume_use_same_frozen_mcp_override() {
        let workspace = TempDir::new().expect("create workspace");
        let mut codex = test_codex();
        let mut env = execution_env(&workspace);
        let mut canonical: MemberMcpConfig = serde_json::from_value(json!({
            "mcpServers": {
                "local": {
                    "command": "/bin/echo",
                    "args": ["serve"],
                    "env": {"TOKEN": "fixed-secret"}
                },
                "remote": {
                    "type": "http",
                    "url": "https://example.test/mcp",
                    "headers": {"Authorization": "Bearer fixed-secret"}
                }
            }
        }))
        .expect("deserialize canonical MCP config");
        codex
            .prepare_mcp_for_run(&canonical, &run_context(&workspace), &mut env)
            .await
            .expect("prepare Codex MCP snapshot");

        canonical.mcp_servers.clear();
        let start = codex.build_thread_start_params(workspace.path());
        let start_wire = serde_json::to_value(&start).expect("serialize thread start params");
        let resume = Codex::build_thread_resume_params(
            start,
            "thread-valid".to_string(),
            PathBuf::from("/tmp/rollout.jsonl"),
        );
        let resume_wire = serde_json::to_value(&resume).expect("serialize thread resume params");

        assert_eq!(
            start_wire["config"]["mcp_servers"],
            resume_wire["config"]["mcp_servers"]
        );
        assert_eq!(resume_wire["threadId"], json!("thread-valid"));
        assert_eq!(
            start_wire["config"]["mcp_servers"]["local"]["env"]["TOKEN"],
            json!("fixed-secret")
        );
        assert_eq!(
            start_wire["config"]["mcp_servers"]["remote"]["http_headers"]["Authorization"],
            json!("Bearer fixed-secret")
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn codex_real_app_server_preserves_config_bytes_and_pipes_stderr() {
        let workspace = TempDir::new().expect("create workspace");
        let codex_home = workspace.path().join("user-codex-home");
        std::fs::create_dir_all(&codex_home).expect("create Codex home");
        let global_config = codex_home.join("config.toml");
        let project_config_dir = workspace.path().join(".codex");
        fs::create_dir_all(&project_config_dir).expect("create project Codex config directory");
        let project_config = project_config_dir.join("config.toml");
        let global_bytes = b"[mcp_servers.global]\ncommand = \"global-secret\"\n";
        let project_bytes = b"[mcp_servers.project]\ncommand = \"project-secret\"\n";
        std::fs::write(&global_config, global_bytes).expect("write global config sentinel");
        std::fs::write(&project_config, project_bytes).expect("write project config sentinel");
        let captured_thread = workspace.path().join("captured-thread-start.json");
        let captured_home = workspace.path().join("captured-codex-home.txt");
        let app_server_script = workspace.path().join("fake-codex-app-server.sh");
        write_executable_script(
            &app_server_script,
            "#!/bin/sh\nIFS= read -r initialize_request\nprintf '%s\\n' '{\"id\":1,\"result\":{\"userAgent\":\"fake-codex\",\"codexHome\":\"/tmp/fake-codex-home\",\"platformFamily\":\"unix\",\"platformOs\":\"macos\"}}'\nIFS= read -r initialized_notification\nIFS= read -r thread_request\nprintf '%s' \"$thread_request\" > \"$CAPTURED_THREAD\"\nprintf '%s' \"$CODEX_HOME\" > \"$CAPTURED_HOME\"\nexit 0\n",
        );
        let mut codex = test_codex();
        codex.test_base_command = Some(app_server_script.to_string_lossy().to_string());
        codex.cmd.env = Some(HashMap::from([
            (
                "CODEX_HOME".to_string(),
                codex_home.to_string_lossy().to_string(),
            ),
            (
                "CAPTURED_THREAD".to_string(),
                captured_thread.to_string_lossy().to_string(),
            ),
            (
                "CAPTURED_HOME".to_string(),
                captured_home.to_string_lossy().to_string(),
            ),
        ]));
        let mut env = execution_env(&workspace);

        let prepared = codex
            .prepare_mcp_for_run(
                &MemberMcpConfig::default(),
                &run_context(&workspace),
                &mut env,
            )
            .await
            .expect("prepare empty Codex MCP snapshot");
        let params = serde_json::to_value(codex.build_thread_start_params(workspace.path()))
            .expect("serialize thread start params");

        assert_eq!(params["config"]["mcp_servers"], json!({}));
        let thread_params = codex.build_thread_start_params(workspace.path());
        let command_parts = codex
            .build_command_builder()
            .unwrap()
            .build_initial()
            .unwrap();
        let mut spawned = codex
            .spawn_app_server(
                workspace.path(),
                command_parts,
                &env,
                move |client, _| async move {
                    client.start_thread(thread_params).await?;
                    Ok(())
                },
            )
            .await
            .expect("spawn fake Codex app server through real process boundary");
        assert!(
            spawned.child.inner().stderr.is_some(),
            "Codex app server stderr must stay piped for workflow log forwarding"
        );
        let exit_signal = spawned.exit_signal.take().expect("Codex exit signal");
        tokio::time::timeout(Duration::from_secs(5), exit_signal)
            .await
            .expect("fake Codex app server exit timeout")
            .expect("receive fake Codex app server exit");

        assert_eq!(std::fs::read(&global_config).unwrap(), global_bytes);
        assert_eq!(std::fs::read(&project_config).unwrap(), project_bytes);
        assert_eq!(
            fs::read_to_string(&captured_home).unwrap(),
            codex_home.to_string_lossy()
        );
        let thread_request: serde_json::Value =
            serde_json::from_slice(&fs::read(&captured_thread).unwrap()).unwrap();
        assert_eq!(thread_request["method"], json!("thread/start"));
        assert_eq!(thread_request["params"]["config"]["mcp_servers"], json!({}));
        assert!(prepared.into_cleanup().is_none());
    }

    #[tokio::test]
    async fn codex_rejects_unsupported_transport_and_unverified_command_override() {
        let workspace = TempDir::new().expect("create workspace");
        let mut env = execution_env(&workspace);
        let sse: MemberMcpConfig = serde_json::from_value(json!({
            "mcpServers": {
                "events": {"type": "sse", "url": "https://example.test/events"}
            }
        }))
        .expect("deserialize SSE config");
        let error = test_codex()
            .prepare_mcp_for_run(&sse, &run_context(&workspace), &mut env)
            .await
            .expect_err("Codex must reject SSE before spawn");
        assert!(matches!(error, ExecutorError::McpNotSupported));

        let mut overridden = test_codex();
        overridden.cmd = CmdOverrides {
            base_command_override: Some("unverified-codex".to_string()),
            ..Default::default()
        };
        let error = overridden
            .prepare_mcp_for_run(
                &MemberMcpConfig::default(),
                &run_context(&workspace),
                &mut env,
            )
            .await
            .expect_err("custom Codex command must fail closed");
        assert!(matches!(error, ExecutorError::Configuration(_)));
    }

    #[tokio::test]
    async fn codex_spawn_fails_closed_before_command_resolution_without_preparation() {
        let workspace = TempDir::new().expect("create workspace");
        let mut codex = test_codex();
        codex.cmd.base_command_override = Some("definitely-not-a-real-codex-command".to_string());

        let error = codex
            .spawn(workspace.path(), "hello", &execution_env(&workspace))
            .await
            .expect_err("unprepared Codex spawn must fail closed");

        assert!(matches!(error, ExecutorError::McpIsolationNotImplemented));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_rejects_base_command_added_after_preparation_before_spawn() {
        let workspace = TempDir::new().expect("create workspace");
        let marker = workspace.path().join("app-server-started");
        let app_server_script = workspace.path().join("must-not-start-codex.sh");
        write_executable_script(
            &app_server_script,
            "#!/bin/sh\nprintf started > \"$START_MARKER\"\nwhile :; do sleep 1; done\n",
        );
        let mut codex = test_codex();
        let mut env = execution_env(&workspace);
        let _prepared = codex
            .prepare_mcp_for_run(
                &MemberMcpConfig::default(),
                &run_context(&workspace),
                &mut env,
            )
            .await
            .expect("prepare Codex MCP snapshot");
        codex.cmd.base_command_override = Some(app_server_script.to_string_lossy().to_string());
        env.insert("START_MARKER", marker.to_string_lossy().to_string());

        let error = codex
            .spawn(workspace.path(), "hello", &env)
            .await
            .expect_err("post-preparation Codex command override must fail");

        assert!(matches!(error, ExecutorError::Configuration(_)));
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn codex_runtime_snapshot_debug_redacts_mcp_secrets() {
        let workspace = TempDir::new().expect("create workspace");
        let secret = "codex-mcp-secret-never-log";
        let canonical: MemberMcpConfig = serde_json::from_value(json!({
            "mcpServers": {
                "local": {"command": "/bin/echo", "env": {"TOKEN": secret}}
            }
        }))
        .expect("deserialize secret config");
        let mut codex = test_codex();
        let mut env = execution_env(&workspace);
        let prepared = codex
            .prepare_mcp_for_run(&canonical, &run_context(&workspace), &mut env)
            .await
            .expect("prepare Codex snapshot");

        let snapshot_debug = format!("{:?}", codex.runtime_mcp_snapshot.as_ref().unwrap());
        assert!(!snapshot_debug.contains(secret));
        assert!(!format!("{codex:?}").contains(secret));
        assert!(!format!("{prepared:?}").contains(secret));
    }

    #[test]
    fn codex_mcp_translation_preserves_extension_fields() {
        let canonical: MemberMcpConfig = serde_json::from_value(json!({
            "mcpServers": {
                "computer-use": {
                    "command": "/bin/echo",
                    "cwd": ".",
                    "enabled": false
                },
                "node_repl": {
                    "command": "/bin/echo",
                    "startup_timeout_sec": 120,
                    "future_option": {"nested": true}
                },
                "remote": {
                    "url": "https://example.test/mcp",
                    "headers": {"X-Mode": "test"},
                    "future_remote_option": "preserved",
                    "disabled": false
                }
            }
        }))
        .expect("deserialize config");

        let converted = build_codex_mcp_servers(&canonical).expect("translate MCP config");

        assert_eq!(converted["computer-use"]["cwd"], json!("."));
        assert_eq!(converted["computer-use"]["enabled"], json!(false));
        assert_eq!(converted["node_repl"]["startup_timeout_sec"], json!(120));
        assert_eq!(
            converted["node_repl"]["future_option"],
            json!({"nested": true})
        );
        assert_eq!(
            converted["remote"]["future_remote_option"],
            json!("preserved")
        );
        assert_eq!(converted["remote"]["http_headers"]["X-Mode"], "test");
        assert_eq!(converted["remote"]["enabled"], json!(true));
        assert!(converted["remote"].get("headers").is_none());
        assert!(converted["remote"].get("disabled").is_none());
    }

    #[test]
    fn configured_model_provider_counts_as_authentication() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
                model_provider = "custom"
                [model_providers.custom]
                base_url = "http://localhost:11434/v1"
            "#,
        )
        .unwrap();
        let env = ExecutionEnv::new(Default::default(), false, String::new());

        assert!(codex_config_has_provider(&path, &env));
    }
}
