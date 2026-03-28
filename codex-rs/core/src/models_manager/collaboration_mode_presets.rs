use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::PlanModePhase;
use codex_protocol::config_types::TUI_VISIBLE_COLLABORATION_MODES;
use codex_protocol::openai_models::ReasoningEffort;

const COLLABORATION_MODE_PLAN: &str = include_str!("../../templates/collaboration_mode/plan.md");
const COLLABORATION_MODE_ULTRA_WORK: &str =
    include_str!("../../templates/collaboration_mode/ultra_work.md");
const COLLABORATION_MODE_DEFAULT: &str =
    include_str!("../../templates/collaboration_mode/default.md");
const COLLABORATION_MODE_ULTRA_WORK_EXECUTE: &str =
    include_str!("../../templates/collaboration_mode/ultra_work_execute.md");
const KNOWN_MODE_NAMES_PLACEHOLDER: &str = "{{KNOWN_MODE_NAMES}}";
const PLAN_PREPARATORY_MUTATIONS_GUIDANCE_PLACEHOLDER: &str =
    "{{PLAN_PREPARATORY_MUTATIONS_GUIDANCE}}";
const REQUEST_USER_INPUT_AVAILABILITY_PLACEHOLDER: &str = "{{REQUEST_USER_INPUT_AVAILABILITY}}";
const ASKING_QUESTIONS_GUIDANCE_PLACEHOLDER: &str = "{{ASKING_QUESTIONS_GUIDANCE}}";

/// Stores feature flags that control collaboration-mode behavior.
///
/// Keep mode-related flags here so new collaboration-mode capabilities can be
/// added without large cross-cutting diffs to constructor and call-site
/// signatures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CollaborationModesConfig {
    /// Enables `request_user_input` availability in Default mode.
    pub default_mode_request_user_input: bool,
    /// Enables the Ultra Work workflow and its execute-phase runtime.
    pub plan_workflow_enabled: bool,
}

pub fn builtin_collaboration_mode_presets(
    collaboration_modes_config: CollaborationModesConfig,
) -> Vec<CollaborationModeMask> {
    vec![
        default_preset(collaboration_modes_config),
        plan_preset(collaboration_modes_config),
        ultra_work_preset(collaboration_modes_config),
    ]
}

fn plan_preset(_collaboration_modes_config: CollaborationModesConfig) -> CollaborationModeMask {
    CollaborationModeMask {
        name: ModeKind::Plan.display_name().to_string(),
        mode: Some(ModeKind::Plan),
        plan_phase: None,
        model: None,
        reasoning_effort: Some(Some(ReasoningEffort::Medium)),
        developer_instructions: Some(Some(plan_mode_instructions(
            COLLABORATION_MODE_PLAN,
            /*plan_workflow_enabled*/ false,
        ))),
    }
}

fn default_preset(collaboration_modes_config: CollaborationModesConfig) -> CollaborationModeMask {
    CollaborationModeMask {
        name: ModeKind::Default.display_name().to_string(),
        mode: Some(ModeKind::Default),
        plan_phase: None,
        model: None,
        reasoning_effort: None,
        developer_instructions: Some(Some(default_mode_instructions(collaboration_modes_config))),
    }
}

fn default_mode_instructions(collaboration_modes_config: CollaborationModesConfig) -> String {
    let known_mode_names = format_mode_names(&TUI_VISIBLE_COLLABORATION_MODES);
    let request_user_input_availability = request_user_input_availability_message(
        ModeKind::Default,
        collaboration_modes_config.default_mode_request_user_input,
    );
    let asking_questions_guidance = asking_questions_guidance_message(
        collaboration_modes_config.default_mode_request_user_input,
    );
    COLLABORATION_MODE_DEFAULT
        .replace(KNOWN_MODE_NAMES_PLACEHOLDER, &known_mode_names)
        .replace(
            REQUEST_USER_INPUT_AVAILABILITY_PLACEHOLDER,
            &request_user_input_availability,
        )
        .replace(
            ASKING_QUESTIONS_GUIDANCE_PLACEHOLDER,
            &asking_questions_guidance,
        )
}

