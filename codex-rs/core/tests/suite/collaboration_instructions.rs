use anyhow::Result;
use std::sync::Arc;

use codex_core::features::Feature;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::PlanModePhase;
use codex_protocol::config_types::Settings;
use codex_protocol::protocol::COLLABORATION_MODE_CLOSE_TAG;
use codex_protocol::protocol::COLLABORATION_MODE_OPEN_TAG;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use codex_state::ThreadPlanSnapshotCreateParams;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use tempfile::TempDir;

fn collab_mode_with_mode_and_instructions(
    mode: ModeKind,
    instructions: Option<&str>,
) -> CollaborationMode {
    CollaborationMode {
        mode,
        plan_phase: mode.is_plan_mode().then_some(PlanModePhase::Planning),
        settings: Settings {
            model: "gpt-5.1".to_string(),
            reasoning_effort: None,
            developer_instructions: instructions.map(str::to_string),
        },
    }
}

fn collab_mode_with_instructions(instructions: Option<&str>) -> CollaborationMode {
    collab_mode_with_mode_and_instructions(ModeKind::Default, instructions)
}

fn developer_texts(input: &[Value]) -> Vec<String> {
    input
        .iter()
        .filter(|item| item.get("role").and_then(Value::as_str) == Some("developer"))
        .filter_map(|item| item.get("content")?.as_array().cloned())
        .flatten()
        .filter_map(|content| {
            let text = content.get("text")?.as_str()?;
            Some(text.to_string())
        })
        .collect()
}

fn developer_message_count(input: &[Value]) -> usize {
    input
        .iter()
        .filter(|item| item.get("role").and_then(Value::as_str) == Some("developer"))
        .count()
}

fn collab_xml(text: &str) -> String {
    format!("{COLLABORATION_MODE_OPEN_TAG}{text}{COLLABORATION_MODE_CLOSE_TAG}")
}

fn count_messages_containing(texts: &[String], target: &str) -> usize {
    texts.iter().filter(|text| text.contains(target)).count()
}

