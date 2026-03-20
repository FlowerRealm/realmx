use super::*;
use pretty_assertions::assert_eq;

#[test]
fn preset_names_use_mode_display_names() {
    assert_eq!(
        plan_preset(CollaborationModesConfig::default()).name,
        ModeKind::Plan.display_name()
    );
    assert_eq!(
        default_preset(CollaborationModesConfig::default()).name,
        ModeKind::Default.display_name()
    );
    assert_eq!(
        plan_preset(CollaborationModesConfig::default()).reasoning_effort,
        Some(Some(ReasoningEffort::Medium))
    );
}

#[test]
fn default_mode_instructions_replace_mode_names_placeholder() {
    let default_instructions = default_preset(CollaborationModesConfig {
        default_mode_request_user_input: true,
        plan_mode_preparatory_mutations: false,
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

    assert!(instructions.contains("Do not perform **mutating** actions in Plan mode."));
    assert!(instructions.contains("`git clone`"));
}

#[test]
fn plan_mode_instructions_allow_preparatory_mutations_when_enabled() {
    let instructions = plan_preset(CollaborationModesConfig {
        default_mode_request_user_input: false,
        plan_mode_preparatory_mutations: true,
    })
    .developer_instructions
    .expect("plan preset should include instructions")
    .expect("plan instructions should be set");

    assert!(instructions.contains("`features.plan_mode_preparatory_mutations`"));
    assert!(instructions.contains("must stay outside the current target repo"));
}
