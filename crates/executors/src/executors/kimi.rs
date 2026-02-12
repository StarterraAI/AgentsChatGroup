use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::Arc,
};

use async_trait::async_trait;
use chrono::Utc;
use command_group::AsyncCommandGroup;
use futures::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{io::AsyncWriteExt, process::Command};
use ts_rs::TS;
use workspace_utils::{msg_store::MsgStore, shell::resolve_executable_path_blocking};

use crate::{
    command::{CmdOverrides, CommandBuildError, CommandBuilder, CommandParts, apply_overrides},
    env::ExecutionEnv,
    executors::{AppendPrompt, AvailabilityInfo, ExecutorError, SpawnedChild, StandardCodingAgentExecutor},
    logs::{
        ActionType, NormalizedEntry, NormalizedEntryType, ToolResult, ToolStatus,
        stderr_processor::normalize_stderr_logs,
        utils::{EntryIndexProvider, patch::ConversationPatch},
    },
    stdout_dup,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, JsonSchema)]
pub struct KimiCode {
    #[serde(default)]
    pub append_prompt: AppendPrompt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yolo: Option<bool>,
    #[serde(flatten)]
    pub cmd: CmdOverrides,
}

#[derive(Clone)]
struct ToolEntryState {
    index: usize,
    tool_name: String,
    arguments: Option<Value>,
}

impl KimiCode {
    const SESSION_PREFIX: &'static str = "[kimi-session] ";
    const SESSION_SENTINEL: &'static str = "KIMI_CONTINUE";

    pub fn base_command() -> &'static str {
        "kimi"
    }

    fn build_command_builder(&self) -> Result<CommandBuilder, CommandBuildError> {
        let mut builder = CommandBuilder::new(Self::base_command())
            .params(["--print", "--output-format", "stream-json"]);

        if let Some(model) = &self.model {
            builder = builder.extend_params(["--model", model.as_str()]);
        }

        if self.yolo.unwrap_or(false) {
            builder = builder.extend_params(["--yolo"]);
        }

        apply_overrides(builder, &self.cmd)
    }

    fn extract_assistant_text(message: &Value) -> String {
        let Some(content) = message.get("content") else {
            return String::new();
        };

        match content {
            Value::String(text) => text.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(|v| v.as_str())
                        .map(|v| v.to_string())
                })
                .collect::<String>(),
            _ => String::new(),
        }
    }

    fn extract_tool_calls(message: &Value) -> Vec<(String, String, Option<Value>)> {
        let mut calls = Vec::new();
        let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) else {
            return calls;
        };

        for call in tool_calls {
            let Some(id) = call.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(function) = call.get("function") else {
                continue;
            };
            let Some(tool_name) = function.get("name").and_then(|v| v.as_str()) else {
                continue;
            };

            let arguments = function
                .get("arguments")
                .and_then(Self::parse_tool_arguments);

            calls.push((id.to_string(), tool_name.to_string(), arguments));
        }

        calls
    }

    fn parse_tool_arguments(value: &Value) -> Option<Value> {
        match value {
            Value::Null => None,
            Value::String(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    serde_json::from_str::<Value>(trimmed)
                        .ok()
                        .or_else(|| Some(Value::String(raw.clone())))
                }
            }
            other => Some(other.clone()),
        }
    }

    fn extract_tool_result(message: &Value) -> (Option<String>, String) {
        let tool_call_id = message
            .get("tool_call_id")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());

        let content = match message.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(|v| v.as_str())
                        .map(|v| v.to_string())
                        .or_else(|| {
                            part.get("content")
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string())
                        })
                })
                .collect::<String>(),
            Some(other) => other.to_string(),
            None => String::new(),
        };

        (tool_call_id, content)
    }

    fn merge_assistant_text(current: &str, incoming: &str) -> String {
        if current.is_empty() {
            return incoming.to_string();
        }
        if incoming.starts_with(current) {
            return incoming.to_string();
        }
        if current.ends_with(incoming) {
            return current.to_string();
        }

        let mut next = String::with_capacity(current.len() + incoming.len());
        next.push_str(current);
        next.push_str(incoming);
        next
    }
}

