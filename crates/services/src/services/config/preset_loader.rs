use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result, anyhow, bail};
use db::models::member_execution_config::MemberExecutionConfig;
use rust_embed::RustEmbed;
use serde::Deserialize;
use utils::{path::home_directory, text::sanitize_member_handle};

use crate::services::config::versions::v9::{
    ChatMemberPreset, ChatPresetsConfig, ChatTeamPreset, ChatTeamTemplateTier, ChatWorkflowStep,
};

const TEAM_COLLABORATION_PROTOCOL_FILE: &str = "team_collaboration_protocol.md";

#[derive(RustEmbed)]
#[folder = "src/services/config/presets/protocol/"]
struct EmbeddedTeamProtocolFiles;

const ROLE_PRESET_MARKDOWN: &[(&str, &str)] = &[
    (
        "coordinator_pmo.md",
        include_str!("presets/roles/coordinator_pmo.md"),
    ),
    (
        "product_manager.md",
        include_str!("presets/roles/product_manager.md"),
    ),
    (
        "system_architect.md",
        include_str!("presets/roles/system_architect.md"),
    ),
    (
        "prompt_engineer.md",
        include_str!("presets/roles/prompt_engineer.md"),
    ),
    (
        "frontend_engineer.md",
        include_str!("presets/roles/frontend_engineer.md"),
    ),
    (
        "backend_engineer.md",
        include_str!("presets/roles/backend_engineer.md"),
    ),
    (
        "fullstack_engineer.md",
        include_str!("presets/roles/fullstack_engineer.md"),
    ),
    ("qa_tester.md", include_str!("presets/roles/qa_tester.md")),
    (
        "ux_ui_designer.md",
        include_str!("presets/roles/ux_ui_designer.md"),
    ),
    (
        "safety_policy_officer.md",
        include_str!("presets/roles/safety_policy_officer.md"),
    ),
    (
        "solution_manager.md",
        include_str!("presets/roles/solution_manager.md"),
    ),
    (
        "code_reviewer.md",
        include_str!("presets/roles/code_reviewer.md"),
    ),
    (
        "devops_engineer.md",
        include_str!("presets/roles/devops_engineer.md"),
    ),
    (
        "product_analyst.md",
        include_str!("presets/roles/product_analyst.md"),
    ),
    (
        "data_analyst.md",
        include_str!("presets/roles/data_analyst.md"),
    ),
    (
        "technical_writer.md",
        include_str!("presets/roles/technical_writer.md"),
    ),
    (
        "content_researcher.md",
        include_str!("presets/roles/content_researcher.md"),
    ),
    (
        "content_editor.md",
        include_str!("presets/roles/content_editor.md"),
    ),
    (
        "frontier_researcher.md",
        include_str!("presets/roles/frontier_researcher.md"),
    ),
    (
        "marketing_specialist.md",
        include_str!("presets/roles/marketing_specialist.md"),
    ),
    (
        "video_editor.md",
        include_str!("presets/roles/video_editor.md"),
    ),
    (
        "market_analyst.md",
        include_str!("presets/roles/market_analyst.md"),
    ),
    // New agents (147 total) - alphabetically sorted
    (
        "accessibility-auditor.md",
        include_str!("presets/roles/accessibility-auditor.md"),
    ),
    (
        "account-strategist.md",
        include_str!("presets/roles/account-strategist.md"),
    ),
    (
        "accounts-payable-agent.md",
        include_str!("presets/roles/accounts-payable-agent.md"),
    ),
    (
        "ad-creative-strategist.md",
        include_str!("presets/roles/ad-creative-strategist.md"),
    ),
    (
        "agentic-identity-trust-architect.md",
        include_str!("presets/roles/agentic-identity-trust-architect.md"),
    ),
    (
        "agents-orchestrator.md",
        include_str!("presets/roles/agents-orchestrator.md"),
    ),
    (
        "ai-data-remediation-engineer.md",
        include_str!("presets/roles/ai-data-remediation-engineer.md"),
    ),
    (
        "ai-engineer.md",
        include_str!("presets/roles/ai-engineer.md"),
    ),
    (
        "analytics-reporter.md",
        include_str!("presets/roles/analytics-reporter.md"),
    ),
    ("api-tester.md", include_str!("presets/roles/api-tester.md")),
    (
        "app-store-optimizer.md",
        include_str!("presets/roles/app-store-optimizer.md"),
    ),
    (
        "automation-governance-architect.md",
        include_str!("presets/roles/automation-governance-architect.md"),
    ),
    (
        "autonomous-optimization-architect.md",
        include_str!("presets/roles/autonomous-optimization-architect.md"),
    ),
    (
        "backend-architect.md",
        include_str!("presets/roles/backend-architect.md"),
    ),
    (
        "baidu-seo-specialist.md",
        include_str!("presets/roles/baidu-seo-specialist.md"),
    ),
    (
        "behavioral-nudge-engine.md",
        include_str!("presets/roles/behavioral-nudge-engine.md"),
    ),
    (
        "bilibili-content-strategist.md",
        include_str!("presets/roles/bilibili-content-strategist.md"),
    ),
    (
        "blockchain-security-auditor.md",
        include_str!("presets/roles/blockchain-security-auditor.md"),
    ),
    (
        "book-co-author.md",
        include_str!("presets/roles/book-co-author.md"),
    ),
    (
        "brand-guardian.md",
        include_str!("presets/roles/brand-guardian.md"),
    ),
    (
        "carousel-growth-engine.md",
        include_str!("presets/roles/carousel-growth-engine.md"),
    ),
    (
        "china-e-commerce-operator.md",
        include_str!("presets/roles/china-e-commerce-operator.md"),
    ),
    (
        "code-reviewer.md",
        include_str!("presets/roles/code-reviewer.md"),
    ),
    (
        "compliance-auditor.md",
        include_str!("presets/roles/compliance-auditor.md"),
    ),
    (
        "content-creator.md",
        include_str!("presets/roles/content-creator.md"),
    ),
    (
        "corporate-training-designer.md",
        include_str!("presets/roles/corporate-training-designer.md"),
    ),
    (
        "cross-border-e-commerce-specialist.md",
        include_str!("presets/roles/cross-border-e-commerce-specialist.md"),
    ),
    (
        "cultural-intelligence-strategist.md",
        include_str!("presets/roles/cultural-intelligence-strategist.md"),
    ),
    (
        "data-consolidation-agent.md",
        include_str!("presets/roles/data-consolidation-agent.md"),
    ),
    (
        "data-engineer.md",
        include_str!("presets/roles/data-engineer.md"),
    ),
    (
        "database-optimizer.md",
        include_str!("presets/roles/database-optimizer.md"),
    ),
    (
        "deal-strategist.md",
        include_str!("presets/roles/deal-strategist.md"),
    ),
    (
        "developer-advocate.md",
        include_str!("presets/roles/developer-advocate.md"),
    ),
    (
        "devops-automator.md",
        include_str!("presets/roles/devops-automator.md"),
    ),
    (
        "discovery-coach.md",
        include_str!("presets/roles/discovery-coach.md"),
    ),
    (
        "document-generator.md",
        include_str!("presets/roles/document-generator.md"),
    ),
    (
        "douyin-strategist.md",
        include_str!("presets/roles/douyin-strategist.md"),
    ),
    (
        "embedded-firmware-engineer.md",
        include_str!("presets/roles/embedded-firmware-engineer.md"),
    ),
    (
        "evidence-collector.md",
        include_str!("presets/roles/evidence-collector.md"),
    ),
    (
        "executive-summary-generator.md",
        include_str!("presets/roles/executive-summary-generator.md"),
    ),
    (
        "experiment-tracker.md",
        include_str!("presets/roles/experiment-tracker.md"),
    ),
    (
        "feedback-synthesizer.md",
        include_str!("presets/roles/feedback-synthesizer.md"),
    ),
    (
        "feishu-integration-developer.md",
        include_str!("presets/roles/feishu-integration-developer.md"),
    ),
    (
        "finance-tracker.md",
        include_str!("presets/roles/finance-tracker.md"),
    ),
    (
        "frontend-developer.md",
        include_str!("presets/roles/frontend-developer.md"),
    ),
    (
        "game-audio-engineer.md",
        include_str!("presets/roles/game-audio-engineer.md"),
    ),
    (
        "game-designer.md",
        include_str!("presets/roles/game-designer.md"),
    ),
    (
        "git-workflow-master.md",
        include_str!("presets/roles/git-workflow-master.md"),
    ),
    (
        "godot-gameplay-scripter.md",
        include_str!("presets/roles/godot-gameplay-scripter.md"),
    ),
    (
        "godot-multiplayer-engineer.md",
        include_str!("presets/roles/godot-multiplayer-engineer.md"),
    ),
    (
        "godot-shader-developer.md",
        include_str!("presets/roles/godot-shader-developer.md"),
    ),
    (
        "government-digital-presales-consultant.md",
        include_str!("presets/roles/government-digital-presales-consultant.md"),
    ),
    (
        "growth-hacker.md",
        include_str!("presets/roles/growth-hacker.md"),
    ),
    (
        "healthcare-marketing-compliance-specialist.md",
        include_str!("presets/roles/healthcare-marketing-compliance-specialist.md"),
    ),
    (
        "identity-graph-operator.md",
        include_str!("presets/roles/identity-graph-operator.md"),
    ),
    (
        "image-prompt-engineer.md",
        include_str!("presets/roles/image-prompt-engineer.md"),
    ),
    (
        "incident-response-commander.md",
        include_str!("presets/roles/incident-response-commander.md"),
    ),
    (
        "inclusive-visuals-specialist.md",
        include_str!("presets/roles/inclusive-visuals-specialist.md"),
    ),
    (
        "infrastructure-maintainer.md",
        include_str!("presets/roles/infrastructure-maintainer.md"),
    ),
    (
        "instagram-curator.md",
        include_str!("presets/roles/instagram-curator.md"),
    ),
    (
        "jira-workflow-steward.md",
        include_str!("presets/roles/jira-workflow-steward.md"),
    ),
    (
        "kuaishou-strategist.md",
        include_str!("presets/roles/kuaishou-strategist.md"),
    ),
    (
        "legal-compliance-checker.md",
        include_str!("presets/roles/legal-compliance-checker.md"),
    ),
    (
        "level-designer.md",
        include_str!("presets/roles/level-designer.md"),
    ),
    (
        "linkedin-content-creator.md",
        include_str!("presets/roles/linkedin-content-creator.md"),
    ),
    (
        "literature-research-assistant.md",
        include_str!("presets/roles/literature_research_assistant.md"),
    ),
    (
        "literature-research-reviewer.md",
        include_str!("presets/roles/literature_research_reviewer.md"),
    ),
    (
        "literature-review-researcher.md",
        include_str!("presets/roles/literature_review_researcher.md"),
    ),
    (
        "livestream-commerce-coach.md",
        include_str!("presets/roles/livestream-commerce-coach.md"),
    ),
    (
        "lsp-index-engineer.md",
        include_str!("presets/roles/lsp-index-engineer.md"),
    ),
    (
        "macos-spatial-metal-engineer.md",
        include_str!("presets/roles/macos-spatial-metal-engineer.md"),
    ),
    (
        "mcp-builder.md",
        include_str!("presets/roles/mcp-builder.md"),
    ),
    (
        "mobile-app-builder.md",
        include_str!("presets/roles/mobile-app-builder.md"),
    ),
    (
        "model-qa-specialist.md",
        include_str!("presets/roles/model-qa-specialist.md"),
    ),
    (
        "narrative-designer.md",
        include_str!("presets/roles/narrative-designer.md"),
    ),
    (
        "outbound-strategist.md",
        include_str!("presets/roles/outbound-strategist.md"),
    ),
    (
        "paid-media-auditor.md",
        include_str!("presets/roles/paid-media-auditor.md"),
    ),
    (
        "paid-social-strategist.md",
        include_str!("presets/roles/paid-social-strategist.md"),
    ),
    (
        "performance-benchmarker.md",
        include_str!("presets/roles/performance-benchmarker.md"),
    ),
    (
        "pipeline-analyst.md",
        include_str!("presets/roles/pipeline-analyst.md"),
    ),
    (
        "podcast-strategist.md",
        include_str!("presets/roles/podcast-strategist.md"),
    ),
    (
        "ppc-campaign-strategist.md",
        include_str!("presets/roles/ppc-campaign-strategist.md"),
    ),
    (
        "private-domain-operator.md",
        include_str!("presets/roles/private-domain-operator.md"),
    ),
    (
        "programmatic-display-buyer.md",
        include_str!("presets/roles/programmatic-display-buyer.md"),
    ),
    (
        "project-shepherd.md",
        include_str!("presets/roles/project-shepherd.md"),
    ),
    (
        "proposal-strategist.md",
        include_str!("presets/roles/proposal-strategist.md"),
    ),
    (
        "rapid-prototyper.md",
        include_str!("presets/roles/rapid-prototyper.md"),
    ),
    (
        "reality-checker.md",
        include_str!("presets/roles/reality-checker.md"),
    ),
    (
        "recruitment-specialist.md",
        include_str!("presets/roles/recruitment-specialist.md"),
    ),
    (
        "reddit-community-builder.md",
        include_str!("presets/roles/reddit-community-builder.md"),
    ),
    (
        "report-distribution-agent.md",
        include_str!("presets/roles/report-distribution-agent.md"),
    ),
    (
        "roblox-avatar-creator.md",
        include_str!("presets/roles/roblox-avatar-creator.md"),
    ),
    (
        "roblox-experience-designer.md",
        include_str!("presets/roles/roblox-experience-designer.md"),
    ),
    (
        "roblox-systems-scripter.md",
        include_str!("presets/roles/roblox-systems-scripter.md"),
    ),
    (
        "sales-coach.md",
        include_str!("presets/roles/sales-coach.md"),
    ),
    (
        "sales-data-extraction-agent.md",
        include_str!("presets/roles/sales-data-extraction-agent.md"),
    ),
    (
        "sales-engineer.md",
        include_str!("presets/roles/sales-engineer.md"),
    ),
    (
        "search-query-analyst.md",
        include_str!("presets/roles/search-query-analyst.md"),
    ),
    (
        "security-engineer.md",
        include_str!("presets/roles/security-engineer.md"),
    ),
    (
        "senior-developer.md",
        include_str!("presets/roles/senior-developer.md"),
    ),
    (
        "senior-project-manager.md",
        include_str!("presets/roles/senior-project-manager.md"),
    ),
    (
        "seo-specialist.md",
        include_str!("presets/roles/seo-specialist.md"),
    ),
    (
        "short-video-editing-coach.md",
        include_str!("presets/roles/short-video-editing-coach.md"),
    ),
    (
        "social-media-strategist.md",
        include_str!("presets/roles/social-media-strategist.md"),
    ),
    (
        "software-architect.md",
        include_str!("presets/roles/software-architect.md"),
    ),
    (
        "solidity-smart-contract-engineer.md",
        include_str!("presets/roles/solidity-smart-contract-engineer.md"),
    ),
    (
        "sprint-prioritizer.md",
        include_str!("presets/roles/sprint-prioritizer.md"),
    ),
    (
        "sre-site-reliability-engineer.md",
        include_str!("presets/roles/sre-site-reliability-engineer.md"),
    ),
    (
        "studio-operations.md",
        include_str!("presets/roles/studio-operations.md"),
    ),
    (
        "studio-producer.md",
        include_str!("presets/roles/studio-producer.md"),
    ),
    (
        "study-abroad-advisor.md",
        include_str!("presets/roles/study-abroad-advisor.md"),
    ),
    (
        "supply-chain-strategist.md",
        include_str!("presets/roles/supply-chain-strategist.md"),
    ),
    (
        "support-responder.md",
        include_str!("presets/roles/support-responder.md"),
    ),
    (
        "technical-artist.md",
        include_str!("presets/roles/technical-artist.md"),
    ),
    (
        "technical-writer.md",
        include_str!("presets/roles/technical-writer.md"),
    ),
    (
        "terminal-integration-specialist.md",
        include_str!("presets/roles/terminal-integration-specialist.md"),
    ),
    (
        "test-results-analyzer.md",
        include_str!("presets/roles/test-results-analyzer.md"),
    ),
    (
        "threat-detection-engineer.md",
        include_str!("presets/roles/threat-detection-engineer.md"),
    ),
    (
        "tiktok-strategist.md",
        include_str!("presets/roles/tiktok-strategist.md"),
    ),
    (
        "tool-evaluator.md",
        include_str!("presets/roles/tool-evaluator.md"),
    ),
    (
        "tracking-measurement-specialist.md",
        include_str!("presets/roles/tracking-measurement-specialist.md"),
    ),
    (
        "trend-researcher.md",
        include_str!("presets/roles/trend-researcher.md"),
    ),
    (
        "twitter-engager.md",
        include_str!("presets/roles/twitter-engager.md"),
    ),
    (
        "ui-designer.md",
        include_str!("presets/roles/ui-designer.md"),
    ),
    (
        "unity-architect.md",
        include_str!("presets/roles/unity-architect.md"),
    ),
    (
        "unity-editor-tool-developer.md",
        include_str!("presets/roles/unity-editor-tool-developer.md"),
    ),
    (
        "unity-multiplayer-engineer.md",
        include_str!("presets/roles/unity-multiplayer-engineer.md"),
    ),
    (
        "unity-shader-graph-artist.md",
        include_str!("presets/roles/unity-shader-graph-artist.md"),
    ),
    (
        "unreal-multiplayer-architect.md",
        include_str!("presets/roles/unreal-multiplayer-architect.md"),
    ),
    (
        "unreal-systems-engineer.md",
        include_str!("presets/roles/unreal-systems-engineer.md"),
    ),
    (
        "unreal-technical-artist.md",
        include_str!("presets/roles/unreal-technical-artist.md"),
    ),
    (
        "unreal-world-builder.md",
        include_str!("presets/roles/unreal-world-builder.md"),
    ),
    (
        "ux-architect.md",
        include_str!("presets/roles/ux-architect.md"),
    ),
    (
        "ux-researcher.md",
        include_str!("presets/roles/ux-researcher.md"),
    ),
    (
        "visionos-spatial-engineer.md",
        include_str!("presets/roles/visionos-spatial-engineer.md"),
    ),
    (
        "visual-storyteller.md",
        include_str!("presets/roles/visual-storyteller.md"),
    ),
    (
        "wechat-mini-program-developer.md",
        include_str!("presets/roles/wechat-mini-program-developer.md"),
    ),
    (
        "wechat-official-account-manager.md",
        include_str!("presets/roles/wechat-official-account-manager.md"),
    ),
    (
        "weibo-strategist.md",
        include_str!("presets/roles/weibo-strategist.md"),
    ),
    (
        "whimsy-injector.md",
        include_str!("presets/roles/whimsy-injector.md"),
    ),
    (
        "workflow-optimizer.md",
        include_str!("presets/roles/workflow-optimizer.md"),
    ),
    (
        "xiaohongshu-specialist.md",
        include_str!("presets/roles/xiaohongshu-specialist.md"),
    ),
    (
        "xr-cockpit-interaction-specialist.md",
        include_str!("presets/roles/xr-cockpit-interaction-specialist.md"),
    ),
    (
        "xr-immersive-developer.md",
        include_str!("presets/roles/xr-immersive-developer.md"),
    ),
    (
        "xr-interface-architect.md",
        include_str!("presets/roles/xr-interface-architect.md"),
    ),
    (
        "zhihu-strategist.md",
        include_str!("presets/roles/zhihu-strategist.md"),
    ),
    ("zk-steward.md", include_str!("presets/roles/zk-steward.md")),
];

