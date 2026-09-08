use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Error;
use executors::{executors::BaseCodingAgent, profile::ExecutorProfileId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utils::{path::home_directory, text::sanitize_member_handle};
pub use v9::{
    ChatBubbleFontSize, ChatCompressionConfig, ChatMemberPreset, ChatPresetsConfig, ChatTeamPreset,
    ChatTeamTemplateTier, ChatWorkflowStep, EditorConfig, EditorType, GitHubConfig,
    NotificationConfig, NotificationInboxSource, NotificationInboxSourcesConfig,
    SendMessageShortcut, ShowcaseState, SoundFile, ThemeMode, UiLanguage,
};

use crate::services::config::{ConfigError, preset_loader::PresetLoader, versions::v9};

pub const KEYBOARD_SHORTCUTS_SCHEMA_VERSION: u32 = 1;

fn default_git_branch_prefix() -> String {
    "vk".to_string()
}

fn default_pr_auto_description_enabled() -> bool {
    true
}

fn default_commit_reminder_enabled() -> bool {
    true
}

fn default_error_reporting_enabled() -> bool {
    true
}

fn default_max_agent_chain_depth() -> u32 {
    8
}

fn default_chat_bubble_font_size() -> ChatBubbleFontSize {
    ChatBubbleFontSize::default()
}

fn deserialize_chat_bubble_font_size<'de, D>(
    deserializer: D,
) -> Result<ChatBubbleFontSize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    ChatBubbleFontSize::deserialize(deserializer)
}

