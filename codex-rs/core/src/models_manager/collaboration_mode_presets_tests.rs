use super::*;
use pretty_assertions::assert_eq;

#[test]
fn preset_names_use_mode_display_names() {
    assert_eq!(
        plan_preset(CollaborationModesConfig::default()).name,
        ModeKind::Plan.display_name()
    );
    assert_eq!(
        ultra_work_preset(CollaborationModesConfig::default()).name,
        ModeKind::UltraWork.display_name()
    );
    assert_eq!(
        default_preset(CollaborationModesConfig::default()).name,
        ModeKind::Default.display_name()
    );
    assert_eq!(
        plan_preset(CollaborationModesConfig::default()).reasoning_effort,
        Some(Some(ReasoningEffort::Medium))
    );
    assert_eq!(
        ultra_work_preset(CollaborationModesConfig::default()).reasoning_effort,
        Some(Some(ReasoningEffort::Medium))
    );
}

#[test]
fn ultra_work_preset_uses_mode_display_name() {
    assert_eq!(
        ultra_work_preset(CollaborationModesConfig::default()).name,
        ModeKind::UltraWork.display_name()
    );
}

#[test]
fn ultra_work_execute_instructions_include_execute_guidance() {
    let instructions = ultra_work_execution_instructions(/*plan_workflow_enabled*/ false);

    assert!(instructions.contains("execution phase of Ultra Work"));
    assert!(instructions.contains("accepted active plan is absolute truth"));
}

#[test]
fn ultra_work_preset_includes_workspace_instructions() {
    let instructions = ultra_work_preset(CollaborationModesConfig::default())
        .developer_instructions
        .expect("ultra work preset should include instructions")
        .expect("ultra work instructions should be set");

    assert!(instructions.contains("Ultra Work"));
    assert!(instructions.contains("tasks.csv"));
    assert!(instructions.contains("plan workspace"));
}

#[test]
fn ultra_work_execute_instructions_use_plan_execution_instructions_when_plan_workflow_enabled() {
    let instructions = ultra_work_execution_instructions(/*plan_workflow_enabled*/ true);

    assert!(
        instructions.contains("accepted active plan is absolute truth during Ultra Work execution")
    );
    assert!(instructions.contains("read the provided `tasks.csv` path before acting"));
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
fn plan_mode_instructions_do_not_use_ultra_work_workspace() {
    let instructions = plan_preset(CollaborationModesConfig::default())
        .developer_instructions
        .expect("plan preset should include instructions")
        .expect("plan instructions should be set");

    assert!(!instructions.contains("tasks.csv"));
    assert!(!instructions.contains("plan workspace"));
    assert!(!instructions.contains("Ultra Work"));
}

#[test]
fn ultra_work_mode_instructions_use_plan_workflow_when_enabled() {
    let instructions = ultra_work_preset(CollaborationModesConfig {
        default_mode_request_user_input: false,
        plan_workflow_enabled: true,
    })
    .developer_instructions
    .expect("ultra work preset should include instructions")
    .expect("ultra work instructions should be set");

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
fn builtin_presets_expose_default_plan_and_ultra_work() {
    let presets = builtin_collaboration_mode_presets(CollaborationModesConfig::default());
    let modes: Vec<Option<ModeKind>> = presets.into_iter().map(|preset| preset.mode).collect();
    assert_eq!(
        modes,
        vec![
            Some(ModeKind::Default),
            Some(ModeKind::Plan),
            Some(ModeKind::UltraWork),
        ]
    );
}