#[derive(Debug, Deserialize)]
struct RolePresetFrontmatter {
    id: String,
    name: String,
    description: String,
    #[serde(default, alias = "default_workspace_path")]
    default_workspace: Option<String>,
    #[serde(default, alias = "allowed_skill_ids")]
    selected_skill_ids: Vec<String>,
    #[serde(default)]
    runner_type: Option<String>,
    #[serde(default)]
    recommended_model: Option<String>,
    #[serde(default)]
    execution_config: Option<MemberExecutionConfig>,
    #[serde(default)]
    tools_enabled: Option<serde_yaml::Value>,
}

#[derive(Debug, PartialEq, Eq)]
struct RolePresetMd {
    id: String,
    name: String,
    description: String,
    role_definition: String,
    default_workspace_path: Option<String>,
    selected_skill_ids: Vec<String>,
    runner_type: Option<String>,
    recommended_model: Option<String>,
    execution_config: Option<MemberExecutionConfig>,
    tools_enabled: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct TeamPresetFrontmatter {
    id: String,
    name: String,
    description: String,
    #[serde(default)]
    member_ids: Vec<String>,
    #[serde(default)]
    members: Vec<TeamMemberPresetInstance>,
    #[serde(default)]
    lead_member_id: Option<String>,
    #[serde(default)]
    workflow_steps: Vec<ChatWorkflowStep>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_team_tier")]
    tier: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct TeamMemberPresetInstance {
    preset_id: String,
    id: String,
    name: String,
}

#[derive(Debug, PartialEq, Eq)]
struct TeamPresetMd {
    id: String,
    name: String,
    description: String,
    member_ids: Vec<String>,
    members: Vec<TeamMemberPresetInstance>,
    lead_member_id: Option<String>,
    workflow_steps: Vec<ChatWorkflowStep>,
    team_protocol: String,
    enabled: bool,
    tier: ChatTeamTemplateTier,
}

impl TeamPresetMd {
    fn instantiate_members(
        &self,
        member_by_id: &HashMap<&str, &ChatMemberPreset>,
    ) -> Result<Vec<ChatMemberPreset>> {
        if !self.members.is_empty() {
            return self
                .members
                .iter()
                .map(|instance| {
                    let mut member = member_by_id
                        .get(instance.preset_id.as_str())
                        .copied()
                        .cloned()
                        .ok_or_else(|| {
                            anyhow!(
                                "built-in team \"{}\" references unknown member preset: {}",
                                self.id,
                                instance.preset_id
                            )
                        })?;
                    member.id = instance.id.clone();
                    member.name = instance.name.clone();
                    Ok(member)
                })
                .collect();
        }

        self.member_ids
            .iter()
            .map(|member_id| {
                member_by_id
                    .get(member_id.as_str())
                    .copied()
                    .cloned()
                    .ok_or_else(|| {
                        anyhow!(
                            "built-in team \"{}\" references unknown member preset: {}",
                            self.id,
                            member_id
                        )
                    })
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuiltinTeamTemplateCatalogEntry {
    pub template_id: String,
    pub tier: String,
    pub enabled: bool,
    pub sort_order: i64,
    pub source_content: String,
}

pub struct PresetLoader;

const TEAM_TEMPLATE_LOCALES: &[&str] = &["en", "zh", "ja", "ko", "fr", "es"];

impl PresetLoader {
    pub fn load_builtin_presets() -> ChatPresetsConfig {
        Self::load_builtin_presets_for_locale(None)
    }

    pub fn load_builtin_presets_for_locale(locale: Option<&str>) -> ChatPresetsConfig {
        let locale = normalize_team_template_locale(locale);
        let default_workspace_path = home_directory().to_string_lossy().to_string();
        let members = ROLE_PRESET_MARKDOWN
            .iter()
            .map(|(path, raw)| Self::parse_chat_member_preset(path, raw, &default_workspace_path))
            .collect::<Result<Vec<_>>>()
            .expect("built-in role preset markdown should be valid");
        let member_by_id: HashMap<&str, &ChatMemberPreset> = members
            .iter()
            .map(|member| (member.id.as_str(), member))
            .collect();
        let files = Self::embedded_team_preset_markdown_files()
            .expect("built-in team preset markdown files should be embedded")
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let teams = files
            .iter()
            .filter(|(path, _)| path.starts_with("en/"))
            .map(|(english_path, english_raw)| {
                Self::localized_team_preset_markdown(&files, english_path, english_raw, locale)
            })
            .map(|team_result| {
                team_result.and_then(|team_md| {
                    let team_members = team_md.instantiate_members(&member_by_id)?;
                    if let Some(ref lead_member_id) = team_md.lead_member_id
                        && !team_members
                            .iter()
                            .any(|member| &member.id == lead_member_id)
                    {
                        bail!(
                            "built-in team \"{}\" lead_member_id references unknown member: {}",
                            team_md.id,
                            lead_member_id
                        );
                    }
                    Ok(ChatTeamPreset {
                        id: team_md.id,
                        name: team_md.name,
                        description: team_md.description,
                        members: team_members,
                        lead_member_id: team_md.lead_member_id,
                        workflow_steps: team_md.workflow_steps,
                        team_protocol: team_md.team_protocol,
                        is_builtin: true,
                        enabled: team_md.enabled,
                        tier: team_md.tier,
                    })
                })
            })
            .collect::<Result<Vec<_>>>()
            .expect("built-in team preset markdown should be valid");

        ChatPresetsConfig {
            members,
            teams,
            team_protocol: None,
        }
    }

    pub fn load_team_protocol() -> String {
        Self::try_load_team_protocol()
            .expect("built-in team collaboration protocol markdown should be valid")
    }

    fn try_load_team_protocol() -> Result<String> {
        let protocol = normalize_newlines(&Self::embedded_markdown_file(
            TEAM_COLLABORATION_PROTOCOL_FILE,
        )?)
        .trim()
        .to_string();
        if protocol.is_empty() {
            bail!("built-in team collaboration protocol is empty");
        }

        Ok(protocol)
    }

    fn embedded_team_preset_markdown_files() -> Result<Vec<(String, String)>> {
        let mut markdown_files = EmbeddedTeamProtocolFiles::iter()
            .filter(|path| {
                path.ends_with(".md") && path.as_ref() != TEAM_COLLABORATION_PROTOCOL_FILE
            })
            .map(|path| {
                let path = path.into_owned();
                let raw = Self::embedded_markdown_file(&path)?;
                Ok((path, raw))
            })
            .collect::<Result<Vec<_>>>()?;

        markdown_files.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(markdown_files)
    }

    pub(crate) fn load_builtin_team_template_catalog_entries()
    -> Result<Vec<BuiltinTeamTemplateCatalogEntry>> {
        let files = Self::embedded_team_preset_markdown_files()?;
        Self::load_builtin_team_template_catalog_entries_from_files(files)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn load_builtin_team_template_catalog_entries_from_files(
        files: Vec<(String, String)>,
    ) -> Result<Vec<BuiltinTeamTemplateCatalogEntry>> {
        let files_by_path = files.iter().cloned().collect::<BTreeMap<_, _>>();
        let mut entries = files
            .iter()
            .filter(|(path, _)| path.starts_with("en/"))
            .enumerate()
            .map(|(index, (path, raw))| {
                let parsed = Self::parse_team_preset_markdown(path, raw)?;
                let file_name = path.strip_prefix("en/").ok_or_else(|| {
                    anyhow!("English team template path must start with en/: {path}")
                })?;
                let source_content = TEAM_TEMPLATE_LOCALES
                    .iter()
                    .filter_map(|locale| {
                        let localized_path = format!("{locale}/{file_name}");
                        files_by_path
                            .get(&localized_path)
                            .map(|raw| (localized_path, raw))
                    })
                    .map(|(path, raw)| format!("{path}\n{}\n", normalize_newlines(raw)))
                    .collect::<String>();
                Ok(BuiltinTeamTemplateCatalogEntry {
                    template_id: parsed.id,
                    tier: team_template_tier_wire_value(parsed.tier).to_string(),
                    enabled: parsed.enabled,
                    sort_order: index as i64,
                    source_content,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        entries.sort_by(|left, right| left.sort_order.cmp(&right.sort_order));
        Ok(entries)
    }

    fn embedded_markdown_file(path: &str) -> Result<String> {
        let asset = EmbeddedTeamProtocolFiles::get(path)
            .ok_or_else(|| anyhow!("missing embedded team protocol markdown: {path}"))?;
        String::from_utf8(asset.data.into_owned())
            .with_context(|| format!("embedded team protocol markdown is not UTF-8: {path}"))
    }

    fn localized_team_preset_markdown(
        files: &BTreeMap<String, String>,
        english_path: &str,
        english_raw: &str,
        locale: &str,
    ) -> Result<TeamPresetMd> {
        let english = Self::parse_team_preset_markdown(english_path, english_raw)
            .with_context(|| format!("failed to parse English team template {english_path}"))?;
        if locale == "en" {
            return Ok(english);
        }

        let file_name = english_path.strip_prefix("en/").ok_or_else(|| {
            anyhow!("English team template path must start with en/: {english_path}")
        })?;
        let localized_path = format!("{locale}/{file_name}");
        let Some(localized_raw) = files.get(&localized_path) else {
            warn_team_template_locale_fallback(&english.id, locale, "localized file is missing");
            return Ok(english);
        };

        match Self::parse_team_preset_markdown(&localized_path, localized_raw).and_then(
            |localized| {
                validate_localized_team_template(&english, localized_path.as_str(), localized)
            },
        ) {
            Ok(localized) => Ok(localized),
            Err(error) => {
                #[cfg(test)]
                record_team_template_locale_fallback_warning(
                    &english.id,
                    locale,
                    &error.to_string(),
                );
                tracing::warn!(
                    template_id = %english.id,
                    locale,
                    error = ?error,
                    "falling back to English team template localization"
                );
                Ok(english)
            }
        }
    }

    fn parse_chat_member_preset(
        path: &str,
        raw: &str,
        default_workspace_path: &str,
    ) -> Result<ChatMemberPreset> {
        let preset = Self::parse_role_preset_markdown(path, raw)?;
        Ok(ChatMemberPreset {
            id: preset.id,
            name: sanitize_member_handle(&preset.name),
            description: preset.description,
            runner_type: preset.runner_type,
            recommended_model: preset.recommended_model,
            execution_config: preset.execution_config,
            system_prompt: preset.role_definition,
            default_workspace_path: Some(default_workspace_path.to_string()),
            selected_skill_ids: preset.selected_skill_ids,
            tools_enabled: preset.tools_enabled,
            is_builtin: true,
            enabled: true,
        })
    }

    fn parse_role_preset_markdown(path: &str, raw: &str) -> Result<RolePresetMd> {
        let normalized = normalize_newlines(raw);
        let (frontmatter_raw, body) = split_frontmatter(&normalized)
            .ok_or_else(|| anyhow!("missing frontmatter delimiters in {path}"))?;
        let frontmatter: RolePresetFrontmatter = serde_yaml::from_str(frontmatter_raw)
            .with_context(|| format!("failed to parse frontmatter in {path}"))?;
        let role_definition = body.trim().to_string();

        if frontmatter.id.trim().is_empty()
            || frontmatter.name.trim().is_empty()
            || frontmatter.description.trim().is_empty()
        {
            bail!("role preset frontmatter contains empty required fields in {path}");
        }
        if role_definition.is_empty() {
            bail!("role preset body is empty in {path}");
        }

        if let Some(mcp) = frontmatter
            .execution_config
            .as_ref()
            .and_then(|config| config.mcp.as_ref())
        {
            mcp.validate(&frontmatter.id)
                .with_context(|| format!("invalid member MCP configuration in {path}"))?;
        }

        let tools_enabled = match frontmatter.tools_enabled {
            Some(value) => serde_json::to_value(value)
                .with_context(|| format!("failed to convert tools_enabled in {path}"))?,
            None => serde_json::json!({}),
        };

        Ok(RolePresetMd {
            id: frontmatter.id,
            name: frontmatter.name,
            description: frontmatter.description,
            role_definition,
            default_workspace_path: frontmatter.default_workspace,
            selected_skill_ids: normalize_selected_skill_ids(frontmatter.selected_skill_ids),
            runner_type: frontmatter.runner_type,
            recommended_model: frontmatter.recommended_model,
            execution_config: frontmatter.execution_config,
            tools_enabled,
        })
    }

    fn parse_team_preset_markdown(path: &str, raw: &str) -> Result<TeamPresetMd> {
        let normalized = normalize_newlines(raw);
        let (frontmatter_raw, body) = split_frontmatter(&normalized)
            .ok_or_else(|| anyhow!("missing frontmatter delimiters in {path}"))?;
        let frontmatter: TeamPresetFrontmatter = serde_yaml::from_str(frontmatter_raw)
            .with_context(|| format!("failed to parse frontmatter in {path}"))?;

        if frontmatter.id.trim().is_empty()
            || frontmatter.name.trim().is_empty()
            || frontmatter.description.trim().is_empty()
        {
            bail!("team preset frontmatter contains empty required fields in {path}");
        }

        let member_ids = normalize_member_ids(frontmatter.member_ids);
        let members = normalize_team_member_instances(frontmatter.members, path)?;
        if !member_ids.is_empty() && !members.is_empty() {
            bail!("team preset must use either members or member_ids, not both, in {path}");
        }
        if member_ids.is_empty()
            && members.is_empty()
            && (path.starts_with("en/") || !path.contains('/'))
        {
            bail!("team preset contains no members in {path}");
        }

        Ok(TeamPresetMd {
            id: frontmatter.id,
            name: frontmatter.name,
            description: frontmatter.description,
            member_ids,
            members,
            lead_member_id: frontmatter
                .lead_member_id
                .map(|lead_member_id| lead_member_id.trim().to_string())
                .filter(|lead_member_id| !lead_member_id.is_empty()),
            workflow_steps: frontmatter.workflow_steps,
            team_protocol: body.trim().to_string(),
            enabled: frontmatter.enabled,
            tier: normalize_team_template_tier(&frontmatter.tier, path)?,
        })
    }
}

fn team_template_tier_wire_value(tier: ChatTeamTemplateTier) -> &'static str {
    match tier {
        ChatTeamTemplateTier::Standard => "standard",
        ChatTeamTemplateTier::Advanced => "advanced",
    }
}

fn default_true() -> bool {
    true
}

fn default_team_tier() -> String {
    "standard".to_string()
}

fn normalize_team_template_tier(tier: &str, path: &str) -> Result<ChatTeamTemplateTier> {
    match tier.trim() {
        "standard" => Ok(ChatTeamTemplateTier::Standard),
        "advanced" => Ok(ChatTeamTemplateTier::Advanced),
        value => bail!("unsupported team template tier \"{value}\" in {path}"),
    }
}

pub fn normalize_team_template_locale(locale: Option<&str>) -> &'static str {
    let Some(locale) = locale else {
        return "en";
    };
    let normalized = locale.trim().to_ascii_lowercase().replace('_', "-");
    let language = normalized.split('-').next().unwrap_or_default();
    match language {
        "en" => "en",
        "zh" | "cn" => "zh",
        "ja" | "jp" => "ja",
        "ko" | "kr" => "ko",
        "fr" => "fr",
        "es" => "es",
        _ => "en",
    }
}

fn validate_localized_team_template(
    english: &TeamPresetMd,
    path: &str,
    mut localized: TeamPresetMd,
) -> Result<TeamPresetMd> {
    if localized.id != english.id {
        bail!(
            "localized team template id mismatch in {path}: expected {}, got {}",
            english.id,
            localized.id
        );
    }
    if localized.name.trim().is_empty()
        || localized.description.trim().is_empty()
        || localized.team_protocol.trim().is_empty()
    {
        bail!("localized team template contains empty localized fields in {path}");
    }
    if localized.workflow_steps.len() != english.workflow_steps.len() {
        bail!(
            "localized team template workflow step count mismatch in {path}: expected {}, got {}",
            english.workflow_steps.len(),
            localized.workflow_steps.len()
        );
    }
    for (index, step) in localized.workflow_steps.iter().enumerate() {
        if step.title.trim().is_empty() || step.description.trim().is_empty() {
            bail!("localized team template workflow step {index} contains empty fields in {path}");
        }
    }
    localized.member_ids = english.member_ids.clone();
    localized.members = english.members.clone();
    localized.lead_member_id = english.lead_member_id.clone();
    localized.enabled = english.enabled;
    localized.tier = english.tier;
    Ok(localized)
}

fn warn_team_template_locale_fallback(template_id: &str, locale: &str, reason: &str) {
    #[cfg(test)]
    record_team_template_locale_fallback_warning(template_id, locale, reason);
    tracing::warn!(
        template_id,
        locale,
        reason,
        "falling back to English team template localization"
    );
}

#[cfg(test)]
fn record_team_template_locale_fallback_warning(template_id: &str, locale: &str, reason: &str) {
    TEAM_TEMPLATE_LOCALE_FALLBACK_WARNINGS.with(|warnings| {
        warnings.borrow_mut().push((
            template_id.to_string(),
            locale.to_string(),
            reason.to_string(),
        ));
    });
}

#[cfg(test)]
thread_local! {
    static TEAM_TEMPLATE_LOCALE_FALLBACK_WARNINGS: std::cell::RefCell<Vec<(String, String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn team_template_locale_fallback_warnings() -> Vec<(String, String, String)> {
    TEAM_TEMPLATE_LOCALE_FALLBACK_WARNINGS.with(|warnings| warnings.borrow().clone())
}

fn normalize_selected_skill_ids(skill_ids: Vec<String>) -> Vec<String> {
    let mut normalized = skill_ids
        .into_iter()
        .map(|skill_id| skill_id.trim().to_string())
        .filter(|skill_id| !skill_id.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_member_ids(member_ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    member_ids
        .into_iter()
        .map(|member_id| member_id.trim().to_string())
        .filter(|member_id| !member_id.is_empty())
        .filter(|member_id| seen.insert(member_id.clone()))
        .collect()
}

fn normalize_team_member_instances(
    members: Vec<TeamMemberPresetInstance>,
    path: &str,
) -> Result<Vec<TeamMemberPresetInstance>> {
    let mut seen_ids = HashSet::new();
    let mut seen_names = HashSet::new();

    members
        .into_iter()
        .enumerate()
        .map(|(index, mut member)| {
            member.preset_id = member.preset_id.trim().to_string();
            member.id = member.id.trim().to_string();
            member.name = sanitize_member_handle(member.name.trim());

            if member.preset_id.is_empty() {
                bail!("team preset member {index} has an empty preset_id in {path}");
            }
            if member.id.is_empty() {
                bail!("team preset member {index} has an empty id in {path}");
            }
            if !member.id.chars().all(|ch| {
                ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-'
            }) {
                bail!(
                    "team preset member id must contain only lowercase letters, numbers, underscores, or hyphens: {} in {path}",
                    member.id
                );
            }
            if member.name.is_empty() {
                bail!("team preset member {index} has an empty name in {path}");
            }
            if !seen_ids.insert(member.id.clone()) {
                bail!(
                    "team preset contains duplicate member instance id: {} in {path}",
                    member.id
                );
            }
            if !seen_names.insert(member.name.to_lowercase()) {
                bail!(
                    "team preset contains duplicate member instance name: {} in {path}",
                    member.name
                );
            }

            Ok(member)
        })
        .collect()
}

fn normalize_newlines(content: &str) -> String {
    content.replace("\r\n", "\n")
}

fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    if let Some(rest) = content.strip_prefix("---\n")
        && let Some((frontmatter, body)) = rest.split_once("\n---\n")
    {
        return Some((frontmatter, body));
    }

    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use db::models::member_execution_config::MemberExecutionConfig;
    use utils::path::home_directory;

    use super::{
        ChatMemberPreset, PresetLoader, TEAM_COLLABORATION_PROTOCOL_FILE, TeamMemberPresetInstance,
        normalize_team_template_locale,
    };

    fn assert_member_mcp_secrets_are_obviously_fake(member: &ChatMemberPreset) {
        let Some(mcp) = member
            .execution_config
            .as_ref()
            .and_then(|config| config.mcp.as_ref())
        else {
            return;
        };

        for (server_name, definition) in &mcp.mcp_servers {
            let Some(server) = definition.as_object() else {
                continue;
            };
            for field in ["env", "headers"] {
                let Some(values) = server.get(field).and_then(serde_json::Value::as_object) else {
                    continue;
                };
                for (key, value) in values {
                    let normalized_key = key.to_ascii_lowercase();
                    let is_secret_key = [
                        "authorization",
                        "cookie",
                        "password",
                        "secret",
                        "token",
                        "api_key",
                        "apikey",
                    ]
                    .iter()
                    .any(|marker| normalized_key.contains(marker));
                    if !is_secret_key {
                        continue;
                    }
                    let value = value.as_str().unwrap_or_default();
                    let normalized_value = value.to_ascii_lowercase();
                    let obviously_fake = [
                        "fake",
                        "test",
                        "example",
                        "placeholder",
                        "dummy",
                        "changeme",
                    ]
                    .iter()
                    .any(|marker| normalized_value.contains(marker))
                        || value.starts_with('$')
                        || value.starts_with('<')
                        || value.contains("{{");
                    assert!(
                        obviously_fake,
                        "built-in member `{}` server `{server_name}` contains a non-placeholder {field}.{key}",
                        member.id
                    );
                }
            }
        }
    }

    #[test]
    fn load_builtin_presets_reads_all_builtin_preset_markdown_files() {
        let presets = PresetLoader::load_builtin_presets();

        assert_eq!(presets.members.len(), 169); // 22 original + 147 new
        let embedded_team_files = PresetLoader::embedded_team_preset_markdown_files()
            .expect("embedded team markdown files should load");
        let english_team_file_count = embedded_team_files
            .iter()
            .filter(|(path, _)| path.starts_with("en/"))
            .count();
        assert_eq!(presets.teams.len(), english_team_file_count);

        let fullstack = presets
            .members
            .iter()
            .find(|preset| preset.id == "fullstack_engineer")
            .expect("fullstack preset should exist");
        assert!(fullstack.execution_config.is_none());
        assert_eq!(fullstack.name, "fullstack");
        let expected_workspace = home_directory().to_string_lossy().to_string();
        assert_eq!(
            fullstack.default_workspace_path.as_deref(),
            Some(expected_workspace.as_str())
        );
        assert!(fullstack.selected_skill_ids.is_empty());
        assert!(
            !fullstack
                .system_prompt
                .contains("Team Collaboration Protocol")
        );
        assert!(
            fullstack
                .system_prompt
                .contains("API-to-UI contract alignment and schema evolution control.")
        );

        let planner = presets
            .members
            .iter()
            .find(|preset| preset.id == "coordinator_pmo")
            .expect("planner preset should exist");
        assert_eq!(planner.runner_type.as_deref(), Some("OPENCODE"));
        assert_eq!(planner.recommended_model.as_deref(), Some("glm-5"));

        let designer = presets
            .members
            .iter()
            .find(|preset| preset.id == "ux_ui_designer")
            .expect("designer preset should exist");
        assert_eq!(designer.runner_type.as_deref(), Some("GEMINI"));
        assert_eq!(
            designer.recommended_model.as_deref(),
            Some("gemini-3-flash-preview")
        );
        let gemini_presets = presets
            .members
            .iter()
            .filter(|preset| preset.runner_type.as_deref() == Some("GEMINI"))
            .collect::<Vec<_>>();
        assert!(!gemini_presets.is_empty());
        for preset in gemini_presets {
            assert_eq!(
                preset.recommended_model.as_deref(),
                Some("gemini-3-flash-preview"),
                "Gemini preset `{}` must use a model advertised by the ACP probe",
                preset.id
            );
        }

        let codex_presets = presets
            .members
            .iter()
            .filter(|preset| preset.runner_type.as_deref() == Some("CODEX"))
            .collect::<Vec<_>>();
        assert!(!codex_presets.is_empty());
        for preset in codex_presets {
            assert!(
                matches!(
                    preset.recommended_model.as_deref(),
                    Some("gpt-6-astra" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna")
                ),
                "Codex preset `{}` must use a current OpenAI model",
                preset.id
            );
        }

        let team = presets
            .teams
            .iter()
            .find(|preset| preset.id == "fullstack_delivery_team")
            .expect("fullstack team preset should exist");
        assert_eq!(team.name, "Full-stack Delivery Team");
        assert_eq!(
            team.description,
            "Planner-led web delivery across design, frontend, backend, QA, and review."
        );
        let team_member_ids = team
            .members
            .iter()
            .map(|member| member.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            team_member_ids,
            vec![
                "coordinator_pmo".to_string(),
                "backend_engineer".to_string(),
                "frontend_engineer".to_string(),
                "qa_tester".to_string(),
            ]
        );
        assert!(!team.team_protocol.contains("`@`"));

        let blank_team = presets
            .teams
            .iter()
            .find(|preset| preset.id == "blank_team")
            .expect("blank team preset should exist");
        assert_eq!(blank_team.name, "Blank Team");
        assert_eq!(blank_team.members.len(), 1);
        assert_eq!(blank_team.members[0].id, "fullstack_engineer");
        assert_eq!(
            blank_team.lead_member_id.as_deref(),
            Some("fullstack_engineer")
        );
    }

    #[test]
    fn load_team_protocol_returns_embedded_markdown_content() {
        let protocol = PresetLoader::load_team_protocol();

        assert_eq!(protocol, "no team collaboration protocol");
    }

    #[test]
    fn builtin_repository_templates_do_not_embed_real_mcp_secrets() {
        let presets = PresetLoader::load_builtin_presets();

        for member in &presets.members {
            assert_member_mcp_secrets_are_obviously_fake(member);
        }
        for team in &presets.teams {
            for member in &team.members {
                assert_member_mcp_secrets_are_obviously_fake(member);
            }
        }
    }

    #[test]
    fn parse_role_preset_markdown_extracts_frontmatter_and_role_definition() {
        let markdown = r#"---
id: sample_role
name: sample
description: Sample role
default_workspace: samples
selected_skill_ids:
  - skill_b
  - skill_a
  - skill_b
runner_type: codex
recommended_model: gpt-5.2-codex
tools_enabled:
  shell: true
execution_config:
  runner_type: CODEX
  model_name: fake-model
  thinking_effort: high
  model_variant: fake-variant
  acp:
    access_mode: workspace_only
    approval_mode: auto_reject
    additional_directories: []
  mcp:
    mcpServers:
      fake-local:
        command: fake-mcp-server
        env:
          API_TOKEN: fake-markdown-token
---

# Role: Sample Role

## Goal
Ship a sample workflow.

## Role Focus
- Keep the contract explicit.
- Provide reproducible evidence.

## Definition of Done
The sample is shippable.

## Collaboration Notes
Coordinate with design before shipping.
"#;

        let parsed = PresetLoader::parse_role_preset_markdown("sample.md", markdown).unwrap();

        assert_eq!(parsed.id, "sample_role");
        assert_eq!(parsed.name, "sample");
        assert_eq!(parsed.description, "Sample role");
        assert_eq!(
            parsed.role_definition,
            r#"# Role: Sample Role

## Goal
Ship a sample workflow.

## Role Focus
- Keep the contract explicit.
- Provide reproducible evidence.

## Definition of Done
The sample is shippable.

## Collaboration Notes
Coordinate with design before shipping."#
        );
        assert_eq!(parsed.default_workspace_path.as_deref(), Some("samples"));
        assert_eq!(
            parsed.selected_skill_ids,
            vec!["skill_a".to_string(), "skill_b".to_string()]
        );
        assert_eq!(parsed.runner_type.as_deref(), Some("codex"));
        assert_eq!(parsed.recommended_model.as_deref(), Some("gpt-5.2-codex"));
        assert_eq!(parsed.tools_enabled, serde_json::json!({ "shell": true }));

        let member = PresetLoader::parse_chat_member_preset("sample.md", markdown, "/workspace")
            .expect("role markdown should build a member preset");
        assert_member_mcp_secrets_are_obviously_fake(&member);
        let execution_config = member
            .execution_config
            .as_ref()
            .expect("execution config should parse");
        assert_eq!(
            execution_config,
            &serde_json::from_value::<MemberExecutionConfig>(serde_json::json!({
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
                            "env": { "API_TOKEN": "fake-markdown-token" }
                        }
                    }
                }
            }))
            .expect("deserialize expected execution config")
        );
        let serialized = serde_json::to_value(member).expect("serialize member preset");
        assert_eq!(
            serialized["execution_config"],
            serde_json::json!({
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
                            "env": { "API_TOKEN": "fake-markdown-token" }
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn parse_role_preset_markdown_rejects_invalid_member_mcp_configuration() {
        let markdown = r#"---
id: invalid-mcp-role
name: invalid-mcp-role
description: Invalid MCP fixture
execution_config:
  mcp:
    mcpServers:
      playwright:
        command: 7
---

This role must not load.
"#;

        let error = PresetLoader::parse_role_preset_markdown("invalid-mcp.md", markdown)
            .expect_err("invalid member MCP configuration should fail");
        let error_chain = format!("{error:#}");

        assert!(error_chain.contains("invalid member MCP configuration in invalid-mcp.md"));
        assert!(error_chain.contains("mcpServers.playwright.command"));
    }

    #[test]
    fn parse_team_preset_markdown_extracts_frontmatter_members_and_protocol() {
        let markdown = r#"---
id: sample_team
name: Sample Team
description: Team description
member_ids:
  - backend_engineer
  - frontend_engineer
  - backend_engineer
---

Coordinate tightly and document every handoff.
- Backend owns API behavior.
- Frontend owns UX delivery.
"#;

        let parsed = PresetLoader::parse_team_preset_markdown("sample_team.md", markdown).unwrap();

        assert_eq!(parsed.id, "sample_team");
        assert_eq!(parsed.name, "Sample Team");
        assert_eq!(parsed.description, "Team description");
        assert_eq!(
            parsed.member_ids,
            vec![
                "backend_engineer".to_string(),
                "frontend_engineer".to_string()
            ]
        );
        assert_eq!(
            parsed.team_protocol,
            "Coordinate tightly and document every handoff.\n- Backend owns API behavior.\n- Frontend owns UX delivery."
        );
        assert_eq!(parsed.lead_member_id, None);
        assert!(parsed.members.is_empty());
        assert!(parsed.workflow_steps.is_empty());
        assert!(parsed.enabled);
    }

    #[test]
    fn parse_and_instantiate_repeated_team_member_preset_instances() {
        let markdown = r#"---
id: research_team
name: Research Team
description: Two independently addressable research assistants
members:
  - preset_id: literature-research-assistant
    id: research_assistant_primary
    name: Research Assistant Primary
  - preset_id: literature-research-assistant
    id: research_assistant_secondary
    name: Research Assistant Secondary
lead_member_id: research_assistant_primary
---

Search independently, then compare evidence.
"#;

        let parsed = PresetLoader::parse_team_preset_markdown("research_team.md", markdown)
            .expect("member preset instances should parse");

        assert!(parsed.member_ids.is_empty());
        assert_eq!(
            parsed.members,
            vec![
                TeamMemberPresetInstance {
                    preset_id: "literature-research-assistant".to_string(),
                    id: "research_assistant_primary".to_string(),
                    name: "ResearchAssistantPrimary".to_string(),
                },
                TeamMemberPresetInstance {
                    preset_id: "literature-research-assistant".to_string(),
                    id: "research_assistant_secondary".to_string(),
                    name: "ResearchAssistantSecondary".to_string(),
                },
            ]
        );

        let presets = PresetLoader::load_builtin_presets();
        let member_by_id = presets
            .members
            .iter()
            .map(|member| (member.id.as_str(), member))
            .collect::<HashMap<_, _>>();
        let source = member_by_id
            .get("literature-research-assistant")
            .expect("source member preset should exist");
        let instances = parsed
            .instantiate_members(&member_by_id)
            .expect("member instances should materialize");

        assert_eq!(
            instances
                .iter()
                .map(|member| (member.id.as_str(), member.name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("research_assistant_primary", "ResearchAssistantPrimary"),
                ("research_assistant_secondary", "ResearchAssistantSecondary"),
            ]
        );
        for instance in &instances {
            assert_eq!(instance.system_prompt, source.system_prompt);
            assert_eq!(instance.runner_type, source.runner_type);
            assert_eq!(instance.execution_config, source.execution_config);
            assert_eq!(instance.tools_enabled, source.tools_enabled);
        }
    }

    #[test]
    fn parse_team_preset_markdown_rejects_ambiguous_or_duplicate_instances() {
        let mixed_syntax = r#"---
id: invalid_team
name: Invalid Team
description: Invalid mixed syntax
member_ids:
  - backend_engineer
members:
  - preset_id: backend_engineer
    id: backend_primary
    name: BackendPrimary
---

Invalid.
"#;
        let duplicate_id = r#"---
id: invalid_team
name: Invalid Team
description: Duplicate instance IDs
members:
  - preset_id: backend_engineer
    id: backend
    name: BackendPrimary
  - preset_id: backend_engineer
    id: backend
    name: BackendSecondary
---

Invalid.
"#;
        let duplicate_name = r#"---
id: invalid_team
name: Invalid Team
description: Duplicate instance names
members:
  - preset_id: backend_engineer
    id: backend_primary
    name: Backend Agent
  - preset_id: backend_engineer
    id: backend_secondary
    name: backendagent
---

Invalid.
"#;

        for (markdown, expected) in [
            (mixed_syntax, "must use either members or member_ids"),
            (duplicate_id, "duplicate member instance id: backend"),
            (
                duplicate_name,
                "duplicate member instance name: backendagent",
            ),
        ] {
            let error = PresetLoader::parse_team_preset_markdown("invalid_team.md", markdown)
                .expect_err("invalid member instances should fail");
            assert!(
                error.to_string().contains(expected),
                "expected {expected:?}, got {error:#}"
            );
        }
    }

    #[test]
    fn parse_team_preset_markdown_supports_all_team_template_metadata() {
        let markdown = r#"---
id: sample_team
name: Sample Team
description: Team description
member_ids:
  - backend_engineer
  - frontend_engineer
lead_member_id: backend_engineer
workflow_steps:
  - title: Plan
    description: Set the implementation direction.
  - title: Ship
    description: Deliver and verify the change.
enabled: false
---

Follow the team protocol.
"#;

        let parsed = PresetLoader::parse_team_preset_markdown("sample_team.md", markdown).unwrap();

        assert_eq!(parsed.lead_member_id.as_deref(), Some("backend_engineer"));
        assert_eq!(parsed.workflow_steps.len(), 2);
        assert_eq!(parsed.workflow_steps[0].title, "Plan");
        assert!(!parsed.enabled);
        assert_eq!(parsed.team_protocol, "Follow the team protocol.");
    }

    #[test]
    fn embedded_team_preset_markdown_files_load_from_protocol_directory() {
        let files = PresetLoader::embedded_team_preset_markdown_files()
            .expect("embedded team markdown files should load");
        let file_names = files
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>();

        assert!(file_names.contains(&"en/fullstack_delivery_team.md"));
        assert!(file_names.contains(&"zh/rapid_bugfix_team.md"));
        assert!(file_names.contains(&"en/blank_team.md"));
        assert!(file_names.contains(&"zh/blank_team.md"));
        assert!(!file_names.contains(&TEAM_COLLABORATION_PROTOCOL_FILE));
    }

    #[test]
    fn load_builtin_presets_parses_eleven_english_team_templates_with_tiers() {
        let presets = PresetLoader::load_builtin_presets_for_locale(Some("en-US"));

        assert_eq!(presets.teams.len(), 11);
        let advanced = presets
            .teams
            .iter()
            .filter(|team| team.tier == crate::services::config::ChatTeamTemplateTier::Advanced)
            .map(|team| team.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            advanced,
            vec!["advanced-growth-ops", "advanced-release-command"]
        );
        assert!(presets.teams.iter().all(|team| {
            !team.name.trim().is_empty()
                && !team.description.trim().is_empty()
                && !team.members.is_empty()
                && !team.team_protocol.trim().is_empty()
        }));
        assert!(
            presets
                .teams
                .iter()
                .filter(|team| team.id != "blank_team")
                .all(|team| !team.workflow_steps.is_empty())
        );

        let literature_research_team = presets
            .teams
            .iter()
            .find(|team| team.id == "literature_research_team")
            .expect("literature research team template exists");
        let member_ids = literature_research_team
            .members
            .iter()
            .map(|member| member.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            member_ids,
            vec![
                "literature-review-researcher",
                "literature-research-assistant",
                "literature-research-assistant-2",
                "literature-research-assistant-3",
                "literature-research-assistant-4",
                "literature-research-assistant-5",
                "literature-research-reviewer",
            ]
        );
        assert_eq!(
            literature_research_team.lead_member_id.as_deref(),
            Some("literature-review-researcher")
        );
        assert_eq!(
            literature_research_team
                .members
                .iter()
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "LiteratureReviewResearcher",
                "LiteratureResearchAssistant",
                "LiteratureResearchAssistant2",
                "LiteratureResearchAssistant3",
                "LiteratureResearchAssistant4",
                "LiteratureResearchAssistant5",
                "LiteratureResearchReviewer",
            ]
        );
        let expected_playwright = serde_json::json!({
            "command": "npx",
            "args": ["@playwright/mcp@latest"]
        });
        for member in &literature_research_team.members {
            let mcp = member
                .execution_config
                .as_ref()
                .and_then(|config| config.mcp.as_ref())
                .expect("literature research member should include MCP configuration");
            mcp.validate(&member.id)
                .expect("literature research member MCP configuration should be valid");
            assert_eq!(mcp.mcp_servers.len(), 1);
            assert_eq!(
                mcp.mcp_servers.get("playwright"),
                Some(&expected_playwright)
            );
            assert_eq!(
                member.tools_enabled["mcpServers"]["playwright"],
                serde_json::json!(true)
            );
        }
    }

    #[test]
    fn load_builtin_presets_applies_all_supported_locales_without_changing_structure() {
        let english = PresetLoader::load_builtin_presets_for_locale(Some("en"));
        let english_fullstack = english
            .teams
            .iter()
            .find(|team| team.id == "fullstack_delivery_team")
            .expect("English fullstack template exists");
        let english_member_ids = english_fullstack
            .members
            .iter()
            .map(|member| member.id.as_str())
            .collect::<Vec<_>>();

        for locale in ["zh-CN", "ja-JP", "ko-KR", "fr-FR", "es-ES"] {
            let localized = PresetLoader::load_builtin_presets_for_locale(Some(locale));
            let fullstack = localized
                .teams
                .iter()
                .find(|team| team.id == "fullstack_delivery_team")
                .expect("localized fullstack template exists");
            let member_ids = fullstack
                .members
                .iter()
                .map(|member| member.id.as_str())
                .collect::<Vec<_>>();

            assert_eq!(localized.teams.len(), english.teams.len(), "{locale}");
            assert_ne!(fullstack.name, english_fullstack.name, "{locale}");
            assert_eq!(member_ids, english_member_ids, "{locale}");
            assert_eq!(fullstack.lead_member_id, english_fullstack.lead_member_id);
            assert_eq!(fullstack.tier, english_fullstack.tier);
            assert!(!fullstack.team_protocol.contains("`@`"), "{locale}");

            let blank_team = localized
                .teams
                .iter()
                .find(|team| team.id == "blank_team")
                .expect("localized blank team template exists");
            assert_ne!(blank_team.name, "Blank Team", "{locale}");
            assert_eq!(blank_team.members.len(), 1, "{locale}");
        }
    }

    #[test]
    fn normalize_team_template_locale_maps_regions_and_unsupported_values() {
        assert_eq!(normalize_team_template_locale(None), "en");
        assert_eq!(normalize_team_template_locale(Some("zh-Hans")), "zh");
        assert_eq!(normalize_team_template_locale(Some("ja_JP")), "ja");
        assert_eq!(normalize_team_template_locale(Some("ko-KR")), "ko");
        assert_eq!(normalize_team_template_locale(Some("fr-CA")), "fr");
        assert_eq!(normalize_team_template_locale(Some("es-MX")), "es");
        assert_eq!(normalize_team_template_locale(Some("de-DE")), "en");
    }

    #[test]
    fn localized_team_template_falls_back_as_a_whole_when_file_is_missing() {
        let (files, english_raw) = english_only_fullstack_files();

        clear_fallback_warnings();
        let localized = PresetLoader::localized_team_preset_markdown(
            &files,
            "en/fullstack_delivery_team.md",
            &english_raw,
            "zh",
        )
        .expect("fallback should succeed");
        let english =
            PresetLoader::parse_team_preset_markdown("en/fullstack_delivery_team.md", &english_raw)
                .expect("English should parse");

        assert_eq!(localized, english);
        assert_fallback_warning("fullstack_delivery_team", "zh", "missing");
    }

    #[test]
    fn localized_team_template_falls_back_as_a_whole_when_file_does_not_parse() {
        let (mut files, english_raw) = english_only_fullstack_files();
        files.insert(
            "zh/fullstack_delivery_team.md".to_string(),
            "not frontmatter".to_string(),
        );

        clear_fallback_warnings();
        let localized = PresetLoader::localized_team_preset_markdown(
            &files,
            "en/fullstack_delivery_team.md",
            &english_raw,
            "zh",
        )
        .expect("fallback should succeed");
        let english =
            PresetLoader::parse_team_preset_markdown("en/fullstack_delivery_team.md", &english_raw)
                .expect("English should parse");

        assert_eq!(localized, english);
        assert_fallback_warning("fullstack_delivery_team", "zh", "frontmatter");
    }

    #[test]
    fn localized_team_template_falls_back_as_a_whole_when_id_mismatches() {
        let (mut files, english_raw) = english_only_fullstack_files();
        let damaged = english_raw.replace(
            "id: fullstack_delivery_team",
            "id: mismatched_fullstack_delivery_team",
        );
        files.insert("zh/fullstack_delivery_team.md".to_string(), damaged);

        clear_fallback_warnings();
        let localized = PresetLoader::localized_team_preset_markdown(
            &files,
            "en/fullstack_delivery_team.md",
            &english_raw,
            "zh",
        )
        .expect("fallback should succeed");
        let english =
            PresetLoader::parse_team_preset_markdown("en/fullstack_delivery_team.md", &english_raw)
                .expect("English should parse");

        assert_eq!(localized, english);
        assert_fallback_warning("fullstack_delivery_team", "zh", "id mismatch");
    }

    #[test]
    fn localized_team_template_falls_back_as_a_whole_when_structure_is_damaged() {
        let (mut files, english_raw) = english_only_fullstack_files();
        let damaged = english_raw.replace("- title: Requirements and planning", "- title: \"\"");
        files.insert("zh/fullstack_delivery_team.md".to_string(), damaged);

        clear_fallback_warnings();
        let localized = PresetLoader::localized_team_preset_markdown(
            &files,
            "en/fullstack_delivery_team.md",
            &english_raw,
            "zh",
        )
        .expect("fallback should succeed");
        let english =
            PresetLoader::parse_team_preset_markdown("en/fullstack_delivery_team.md", &english_raw)
                .expect("English should parse");

        assert_eq!(localized, english);
        assert_fallback_warning("fullstack_delivery_team", "zh", "empty fields");
    }

    #[test]
    fn catalog_entries_do_not_parse_or_fail_on_damaged_localized_files() {
        let (_files, english_raw) = english_only_fullstack_files();
        let files = vec![
            (
                "en/fullstack_delivery_team.md".to_string(),
                english_raw.clone(),
            ),
            (
                "zh/fullstack_delivery_team.md".to_string(),
                "not frontmatter".to_string(),
            ),
        ];

        let entries = PresetLoader::load_builtin_team_template_catalog_entries_from_files(files)
            .expect("damaged localized files should not abort catalog source indexing");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].template_id, "fullstack_delivery_team");
        assert!(entries[0].source_content.contains(&english_raw));
        assert!(
            entries[0]
                .source_content
                .contains("zh/fullstack_delivery_team.md\nnot frontmatter")
        );
    }

    fn english_only_fullstack_files() -> (std::collections::BTreeMap<String, String>, String) {
        let files = PresetLoader::embedded_team_preset_markdown_files()
            .expect("embedded team markdown files should load")
            .into_iter()
            .filter(|(path, _)| path.starts_with("en/"))
            .collect::<std::collections::BTreeMap<_, _>>();
        let english_raw = files
            .get("en/fullstack_delivery_team.md")
            .expect("English fullstack file exists")
            .clone();
        (files, english_raw)
    }

    fn clear_fallback_warnings() {
        super::TEAM_TEMPLATE_LOCALE_FALLBACK_WARNINGS
            .with(|warnings| warnings.borrow_mut().clear());
    }

    fn assert_fallback_warning(template_id: &str, locale: &str, reason_contains: &str) {
        let warnings = super::team_template_locale_fallback_warnings();
        assert!(
            warnings
                .iter()
                .any(|(logged_template_id, logged_locale, reason)| {
                    logged_template_id == template_id
                        && logged_locale == locale
                        && reason.contains(reason_contains)
                }),
            "expected warning for {template_id}/{locale} containing {reason_contains:?}, got {warnings:?}"
        );
    }

    #[test]
    fn preset_ids_are_unique() {
        let presets = PresetLoader::load_builtin_presets();
        let mut seen = std::collections::HashSet::new();

        for preset in &presets.members {
            assert!(
                seen.insert(&preset.id),
                "Duplicate preset ID found: {}",
                preset.id
            );
        }

        assert_eq!(seen.len(), 169);
    }

    #[test]
    fn all_presets_have_required_fields() {
        let presets = PresetLoader::load_builtin_presets();

        for preset in &presets.members {
            assert!(!preset.id.is_empty(), "Preset has empty ID");
            assert!(
                !preset.name.is_empty(),
                "Preset {} has empty name",
                preset.id
            );
            assert!(
                !preset.description.is_empty(),
                "Preset {} has empty description",
                preset.id
            );
            assert!(
                !preset.system_prompt.is_empty(),
                "Preset {} has empty system_prompt",
                preset.id
            );
        }
    }

    #[test]
    fn load_builtin_presets_strips_spaces_from_member_names() {
        let presets = PresetLoader::load_builtin_presets();

        assert!(presets.members.iter().all(|preset| {
            preset
                .name
                .chars()
                .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-')
        }));
        let auditor = presets
            .members
            .iter()
            .find(|preset| preset.id == "accessibility-auditor")
            .expect("accessibility auditor preset should exist");
        assert_eq!(auditor.name, "AccessibilityAuditor");
        let trust_architect = presets
            .members
            .iter()
            .find(|preset| preset.id == "agentic-identity-trust-architect")
            .expect("trust architect preset should exist");
        assert_eq!(trust_architect.name, "AgenticIdentityTrustArchitect");
    }

    #[test]
    fn original_22_presets_still_exist() {
        let presets = PresetLoader::load_builtin_presets();
        let original_ids = vec![
            "coordinator_pmo",
            "product_manager",
            "system_architect",
            "prompt_engineer",
            "frontend_engineer",
            "backend_engineer",
            "fullstack_engineer",
            "qa_tester",
            "ux_ui_designer",
            "safety_policy_officer",
            "solution_manager",
            "code_reviewer",
            "devops_engineer",
            "product_analyst",
            "data_analyst",
            "technical_writer",
            "content_researcher",
            "content_editor",
            "frontier_researcher",
            "marketing_specialist",
            "video_editor",
            "market_analyst",
        ];

        for id in original_ids {
            assert!(
                presets.members.iter().any(|p| p.id == id),
                "Original preset {} is missing",
                id
            );
        }
    }

    #[test]
    fn new_presets_have_metadata() {
        let presets = PresetLoader::load_builtin_presets();

        // Count presets with metadata
        let mut with_metadata = 0;

        for preset in &presets.members {
            if let Some(metadata) = preset.tools_enabled.get("metadata") {
                // Verify metadata structure
                if let Some(category) = metadata.get("category") {
                    assert!(
                        category.is_string(),
                        "Preset {} has non-string category",
                        preset.id
                    );
                    with_metadata += 1;
                }
            }
        }

        // At least the 144 new presets should have metadata
        assert!(
            with_metadata >= 144,
            "Expected at least 144 presets with metadata, found {}",
            with_metadata
        );
    }
}
