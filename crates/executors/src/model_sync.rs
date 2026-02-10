use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    env::ExecutionEnv,
    executors::{BaseCodingAgent, CodingAgent, StandardCodingAgentExecutor},
    profile::{ExecutorConfig, ExecutorConfigs, ProfileError, canonical_variant_key},
};

const AUTO_MODEL_VARIANT_PREFIX: &str = "AUTO_MODEL_";

pub async fn refresh_profiles_from_agent_models(
    current_dir: &Path,
    env: &ExecutionEnv,
) -> Result<bool, ProfileError> {
    ExecutorConfigs::reload();
    let mut configs = ExecutorConfigs::get_cached();
    let updates = discover_models(&configs, current_dir, env).await;

    if updates.is_empty() {
        return Ok(false);
    }

    let changed = apply_model_updates(&mut configs, &updates);
    if changed {
        configs.save_overrides()?;
        ExecutorConfigs::reload();
    }

    Ok(changed)
}

async fn discover_models(
    configs: &ExecutorConfigs,
    current_dir: &Path,
    env: &ExecutionEnv,
) -> HashMap<BaseCodingAgent, Vec<String>> {
    let mut updates = HashMap::new();

    for (executor, executor_config) in &configs.executors {
        let Some(base) = executor_config
            .get_default()
            .or_else(|| executor_config.configurations.values().next())
        else {
            continue;
        };

        if !base.get_availability_info().is_available() {
            continue;
        }

        match base {
            CodingAgent::Opencode(opencode) => match opencode.list_models(current_dir, env).await {
                Ok(models) => {
                    updates.insert(*executor, models);
                }
                Err(err) => {
                    tracing::debug!("Failed to list models for {executor}: {err}");
                }
            },
            _ => {}
        }
    }

    updates
}

fn apply_model_updates(
    configs: &mut ExecutorConfigs,
    updates: &HashMap<BaseCodingAgent, Vec<String>>,
) -> bool {
    let mut changed = false;

    for (executor, models) in updates {
        let Some(executor_config) = configs.executors.get_mut(executor) else {
            continue;
        };

        changed |= upsert_model_variants(executor_config, models);
    }

    changed
}

fn upsert_model_variants(executor_config: &mut ExecutorConfig, models: &[String]) -> bool {
    let Some(base) = executor_config
        .get_default()
        .or_else(|| executor_config.configurations.values().next())
        .cloned()
    else {
        return false;
    };

    if !supports_model(&base) {
        return false;
    }

    let mut changed = false;
    let mut desired = HashSet::new();
    for model in models {
        desired.insert(auto_variant_key(model));
    }

    let existing_auto: Vec<String> = executor_config
        .configurations
        .keys()
        .filter(|key| key.starts_with(AUTO_MODEL_VARIANT_PREFIX))
        .cloned()
        .collect();

    for key in existing_auto {
        if !desired.contains(&key) {
            executor_config.configurations.remove(&key);
            changed = true;
        }
    }

    for model in models {
        let key = auto_variant_key(model);
        let Some(config) = with_model(&base, model) else {
            continue;
        };

        match executor_config.configurations.get(&key) {
            Some(existing) if existing == &config => {}
            _ => {
                executor_config.configurations.insert(key, config);
                changed = true;
            }
        }
    }

    changed
}

fn supports_model(config: &CodingAgent) -> bool {
    matches!(
        config,
        CodingAgent::Codex(_)
            | CodingAgent::ClaudeCode(_)
            | CodingAgent::Gemini(_)
            | CodingAgent::Opencode(_)
            | CodingAgent::CursorAgent(_)
            | CodingAgent::Copilot(_)
            | CodingAgent::Droid(_)
    )
}

fn with_model(config: &CodingAgent, model: &str) -> Option<CodingAgent> {
    let model = model.to_string();
    match config {
        CodingAgent::Codex(base) => {
            let mut next = base.clone();
            next.model = Some(model);
            Some(CodingAgent::Codex(next))
        }
        CodingAgent::ClaudeCode(base) => {
            let mut next = base.clone();
            next.model = Some(model);
            Some(CodingAgent::ClaudeCode(next))
        }
        CodingAgent::Gemini(base) => {
            let mut next = base.clone();
            next.model = Some(model);
            Some(CodingAgent::Gemini(next))
        }
        CodingAgent::Opencode(base) => {
            let mut next = base.clone();
            next.model = Some(model);
            Some(CodingAgent::Opencode(next))
        }
        CodingAgent::CursorAgent(base) => {
            let mut next = base.clone();
            next.model = Some(model);
            Some(CodingAgent::CursorAgent(next))
        }
        CodingAgent::Copilot(base) => {
            let mut next = base.clone();
            next.model = Some(model);
            Some(CodingAgent::Copilot(next))
        }
        CodingAgent::Droid(base) => {
            let mut next = base.clone();
            next.model = Some(model);
            Some(CodingAgent::Droid(next))
        }
        _ => None,
    }
}

fn auto_variant_key(model: &str) -> String {
    format!("{}{}", AUTO_MODEL_VARIANT_PREFIX, canonical_variant_key(model))
}