fn ultra_work_preset(
    collaboration_modes_config: CollaborationModesConfig,
) -> CollaborationModeMask {
    CollaborationModeMask {
        name: ModeKind::UltraWork.display_name().to_string(),
        mode: Some(ModeKind::UltraWork),
        plan_phase: Some(PlanModePhase::Planning),
        model: None,
        reasoning_effort: Some(Some(ReasoningEffort::Medium)),
        developer_instructions: Some(Some(ultra_work_mode_instructions(
            collaboration_modes_config.plan_workflow_enabled,
        ))),
    }
}

fn plan_mode_instructions(template: &str, plan_workflow_enabled: bool) -> String {
    if plan_workflow_enabled {
        template.replace(
            PLAN_PREPARATORY_MUTATIONS_GUIDANCE_PLACEHOLDER,
            &plan_preparatory_mutations_guidance(),
        )
    } else {
        template.to_string()
    }
}

fn ultra_work_mode_instructions(plan_workflow_enabled: bool) -> String {
    if plan_workflow_enabled {
        plan_mode_instructions(COLLABORATION_MODE_ULTRA_WORK, true)
    } else {
        COLLABORATION_MODE_ULTRA_WORK.to_string()
    }
}

pub fn ultra_work_execution_instructions(plan_workflow_enabled: bool) -> String {
    let _ = plan_workflow_enabled;
    COLLABORATION_MODE_ULTRA_WORK_EXECUTE.to_string()
}

fn format_mode_names(modes: &[ModeKind]) -> String {
    let mode_names: Vec<&str> = modes.iter().map(|mode| mode.display_name()).collect();
    match mode_names.as_slice() {
        [] => "none".to_string(),
        [mode_name] => (*mode_name).to_string(),
        [first, second] => format!("{first} and {second}"),
        [..] => mode_names.join(", "),
    }
}

fn request_user_input_availability_message(
    mode: ModeKind,
    default_mode_request_user_input: bool,
) -> String {
    let mode_name = mode.display_name();
    if mode.allows_request_user_input()
        || (default_mode_request_user_input && mode == ModeKind::Default)
    {
        format!("The `request_user_input` tool is available in {mode_name} mode.")
    } else {
        format!(
            "The `request_user_input` tool is unavailable in {mode_name} mode. If you call it while in {mode_name} mode, it will return an error."
        )
    }
}

fn asking_questions_guidance_message(default_mode_request_user_input: bool) -> String {
    if default_mode_request_user_input {
        "In Default mode, strongly prefer making reasonable assumptions and executing the user's request rather than stopping to ask questions. If you absolutely must ask a question because the answer cannot be discovered from local context and a reasonable assumption would be risky, prefer using the `request_user_input` tool rather than writing a multiple choice question as a textual assistant message. Never write a multiple choice question as a textual assistant message.".to_string()
    } else {
        "In Default mode, strongly prefer making reasonable assumptions and executing the user's request rather than stopping to ask questions. If you absolutely must ask a question because the answer cannot be discovered from local context and a reasonable assumption would be risky, ask the user directly with a concise plain-text question. Never write a multiple choice question as a textual assistant message.".to_string()
    }
}

fn plan_preparatory_mutations_guidance() -> String {
    "When `features.plan_workflow` is enabled, you may perform **preparatory mutations** that improve the plan without implementing it. These are side-effectful setup actions whose purpose is to gather truth or create a temporary analysis environment outside the target repo. When you need to inspect another code repository, prefer direct `git` access over `web search`. Clone or fetch into a temporary directory or other scratch location outside the current target repo, keep the checkout shallow, and avoid pulling unnecessary history. If the user specifies a branch, prefer `git clone --depth 1 --single-branch --branch <branch>` or the matching shallow `git fetch` for that branch. If the user does not specify a branch, resolve the remote default branch first, then use the same shallow single-branch strategy for that default branch. Use `web search` only after you already have the repository locally and only to supplement documentation or background context, not as the primary source for repository code. Other allowed examples include downloading read-only reference material into a temporary directory and generating temporary artifacts in a scratch workspace that exists only to inspect, build, or analyze. These actions must stay outside the current target repo and must not modify or create implementation files inside the target repo.\n\nEven when preparatory mutations are enabled, you must still not edit tracked files in the current target repo, run codegen/formatters/migrations that rewrite files in the current target repo, or carry out implementation work under the guise of planning. If a side-effectful setup action would write inside the current target repo, do not do it during Ultra Work planning; use a temporary or scratch directory instead.".to_string()
}

#[cfg(test)]
#[path = "collaboration_mode_presets_tests.rs"]
mod tests;