async fn spawn_kimi(
    command_parts: CommandParts,
    prompt: &str,
    current_dir: &Path,
    env: &ExecutionEnv,
    cmd_overrides: &CmdOverrides,
) -> Result<SpawnedChild, ExecutorError> {
    let (program_path, args) = command_parts.into_resolved().await?;

    let mut command = Command::new(program_path);
    command
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(current_dir)
        .env("NO_COLOR", "1")
        .args(args);

    env.clone()
        .with_profile(cmd_overrides)
        .apply_to_command(&mut command);

    let mut child = command.group_spawn()?;

    if let Some(mut stdin) = child.inner().stdin.take() {
        stdin.write_all(prompt.as_bytes()).await?;
        stdin.shutdown().await?;
    }

    let (_, appender) = stdout_dup::tee_stdout_with_appender(&mut child)?;
    appender.append_line(format!(
        "{}{}",
        KimiCode::SESSION_PREFIX,
        KimiCode::SESSION_SENTINEL
    ));

    Ok(child.into())
}

#[async_trait]
impl StandardCodingAgentExecutor for KimiCode {
    async fn spawn(
        &self,
        current_dir: &Path,
        prompt: &str,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        let command = self.build_command_builder()?.build_initial()?;
        let combined_prompt = self.append_prompt.combine_prompt(prompt);
        spawn_kimi(command, &combined_prompt, current_dir, env, &self.cmd).await
    }

    async fn spawn_follow_up(
        &self,
        current_dir: &Path,
        prompt: &str,
        session_id: &str,
        _reset_to_message_id: Option<&str>,
        env: &ExecutionEnv,
    ) -> Result<SpawnedChild, ExecutorError> {
        let additional_args = if session_id == Self::SESSION_SENTINEL {
            vec!["--continue".to_string()]
        } else {
            vec!["--session".to_string(), session_id.to_string()]
        };

        let command = self
            .build_command_builder()?
            .build_follow_up(&additional_args)?;
        let combined_prompt = self.append_prompt.combine_prompt(prompt);
        spawn_kimi(command, &combined_prompt, current_dir, env, &self.cmd).await
    }