fn count_collaboration_mode_messages(texts: &[String]) -> usize {
    count_messages_containing(texts, COLLABORATION_MODE_OPEN_TAG)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_collaboration_instructions_by_default() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let req = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let test = test_codex().build(&server).await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req.single_request().input();
    assert_eq!(developer_message_count(&input), 1);
    let dev_texts = developer_texts(&input);
    assert!(
        dev_texts
            .iter()
            .any(|text| text.contains("<permissions instructions>")),
        "expected permissions instructions in developer messages, got {dev_texts:?}"
    );
    assert_eq!(
        count_messages_containing(&dev_texts, COLLABORATION_MODE_OPEN_TAG),
        0
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_input_includes_collaboration_instructions_after_override() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let req = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let test = test_codex().build(&server).await?;

    let collab_text = "collab instructions";
    let collaboration_mode = collab_mode_with_instructions(Some(collab_text));
    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collaboration_mode),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req.single_request().input();
    let dev_texts = developer_texts(&input);
    let collab_text = collab_xml(collab_text);
    assert_eq!(count_messages_containing(&dev_texts, &collab_text), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collaboration_instructions_added_on_user_turn() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let req = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let test = test_codex().build(&server).await?;
    let collab_text = "turn instructions";
    let collaboration_mode = collab_mode_with_instructions(Some(collab_text));

    test.codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "hello".into(),
                text_elements: Vec::new(),
            }],
            cwd: test.config.cwd.to_path_buf(),
            approval_policy: test.config.permissions.approval_policy.value(),
            approvals_reviewer: None,
            sandbox_policy: test.config.permissions.sandbox_policy.get().clone(),
            model: test.session_configured.model.clone(),
            effort: None,
            summary: Some(
                test.config
                    .model_reasoning_summary
                    .unwrap_or(codex_protocol::config_types::ReasoningSummary::Auto),
            ),
            service_tier: None,
            collaboration_mode: Some(collaboration_mode),
            final_output_json_schema: None,
            personality: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req.single_request().input();
    let dev_texts = developer_texts(&input);
    let collab_text = collab_xml(collab_text);
    assert_eq!(count_messages_containing(&dev_texts, &collab_text), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn override_then_next_turn_uses_updated_collaboration_instructions() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let req = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let test = test_codex().build(&server).await?;
    let collab_text = "override instructions";
    let collaboration_mode = collab_mode_with_instructions(Some(collab_text));

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collaboration_mode),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req.single_request().input();
    let dev_texts = developer_texts(&input);
    let collab_text = collab_xml(collab_text);
    assert_eq!(count_messages_containing(&dev_texts, &collab_text), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_turn_overrides_collaboration_instructions_after_override() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let req = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let test = test_codex().build(&server).await?;
    let base_text = "base instructions";
    let base_mode = collab_mode_with_instructions(Some(base_text));
    let turn_text = "turn override";
    let turn_mode = collab_mode_with_instructions(Some(turn_text));

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(base_mode),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "hello".into(),
                text_elements: Vec::new(),
            }],
            cwd: test.config.cwd.to_path_buf(),
            approval_policy: test.config.permissions.approval_policy.value(),
            approvals_reviewer: None,
            sandbox_policy: test.config.permissions.sandbox_policy.get().clone(),
            model: test.session_configured.model.clone(),
            effort: None,
            summary: Some(
                test.config
                    .model_reasoning_summary
                    .unwrap_or(codex_protocol::config_types::ReasoningSummary::Auto),
            ),
            service_tier: None,
            collaboration_mode: Some(turn_mode),
            final_output_json_schema: None,
            personality: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req.single_request().input();
    let dev_texts = developer_texts(&input);
    let base_text = collab_xml(base_text);
    let turn_text = collab_xml(turn_text);
    assert_eq!(count_messages_containing(&dev_texts, &base_text), 0);
    assert_eq!(count_messages_containing(&dev_texts, &turn_text), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collaboration_mode_update_emits_new_instruction_message() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let _req1 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;
    let req2 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
    )
    .await;

    let test = test_codex().build(&server).await?;
    let first_text = "first instructions";
    let second_text = "second instructions";

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_instructions(Some(first_text))),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 1".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_instructions(Some(second_text))),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 2".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req2.single_request().input();
    let dev_texts = developer_texts(&input);
    let first_text = collab_xml(first_text);
    let second_text = collab_xml(second_text);
    assert_eq!(count_messages_containing(&dev_texts, &first_text), 0);
    assert_eq!(count_messages_containing(&dev_texts, &second_text), 1);
    assert_eq!(count_collaboration_mode_messages(&dev_texts), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collaboration_mode_update_noop_does_not_append() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let _req1 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;
    let req2 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
    )
    .await;

    let test = test_codex().build(&server).await?;
    let collab_text = "same instructions";

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_instructions(Some(collab_text))),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 1".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_instructions(Some(collab_text))),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 2".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req2.single_request().input();
    let dev_texts = developer_texts(&input);
    let collab_text = collab_xml(collab_text);
    assert_eq!(count_messages_containing(&dev_texts, &collab_text), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collaboration_mode_update_emits_new_instruction_message_when_mode_changes() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let _req1 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;
    let req2 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
    )
    .await;

    let test = test_codex().build(&server).await?;
    let default_text = "default mode instructions";
    let plan_text = "plan mode instructions";

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_mode_and_instructions(
                ModeKind::Default,
                Some(default_text),
            )),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 1".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_mode_and_instructions(
                ModeKind::Plan,
                Some(plan_text),
            )),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 2".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req2.single_request().input();
    let dev_texts = developer_texts(&input);
    let default_text = collab_xml(default_text);
    let plan_text = collab_xml(plan_text);
    assert_eq!(count_messages_containing(&dev_texts, &default_text), 0);
    assert_eq!(count_messages_containing(&dev_texts, &plan_text), 1);
    assert_eq!(count_collaboration_mode_messages(&dev_texts), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collaboration_mode_switch_from_plan_to_default_only_keeps_default_prompt() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let _req1 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;
    let req2 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
    )
    .await;

    let test = test_codex().build(&server).await?;
    let plan_text = "plan mode instructions";
    let default_text = "default mode instructions";

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_mode_and_instructions(
                ModeKind::Plan,
                Some(plan_text),
            )),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 1".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_mode_and_instructions(
                ModeKind::Default,
                Some(default_text),
            )),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 2".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req2.single_request().input();
    let dev_texts = developer_texts(&input);
    let plan_text = collab_xml(plan_text);
    let default_text = collab_xml(default_text);
    assert_eq!(count_messages_containing(&dev_texts, &plan_text), 0);
    assert_eq!(count_messages_containing(&dev_texts, &default_text), 1);
    assert_eq!(count_collaboration_mode_messages(&dev_texts), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collaboration_mode_update_noop_does_not_append_when_mode_is_unchanged() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let _req1 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;
    let req2 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
    )
    .await;

    let test = test_codex().build(&server).await?;
    let collab_text = "mode-stable instructions";

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_mode_and_instructions(
                ModeKind::Default,
                Some(collab_text),
            )),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 1".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_mode_and_instructions(
                ModeKind::Default,
                Some(collab_text),
            )),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 2".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req2.single_request().input();
    let dev_texts = developer_texts(&input);
    let collab_text = collab_xml(collab_text);
    assert_eq!(count_messages_containing(&dev_texts, &collab_text), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_replays_collaboration_instructions() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let _req1 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;
    let req2 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
    )
    .await;

    let mut builder = test_codex();
    let initial = builder.build(&server).await?;
    let rollout_path = initial
        .session_configured
        .rollout_path
        .clone()
        .expect("rollout path");
    let home = initial.home.clone();

    let collab_text = "resume instructions";
    initial
        .codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_instructions(Some(collab_text))),
            personality: None,
        })
        .await?;

    initial
        .codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&initial.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let resumed = builder.resume(&server, home, rollout_path).await?;
    resumed
        .codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "after resume".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&resumed.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req2.single_request().input();
    let dev_texts = developer_texts(&input);
    let collab_text = collab_xml(collab_text);
    assert_eq!(count_messages_containing(&dev_texts, &collab_text), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_collaboration_instructions_are_ignored() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let req = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let test = test_codex().build(&server).await?;
    let current_model = test.session_configured.model.clone();

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(CollaborationMode {
                mode: ModeKind::Default,
                plan_phase: None,
                settings: Settings {
                    model: current_model,
                    reasoning_effort: None,
                    developer_instructions: Some("".to_string()),
                },
            }),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req.single_request().input();
    assert_eq!(developer_message_count(&input), 1);
    let dev_texts = developer_texts(&input);
    let collab_text = collab_xml("");
    assert_eq!(count_messages_containing(&dev_texts, &collab_text), 0);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clearing_collaboration_instructions_removes_stale_prompt() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let _req1 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;
    let req2 = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
    )
    .await;

    let test = test_codex().build(&server).await?;
    let collab_text = "plan mode instructions";
    let current_model = test.session_configured.model.clone();

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collab_mode_with_mode_and_instructions(
                ModeKind::Plan,
                Some(collab_text),
            )),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 1".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(CollaborationMode {
                mode: ModeKind::Default,
                plan_phase: None,
                settings: Settings {
                    model: current_model,
                    reasoning_effort: None,
                    developer_instructions: Some("".to_string()),
                },
            }),
            personality: None,
        })
        .await?;

    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "hello 2".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req2.single_request().input();
    let dev_texts = developer_texts(&input);
    let collab_text = collab_xml(collab_text);
    assert_eq!(count_messages_containing(&dev_texts, &collab_text), 0);
    assert_eq!(count_collaboration_mode_messages(&dev_texts), 0);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ultra_work_execution_injects_plan_paths_and_current_row_only() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let req = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let home = Arc::new(TempDir::new()?);
    let test = test_codex()
        .with_home(home)
        .with_config(|config| {
            config
                .features
                .enable(Feature::PlanWorkflow)
                .expect("feature should enable in test");
        })
        .build(&server)
        .await?;
    let db = test.codex.state_db().expect("state db");
    db.replace_active_thread_plan(&ThreadPlanSnapshotCreateParams {
        id: "snapshot-1".to_string(),
        thread_id: test.session_configured.session_id.to_string(),
        source_turn_id: "turn-1".to_string(),
        source_item_id: "item-1".to_string(),
        raw_csv: "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,pending,Guard execute,codex-rs/core/src/execute_plan_guard.rs,add execute plan guard,,,,guard selects current row
plan-02,pending,Wire handler,codex-rs/core/src/tools/handlers/plan.rs,gate update_plan,,,plan-01,handler rejects invalid updates
"
        .to_string(),
    })
    .await?;

    test.codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "run execute mode".into(),
                text_elements: Vec::new(),
            }],
            cwd: test.config.cwd.clone(),
            approval_policy: test.config.permissions.approval_policy.value(),
            sandbox_policy: test.config.permissions.sandbox_policy.get().clone(),
            model: test.session_configured.model.clone(),
            effort: None,
            summary: Some(
                test.config
                    .model_reasoning_summary
                    .unwrap_or(codex_protocol::config_types::ReasoningSummary::Auto),
            ),
            service_tier: None,
            collaboration_mode: Some(CollaborationMode {
                mode: ModeKind::UltraWork,
                plan_phase: Some(PlanModePhase::Executing),
                settings: Settings {
                    model: test.session_configured.model.clone(),
                    reasoning_effort: None,
                    developer_instructions: None,
                },
            }),
            final_output_json_schema: None,
            personality: None,
        })
        .await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let input = req.single_request().input();
    let dev_texts = developer_texts(&input);
    let joined = dev_texts.join("\n");
    assert!(joined.contains("tasks.csv"));
    assert!(joined.contains("tasks.md"));
    assert!(joined.contains("plan-01"));
    assert!(joined.contains("Guard execute"));
    assert!(joined.contains("codex-rs/core/src/execute_plan_guard.rs"));
    assert!(!joined.contains("plan-02,pending,Wire handler"));
    assert!(!joined.contains("handler rejects invalid updates"));

    Ok(())
}