fn default_chat_compression() -> ChatCompressionConfig {
    ChatCompressionConfig::default()
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

fn default_keyboard_shortcuts_schema_version() -> u32 {
    KEYBOARD_SHORTCUTS_SCHEMA_VERSION
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct KeyboardShortcutsConfig {
    #[serde(default = "default_keyboard_shortcuts_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub platform_overrides: BTreeMap<String, BTreeMap<String, KeyboardShortcutOverride>>,
}

impl Default for KeyboardShortcutsConfig {
    fn default() -> Self {
        Self {
            schema_version: KEYBOARD_SHORTCUTS_SCHEMA_VERSION,
            platform_overrides: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct KeyboardShortcutBinding {
    pub sequence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(untagged)]
pub enum KeyboardShortcutOverride {
    Binding(KeyboardShortcutBinding),
    Invalid(serde_json::Value),
}

impl KeyboardShortcutsConfig {
    pub fn validate_loaded_schema(&self) -> Result<(), ConfigError> {
        if self.schema_version != KEYBOARD_SHORTCUTS_SCHEMA_VERSION {
            return Err(ConfigError::ValidationError(format!(
                "Unsupported keyboard shortcut schema version: {}",
                self.schema_version
            )));
        }
        Ok(())
    }

    pub fn validate_for_save(&self) -> Result<(), ConfigError> {
        self.validate_for_save_inner(None)
    }

    pub fn validate_for_save_against(
        &self,
        previous: &KeyboardShortcutsConfig,
    ) -> Result<(), ConfigError> {
        self.validate_for_save_inner(Some(previous))
    }

    fn validate_for_save_inner(
        &self,
        previous: Option<&KeyboardShortcutsConfig>,
    ) -> Result<(), ConfigError> {
        self.validate_loaded_schema()?;
        for (platform, overrides) in &self.platform_overrides {
            validate_platform(platform)?;
            for (command_id, value) in overrides {
                validate_command_id(command_id)?;
                match value {
                    KeyboardShortcutOverride::Binding(binding) => {
                        validate_sequence(&binding.sequence)?;
                    }
                    KeyboardShortcutOverride::Invalid(invalid) => {
                        let unchanged_quarantined_value = previous
                            .and_then(|config| config.platform_overrides.get(platform))
                            .and_then(|entries| entries.get(command_id))
                            .is_some_and(|old| {
                                matches!(
                                    old,
                                    KeyboardShortcutOverride::Invalid(old_invalid)
                                        if old_invalid == invalid
                                )
                            });
                        if !unchanged_quarantined_value {
                            return Err(ConfigError::ValidationError(format!(
                                "Invalid keyboard shortcut override: {platform}.{command_id}"
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn validate_platform(platform: &str) -> Result<(), ConfigError> {
    match platform {
        "macos" | "windows" | "linux" => Ok(()),
        _ => Err(ConfigError::ValidationError(format!(
            "Unknown keyboard shortcut platform: {platform}"
        ))),
    }
}

fn validate_command_id(command_id: &str) -> Result<(), ConfigError> {
    let valid = command_id.contains('.')
        && command_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(ConfigError::ValidationError(format!(
            "Invalid keyboard shortcut command id: {command_id}"
        )))
    }
}

fn validate_sequence(sequence: &[String]) -> Result<(), ConfigError> {
    if sequence.len() > 2 {
        return Err(ConfigError::ValidationError(
            "Keyboard shortcut sequence may contain at most two strokes".to_string(),
        ));
    }
    sequence
        .iter()
        .try_for_each(|stroke| validate_stroke(stroke))
}

fn validate_stroke(stroke: &str) -> Result<(), ConfigError> {
    const MODIFIERS: [&str; 4] = ["ctrl", "meta", "alt", "shift"];
    const NAMED_KEYS: [&str; 13] = [
        "enter",
        "tab",
        "escape",
        "space",
        "arrowup",
        "arrowdown",
        "arrowleft",
        "arrowright",
        "comma",
        "delete",
        "backspace",
        "plus",
        "minus",
    ];

    if stroke.is_empty() || stroke != stroke.to_ascii_lowercase() {
        return Err(ConfigError::ValidationError(format!(
            "Keyboard shortcut stroke is not canonical: {stroke}"
        )));
    }
    let tokens = stroke.split('+').collect::<Vec<_>>();
    if tokens.iter().any(|token| token.is_empty()) {
        return Err(ConfigError::ValidationError(format!(
            "Keyboard shortcut stroke contains an empty token: {stroke}"
        )));
    }

    let mut last_modifier_rank = None;
    let mut key = None;
    for token in tokens {
        if let Some(rank) = MODIFIERS.iter().position(|modifier| *modifier == token) {
            if key.is_some() || last_modifier_rank.is_some_and(|previous| previous >= rank) {
                return Err(ConfigError::ValidationError(format!(
                    "Keyboard shortcut modifiers are duplicated or out of order: {stroke}"
                )));
            }
            last_modifier_rank = Some(rank);
            continue;
        }

        let printable = token.len() == 1 && token.as_bytes()[0].is_ascii_graphic() && token != ",";
        if key.is_some() || (!printable && !NAMED_KEYS.contains(&token)) {
            return Err(ConfigError::ValidationError(format!(
                "Keyboard shortcut stroke must contain exactly one supported key: {stroke}"
            )));
        }
        key = Some(token);
    }

    if key.is_none() {
        return Err(ConfigError::ValidationError(format!(
            "Keyboard shortcut stroke is missing a key: {stroke}"
        )));
    }
    Ok(())
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
    #[serde(default = "default_error_reporting_enabled")]
    pub error_reporting_enabled: bool,
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
    #[serde(default = "default_chat_presets")]
    pub chat_presets: ChatPresetsConfig,
    #[serde(
        default = "default_chat_bubble_font_size",
        deserialize_with = "deserialize_chat_bubble_font_size"
    )]
    pub chat_bubble_font_size: ChatBubbleFontSize,
    #[serde(default = "default_chat_compression")]
    pub chat_compression: ChatCompressionConfig,
    #[serde(default = "default_max_agent_chain_depth")]
    pub max_agent_chain_depth: u32,
    #[serde(default)]
    pub keyboard_shortcuts: KeyboardShortcutsConfig,
}

impl Config {
    fn with_completed_chat_presets(mut self) -> Self {
        complete_chat_presets_with_builtins(&mut self.chat_presets);
        self
    }

    fn from_v9_config(old: v9::Config) -> Self {
        Self {
            config_version: "v10".to_string(),
            theme: old.theme,
            executor_profile: old.executor_profile,
            disclaimer_acknowledged: old.disclaimer_acknowledged,
            onboarding_acknowledged: old.onboarding_acknowledged,
            notifications: old.notifications,
            editor: old.editor,
            github: old.github,
            analytics_enabled: old.analytics_enabled,
            error_reporting_enabled: true,
            workspace_dir: old.workspace_dir,
            worktree_sessions_dir: old.worktree_sessions_dir,
            last_app_version: old.last_app_version,
            show_release_notes: old.show_release_notes,
            language: old.language,
            git_branch_prefix: old.git_branch_prefix,
            showcases: old.showcases,
            pr_auto_description_enabled: old.pr_auto_description_enabled,
            pr_auto_description_prompt: old.pr_auto_description_prompt,
            beta_workspaces: old.beta_workspaces,
            beta_workspaces_invitation_sent: old.beta_workspaces_invitation_sent,
            commit_reminder_enabled: old.commit_reminder_enabled,
            commit_reminder_prompt: old.commit_reminder_prompt,
            send_message_shortcut: old.send_message_shortcut,
            chat_presets: old.chat_presets,
            chat_bubble_font_size: old.chat_bubble_font_size,
            chat_compression: old.chat_compression,
            max_agent_chain_depth: old.max_agent_chain_depth,
            keyboard_shortcuts: KeyboardShortcutsConfig::default(),
        }
    }

    pub fn from_previous_version(raw: &str) -> Result<Self, Error> {
        Ok(Self::from_v9_config(v9::Config::try_from_raw_config(raw)?))
    }

    pub fn try_from_raw_config(raw: &str) -> Result<Self, Error> {
        match serde_json::from_str::<Self>(raw) {
            Ok(config) if config.config_version == "v10" => {
                config.keyboard_shortcuts.validate_loaded_schema()?;
                Ok(config.with_completed_chat_presets())
            }
            Err(error) if raw_config_declares_v10(raw) => Err(error.into()),
            _ => Self::from_previous_version(raw),
        }
    }
}

impl From<String> for Config {
    fn from(raw_config: String) -> Self {
        match Self::try_from_raw_config(&raw_config) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!("Config load failed: {}, using default", error);
                Self::default().with_completed_chat_presets()
            }
        }
    }
}

fn raw_config_declares_v10(raw: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .get("config_version")
                .and_then(serde_json::Value::as_str)
                .map(|version| version == "v10")
        })
        .unwrap_or(false)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: "v10".to_string(),
            theme: ThemeMode::System,
            executor_profile: ExecutorProfileId::new(BaseCodingAgent::OpenTeamsCli),
            disclaimer_acknowledged: false,
            onboarding_acknowledged: false,
            notifications: NotificationConfig::default(),
            editor: EditorConfig::default(),
            github: GitHubConfig::default(),
            analytics_enabled: true,
            error_reporting_enabled: true,
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
            keyboard_shortcuts: KeyboardShortcutsConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn new_config_follows_system_appearance_and_browser_language() {
        let config = serde_json::to_value(Config::default()).expect("serialize default config");

        assert_eq!(config["theme"], "SYSTEM");
        assert_eq!(config["language"], "BROWSER");
    }

    #[test]
    fn current_config_refreshes_builtin_model_recommendations() {
        let mut config = Config::default();
        let builtin = config
            .chat_presets
            .members
            .iter_mut()
            .find(|preset| preset.id == "frontend_engineer")
            .expect("frontend preset should exist");
        builtin.recommended_model = Some("gpt-5.2-codex".to_string());

        let config = config.with_completed_chat_presets();
        let builtin = config
            .chat_presets
            .members
            .iter()
            .find(|preset| preset.id == "frontend_engineer")
            .expect("frontend preset should exist");

        assert_eq!(builtin.recommended_model.as_deref(), Some("gpt-6-astra"));
    }
}
