use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use url::Url;

const DEFAULT_CUSTOM_PROVIDER_NPM: &str = "@ai-sdk/openai-compatible";
const LEGACY_CUSTOM_PROVIDER_NPM: &str = "@ai-sdk/anthropic";

/// CLI configuration for OpenTeams built-in executor (stored in ~/.openteams/config.toml)
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CliConfig {
    pub provider: ProviderConfig,
    pub model: ModelConfig,
    pub behavior: BehaviorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderConfig {
    /// Default provider name: anthropic, openai, google, openrouter, minimax, ollama, custom
    pub default: String,
    pub anthropic: Option<ProviderCredentials>,
    pub openai: Option<ProviderCredentials>,
    pub google: Option<ProviderCredentials>,
    pub openrouter: Option<ProviderCredentials>,
    pub minimax: Option<ProviderCredentials>,
    pub ollama: Option<OllamaConfig>,
    pub custom: Option<CustomProviderConfig>,
    /// 多自定义 Provider 配置
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub custom_providers: Option<HashMap<String, CustomProviderEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderCredentials {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OllamaConfig {
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CustomProviderConfig {
    pub name: Option<String>,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
}

/// 自定义 Provider 完整配置（支持多 Provider、含 models 的 modalities/limit）
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CustomProviderEntry {
    /// 唯一标识，如 "bailian-coding-plan"
    pub id: String,
    /// 显示名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// NPM 包名，默认 "@ai-sdk/anthropic"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,
    /// Provider 连接选项
    pub options: CustomProviderOptions,
    /// 模型配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<HashMap<String, CustomModelConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CustomProviderOptions {
    #[serde(rename = "baseURL")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CustomModelConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<ModelModalities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<ModelLimits>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelConfig {
    /// Default model name
    pub default: String,
    pub anthropic: Option<ProviderModelConfig>,
    pub openai: Option<ProviderModelConfig>,
    pub google: Option<ProviderModelConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderModelConfig {
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BehaviorConfig {
    pub auto_approve: bool,
    pub auto_compact: bool,
}

impl CliConfig {
    pub fn default_config() -> Self {
        Self {
            provider: ProviderConfig {
                default: "anthropic".to_string(),
                anthropic: None,
                openai: None,
                google: None,
                openrouter: None,
                minimax: None,
                ollama: None,
                custom: None,
                custom_providers: None,
            },
            model: ModelConfig {
                default: "claude-sonnet-5".to_string(),
                anthropic: None,
                openai: None,
                google: None,
            },
            behavior: BehaviorConfig {
                auto_approve: false,
                auto_compact: true,
            },
        }
    }

    pub fn config_path() -> Option<std::path::PathBuf> {
        dirs::home_dir().map(|h| h.join(".openteams").join("config.toml"))
    }
}

pub fn normalize_custom_provider_entries(config: &mut CliConfig) {
    if let Some(custom_providers) = config.provider.custom_providers.as_mut() {
        for (id, entry) in custom_providers.iter_mut() {
            entry.npm = normalized_custom_provider_npm(id, entry);
        }
    }
}

pub fn normalized_custom_provider_npm(id: &str, entry: &CustomProviderEntry) -> Option<String> {
    if should_use_openai_compatible_npm(id, entry) {
        return Some(DEFAULT_CUSTOM_PROVIDER_NPM.to_string());
    }

    entry
        .npm
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn should_use_openai_compatible_npm(id: &str, entry: &CustomProviderEntry) -> bool {
    let npm = entry
        .npm
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let id_or_name_mentions_litellm = id.to_ascii_lowercase().contains("litellm")
        || entry
            .name
            .as_deref()
            .is_some_and(|name| name.to_ascii_lowercase().contains("litellm"));
    let endpoint_mentions_litellm = entry.options.base_url.as_deref().is_some_and(|base_url| {
        base_url.to_ascii_lowercase().contains("litellm")
            || Url::parse(base_url)
                .ok()
                .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
                .is_some_and(|host| host.contains("litellm"))
    });

    (id_or_name_mentions_litellm || endpoint_mentions_litellm)
        && matches!(npm, None | Some(LEGACY_CUSTOM_PROVIDER_NPM))
}

/// openteams-cli provider configuration (stored in ~/.config/openteams-cli/openteams.json)
/// Matches the openteams-cli config.ts Provider schema
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OpenTeamsCliProviderConfig {
    /// NPM package for the provider (e.g., "@ai-sdk/anthropic")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,
    /// Display name for the provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Provider options (apiKey, baseURL, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<OpenTeamsCliProviderOptions>,
    /// Model configurations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<HashMap<String, OpenTeamsCliModelConfig>>,
    /// Whitelist of model IDs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whitelist: Option<Vec<String>>,
    /// Blacklist of model IDs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blacklist: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OpenTeamsCliProviderOptions {
    /// API key for the provider
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "apiKey", alias = "api_key")]
    pub api_key: Option<String>,
    /// Base URL for the provider API
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "baseURL", alias = "base_url")]
    pub base_url: Option<String>,
    /// Request timeout in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Chunk timeout in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "chunkTimeout", alias = "chunk_timeout")]
    pub chunk_timeout: Option<u64>,
    /// GitHub Enterprise URL for copilot authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "enterpriseUrl", alias = "enterprise_url")]
    pub enterprise_url: Option<String>,
    /// Enable promptCacheKey for this provider
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "setCacheKey", alias = "set_cache_key")]
    pub set_cache_key: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OpenTeamsCliModelConfig {
    /// Display name for the model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Model modalities (input/output capabilities)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<ModelModalities>,
    /// Model-specific options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
    /// Model limits (context window, output tokens)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<ModelLimits>,
    /// Variant-specific configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<HashMap<String, ModelVariantConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelModalities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelVariantConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
}

/// Root configuration for openteams-cli (openteams.json)
#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct OpenTeamsCliConfig {
    /// Provider configurations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<HashMap<String, OpenTeamsCliProviderConfig>>,
    /// Default model in provider/model format
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Small model for tasks like title generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,
    /// Agent configurations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<serde_json::Value>,
    /// Custom commands
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<serde_json::Value>,
    /// MCP server configurations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<serde_json::Value>,
    /// Permission settings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<serde_json::Value>,
    /// LSP server configurations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp: Option<serde_json::Value>,
    /// Formatter configurations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatter: Option<serde_json::Value>,
    /// Experimental features
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<serde_json::Value>,
    /// Username to display
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Log level
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
}

impl OpenTeamsCliConfig {
    pub fn config_path() -> Option<std::path::PathBuf> {
        dirs::home_dir().map(|h| {
            let config_dir = h.join(".config").join("openteams-cli");
            let jsonc_path = config_dir.join("openteams.jsonc");
            let json_path = config_dir.join("openteams.json");

            if jsonc_path.exists() {
                jsonc_path
            } else {
                json_path
            }
        })
    }

    pub fn config_dir() -> Option<std::path::PathBuf> {
        dirs::home_dir().map(|h| h.join(".config").join("openteams-cli"))
    }
}

#[cfg(test)]
mod tests {
    use super::CliConfig;

    #[test]
    fn default_config_uses_current_anthropic_model() {
        let config = CliConfig::default_config();

        assert_eq!(config.provider.default, "anthropic");
        assert_eq!(config.model.default, "claude-sonnet-5");
    }
}
