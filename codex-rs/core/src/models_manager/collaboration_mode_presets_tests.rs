use super::*;
use pretty_assertions::assert_eq;

#[test]
fn preset_names_use_mode_display_names() {
    assert_eq!(
        plan_preset(CollaborationModesConfig::default()).name,
        ModeKind::Plan.display_name()
    );
    assert_eq!(
        auto_plan_preset(CollaborationModesConfig::default()).name,
        ModeKind::AutoPlan.display_name()
    );
    assert_eq!(
        default_preset(CollaborationModesConfig::default()).name,
        ModeKind::Default.display_name()
    );
    assert_eq!(
        execute_preset(CollaborationModesConfig::default()).name,
        ModeKind::Execute.display_name()
    );
    assert_eq!(
        plan_preset(CollaborationModesConfig::default()).reasoning_effort,
        Some(Some(ReasoningEffort::Medium))
    );
    assert_eq!(
        auto_plan_preset(CollaborationModesConfig::default()).reasoning_effort,
        Some(Some(ReasoningEffort::Medium))
    );
}

#[test]
fn execute_preset_uses_mode_display_name() {
    assert_eq!(
        execute_preset(CollaborationModesConfig::default()).name,
        ModeKind::Execute.display_name()
    );
}

#[test]
fn execute_preset_includes_execute_instructions() {
    let instructions = execute_preset(CollaborationModesConfig::default())
        .developer_instructions
        .expect("execute preset should include instructions")
        .expect("execute instructions should be set");

    assert!(instructions.contains("You execute on a well-specified task independently"));
}

#[test]
fn execute_preset_uses_plan_execution_instructions_when_plan_workflow_enabled() {
    let instructions = execute_preset(CollaborationModesConfig {
        default_mode_request_user_input: false,
        plan_workflow_enabled: true,
    })
    .developer_instructions
    .expect("execute preset should include instructions")
    .expect("execute instructions should be set");

    assert!(instructions.contains("accepted active plan is absolute truth during Plan execution"));
    assert!(instructions.contains("read provided tasks.csv path before acting"));
    assert!(instructions.contains("only execute the server-selected row"));
    assert!(instructions.contains("Record plan-external work only in `update_plan.explanation`"));
    assert!(instructions.contains("automatic plan-dispatch tool"));
}

#[test]
fn default_mode_instructions_replace_mode_names_placeholder() {
    let default_instructions = default_preset(CollaborationModesConfig {
        default_mode_request_user_input: true,
        plan_workflow_enabled: false,
    })
    .developer_instructions
    .expect("default preset should include instructions")
    .expect("default instructions should be set");

    assert!(!default_instructions.contains(KNOWN_MODE_NAMES_PLACEHOLDER));
    assert!(!default_instructions.contains(REQUEST_USER_INPUT_AVAILABILITY_PLACEHOLDER));
    assert!(!default_instructions.contains(ASKING_QUESTIONS_GUIDANCE_PLACEHOLDER));

    let known_mode_names = format_mode_names(&TUI_VISIBLE_COLLABORATION_MODES);
    let expected_snippet = format!("Known mode names are {known_mode_names}.");
    assert!(default_instructions.contains(&expected_snippet));

    let expected_availability_message =
        request_user_input_availability_message(ModeKind::Default, true);
    assert!(default_instructions.contains(&expected_availability_message));
    assert!(default_instructions.contains("prefer using the `request_user_input` tool"));
}

#[test]
fn default_mode_instructions_use_plain_text_questions_when_feature_disabled() {
    let default_instructions = default_preset(CollaborationModesConfig::default())
        .developer_instructions
        .expect("default preset should include instructions")
        .expect("default instructions should be set");

    assert!(!default_instructions.contains("prefer using the `request_user_input` tool"));
    assert!(
        default_instructions.contains("ask the user directly with a concise plain-text question")
    );
}

#[test]
fn plan_mode_instructions_disallow_preparatory_mutations_by_default() {
    let instructions = plan_preset(CollaborationModesConfig::default())
        .developer_instructions
        .expect("plan preset should include instructions")
        .expect("plan instructions should be set");

    assert!(!instructions.contains("tasks.csv"));
    assert!(!instructions.contains("plan workspace"));
}

#[test]
fn plan_mode_instructions_use_plan_workflow_when_enabled() {
    let instructions = plan_preset(CollaborationModesConfig {
        default_mode_request_user_input: false,
        plan_workflow_enabled: true,
    })
    .developer_instructions
    .expect("plan preset should include instructions")
    .expect("plan instructions should be set");

    assert!(instructions.contains("`features.plan_workflow`"));
    assert!(instructions.contains("tasks.csv"));
    assert!(instructions.contains("plan workspace"));
    assert!(instructions.contains("must stay outside the current target repo"));
    assert!(instructions.contains("prefer direct `git` access over `web search`"));
    assert!(instructions.contains("`git clone --depth 1 --single-branch --branch <branch>`"));
    assert!(instructions.contains("resolve the remote default branch first"));
    assert!(
        instructions
            .contains("Use `web search` only after you already have the repository locally")
    );
    assert!(instructions.contains("not as the primary source for repository code"));
}

#[test]
fn builtin_presets_keep_auto_plan_for_backward_compatibility() {
    let presets = builtin_collaboration_mode_presets(CollaborationModesConfig::default());
    let modes: Vec<Option<ModeKind>> = presets.into_iter().map(|preset| preset.mode).collect();
    assert_eq!(
        modes,
        vec![
            Some(ModeKind::Plan),
            Some(ModeKind::AutoPlan),
            Some(ModeKind::Default),
            Some(ModeKind::Execute),
        ]
    );
}