    fn normalize_logs(&self, msg_store: Arc<MsgStore>, _worktree_path: &Path) {
        let entry_index_provider = EntryIndexProvider::start_from(&msg_store);
        normalize_stderr_logs(msg_store.clone(), entry_index_provider.clone());

        tokio::spawn(async move {
            let mut stdout_lines = msg_store.stdout_lines_stream();
            let mut model_reported = false;
            let mut current_assistant_index: Option<usize> = None;
            let mut current_assistant_text = String::new();
            let mut tool_entries: HashMap<String, ToolEntryState> = HashMap::new();

            while let Some(Ok(line)) = stdout_lines.next().await {
                if let Some(session_id) = line.strip_prefix(KimiCode::SESSION_PREFIX) {
                    msg_store.push_session_id(session_id.trim().to_string());
                    continue;
                }

                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let payload: Value = match serde_json::from_str(trimmed) {
                    Ok(value) => value,
                    Err(_) => {
                        let entry = NormalizedEntry {
                            timestamp: None,
                            entry_type: NormalizedEntryType::SystemMessage,
                            content: strip_ansi_escapes::strip_str(trimmed),
                            metadata: None,
                        };
                        let index = entry_index_provider.next();
                        msg_store.push_patch(ConversationPatch::add_normalized_entry(index, entry));
                        continue;
                    }
                };

                let event_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or_default();
                let message = payload.get("message");

                match (event_type, message) {
                    ("assistant", Some(message)) => {
                        if !model_reported
                            && let Some(model) = message.get("model").and_then(|v| v.as_str())
                        {
                            model_reported = true;
                            let entry = NormalizedEntry {
                                timestamp: None,
                                entry_type: NormalizedEntryType::SystemMessage,
                                content: format!("model: {model}"),
                                metadata: None,
                            };
                            let index = entry_index_provider.next();
                            msg_store.push_patch(ConversationPatch::add_normalized_entry(index, entry));
                        }

                        for (tool_call_id, tool_name, arguments) in
                            KimiCode::extract_tool_calls(message)
                        {
                            if tool_entries.contains_key(&tool_call_id) {
                                continue;
                            }

                            let action_type = ActionType::Tool {
                                tool_name: tool_name.clone(),
                                arguments: arguments.clone(),
                                result: None,
                            };
                            let entry = NormalizedEntry {
                                timestamp: None,
                                entry_type: NormalizedEntryType::ToolUse {
                                    tool_name: tool_name.clone(),
                                    action_type,
                                    status: ToolStatus::Created,
                                },
                                content: tool_name.clone(),
                                metadata: None,
                            };

                            let index = entry_index_provider.next();
                            msg_store.push_patch(ConversationPatch::add_normalized_entry(index, entry));
                            tool_entries.insert(
                                tool_call_id,
                                ToolEntryState {
                                    index,
                                    tool_name,
                                    arguments,
                                },
                            );
                        }

                        let text = KimiCode::extract_assistant_text(message);
                        if text.is_empty() {
                            continue;
                        }

                        let merged = KimiCode::merge_assistant_text(&current_assistant_text, &text);
                        current_assistant_text = merged.clone();

                        let entry = NormalizedEntry {
                            timestamp: None,
                            entry_type: NormalizedEntryType::AssistantMessage,
                            content: merged,
                            metadata: None,
                        };

                        if let Some(index) = current_assistant_index {
                            msg_store.push_patch(ConversationPatch::replace(index, entry));
                        } else {
                            let index = entry_index_provider.next();
                            current_assistant_index = Some(index);
                            msg_store.push_patch(ConversationPatch::add_normalized_entry(index, entry));
                        }
                    }
                    ("tool", Some(message)) => {
                        current_assistant_index = None;
                        current_assistant_text.clear();

                        let (tool_call_id, result_text) = KimiCode::extract_tool_result(message);
                        let Some(tool_call_id) = tool_call_id else {
                            continue;
                        };
                        let Some(state) = tool_entries.get(&tool_call_id).cloned() else {
                            continue;
                        };

                        let action_type = ActionType::Tool {
                            tool_name: state.tool_name.clone(),
                            arguments: state.arguments.clone(),
                            result: if result_text.trim().is_empty() {
                                None
                            } else {
                                Some(ToolResult::markdown(result_text.clone()))
                            },
                        };

                        let entry = NormalizedEntry {
                            timestamp: None,
                            entry_type: NormalizedEntryType::ToolUse {
                                tool_name: state.tool_name,
                                action_type,
                                status: ToolStatus::Success,
                            },
                            content: if result_text.trim().is_empty() {
                                "Tool completed".to_string()
                            } else {
                                result_text
                            },
                            metadata: None,
                        };
                        msg_store.push_patch(ConversationPatch::replace(state.index, entry));
                    }
                    _ => {
                        current_assistant_index = None;
                        current_assistant_text.clear();
                        let entry = NormalizedEntry {
                            timestamp: None,
                            entry_type: NormalizedEntryType::SystemMessage,
                            content: payload.to_string(),
                            metadata: None,
                        };
                        let index = entry_index_provider.next();
                        msg_store.push_patch(ConversationPatch::add_normalized_entry(index, entry));
                    }
                }
            }
        });
    }

    fn default_mcp_config_path(&self) -> Option<std::path::PathBuf> {
        dirs::home_dir().map(|home| home.join(".kimi").join("mcp.json"))
    }

    fn get_availability_info(&self) -> AvailabilityInfo {
        if resolve_executable_path_blocking(Self::base_command()).is_none() {
            return AvailabilityInfo::NotFound;
        }

        if std::env::var("MOONSHOT_API_KEY")
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
        {
            return AvailabilityInfo::LoginDetected {
                last_auth_timestamp: Utc::now().timestamp(),
            };
        }

        AvailabilityInfo::InstallationFound
    }
}
