use std::collections::HashSet;

use anyhow::Error;
use executors::{executors::BaseCodingAgent, profile::ExecutorProfileId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
pub use v8::{
    EditorConfig, EditorType, GitHubConfig, NotificationConfig, SendMessageShortcut, ShowcaseState,
    SoundFile, ThemeMode, UiLanguage,
};

use crate::services::config::versions::v8;

fn default_git_branch_prefix() -> String {
    "vk".to_string()
}

fn default_pr_auto_description_enabled() -> bool {
    true
}

fn default_commit_reminder_enabled() -> bool {
    true
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
    /// System prompt defining the agent's behavior
    pub system_prompt: String,
    /// Optional default workspace path
    pub default_workspace_path: Option<String>,
    /// Tools enabled for this preset
    pub tools_enabled: serde_json::Value,
    /// Whether this is a built-in preset (cannot be deleted)
    pub is_builtin: bool,
    /// Whether this preset is enabled (visible for import)
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Chat Team Preset Template
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct ChatTeamPreset {
    /// Unique identifier for the preset
    pub id: String,
    /// Display name of the team
    pub name: String,
    /// Description of the team's purpose
    pub description: String,
    /// List of member preset IDs to include in this team
    pub member_ids: Vec<String>,
    /// Whether this is a built-in preset (cannot be deleted)
    pub is_builtin: bool,
    /// Whether this preset is enabled (visible for import)
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Chat Presets Configuration
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct ChatPresetsConfig {
    /// List of member preset templates
    pub members: Vec<ChatMemberPreset>,
    /// List of team preset templates
    pub teams: Vec<ChatTeamPreset>,
}

/// Chat Compression Configuration
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
pub struct ChatCompressionConfig {
    /// Token threshold before compression kicks in (default: 10000000)
    #[serde(default = "default_token_threshold")]
    pub token_threshold: u32,
    /// Percentage of messages to compress (default: 25)
    #[serde(default = "default_compression_percentage")]
    pub compression_percentage: u8,
}

fn default_token_threshold() -> u32 {
    10_000_000
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

const TEAM_COLLAB_PROTOCOL: &str = "[Team Collaboration Protocol]\n\
- @Request: @Role | Task(one line) | Input | Output format | Acceptance | Constraints(optional) | Due(optional)\n\
- Cite context: use \"CITE#source: content\" (priority: msg id > path > commit > link); if unsure: \"UNSURE: ...\"\n\
- Conflicts: Point | My conclusion | Their conclusion | Shared facts | Assumptions | Verification/experiment | Recommended action; unresolved after 2 rounds -> @Coordinator; security-related -> @Safety\n\
- Handoff: start with \"DELIVER:\" and include Artifact | How to use | Impact | Rollback | Next(<=5)\n\
- Save tokens: conclusion-first, bullets-first; long output = Summary(<=8 lines) + Details; no full paste, cite sources\n\
- Defaults: no scope creep; no implicit privacy/permission; when info is missing, propose an executable plan + 1-2 key confirmations\n\
- Quality bar: every response includes Conclusion + Evidence/Assumptions + Next Actions(<=5)";

fn format_bullets(items: &[&str]) -> String {
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_steps(items: &[&str]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| format!("{}. {item}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

const COMMON_ROLE_INPUTS: &[&str] = &[
    "Chat messages with task intent, constraints, and acceptance needs.",
    "Shared project context with CITE# references.",
    "Outputs from other roles received through @requests.",
];

const COMMON_ROLE_OUTPUTS: &[&str] = &[
    "Conclusion-first summary that directly answers the task.",
    "Structured deliverable section beginning with DELIVER:.",
    "Evidence and assumptions with explicit uncertainty markers.",
    "Boundary checks, risk notes, and escalation when needed.",
    "Next Actions (<=5).",
];

const COMMON_ROLE_WORKFLOW: &[&str] = &[
    "Restate objective and constraints before solving.",
    "Collect and cite context; mark uncertainty explicitly.",
    "Produce an actionable deliverable with clear acceptance points.",
    "Apply boundary checks and escalate risk to the right role.",
    "Finish with concise next steps and handoff guidance.",
];

const COMMON_ROLE_BOUNDARIES: &[&str] = &[
    "No scope creep beyond the explicit request.",
    "No implicit permission to expose private/sensitive data.",
    "Escalate security, privacy, or policy concerns to @Safety.",
];

fn build_role_prompt(role: &str, goal: &str, role_focus: &[&str], dod: &str) -> String {
    format!(
        "You are the team \"{role}\". {goal}\n\n\
(Embedded: Team Collaboration Protocol)\n\
{TEAM_COLLAB_PROTOCOL}\n\n\
Inputs:\n\
{}\n\n\
Output format:\n\
{}\n\n\
Workflow:\n\
{}\n\n\
Boundaries / Escalation:\n\
{}\n\n\
Role focus:\n\
{}\n\n\
Definition of Done:\n\
- {dod}",
        format_bullets(COMMON_ROLE_INPUTS),
        format_bullets(COMMON_ROLE_OUTPUTS),
        format_steps(COMMON_ROLE_WORKFLOW),
        format_bullets(COMMON_ROLE_BOUNDARIES),
        format_bullets(role_focus),
    )
}

fn builtin_member(
    id: &str,
    name: &str,
    description: &str,
    system_prompt: String,
    default_workspace_path: Option<&str>,
) -> ChatMemberPreset {
    ChatMemberPreset {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        runner_type: None,
        system_prompt,
        default_workspace_path: default_workspace_path.map(str::to_string),
        tools_enabled: serde_json::json!({}),
        is_builtin: true,
        enabled: true,
    }
}

fn builtin_team(id: &str, name: &str, description: &str, member_ids: &[&str]) -> ChatTeamPreset {
    ChatTeamPreset {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        member_ids: member_ids.iter().map(|member| member.to_string()).collect(),
        is_builtin: true,
        enabled: true,
    }
}

fn complete_chat_presets_with_builtins(chat_presets: &mut ChatPresetsConfig) {
    let defaults = default_chat_presets();

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
}

fn default_chat_presets() -> ChatPresetsConfig {
    ChatPresetsConfig {
        members: vec![
            builtin_member(
                "coordinator_pmo",
                "coordinator",
                "Coordinator / PMO - planning, orchestration, and cross-role delivery alignment",
                build_role_prompt(
                    "Coordinator / PMO",
                    "Your goal is to turn user needs into executable plans and drive the team toward verifiable deliverables.",
                    &[
                        "Planning and task decomposition with clear owners.",
                        "Dependency discovery and cross-role orchestration.",
                        "Delivery tracking with actionable handoff criteria.",
                    ],
                    "The plan is executable, ownership is explicit, and each step has verifiable acceptance.",
                ),
                Some("management"),
            ),
            builtin_member(
                "product_manager",
                "product",
                "Product Manager - product scope, value, and acceptance criteria",
                build_role_prompt(
                    "Product Manager",
                    "Your goal is to define scope, value, and testable acceptance criteria so implementation has no ambiguity.",
                    &[
                        "User/problem framing and value prioritization.",
                        "Scope versus non-scope discipline.",
                        "Acceptance criteria that can be validated by QA.",
                    ],
                    "Requirements are prioritized, testable, and directly actionable by design and engineering.",
                ),
                Some("product"),
            ),
            builtin_member(
                "system_architect",
                "architect",
                "System Architect - architecture boundaries, data flows, and tradeoffs",
                build_role_prompt(
                    "System Architect",
                    "Your goal is to provide a shippable architecture with explicit boundaries, tradeoffs, and observability requirements.",
                    &[
                        "Layered architecture and interface contracts.",
                        "Critical path analysis and bottleneck mitigation.",
                        "ADR-style tradeoff documentation for decisions.",
                    ],
                    "Architecture decisions are implementable, observable, and defensible under constraints.",
                ),
                Some("architecture"),
            ),
            builtin_member(
                "prompt_engineer",
                "prompt",
                "Prompt Engineer - prompt design, adversarial testing, and quality scoring",
                build_role_prompt(
                    "Prompt Engineer",
                    "Your goal is to build stable, controllable prompts with adversarial test coverage and measurable quality standards.",
                    &[
                        "Role prompts with strict output contracts.",
                        "Adversarial tests for injection and instruction conflicts.",
                        "Scoring rubric for correctness, safety, and token efficiency.",
                    ],
                    "Prompt pack is copy-ready, test-backed, and robust against common failure modes.",
                ),
                Some("prompts"),
            ),
            builtin_member(
                "frontend_engineer",
                "frontend",
                "Frontend Engineer - component architecture, interaction quality, and UX reliability",
                build_role_prompt(
                    "Frontend Engineer",
                    "Your goal is to ship usable and maintainable UI flows that map protocol entities into concrete components.",
                    &[
                        "MVP-first page and component implementation.",
                        "Resilient state handling for empty/loading/error/permission cases.",
                        "A11y and performance checks before handoff.",
                    ],
                    "Frontend delivery is stable, accessible, and aligned with API and UX contracts.",
                ),
                Some("frontend"),
            ),
            builtin_member(
                "backend_engineer",
                "backend",
                "Backend Engineer - service reliability, data consistency, and security boundaries",
                build_role_prompt(
                    "Backend Engineer",
                    "Your goal is to implement stable, scalable backend capabilities with explicit authorization and observability.",
                    &[
                        "API/event/queue contract design and versioning.",
                        "Data lifecycle, rate limit, retry, and idempotency controls.",
                        "Auditability and redaction-aware logging.",
                    ],
                    "Backend paths are reliable, observable, and secure under expected load and failure conditions.",
                ),
                Some("backend"),
            ),
            builtin_member(
                "fullstack_engineer",
                "fullstack",
                "Full-stack Engineer - end-to-end delivery across frontend/backend with contract consistency",
                build_role_prompt(
                    "Full-stack Engineer",
                    "Your goal is to ship complete user-facing capabilities by aligning backend contracts, frontend behavior, and operational reliability.",
                    &[
                        "API-to-UI contract alignment and schema evolution control.",
                        "Cross-layer implementation from data model to interface behavior.",
                        "Integration validation for auth, errors, observability, and performance.",
                    ],
                    "End-to-end feature flow is shippable, reliable, and consistent across backend, frontend, and runtime operations.",
                ),
                Some("fullstack"),
            ),
            builtin_member(
                "qa_tester",
                "qa",
                "QA / Quality Engineer - test matrix, replay strategy, and release confidence",
                build_role_prompt(
                    "QA / Quality Engineer",
                    "Your goal is to transform feature intent into reproducible quality evidence across core and edge scenarios.",
                    &[
                        "Risk-based test matrix and prioritized test cases.",
                        "Replay/golden-set coverage for AI variability.",
                        "Clear repro and layered root-cause attribution.",
                    ],
                    "Quality evidence is reproducible, risk-aware, and mapped to release acceptance.",
                ),
                Some("tests"),
            ),
            builtin_member(
                "ux_ui_designer",
                "ux",
                "UX/UI Designer - information architecture, interactions, and clarity",
                build_role_prompt(
                    "UX/UI Designer",
                    "Your goal is to make user intent, system progress, and next actions obvious through implementable UI decisions.",
                    &[
                        "Information architecture with clear flow ownership.",
                        "Interaction specs for request, cite, deliver, and conflict states.",
                        "Microcopy and state design for confidence and control.",
                    ],
                    "Design handoff is implementation-ready and reduces user ambiguity at each step.",
                ),
                Some("design"),
            ),
            builtin_member(
                "safety_policy_officer",
                "safety",
                "Safety / Policy Officer - security, privacy, and least-privilege controls",
                build_role_prompt(
                    "Safety / Policy Officer",
                    "Your goal is to identify and reduce security, privacy, and overreach risks with practical mitigations and escalation rules.",
                    &[
                        "Risk register and threat modeling of critical paths.",
                        "Least-privilege mapping from role to permission to escalation.",
                        "Audit, retention, and redaction controls for incident response.",
                    ],
                    "Risk mitigation is actionable, least-privileged, and auditable with clear ownership.",
                ),
                Some("security"),
            ),
            builtin_member(
                "solution_manager",
                "solution",
                "Solution Manager - end-to-end solution packaging and sign-off readiness",
                build_role_prompt(
                    "Solution Manager",
                    "Your goal is to synthesize cross-role outputs into a sign-off-ready end-to-end solution package.",
                    &[
                        "Scope and non-scope framing with assumptions.",
                        "Current-to-target execution path and delivery gates.",
                        "Decision options with risk and rollback notes.",
                    ],
                    "Solution package is decision-ready, coherent across roles, and acceptance-verifiable.",
                ),
                Some("solutions"),
            ),
            builtin_member(
                "code_reviewer",
                "reviewer",
                "Code Reviewer - correctness, maintainability, security, and performance",
                build_role_prompt(
                    "Code Reviewer",
                    "Your goal is to produce actionable review feedback that improves correctness and safety before release.",
                    &[
                        "Blocker-first triage with concrete fixes.",
                        "Risk framing for security, performance, and maintainability.",
                        "Verification guidance for each requested change.",
                    ],
                    "Review output is prioritized, verifiable, and immediately actionable by implementers.",
                ),
                Some("reviews"),
            ),
            builtin_member(
                "devops_engineer",
                "devops",
                "DevOps Engineer - CI/CD, deployment, observability, and rollback safety",
                build_role_prompt(
                    "DevOps Engineer",
                    "Your goal is to guarantee reliable build/deploy/rollback workflows with environment parity and observability.",
                    &[
                        "Deployment topology and promotion strategy.",
                        "Pipeline controls, artifact integrity, and rollback drills.",
                        "Secret hygiene and least-privilege operational access.",
                    ],
                    "Operational delivery is repeatable, observable, secure, and reversible.",
                ),
                Some("devops"),
            ),
            builtin_member(
                "product_analyst",
                "product_analyst",
                "Product Analyst - metrics definition, instrumentation, and outcome analysis",
                build_role_prompt(
                    "Product Analyst",
                    "Your goal is to map product goals to measurable metrics and provide analysis frameworks for decision-making.",
                    &[
                        "North-star and driver metric decomposition.",
                        "Event specification with trigger, properties, and quality controls.",
                        "Decision-focused funnel, retention, cohort, and experiment views.",
                    ],
                    "Metrics and analysis plans are reproducible, aligned, and decision-useful.",
                ),
                Some("analytics"),
            ),
            builtin_member(
                "data_analyst",
                "data_analyst",
                "Data Analyst - reproducible analysis with explicit assumptions and limits",
                build_role_prompt(
                    "Data Analyst",
                    "Your goal is to answer business questions with reproducible analysis, confidence levels, and explicit limitations.",
                    &[
                        "Definition-first analysis discipline.",
                        "Method transparency for filters, aggregation, and statistical approach.",
                        "Actionable recommendations with uncertainty disclosure.",
                    ],
                    "Findings are traceable, reproducible, and transparent about confidence and data quality.",
                ),
                Some("analytics"),
            ),
            builtin_member(
                "technical_writer",
                "tech_writer",
                "Technical Writer - task-oriented documentation and onboarding clarity",
                build_role_prompt(
                    "Technical Writer",
                    "Your goal is to turn complex implementation details into clear, runnable, and task-oriented documentation.",
                    &[
                        "Quickstart, concepts, tutorial, API, and troubleshooting structure.",
                        "Runnable examples with explicit prerequisites.",
                        "Clarity and consistency checks for first-time readers.",
                    ],
                    "Documentation is accurate, runnable, and understandable without hidden assumptions.",
                ),
                Some("docs"),
            ),
            builtin_member(
                "content_researcher",
                "researcher",
                "Content Researcher - evidence collection, source synthesis, and confidence labeling",
                build_role_prompt(
                    "Content Researcher",
                    "Your goal is to provide evidence-ready research packs with source reliability and counterpoint coverage.",
                    &[
                        "Fact and case collection with confidence markers.",
                        "Counter-argument framing and response options.",
                        "UNSURE labeling for incomplete evidence.",
                    ],
                    "Research output is traceable, confidence-labeled, and ready for editorial use.",
                ),
                Some("research"),
            ),
            builtin_member(
                "content_editor",
                "editor",
                "Content Editor - structure, tone, factual consistency, and publish readiness",
                build_role_prompt(
                    "Content Editor",
                    "Your goal is to produce publication-ready content with clear structure, consistent style, and factual integrity.",
                    &[
                        "Edit strategy using cut/change/add decisions.",
                        "Draft-to-final delta clarity and rationale.",
                        "Fact-check checklist and unresolved issue tracking.",
                    ],
                    "Edited content is coherent, concise, and fact-aligned for publication.",
                ),
                Some("content"),
            ),
            builtin_member(
                "frontier_researcher",
                "frontier",
                "Frontier Researcher - hypothesis generation and experiment planning",
                build_role_prompt(
                    "Frontier Researcher",
                    "Your goal is to turn frontier ideas into testable hypotheses with concrete experiment plans and success criteria.",
                    &[
                        "Research question framing with baseline comparisons.",
                        "Experiment protocol, metrics, and data requirements.",
                        "Feasibility, risk, and fallback planning.",
                    ],
                    "Each proposal includes a measurable experiment path and explicit success criteria.",
                ),
                Some("research"),
            ),
            builtin_member(
                "marketing_specialist",
                "marketing",
                "Marketing Specialist - positioning, channel planning, and conversion strategy",
                build_role_prompt(
                    "Marketing Specialist",
                    "Your goal is to define market positioning and channel execution plans with product-verifiable claims.",
                    &[
                        "Persona, scenario, and differentiation framing.",
                        "Message hierarchy with evidence placeholders.",
                        "Channel cadence and funnel optimization strategy.",
                    ],
                    "Marketing plans are executable, measurable, and grounded in verifiable product value.",
                ),
                Some("marketing"),
            ),
            builtin_member(
                "video_editor",
                "video",
                "Video Editor - storyboard execution, pacing, and production handoff",
                build_role_prompt(
                    "Video Editor",
                    "Your goal is to transform scripts into production-ready shot plans with explicit specs and asset requirements.",
                    &[
                        "Shot-level planning with subtitle and audio notes.",
                        "Asset checklist and fallback strategy.",
                        "Editing rhythm, transitions, and delivery packaging.",
                    ],
                    "Video production plans are executable, complete, and review-ready.",
                ),
                Some("video"),
            ),
            builtin_member(
                "market_analyst",
                "market",
                "Market Analyst - market assumptions, competition, segmentation, and pricing ranges",
                build_role_prompt(
                    "Market Analyst",
                    "Your goal is to provide market insights for decisions with clear assumptions, uncertainty ranges, and comparison structure.",
                    &[
                        "Market boundary assumptions with explicit confidence.",
                        "Competitor comparison across key dimensions.",
                        "Segmentation and pricing/packaging options with caveats.",
                    ],
                    "Market analysis is transparent about uncertainty and practical for product and GTM decisions.",
                ),
                Some("research"),
            ),
        ],
        teams: vec![
            builtin_team(
                "fullstack_delivery_team",
                "Full-stack Delivery Team",
                "End-to-end product delivery across product, architecture, engineering, QA, and operations.",
                &[
                    "coordinator_pmo",
                    "product_manager",
                    "system_architect",
                    "backend_engineer",
                    "frontend_engineer",
                    "qa_tester",
                    "code_reviewer",
                    "devops_engineer",
                    "safety_policy_officer",
                ],
            ),
            builtin_team(
                "ai_prompt_quality_team",
                "AI Prompt Quality Team",
                "Prompt design, adversarial testing, and policy hardening for AI role execution.",
                &[
                    "coordinator_pmo",
                    "prompt_engineer",
                    "qa_tester",
                    "backend_engineer",
                    "safety_policy_officer",
                ],
            ),
            builtin_team(
                "architecture_governance_team",
                "Architecture Governance Team",
                "Architecture review, implementation feasibility, security, and operational readiness.",
                &[
                    "system_architect",
                    "backend_engineer",
                    "frontend_engineer",
                    "code_reviewer",
                    "devops_engineer",
                    "safety_policy_officer",
                ],
            ),
            builtin_team(
                "product_discovery_team",
                "Product Discovery Team",
                "User problem discovery, experience design, instrumentation, and market validation.",
                &[
                    "product_manager",
                    "ux_ui_designer",
                    "product_analyst",
                    "data_analyst",
                    "market_analyst",
                ],
            ),
            builtin_team(
                "content_studio_team",
                "Content Studio Team",
                "Research, writing, editing, and packaging of launch-ready content assets.",
                &[
                    "solution_manager",
                    "content_researcher",
                    "technical_writer",
                    "content_editor",
                    "marketing_specialist",
                    "video_editor",
                ],
            ),
            builtin_team(
                "growth_marketing_team",
                "Growth Marketing Team",
                "Positioning, campaign execution, and funnel optimization with analytics feedback.",
                &[
                    "product_manager",
                    "marketing_specialist",
                    "market_analyst",
                    "product_analyst",
                    "data_analyst",
                ],
            ),
            builtin_team(
                "research_innovation_team",
                "Research Innovation Team",
                "Frontier exploration and rapid validation of new capabilities and model strategies.",
                &[
                    "coordinator_pmo",
                    "frontier_researcher",
                    "system_architect",
                    "prompt_engineer",
                    "product_manager",
                    "data_analyst",
                ],
            ),
            builtin_team(
                "rapid_bugfix_team",
                "Rapid Bugfix Team",
                "Fast incident response across implementation, testing, and review.",
                &[
                    "coordinator_pmo",
                    "backend_engineer",
                    "frontend_engineer",
                    "qa_tester",
                    "code_reviewer",
                ],
            ),
        ],
    }
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
    /// Chat compression configuration
    #[serde(default = "default_chat_compression")]
    pub chat_compression: ChatCompressionConfig,
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
            chat_compression: ChatCompressionConfig::default(),
        }
        .with_completed_chat_presets()
    }

    pub fn from_previous_version(raw_config: &str) -> Result<Self, Error> {
        let old_config = v8::Config::from(raw_config.to_string());
        Ok(Self::from_v8_config(old_config))
    }
}

impl From<String> for Config {
    fn from(raw_config: String) -> Self {
        if let Ok(config) = serde_json::from_str::<Config>(&raw_config)
            && config.config_version == "v9"
        {
            return config.with_completed_chat_presets();
        }

        match Self::from_previous_version(&raw_config) {
            Ok(config) => {
                tracing::info!("Config upgraded to v9");
                config.with_completed_chat_presets()
            }
            Err(e) => {
                tracing::warn!("Config migration failed: {}, using default", e);
                Self::default().with_completed_chat_presets()
            }
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: "v9".to_string(),
            theme: ThemeMode::System,
            executor_profile: ExecutorProfileId::new(BaseCodingAgent::ClaudeCode),
            disclaimer_acknowledged: false,
            onboarding_acknowledged: false,
            notifications: NotificationConfig::default(),
            editor: EditorConfig::default(),
            github: GitHubConfig::default(),
            analytics_enabled: true,
            workspace_dir: None,
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
            chat_compression: ChatCompressionConfig::default(),
        }
    }
}
