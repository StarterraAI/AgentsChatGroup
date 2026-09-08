use std::collections::{HashMap, HashSet};

use anyhow::{Context, Error};
use db::models::member_execution_config::MemberExecutionConfig;
use executors::{executors::BaseCodingAgent, profile::ExecutorProfileId};
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;
use utils::{path::home_directory, text::sanitize_member_handle};
pub use v8::{
    EditorConfig, EditorType, GitHubConfig, NotificationConfig, NotificationInboxSource,
    NotificationInboxSourcesConfig, SendMessageShortcut, ShowcaseState, SoundFile, ThemeMode,
    UiLanguage,
};

use crate::services::config::{preset_loader::PresetLoader, versions::v8};

fn default_git_branch_prefix() -> String {
    "vk".to_string()
}

fn default_pr_auto_description_enabled() -> bool {
    true
}

fn default_commit_reminder_enabled() -> bool {
    true
}

fn default_max_agent_chain_depth() -> u32 {
    8
}

#[derive(Clone, Debug, Default, Serialize, TS, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[ts(use_ts_enum)]
pub enum ChatBubbleFontSize {
    Px12,
    Px13,
    #[default]
    Px14,
    Px15,
    Px16,
    Px18,
}

fn default_chat_bubble_font_size() -> ChatBubbleFontSize {
    ChatBubbleFontSize::default()
}

impl<'de> Deserialize<'de> for ChatBubbleFontSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "small" | "px12" | "12px" => Ok(Self::Px12),
            "px13" | "13px" => Ok(Self::Px13),
            "medium" | "px14" | "14px" => Ok(Self::Px14),
            "px15" | "15px" => Ok(Self::Px15),
            "large" | "px16" | "16px" => Ok(Self::Px16),
            "px18" | "18px" => Ok(Self::Px18),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &[
                    "small", "medium", "large", "px12", "px13", "px14", "px15", "px16", "px18",
                    "12px", "13px", "14px", "15px", "16px", "18px",
                ],
            )),
        }
    }
}

fn deserialize_chat_bubble_font_size<'de, D>(
    deserializer: D,
) -> Result<ChatBubbleFontSize, D::Error>
where
    D: Deserializer<'de>,
{
    ChatBubbleFontSize::deserialize(deserializer)
}

/// Chat Member Preset Template
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct ChatMemberPreset {
    /// Unique identifier for the preset
    pub id: String,
    /// Display name (also used as @mention handle)
    pub name: String,
    /// Description of the preset's purpose
    pub description: String,
    /// Optional runner type (null means use default)
    pub runner_type: Option<String>,
    /// Optional recommended model identifier for the selected runner
    #[serde(default)]
    pub recommended_model: Option<String>,
    /// Complete member-scoped execution settings copied when applying this preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub execution_config: Option<MemberExecutionConfig>,
    /// System prompt defining the agent's behavior
    pub system_prompt: String,
    /// Optional default workspace path
    pub default_workspace_path: Option<String>,
    /// Skills preselected for members created from this preset
    #[serde(default)]
    pub selected_skill_ids: Vec<String>,
    /// Tools enabled for this preset
    #[serde(default)]
    pub tools_enabled: serde_json::Value,
    /// Whether this is a built-in preset (cannot be deleted)
    pub is_builtin: bool,
    /// Whether this preset is enabled (visible for import)
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// A single workflow step in a team template.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct ChatWorkflowStep {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
}

/// Chat Team Preset Template (aggregate: embeds member snapshots)
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct ChatTeamPreset {
    /// Unique identifier for the preset
    pub id: String,
    /// Display name of the team
    pub name: String,
    /// Description of the team's purpose
    pub description: String,
    /// Embedded team member snapshots (aggregate model).
    #[serde(default)]
    pub members: Vec<ChatMemberPreset>,
    /// Optional ID of the lead member (references a member in `members`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_member_id: Option<String>,
    /// Optional workflow steps for the team template.
    #[serde(default)]
    pub workflow_steps: Vec<ChatWorkflowStep>,
    /// Optional team protocol injected when importing this team preset
    #[serde(default)]
    pub team_protocol: String,
    /// Whether this is a built-in preset (cannot be deleted)
    pub is_builtin: bool,
    /// Whether this preset is enabled (visible for import)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Standard templates are shown before advanced templates in the catalog.
    #[serde(default = "default_team_template_tier")]
    pub tier: ChatTeamTemplateTier,
}

/// Intermediate representation used only to deserialize legacy and aggregate
/// team preset JSON. Supports both the legacy `member_ids` form and the new
/// embedded `members` form.
#[derive(Deserialize)]
struct ChatTeamPresetRaw {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    member_ids: Vec<String>,
    #[serde(default)]
    members: Vec<ChatMemberPreset>,
    #[serde(default)]
    lead_member_id: Option<String>,
    #[serde(default)]
    workflow_steps: Vec<ChatWorkflowStep>,
    #[serde(default)]
    team_protocol: String,
    #[serde(default)]
    is_builtin: bool,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_team_template_tier")]
    tier: ChatTeamTemplateTier,
}

#[derive(Deserialize)]
struct ChatPresetsConfigRaw {
    #[serde(default)]
    members: Vec<ChatMemberPreset>,
    #[serde(default)]
    teams: Vec<ChatTeamPresetRaw>,
    #[serde(default)]
    team_protocol: Option<String>,
}

