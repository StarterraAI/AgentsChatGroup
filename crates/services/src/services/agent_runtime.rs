use std::{
    collections::{BTreeSet, HashMap},
    fs,
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use executors::{
    command::{CmdOverrides, CommandBuilder, redacted_command},
    env::ExecutionEnv,
    executors::{
        AcpModelFallback, AcpProbeAuthState, AvailabilityInfo, BaseCodingAgent, CodingAgent,
        ExecutorError, StandardCodingAgentExecutor, acp::AcpCapabilityProbe, codex::Codex,
        opencode::Opencode, pi::Pi,
    },
    profile::{ExecutorConfig, ExecutorConfigs, ProfileError},
};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{process::Command, time::timeout};
use ts_rs::TS;

use super::pi_models::{PiModelsSyncDiagnostic, coordinate_pi_models_with_diagnostic};

const STORE_FILE_NAME: &str = "agent_runtime_config.json";
const RUNTIME_DISCOVERY_CONCURRENCY: usize = 4;
const CLI_PROBE_CACHE_TTL: Duration = Duration::from_secs(30);

static RUNTIME_REFRESH_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static RUNTIME_STORE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static CLI_PROBE_GATES: LazyLock<DashMap<CliProbeCacheKey, Arc<tokio::sync::Mutex<()>>>> =
    LazyLock::new(DashMap::new);
static CLI_RUNNER_GATES: LazyLock<DashMap<BaseCodingAgent, Arc<tokio::sync::Mutex<()>>>> =
    LazyLock::new(DashMap::new);
static CLI_PROBE_CACHE: LazyLock<DashMap<CliProbeCacheKey, CachedCliProbe>> =
    LazyLock::new(DashMap::new);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliProbeCachePolicy {
    Reuse,
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CliProbeRequestKey {
    runner: BaseCodingAgent,
    current_dir: PathBuf,
    execution_fingerprint: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CliProbeKind {
    Version,
    Models,
    Acp,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CliProbeCacheKey {
    request: CliProbeRequestKey,
    kind: CliProbeKind,
}

#[derive(Debug, Clone)]
enum CliProbeValue {
    Version(Option<String>),
    Models(Option<Vec<String>>),
    Acp(AcpProbeOutcome),
    Command(Option<ResolvedRuntimeCommand>),
}

#[derive(Debug, Clone)]
enum AcpProbeOutcome {
    Probed(Option<AcpCapabilityProbe>),
    Unauthenticated,
}

#[derive(Debug, Clone)]
struct CachedCliProbe {
    completed_at: Instant,
    result: Result<CliProbeValue, String>,
}

#[derive(Debug, Error)]
pub enum AgentRuntimeError {
    #[error("invalid environment variable key: {0}")]
    InvalidEnvKey(String),
    #[error("unknown runner: {0}")]
    UnknownRunner(String),
    #[error("invalid runtime workspace path: {0}")]
    InvalidWorkspacePath(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Profile(#[from] ProfileError),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentRunMode {
    #[default]
    Auto,
    Local,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
pub struct AgentRuntimeConfig {
    pub runner_type: BaseCodingAgent,
    pub run_mode: AgentRunMode,
    pub env_json: HashMap<String, String>,
    #[serde(default)]
    #[ts(type = "JsonValue")]
    pub executor_options: Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
pub struct UpdateAgentRuntimeConfig {
    pub run_mode: Option<AgentRunMode>,
    pub env_json: Option<HashMap<String, String>>,
    #[ts(type = "JsonValue | null")]
    pub executor_options: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
pub struct AgentRuntimeEnvSummary {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentRuntimeModelSource {
    Runner,
    ProfileFallback,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentRuntimeAuthState {
    Authenticated,
    Unauthenticated,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentRuntimeStatus {
    pub runner_type: BaseCodingAgent,
    pub installed: bool,
    pub executable: bool,
    pub availability: AvailabilityInfo,
    pub auth_state: AgentRuntimeAuthState,
    /// Whether a Node.js runtime was detected on this machine. Drives the
    /// "install Node.js" guidance for Node-based runners.
    pub node_available: bool,
    /// Whether the npm CLI was detected through the login-shell PATH.
    pub npm_available: bool,
    /// Whether the npx CLI was detected through the login-shell PATH.
    pub npx_available: bool,
    pub discovered_models: Vec<String>,
    pub model_source: AgentRuntimeModelSource,
    pub version: Option<String>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub run_mode: AgentRunMode,
    pub env_summary: Vec<AgentRuntimeEnvSummary>,
    #[ts(type = "JsonValue")]
    pub executor_options: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum AgentRuntimeReasoningCapability {
    Effort { options: Vec<String> },
    Variant { options: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentRuntimeListResponse {
    pub runners: Vec<AgentRuntimeStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pi_models_sync: Option<PiModelsSyncDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
pub struct AgentRuntimeRefreshError {
    pub runner_type: BaseCodingAgent,
    pub message: String,
    pub preserved_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentRuntimeRefreshResponse {
    pub runners: Vec<AgentRuntimeStatus>,
    pub errors: Vec<AgentRuntimeRefreshError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pi_models_sync: Option<PiModelsSyncDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentRuntimeDiagnostics {
    pub runner_type: BaseCodingAgent,
    pub installed: bool,
    pub executable: bool,
    pub availability: AvailabilityInfo,
    pub auth_state: AgentRuntimeAuthState,
    pub node_available: bool,
    pub npm_available: bool,
    pub npx_available: bool,
    pub config_path: String,
    pub install_indicator_path: Option<String>,
    pub resolved_command: Option<String>,
    pub command_source: Option<String>,
    pub acp_probe: Option<AcpCapabilityProbe>,
    pub acp_probe_error: Option<String>,
    pub discovered_models: Vec<String>,
    pub model_source: AgentRuntimeModelSource,
    pub version: Option<String>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub run_mode: AgentRunMode,
    pub env_summary: Vec<AgentRuntimeEnvSummary>,
    #[ts(type = "JsonValue")]
    pub executor_options: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pi_models_sync: Option<PiModelsSyncDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct AgentRuntimeDiscovery {
    models: Vec<String>,
    version: Option<String>,
    #[serde(default)]
    auth_state: Option<AgentRuntimeAuthState>,
    last_checked_at: DateTime<Utc>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
struct AgentRuntimeStore {
    #[serde(default)]
    configs: HashMap<BaseCodingAgent, AgentRuntimeConfig>,
    #[serde(default)]
    discoveries: HashMap<BaseCodingAgent, AgentRuntimeDiscovery>,
}

pub fn store_path() -> PathBuf {
    utils::assets::asset_dir().join(STORE_FILE_NAME)
}

pub fn resolve_runtime_probe_dir(
    workspace_path: Option<&Path>,
) -> Result<PathBuf, AgentRuntimeError> {
    let Some(workspace_path) = workspace_path else {
        let fallback = utils::assets::asset_dir().join("agent-runtime");
        fs::create_dir_all(&fallback)?;
        return Ok(fallback);
    };

    if workspace_path.as_os_str().is_empty() {
        return Err(AgentRuntimeError::InvalidWorkspacePath(
            "path cannot be empty".to_string(),
        ));
    }

    let resolved = fs::canonicalize(workspace_path).map_err(|error| {
        AgentRuntimeError::InvalidWorkspacePath(format!("{}: {error}", workspace_path.display()))
    })?;
    if !resolved.is_dir() {
        return Err(AgentRuntimeError::InvalidWorkspacePath(format!(
            "{} is not a directory",
            workspace_path.display()
        )));
    }
    Ok(resolved)
}

pub async fn list_runtime_statuses() -> Result<AgentRuntimeListResponse, AgentRuntimeError> {
    let pi_models_sync = coordinate_pi_models_with_diagnostic().await;
    let store = read_store(&store_path())?;
    let profiles = ExecutorConfigs::get_cached();
    Ok(AgentRuntimeListResponse {
        runners: build_statuses(&profiles, &store),
        pi_models_sync: Some(pi_models_sync),
    })
}

pub async fn list_runtime_statuses_with_discovery(
    _current_dir: &Path,
) -> Result<AgentRuntimeListResponse, AgentRuntimeError> {
    list_runtime_statuses().await
}

/// Rebuilds the runtime snapshot exclusively from local files and cached
/// metadata. This is safe for startup, list, and window-focus paths because it
/// never runs a CLI command.
pub async fn refresh_runtime_statuses() -> Result<AgentRuntimeRefreshResponse, AgentRuntimeError> {
    let pi_models_sync = coordinate_pi_models_with_diagnostic().await;
    let store = read_store(&store_path())?;
    let profiles = ExecutorConfigs::get_cached();
    Ok(AgentRuntimeRefreshResponse {
        runners: build_statuses(&profiles, &store),
        errors: Vec::new(),
        pi_models_sync: Some(pi_models_sync),
    })
}

/// Reconciles profile-backed model changes into a runner's cached discovery so
/// lightweight status reads immediately expose additions and removals while
/// retaining models that came only from the CLI probe.
pub async fn reconcile_runtime_model_discovery(
    runner: BaseCodingAgent,
    previous_profile_models: &[String],
    current_profile_models: &[String],
) -> Result<(), AgentRuntimeError> {
    let _guard = RUNTIME_REFRESH_LOCK.lock().await;
    update_store(&store_path(), |store| {
        reconcile_cached_runtime_models(
            store,
            runner,
            previous_profile_models,
            current_profile_models,
        );
        Ok(())
    })?;
    CLI_PROBE_CACHE.retain(|key, _| {
        key.request.runner != runner
            || !matches!(key.kind, CliProbeKind::Models | CliProbeKind::Acp)
    });
    Ok(())
}

pub async fn refresh_runtime_discovery(
    current_dir: &Path,
) -> Result<AgentRuntimeRefreshResponse, AgentRuntimeError> {
    let pi_models_sync = coordinate_pi_models_with_diagnostic().await;
    let _guard = RUNTIME_REFRESH_LOCK.lock().await;
    let mut response = refresh_runtime_discovery_unlocked(current_dir).await?;
    response.pi_models_sync = Some(pi_models_sync);
    Ok(response)
}

async fn refresh_runtime_discovery_unlocked(
    current_dir: &Path,
) -> Result<AgentRuntimeRefreshResponse, AgentRuntimeError> {
    let path = store_path();
    let store = read_store(&path)?;
    let profiles = ExecutorConfigs::get_cached();
    let current_dir = current_dir.to_path_buf();
    let store_snapshot = Arc::new(store.clone());
    let discovery_inputs = profiles
        .executors
        .iter()
        .map(|(runner, executor_config)| (*runner, executor_config.clone()))
        .collect::<Vec<_>>();

    let outcomes = stream::iter(
        discovery_inputs
            .into_iter()
            .map(|(runner, executor_config)| {
                let current_dir = current_dir.clone();
                let store = Arc::clone(&store_snapshot);
                async move {
                    discover_runner_runtime(runner, &executor_config, &store, &current_dir).await
                }
            }),
    )
    .buffer_unordered(RUNTIME_DISCOVERY_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    let (store, errors) = update_store(&path, |latest| {
        Ok(apply_discovery_outcomes(latest, outcomes))
    })?;
    Ok(AgentRuntimeRefreshResponse {
        runners: build_statuses(&profiles, &store),
        errors,
        pi_models_sync: None,
    })
}

enum RunnerDiscoveryOutcome {
    Skipped,
    Discovered {
        runner: BaseCodingAgent,
        models: Option<Vec<String>>,
        detected_version: Option<String>,
        version_error: Option<String>,
        auth_state: Option<AgentRuntimeAuthState>,
    },
    Failed {
        runner: BaseCodingAgent,
        message: String,
        detected_version: Option<String>,
        preserved_models: Vec<String>,
    },
}

fn apply_discovery_outcomes(
    store: &mut AgentRuntimeStore,
    outcomes: Vec<RunnerDiscoveryOutcome>,
) -> Vec<AgentRuntimeRefreshError> {
    let mut errors = Vec::new();
    for outcome in outcomes {
        match outcome {
            RunnerDiscoveryOutcome::Skipped => {}
            RunnerDiscoveryOutcome::Discovered {
                runner,
                models,
                detected_version,
                version_error,
                auth_state,
            } => {
                let previous = store.discoveries.get(&runner);
                let models = models.unwrap_or_else(|| {
                    previous
                        .map(|entry| entry.models.clone())
                        .unwrap_or_default()
                });
                let auth_state = auth_state.or_else(|| previous.and_then(|entry| entry.auth_state));
                let version = version_for_discovery_update(store, runner, detected_version);
                let last_error =
                    version_error.map(|error| status_error_detail("version_check", error));
                if let Some(message) = last_error.clone() {
                    errors.push(AgentRuntimeRefreshError {
                        runner_type: runner,
                        message,
                        preserved_models: models.clone(),
                    });
                }
                store.discoveries.insert(
                    runner,
                    AgentRuntimeDiscovery {
                        models,
                        version,
                        auth_state,
                        last_checked_at: Utc::now(),
                        last_error,
                    },
                );
            }
            RunnerDiscoveryOutcome::Failed {
                runner,
                message,
                detected_version,
                preserved_models,
            } => {
                store
                    .discoveries
                    .entry(runner)
                    .and_modify(|entry| {
                        entry.last_checked_at = Utc::now();
                        entry.last_error = Some(message.clone());
                        if let Some(version) = detected_version.clone() {
                            entry.version = Some(version);
                        }
                    })
                    .or_insert_with(|| AgentRuntimeDiscovery {
                        models: Vec::new(),
                        version: detected_version.clone(),
                        auth_state: None,
                        last_checked_at: Utc::now(),
                        last_error: Some(message.clone()),
                    });
                errors.push(AgentRuntimeRefreshError {
                    runner_type: runner,
                    message,
                    preserved_models,
                });
            }
        }
    }
    errors
}

async fn discover_runner_runtime(
    runner: BaseCodingAgent,
    executor_config: &ExecutorConfig,
    store: &AgentRuntimeStore,
    current_dir: &Path,
) -> RunnerDiscoveryOutcome {
    let Some(mut base) = executor_config
        .get_default()
        .or_else(|| executor_config.configurations.values().next())
        .cloned()
    else {
        return RunnerDiscoveryOutcome::Skipped;
    };

    let mut env = ExecutionEnv::new(Default::default(), false, String::new());
    if let Err(error) = apply_config_to_executor_and_env(runner, &mut base, &mut env, store) {
        return RunnerDiscoveryOutcome::Failed {
            runner,
            message: status_error_detail("runtime_configuration", error),
            detected_version: None,
            preserved_models: models_for_runner(runner, executor_config, store),
        };
    }
    if !base.get_availability_info().is_available() {
        return RunnerDiscoveryOutcome::Skipped;
    }

    let (version_result, discovered_models) = tokio::join!(
        coordinated_detect_cli_version(
            runner,
            &base,
            current_dir,
            &env,
            None,
            CliProbeCachePolicy::Refresh,
        ),
        discover_models_for_executor(runner, &base, current_dir, &env)
    );
    let (detected_version, version_error) = split_probe_result(version_result);

    match discovered_models {
        Ok((models, auth_state)) => RunnerDiscoveryOutcome::Discovered {
            runner,
            models,
            detected_version,
            version_error,
            auth_state,
        },
        Err(message) => RunnerDiscoveryOutcome::Failed {
            runner,
            message: merge_status_error_details([
                Some(status_error_detail("model_discovery", message)),
                version_error.map(|error| status_error_detail("version_check", error)),
            ])
            .expect("model discovery failure always produces an error detail"),
            detected_version,
            preserved_models: models_for_runner(runner, executor_config, store),
        },
    }
}

pub fn update_runtime_config(
    runner: BaseCodingAgent,
    payload: UpdateAgentRuntimeConfig,
) -> Result<AgentRuntimeStatus, AgentRuntimeError> {
    let path = store_path();
    let profiles = ExecutorConfigs::get_cached();

    if !profiles.executors.contains_key(&runner) {
        return Err(AgentRuntimeError::UnknownRunner(runner.to_string()));
    }

    let (store, ()) = update_store(&path, |store| {
        let mut config = store
            .configs
            .get(&runner)
            .cloned()
            .unwrap_or_else(|| default_config(runner));

        if let Some(run_mode) = payload.run_mode {
            config.run_mode = run_mode;
        }
        if let Some(env_json) = payload.env_json {
            validate_env_json(&env_json)?;
            config.env_json = env_json;
        }
        if let Some(executor_options) = payload.executor_options {
            let mut executor = profiles
                .executors
                .get(&runner)
                .and_then(|entry| {
                    entry
                        .get_default()
                        .or_else(|| entry.configurations.values().next())
                })
                .cloned()
                .ok_or_else(|| AgentRuntimeError::UnknownRunner(runner.to_string()))?;
            apply_executor_options(runner, &mut executor, &executor_options)?;
            config.executor_options = executor_options;
        }
        config.updated_at = Utc::now();

        store.configs.insert(runner, config);
        Ok(())
    })?;

    let status = build_statuses(&profiles, &store)
        .into_iter()
        .find(|status| status.runner_type == runner)
        .ok_or_else(|| AgentRuntimeError::UnknownRunner(runner.to_string()))?;
    Ok(status)
}

pub async fn runtime_diagnostics(
    runner: BaseCodingAgent,
    probe_dir: &Path,
    auth_method_id: Option<&str>,
) -> Result<AgentRuntimeDiagnostics, AgentRuntimeError> {
    let pi_models_sync = if runner == BaseCodingAgent::Pi {
        Some(coordinate_pi_models_with_diagnostic().await)
    } else {
        None
    };
    let path = store_path();
    let store = read_store(&path)?;
    let profiles = ExecutorConfigs::get_cached();
    let config = profiles
        .executors
        .get(&runner)
        .ok_or_else(|| AgentRuntimeError::UnknownRunner(runner.to_string()))?;
    let Some(base) = config
        .get_default()
        .or_else(|| config.configurations.values().next())
    else {
        return Err(AgentRuntimeError::UnknownRunner(runner.to_string()));
    };

    let dependency_availability = detect_runtime_dependency_availability();
    let status = build_status(runner, config, base, &store, dependency_availability);
    let mut runtime_executor = base.clone();
    let mut env = ExecutionEnv::new(Default::default(), false, String::new());
    apply_config_to_executor_and_env(runner, &mut runtime_executor, &mut env, &store)?;
    let runtime_config_path = runtime_executor
        .default_runtime_config_path()
        .map(|path| path.display().to_string());

    let version_result = if status.installed {
        coordinated_detect_cli_version(
            runner,
            &runtime_executor,
            probe_dir,
            &env,
            auth_method_id,
            CliProbeCachePolicy::Reuse,
        )
        .await
    } else {
        Ok(None)
    };
    let (detected_version, version_error) = split_probe_result(version_result);
    let (resolved_runtime_command, command_error) = split_probe_result(
        coordinated_resolve_runtime_command(
            runner,
            &runtime_executor,
            probe_dir,
            &env,
            auth_method_id,
            status.installed,
            CliProbeCachePolicy::Reuse,
        )
        .await,
    );
    let command_source = resolved_runtime_command.as_ref().map(|_| {
        if cmd_overrides_for_executor(&runtime_executor)
            .and_then(|cmd| cmd.base_command_override.as_deref())
            .is_some_and(|command| !command.trim().is_empty())
        {
            "override".to_string()
        } else {
            match &runtime_executor {
                CodingAgent::Gemini(_)
                | CodingAgent::QwenCode(_)
                | CodingAgent::KimiCode(_)
                | CodingAgent::QoderCli(_)
                | CodingAgent::Hermes(_)
                | CodingAgent::KiroCli(_) => "native",
                CodingAgent::Pi(_) => "npx",
                _ => "default",
            }
            .to_string()
        }
    });
    let install_indicator_path = resolved_runtime_command
        .as_ref()
        .map(|command| command.executable_path.clone());
    let resolved_command = resolved_runtime_command.map(|command| command.rendered);
    let (acp_probe, acp_probe_error, acp_probe_completed, acp_auth_state_override) =
        if status.installed {
            match coordinated_probe_acp(
                runner,
                &runtime_executor,
                probe_dir,
                &env,
                auth_method_id,
                CliProbeCachePolicy::Reuse,
            )
            .await
            {
                Ok(AcpProbeOutcome::Probed(probe)) => (probe, None, true, None),
                Ok(AcpProbeOutcome::Unauthenticated) => (
                    None,
                    None,
                    true,
                    Some(AgentRuntimeAuthState::Unauthenticated),
                ),
                Err(error) => (None, Some(error), false, None),
            }
        } else {
            (None, None, false, None)
        };
    let acp_interpretation = acp_probe
        .as_ref()
        .map(|probe| runtime_executor.interpret_acp_probe(probe));
    let acp_probe_models = acp_interpretation
        .as_ref()
        .and_then(|interpretation| interpretation.models.clone());
    let acp_auth_state = acp_auth_state_override.or_else(|| {
        acp_interpretation
            .as_ref()
            .and_then(|interpretation| interpretation.auth_state)
            .map(agent_runtime_auth_state)
    });
    let latest_store = if detected_version.is_some() || acp_probe_completed {
        update_store(&path, |latest| {
            if let Some(version) = detected_version.as_deref() {
                cache_runner_version(latest, runner, version.to_string());
            }
            if acp_probe_completed {
                clear_cached_authentication_required_error(latest, runner);
                if let Some(models) = acp_probe_models.as_ref() {
                    cache_runner_acp_models(latest, runner, models.clone());
                }
                if let Some(auth_state) = acp_auth_state {
                    cache_runner_auth_state(latest, runner, auth_state);
                }
            }
            Ok(())
        })?
        .0
    } else {
        read_store(&path)?
    };
    let latest_status = build_status(runner, config, base, &latest_store, dependency_availability);
    let version = detected_version.or(latest_status.version.clone());
    let last_error = merge_status_error_details([
        latest_status.last_error.clone(),
        version_error.map(|error| status_error_detail("version_check", error)),
        command_error.map(|error| status_error_detail("command_resolution", error)),
        acp_probe_error
            .clone()
            .map(|error| status_error_detail("acp_probe", error)),
    ]);

    Ok(AgentRuntimeDiagnostics {
        runner_type: latest_status.runner_type,
        installed: latest_status.installed,
        executable: latest_status.executable,
        availability: latest_status.availability,
        auth_state: latest_status.auth_state,
        node_available: latest_status.node_available,
        npm_available: latest_status.npm_available,
        npx_available: latest_status.npx_available,
        config_path: runtime_config_path.unwrap_or_else(|| path.display().to_string()),
        install_indicator_path,
        resolved_command,
        command_source,
        acp_probe,
        acp_probe_error,
        discovered_models: latest_status.discovered_models,
        model_source: latest_status.model_source,
        version,
        last_checked_at: latest_status.last_checked_at,
        last_error,
        run_mode: latest_status.run_mode,
        env_summary: latest_status.env_summary,
        executor_options: latest_status.executor_options,
        pi_models_sync,
    })
}

#[derive(Debug, Clone)]
struct ResolvedRuntimeCommand {
    executable_path: String,
    rendered: String,
}

async fn resolve_runtime_command_for_diagnostics(
    installed: bool,
    executor: &CodingAgent,
) -> Result<Option<ResolvedRuntimeCommand>, String> {
    if !installed {
        return Ok(None);
    }
    resolve_runtime_command(executor).await
}

async fn resolve_runtime_command(
    executor: &CodingAgent,
) -> Result<Option<ResolvedRuntimeCommand>, String> {
    let parts = match executor
        .runtime_command_for_diagnostics()
        .map_err(|error| {
            command_failure_detail(
                "<configured command could not be built>",
                "build runtime command",
                error,
            )
        })? {
        Some(parts) => parts,
        None => {
            let Some(base) = runtime_command_base(executor) else {
                return Ok(None);
            };
            CommandBuilder::new(base).build_initial().map_err(|error| {
                command_failure_detail(
                    "<configured command could not be parsed>",
                    "parse runtime command",
                    error,
                )
            })?
        }
    };
    let unresolved_command = parts.redacted_display();
    let (executable, args) = parts.into_resolved().await.map_err(|error| {
        command_failure_detail(&unresolved_command, "resolve runtime executable", error)
    })?;
    let executable_path = executable.display().to_string();
    let rendered = redacted_command(&executable_path, &args);
    Ok(Some(ResolvedRuntimeCommand {
        executable_path,
        rendered,
    }))
}

fn runtime_command_base(executor: &CodingAgent) -> Option<String> {
    if let Some(base_override) = cmd_overrides_for_executor(executor)
        .and_then(|cmd| cmd.base_command_override.as_deref())
        .map(str::trim)
        .filter(|base| !base.is_empty())
    {
        return Some(base_override.to_string());
    }
    match executor {
        CodingAgent::Pi(_) => Some(Pi::default_command()),
        _ => version_command_base(executor),
    }
}

pub fn apply_agent_runtime_config(
    runner: BaseCodingAgent,
    executor: &mut CodingAgent,
    env: &mut ExecutionEnv,
) -> Result<(), AgentRuntimeError> {
    let store = read_store(&store_path())?;
    apply_config_to_executor_and_env(runner, executor, env, &store)?;
    Ok(())
}

fn apply_config_to_executor_and_env(
    runner: BaseCodingAgent,
    executor: &mut CodingAgent,
    env: &mut ExecutionEnv,
    store: &AgentRuntimeStore,
) -> Result<(), AgentRuntimeError> {
    if let Some(config) = store.configs.get(&runner) {
        merge_agent_env_without_overwriting_session(env, &config.env_json);
        apply_executor_options(runner, executor, &config.executor_options)?;
    }
    Ok(())
}

fn apply_executor_options(
    runner: BaseCodingAgent,
    executor: &mut CodingAgent,
    executor_options: &Value,
) -> Result<(), AgentRuntimeError> {
    let Some(options) = executor_options
        .as_object()
        .filter(|options| !options.is_empty())
    else {
        return Ok(());
    };

    let tag = serde_json::to_value(runner)?
        .as_str()
        .unwrap_or_default()
        .to_string();
    let mut wrapped = serde_json::to_value(&*executor)?;
    let Value::Object(root) = &mut wrapped else {
        return Ok(());
    };
    let Some(inner) = root.get_mut(&tag) else {
        return Ok(());
    };

    merge_json_object(inner, &Value::Object(options.clone()));
    *executor = serde_json::from_value(wrapped)?;
    Ok(())
}

fn merge_json_object(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(target_map), Value::Object(source_map)) => {
            for (key, value) in source_map {
                match (target_map.get_mut(key), value) {
                    (Some(existing @ Value::Object(_)), Value::Object(_)) => {
                        merge_json_object(existing, value);
                    }
                    _ => {
                        target_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (target, source) => {
            *target = source.clone();
        }
    }
}

fn merge_agent_env_without_overwriting_session(
    env: &mut ExecutionEnv,
    agent_env: &HashMap<String, String>,
) {
    for (key, value) in agent_env {
        if !env.contains_key(key) {
            env.insert(key.clone(), value.clone());
        }
    }
}

async fn detect_cli_version(
    executor: &CodingAgent,
    env: &ExecutionEnv,
) -> Result<Option<String>, String> {
    let parts = match executor
        .version_command_for_diagnostics()
        .map_err(|error| {
            command_failure_detail(
                "<configured command could not be built>",
                "build version command",
                error,
            )
        })? {
        Some(parts) => parts,
        None => {
            let Some(base) = version_command_base(executor) else {
                return Ok(None);
            };
            CommandBuilder::new(base)
                .extend_params(["--version"])
                .build_initial()
                .map_err(|error| {
                    command_failure_detail(
                        "<configured command could not be parsed>",
                        "parse version command",
                        error,
                    )
                })?
        }
    };
    let unresolved_command = parts.redacted_display();
    let parts = parts.into_resolved().await.map_err(|error| {
        command_failure_detail(&unresolved_command, "resolve version executable", error)
    })?;
    let (executable_path, args) = parts;
    let command_display = redacted_command(&executable_path.display().to_string(), &args);

    let mut command = Command::new(executable_path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let effective_env = if let Some(cmd_overrides) = cmd_overrides_for_executor(executor) {
        env.clone().with_profile(cmd_overrides)
    } else {
        env.clone()
    };
    let output_redactor = effective_env.sensitive_value_redactor();
    effective_env.apply_to_command(&mut command);

    let output = timeout(Duration::from_secs(12), command.output())
        .await
        .map_err(|_| {
            command_failure_detail(
                &command_display,
                "execute version command",
                "timed out after 12 seconds",
            )
        })?
        .map_err(|error| {
            command_failure_detail(&command_display, "execute version command", error)
        })?;
    if !output.status.success() {
        let status = output
            .status
            .code()
            .map(|code| format!("exit code {code}"))
            .unwrap_or_else(|| "terminated by signal".to_string());
        let evidence = normalize_cli_version_output(&output.stderr, &output.stdout)
            .map(|line| format!(": {}", output_redactor.redact(&line)))
            .unwrap_or_default();
        return Err(command_failure_detail(
            &command_display,
            "execute version command",
            format!("process failed with {status}{evidence}"),
        ));
    }

    normalize_cli_version_output(&output.stdout, &output.stderr)
        .map(|line| output_redactor.redact(&line))
        .map(Some)
        .ok_or_else(|| {
            command_failure_detail(
                &command_display,
                "parse version output",
                "process exited successfully but produced no version output",
            )
        })
}

fn command_failure_detail(
    command: &str,
    operation: &str,
    result: impl std::fmt::Display,
) -> String {
    format!(
        "command=`{command}`; operation={operation}; result={}",
        result.to_string().trim()
    )
}

fn version_command_base(executor: &CodingAgent) -> Option<String> {
    if let Some(base_override) = cmd_overrides_for_executor(executor)
        .and_then(|cmd| cmd.base_command_override.as_deref())
        .map(str::trim)
        .filter(|base| !base.is_empty())
    {
        return Some(base_override.to_string());
    }

    Some(match executor {
        CodingAgent::ClaudeCode(config) => {
            if config.claude_code_router.unwrap_or(false) {
                "npx -y @musistudio/claude-code-router@2.0.0".to_string()
            } else {
                "npx -y @anthropic-ai/claude-code@2.1.161".to_string()
            }
        }
        CodingAgent::Amp(_) => "amp".to_string(),
        CodingAgent::Gemini(_) => "gemini".to_string(),
        CodingAgent::Codex(_) => Codex::base_command().to_string(),
        CodingAgent::Opencode(_) => {
            format!("npx -y opencode-ai@{}", Opencode::PACKAGE_VERSION)
        }
        CodingAgent::OpenTeamsCli(_) => openteams_cli_binary_base(),
        CodingAgent::CursorAgent(_) => "cursor-agent".to_string(),
        CodingAgent::QwenCode(_) => "qwen".to_string(),
        CodingAgent::Copilot(_) => "copilot".to_string(),
        CodingAgent::Droid(_) => "droid".to_string(),
        CodingAgent::KimiCode(_) => "kimi".to_string(),
        CodingAgent::QoderCli(_) => "qodercli".to_string(),
        CodingAgent::Hermes(_) => "hermes".to_string(),
        CodingAgent::KiroCli(_) | CodingAgent::DeepseekHarness(_) => return None,
        CodingAgent::Pi(_) => Pi::version_command(),
        #[cfg(feature = "qa-mode")]
        CodingAgent::QaMock(_) => return None,
        #[cfg(feature = "qa-mode")]
        CodingAgent::AcpQa(config) => config.command.clone(),
    })
}

fn cmd_overrides_for_executor(executor: &CodingAgent) -> Option<&CmdOverrides> {
    match executor {
        CodingAgent::ClaudeCode(config) => Some(&config.cmd),
        CodingAgent::Amp(config) => Some(&config.cmd),
        CodingAgent::Gemini(config) => Some(&config.cmd),
        CodingAgent::Codex(config) => Some(&config.cmd),
        CodingAgent::Opencode(config) => Some(&config.cmd),
        CodingAgent::OpenTeamsCli(config) => Some(&config.cmd),
        CodingAgent::CursorAgent(config) => Some(&config.cmd),
        CodingAgent::QwenCode(config) => Some(&config.cmd),
        CodingAgent::Copilot(config) => Some(&config.cmd),
        CodingAgent::Droid(config) => Some(&config.cmd),
        CodingAgent::KimiCode(config) => Some(&config.cmd),
        CodingAgent::QoderCli(config) => Some(&config.cmd),
        CodingAgent::Pi(config) => Some(&config.cmd),
        CodingAgent::Hermes(config) => Some(&config.cmd),
        CodingAgent::KiroCli(config) => Some(&config.cmd),
        CodingAgent::DeepseekHarness(config) => Some(&config.cmd),
        #[cfg(feature = "qa-mode")]
        CodingAgent::QaMock(_) => None,
        #[cfg(feature = "qa-mode")]
        CodingAgent::AcpQa(config) => Some(&config.cmd),
    }
}

fn openteams_cli_binary_base() -> String {
    let binary_name = if cfg!(windows) {
        "openteams-cli.exe"
    } else {
        "openteams-cli"
    };

    if let Ok(path) = std::env::var("OPENTEAMS_CLI_PATH") {
        let path = PathBuf::from(path);
        if path.exists() {
            return command_base_from_path(path);
        }
    }

    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        let bundled = exe_dir.join(binary_name);
        if bundled.exists() {
            return command_base_from_path(bundled);
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        let dev_binary = cwd.join("binaries").join(binary_name);
        if dev_binary.exists() {
            return command_base_from_path(dev_binary);
        }
    }

    if let Some(home) = dirs::home_dir() {
        let bundled = home.join(".openteams").join("bin").join(binary_name);
        if bundled.exists() {
            return command_base_from_path(bundled);
        }
    }

    which::which("openteams-cli")
        .ok()
        .map(command_base_from_path)
        .unwrap_or_else(|| "openteams-cli".to_string())
}

fn command_base_from_path(path: PathBuf) -> String {
    let raw = path.to_string_lossy();
    if raw.contains(' ') {
        format!("\"{raw}\"")
    } else {
        raw.to_string()
    }
}

fn normalize_cli_version_output(stdout: &[u8], stderr: &[u8]) -> Option<String> {
    let stdout = String::from_utf8_lossy(stdout);
    first_version_line(&stdout).or_else(|| {
        let stderr = String::from_utf8_lossy(stderr);
        first_version_line(&stderr)
    })
}

fn first_version_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(160).collect())
}

fn cache_runner_version(store: &mut AgentRuntimeStore, runner: BaseCodingAgent, version: String) {
    let now = Utc::now();
    store
        .discoveries
        .entry(runner)
        .and_modify(|entry| {
            entry.version = Some(version.clone());
            entry.last_checked_at = now;
            entry.last_error =
                remove_status_error_stage(entry.last_error.as_deref(), "version_check");
        })
        .or_insert_with(|| AgentRuntimeDiscovery {
            models: Vec::new(),
            version: Some(version),
            auth_state: None,
            last_checked_at: now,
            last_error: None,
        });
}

/// Persists the model list discovered through an ACP capability probe so the
/// next lightweight `list_runtime_statuses` call can populate the model
/// dropdown immediately instead of waiting for another probe.
fn cache_runner_acp_models(
    store: &mut AgentRuntimeStore,
    runner: BaseCodingAgent,
    models: Vec<String>,
) {
    if models.is_empty() {
        return;
    }
    let now = Utc::now();
    store
        .discoveries
        .entry(runner)
        .and_modify(|entry| {
            entry.models = models.clone();
            entry.last_checked_at = now;
            entry.last_error =
                remove_status_error_stage(entry.last_error.as_deref(), "model_discovery");
        })
        .or_insert_with(|| AgentRuntimeDiscovery {
            models,
            version: None,
            auth_state: None,
            last_checked_at: now,
            last_error: None,
        });
}

fn cache_runner_auth_state(
    store: &mut AgentRuntimeStore,
    runner: BaseCodingAgent,
    auth_state: AgentRuntimeAuthState,
) {
    let now = Utc::now();
    store
        .discoveries
        .entry(runner)
        .and_modify(|entry| {
            entry.auth_state = Some(auth_state);
            entry.last_checked_at = now;
            entry.last_error = remove_status_error_stage(
                remove_status_error_stage(entry.last_error.as_deref(), "model_discovery")
                    .as_deref(),
                "acp_probe",
            );
        })
        .or_insert_with(|| AgentRuntimeDiscovery {
            models: Vec::new(),
            version: None,
            auth_state: Some(auth_state),
            last_checked_at: now,
            last_error: None,
        });
}

fn version_for_discovery_update(
    store: &AgentRuntimeStore,
    runner: BaseCodingAgent,
    detected_version: Option<String>,
) -> Option<String> {
    detected_version.or_else(|| {
        store
            .discoveries
            .get(&runner)
            .and_then(|entry| entry.version.clone())
    })
}

fn cli_probe_request_key(
    runner: BaseCodingAgent,
    executor: &CodingAgent,
    current_dir: &Path,
    env: &ExecutionEnv,
    auth_method_id: Option<&str>,
) -> Result<CliProbeRequestKey, String> {
    let mut hasher = Sha256::new();
    let executor_json = serde_json::to_vec(executor).map_err(|error| error.to_string())?;
    hasher.update((executor_json.len() as u64).to_le_bytes());
    hasher.update(executor_json);

    let mut env_vars = env.vars.iter().collect::<Vec<_>>();
    env_vars.sort_unstable_by(|left, right| left.0.cmp(right.0));
    for (key, value) in env_vars {
        hasher.update((key.len() as u64).to_le_bytes());
        hasher.update(key.as_bytes());
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    if let Some(auth_method_id) = auth_method_id {
        hasher.update((auth_method_id.len() as u64).to_le_bytes());
        hasher.update(auth_method_id.as_bytes());
    }

    Ok(CliProbeRequestKey {
        runner,
        current_dir: current_dir.to_path_buf(),
        execution_fingerprint: hasher.finalize().into(),
    })
}

fn reusable_cached_cli_probe(
    key: &CliProbeCacheKey,
    policy: CliProbeCachePolicy,
    requested_at: Instant,
) -> Option<Result<CliProbeValue, String>> {
    let cached = CLI_PROBE_CACHE.get(key)?;
    let completed_during_request = cached.completed_at >= requested_at;
    let reusable_success = policy == CliProbeCachePolicy::Reuse
        && cached.completed_at.elapsed() <= CLI_PROBE_CACHE_TTL
        && cached.result.is_ok();
    (completed_during_request || reusable_success).then(|| cached.result.clone())
}

async fn run_coordinated_cli_probe<F, Fut>(
    key: CliProbeCacheKey,
    policy: CliProbeCachePolicy,
    probe: F,
) -> Result<CliProbeValue, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<CliProbeValue, String>>,
{
    let requested_at = Instant::now();
    if let Some(cached) = reusable_cached_cli_probe(&key, policy, requested_at) {
        return cached;
    }

    let gate = CLI_PROBE_GATES
        .entry(key.clone())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let _guard = gate.lock().await;

    if let Some(cached) = reusable_cached_cli_probe(&key, policy, requested_at) {
        return cached;
    }

    // CLI implementations commonly share user-level auth/config files. The
    // runner gate prevents unrelated protocol probes from racing inside one CLI
    // while the request gate above still provides exact-key singleflight.
    let runner_gate = CLI_RUNNER_GATES
        .entry(key.request.runner)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let _runner_guard = runner_gate.lock().await;
    let result = probe().await;
    CLI_PROBE_CACHE.retain(|_, cached| cached.completed_at.elapsed() <= CLI_PROBE_CACHE_TTL);
    CLI_PROBE_CACHE.insert(
        key,
        CachedCliProbe {
            completed_at: Instant::now(),
            result: result.clone(),
        },
    );
    result
}

async fn coordinated_probe_acp(
    runner: BaseCodingAgent,
    executor: &CodingAgent,
    current_dir: &Path,
    env: &ExecutionEnv,
    auth_method_id: Option<&str>,
    policy: CliProbeCachePolicy,
) -> Result<AcpProbeOutcome, String> {
    let key = CliProbeCacheKey {
        request: cli_probe_request_key(runner, executor, current_dir, env, auth_method_id)?,
        kind: CliProbeKind::Acp,
    };
    match run_coordinated_cli_probe(key, policy, || async {
        match executor.probe_acp(current_dir, env, auth_method_id).await {
            Ok(probe) => Ok(CliProbeValue::Acp(AcpProbeOutcome::Probed(probe))),
            Err(ExecutorError::AuthRequired(_)) => {
                Ok(CliProbeValue::Acp(AcpProbeOutcome::Unauthenticated))
            }
            Err(error) => Err(error.to_string()),
        }
    })
    .await
    {
        Ok(CliProbeValue::Acp(outcome)) => Ok(outcome),
        Ok(_) => Err("CLI probe coordinator returned an unexpected ACP result".to_string()),
        Err(error) => Err(error),
    }
}

async fn coordinated_detect_cli_version(
    runner: BaseCodingAgent,
    executor: &CodingAgent,
    current_dir: &Path,
    env: &ExecutionEnv,
    auth_method_id: Option<&str>,
    policy: CliProbeCachePolicy,
) -> Result<Option<String>, String> {
    let key = CliProbeCacheKey {
        request: cli_probe_request_key(runner, executor, current_dir, env, auth_method_id)?,
        kind: CliProbeKind::Version,
    };
    match run_coordinated_cli_probe(key, policy, || async {
        detect_cli_version(executor, env)
            .await
            .map(CliProbeValue::Version)
    })
    .await
    {
        Ok(CliProbeValue::Version(version)) => Ok(version),
        Ok(_) => Err("CLI probe coordinator returned an unexpected version result".to_string()),
        Err(error) => Err(error),
    }
}

async fn coordinated_list_models(
    runner: BaseCodingAgent,
    executor: &CodingAgent,
    current_dir: &Path,
    env: &ExecutionEnv,
    auth_method_id: Option<&str>,
    policy: CliProbeCachePolicy,
) -> Result<Option<Vec<String>>, String> {
    let key = CliProbeCacheKey {
        request: cli_probe_request_key(runner, executor, current_dir, env, auth_method_id)?,
        kind: CliProbeKind::Models,
    };
    match run_coordinated_cli_probe(key, policy, || async {
        executor
            .list_models(current_dir, env)
            .await
            .map(CliProbeValue::Models)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(CliProbeValue::Models(models)) => Ok(models),
        Ok(_) => Err("CLI probe coordinator returned an unexpected model result".to_string()),
        Err(error) => Err(error),
    }
}

async fn coordinated_resolve_runtime_command(
    runner: BaseCodingAgent,
    executor: &CodingAgent,
    current_dir: &Path,
    env: &ExecutionEnv,
    auth_method_id: Option<&str>,
    installed: bool,
    policy: CliProbeCachePolicy,
) -> Result<Option<ResolvedRuntimeCommand>, String> {
    let key = CliProbeCacheKey {
        request: cli_probe_request_key(runner, executor, current_dir, env, auth_method_id)?,
        kind: CliProbeKind::Command,
    };
    match run_coordinated_cli_probe(key, policy, || async {
        resolve_runtime_command_for_diagnostics(installed, executor)
            .await
            .map(CliProbeValue::Command)
    })
    .await
    {
        Ok(CliProbeValue::Command(command)) => Ok(command),
        Ok(_) => Err("CLI probe coordinator returned an unexpected command result".to_string()),
        Err(error) => Err(error),
    }
}

async fn discover_models_for_executor(
    runner: BaseCodingAgent,
    executor: &CodingAgent,
    current_dir: &Path,
    env: &ExecutionEnv,
) -> Result<(Option<Vec<String>>, Option<AgentRuntimeAuthState>), String> {
    let acp_result = coordinated_probe_acp(
        runner,
        executor,
        current_dir,
        env,
        None,
        CliProbeCachePolicy::Refresh,
    )
    .await;
    let mut auth_state = None;
    let mut model_fallback = executor.acp_model_fallback();
    let acp_error = match &acp_result {
        Ok(AcpProbeOutcome::Probed(Some(probe))) => {
            let interpretation = executor.interpret_acp_probe(probe);
            auth_state = interpretation.auth_state.map(agent_runtime_auth_state);
            model_fallback = interpretation.model_fallback;
            if let Some(models) = interpretation.models {
                return Ok((Some(models), auth_state));
            }
            None
        }
        Ok(AcpProbeOutcome::Probed(None)) => None,
        Ok(AcpProbeOutcome::Unauthenticated) => {
            return Ok((None, Some(AgentRuntimeAuthState::Unauthenticated)));
        }
        Err(error) => Some(error.clone()),
    };

    if model_fallback == AcpModelFallback::Disabled {
        return match acp_error {
            Some(error) => Err(format!("ACP initialize failed: {error}")),
            None => Ok((None, auth_state)),
        };
    }

    match coordinated_list_models(
        runner,
        executor,
        current_dir,
        env,
        None,
        CliProbeCachePolicy::Refresh,
    )
    .await
    {
        Ok(Some(models)) => Ok((Some(models), auth_state)),
        Ok(None) => match acp_error {
            Some(error) => Err(error),
            None => Ok((None, auth_state)),
        },
        Err(model_error) => Err(match acp_error {
            None => model_error,
            Some(acp_error) => {
                format!("ACP probe failed: {acp_error}; model listing failed: {model_error}")
            }
        }),
    }
}

fn build_statuses(
    profiles: &ExecutorConfigs,
    store: &AgentRuntimeStore,
) -> Vec<AgentRuntimeStatus> {
    let dependency_availability = detect_runtime_dependency_availability();
    let mut runners = profiles
        .executors
        .iter()
        .filter_map(|(runner, config)| {
            let base = config
                .get_default()
                .or_else(|| config.configurations.values().next())?;
            Some(build_status(
                *runner,
                config,
                base,
                store,
                dependency_availability,
            ))
        })
        .collect::<Vec<_>>();
    runners.sort_by_key(|status| status.runner_type.to_string());
    runners
}

fn build_status(
    runner: BaseCodingAgent,
    executor_config: &ExecutorConfig,
    base: &CodingAgent,
    store: &AgentRuntimeStore,
    dependency_availability: RuntimeDependencyAvailability,
) -> AgentRuntimeStatus {
    let config = store
        .configs
        .get(&runner)
        .cloned()
        .unwrap_or_else(|| default_config(runner));
    let discovery = store.discoveries.get(&runner);
    let mut configured_base = base.clone();
    let configuration_error = if let Err(error) =
        apply_executor_options(runner, &mut configured_base, &config.executor_options)
    {
        tracing::warn!(
            runner = %runner,
            error = %error,
            "failed to apply runtime config while checking availability"
        );
        Some(status_error_detail("runtime_configuration", error))
    } else {
        None
    };
    let availability = if runner == BaseCodingAgent::Pi {
        if dependency_availability.node {
            AvailabilityInfo::InstallationFound
        } else {
            AvailabilityInfo::NotFound
        }
    } else {
        configured_base.get_availability_info()
    };
    let installed = availability.is_available();
    let executable = installed
        && config.run_mode != AgentRunMode::Disabled
        && runtime_dependencies_available(runner, dependency_availability);
    let mut auth_env = ExecutionEnv::new(Default::default(), false, String::new());
    auth_env.merge(&config.env_json);
    let auth_state = if let Some(auth_state) = discovery.and_then(|entry| entry.auth_state) {
        auth_state
    } else if configured_base.is_authenticated(&auth_env) {
        AgentRuntimeAuthState::Authenticated
    } else {
        AgentRuntimeAuthState::Unauthenticated
    };

    AgentRuntimeStatus {
        runner_type: runner,
        installed,
        executable,
        availability,
        auth_state,
        node_available: dependency_availability.node,
        npm_available: dependency_availability.npm,
        npx_available: dependency_availability.npx,
        discovered_models: models_for_runner(runner, executor_config, store),
        model_source: model_source_for_runner(runner, executor_config, store),
        version: discovery.and_then(|entry| entry.version.clone()),
        last_checked_at: discovery.map(|entry| entry.last_checked_at),
        last_error: merge_status_error_details([
            discovery.and_then(|entry| entry.last_error.clone()),
            configuration_error,
        ]),
        run_mode: config.run_mode,
        env_summary: summarize_env(&config.env_json),
        executor_options: config.executor_options,
    }
}

fn agent_runtime_auth_state(state: AcpProbeAuthState) -> AgentRuntimeAuthState {
    match state {
        AcpProbeAuthState::Authenticated => AgentRuntimeAuthState::Authenticated,
        AcpProbeAuthState::Unauthenticated => AgentRuntimeAuthState::Unauthenticated,
    }
}

fn status_error_detail(stage: &str, error: impl std::fmt::Display) -> String {
    format!("[{stage}] {}", error.to_string().trim())
}

fn merge_status_error_details(errors: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let mut details = Vec::new();
    for error in errors.into_iter().flatten() {
        let error = error.trim();
        if !error.is_empty() && !details.iter().any(|existing| existing == error) {
            details.push(error.to_string());
        }
    }
    (!details.is_empty()).then(|| details.join("\n"))
}

fn remove_status_error_stage(error: Option<&str>, stage: &str) -> Option<String> {
    let prefix = format!("[{stage}]");
    merge_status_error_details(
        error
            .into_iter()
            .flat_map(str::lines)
            .map(|line| (!line.trim_start().starts_with(&prefix)).then(|| line.to_string())),
    )
}

fn clear_cached_authentication_required_error(
    store: &mut AgentRuntimeStore,
    runner: BaseCodingAgent,
) {
    let Some(discovery) = store.discoveries.get_mut(&runner) else {
        return;
    };
    discovery.last_error = merge_status_error_details(
        discovery
            .last_error
            .as_deref()
            .into_iter()
            .flat_map(str::lines)
            .map(|line| {
                let normalized = line.to_ascii_lowercase();
                (!normalized.contains("authentication required")
                    && !normalized.contains("auth required"))
                .then(|| line.to_string())
            }),
    );
}

fn split_probe_result<T>(result: Result<Option<T>, String>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (value, None),
        Err(error) => (None, Some(error)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeDependencyAvailability {
    node: bool,
    npm: bool,
    npx: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeDependencyRequirement {
    None,
    NodeAndNpm,
    NodeNpmAndNpx,
}

/// Resolve runtime prerequisites through the refreshed login-shell PATH so
/// commands installed from a regular terminal are found without an app restart.
fn detect_runtime_dependency_availability() -> RuntimeDependencyAvailability {
    RuntimeDependencyAvailability {
        node: utils::shell::resolve_executable_path_blocking("node").is_some(),
        npm: utils::shell::resolve_executable_path_blocking("npm").is_some(),
        npx: utils::shell::resolve_executable_path_blocking("npx").is_some(),
    }
}

fn runtime_dependency_requirement(runner: BaseCodingAgent) -> RuntimeDependencyRequirement {
    match runner {
        BaseCodingAgent::ClaudeCode
        | BaseCodingAgent::Codex
        | BaseCodingAgent::Opencode
        | BaseCodingAgent::Pi => RuntimeDependencyRequirement::NodeNpmAndNpx,
        BaseCodingAgent::Amp
        | BaseCodingAgent::Copilot
        | BaseCodingAgent::Gemini
        | BaseCodingAgent::QwenCode => RuntimeDependencyRequirement::NodeAndNpm,
        _ => RuntimeDependencyRequirement::None,
    }
}

fn runtime_dependencies_available(
    runner: BaseCodingAgent,
    availability: RuntimeDependencyAvailability,
) -> bool {
    match runtime_dependency_requirement(runner) {
        RuntimeDependencyRequirement::None => true,
        RuntimeDependencyRequirement::NodeAndNpm => availability.node && availability.npm,
        RuntimeDependencyRequirement::NodeNpmAndNpx => {
            availability.node && availability.npm && availability.npx
        }
    }
}

fn reasoning_capability_for_runner(
    runner: BaseCodingAgent,
) -> Option<AgentRuntimeReasoningCapability> {
    match runner {
        BaseCodingAgent::ClaudeCode => Some(AgentRuntimeReasoningCapability::Effort {
            options: strings(["low", "medium", "high"]),
        }),
        BaseCodingAgent::Codex => Some(AgentRuntimeReasoningCapability::Effort {
            options: strings(["low", "medium", "high", "xhigh", "max", "ultra"]),
        }),
        BaseCodingAgent::Droid => Some(AgentRuntimeReasoningCapability::Effort {
            options: strings(["none", "dynamic", "off", "low", "medium", "high"]),
        }),
        BaseCodingAgent::Gemini => Some(AgentRuntimeReasoningCapability::Effort {
            options: strings(["low", "medium", "high"]),
        }),
        BaseCodingAgent::Opencode | BaseCodingAgent::OpenTeamsCli => {
            Some(AgentRuntimeReasoningCapability::Effort {
                options: strings(["thinking-low", "thinking-medium", "thinking-high"]),
            })
        }
        BaseCodingAgent::QwenCode => Some(AgentRuntimeReasoningCapability::Effort {
            options: strings(["low", "medium", "high", "xhigh", "max"]),
        }),
        BaseCodingAgent::KimiCode => Some(AgentRuntimeReasoningCapability::Effort {
            options: strings(["low", "high", "max"]),
        }),
        BaseCodingAgent::QoderCli => None,
        BaseCodingAgent::Amp
        | BaseCodingAgent::CursorAgent
        | BaseCodingAgent::Copilot
        | BaseCodingAgent::Pi
        | BaseCodingAgent::Hermes
        | BaseCodingAgent::KiroCli
        | BaseCodingAgent::DeepseekHarness => None,
        #[cfg(feature = "qa-mode")]
        BaseCodingAgent::QaMock | BaseCodingAgent::AcpQa => None,
    }
}

pub fn reasoning_capability_for_runner_type(
    runner: BaseCodingAgent,
) -> Option<AgentRuntimeReasoningCapability> {
    reasoning_capability_for_runner(runner)
}

fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
    values.into_iter().map(String::from).collect()
}

fn models_for_runner(
    runner: BaseCodingAgent,
    executor_config: &ExecutorConfig,
    store: &AgentRuntimeStore,
) -> Vec<String> {
    if let Some(discovery) = store.discoveries.get(&runner)
        && !discovery.models.is_empty()
    {
        return discovery.models.clone();
    }

    configured_models(executor_config)
}

fn reconcile_cached_runtime_models(
    store: &mut AgentRuntimeStore,
    runner: BaseCodingAgent,
    previous_profile_models: &[String],
    current_profile_models: &[String],
) -> bool {
    let Some(discovery) = store.discoveries.get_mut(&runner) else {
        return false;
    };
    let previous_profile_models = previous_profile_models
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut next_models = discovery
        .models
        .iter()
        .filter(|model| !previous_profile_models.contains(model.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>();
    next_models.extend(current_profile_models.iter().cloned());
    let next_models = next_models.into_iter().collect::<Vec<_>>();

    if discovery.models == next_models {
        return false;
    }
    discovery.models = next_models;
    true
}

fn model_source_for_runner(
    runner: BaseCodingAgent,
    executor_config: &ExecutorConfig,
    store: &AgentRuntimeStore,
) -> AgentRuntimeModelSource {
    if let Some(discovery) = store.discoveries.get(&runner)
        && !discovery.models.is_empty()
    {
        return AgentRuntimeModelSource::Runner;
    }

    if configured_models(executor_config).is_empty() {
        AgentRuntimeModelSource::None
    } else {
        AgentRuntimeModelSource::ProfileFallback
    }
}

fn configured_models(executor_config: &ExecutorConfig) -> Vec<String> {
    let mut models = BTreeSet::new();
    for config in executor_config.configurations.values() {
        if let Some(model) = model_name(config) {
            models.insert(model.to_string());
        }
    }
    models.into_iter().collect()
}

fn model_name(config: &CodingAgent) -> Option<&str> {
    match config {
        CodingAgent::Codex(config) => config.model.as_deref(),
        CodingAgent::ClaudeCode(config) => config.model.as_deref(),
        CodingAgent::Gemini(config) => config.model.as_deref(),
        CodingAgent::Opencode(config) => config.model.as_deref(),
        CodingAgent::OpenTeamsCli(config) => config.model.as_deref(),
        CodingAgent::QwenCode(config) => config.model.as_deref(),
        CodingAgent::CursorAgent(config) => config.model.as_deref(),
        CodingAgent::Copilot(config) => config.model.as_deref(),
        CodingAgent::Droid(config) => config.model.as_deref(),
        CodingAgent::KimiCode(config) => config.model.as_deref(),
        CodingAgent::QoderCli(config) => config.model.as_deref(),
        CodingAgent::Pi(config) => config.model.as_deref(),
        CodingAgent::Hermes(config) => config.model.as_deref(),
        CodingAgent::KiroCli(config) => config.model.as_deref(),
        CodingAgent::DeepseekHarness(config) => config.model.as_deref(),
        #[cfg(feature = "qa-mode")]
        CodingAgent::QaMock(_) | CodingAgent::AcpQa(_) => None,
        _ => None,
    }
}

fn default_config(runner: BaseCodingAgent) -> AgentRuntimeConfig {
    AgentRuntimeConfig {
        runner_type: runner,
        run_mode: AgentRunMode::Auto,
        env_json: HashMap::new(),
        executor_options: serde_json::json!({}),
        updated_at: Utc::now(),
    }
}

fn summarize_env(env: &HashMap<String, String>) -> Vec<AgentRuntimeEnvSummary> {
    let mut summaries = env
        .iter()
        .map(|(key, value)| AgentRuntimeEnvSummary {
            key: key.clone(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|a, b| a.key.cmp(&b.key));
    summaries
}

fn validate_env_json(env: &HashMap<String, String>) -> Result<(), AgentRuntimeError> {
    for key in env.keys() {
        validate_env_key(key)?;
    }
    Ok(())
}

fn validate_env_key(key: &str) -> Result<(), AgentRuntimeError> {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return Err(AgentRuntimeError::InvalidEnvKey(key.to_string()));
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return Err(AgentRuntimeError::InvalidEnvKey(key.to_string()));
    }
    if chars.any(|ch| !(ch == '_' || ch.is_ascii_alphanumeric())) {
        return Err(AgentRuntimeError::InvalidEnvKey(key.to_string()));
    }
    Ok(())
}

fn read_store_unlocked(path: &Path) -> Result<AgentRuntimeStore, AgentRuntimeError> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(serde_json::from_str(&content)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(AgentRuntimeStore::default()),
        Err(err) => Err(err.into()),
    }
}

fn write_store_unlocked(path: &Path, store: &AgentRuntimeStore) -> Result<(), AgentRuntimeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(store)?)?;
    Ok(())
}

fn read_store(path: &Path) -> Result<AgentRuntimeStore, AgentRuntimeError> {
    let _guard = RUNTIME_STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    read_store_unlocked(path)
}

#[cfg(test)]
fn write_store(path: &Path, store: &AgentRuntimeStore) -> Result<(), AgentRuntimeError> {
    let _guard = RUNTIME_STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    write_store_unlocked(path, store)
}

fn update_store<T>(
    path: &Path,
    update: impl FnOnce(&mut AgentRuntimeStore) -> Result<T, AgentRuntimeError>,
) -> Result<(AgentRuntimeStore, T), AgentRuntimeError> {
    let _guard = RUNTIME_STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = read_store_unlocked(path)?;
    let result = update(&mut store)?;
    write_store_unlocked(path, &store)?;
    Ok((store, result))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use executors::executors::{
        AppendPrompt,
        acp::{AcpConfigChoice, AcpConfigOptionKind, AcpConfigOptionSnapshot, AcpConfigSource},
        deepseek_harness::DeepseekHarness,
        kimi::KimiCode,
        kiro::KiroCli,
        pi::Pi,
        qoder::QoderCli,
    };

    use super::*;

    #[test]
    fn runtime_probe_dir_resolves_an_explicit_workspace() {
        let workspace = tempfile::tempdir().unwrap();

        let resolved = resolve_runtime_probe_dir(Some(workspace.path())).unwrap();

        assert_eq!(resolved, fs::canonicalize(workspace.path()).unwrap());
    }

    #[test]
    fn runtime_probe_dir_rejects_non_directories_and_missing_paths() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(matches!(
            resolve_runtime_probe_dir(Some(file.path())),
            Err(AgentRuntimeError::InvalidWorkspacePath(_))
        ));

        let missing = file.path().with_extension("missing");
        assert!(matches!(
            resolve_runtime_probe_dir(Some(&missing)),
            Err(AgentRuntimeError::InvalidWorkspacePath(_))
        ));
    }

    fn model_agent(model: Option<&str>) -> CodingAgent {
        CodingAgent::KimiCode(KimiCode {
            append_prompt: AppendPrompt::default(),
            model: model.map(str::to_string),
            thinking_effort: None,
            acp: None,
            cmd: Default::default(),
            acp_mcp_policy: Default::default(),
            approvals: None,
        })
    }

    fn pi_agent() -> CodingAgent {
        CodingAgent::Pi(Pi::default())
    }

    fn dependencies(node: bool, npm: bool, npx: bool) -> RuntimeDependencyAvailability {
        RuntimeDependencyAvailability { node, npm, npx }
    }

    #[test]
    fn runtime_config_paths_are_separate_from_mcp_paths_where_required() {
        let profiles = ExecutorConfigs::from_defaults();
        let separate_paths = [
            (BaseCodingAgent::Copilot, "settings.json", "mcp-config.json"),
            (BaseCodingAgent::CursorAgent, "cli-config.json", "mcp.json"),
            (BaseCodingAgent::KimiCode, "config.toml", "mcp.json"),
            (BaseCodingAgent::Pi, "settings.json", "mcp.json"),
            (BaseCodingAgent::Droid, "settings.json", "mcp.json"),
        ];
        let runtime_only_paths = [
            (BaseCodingAgent::KiroCli, "cli.json"),
            (BaseCodingAgent::DeepseekHarness, "cordis.yml"),
        ];

        for (runner, runtime_file_name, mcp_file_name) in separate_paths {
            let executor = profiles
                .executors
                .get(&runner)
                .and_then(|config| {
                    config
                        .get_default()
                        .or_else(|| config.configurations.values().next())
                })
                .unwrap_or_else(|| panic!("missing default executor for {runner}"));
            let runtime_path = executor
                .default_runtime_config_path()
                .unwrap_or_else(|| panic!("missing runtime config path for {runner}"));
            let mcp_path = executor
                .default_mcp_config_path()
                .unwrap_or_else(|| panic!("missing MCP config path for {runner}"));

            assert_eq!(
                runtime_path.file_name().and_then(|name| name.to_str()),
                Some(runtime_file_name),
                "{runner}"
            );
            assert_eq!(
                mcp_path.file_name().and_then(|name| name.to_str()),
                Some(mcp_file_name),
                "{runner}"
            );
            assert_ne!(runtime_path, mcp_path, "{runner}");
        }

        for (runner, runtime_file_name) in runtime_only_paths {
            let executor = profiles
                .executors
                .get(&runner)
                .and_then(|config| {
                    config
                        .get_default()
                        .or_else(|| config.configurations.values().next())
                })
                .unwrap_or_else(|| panic!("missing default executor for {runner}"));
            let runtime_path = executor
                .default_runtime_config_path()
                .unwrap_or_else(|| panic!("missing runtime config path for {runner}"));

            assert_eq!(
                runtime_path.file_name().and_then(|name| name.to_str()),
                Some(runtime_file_name),
                "{runner}"
            );
            assert_eq!(executor.default_mcp_config_path(), None, "{runner}");
        }

        for (runner, config) in &profiles.executors {
            if separate_paths
                .iter()
                .any(|(separate_runner, _, _)| separate_runner == runner)
                || runtime_only_paths
                    .iter()
                    .any(|(runtime_only_runner, _)| runtime_only_runner == runner)
            {
                continue;
            }
            let executor = config
                .get_default()
                .or_else(|| config.configurations.values().next())
                .unwrap_or_else(|| panic!("missing default executor for {runner}"));

            assert_eq!(
                executor.default_runtime_config_path(),
                executor.default_mcp_config_path(),
                "{runner} should retain its existing diagnostic config path"
            );
        }
    }

    #[test]
    fn pi_installation_status_tracks_node_without_absorbing_cli_dependencies() {
        let runner = BaseCodingAgent::Pi;
        let executor_config = ExecutorConfig::new_with_default(pi_agent());
        let base = executor_config.get_default().unwrap();
        let store = AgentRuntimeStore::default();

        for (node, expected) in [(false, false), (true, true)] {
            let status = build_status(
                runner,
                &executor_config,
                base,
                &store,
                dependencies(node, true, true),
            );
            assert_eq!(status.installed, expected);
            assert_eq!(status.executable, expected);
            assert_eq!(status.node_available, node);
        }

        let missing_npm = build_status(
            runner,
            &executor_config,
            base,
            &store,
            dependencies(true, false, true),
        );
        assert!(missing_npm.installed);
        assert!(!missing_npm.executable);
        assert!(!missing_npm.npm_available);
    }

    #[test]
    fn runtime_dependency_requirements_follow_default_executor_families() {
        for runner in [
            BaseCodingAgent::ClaudeCode,
            BaseCodingAgent::Codex,
            BaseCodingAgent::Opencode,
            BaseCodingAgent::Pi,
        ] {
            assert_eq!(
                runtime_dependency_requirement(runner),
                RuntimeDependencyRequirement::NodeNpmAndNpx,
                "{runner}"
            );
        }

        for runner in [
            BaseCodingAgent::Amp,
            BaseCodingAgent::Copilot,
            BaseCodingAgent::Gemini,
            BaseCodingAgent::QwenCode,
        ] {
            assert_eq!(
                runtime_dependency_requirement(runner),
                RuntimeDependencyRequirement::NodeAndNpm,
                "{runner}"
            );
        }

        for runner in [
            BaseCodingAgent::OpenTeamsCli,
            BaseCodingAgent::CursorAgent,
            BaseCodingAgent::Droid,
            BaseCodingAgent::KimiCode,
            BaseCodingAgent::QoderCli,
            BaseCodingAgent::Hermes,
            BaseCodingAgent::KiroCli,
            BaseCodingAgent::DeepseekHarness,
        ] {
            assert_eq!(
                runtime_dependency_requirement(runner),
                RuntimeDependencyRequirement::None,
                "{runner}"
            );
        }
    }

    #[test]
    fn missing_runtime_dependencies_block_only_their_executor_families() {
        let all = dependencies(true, true, true);
        let no_node = dependencies(false, true, true);
        let no_npm = dependencies(true, false, true);
        let no_npx = dependencies(true, true, false);

        assert!(runtime_dependencies_available(BaseCodingAgent::Codex, all));
        assert!(!runtime_dependencies_available(
            BaseCodingAgent::Codex,
            no_node
        ));
        assert!(!runtime_dependencies_available(
            BaseCodingAgent::Codex,
            no_npm
        ));
        assert!(!runtime_dependencies_available(
            BaseCodingAgent::Codex,
            no_npx
        ));

        assert!(runtime_dependencies_available(
            BaseCodingAgent::Gemini,
            no_npx
        ));
        assert!(!runtime_dependencies_available(
            BaseCodingAgent::Gemini,
            no_node
        ));
        assert!(!runtime_dependencies_available(
            BaseCodingAgent::Gemini,
            no_npm
        ));

        assert!(runtime_dependencies_available(
            BaseCodingAgent::KimiCode,
            dependencies(false, false, false)
        ));
    }

    #[test]
    fn runtime_responses_expose_structured_pi_sync_diagnostics() {
        let diagnostic = PiModelsSyncDiagnostic {
            synchronized: false,
            result: None,
            error: Some("Pi model coordination failed".to_string()),
            retry_available: true,
            retry_path: super::super::pi_models::PI_MODELS_SYNC_RETRY_PATH.to_string(),
        };
        let response = AgentRuntimeListResponse {
            runners: Vec::new(),
            pi_models_sync: Some(diagnostic),
        };
        let serialized = serde_json::to_value(response).expect("runtime response");

        assert_eq!(serialized["pi_models_sync"]["synchronized"], false);
        assert_eq!(serialized["pi_models_sync"]["retry_available"], true);
        assert_eq!(
            serialized["pi_models_sync"]["retry_path"],
            super::super::pi_models::PI_MODELS_SYNC_RETRY_PATH
        );
    }

    #[test]
    fn pi_acp_model_values_remain_exact() {
        let exact_models = vec![
            "openrouter/anthropic/claude-sonnet-4.5".to_string(),
            "custom-provider/model:id@revision".to_string(),
        ];
        let probe = AcpCapabilityProbe {
            protocol_version: "1".to_string(),
            agent_name: Some("pi-acp".to_string()),
            agent_version: Some("0.0.33".to_string()),
            auth_methods: Vec::new(),
            supports_session_list: true,
            supports_session_resume: false,
            supports_session_load: true,
            supports_session_close: false,
            supports_session_delete: true,
            supports_additional_directories: false,
            agent_capabilities: serde_json::json!({}),
            config_source: AcpConfigSource::Stable,
            config_options: vec![AcpConfigOptionSnapshot {
                id: "model".to_string(),
                name: "Model".to_string(),
                description: None,
                category: Some("model".to_string()),
                kind: AcpConfigOptionKind::Select {
                    current_value: exact_models[0].clone(),
                    options: exact_models
                        .iter()
                        .map(|value| AcpConfigChoice {
                            value: value.clone(),
                            name: value.clone(),
                            description: None,
                        })
                        .collect(),
                },
            }],
        };

        assert_eq!(probe.model_ids().unwrap(), exact_models);
    }

    #[test]
    fn pi_acp_probe_failure_preserves_installed_state_and_cached_models() {
        let runner = BaseCodingAgent::Pi;
        let executor_config = ExecutorConfig::new_with_default(pi_agent());
        let base = executor_config.get_default().unwrap();
        let preserved_models = vec!["provider/exact-model".to_string()];
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: preserved_models.clone(),
                version: Some("pi 0.83.0".to_string()),
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: None,
            },
        );

        let errors = apply_discovery_outcomes(
            &mut store,
            vec![RunnerDiscoveryOutcome::Failed {
                runner,
                message: "[model_discovery] ACP initialize failed: NPX process exited".to_string(),
                detected_version: None,
                preserved_models: preserved_models.clone(),
            }],
        );
        let status = build_status(
            runner,
            &executor_config,
            base,
            &store,
            dependencies(true, true, true),
        );

        assert!(status.installed);
        assert!(status.executable);
        assert_eq!(status.discovered_models, preserved_models);
        assert_eq!(errors[0].preserved_models, status.discovered_models);
        assert!(status.last_error.unwrap().contains("ACP initialize failed"));
    }

    fn test_probe_key(runner: BaseCodingAgent, kind: CliProbeKind, salt: u8) -> CliProbeCacheKey {
        let mut fingerprint = [0; 32];
        fingerprint[0] = salt;
        CliProbeCacheKey {
            request: CliProbeRequestKey {
                runner,
                current_dir: PathBuf::from(format!("/openteams-probe-test-{salt}")),
                execution_fingerprint: fingerprint,
            },
            kind,
        }
    }

    #[tokio::test]
    async fn concurrent_identical_acp_probes_share_one_execution() {
        let key = test_probe_key(BaseCodingAgent::Gemini, CliProbeKind::Acp, 201);
        CLI_PROBE_CACHE.remove(&key);
        let starts = Arc::new(AtomicUsize::new(0));

        let first_starts = Arc::clone(&starts);
        let first = run_coordinated_cli_probe(
            key.clone(),
            CliProbeCachePolicy::Reuse,
            move || async move {
                first_starts.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(40)).await;
                Ok(CliProbeValue::Acp(AcpProbeOutcome::Probed(None)))
            },
        );
        let second_starts = Arc::clone(&starts);
        let second = run_coordinated_cli_probe(
            key.clone(),
            CliProbeCachePolicy::Refresh,
            move || async move {
                second_starts.fetch_add(1, Ordering::SeqCst);
                Ok(CliProbeValue::Acp(AcpProbeOutcome::Probed(None)))
            },
        );

        let (first_result, second_result) = tokio::join!(first, second);

        assert!(matches!(
            first_result,
            Ok(CliProbeValue::Acp(AcpProbeOutcome::Probed(None)))
        ));
        assert!(matches!(
            second_result,
            Ok(CliProbeValue::Acp(AcpProbeOutcome::Probed(None)))
        ));
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        CLI_PROBE_CACHE.remove(&key);
    }

    #[tokio::test]
    async fn concurrent_identical_non_acp_probes_share_one_execution() {
        let key = test_probe_key(BaseCodingAgent::Codex, CliProbeKind::Version, 202);
        CLI_PROBE_CACHE.remove(&key);
        let starts = Arc::new(AtomicUsize::new(0));

        let make_probe = |starts: Arc<AtomicUsize>| async move {
            starts.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(40)).await;
            Ok(CliProbeValue::Version(Some("codex 1.2.3".to_string())))
        };
        let first = run_coordinated_cli_probe(key.clone(), CliProbeCachePolicy::Reuse, {
            let starts = Arc::clone(&starts);
            move || make_probe(starts)
        });
        let second = run_coordinated_cli_probe(key.clone(), CliProbeCachePolicy::Refresh, {
            let starts = Arc::clone(&starts);
            move || make_probe(starts)
        });

        let (first_result, second_result) = tokio::join!(first, second);

        assert!(matches!(first_result, Ok(CliProbeValue::Version(Some(_)))));
        assert!(matches!(second_result, Ok(CliProbeValue::Version(Some(_)))));
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        CLI_PROBE_CACHE.remove(&key);
    }

    #[tokio::test]
    async fn different_probe_keys_for_one_runner_are_serialized() {
        let first_key = test_probe_key(BaseCodingAgent::Codex, CliProbeKind::Version, 203);
        let second_key = test_probe_key(BaseCodingAgent::Codex, CliProbeKind::Models, 204);
        CLI_PROBE_CACHE.remove(&first_key);
        CLI_PROBE_CACHE.remove(&second_key);
        let active = Arc::new(AtomicUsize::new(0));
        let maximum_active = Arc::new(AtomicUsize::new(0));

        let make_probe = |active: Arc<AtomicUsize>, maximum_active: Arc<AtomicUsize>| async move {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum_active.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(40)).await;
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(CliProbeValue::Version(None))
        };
        let first = run_coordinated_cli_probe(first_key.clone(), CliProbeCachePolicy::Refresh, {
            let active = Arc::clone(&active);
            let maximum_active = Arc::clone(&maximum_active);
            move || make_probe(active, maximum_active)
        });
        let second = run_coordinated_cli_probe(second_key.clone(), CliProbeCachePolicy::Refresh, {
            let active = Arc::clone(&active);
            let maximum_active = Arc::clone(&maximum_active);
            move || make_probe(active, maximum_active)
        });

        let (first_result, second_result) = tokio::join!(first, second);

        assert!(first_result.is_ok());
        assert!(second_result.is_ok());
        assert_eq!(maximum_active.load(Ordering::SeqCst), 1);
        CLI_PROBE_CACHE.remove(&first_key);
        CLI_PROBE_CACHE.remove(&second_key);
    }

    #[tokio::test]
    async fn concurrent_probe_failure_is_shared_but_not_reused_later() {
        let key = test_probe_key(BaseCodingAgent::Opencode, CliProbeKind::Version, 205);
        CLI_PROBE_CACHE.remove(&key);
        let starts = Arc::new(AtomicUsize::new(0));

        let first_starts = Arc::clone(&starts);
        let first = run_coordinated_cli_probe(
            key.clone(),
            CliProbeCachePolicy::Reuse,
            move || async move {
                first_starts.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(40)).await;
                Err("temporary probe failure".to_string())
            },
        );
        let second_starts = Arc::clone(&starts);
        let second = run_coordinated_cli_probe(
            key.clone(),
            CliProbeCachePolicy::Reuse,
            move || async move {
                second_starts.fetch_add(1, Ordering::SeqCst);
                Ok(CliProbeValue::Version(None))
            },
        );
        let (first_result, second_result) = tokio::join!(first, second);
        assert_eq!(first_result.unwrap_err(), "temporary probe failure");
        assert_eq!(second_result.unwrap_err(), "temporary probe failure");
        assert_eq!(starts.load(Ordering::SeqCst), 1);

        let retry_starts = Arc::clone(&starts);
        let retry = run_coordinated_cli_probe(
            key.clone(),
            CliProbeCachePolicy::Reuse,
            move || async move {
                retry_starts.fetch_add(1, Ordering::SeqCst);
                Ok(CliProbeValue::Version(None))
            },
        )
        .await;
        assert!(retry.is_ok());
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        CLI_PROBE_CACHE.remove(&key);
    }

    #[test]
    fn cli_probe_key_tracks_runner_workspace_auth_and_config() {
        let runner = BaseCodingAgent::KimiCode;
        let executor = model_agent(Some("kimi-k2.5"));
        let env = ExecutionEnv::new(Default::default(), false, String::new());
        let base = cli_probe_request_key(
            runner,
            &executor,
            Path::new("/workspace/one"),
            &env,
            Some("oauth"),
        )
        .unwrap();
        let workspace_changed = cli_probe_request_key(
            runner,
            &executor,
            Path::new("/workspace/two"),
            &env,
            Some("oauth"),
        )
        .unwrap();
        let auth_changed = cli_probe_request_key(
            runner,
            &executor,
            Path::new("/workspace/one"),
            &env,
            Some("api_key"),
        )
        .unwrap();
        let config_changed = cli_probe_request_key(
            runner,
            &model_agent(Some("kimi-k2.5-preview")),
            Path::new("/workspace/one"),
            &env,
            Some("oauth"),
        )
        .unwrap();

        assert_ne!(base, workspace_changed);
        assert_ne!(base, auth_changed);
        assert_ne!(base, config_changed);
    }

    #[tokio::test]
    async fn invalid_runner_probe_config_is_isolated_as_one_outcome() {
        let runner = BaseCodingAgent::KimiCode;
        let executor_config = ExecutorConfig::new_with_default(model_agent(None));
        let mut store = AgentRuntimeStore::default();
        let mut config = default_config(runner);
        config.executor_options = serde_json::json!({ "model": 123 });
        store.configs.insert(runner, config);

        let outcome = discover_runner_runtime(
            runner,
            &executor_config,
            &store,
            Path::new("/workspace/one"),
        )
        .await;

        assert!(matches!(
            outcome,
            RunnerDiscoveryOutcome::Failed { runner: failed_runner, message, .. }
                if failed_runner == runner && message.starts_with("[runtime_configuration]")
        ));
    }

    #[test]
    fn env_key_validation_accepts_shell_safe_names() {
        let mut env = HashMap::new();
        env.insert("OPENAI_API_KEY".to_string(), "secret".to_string());
        env.insert("_CUSTOM_1".to_string(), "secret".to_string());

        assert!(validate_env_json(&env).is_ok());
    }

    #[test]
    fn env_key_validation_rejects_invalid_names() {
        let mut env = HashMap::new();
        env.insert("BAD-NAME".to_string(), "secret".to_string());

        assert!(matches!(
            validate_env_json(&env),
            Err(AgentRuntimeError::InvalidEnvKey(key)) if key == "BAD-NAME"
        ));
    }

    #[test]
    fn env_summary_includes_values() {
        let mut env = HashMap::new();
        env.insert("OPENAI_API_KEY".to_string(), "sk-test".to_string());

        let summary = summarize_env(&env);

        assert_eq!(summary[0].key, "OPENAI_API_KEY");
        assert_eq!(summary[0].value, "sk-test");
    }

    #[test]
    fn cli_version_output_prefers_stdout_and_trims() {
        let version =
            normalize_cli_version_output(b"\n codex-cli 0.125.0 \n", b"npm notice ignored\n");

        assert_eq!(version.as_deref(), Some("codex-cli 0.125.0"));
    }

    #[test]
    fn cli_version_output_falls_back_to_stderr() {
        let version = normalize_cli_version_output(b"", b"\nclaude-code 2.1.74\n");

        assert_eq!(version.as_deref(), Some("claude-code 2.1.74"));
    }

    #[test]
    fn discovery_update_version_prefers_detected_then_cached() {
        let runner = BaseCodingAgent::Opencode;
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: vec!["openai/gpt-5.2-codex".to_string()],
                version: Some("opencode 1.2.23".to_string()),
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: None,
            },
        );

        assert_eq!(
            version_for_discovery_update(&store, runner, Some("opencode 1.2.24".to_string()))
                .as_deref(),
            Some("opencode 1.2.24")
        );
        assert_eq!(
            version_for_discovery_update(&store, runner, None).as_deref(),
            Some("opencode 1.2.23")
        );
    }

    #[test]
    fn version_only_discovery_clears_stale_error_and_preserves_models() {
        let runner = BaseCodingAgent::Opencode;
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: vec!["openai/gpt-5.2-codex".to_string()],
                version: Some("opencode 1.2.23".to_string()),
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: Some("temporary provider failure".to_string()),
            },
        );

        let errors = apply_discovery_outcomes(
            &mut store,
            vec![RunnerDiscoveryOutcome::Discovered {
                runner,
                models: None,
                detected_version: Some("opencode 1.2.24".to_string()),
                version_error: None,
                auth_state: None,
            }],
        );

        let discovery = store
            .discoveries
            .get(&runner)
            .expect("runtime discovery should remain cached");
        assert_eq!(discovery.models, vec!["openai/gpt-5.2-codex"]);
        assert_eq!(discovery.version.as_deref(), Some("opencode 1.2.24"));
        assert_eq!(discovery.last_error, None);
        assert!(errors.is_empty());
    }

    #[test]
    fn version_only_discovery_reports_version_probe_error() {
        let runner = BaseCodingAgent::Opencode;
        let mut store = AgentRuntimeStore::default();

        let errors = apply_discovery_outcomes(
            &mut store,
            vec![RunnerDiscoveryOutcome::Discovered {
                runner,
                models: None,
                detected_version: None,
                version_error: Some(
                    "failed to resolve version executable: opencode not found".to_string(),
                ),
                auth_state: None,
            }],
        );

        let discovery = store
            .discoveries
            .get(&runner)
            .expect("version failure should be cached for status reporting");
        assert_eq!(
            discovery.last_error.as_deref(),
            Some("[version_check] failed to resolve version executable: opencode not found")
        );
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn refresh_response_reports_version_probe_error() {
        let runner = BaseCodingAgent::Opencode;
        let mut store = AgentRuntimeStore::default();

        let errors = apply_discovery_outcomes(
            &mut store,
            vec![RunnerDiscoveryOutcome::Discovered {
                runner,
                models: None,
                detected_version: None,
                version_error: Some("version command timed out after 12 seconds".to_string()),
                auth_state: None,
            }],
        );

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].runner_type, runner);
        assert_eq!(
            errors[0].message,
            "[version_check] version command timed out after 12 seconds"
        );
        assert_eq!(
            store.discoveries[&runner].last_error,
            Some("[version_check] version command timed out after 12 seconds".to_string())
        );
    }

    #[test]
    fn successful_version_probe_clears_only_version_error() {
        let runner = BaseCodingAgent::Opencode;
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: Vec::new(),
                version: None,
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: Some(
                    "[model_discovery] provider unavailable\n[version_check] executable not found"
                        .to_string(),
                ),
            },
        );

        cache_runner_version(&mut store, runner, "opencode 1.2.24".to_string());

        let discovery = &store.discoveries[&runner];
        assert_eq!(discovery.version.as_deref(), Some("opencode 1.2.24"));
        assert_eq!(
            discovery.last_error.as_deref(),
            Some("[model_discovery] provider unavailable")
        );
    }

    #[test]
    fn successful_acp_probe_clears_cached_authentication_required_error() {
        let runner = BaseCodingAgent::QoderCli;
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: Vec::new(),
                version: Some("qodercli 1.2.3".to_string()),
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: Some(
                    "[model_discovery] I/O error: Authentication required: Authentication is required\n[version_check] temporary warning"
                        .to_string(),
                ),
            },
        );

        clear_cached_authentication_required_error(&mut store, runner);

        assert_eq!(
            store.discoveries[&runner].last_error.as_deref(),
            Some("[version_check] temporary warning")
        );
    }

    #[test]
    fn acp_probe_models_are_cached_for_runtime_status() {
        let runner = BaseCodingAgent::Pi;
        let mut store = AgentRuntimeStore::default();
        cache_runner_acp_models(
            &mut store,
            runner,
            vec![
                "provider/model-a".to_string(),
                "provider/model-b".to_string(),
            ],
        );

        let discovery = store
            .discoveries
            .get(&runner)
            .expect("ACP probe models should be cached");
        assert_eq!(
            discovery.models,
            vec!["provider/model-a", "provider/model-b"]
        );
    }

    #[test]
    fn acp_probe_models_clear_stale_model_discovery_error() {
        let runner = BaseCodingAgent::Pi;
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: Vec::new(),
                version: Some("pi 0.83.0".to_string()),
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: Some(
                    "[model_discovery] ACP initialize failed: timeout\n[version_check] temporary warning"
                        .to_string(),
                ),
            },
        );

        cache_runner_acp_models(&mut store, runner, vec!["provider/model-a".to_string()]);

        let discovery = store.discoveries.get(&runner).unwrap();
        assert_eq!(discovery.models, vec!["provider/model-a"]);
        assert_eq!(discovery.version.as_deref(), Some("pi 0.83.0"));
        assert_eq!(
            discovery.last_error.as_deref(),
            Some("[version_check] temporary warning")
        );
    }

    #[test]
    fn acp_probe_empty_models_are_not_cached() {
        let runner = BaseCodingAgent::Pi;
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: vec!["existing-model".to_string()],
                version: None,
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: None,
            },
        );

        cache_runner_acp_models(&mut store, runner, Vec::new());

        let discovery = store.discoveries.get(&runner).unwrap();
        assert_eq!(discovery.models, vec!["existing-model"]);
    }

    #[test]
    fn status_error_details_preserve_each_failed_stage() {
        let merged = merge_status_error_details([
            Some(status_error_detail(
                "model_discovery",
                "provider request failed",
            )),
            Some(status_error_detail(
                "version_check",
                "version command timed out after 12 seconds",
            )),
        ])
        .unwrap();

        assert_eq!(
            merged,
            "[model_discovery] provider request failed\n[version_check] version command timed out after 12 seconds"
        );
    }

    #[test]
    fn command_failure_identifies_command_operation_and_result() {
        let detail = command_failure_detail(
            "/Users/test/.local/bin/copilot --version",
            "execute version command",
            "process failed with exit code 1: authentication required",
        );

        assert_eq!(
            detail,
            "command=`/Users/test/.local/bin/copilot --version`; operation=execute version command; result=process failed with exit code 1: authentication required"
        );
    }

    #[tokio::test]
    async fn missing_runner_skips_runtime_command_resolution() {
        let mut executor = model_agent(None);
        let CodingAgent::KimiCode(config) = &mut executor else {
            panic!("expected KimiCode executor");
        };
        config.cmd.base_command_override =
            Some("openteams-test-command-that-must-not-exist-8dd8c9e9".to_string());

        let result = resolve_runtime_command_for_diagnostics(false, &executor)
            .await
            .expect("uninstalled runner should not resolve its command");

        assert!(result.is_none());
    }

    #[test]
    fn successful_refresh_replaces_cached_cli_model_list() {
        let runner = BaseCodingAgent::Gemini;
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: vec!["gemini-old".to_string()],
                version: Some("gemini 1.0.0".to_string()),
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: None,
            },
        );

        let errors = apply_discovery_outcomes(
            &mut store,
            vec![RunnerDiscoveryOutcome::Discovered {
                runner,
                models: Some(vec!["gemini-new".to_string()]),
                detected_version: Some("gemini 1.1.0".to_string()),
                version_error: None,
                auth_state: None,
            }],
        );

        let discovery = store.discoveries.get(&runner).unwrap();
        assert!(errors.is_empty());
        assert_eq!(discovery.models, vec!["gemini-new"]);
        assert_eq!(discovery.version.as_deref(), Some("gemini 1.1.0"));
    }

    #[test]
    fn refresh_failure_preserves_old_discovery_models() {
        let runner = BaseCodingAgent::Opencode;
        let mut configs = HashMap::new();
        configs.insert(
            runner,
            AgentRuntimeDiscovery {
                models: vec!["openai/gpt-5.2-codex".to_string()],
                version: None,
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: None,
            },
        );
        let store = AgentRuntimeStore {
            configs: HashMap::new(),
            discoveries: configs,
        };
        let executor_config = ExecutorConfig::new_with_default(model_agent(None));

        let models = models_for_runner(runner, &executor_config, &store);

        assert_eq!(models, vec!["openai/gpt-5.2-codex"]);
    }

    #[test]
    fn aggregation_returns_runner_config_and_models() {
        let runner = BaseCodingAgent::KimiCode;
        let mut executors = HashMap::new();
        executors.insert(
            runner,
            ExecutorConfig::new_with_default(model_agent(Some("kimi-k2.5"))),
        );
        let profiles = ExecutorConfigs { executors };
        let mut runtime = default_config(runner);
        runtime.run_mode = AgentRunMode::Local;
        runtime
            .env_json
            .insert("KIMI_API_KEY".to_string(), "secret".to_string());
        let mut store = AgentRuntimeStore::default();
        store.configs.insert(runner, runtime);

        let statuses = build_statuses(&profiles, &store);

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].runner_type, runner);
        assert_eq!(statuses[0].run_mode, AgentRunMode::Local);
        assert_eq!(statuses[0].discovered_models, vec!["kimi-k2.5"]);
        assert_eq!(
            statuses[0].model_source,
            AgentRuntimeModelSource::ProfileFallback
        );
        assert_eq!(statuses[0].env_summary[0].value, "secret");
        assert_eq!(statuses[0].auth_state, AgentRuntimeAuthState::Authenticated);
    }

    #[test]
    fn qoder_runtime_status_recognizes_local_auth_state_file() {
        let temp = tempfile::tempdir().expect("temporary Qoder home");
        let auth_dir = temp.path().join(".auth");
        std::fs::create_dir(&auth_dir).expect("create auth directory");
        std::fs::write(auth_dir.join("user"), "encrypted-login-state")
            .expect("write Qoder auth state");

        let runner = BaseCodingAgent::QoderCli;
        let executor = CodingAgent::QoderCli(QoderCli {
            append_prompt: AppendPrompt::default(),
            model: Some("auto".to_string()),
            acp: None,
            cmd: CmdOverrides::default(),
            acp_mcp_policy: Default::default(),
            approvals: None,
        });
        let executor_config = ExecutorConfig::new_with_default(executor);
        let mut runtime = default_config(runner);
        runtime.env_json.insert(
            "QODER_CONFIG_DIR".to_string(),
            temp.path().to_string_lossy().into_owned(),
        );
        let mut store = AgentRuntimeStore::default();
        store.configs.insert(runner, runtime);
        let base = executor_config.get_default().expect("Qoder default config");

        let status = build_status(
            runner,
            &executor_config,
            base,
            &store,
            dependencies(true, true, true),
        );

        assert_eq!(status.auth_state, AgentRuntimeAuthState::Authenticated);
    }

    #[test]
    fn status_reports_invalid_runtime_configuration_detail() {
        let runner = BaseCodingAgent::KimiCode;
        let executor_config = ExecutorConfig::new_with_default(model_agent(None));
        let mut runtime = default_config(runner);
        runtime.executor_options = serde_json::json!({ "model": 42 });
        let mut store = AgentRuntimeStore::default();
        store.configs.insert(runner, runtime);
        let base = executor_config.get_default().unwrap();

        let status = build_status(
            runner,
            &executor_config,
            base,
            &store,
            dependencies(true, true, true),
        );

        let error = status
            .last_error
            .expect("invalid runtime configuration should be reported");
        assert!(error.starts_with("[runtime_configuration]"));
        assert!(error.contains("invalid type"));
        assert!(error.contains("expected a string"));
    }

    #[test]
    fn model_source_prefers_runner_discovery_over_profile_fallback() {
        let runner = BaseCodingAgent::Opencode;
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: vec!["opencode/free-model".to_string()],
                version: None,
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: None,
            },
        );
        let executor_config =
            ExecutorConfig::new_with_default(model_agent(Some("profile/fallback-model")));

        assert_eq!(
            models_for_runner(runner, &executor_config, &store),
            vec!["opencode/free-model"]
        );
        assert_eq!(
            model_source_for_runner(runner, &executor_config, &store),
            AgentRuntimeModelSource::Runner
        );
    }

    #[test]
    fn reconciled_discovery_applies_profile_changes_and_preserves_probe_models() {
        let runner = BaseCodingAgent::OpenTeamsCli;
        let checked_at = Utc::now();
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: vec![
                    "provider/stale-model".to_string(),
                    "opencode/free-model".to_string(),
                ],
                version: Some("openteams-cli 1.2.3".to_string()),
                auth_state: Some(AgentRuntimeAuthState::Authenticated),
                last_checked_at: checked_at,
                last_error: None,
            },
        );
        let executor_config =
            ExecutorConfig::new_with_default(model_agent(Some("provider/updated-model")));

        assert!(reconcile_cached_runtime_models(
            &mut store,
            runner,
            &["provider/stale-model".to_string()],
            &["provider/updated-model".to_string()],
        ));
        assert_eq!(
            models_for_runner(runner, &executor_config, &store),
            vec!["opencode/free-model", "provider/updated-model"]
        );
        assert_eq!(
            model_source_for_runner(runner, &executor_config, &store),
            AgentRuntimeModelSource::Runner
        );

        let discovery = store.discoveries.get(&runner).unwrap();
        assert_eq!(discovery.version.as_deref(), Some("openteams-cli 1.2.3"));
        assert_eq!(
            discovery.auth_state,
            Some(AgentRuntimeAuthState::Authenticated)
        );
        assert_eq!(discovery.last_checked_at, checked_at);
    }

    #[test]
    fn model_source_reports_none_when_no_models_are_available() {
        let runner = BaseCodingAgent::OpenTeamsCli;
        let store = AgentRuntimeStore::default();
        let executor_config = ExecutorConfig::new_with_default(model_agent(None));

        assert_eq!(
            model_source_for_runner(runner, &executor_config, &store),
            AgentRuntimeModelSource::None
        );
    }

    #[test]
    fn config_store_round_trips_runtime_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime.json");
        let runner = BaseCodingAgent::KimiCode;
        let mut runtime = default_config(runner);
        runtime.run_mode = AgentRunMode::Disabled;
        runtime.executor_options = serde_json::json!({
            "model": "kimi-k2.6",
            "cmd": {
                "base_command_override": "kimi-dev"
            }
        });
        runtime
            .env_json
            .insert("KIMI_API_KEY".to_string(), "secret".to_string());
        let mut store = AgentRuntimeStore::default();
        store.configs.insert(runner, runtime);

        write_store(&path, &store).unwrap();
        let restored = read_store(&path).unwrap();

        let restored_config = restored.configs.get(&runner).unwrap();
        assert_eq!(restored_config.runner_type, runner);
        assert_eq!(restored_config.run_mode, AgentRunMode::Disabled);
        assert_eq!(restored_config.env_json["KIMI_API_KEY"], "secret");
        assert_eq!(restored_config.executor_options["model"], "kimi-k2.6");
    }

    #[test]
    fn concurrent_config_and_discovery_updates_preserve_both() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime.json");
        let runner = BaseCodingAgent::KimiCode;
        write_store(&path, &AgentRuntimeStore::default()).unwrap();

        let barrier = Arc::new(std::sync::Barrier::new(2));
        let config_path = path.clone();
        let config_barrier = Arc::clone(&barrier);
        let config_update = std::thread::spawn(move || {
            config_barrier.wait();
            update_store(&config_path, |store| {
                let mut config = default_config(runner);
                config.run_mode = AgentRunMode::Local;
                config
                    .env_json
                    .insert("KIMI_API_KEY".to_string(), "new-secret".to_string());
                config.executor_options = serde_json::json!({ "model": "kimi-k2.6" });
                store.configs.insert(runner, config);
                Ok(())
            })
            .unwrap();
        });

        let discovery_path = path.clone();
        let discovery_barrier = Arc::clone(&barrier);
        let discovery_update = std::thread::spawn(move || {
            discovery_barrier.wait();
            update_store(&discovery_path, |store| {
                Ok(apply_discovery_outcomes(
                    store,
                    vec![RunnerDiscoveryOutcome::Discovered {
                        runner,
                        models: Some(vec!["kimi-k2.6".to_string()]),
                        detected_version: Some("kimi 1.0.0".to_string()),
                        version_error: None,
                        auth_state: None,
                    }],
                ))
            })
            .unwrap();
        });

        config_update.join().unwrap();
        discovery_update.join().unwrap();

        let restored = read_store(&path).unwrap();
        let config = restored.configs.get(&runner).unwrap();
        assert_eq!(config.run_mode, AgentRunMode::Local);
        assert_eq!(config.env_json["KIMI_API_KEY"], "new-secret");
        assert_eq!(config.executor_options["model"], "kimi-k2.6");
        let discovery = restored.discoveries.get(&runner).unwrap();
        assert_eq!(discovery.models, vec!["kimi-k2.6"]);
        assert_eq!(discovery.version.as_deref(), Some("kimi 1.0.0"));
    }

    #[test]
    fn executor_options_merge_into_default_executor() {
        let runner = BaseCodingAgent::KimiCode;
        let mut runtime = default_config(runner);
        runtime.executor_options = serde_json::json!({
            "model": "kimi-k2.6"
        });
        let mut store = AgentRuntimeStore::default();
        store.configs.insert(runner, runtime);
        let mut executor = model_agent(Some("gpt-5.2-codex"));
        let mut env = ExecutionEnv::new(Default::default(), false, String::new());

        apply_config_to_executor_and_env(runner, &mut executor, &mut env, &store).unwrap();

        assert_eq!(model_name(&executor), Some("kimi-k2.6"));
        let CodingAgent::KimiCode(config) = executor else {
            panic!("expected KimiCode executor");
        };
        assert_eq!(config.model.as_deref(), Some("kimi-k2.6"));
    }

    #[test]
    fn deepseek_model_option_merges_into_executor() {
        let runner = BaseCodingAgent::DeepseekHarness;
        let mut runtime = default_config(runner);
        runtime.executor_options = serde_json::json!({
            "model": "deepseek-v4-flash"
        });
        let mut store = AgentRuntimeStore::default();
        store.configs.insert(runner, runtime);
        let mut executor = CodingAgent::DeepseekHarness(DeepseekHarness::default());
        let mut env = ExecutionEnv::new(Default::default(), false, String::new());

        apply_config_to_executor_and_env(runner, &mut executor, &mut env, &store).unwrap();

        assert_eq!(model_name(&executor), Some("deepseek-v4-flash"));
        let CodingAgent::DeepseekHarness(config) = executor else {
            panic!("expected DeepSeek Harness executor");
        };
        assert_eq!(config.model.as_deref(), Some("deepseek-v4-flash"));
    }

    #[test]
    fn session_env_wins_over_agent_env_on_conflict() {
        let runner = BaseCodingAgent::KimiCode;
        let mut runtime = default_config(runner);
        runtime
            .env_json
            .insert("VK_CHAT_SESSION_ID".to_string(), "agent".to_string());
        runtime
            .env_json
            .insert("OPENAI_API_KEY".to_string(), "agent-key".to_string());
        let mut store = AgentRuntimeStore::default();
        store.configs.insert(runner, runtime);
        let mut executor = model_agent(None);
        let mut env = ExecutionEnv::new(Default::default(), false, String::new());
        env.insert("VK_CHAT_SESSION_ID", "session");

        apply_config_to_executor_and_env(runner, &mut executor, &mut env, &store).unwrap();

        assert_eq!(
            env.get("VK_CHAT_SESSION_ID").map(String::as_str),
            Some("session")
        );
        assert_eq!(
            env.get("OPENAI_API_KEY").map(String::as_str),
            Some("agent-key")
        );
    }

    #[test]
    fn serialized_runtime_status_has_no_model_override_or_reasoning_level() {
        let status = AgentRuntimeStatus {
            runner_type: BaseCodingAgent::Codex,
            installed: true,
            executable: true,
            availability: AvailabilityInfo::InstallationFound,
            auth_state: AgentRuntimeAuthState::Authenticated,
            node_available: true,
            npm_available: true,
            npx_available: false,
            discovered_models: vec!["gpt-5.2-codex".to_string()],
            model_source: AgentRuntimeModelSource::Runner,
            version: None,
            last_checked_at: None,
            last_error: None,
            run_mode: AgentRunMode::Auto,
            env_summary: Vec::new(),
            executor_options: serde_json::json!({ "ask_for_approval": "never" }),
        };

        let value = serde_json::to_value(status).unwrap();

        assert!(value.get("model_override").is_none());
        assert!(value.get("reasoning_level").is_none());
        assert!(value.get("model_reasoning_effort").is_none());
        assert_eq!(value["node_available"], true);
        assert_eq!(value["npm_available"], true);
        assert_eq!(value["npx_available"], false);
        assert_eq!(value["executor_options"]["ask_for_approval"], "never");
    }

    #[test]
    fn serialized_runtime_diagnostics_exposes_all_dependency_availability() {
        let diagnostics = AgentRuntimeDiagnostics {
            runner_type: BaseCodingAgent::Codex,
            installed: true,
            executable: false,
            availability: AvailabilityInfo::InstallationFound,
            auth_state: AgentRuntimeAuthState::Authenticated,
            node_available: true,
            npm_available: false,
            npx_available: true,
            config_path: "/tmp/config".to_string(),
            install_indicator_path: None,
            resolved_command: None,
            command_source: None,
            acp_probe: None,
            acp_probe_error: None,
            discovered_models: Vec::new(),
            model_source: AgentRuntimeModelSource::None,
            version: None,
            last_checked_at: None,
            last_error: None,
            run_mode: AgentRunMode::Auto,
            env_summary: Vec::new(),
            executor_options: serde_json::json!({}),
            pi_models_sync: None,
        };

        let value = serde_json::to_value(diagnostics).unwrap();

        assert_eq!(value["node_available"], true);
        assert_eq!(value["npm_available"], false);
        assert_eq!(value["npx_available"], true);
    }

    #[test]
    fn reasoning_capabilities_include_opencode_family_effort() {
        for runner in [BaseCodingAgent::Opencode, BaseCodingAgent::OpenTeamsCli] {
            let capability = reasoning_capability_for_runner(runner)
                .unwrap_or_else(|| panic!("{runner} should expose reasoning capability"));
            assert_eq!(
                capability,
                AgentRuntimeReasoningCapability::Effort {
                    options: vec![
                        "thinking-low".to_string(),
                        "thinking-medium".to_string(),
                        "thinking-high".to_string(),
                    ],
                }
            );
        }
    }

    #[test]
    fn reasoning_capabilities_match_current_acp_controls() {
        assert_eq!(
            reasoning_capability_for_runner(BaseCodingAgent::Codex),
            Some(AgentRuntimeReasoningCapability::Effort {
                options: strings(["low", "medium", "high", "xhigh", "max", "ultra"]),
            })
        );
        assert_eq!(
            reasoning_capability_for_runner(BaseCodingAgent::QwenCode),
            Some(AgentRuntimeReasoningCapability::Effort {
                options: strings(["low", "medium", "high", "xhigh", "max"]),
            })
        );
        assert_eq!(
            reasoning_capability_for_runner(BaseCodingAgent::Gemini),
            Some(AgentRuntimeReasoningCapability::Effort {
                options: strings(["low", "medium", "high"]),
            })
        );
        assert_eq!(
            reasoning_capability_for_runner(BaseCodingAgent::KimiCode),
            Some(AgentRuntimeReasoningCapability::Effort {
                options: strings(["low", "high", "max"]),
            })
        );
    }

    #[test]
    fn hermes_is_registered_as_an_acp_runner_without_static_reasoning() {
        assert!(
            ExecutorConfigs::from_defaults()
                .executors
                .contains_key(&BaseCodingAgent::Hermes),
            "Hermes must have a default profile"
        );
        assert_eq!(
            reasoning_capability_for_runner(BaseCodingAgent::Hermes),
            None,
            "Hermes model and options must come from the ACP probe, not static reasoning"
        );
        assert_eq!(
            runtime_dependency_requirement(BaseCodingAgent::Hermes),
            RuntimeDependencyRequirement::None,
            "Hermes is a native CLI without node/npm/npx dependencies"
        );
        assert_eq!(
            version_command_base(&CodingAgent::Hermes(
                executors::executors::hermes::Hermes::default()
            )),
            Some("hermes".to_string())
        );
    }

    #[test]
    fn kiro_is_registered_as_a_native_acp_runner_without_static_reasoning() {
        assert!(
            ExecutorConfigs::from_defaults()
                .executors
                .contains_key(&BaseCodingAgent::KiroCli),
            "Kiro CLI must have a default profile"
        );
        assert_eq!(
            reasoning_capability_for_runner(BaseCodingAgent::KiroCli),
            None,
            "Kiro model choices come from the ACP probe"
        );
        assert_eq!(
            runtime_dependency_requirement(BaseCodingAgent::KiroCli),
            RuntimeDependencyRequirement::None,
            "Kiro is a native CLI without node/npm/npx dependencies"
        );

        let executor = CodingAgent::KiroCli(KiroCli::default());
        assert!(cmd_overrides_for_executor(&executor).is_some());
        assert_eq!(version_command_base(&executor), None);
        assert_eq!(model_name(&executor), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kiro_version_detection_uses_the_executor_owned_command() {
        let mut kiro = KiroCli::default();
        kiro.cmd.base_command_override = Some("sh -c 'printf \"kiro-cli 2.20.1\\n\"'".to_string());
        kiro.cmd.additional_params = Some(vec!["--acp-only-option".to_string()]);
        let executor = CodingAgent::KiroCli(kiro);
        let mut env = ExecutionEnv::new(Default::default(), false, String::new());
        env.insert("KIRO_API_KEY", "fixture-secret-never-output");

        let version = detect_cli_version(&executor, &env)
            .await
            .expect("Kiro version detection");

        assert_eq!(version.as_deref(), Some("kiro-cli 2.20.1"));
        assert!(
            !version
                .unwrap_or_default()
                .contains("fixture-secret-never-output")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kiro_missing_auth_is_not_an_acp_or_discovery_error() {
        let mut kiro = KiroCli::default();
        kiro.cmd.base_command_override = Some("sh -c 'exit 1'".to_string());
        let executor = CodingAgent::KiroCli(kiro);
        let env = ExecutionEnv::new(Default::default(), false, String::new());

        let outcome = coordinated_probe_acp(
            BaseCodingAgent::KiroCli,
            &executor,
            Path::new("."),
            &env,
            None,
            CliProbeCachePolicy::Refresh,
        )
        .await
        .expect("missing Kiro authentication is a valid probe outcome");
        assert!(matches!(outcome, AcpProbeOutcome::Unauthenticated));

        let (models, auth_state) =
            discover_models_for_executor(BaseCodingAgent::KiroCli, &executor, Path::new("."), &env)
                .await
                .expect("missing Kiro authentication must not fail discovery");
        assert_eq!(models, None);
        assert_eq!(auth_state, Some(AgentRuntimeAuthState::Unauthenticated));

        let runner = BaseCodingAgent::KiroCli;
        let preserved_models = vec!["kiro/model".to_string()];
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: preserved_models.clone(),
                version: None,
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: Some(
                    "[model_discovery] ACP initialize failed: Auth required: login".to_string(),
                ),
            },
        );
        let errors = apply_discovery_outcomes(
            &mut store,
            vec![RunnerDiscoveryOutcome::Discovered {
                runner,
                models,
                detected_version: None,
                version_error: None,
                auth_state,
            }],
        );
        assert!(errors.is_empty());
        assert_eq!(store.discoveries[&runner].models, preserved_models);
        assert_eq!(
            store.discoveries[&runner].auth_state,
            Some(AgentRuntimeAuthState::Unauthenticated)
        );
        assert_eq!(store.discoveries[&runner].last_error, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kiro_probe_failure_redacts_api_key_from_cli_stderr() {
        let short_api_key = "abc";
        let mut kiro = KiroCli::default();
        kiro.cmd.base_command_override =
            Some("sh -c 'printf \"%s\" \"$KIRO_API_KEY\" >&2'".to_string());
        let executor = CodingAgent::KiroCli(kiro);
        let mut env = ExecutionEnv::new(Default::default(), false, String::new());
        env.insert("KIRO_API_KEY", short_api_key);

        let error = coordinated_probe_acp(
            BaseCodingAgent::KiroCli,
            &executor,
            Path::new("."),
            &env,
            None,
            CliProbeCachePolicy::Refresh,
        )
        .await
        .expect_err("invalid Kiro credentials must fail the ACP probe");

        assert!(error.contains("[redacted]"), "{error}");
        assert!(!error.contains(short_api_key));
    }

    #[test]
    fn deepseek_harness_is_registered_as_source_checkout_acp_runner() {
        assert!(
            ExecutorConfigs::from_defaults()
                .executors
                .contains_key(&BaseCodingAgent::DeepseekHarness),
            "DeepSeek Harness must have a default profile"
        );
        assert_eq!(
            reasoning_capability_for_runner(BaseCodingAgent::DeepseekHarness),
            None,
            "DeepSeek provider and model are configured by its Cordis composition"
        );
        assert_eq!(
            runtime_dependency_requirement(BaseCodingAgent::DeepseekHarness),
            RuntimeDependencyRequirement::None,
            "the built checkout launches directly through Node without npm or npx"
        );
        assert_eq!(
            version_command_base(&CodingAgent::DeepseekHarness(
                executors::executors::deepseek_harness::DeepseekHarness::default()
            )),
            None,
            "the ACP stdio server must not receive a generic --version argument"
        );
    }

    fn hermes_agent() -> CodingAgent {
        CodingAgent::Hermes(executors::executors::hermes::Hermes::default())
    }

    #[test]
    fn hermes_version_command_appends_version_flag_to_base() {
        let executor = hermes_agent();
        let base = version_command_base(&executor).expect("version base");
        let parts = CommandBuilder::new(base)
            .extend_params(["--version"])
            .build_initial()
            .expect("build version command");
        let display = parts.redacted_display();
        assert_eq!(display, "hermes --version");
    }

    #[test]
    fn hermes_default_profile_reports_not_found_without_cli() {
        let runner = BaseCodingAgent::Hermes;
        let mut executor = executors::executors::hermes::Hermes::default();
        executor.cmd.base_command_override =
            Some("openteams-hermes-cli-not-installed-never-real".to_string());
        let executor_config = ExecutorConfig::new_with_default(CodingAgent::Hermes(executor));
        let base = executor_config.get_default().unwrap();
        let store = AgentRuntimeStore::default();
        let status = build_status(
            runner,
            &executor_config,
            base,
            &store,
            dependencies(false, false, false),
        );
        assert!(!status.installed);
        assert!(!status.executable);
        assert!(
            matches!(status.availability, AvailabilityInfo::NotFound),
            "Hermes must be NotFound when `hermes` is not on PATH"
        );
        assert_eq!(status.discovered_models, Vec::<String>::new());
        assert_eq!(
            status.model_source,
            crate::services::agent_runtime::AgentRuntimeModelSource::None
        );
    }

    #[test]
    fn hermes_executor_options_merge_model_and_command_override() {
        let runner = BaseCodingAgent::Hermes;
        let mut executor = hermes_agent();
        let options = serde_json::json!({
            "model": "hermes-pro",
            "base_command_override": "/opt/hermes/bin/hermes"
        });
        apply_executor_options(runner, &mut executor, &options).expect("apply options");
        let CodingAgent::Hermes(config) = executor else {
            panic!("expected Hermes after merge");
        };
        assert_eq!(config.model.as_deref(), Some("hermes-pro"));
        assert_eq!(
            config.cmd.base_command_override.as_deref(),
            Some("/opt/hermes/bin/hermes")
        );
    }

    #[test]
    fn hermes_executor_options_merge_acp_approval_mode() {
        let runner = BaseCodingAgent::Hermes;
        let mut executor = hermes_agent();
        let options = serde_json::json!({
            "acp": { "approval_mode": "auto_allow" }
        });
        apply_executor_options(runner, &mut executor, &options).expect("apply acp options");
        let CodingAgent::Hermes(config) = executor else {
            panic!("expected Hermes after acp merge");
        };
        let acp = config.acp.expect("acp options present");
        assert_eq!(
            acp.approval_mode,
            Some(executors::executors::acp::AcpApprovalMode::AutoAllow)
        );
    }

    #[test]
    fn hermes_acp_probe_caches_models_and_clears_auth_error() {
        let runner = BaseCodingAgent::Hermes;
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: Vec::new(),
                version: None,
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: Some(status_error_detail(
                    "acp_probe",
                    "Auth required: ACP authentication method was not advertised",
                )),
            },
        );
        let probe_models = vec!["hermes-pro".to_string(), "hermes-flash".to_string()];
        cache_runner_acp_models(&mut store, runner, probe_models.clone());
        clear_cached_authentication_required_error(&mut store, runner);
        let entry = store.discoveries.get(&runner).expect("discovery entry");
        assert_eq!(entry.models, probe_models);
        assert!(
            !entry
                .last_error
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains("authentication required"),
            "successful ACP probe must clear the cached authentication required error"
        );
    }

    #[test]
    fn hermes_acp_probe_failure_preserves_installed_state_and_cached_models() {
        let runner = BaseCodingAgent::Hermes;
        let mut executor = executors::executors::hermes::Hermes::default();
        executor.cmd.base_command_override = Some("/usr/bin/true".to_string());
        let executor_config = ExecutorConfig::new_with_default(CodingAgent::Hermes(executor));
        let base = executor_config.get_default().unwrap();
        let preserved_models = vec!["provider/hermes-pro".to_string()];
        let mut store = AgentRuntimeStore::default();
        store.discoveries.insert(
            runner,
            AgentRuntimeDiscovery {
                models: preserved_models.clone(),
                version: Some("hermes 1.0.0".to_string()),
                auth_state: None,
                last_checked_at: Utc::now(),
                last_error: None,
            },
        );

        let errors = apply_discovery_outcomes(
            &mut store,
            vec![RunnerDiscoveryOutcome::Failed {
                runner,
                message: "[model_discovery] ACP initialize failed: hermes exited".to_string(),
                detected_version: None,
                preserved_models: preserved_models.clone(),
            }],
        );
        let status = build_status(
            runner,
            &executor_config,
            base,
            &store,
            dependencies(false, false, false),
        );

        assert!(status.installed);
        assert_eq!(status.discovered_models, preserved_models);
        assert_eq!(errors[0].preserved_models, status.discovered_models);
        assert!(status.last_error.unwrap().contains("ACP initialize failed"));
    }

    #[test]
    fn hermes_cached_probe_auth_state_controls_installed_status() {
        let runner = BaseCodingAgent::Hermes;
        let mut executor = executors::executors::hermes::Hermes::default();
        executor.cmd.base_command_override = Some("/usr/bin/true".to_string());
        let executor_config = ExecutorConfig::new_with_default(CodingAgent::Hermes(executor));
        let base = executor_config.get_default().expect("Hermes executor");

        for auth_state in [
            AgentRuntimeAuthState::Unauthenticated,
            AgentRuntimeAuthState::Authenticated,
        ] {
            let mut store = AgentRuntimeStore::default();
            store.discoveries.insert(
                runner,
                AgentRuntimeDiscovery {
                    models: Vec::new(),
                    version: Some("Hermes Agent v0.20.0".to_string()),
                    auth_state: Some(auth_state),
                    last_checked_at: Utc::now(),
                    last_error: None,
                },
            );
            let status = build_status(
                runner,
                &executor_config,
                base,
                &store,
                dependencies(false, false, false),
            );
            assert!(status.installed);
            assert!(status.executable);
            assert_eq!(status.auth_state, auth_state);
            assert_eq!(status.version.as_deref(), Some("Hermes Agent v0.20.0"));
        }
    }

    #[test]
    fn hermes_discovery_auth_state_is_backward_compatible() {
        let discovery: AgentRuntimeDiscovery = serde_json::from_value(serde_json::json!({
            "models": [],
            "version": "Hermes Agent v0.20.0",
            "last_checked_at": Utc::now(),
            "last_error": null
        }))
        .expect("legacy discovery JSON");
        assert_eq!(discovery.auth_state, None);
    }

    #[test]
    fn hermes_run_mode_disabled_blocks_executable_while_reporting_installed() {
        let runner = BaseCodingAgent::Hermes;
        let executor_config = ExecutorConfig::new_with_default(hermes_agent());
        let base = executor_config.get_default().unwrap();
        let mut store = AgentRuntimeStore::default();
        store.configs.insert(
            runner,
            AgentRuntimeConfig {
                runner_type: runner,
                run_mode: crate::services::agent_runtime::AgentRunMode::Disabled,
                env_json: HashMap::new(),
                executor_options: serde_json::json!({}),
                updated_at: Utc::now(),
            },
        );
        let status = build_status(
            runner,
            &executor_config,
            base,
            &store,
            dependencies(false, false, false),
        );
        assert!(!status.executable);
        assert_eq!(
            status.run_mode,
            crate::services::agent_runtime::AgentRunMode::Disabled
        );
    }

    #[test]
    fn hermes_default_profile_has_empty_model_so_probe_is_authoritative() {
        let profiles = ExecutorConfigs::from_defaults();
        let executor = profiles
            .executors
            .get(&BaseCodingAgent::Hermes)
            .and_then(|config| {
                config
                    .get_default()
                    .or_else(|| config.configurations.values().next())
            })
            .expect("Hermes default profile");
        let CodingAgent::Hermes(config) = executor else {
            panic!("expected Hermes default profile");
        };
        assert!(
            config.model.is_none(),
            "default Hermes profile must not pin a model so the ACP probe stays authoritative"
        );
        assert!(
            config.acp.is_none(),
            "default Hermes profile must not carry ACP options so member overlays apply cleanly"
        );
    }
}
