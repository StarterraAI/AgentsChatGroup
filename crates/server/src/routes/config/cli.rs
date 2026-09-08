async fn get_cli_config(
    State(_deployment): State<DeploymentImpl>,
) -> ResponseJson<ApiResponse<CliConfig>> {
    let config = read_cli_config_from_disk().await;
    ResponseJson(ApiResponse::success(mask_api_keys(config)))
}

/// Write CLI config to ~/.openteams/config.toml
async fn update_cli_config(
    State(_deployment): State<DeploymentImpl>,
    Json(mut new_config): Json<CliConfig>,
) -> ResponseJson<ApiResponse<CliConfig>> {
    if let Ok(old_config) = try_read_cli_config_from_disk().await {
        merge_masked_keys(&mut new_config, &old_config);
    }

    match write_cli_config_to_disk(&new_config).await {
        Ok(_) => {
            // 同步默认 provider/model 到 openteams-cli
            if let Err(e) = sync_openteams_cli_profiles_from_disk().await {
                tracing::error!("Failed to sync OpenTeams CLI profiles: {}", e);
            }
            ResponseJson(ApiResponse::success(mask_api_keys(new_config)))
        }
        Err(e) => ResponseJson(ApiResponse::error(&format!(
            "Failed to save CLI config: {}",
            e
        ))),
    }
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct SyncToCliRequest {
    pub custom_provider_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct SyncToCliResponse {
    pub synced: bool,
    pub message: String,
    pub config_path: Option<String>,
}

async fn sync_cli_config_to_openteams_cli(
    State(_deployment): State<DeploymentImpl>,
    Json(req): Json<SyncToCliRequest>,
) -> ResponseJson<ApiResponse<SyncToCliResponse>> {
    let openteams_config = read_cli_config_from_disk().await;

    match sync_to_openteams_cli(&openteams_config, req.custom_provider_id.as_deref()).await {
        Ok(config_path) => ResponseJson(ApiResponse::success(SyncToCliResponse {
            synced: true,
            message: "Configuration synced to openteams-cli".to_string(),
            config_path: Some(config_path),
        })),
        Err(e) => ResponseJson(ApiResponse::error(&format!(
            "Failed to sync to openteams-cli: {}",
            e
        ))),
    }
}

async fn sync_pi_models_explicitly(
    State(_deployment): State<DeploymentImpl>,
) -> ResponseJson<ApiResponse<PiModelsSyncResult>> {
    match coordinate_pi_models_from_saved_config().await {
        Ok(result) => ResponseJson(ApiResponse::success(result)),
        Err(error) => {
            tracing::warn!(error = %error, "Failed to synchronize Pi models");
            ResponseJson(ApiResponse::error(&pi_sync_error_message(
                &error.to_string(),
            )))
        }
    }
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct ProviderMutationWarning {
    pub settings_saved: bool,
    pub retry_available: bool,
    pub retry_path: String,
    pub failed_syncs: Vec<String>,
    pub message: String,
}

fn pi_sync_warning(
    saved_action: &str,
    openteams_cli_failed: bool,
    profiles_failed: bool,
) -> ProviderMutationWarning {
    let mut failed_syncs = vec!["pi_models".to_string()];
    if openteams_cli_failed {
        failed_syncs.push("openteams_cli".to_string());
    }
    if profiles_failed {
        failed_syncs.push("profiles".to_string());
    }
    ProviderMutationWarning {
        settings_saved: true,
        retry_available: true,
        retry_path: PI_MODELS_SYNC_RETRY_PATH.to_string(),
        failed_syncs,
        message: format!(
            "Provider {saved_action}, but Pi model synchronization failed. The saved settings were not rolled back."
        ),
    }
}

fn pi_sync_error_message(error: &str) -> String {
    format!(
        "Failed to synchronize Pi models; the existing models file was preserved: {error}"
    )
}

enum ProviderMutationSyncFailure {
    Pi(ProviderMutationWarning),
    Other(String),
}

fn aggregate_provider_sync_failures(
    saved_action: &str,
    pi_failed: bool,
    cli_error: Option<String>,
    profile_error: Option<String>,
) -> Option<ProviderMutationSyncFailure> {
    if pi_failed {
        return Some(ProviderMutationSyncFailure::Pi(pi_sync_warning(
            saved_action,
            cli_error.is_some(),
            profile_error.is_some(),
        )));
    }
    if let Some(error) = cli_error {
        return Some(ProviderMutationSyncFailure::Other(format!(
            "Provider {saved_action} but failed to sync to openteams-cli: {error}"
        )));
    }
    profile_error.map(|error| {
        ProviderMutationSyncFailure::Other(format!(
            "Provider {saved_action} but failed to sync OpenTeams CLI profiles: {error}"
        ))
    })
}

async fn sync_to_openteams_cli(
    openteams_config: &CliConfig,
    custom_provider_id: Option<&str>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let cli_config_path = OpenTeamsCliConfig::config_path()
        .ok_or("Cannot determine openteams-cli config directory")?;

    let mut cli_config = try_read_openteams_cli_config_from_disk().await?;
    let original_cli_config = cli_config.clone();

    sync_requested_provider_to_cli_config(&mut cli_config, openteams_config, custom_provider_id);

    if cli_config_changed(&original_cli_config, &cli_config)? {
        write_openteams_cli_config_to_disk(&cli_config).await?;
    }
    sync_openteams_cli_profiles_from_cli_config(&cli_config).await?;

    Ok(cli_config_path.to_string_lossy().to_string())
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct RestartCliResponse {
    pub restarted: bool,
    pub message: String,
    pub base_url: Option<String>,
    pub port: Option<u16>,
}

/// POST /config/cli/restart-service
/// 重启 openteams-cli 服务，使配置变更生效
async fn restart_cli_service(
    State(deployment): State<DeploymentImpl>,
) -> ResponseJson<ApiResponse<RestartCliResponse>> {
    match deployment.restart_cli().await {
        Ok((base_url, port)) => ResponseJson(ApiResponse::success(RestartCliResponse {
            restarted: true,
            message: "openteams-cli service restarted successfully".to_string(),
            base_url: Some(base_url),
            port: Some(port),
        })),
        Err(e) => ResponseJson(ApiResponse::error(&format!(
            "Failed to restart openteams-cli: {}",
            e
        ))),
    }
}

// ── Custom Providers CRUD ──────────────────────────────────────────

/// GET /config/cli/custom-providers
async fn list_custom_providers(
    State(_deployment): State<DeploymentImpl>,
) -> ResponseJson<ApiResponse<Vec<CustomProviderEntry>>> {
    let config = read_cli_config_from_disk().await;
    let providers: Vec<CustomProviderEntry> = config
        .provider
        .custom_providers
        .unwrap_or_default()
        .into_values()
        .map(|mut p| {
            mask_custom_provider_key(&mut p);
            p
        })
        .collect();
    ResponseJson(ApiResponse::success(providers))
}

/// POST /config/cli/custom-providers
async fn create_custom_provider(
    State(_deployment): State<DeploymentImpl>,
    Json(entry): Json<CustomProviderEntry>,
) -> ResponseJson<ApiResponse<CustomProviderEntry, ProviderMutationWarning>> {
    if entry.id.is_empty() {
        return ResponseJson(ApiResponse::error("Provider id cannot be empty"));
    }

    let mut config = read_cli_config_from_disk().await;
    let providers = config
        .provider
        .custom_providers
        .get_or_insert_with(HashMap::new);

    if providers.contains_key(&entry.id) {
        return ResponseJson(ApiResponse::error(&format!(
            "Provider '{}' already exists",
            entry.id
        )));
    }

    providers.insert(entry.id.clone(), entry.clone());

    if let Err(e) = write_cli_config_to_disk(&config).await {
        return ResponseJson(ApiResponse::error(&format!("Failed to save config: {}", e)));
    }

    let pi_sync_error = synchronize_pi_models(&config).await.err();
    if let Some(error) = &pi_sync_error {
        tracing::warn!(error = %error, provider_id = %entry.id, "Provider saved but Pi model synchronization failed");
    }

    let cli_sync_error = sync_custom_providers_to_cli(&config, None).await.err();
    if let Some(e) = &cli_sync_error {
        tracing::error!("Failed to sync custom providers to cli: {}", e);
    }
    let profile_sync_error = sync_openteams_cli_profiles_from_disk().await.err();
    if let Some(e) = &profile_sync_error {
        tracing::error!("Failed to sync OpenTeams CLI profiles: {}", e);
    }

    if let Some(failure) = aggregate_provider_sync_failures(
        "saved",
        pi_sync_error.is_some(),
        cli_sync_error.map(|error| error.to_string()),
        profile_sync_error.map(|error| error.to_string()),
    ) {
        return match failure {
            ProviderMutationSyncFailure::Pi(warning) => {
                ResponseJson(ApiResponse::error_with_data(warning))
            }
            ProviderMutationSyncFailure::Other(message) => {
                ResponseJson(ApiResponse::error(&message))
            }
        };
    }

    let mut masked = entry;
    mask_custom_provider_key(&mut masked);
    ResponseJson(ApiResponse::success(masked))
}

/// PUT /config/cli/custom-providers/{id}
async fn update_custom_provider(
    State(_deployment): State<DeploymentImpl>,
    Path(id): Path<String>,
    Json(mut entry): Json<CustomProviderEntry>,
) -> ResponseJson<ApiResponse<CustomProviderEntry, ProviderMutationWarning>> {
    entry.id = id.clone();

    let mut config = read_cli_config_from_disk().await;
    let providers = config
        .provider
        .custom_providers
        .get_or_insert_with(HashMap::new);

    if !providers.contains_key(&id) {
        return ResponseJson(ApiResponse::error(&format!("Provider '{}' not found", id)));
    }

    // 如果 api_key 是掩码，保留旧值
    if let Some(old) = providers.get(&id)
        && let Some(ref new_key) = entry.options.api_key
        && new_key.contains("***")
    {
        entry.options.api_key = old.options.api_key.clone();
    }

    providers.insert(id.clone(), entry.clone());

    if let Err(e) = write_cli_config_to_disk(&config).await {
        return ResponseJson(ApiResponse::error(&format!("Failed to save config: {}", e)));
    }

    let pi_sync_error = synchronize_pi_models(&config).await.err();
    if let Some(error) = &pi_sync_error {
        tracing::warn!(error = %error, provider_id = %id, "Provider saved but Pi model synchronization failed");
    }

    let cli_sync_error = sync_custom_providers_to_cli(&config, None).await.err();
    if let Some(e) = &cli_sync_error {
        tracing::error!("Failed to sync custom providers to cli: {}", e);
    }
    let profile_sync_error = sync_openteams_cli_profiles_from_disk().await.err();
    if let Some(e) = &profile_sync_error {
        tracing::error!("Failed to sync OpenTeams CLI profiles: {}", e);
    }

    if let Some(failure) = aggregate_provider_sync_failures(
        "saved",
        pi_sync_error.is_some(),
        cli_sync_error.map(|error| error.to_string()),
        profile_sync_error.map(|error| error.to_string()),
    ) {
        return match failure {
            ProviderMutationSyncFailure::Pi(warning) => {
                ResponseJson(ApiResponse::error_with_data(warning))
            }
            ProviderMutationSyncFailure::Other(message) => {
                ResponseJson(ApiResponse::error(&message))
            }
        };
    }

    mask_custom_provider_key(&mut entry);
    ResponseJson(ApiResponse::success(entry))
}

/// DELETE /config/cli/custom-providers/{id}
async fn delete_custom_provider(
    State(_deployment): State<DeploymentImpl>,
    Path(id): Path<String>,
) -> ResponseJson<ApiResponse<(), ProviderMutationWarning>> {
    let mut config = read_cli_config_from_disk().await;
    let providers = config
        .provider
        .custom_providers
        .get_or_insert_with(HashMap::new);

    if providers.remove(&id).is_none() {
        return ResponseJson(ApiResponse::error(&format!("Provider '{}' not found", id)));
    }

    // 任务7：如果删除的是当前默认 Provider，自动回退到 anthropic
    if config.provider.default == id {
        config.provider.default = "anthropic".to_string();
        config.model.default = CliConfig::default_config().model.default;
    }

    if let Err(e) = write_cli_config_to_disk(&config).await {
        return ResponseJson(ApiResponse::error(&format!("Failed to save config: {}", e)));
    }

    let pi_sync_error = synchronize_pi_models(&config).await.err();
    if let Some(error) = &pi_sync_error {
        tracing::warn!(error = %error, provider_id = %id, "Provider deleted but Pi model synchronization failed");
    }

    let cli_sync_error = sync_custom_providers_to_cli(&config, Some(&id)).await.err();
    if let Some(e) = &cli_sync_error {
        tracing::error!("Failed to sync custom providers to cli: {}", e);
    }
    let profile_sync_error = sync_openteams_cli_profiles_from_disk().await.err();
    if let Some(e) = &profile_sync_error {
        tracing::error!("Failed to sync OpenTeams CLI profiles: {}", e);
    }

    if let Some(failure) = aggregate_provider_sync_failures(
        "deleted",
        pi_sync_error.is_some(),
        cli_sync_error.map(|error| error.to_string()),
        profile_sync_error.map(|error| error.to_string()),
    ) {
        return match failure {
            ProviderMutationSyncFailure::Pi(warning) => {
                ResponseJson(ApiResponse::error_with_data(warning))
            }
            ProviderMutationSyncFailure::Other(message) => {
                ResponseJson(ApiResponse::error(&message))
            }
        };
    }

    ResponseJson(ApiResponse::success(()))
}

/// 将 custom_providers 同步到 ~/.config/openteams-cli/openteams.json
/// deleted_id: 如果有刚删除的 provider id，同步时从 CLI 配置中移除
async fn sync_custom_providers_to_cli(
    app_config: &CliConfig,
    deleted_id: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut cli_config = try_read_openteams_cli_config_from_disk().await?;
    let original_cli_config = cli_config.clone();

    sync_managed_custom_providers_to_cli_config(&mut cli_config, app_config, deleted_id);

    if cli_config_changed(&original_cli_config, &cli_config)? {
        write_openteams_cli_config_to_disk(&cli_config).await?;
    }
    Ok(())
}

/*

    // 任务8：如果有刚删除的 provider，从 CLI 配置中移除
    if let Some(id) = deleted_id {
        cli_providers.remove(id);
    }

    // 仅 upsert custom_providers 中的条目，不删除用户手工维护的其他 provider
    if let Some(providers) = custom_providers {
        for (id, entry) in providers {
            let cli_provider = OpenTeamsCliProviderConfig {
                npm: normalized_custom_provider_npm(id, entry),
                name: entry.name.clone(),
                options: Some(OpenTeamsCliProviderOptions {
                    api_key: entry.options.api_key.clone(),
                    base_url: entry.options.base_url.clone(),
                    timeout: entry.options.timeout,
                    chunk_timeout: None,
                    enterprise_url: None,
                    set_cache_key: None,
                }),
                models: entry.models.as_ref().map(|models| {
                    models
                        .iter()
                        .map(|(model_id, model_cfg)| {
                            let cli_model =
                                services::services::cli_config::OpenTeamsCliModelConfig {
                                    name: model_cfg.name.clone(),
                                    modalities: model_cfg.modalities.clone(),
                                    options: model_cfg.options.clone(),
                                    limit: model_cfg.limit.clone(),
                                    variants: None,
                                };
                            (model_id.clone(), cli_model)
                        })
                        .collect()
                }),
                whitelist: None,
                blacklist: None,
            };
            cli_providers.insert(id.clone(), cli_provider);
        }
    }

    // 同步默认 provider/model 到 openteams-cli
    let default_provider = &app_config.provider.default;
    let default_model = &app_config.model.default;

    if builtin_keys.contains(default_provider.as_str()) {
        // 内置 provider：直接使用 provider/model 格式
        cli_config.model = Some(format!("{}/{}", default_provider, default_model));
    } else if cli_providers.contains_key(default_provider) {
        // 自定义 provider：确认已注册后设置
        cli_config.model = Some(format!("{}/{}", default_provider, default_model));
    }

    write_openteams_cli_config_to_disk(&cli_config).await?;
    Ok(())
}

*/

fn sync_requested_provider_to_cli_config(
    cli_config: &mut OpenTeamsCliConfig,
    app_config: &CliConfig,
    custom_provider_id: Option<&str>,
) {
    if let Some(provider_id) = custom_provider_id {
        if let Some(entry) = app_config
            .provider
            .custom_providers
            .as_ref()
            .and_then(|providers| providers.get(provider_id))
        {
            cli_config.provider.get_or_insert_with(HashMap::new).insert(
                provider_id.to_string(),
                build_cli_provider_config(provider_id, entry),
            );
        }
        return;
    }

    let cli_providers = cli_config.provider.get_or_insert_with(HashMap::new);

    sync_builtin_provider_to_cli_config(
        cli_providers,
        "anthropic",
        build_builtin_provider_config(app_config.provider.anthropic.as_ref()),
    );
    sync_builtin_provider_to_cli_config(
        cli_providers,
        "openai",
        build_builtin_provider_config(app_config.provider.openai.as_ref()),
    );
    sync_builtin_provider_to_cli_config(
        cli_providers,
        "google",
        build_builtin_provider_config(app_config.provider.google.as_ref()),
    );
    sync_builtin_provider_to_cli_config(
        cli_providers,
        "openrouter",
        build_builtin_provider_config(app_config.provider.openrouter.as_ref()),
    );
    sync_builtin_provider_to_cli_config(
        cli_providers,
        "minimax",
        build_builtin_provider_config(app_config.provider.minimax.as_ref()),
    );
    sync_builtin_provider_to_cli_config(
        cli_providers,
        "ollama",
        build_ollama_provider_config(app_config.provider.ollama.as_ref()),
    );

    if app_config.provider.default.trim() != "custom" {
        cli_providers.remove("custom");
        return;
    }

    if let Some(custom) = &app_config.provider.custom {
        let provider_id = custom_provider_id.unwrap_or("custom").to_string();
        let provider_config = OpenTeamsCliProviderConfig {
            name: custom.name.clone(),
            npm: None,
            options: Some(OpenTeamsCliProviderOptions {
                api_key: custom.api_key.clone(),
                base_url: custom.endpoint.clone(),
                timeout: None,
                chunk_timeout: None,
                enterprise_url: None,
                set_cache_key: None,
            }),
            models: None,
            whitelist: None,
            blacklist: None,
        };
        cli_providers.insert(provider_id, provider_config);
    } else {
        cli_providers.remove("custom");
    }
}

fn sync_builtin_provider_to_cli_config(
    cli_providers: &mut HashMap<String, OpenTeamsCliProviderConfig>,
    provider_id: &str,
    provider_config: Option<OpenTeamsCliProviderConfig>,
) {
    if let Some(provider_config) = provider_config {
        cli_providers.insert(provider_id.to_string(), provider_config);
    } else {
        cli_providers.remove(provider_id);
    }
}

fn build_builtin_provider_config(
    credentials: Option<&ProviderCredentials>,
) -> Option<OpenTeamsCliProviderConfig> {
    let credentials = credentials?;
    if credentials.api_key.is_none() && credentials.endpoint.is_none() {
        return None;
    }

    Some(OpenTeamsCliProviderConfig {
        npm: None,
        name: None,
        options: Some(OpenTeamsCliProviderOptions {
            api_key: credentials.api_key.clone(),
            base_url: credentials.endpoint.clone(),
            timeout: None,
            chunk_timeout: None,
            enterprise_url: None,
            set_cache_key: None,
        }),
        models: None,
        whitelist: None,
        blacklist: None,
    })
}

fn build_ollama_provider_config(
    config: Option<&OllamaConfig>,
) -> Option<OpenTeamsCliProviderConfig> {
    let config = config?;
    config.endpoint.as_ref()?;

    Some(OpenTeamsCliProviderConfig {
        npm: None,
        name: None,
        options: Some(OpenTeamsCliProviderOptions {
            api_key: None,
            base_url: config.endpoint.clone(),
            timeout: None,
            chunk_timeout: None,
            enterprise_url: None,
            set_cache_key: None,
        }),
        models: None,
        whitelist: None,
        blacklist: None,
    })
}

fn sync_managed_custom_providers_to_cli_config(
    cli_config: &mut OpenTeamsCliConfig,
    app_config: &CliConfig,
    deleted_id: Option<&str>,
) {
    let cli_providers = cli_config.provider.get_or_insert_with(HashMap::new);

    if let Some(id) = deleted_id {
        cli_providers.remove(id);
    }

    if let Some(providers) = &app_config.provider.custom_providers {
        for (id, entry) in providers {
            cli_providers.insert(id.clone(), build_cli_provider_config(id, entry));
        }
    }
}

fn build_cli_provider_config(id: &str, entry: &CustomProviderEntry) -> OpenTeamsCliProviderConfig {
    OpenTeamsCliProviderConfig {
        npm: normalized_custom_provider_npm(id, entry),
        name: entry.name.clone(),
        options: Some(OpenTeamsCliProviderOptions {
            api_key: entry.options.api_key.clone(),
            base_url: entry.options.base_url.clone(),
            timeout: entry.options.timeout,
            chunk_timeout: None,
            enterprise_url: None,
            set_cache_key: None,
        }),
        models: entry.models.as_ref().map(|models| {
            models
                .iter()
                .map(|(model_id, model_cfg)| {
                    let cli_model = services::services::cli_config::OpenTeamsCliModelConfig {
                        name: model_cfg.name.clone(),
                        modalities: model_cfg.modalities.clone(),
                        options: model_cfg.options.clone(),
                        limit: model_cfg.limit.clone(),
                        variants: None,
                    };
                    (model_id.clone(), cli_model)
                })
                .collect()
        }),
        whitelist: None,
        blacklist: None,
    }
}

fn cli_config_changed(
    original: &OpenTeamsCliConfig,
    updated: &OpenTeamsCliConfig,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    Ok(serde_json::to_value(original)? != serde_json::to_value(updated)?)
}

async fn sync_openteams_cli_profiles_from_disk()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli_config = try_read_openteams_cli_config_from_disk().await?;
    sync_openteams_cli_profiles_from_cli_config(&cli_config).await
}

async fn sync_openteams_cli_profiles_from_cli_config(
    cli_config: &OpenTeamsCliConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    ExecutorConfigs::reload();
    let mut profiles = ExecutorConfigs::get_cached();
    let previous_profile_models = openteams_cli_profile_model_ids(&profiles);
    let changed = sync_openteams_cli_models_into_profiles(&mut profiles, cli_config);
    let current_profile_models = openteams_cli_profile_model_ids(&profiles);

    if changed {
        profiles.save_overrides()?;
        ExecutorConfigs::reload();
    }

    reconcile_runtime_model_discovery(
        BaseCodingAgent::OpenTeamsCli,
        &previous_profile_models,
        &current_profile_models,
    )
    .await?;

    Ok(())
}

fn openteams_cli_profile_model_ids(profiles: &ExecutorConfigs) -> Vec<String> {
    let mut models = BTreeSet::new();
    if let Some(executor_config) = profiles.executors.get(&BaseCodingAgent::OpenTeamsCli) {
        for config in executor_config.configurations.values() {
            if let CodingAgent::OpenTeamsCli(config) = config
                && let Some(model) = config.model.as_deref().map(str::trim)
                && !model.is_empty()
            {
                models.insert(model.to_string());
            }
        }
    }
    models.into_iter().collect()
}

fn sync_openteams_cli_models_into_profiles(
    profiles: &mut ExecutorConfigs,
    cli_config: &OpenTeamsCliConfig,
) -> bool {
    let Some(executor_config) = profiles.executors.get_mut(&BaseCodingAgent::OpenTeamsCli) else {
        return false;
    };

    let before = executor_config.clone();
    let Some(base_config) = executor_config
        .get_default()
        .cloned()
        .or_else(|| executor_config.configurations.values().next().cloned())
        .and_then(openteams_cli_default_config)
    else {
        return false;
    };

    executor_config.configurations.remove("PLAN");
    executor_config.configurations.remove("APPROVALS");
    executor_config.set_default(base_config.clone());

    let desired_variant_keys: BTreeSet<_> = resolve_openteams_cli_model_ids(cli_config)
        .into_iter()
        .map(|qualified_model| model_variant_key(&qualified_model))
        .collect();

    let existing_managed_variants: Vec<_> = executor_config
        .configurations
        .iter()
        .filter_map(|(key, config)| {
            is_managed_openteams_cli_model_variant(key, config, &base_config).then_some(key.clone())
        })
        .collect();
    for variant_key in existing_managed_variants {
        if !desired_variant_keys.contains(&variant_key) {
            executor_config.configurations.remove(&variant_key);
        }
    }

    for variant_key in desired_variant_keys {
        let Some(model_config) = with_model(&base_config, &variant_key) else {
            continue;
        };
        executor_config
            .configurations
            .insert(variant_key, model_config);
    }

    before != *executor_config
}

fn is_managed_openteams_cli_model_variant(
    variant_key: &str,
    config: &CodingAgent,
    base_config: &CodingAgent,
) -> bool {
    if variant_key.starts_with(AUTO_MODEL_VARIANT_PREFIX) {
        return true;
    }

    if !variant_key.contains('/') {
        return false;
    }

    match (config, base_config) {
        (CodingAgent::OpenTeamsCli(current), CodingAgent::OpenTeamsCli(base)) => {
            if current.model.as_deref() != Some(variant_key)
                || current.variant.is_some()
                || current.agent.is_some()
            {
                return false;
            }

            let mut normalized = current.clone();
            normalized.model = None;
            normalized.variant = None;
            normalized.agent = None;

            normalized == *base
        }
        _ => false,
    }
}

fn openteams_cli_default_config(config: CodingAgent) -> Option<CodingAgent> {
    match config {
        CodingAgent::OpenTeamsCli(mut inner) => {
            inner.model = None;
            inner.variant = None;
            inner.agent = None;
            Some(CodingAgent::OpenTeamsCli(inner))
        }
        _ => None,
    }
}

fn resolve_openteams_cli_model_ids(cli_config: &OpenTeamsCliConfig) -> Vec<String> {
    let mut model_ids = BTreeSet::new();
    if let Some(providers) = &cli_config.provider {
        for (provider_id, provider) in providers {
            if let Some(models) = &provider.models {
                for model_id in models.keys() {
                    if let Some(qualified) = qualify_model_id(provider_id, model_id) {
                        model_ids.insert(qualified);
                    }
                }
            }
        }
    }

    if let Some(default_model) = cli_config.model.as_deref().map(str::trim)
        && !default_model.is_empty()
    {
        model_ids.insert(default_model.to_string());
    }

    model_ids.into_iter().collect()
}

fn qualify_model_id(provider_id: &str, model_id: &str) -> Option<String> {
    let provider_id = provider_id.trim();
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return None;
    }
    if model_id.contains('/') || provider_id.is_empty() {
        return Some(model_id.to_string());
    }
    Some(format!("{provider_id}/{model_id}"))
}

fn model_variant_key(model_id: &str) -> String {
    canonical_variant_key(model_id.trim())
}

fn mask_custom_provider_key(entry: &mut CustomProviderEntry) {
    if let Some(ref key) = entry.options.api_key {
        entry.options.api_key = Some(mask_key(key));
    }
}

async fn try_read_openteams_cli_config_from_disk()
-> Result<OpenTeamsCliConfig, Box<dyn std::error::Error + Send + Sync>> {
    let path = OpenTeamsCliConfig::config_path()
        .ok_or("Cannot determine openteams-cli config directory")?;

    if !path.exists() {
        return Ok(OpenTeamsCliConfig::default());
    }

    let content = fs::read_to_string(&path).await?;
    parse_openteams_cli_config_content(&content)
}

fn parse_openteams_cli_config_content(
    content: &str,
) -> Result<OpenTeamsCliConfig, Box<dyn std::error::Error + Send + Sync>> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(OpenTeamsCliConfig::default());
    }

    match jsonc_parser::parse_to_serde_value(trimmed, &ParseOptions::default()) {
        Ok(Some(value)) => {
            let config = serde_json::from_value(value)?;
            return Ok(config);
        }
        Ok(None) => return Ok(OpenTeamsCliConfig::default()),
        Err(err) => {
            tracing::debug!(?err, "Failed to parse openteams-cli config as JSONC");
        }
    }

    let config: OpenTeamsCliConfig = serde_json::from_str(trimmed).or_else(|e| {
        let cleaned = remove_json_trailing_commas(trimmed);
        serde_json::from_str(&cleaned).map_err(|_| e)
    })?;
    Ok(config)
}

