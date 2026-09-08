use std::{
    collections::HashMap,
    fs,
    path::Path,
    str::FromStr,
    sync::{LazyLock, RwLock},
};

use convert_case::{Case, Casing};
use serde::{Deserialize, Deserializer, Serialize, de::Error as DeError};
use thiserror::Error;
use ts_rs::TS;

use crate::executors::{
    AvailabilityInfo, BaseCodingAgent, CodingAgent, StandardCodingAgentExecutor,
};

/// Return the canonical form for variant keys.
/// – "DEFAULT" is kept as-is  
/// – everything else is converted to SCREAMING_SNAKE_CASE
pub fn canonical_variant_key<S: AsRef<str>>(raw: S) -> String {
    let key = raw
        .as_ref()
        .strip_prefix("AUTO_MODEL_")
        .unwrap_or_else(|| raw.as_ref());
    if key.eq_ignore_ascii_case("DEFAULT") {
        "DEFAULT".to_string()
    } else if key.contains('/') {
        key.to_string()
    } else {
        // Convert to SCREAMING_SNAKE_CASE by first going to snake_case then uppercase
        key.to_case(Case::Snake).to_case(Case::ScreamingSnake)
    }
}

#[derive(Error, Debug)]
pub enum ProfileError {
    #[error("Built-in executor '{executor}' cannot be deleted")]
    CannotDeleteExecutor { executor: BaseCodingAgent },

    #[error("Built-in configuration '{executor}:{variant}' cannot be deleted")]
    CannotDeleteBuiltInConfig {
        executor: BaseCodingAgent,
        variant: String,
    },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error("No available executor profile")]
    NoAvailableExecutorProfile,
}

static EXECUTOR_PROFILES_CACHE: LazyLock<RwLock<ExecutorConfigs>> =
    LazyLock::new(|| RwLock::new(ExecutorConfigs::load()));

// New format default profiles (v3 - flattened)
const DEFAULT_PROFILES_JSON: &str = include_str!("../default_profiles.json");

// Executor-centric profile identifier
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, Hash, Eq)]
pub struct ExecutorProfileId {
    /// The executor type (e.g., "CLAUDE_CODE", "AMP")
    #[serde(alias = "profile", deserialize_with = "de_base_coding_agent_kebab")]
    // Backwards compatibility with ProfileVariantIds, esp stored in DB under ExecutorAction
    pub executor: BaseCodingAgent,
    /// Optional variant name (e.g., "PLAN", "ROUTER")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

// Convert legacy profile/executor names from kebab-case to SCREAMING_SNAKE_CASE, can be deleted 14 days from 3/9/25
fn de_base_coding_agent_kebab<'de, D>(de: D) -> Result<BaseCodingAgent, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(de)?;
    // kebab-case -> SCREAMING_SNAKE_CASE
    let norm = raw.replace('-', "_").to_ascii_uppercase();
    BaseCodingAgent::from_str(&norm)
        .map_err(|_| D::Error::custom(format!("unknown executor '{raw}' (normalized to '{norm}')")))
}

impl ExecutorProfileId {
    /// Create a new executor profile ID with default variant
    pub fn new(executor: BaseCodingAgent) -> Self {
        Self {
            executor,
            variant: None,
        }
    }

    /// Create a new executor profile ID with specific variant
    pub fn with_variant(executor: BaseCodingAgent, variant: String) -> Self {
        Self {
            executor,
            variant: Some(variant),
        }
    }

    /// Get cache key for this executor profile
    pub fn cache_key(&self) -> String {
        match &self.variant {
            Some(variant) => format!("{}:{}", self.executor, variant),
            None => self.executor.clone().to_string(),
        }
    }
}

impl std::fmt::Display for ExecutorProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.variant {
            Some(variant) => write!(f, "{}:{}", self.executor, variant),
            None => write!(f, "{}", self.executor),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct ExecutorConfig {
    #[serde(flatten)]
    pub configurations: HashMap<String, CodingAgent>,
}

impl ExecutorConfig {
    /// Get variant configuration by name, or None if not found
    pub fn get_variant(&self, variant: &str) -> Option<&CodingAgent> {
        self.configurations.get(variant)
    }

