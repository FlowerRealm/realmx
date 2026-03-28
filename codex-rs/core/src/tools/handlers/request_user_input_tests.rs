use super::*;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::PlanModePhase;
use codex_protocol::config_types::Settings;
use pretty_assertions::assert_eq;

#[test]
fn request_user_input_mode_availability_defaults_to_plan_only() {
    assert!(ModeKind::Plan.allows_request_user_input());
    assert!(ModeKind::UltraWork.allows_request_user_input());
    assert!(!ModeKind::Default.allows_request_user_input());
    assert!(!ModeKind::Execute.allows_request_user_input());
    assert!(!ModeKind::PairProgramming.allows_request_user_input());
}

#[test]
fn request_user_input_unavailable_messages_respect_default_mode_feature_flag() {
    assert_eq!(
        request_user_input_unavailable_message(ModeKind::Plan, false),
        None
    );
    assert_eq!(
        request_user_input_unavailable_message(ModeKind::UltraWork, false),
        None
    );
    assert_eq!(
        request_user_input_unavailable_message(ModeKind::Default, false),
        Some("request_user_input is unavailable in Default mode".to_string())
    );
    assert_eq!(
        request_user_input_unavailable_message(ModeKind::Default, true),
        None
    );
    assert_eq!(
        request_user_input_unavailable_message(ModeKind::Execute, false),
        Some("request_user_input is unavailable in Ultra Work execution phase".to_string())
    );
    assert_eq!(
        request_user_input_unavailable_message(ModeKind::PairProgramming, false),
        Some("request_user_input is unavailable in Pair Programming mode".to_string())
    );
}

#[test]
fn request_user_input_tool_description_mentions_available_modes() {
    assert_eq!(
        request_user_input_tool_description(false),
        "Request user input for one to three short questions and wait for the response. This tool is only available in Plan or Ultra Work mode.".to_string()
    );
    assert_eq!(
        request_user_input_tool_description(true),
        "Request user input for one to three short questions and wait for the response. This tool is only available in Default, Plan, or Ultra Work mode.".to_string()
    );
}

#[test]
fn request_user_input_unavailable_message_mentions_plan_execution_phase() {
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::UltraWork,
        plan_phase: Some(PlanModePhase::Executing),
        settings: Settings {
            model: "gpt-5".to_string(),
            reasoning_effort: None,
            developer_instructions: None,
        },
    };

    assert_eq!(
        request_user_input_unavailable_message_for_collaboration_mode(&collaboration_mode, false),
        Some("request_user_input is unavailable in Ultra Work execution phase".to_string())
    );
}