/// Chat Presets Configuration
#[derive(Clone, Debug, Serialize, TS, PartialEq, Eq)]
pub struct ChatPresetsConfig {
    /// Built-in role catalog and legacy member presets (build input only).
    pub members: Vec<ChatMemberPreset>,
    /// List of team preset templates (aggregate: each team embeds its members).
    pub teams: Vec<ChatTeamPreset>,
    /// Team collaboration protocol content; empty string disables injection
    pub team_protocol: Option<String>,
}

impl<'de> Deserialize<'de> for ChatPresetsConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = ChatPresetsConfigRaw::deserialize(deserializer)?;
        let member_by_id: HashMap<&str, &ChatMemberPreset> = raw
            .members
            .iter()
            .map(|member| (member.id.as_str(), member))
            .collect();

        let teams = raw
            .teams
            .into_iter()
            .map(|team_raw| {
                let members = if !team_raw.members.is_empty() {
                    team_raw.members
                } else if !team_raw.member_ids.is_empty() {
                    team_raw
                        .member_ids
                        .iter()
                        .map(|member_id| {
                            member_by_id
                                .get(member_id.as_str())
                                .copied()
                                .cloned()
                                .ok_or_else(|| {
                                    serde::de::Error::custom(format!(
                                        "team preset \"{}\" references unknown member preset: {}",
                                        team_raw.id, member_id
                                    ))
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    Vec::new()
                };
                let lead_member_id = team_raw.lead_member_id;
                if let Some(ref lead_id) = lead_member_id
                    && !members.iter().any(|member| &member.id == lead_id)
                {
                    return Err(serde::de::Error::custom(format!(
                        "team preset \"{}\" lead_member_id references unknown member: {}",
                        team_raw.id, lead_id
                    )));
                }
                Ok(ChatTeamPreset {
                    id: team_raw.id,
                    name: team_raw.name,
                    description: team_raw.description,
                    members,
                    lead_member_id,
                    workflow_steps: team_raw.workflow_steps,
                    team_protocol: team_raw.team_protocol,
                    is_builtin: team_raw.is_builtin,
                    enabled: team_raw.enabled,
                    tier: team_raw.tier,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ChatPresetsConfig {
            members: raw.members,
            teams,
            team_protocol: raw.team_protocol,
        })
    }
}

/// Chat Compression Configuration
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
pub struct ChatCompressionConfig {
    /// Token threshold before compression kicks in (default: 5000000)
    #[serde(default = "default_token_threshold")]
    pub token_threshold: u32,
    /// Percentage of messages to compress (default: 25)
    #[serde(default = "default_compression_percentage")]
    pub compression_percentage: u8,
}

fn default_token_threshold() -> u32 {
    50000
}

fn default_compression_percentage() -> u8 {
    25
}

impl Default for ChatCompressionConfig {
    fn default() -> Self {
        Self {
            token_threshold: default_token_threshold(),
            compression_percentage: default_compression_percentage(),
        }
    }
}

fn default_chat_compression() -> ChatCompressionConfig {
    ChatCompressionConfig::default()
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChatTeamTemplateTier {
    #[default]
    Standard,
    Advanced,
}

fn default_team_template_tier() -> ChatTeamTemplateTier {
    ChatTeamTemplateTier::Standard
}

fn normalize_selected_skill_ids(skill_ids: &[String]) -> Vec<String> {
    let mut normalized = skill_ids
        .iter()
        .map(|skill_id| skill_id.trim().to_string())
        .filter(|skill_id| !skill_id.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn complete_chat_presets_with_builtins(chat_presets: &mut ChatPresetsConfig) {
    let defaults = default_chat_presets();
    let legacy_default_team_protocol = PresetLoader::load_team_protocol();
    let default_workspace_path = Some(home_directory().to_string_lossy().to_string());
    let default_builtin_members: HashMap<String, ChatMemberPreset> = defaults
        .members
        .iter()
        .map(|preset| (preset.id.clone(), preset.clone()))
        .collect();
    let default_builtin_teams: HashMap<String, ChatTeamPreset> = defaults
        .teams
        .iter()
        .map(|preset| (preset.id.clone(), preset.clone()))
        .collect();

    let builtin_member_ids: HashSet<&str> = defaults
        .members
        .iter()
        .map(|preset| preset.id.as_str())
        .collect();
    let builtin_team_ids: HashSet<&str> = defaults
        .teams
        .iter()
        .map(|preset| preset.id.as_str())
        .collect();

    // Keep custom presets untouched; remove only legacy built-in entries
    // that are no longer part of the current built-in catalog.
    chat_presets
        .members
        .retain(|preset| !preset.is_builtin || builtin_member_ids.contains(preset.id.as_str()));
    chat_presets
        .teams
        .retain(|preset| !preset.is_builtin || builtin_team_ids.contains(preset.id.as_str()));

    for preset in &mut chat_presets.members {
        preset.selected_skill_ids = normalize_selected_skill_ids(&preset.selected_skill_ids);
        preset.default_workspace_path = default_workspace_path.clone();
        if preset.is_builtin
            && let Some(default_preset) = default_builtin_members.get(&preset.id)
        {
            preset.name = default_preset.name.clone();
            preset.description = default_preset.description.clone();
            preset.runner_type = default_preset.runner_type.clone();
            preset.recommended_model = default_preset.recommended_model.clone();
            preset.execution_config = default_preset.execution_config.clone();
            preset.system_prompt = default_preset.system_prompt.clone();
            preset.selected_skill_ids =
                normalize_selected_skill_ids(&default_preset.selected_skill_ids);
            preset.tools_enabled = default_preset.tools_enabled.clone();
            preset.enabled = default_preset.enabled;
        }
        preset.name = sanitize_member_handle(&preset.name);
    }

    for preset in &mut chat_presets.teams {
        if preset.is_builtin
            && let Some(default_preset) = default_builtin_teams.get(&preset.id)
        {
            preset.name = default_preset.name.clone();
            preset.description = default_preset.description.clone();
            preset.members = default_preset.members.clone();
            preset.lead_member_id = default_preset.lead_member_id.clone();
            preset.workflow_steps = default_preset.workflow_steps.clone();
            preset.team_protocol = default_preset.team_protocol.clone();
            preset.enabled = default_preset.enabled;
            preset.tier = default_preset.tier;
        }
    }

    let mut existing_member_ids: HashSet<String> = chat_presets
        .members
        .iter()
        .map(|preset| preset.id.clone())
        .collect();
    for preset in defaults.members {
        if existing_member_ids.insert(preset.id.clone()) {
            chat_presets.members.push(preset);
        }
    }

    let mut existing_team_ids: HashSet<String> = chat_presets
        .teams
        .iter()
        .map(|preset| preset.id.clone())
        .collect();
    for preset in defaults.teams {
        if existing_team_ids.insert(preset.id.clone()) {
            chat_presets.teams.push(preset);
        }
    }

    if matches!(
        chat_presets.team_protocol.as_deref(),
        Some(protocol) if protocol == legacy_default_team_protocol.as_str()
    ) {
        chat_presets.team_protocol = Some(String::new());
    } else if chat_presets.team_protocol.is_none() {
        chat_presets.team_protocol = defaults.team_protocol;
    }
}

fn default_chat_presets() -> ChatPresetsConfig {
    let mut chat_presets = PresetLoader::load_builtin_presets();
    chat_presets.team_protocol = Some(String::new());
    chat_presets
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Config {
    pub config_version: String,
    pub theme: ThemeMode,
    pub executor_profile: ExecutorProfileId,
    pub disclaimer_acknowledged: bool,
    pub onboarding_acknowledged: bool,
    pub notifications: NotificationConfig,
    pub editor: EditorConfig,
    pub github: GitHubConfig,
    pub analytics_enabled: bool,
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub worktree_sessions_dir: Option<String>,
    pub last_app_version: Option<String>,
    pub show_release_notes: bool,
    #[serde(default)]
    pub language: UiLanguage,
    #[serde(default = "default_git_branch_prefix")]
    pub git_branch_prefix: String,
    #[serde(default)]
    pub showcases: ShowcaseState,
    #[serde(default = "default_pr_auto_description_enabled")]
    pub pr_auto_description_enabled: bool,
    #[serde(default)]
    pub pr_auto_description_prompt: Option<String>,
    #[serde(default)]
    pub beta_workspaces: bool,
    #[serde(default)]
    pub beta_workspaces_invitation_sent: bool,
    #[serde(default = "default_commit_reminder_enabled")]
    pub commit_reminder_enabled: bool,
    #[serde(default)]
    pub commit_reminder_prompt: Option<String>,
    #[serde(default)]
    pub send_message_shortcut: SendMessageShortcut,
    /// Chat presets configuration (member and team templates)
    #[serde(default = "default_chat_presets")]
    pub chat_presets: ChatPresetsConfig,
    /// Global chat bubble font size preference
    #[serde(
        default = "default_chat_bubble_font_size",
        deserialize_with = "deserialize_chat_bubble_font_size"
    )]
    pub chat_bubble_font_size: ChatBubbleFontSize,
    /// Chat compression configuration
    #[serde(default = "default_chat_compression")]
    pub chat_compression: ChatCompressionConfig,
    #[serde(default = "default_max_agent_chain_depth")]
    pub max_agent_chain_depth: u32,
}

impl Config {
    fn with_completed_chat_presets(mut self) -> Self {
        complete_chat_presets_with_builtins(&mut self.chat_presets);
        self
    }

    fn from_v8_config(old_config: v8::Config) -> Self {
        Self {
            config_version: "v9".to_string(),
            theme: old_config.theme,
            executor_profile: old_config.executor_profile,
            disclaimer_acknowledged: old_config.disclaimer_acknowledged,
            onboarding_acknowledged: old_config.onboarding_acknowledged,
            notifications: old_config.notifications,
            editor: old_config.editor,
            github: old_config.github,
            analytics_enabled: old_config.analytics_enabled,
            workspace_dir: old_config.workspace_dir,
            worktree_sessions_dir: None,
            last_app_version: old_config.last_app_version,
            show_release_notes: old_config.show_release_notes,
            language: old_config.language,
            git_branch_prefix: old_config.git_branch_prefix,
            showcases: old_config.showcases,
            pr_auto_description_enabled: old_config.pr_auto_description_enabled,
            pr_auto_description_prompt: old_config.pr_auto_description_prompt,
            beta_workspaces: old_config.beta_workspaces,
            beta_workspaces_invitation_sent: old_config.beta_workspaces_invitation_sent,
            commit_reminder_enabled: old_config.commit_reminder_enabled,
            commit_reminder_prompt: old_config.commit_reminder_prompt,
            send_message_shortcut: old_config.send_message_shortcut,
            chat_presets: default_chat_presets(),
            chat_bubble_font_size: default_chat_bubble_font_size(),
            chat_compression: ChatCompressionConfig::default(),
            max_agent_chain_depth: default_max_agent_chain_depth(),
        }
        .with_completed_chat_presets()
    }

    pub fn from_previous_version(raw_config: &str) -> Result<Self, Error> {
        let old_config = v8::Config::from(raw_config.to_string());
        Ok(Self::from_v8_config(old_config))
    }

    pub fn try_from_raw_config(raw_config: &str) -> Result<Self, Error> {
        match serde_json::from_str::<Config>(raw_config) {
            Ok(config) if config.config_version == "v9" => {
                return Ok(config.with_completed_chat_presets());
            }
            Ok(_) => {}
            Err(error) if raw_config_declares_v9(raw_config) => {
                return Err(error).context("failed to parse v9 config");
            }
            Err(_) => {}
        }

        Self::from_previous_version(raw_config).map(|config| {
            tracing::info!("Config upgraded to v9");
            config.with_completed_chat_presets()
        })
    }
}

impl From<String> for Config {
    fn from(raw_config: String) -> Self {
        match Self::try_from_raw_config(&raw_config) {
            Ok(config) => config,
            Err(e) => {
                tracing::warn!("Config load failed: {}, using default", e);
                Self::default().with_completed_chat_presets()
            }
        }
    }
}

fn raw_config_declares_v9(raw_config: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(raw_config)
        .ok()
        .and_then(|value| {
            value
                .get("config_version")
                .and_then(serde_json::Value::as_str)
                .map(|version| version == "v9")
        })
        .unwrap_or(false)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: "v9".to_string(),
            theme: ThemeMode::Light,
            executor_profile: ExecutorProfileId::new(BaseCodingAgent::OpenTeamsCli),
            disclaimer_acknowledged: false,
            onboarding_acknowledged: false,
            notifications: NotificationConfig::default(),
            editor: EditorConfig::default(),
            github: GitHubConfig::default(),
            analytics_enabled: true,
            workspace_dir: None,
            worktree_sessions_dir: None,
            last_app_version: None,
            show_release_notes: false,
            language: UiLanguage::default(),
            git_branch_prefix: default_git_branch_prefix(),
            showcases: ShowcaseState::default(),
            pr_auto_description_enabled: true,
            pr_auto_description_prompt: None,
            beta_workspaces: false,
            beta_workspaces_invitation_sent: false,
            commit_reminder_enabled: true,
            commit_reminder_prompt: None,
            send_message_shortcut: SendMessageShortcut::default(),
            chat_presets: default_chat_presets(),
            chat_bubble_font_size: default_chat_bubble_font_size(),
            chat_compression: ChatCompressionConfig::default(),
            max_agent_chain_depth: default_max_agent_chain_depth(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use utils::path::home_directory;

    use super::*;

    #[test]
    fn complete_chat_presets_clears_legacy_default_team_protocol() {
        let mut chat_presets = default_chat_presets();
        chat_presets.team_protocol = Some(PresetLoader::load_team_protocol());

        complete_chat_presets_with_builtins(&mut chat_presets);

        assert_eq!(chat_presets.team_protocol.as_deref(), Some(""));
    }

    #[test]
    fn complete_chat_presets_refreshes_builtin_workspace_path() {
        let mut chat_presets = default_chat_presets();
        let expected_workspace = home_directory().to_string_lossy().to_string();

        let builtin = chat_presets
            .members
            .iter_mut()
            .find(|preset| preset.id == "backend_engineer")
            .expect("backend preset should exist");
        builtin.default_workspace_path = Some("backend".to_string());
        builtin.selected_skill_ids = vec![" skill_b ".to_string(), "skill_a".to_string()];

        complete_chat_presets_with_builtins(&mut chat_presets);

        let builtin = chat_presets
            .members
            .iter()
            .find(|preset| preset.id == "backend_engineer")
            .expect("backend preset should exist");
        assert_eq!(
            builtin.default_workspace_path.as_deref(),
            Some(expected_workspace.as_str())
        );
        assert!(builtin.selected_skill_ids.is_empty());
    }

    #[test]
    fn complete_chat_presets_refreshes_builtin_catalog_fields() {
        let mut chat_presets = default_chat_presets();
        let builtin = chat_presets
            .members
            .iter_mut()
            .find(|preset| preset.id == "frontend_engineer")
            .expect("frontend preset should exist");
        builtin.name = "Custom Frontend".to_string();
        builtin.runner_type = Some("CLAUDE_CODE".to_string());
        builtin.recommended_model = Some("gpt-5.4".to_string());
        builtin.system_prompt = "old prompt".to_string();

        complete_chat_presets_with_builtins(&mut chat_presets);

        let builtin = chat_presets
            .members
            .iter()
            .find(|preset| preset.id == "frontend_engineer")
            .expect("frontend preset should exist");
        assert_eq!(builtin.name, "frontend");
        assert_eq!(builtin.runner_type.as_deref(), Some("CODEX"));
        assert_eq!(builtin.recommended_model.as_deref(), Some("gpt-6-astra"));
        assert_ne!(builtin.system_prompt, "old prompt");
    }

    #[test]
    fn complete_chat_presets_refreshes_custom_workspace_path() {
        let mut chat_presets = default_chat_presets();
        let expected_workspace = home_directory().to_string_lossy().to_string();

        chat_presets.members.push(ChatMemberPreset {
            id: "custom_member".to_string(),
            name: "Custom Member".to_string(),
            description: "Custom member".to_string(),
            runner_type: None,
            recommended_model: None,
            execution_config: None,
            system_prompt: "Prompt".to_string(),
            default_workspace_path: Some("E:/workspace/custom".to_string()),
            selected_skill_ids: vec![],
            tools_enabled: serde_json::json!({}),
            is_builtin: false,
            enabled: true,
        });

        complete_chat_presets_with_builtins(&mut chat_presets);

        let custom = chat_presets
            .members
            .iter()
            .find(|preset| preset.id == "custom_member")
            .expect("custom preset should exist");
        assert_eq!(custom.name, "CustomMember");
        assert_eq!(
            custom.default_workspace_path.as_deref(),
            Some(expected_workspace.as_str())
        );
    }

    #[test]
    fn team_preset_deserializes_missing_team_protocol() {
        let preset: ChatTeamPreset = serde_json::from_value(json!({
            "id": "custom_team",
            "name": "Custom Team",
            "description": "Custom description",
            "members": [{
                "id": "lead",
                "name": "Lead",
                "description": "Lead member",
                "system_prompt": "You are the lead.",
                "is_builtin": false,
                "enabled": true
            }],
            "is_builtin": false,
            "enabled": true
        }))
        .expect("team preset should deserialize");

        assert_eq!(preset.team_protocol, "");
        assert_eq!(preset.members.len(), 1);
        assert!(preset.workflow_steps.is_empty());
    }

    #[test]
    fn chat_presets_config_migrates_legacy_member_ids_to_embedded_members() {
        let raw = json!({
            "members": [
                {
                    "id": "lead",
                    "name": "Lead",
                    "description": "Lead member",
                    "system_prompt": "You are the lead.",
                    "is_builtin": false,
                    "enabled": true
                },
                {
                    "id": "backend",
                    "name": "Backend",
                    "description": "Backend member",
                    "system_prompt": "You are backend.",
                    "is_builtin": false,
                    "enabled": true
                }
            ],
            "teams": [
                {
                    "id": "custom_team",
                    "name": "Custom Team",
                    "description": "Custom description",
                    "member_ids": ["lead", "backend"],
                    "lead_member_id": "lead",
                    "is_builtin": false,
                    "enabled": true
                }
            ]
        });

        let config: ChatPresetsConfig =
            serde_json::from_value(raw).expect("legacy config should migrate");

        let team = &config.teams[0];
        assert_eq!(team.members.len(), 2);
        assert_eq!(team.members[0].id, "lead");
        assert_eq!(team.members[1].id, "backend");
        assert_eq!(team.lead_member_id.as_deref(), Some("lead"));
        assert!(
            team.members
                .iter()
                .all(|member| member.execution_config.is_none())
        );
    }

    #[test]
    fn config_try_from_raw_v9_migrates_legacy_member_ids_and_serializes_aggregate_teams() {
        let mut raw_config =
            serde_json::to_value(Config::default()).expect("serialize default config");
        raw_config["chat_presets"] = json!({
            "members": [
                {
                    "id": "legacy_lead",
                    "name": "LegacyLead",
                    "description": "Leads the migrated team",
                    "system_prompt": "Lead the migrated team.",
                    "selected_skill_ids": ["planning"],
                    "tools_enabled": { "mcpServers": { "filesystem": true } },
                    "execution_config": {
                        "runner_type": "CODEX",
                        "model_name": "fake-model",
                        "thinking_effort": "high",
                        "model_variant": "fake-variant",
                        "acp": {
                            "access_mode": "workspace_only",
                            "approval_mode": "auto_reject",
                            "additional_directories": []
                        },
                        "mcp": {
                            "mcpServers": {
                                "fake-local": {
                                    "command": "fake-mcp-server",
                                    "env": { "API_TOKEN": "fake-template-token" }
                                }
                            }
                        }
                    },
                    "is_builtin": false,
                    "enabled": true
                },
                {
                    "id": "legacy_reviewer",
                    "name": "LegacyReviewer",
                    "description": "Reviews the migrated team",
                    "system_prompt": "Review migrated work.",
                    "selected_skill_ids": ["review"],
                    "tools_enabled": { "mcpServers": { "browser": true } },
                    "is_builtin": false,
                    "enabled": true
                }
            ],
            "teams": [
                {
                    "id": "legacy_team",
                    "name": "Legacy Team",
                    "description": "Uses member_ids before migration",
                    "member_ids": ["legacy_lead", "legacy_reviewer"],
                    "lead_member_id": "legacy_lead",
                    "workflow_steps": [
                        { "title": "Plan", "description": "Migrate safely." }
                    ],
                    "team_protocol": "Coordinate migrated work.",
                    "is_builtin": false,
                    "enabled": true
                }
            ]
        });

        let mut config = Config::try_from_raw_config(&raw_config.to_string())
            .expect("legacy v9 config should migrate");
        let team = config
            .chat_presets
            .teams
            .iter()
            .find(|team| team.id == "legacy_team")
            .expect("legacy team should remain after completion");

        assert_eq!(team.members.len(), 2);
        assert_eq!(team.lead_member_id.as_deref(), Some("legacy_lead"));
        assert_eq!(team.members[0].system_prompt, "Lead the migrated team.");
        assert_eq!(team.members[1].selected_skill_ids, vec!["review"]);
        assert_eq!(
            team.members[0].tools_enabled,
            json!({ "mcpServers": { "filesystem": true } })
        );
        let serialized_member =
            serde_json::to_value(&team.members[0]).expect("serialize migrated member");
        assert_eq!(
            serialized_member["execution_config"],
            json!({
                "runner_type": "CODEX",
                "model_name": "fake-model",
                "thinking_effort": "high",
                "model_variant": "fake-variant",
                "acp": {
                    "access_mode": "workspace_only",
                    "approval_mode": "auto_reject",
                    "additional_directories": []
                },
                "mcp": {
                    "mcpServers": {
                        "fake-local": {
                            "command": "fake-mcp-server",
                            "env": { "API_TOKEN": "fake-template-token" }
                        }
                    }
                }
            })
        );

        let serialized = serde_json::to_value(&config).expect("serialize migrated config");
        let serialized_team = serialized["chat_presets"]["teams"]
            .as_array()
            .expect("teams should serialize as an array")
            .iter()
            .find(|team| team["id"] == "legacy_team")
            .expect("legacy team should serialize");
        assert!(serialized_team.get("member_ids").is_none());
        assert_eq!(serialized_team["members"][0]["id"], "legacy_lead");
        assert_eq!(
            serialized_team["members"][1]["tools_enabled"]["mcpServers"]["browser"],
            true
        );
        assert_eq!(
            serialized_team["members"][0]["execution_config"],
            serialized_member["execution_config"]
        );

        let source_execution_config = config
            .chat_presets
            .members
            .iter()
            .find(|member| member.id == "legacy_lead")
            .and_then(|member| member.execution_config.clone())
            .expect("legacy lookup source should keep execution config");
        config
            .chat_presets
            .teams
            .iter_mut()
            .find(|team| team.id == "legacy_team")
            .expect("legacy team should remain")
            .members[0]
            .execution_config
            .as_mut()
            .expect("embedded member should keep execution config")
            .thinking_effort = Some("changed-after-migration".to_string());
        assert_eq!(
            config
                .chat_presets
                .members
                .iter()
                .find(|member| member.id == "legacy_lead")
                .and_then(|member| member.execution_config.as_ref()),
            Some(&source_execution_config),
            "legacy member_ids lookup must deep-clone embedded member execution config"
        );
    }

    #[test]
    fn chat_presets_config_returns_diagnostic_error_for_dangling_member_ids() {
        let raw = json!({
            "members": [],
            "teams": [
                {
                    "id": "custom_team",
                    "name": "Custom Team",
                    "description": "Custom description",
                    "member_ids": ["missing_member"],
                    "is_builtin": false,
                    "enabled": true
                }
            ]
        });

        let error = serde_json::from_value::<ChatPresetsConfig>(raw)
            .expect_err("dangling member reference should fail");
        let message = format!("{error}");
        assert!(
            message.contains("missing_member"),
            "error should mention missing member id: {message}"
        );
        assert!(
            message.contains("custom_team"),
            "error should mention team id: {message}"
        );
    }

    #[test]
    fn chat_presets_config_prefers_embedded_members_over_legacy_member_ids() {
        let raw = json!({
            "members": [
                {
                    "id": "global_lead",
                    "name": "GlobalLead",
                    "description": "global",
                    "system_prompt": "global prompt",
                    "execution_config": {
                        "model_name": "global-model",
                        "mcp": { "mcpServers": {} }
                    },
                    "is_builtin": false,
                    "enabled": true
                }
            ],
            "teams": [
                {
                    "id": "custom_team",
                    "name": "Custom Team",
                    "description": "Custom description",
                    "member_ids": ["global_lead"],
                    "members": [{
                        "id": "embedded_lead",
                        "name": "EmbeddedLead",
                        "description": "embedded",
                        "system_prompt": "embedded prompt",
                        "execution_config": {
                            "model_name": "embedded-model",
                            "mcp": {
                                "mcpServers": {
                                    "fake-local": {
                                        "command": "fake-mcp-server",
                                        "env": { "API_TOKEN": "fake-embedded-token" }
                                    }
                                }
                            }
                        },
                        "is_builtin": false,
                        "enabled": true
                    }],
                    "is_builtin": false,
                    "enabled": true
                }
            ]
        });

        let config: ChatPresetsConfig =
            serde_json::from_value(raw).expect("aggregate config should deserialize");

        assert_eq!(config.teams[0].members.len(), 1);
        assert_eq!(config.teams[0].members[0].id, "embedded_lead");
        let execution_config = config.teams[0].members[0]
            .execution_config
            .as_ref()
            .expect("embedded execution config should win");
        assert_eq!(
            execution_config.model_name.as_deref(),
            Some("embedded-model")
        );
        assert!(execution_config.mcp.is_some());
    }

    #[test]
    fn chat_presets_config_rejects_dangling_lead_member_id() {
        let raw = json!({
            "members": [
                {
                    "id": "lead",
                    "name": "Lead",
                    "description": "Lead member",
                    "system_prompt": "You are the lead.",
                    "is_builtin": false,
                    "enabled": true
                }
            ],
            "teams": [
                {
                    "id": "custom_team",
                    "name": "Custom Team",
                    "description": "Custom description",
                    "member_ids": ["lead"],
                    "lead_member_id": "missing_lead",
                    "is_builtin": false,
                    "enabled": true
                }
            ]
        });

        let error = serde_json::from_value::<ChatPresetsConfig>(raw)
            .expect_err("dangling lead_member_id should fail");
        let message = format!("{error}");
        assert!(
            message.contains("missing_lead"),
            "error should mention dangling lead id: {message}"
        );
        assert!(
            message.contains("custom_team"),
            "error should mention team id: {message}"
        );
    }

    #[test]
    fn config_try_from_raw_config_returns_diagnostic_error_for_invalid_v9_chat_presets() {
        let mut raw_config =
            serde_json::to_value(Config::default()).expect("serialize default config");
        raw_config["chat_presets"] = json!({
            "members": [],
            "teams": [
                {
                    "id": "custom_team",
                    "name": "Custom Team",
                    "description": "Custom description",
                    "member_ids": ["missing_member"],
                    "is_builtin": false,
                    "enabled": true
                }
            ]
        });

        let error = Config::try_from_raw_config(&raw_config.to_string())
            .expect_err("invalid v9 chat presets should return a diagnostic error");
        let message = format!("{error:#}");

        assert!(
            message.contains("failed to parse v9 config"),
            "error should identify v9 parse failure: {message}"
        );
        assert!(
            message.contains("custom_team"),
            "error should mention the team id: {message}"
        );
        assert!(
            message.contains("missing_member"),
            "error should mention the missing member id: {message}"
        );
    }

    #[test]
    fn chat_team_preset_round_trips_workflow_steps() {
        let preset = ChatTeamPreset {
            id: "custom_team".to_string(),
            name: "Custom Team".to_string(),
            description: "Custom description".to_string(),
            members: vec![],
            lead_member_id: None,
            workflow_steps: vec![
                ChatWorkflowStep {
                    title: "Plan".to_string(),
                    description: "Clarify scope.".to_string(),
                },
                ChatWorkflowStep {
                    title: "Build".to_string(),
                    description: String::new(),
                },
            ],
            team_protocol: "Coordinate tightly.".to_string(),
            is_builtin: false,
            enabled: true,
            tier: ChatTeamTemplateTier::Standard,
        };

        let serialized = serde_json::to_string(&preset).expect("serialize");
        let deserialized: ChatTeamPreset = serde_json::from_str(&serialized).expect("deserialize");

        assert_eq!(preset, deserialized);
        assert_eq!(deserialized.workflow_steps.len(), 2);
        assert_eq!(deserialized.workflow_steps[0].title, "Plan");
    }

    #[test]
    fn chat_team_preset_tier_defaults_to_standard_and_uses_snake_case() {
        let preset: ChatTeamPreset = serde_json::from_value(json!({
            "id": "legacy_custom",
            "name": "Legacy Custom",
            "description": "Legacy custom team",
            "members": [],
            "is_builtin": false,
            "enabled": true
        }))
        .expect("missing tier should deserialize");
        assert_eq!(preset.tier, ChatTeamTemplateTier::Standard);

        let advanced: ChatTeamPreset = serde_json::from_value(json!({
            "id": "advanced_custom",
            "name": "Advanced Custom",
            "description": "Advanced custom team",
            "members": [],
            "is_builtin": false,
            "enabled": true,
            "tier": "advanced"
        }))
        .expect("advanced tier should deserialize");
        let serialized = serde_json::to_value(&advanced).expect("serialize advanced");

        assert_eq!(advanced.tier, ChatTeamTemplateTier::Advanced);
        assert_eq!(serialized["tier"], "advanced");
    }

    #[test]
    fn default_chat_presets_loads_builtin_team_metadata_from_markdown() {
        let chat_presets = default_chat_presets();

        let planner = chat_presets
            .members
            .iter()
            .find(|preset| preset.id == "coordinator_pmo")
            .expect("planner preset should exist");
        assert_eq!(planner.runner_type.as_deref(), Some("OPENCODE"));
        assert_eq!(planner.recommended_model.as_deref(), Some("glm-5"));

        let fullstack = chat_presets
            .teams
            .iter()
            .find(|preset| preset.id == "fullstack_delivery_team")
            .expect("fullstack team should exist");

        assert_eq!(fullstack.name, "Full-stack Delivery Team");
        assert_eq!(
            fullstack.description,
            "Planner-led web delivery across design, frontend, backend, QA, and review."
        );
        let fullstack_member_ids = fullstack
            .members
            .iter()
            .map(|member| member.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            fullstack_member_ids,
            vec![
                "coordinator_pmo".to_string(),
                "backend_engineer".to_string(),
                "frontend_engineer".to_string(),
                "qa_tester".to_string(),
            ]
        );
        assert!(!fullstack.team_protocol.trim().is_empty());
        assert_eq!(fullstack.tier, ChatTeamTemplateTier::Standard);
        let advanced = chat_presets
            .teams
            .iter()
            .filter(|preset| preset.tier == ChatTeamTemplateTier::Advanced)
            .map(|preset| preset.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            advanced,
            vec!["advanced-growth-ops", "advanced-release-command"]
        );
        assert!(!fullstack.team_protocol.contains("`@`"));
    }

    #[test]
    fn config_defaults_chat_bubble_font_size_to_px14_when_missing() {
        let mut raw_config =
            serde_json::to_value(Config::default()).expect("serialize default config");
        raw_config
            .as_object_mut()
            .expect("config should serialize as object")
            .remove("chat_bubble_font_size");

        let config = Config::from(raw_config.to_string());

        assert_eq!(config.chat_bubble_font_size, ChatBubbleFontSize::Px14);
    }

    #[test]
    fn config_deserializes_legacy_chat_bubble_font_size_aliases() {
        let mut small_raw =
            serde_json::to_value(Config::default()).expect("serialize default config");
        small_raw
            .as_object_mut()
            .expect("config should serialize as object")
            .insert("chat_bubble_font_size".to_string(), json!("small"));

        let mut medium_raw =
            serde_json::to_value(Config::default()).expect("serialize default config");
        medium_raw
            .as_object_mut()
            .expect("config should serialize as object")
            .insert("chat_bubble_font_size".to_string(), json!("medium"));

        let mut large_raw =
            serde_json::to_value(Config::default()).expect("serialize default config");
        large_raw
            .as_object_mut()
            .expect("config should serialize as object")
            .insert("chat_bubble_font_size".to_string(), json!("large"));

        let small: Config = serde_json::from_value(small_raw)
            .unwrap_or_else(|error| panic!("small alias should deserialize: {error}"));
        let medium: Config = serde_json::from_value(medium_raw)
            .unwrap_or_else(|error| panic!("medium alias should deserialize: {error}"));
        let large: Config = serde_json::from_value(large_raw)
            .unwrap_or_else(|error| panic!("large alias should deserialize: {error}"));

        assert_eq!(small.chat_bubble_font_size, ChatBubbleFontSize::Px12);
        assert_eq!(medium.chat_bubble_font_size, ChatBubbleFontSize::Px14);
        assert_eq!(large.chat_bubble_font_size, ChatBubbleFontSize::Px16);
    }

    #[test]
    fn complete_chat_presets_refreshes_builtin_team_aggregate_fields() {
        let mut chat_presets = default_chat_presets();
        let custom_team = ChatTeamPreset {
            id: "custom_team".to_string(),
            name: "Custom Team".to_string(),
            description: "Custom description".to_string(),
            members: vec![ChatMemberPreset {
                id: "custom_member".to_string(),
                name: "custom_member".to_string(),
                description: "Custom member".to_string(),
                runner_type: None,
                recommended_model: None,
                execution_config: None,
                system_prompt: "Custom prompt".to_string(),
                default_workspace_path: None,
                selected_skill_ids: vec![],
                tools_enabled: serde_json::json!({}),
                is_builtin: false,
                enabled: true,
            }],
            lead_member_id: Some("custom_member".to_string()),
            workflow_steps: vec![ChatWorkflowStep {
                title: "Custom".to_string(),
                description: "Keep this custom workflow.".to_string(),
            }],
            team_protocol: "Custom team protocol".to_string(),
            is_builtin: false,
            enabled: true,
            tier: ChatTeamTemplateTier::Standard,
        };
        chat_presets.teams.push(custom_team.clone());

        let team = chat_presets
            .teams
            .iter_mut()
            .find(|preset| preset.id == "rapid_bugfix_team")
            .expect("rapid bugfix team should exist");
        team.name = "Stale Built-in".to_string();
        team.members = vec![ChatMemberPreset {
            id: "stale_member".to_string(),
            name: "stale_member".to_string(),
            description: "Stale member".to_string(),
            runner_type: None,
            recommended_model: None,
            execution_config: None,
            system_prompt: "Stale prompt".to_string(),
            default_workspace_path: None,
            selected_skill_ids: vec![],
            tools_enabled: serde_json::json!({}),
            is_builtin: true,
            enabled: true,
        }];
        team.workflow_steps = vec![ChatWorkflowStep {
            title: "Stale".to_string(),
            description: "Stale workflow.".to_string(),
        }];
        team.team_protocol = "Stale rapid response protocol".to_string();

        complete_chat_presets_with_builtins(&mut chat_presets);

        let defaults = default_chat_presets();
        let default_team = defaults
            .teams
            .iter()
            .find(|preset| preset.id == "rapid_bugfix_team")
            .expect("default rapid bugfix team should exist");
        let team = chat_presets
            .teams
            .iter()
            .find(|preset| preset.id == "rapid_bugfix_team")
            .expect("rapid bugfix team should exist");
        assert_eq!(team.name, default_team.name);
        assert_eq!(team.members, default_team.members);
        assert_eq!(team.workflow_steps, default_team.workflow_steps);
        assert_eq!(team.team_protocol, default_team.team_protocol);
        assert!(chat_presets.teams.iter().any(|team| team == &custom_team));
    }
}