    /// Get the default configuration for this executor
    pub fn get_default(&self) -> Option<&CodingAgent> {
        self.configurations.get("DEFAULT")
    }

    /// Create a new executor profile with just a default configuration
    pub fn new_with_default(default_config: CodingAgent) -> Self {
        let mut configurations = HashMap::new();
        configurations.insert("DEFAULT".to_string(), default_config);
        Self { configurations }
    }

    /// Add or update a variant configuration
    pub fn set_variant(
        &mut self,
        variant_name: String,
        config: CodingAgent,
    ) -> Result<(), &'static str> {
        let key = canonical_variant_key(&variant_name);
        if key == "DEFAULT" {
            return Err(
                "Cannot override 'DEFAULT' variant using set_variant, use set_default instead",
            );
        }
        self.configurations.insert(key, config);
        Ok(())
    }

    /// Set the default configuration
    pub fn set_default(&mut self, config: CodingAgent) {
        self.configurations.insert("DEFAULT".to_string(), config);
    }

    /// Get all variant names (excluding "DEFAULT")
    pub fn variant_names(&self) -> Vec<&String> {
        self.configurations
            .keys()
            .filter(|k| *k != "DEFAULT")
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct ExecutorConfigs {
    pub executors: HashMap<BaseCodingAgent, ExecutorConfig>,
}

impl ExecutorConfigs {
    /// Normalise all variant keys in-place
    fn canonicalise(&mut self) {
        for profile in self.executors.values_mut() {
            let mut replacements = Vec::new();
            for key in profile.configurations.keys().cloned().collect::<Vec<_>>() {
                let canon = canonical_variant_key(&key);
                if canon != key {
                    replacements.push((key, canon));
                }
            }
            for (old, new) in replacements {
                if let Some(cfg) = profile.configurations.remove(&old) {
                    // If both lowercase and canonical forms existed, keep canonical one
                    profile.configurations.entry(new).or_insert(cfg);
                }
            }
        }
    }

    /// Get cached executor profiles
    pub fn get_cached() -> ExecutorConfigs {
        EXECUTOR_PROFILES_CACHE.read().unwrap().clone()
    }

    /// Reload executor profiles cache
    pub fn reload() {
        let mut cache = EXECUTOR_PROFILES_CACHE.write().unwrap();
        *cache = Self::load();
    }

    /// Load executor profiles from file or defaults
    pub fn load() -> Self {
        let profiles_path = workspace_utils::assets::profiles_path();

        // Load defaults first
        let mut defaults = Self::from_defaults();
        defaults.canonicalise();

        // Try to load user overrides
        let content = match fs::read_to_string(&profiles_path) {
            Ok(content) => content,
            Err(_) => {
                tracing::info!("No user profiles.json found, using defaults only");
                return defaults;
            }
        };

        // Parse user overrides
        match serde_json::from_str::<Self>(&content) {
            Ok(mut user_overrides) => {
                // tracing::info!("Loaded user profile overrides from profiles.json");
                user_overrides.canonicalise();
                if user_overrides.migrate_opencode_model_overrides(&defaults) {
                    match Self::write_profiles_file(&profiles_path, &user_overrides) {
                        Ok(()) => tracing::info!(
                            "Removed stale OpenCode model entries from user profiles.json"
                        ),
                        Err(err) => tracing::warn!(
                            "Failed to persist OpenCode profiles.json migration: {err}"
                        ),
                    }
                }
                Self::merge_with_defaults(defaults, user_overrides)
            }
            Err(e) => {
                tracing::error!(
                    "Failed to parse user profiles.json: {}, using defaults only",
                    e
                );
                defaults
            }
        }
    }

    fn migrate_opencode_model_overrides(&mut self, defaults: &Self) -> bool {
        let Some(default_profile) = defaults.executors.get(&BaseCodingAgent::Opencode) else {
            return false;
        };
        let Some(user_profile) = self.executors.get_mut(&BaseCodingAgent::Opencode) else {
            return false;
        };

        let original_len = user_profile.configurations.len();
        user_profile.configurations.retain(|variant, config| {
            default_profile.configurations.contains_key(variant)
                && matches!(config, CodingAgent::Opencode(_))
        });
        let mut changed = user_profile.configurations.len() != original_len;

        for config in user_profile.configurations.values_mut() {
            if let CodingAgent::Opencode(user_config) = config
                && user_config.model.take().is_some()
            {
                changed = true;
            }
        }

        if user_profile.configurations.is_empty() {
            self.executors.remove(&BaseCodingAgent::Opencode);
            changed = true;
        }

        changed
    }

    fn write_profiles_file(path: &Path, profiles: &Self) -> Result<(), ProfileError> {
        let content = serde_json::to_string_pretty(profiles)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Save user profile overrides to file (only saves what differs from defaults)
    pub fn save_overrides(&self) -> Result<(), ProfileError> {
        let profiles_path = workspace_utils::assets::profiles_path();
        let mut defaults = Self::from_defaults();
        defaults.canonicalise();

        // Canonicalise current config before computing overrides
        let mut self_clone = self.clone();
        self_clone.canonicalise();

        // Compute differences from defaults
        let overrides = Self::compute_overrides(&defaults, &self_clone)?;

        // Validate the merged result would be valid
        let merged = Self::merge_with_defaults(defaults, overrides.clone());
        Self::validate_merged(&merged)?;

        // Write overrides directly to file
        let content = serde_json::to_string_pretty(&overrides)?;
        fs::write(&profiles_path, content)?;

        tracing::info!("Saved profile overrides to {:?}", profiles_path);
        Ok(())
    }

    /// Deep merge defaults with user overrides
    fn merge_with_defaults(mut defaults: Self, overrides: Self) -> Self {
        for (executor_key, override_profile) in overrides.executors {
            match defaults.executors.get_mut(&executor_key) {
                Some(default_profile) => {
                    // Merge configurations (user configs override defaults, new ones are added)
                    for (config_name, config) in override_profile.configurations {
                        default_profile.configurations.insert(config_name, config);
                    }
                }
                None => {
                    // New executor, add completely
                    defaults.executors.insert(executor_key, override_profile);
                }
            }
        }
        defaults
    }

    /// Compute what overrides are needed to transform defaults into current config
    fn compute_overrides(defaults: &Self, current: &Self) -> Result<Self, ProfileError> {
        let mut overrides = Self {
            executors: HashMap::new(),
        };

        // Fast scan for any illegal deletions BEFORE allocating/cloning
        for (executor_key, default_profile) in &defaults.executors {
            // Check if executor was removed entirely
            if !current.executors.contains_key(executor_key) {
                return Err(ProfileError::CannotDeleteExecutor {
                    executor: *executor_key,
                });
            }

            let current_profile = &current.executors[executor_key];

            // Check if ANY built-in configuration was removed
            for config_name in default_profile.configurations.keys() {
                if !current_profile.configurations.contains_key(config_name) {
                    return Err(ProfileError::CannotDeleteBuiltInConfig {
                        executor: *executor_key,
                        variant: config_name.clone(),
                    });
                }
            }
        }

        for (executor_key, current_profile) in &current.executors {
            if let Some(default_profile) = defaults.executors.get(executor_key) {
                let mut override_configurations = HashMap::new();

                // Check each configuration in current profile
                for (config_name, current_config) in &current_profile.configurations {
                    if let Some(default_config) = default_profile.configurations.get(config_name) {
                        // Only include if different from default
                        if current_config != default_config {
                            override_configurations
                                .insert(config_name.clone(), current_config.clone());
                        }
                    } else {
                        // New configuration, always include
                        override_configurations.insert(config_name.clone(), current_config.clone());
                    }
                }

                // Only include executor if there are actual differences
                if !override_configurations.is_empty() {
                    overrides.executors.insert(
                        *executor_key,
                        ExecutorConfig {
                            configurations: override_configurations,
                        },
                    );
                }
            } else {
                // New executor, include completely
                overrides
                    .executors
                    .insert(*executor_key, current_profile.clone());
            }
        }

        Ok(overrides)
    }

    /// Validate that merged profiles are consistent and valid
    fn validate_merged(merged: &Self) -> Result<(), ProfileError> {
        for (executor_key, profile) in &merged.executors {
            // Ensure default configuration exists
            let default_config = profile.configurations.get("DEFAULT").ok_or_else(|| {
                ProfileError::Validation(format!(
                    "Executor '{executor_key}' is missing required 'default' configuration"
                ))
            })?;

            // Validate that the default agent type matches the executor key
            if BaseCodingAgent::from(default_config) != *executor_key {
                return Err(ProfileError::Validation(format!(
                    "Executor key '{executor_key}' does not match the agent variant '{default_config}'"
                )));
            }

            // Ensure configuration names don't conflict with reserved words
            for config_name in profile.configurations.keys() {
                if config_name.starts_with("__") {
                    return Err(ProfileError::Validation(format!(
                        "Configuration name '{config_name}' is reserved (starts with '__')"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Load from the new v3 defaults
    pub fn from_defaults() -> Self {
        serde_json::from_str(DEFAULT_PROFILES_JSON).unwrap_or_else(|e| {
            tracing::error!("Failed to parse embedded default_profiles.json: {}", e);
            panic!("Default profiles v3 JSON is invalid")
        })
    }

    pub fn get_coding_agent(&self, executor_profile_id: &ExecutorProfileId) -> Option<CodingAgent> {
        self.executors
            .get(&executor_profile_id.executor)
            .and_then(|executor| {
                executor.get_variant(
                    &executor_profile_id
                        .variant
                        .clone()
                        .unwrap_or("DEFAULT".to_string()),
                )
            })
            .cloned()
    }

    pub fn get_coding_agent_or_default(
        &self,
        executor_profile_id: &ExecutorProfileId,
    ) -> CodingAgent {
        #[cfg(feature = "qa-mode")]
        if executor_profile_id.executor == BaseCodingAgent::AcpQa {
            return CodingAgent::AcpQa(crate::executors::acp::AcpQaExecutor::from_qa_environment());
        }
        #[cfg(feature = "qa-mode")]
        if executor_profile_id.executor == BaseCodingAgent::Pi {
            return CodingAgent::Pi(crate::executors::pi::Pi::from_qa_environment());
        }
        self.get_coding_agent(executor_profile_id)
            .unwrap_or_else(|| {
                let mut default_executor_profile_id = executor_profile_id.clone();
                default_executor_profile_id.variant = Some("DEFAULT".to_string());
                self.get_coding_agent(&default_executor_profile_id)
                    .expect("No default variant found")
            })
    }
    pub async fn get_recommended_executor_profile(
        &self,
    ) -> Result<ExecutorProfileId, ProfileError> {
        let mut agents_with_info: Vec<(BaseCodingAgent, AvailabilityInfo)> = Vec::new();

        for &base_agent in self.executors.keys() {
            let profile_id = ExecutorProfileId::new(base_agent);
            if let Some(coding_agent) = self.get_coding_agent(&profile_id) {
                let info = coding_agent.get_availability_info();
                if info.is_available() {
                    agents_with_info.push((base_agent, info));
                }
            }
        }

        if agents_with_info.is_empty() {
            return Err(ProfileError::NoAvailableExecutorProfile);
        }

        agents_with_info.sort_by(|a, b| {
            use crate::executors::AvailabilityInfo;
            match (&a.1, &b.1) {
                // Both have login detected - compare timestamps (most recent first)
                (
                    AvailabilityInfo::LoginDetected {
                        last_auth_timestamp: time_a,
                    },
                    AvailabilityInfo::LoginDetected {
                        last_auth_timestamp: time_b,
                    },
                ) => time_b.cmp(time_a),
                // LoginDetected > InstallationFound
                (AvailabilityInfo::LoginDetected { .. }, AvailabilityInfo::InstallationFound) => {
                    std::cmp::Ordering::Less
                }
                (AvailabilityInfo::InstallationFound, AvailabilityInfo::LoginDetected { .. }) => {
                    std::cmp::Ordering::Greater
                }
                // LoginDetected > NotFound
                (AvailabilityInfo::LoginDetected { .. }, AvailabilityInfo::NotFound) => {
                    std::cmp::Ordering::Less
                }
                (AvailabilityInfo::NotFound, AvailabilityInfo::LoginDetected { .. }) => {
                    std::cmp::Ordering::Greater
                }
                // InstallationFound > NotFound
                (AvailabilityInfo::InstallationFound, AvailabilityInfo::NotFound) => {
                    std::cmp::Ordering::Less
                }
                (AvailabilityInfo::NotFound, AvailabilityInfo::InstallationFound) => {
                    std::cmp::Ordering::Greater
                }
                // Same state - equal
                _ => std::cmp::Ordering::Equal,
            }
        });

        let selected = agents_with_info[0].0;
        tracing::info!("Recommended executor: {}", selected);
        Ok(ExecutorProfileId::new(selected))
    }
}

pub fn to_default_variant(id: &ExecutorProfileId) -> ExecutorProfileId {
    ExecutorProfileId {
        executor: id.executor,
        variant: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::{
        claude::ClaudeCode,
        codex::{Codex, ReasoningSummary},
        kimi::KimiCode,
        pi::Pi,
    };

    #[test]
    fn default_profiles_include_strongly_typed_pi() {
        match ExecutorConfigs::from_defaults()
            .get_coding_agent(&ExecutorProfileId::new(BaseCodingAgent::Pi))
        {
            Some(CodingAgent::Pi(Pi { model, .. })) => assert_eq!(model, None),
            other => panic!("expected Pi DEFAULT profile, got {other:?}"),
        }
    }

    #[test]
    fn default_profiles_include_supported_codex_models() {
        let profiles = ExecutorConfigs::from_defaults();

        assert_codex_model(&profiles, "DEFAULT", "gpt-6-astra");
        assert_codex_model(&profiles, "GPT_6_ASTRA", "gpt-6-astra");
        assert_codex_model(&profiles, "GPT_5.6_SOL", "gpt-5.6-sol");
        assert_codex_model(&profiles, "GPT_5.6_TERRA", "gpt-5.6-terra");
        assert_codex_model(&profiles, "GPT_5.6_LUNA", "gpt-5.6-luna");
        assert_codex_model(&profiles, "GPT_5.3_CODEX_SPARK", "gpt-5.3-codex-spark");
        assert_codex_model(&profiles, "GPT_5.4", "gpt-5.4");
        assert_codex_model(&profiles, "CODEX_AUTO_REVIEW", "codex-auto-review");
        assert_codex_model(&profiles, "GPT_5.5", "gpt-5.5");
        assert_codex_model(&profiles, "GPT_5.4_MINI", "gpt-5.4-mini");

        for variant in ["HIGH", "APPROVALS", "MAX", "GPT_5.2", "GPT_5.3_CODEX"] {
            assert!(
                profiles
                    .get_coding_agent(&ExecutorProfileId::with_variant(
                        BaseCodingAgent::Codex,
                        variant.to_string(),
                    ))
                    .is_none(),
                "codex {variant} profile should not exist"
            );
        }
    }

    #[test]
    fn default_profiles_do_not_hardcode_opencode_models() {
        let profiles = ExecutorConfigs::from_defaults();
        let opencode = profiles
            .executors
            .get(&BaseCodingAgent::Opencode)
            .expect("opencode profiles should exist");
        let mut models = opencode
            .configurations
            .values()
            .filter_map(|config| match config {
                CodingAgent::Opencode(config) => config.model.clone(),
                _ => None,
            })
            .collect::<Vec<_>>();
        models.sort();
        models.dedup();

        assert!(models.is_empty());
        for variant in [
            "opencode/glm-5-free",
            "opencode/kimi-k2.5-free",
            "opencode/minimax-m2.5-free",
        ] {
            assert!(
                !opencode.configurations.contains_key(variant),
                "opencode {variant} should come from runtime discovery, not defaults"
            );
        }
    }

    #[test]
    fn opencode_user_profile_migration_resets_models_and_preserves_builtin_settings() {
        let defaults = ExecutorConfigs::from_defaults();
        let mut profiles: ExecutorConfigs = serde_json::from_value(serde_json::json!({
            "executors": {
                "OPENCODE": {
                    "DEFAULT": {
                        "OPENCODE": {
                            "model": "openai/gpt-5",
                            "variant": "high",
                            "auto_approve": false
                        }
                    },
                    "PLAN": {
                        "OPENCODE": {
                            "model": "openai/gpt-5",
                            "mode": "plan",
                            "auto_compact": false
                        }
                    },
                    "APPROVALS": {
                        "OPENCODE": {
                            "model": "openai/gpt-5",
                            "auto_approve": false
                        }
                    },
                    "legacy/model": { "OPENCODE": { "model": "legacy/model" } },
                    "custom-no-model": { "OPENCODE": { "auto_approve": false } },
                    "opencode/glm-5-free": {
                        "OPENCODE": { "model": "opencode/glm-5-free" }
                    }
                }
            }
        }))
        .expect("user profiles should deserialize");

        assert!(profiles.migrate_opencode_model_overrides(&defaults));
        let opencode = profiles
            .executors
            .get(&BaseCodingAgent::Opencode)
            .expect("opencode overrides should remain");
        assert_eq!(opencode.configurations.len(), 3);
        assert!(!opencode.configurations.contains_key("legacy/model"));
        assert!(!opencode.configurations.contains_key("custom-no-model"));
        assert!(!opencode.configurations.contains_key("opencode/glm-5-free"));
        match opencode.configurations.get("DEFAULT") {
            Some(CodingAgent::Opencode(config)) => {
                assert_eq!(config.model.as_deref(), None);
                assert_eq!(config.variant.as_deref(), Some("high"));
                assert!(!config.auto_approve);
            }
            other => panic!("expected OpenCode DEFAULT override, got {other:?}"),
        }
        match opencode.configurations.get("PLAN") {
            Some(CodingAgent::Opencode(config)) => {
                assert_eq!(config.model.as_deref(), None);
                assert_eq!(config.agent.as_deref(), Some("plan"));
                assert!(!config.auto_compact);
            }
            other => panic!("expected OpenCode PLAN override, got {other:?}"),
        }
        match opencode.configurations.get("APPROVALS") {
            Some(CodingAgent::Opencode(config)) => {
                assert_eq!(config.model.as_deref(), None);
                assert!(!config.auto_approve);
            }
            other => panic!("expected OpenCode APPROVALS override, got {other:?}"),
        }
        assert!(!profiles.migrate_opencode_model_overrides(&defaults));
    }

    #[test]
    fn opencode_user_profile_migration_drops_custom_only_executor_override() {
        let defaults = ExecutorConfigs::from_defaults();
        let mut profiles: ExecutorConfigs = serde_json::from_value(serde_json::json!({
            "executors": {
                "OPENCODE": {
                    "legacy/model": { "OPENCODE": { "model": "legacy/model" } }
                }
            }
        }))
        .expect("user profiles should deserialize");

        assert!(profiles.migrate_opencode_model_overrides(&defaults));
        assert!(!profiles.executors.contains_key(&BaseCodingAgent::Opencode));
    }

    #[test]
    fn canonical_variant_key_preserves_provider_model_ids() {
        assert_eq!(canonical_variant_key("litellm/gpt-5.4"), "litellm/gpt-5.4");
    }

    #[test]
    fn canonical_variant_key_strips_legacy_auto_model_prefix() {
        assert_eq!(
            canonical_variant_key("AUTO_MODEL_opencode/glm-5-free"),
            "opencode/glm-5-free"
        );
        assert_eq!(canonical_variant_key("AUTO_MODEL_GPT_5_2"), "GPT_5_2");
    }

    #[test]
    fn default_profiles_include_new_model_variants() {
        let profiles = ExecutorConfigs::from_defaults();

        assert_claude_model(&profiles, "DEFAULT", "default");
        assert_claude_model(&profiles, "BEST", "best");
        assert_claude_model(&profiles, "OPUS", "opus");
        assert_claude_model(&profiles, "SONNET", "sonnet");
        assert_claude_model(&profiles, "HAIKU", "haiku");
        assert_claude_model(&profiles, "SONNET_1M", "sonnet[1m]");
        assert_claude_model(&profiles, "OPUS_1M", "opus[1m]");
        assert_claude_model(&profiles, "OPUSPLAN", "opusplan");
        assert_claude_model(&profiles, "OPUS_4_6", "claude-opus-4-6");
        assert_claude_model(&profiles, "SONNET_4_6", "claude-sonnet-4-6");
        assert_claude_model(&profiles, "OPUS_4_7", "claude-opus-4-7");
        assert_claude_model(&profiles, "OPUS_4_8", "claude-opus-4-8");
        assert_claude_model(&profiles, "FABLE_5_1", "claude-fable-5-1");
        assert_claude_model(&profiles, "OPUS_5", "claude-opus-5");
        assert_claude_model(&profiles, "SONNET_5", "claude-sonnet-5");
        assert_claude_model(&profiles, "HAIKU_4_5", "claude-haiku-4-5-20251001");
        assert_kimi_model(
            &profiles,
            "moonshot-cn/kimi-k2.6,thinking",
            "moonshot-cn/kimi-k2.6,thinking",
        );
        assert_kimi_model(
            &profiles,
            "moonshot-cn/kimi-k2.5,thinking",
            "moonshot-cn/kimi-k2.5,thinking",
        );
        assert_kimi_model(&profiles, "moonshot-cn/kimi-k2.5", "moonshot-cn/kimi-k2.5");
        assert_kimi_model(
            &profiles,
            "kimi-code/kimi-for-coding,thinking",
            "kimi-code/kimi-for-coding,thinking",
        );
        assert_kimi_model(
            &profiles,
            "kimi-code/kimi-for-coding",
            "kimi-code/kimi-for-coding",
        );
        assert_kimi_model(&profiles, "KIMI_K2_5", "moonshot-cn/kimi-k2.6");
        assert_kimi_model(&profiles, "KIMI_K2_6", "moonshot-cn/kimi-k2.6");
    }

    fn assert_claude_model(profiles: &ExecutorConfigs, variant: &str, expected: &str) {
        match profiles
            .get_coding_agent(&ExecutorProfileId::with_variant(
                BaseCodingAgent::ClaudeCode,
                variant.to_string(),
            ))
            .unwrap_or_else(|| panic!("claude code {variant} profile should exist"))
        {
            CodingAgent::ClaudeCode(ClaudeCode { model, .. }) => {
                assert_eq!(model.as_deref(), Some(expected));
            }
            other => panic!("expected claude code profile, got {other:?}"),
        }
    }

    fn assert_codex_model(profiles: &ExecutorConfigs, variant: &str, expected: &str) {
        match profiles
            .get_coding_agent(&ExecutorProfileId::with_variant(
                BaseCodingAgent::Codex,
                variant.to_string(),
            ))
            .unwrap_or_else(|| panic!("codex {variant} profile should exist"))
        {
            CodingAgent::Codex(Codex {
                model,
                model_reasoning_summary,
                ..
            }) => {
                assert_eq!(model.as_deref(), Some(expected));
                assert_eq!(model_reasoning_summary, Some(ReasoningSummary::Auto));
            }
            other => panic!("expected codex profile, got {other:?}"),
        }
    }

    fn assert_kimi_model(profiles: &ExecutorConfigs, variant: &str, expected: &str) {
        match profiles
            .get_coding_agent(&ExecutorProfileId::with_variant(
                BaseCodingAgent::KimiCode,
                variant.to_string(),
            ))
            .unwrap_or_else(|| panic!("kimi code {variant} profile should exist"))
        {
            CodingAgent::KimiCode(KimiCode { model, .. }) => {
                assert_eq!(model.as_deref(), Some(expected));
            }
            other => panic!("expected kimi code profile, got {other:?}"),
        }
    }
}