fn remove_json_trailing_commas(json: &str) -> String {
    let mut result = String::with_capacity(json.len());
    let mut in_string = false;
    let mut escape_next = false;
    let chars: Vec<char> = json.chars().collect();

    for i in 0..chars.len() {
        let c = chars[i];

        if escape_next {
            result.push(c);
            escape_next = false;
            continue;
        }

        if c == '\\' && in_string {
            result.push(c);
            escape_next = true;
            continue;
        }

        if c == '"' && !escape_next {
            in_string = !in_string;
            result.push(c);
            continue;
        }

        if in_string {
            result.push(c);
            continue;
        }

        // Check for trailing comma before ] or }
        if c == ',' {
            // Look ahead for the next non-whitespace character
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == ']' || chars[j] == '}') {
                // Skip this trailing comma
                continue;
            }
        }

        result.push(c);
    }

    result
}

async fn write_openteams_cli_config_to_disk(
    config: &OpenTeamsCliConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = OpenTeamsCliConfig::config_path()
        .ok_or("Cannot determine openteams-cli config directory")?;

    let content = serde_json::to_string_pretty(config)?;

    // 复用安全写入逻辑（Windows 上 fs::rename 不能覆盖已有文件）
    write_secure_cli_config_file(path, content).await?;

    Ok(())
}
