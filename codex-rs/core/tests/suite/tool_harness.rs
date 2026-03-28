#![cfg(not(target_os = "windows"))]

use std::fs;
use std::sync::Arc;

use assert_matches::assert_matches;
use codex_core::features::Feature;
use codex_core::plan_workspace::PlanWorkspace;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::PlanModePhase;
use codex_protocol::config_types::Settings;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::user_input::UserInput;
use codex_state::ThreadPlanItemStatus;
use codex_state::ThreadPlanSnapshotCreateParams;
use core_test_support::assert_regex_match;
use core_test_support::responses;
use core_test_support::responses::ResponsesRequest;
use core_test_support::responses::ev_apply_patch_function_call;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_local_shell_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;
fn call_output(req: &ResponsesRequest, call_id: &str) -> (String, Option<bool>) {
    let raw = req.function_call_output(call_id);
    assert_eq!(
        raw.get("call_id").and_then(Value::as_str),
        Some(call_id),
        "mismatched call_id in function_call_output"
    );
    let (content_opt, success) = match req.function_call_output_content_and_success(call_id) {
        Some(values) => values,
        None => panic!("function_call_output present"),
    };
    let content = match content_opt {
        Some(c) => c,
        None => panic!("function_call_output content present"),
    };
    (content, success)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_tool_executes_command_and_streams_output() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let mut builder = test_codex().with_model("gpt-5");
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let call_id = "shell-tool-call";
    let command = vec!["/bin/echo", "tool harness"];
    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_local_shell_call(call_id, "completed", command),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once(&server, first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "all done"),
        ev_completed("resp-2"),
    ]);
    let second_mock = responses::mount_sse_once(&server, second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "please run the shell command".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    wait_for_event(&codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let req = second_mock.single_request();
    let (output_text, _) = call_output(&req, call_id);
    let exec_output: Value = serde_json::from_str(&output_text)?;
    assert_eq!(exec_output["metadata"]["exit_code"], 0);
    let stdout = exec_output["output"].as_str().expect("stdout field");
    assert_regex_match(r"(?s)^tool harness\n?$", stdout);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_plan_tool_emits_plan_update_event() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let mut builder = test_codex();
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let call_id = "plan-tool-call";
    let plan_args = json!({
        "explanation": "Tool harness check",
        "plan": [
            {"step": "Inspect workspace", "status": "in_progress"},
            {"step": "Report results", "status": "pending"},
        ],
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "update_plan", &plan_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once(&server, first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "plan acknowledged"),
        ev_completed("resp-2"),
    ]);
    let second_mock = responses::mount_sse_once(&server, second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "please update the plan".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let mut saw_plan_update = false;
    wait_for_event(&codex, |event| match event {
        EventMsg::PlanUpdate(update) => {
            saw_plan_update = true;
            assert_eq!(update.explanation.as_deref(), Some("Tool harness check"));
            assert_eq!(update.plan.len(), 2);
            assert_eq!(update.plan[0].step, "Inspect workspace");
            assert_matches!(update.plan[0].status, StepStatus::InProgress);
            assert_eq!(update.plan[1].step, "Report results");
            assert_matches!(update.plan[1].status, StepStatus::Pending);
            false
        }
        EventMsg::TurnComplete(_) => true,
        _ => false,
    })
    .await;

    assert!(saw_plan_update, "expected PlanUpdate event");

    let req = second_mock.single_request();
    let (output_text, _success_flag) = call_output(&req, call_id);
    assert_eq!(output_text, "Plan updated");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_plan_tool_keeps_non_id_rows_when_active_plan_exists() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);

    let mut builder = test_codex().with_home(home);
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let db = codex.state_db().expect("state db");
    db.replace_active_thread_plan(&ThreadPlanSnapshotCreateParams {
        id: "snapshot-1".to_string(),
        thread_id: session_configured.session_id.to_string(),
        source_turn_id: "turn-1".to_string(),
        source_item_id: "item-1".to_string(),
        raw_csv: "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-1,pending,Inspect workspace,codex-rs/core/src/tools/handlers/plan.rs,sync state,,,,
"
        .to_string(),
    })
    .await?;

    let call_id = "plan-tool-mixed-rows";
    let plan_args = json!({
        "explanation": "Mixed update",
        "plan": [
            {"id": "plan-1", "step": "Inspect workspace", "status": "completed"},
            {"step": "Report results", "status": "pending"},
        ],
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "update_plan", &plan_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once(&server, first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "plan acknowledged"),
        ev_completed("resp-2"),
    ]);
    let second_mock = responses::mount_sse_once(&server, second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "please update the active plan".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let mut saw_plan_update = false;
    wait_for_event(&codex, |event| match event {
        EventMsg::PlanUpdate(update) => {
            saw_plan_update = true;
            assert_eq!(update.explanation.as_deref(), Some("Mixed update"));
            assert_eq!(update.plan.len(), 2);
            assert_eq!(update.plan[0].id.as_deref(), Some("plan-1"));
            assert_eq!(update.plan[0].step, "Inspect workspace");
            assert_matches!(update.plan[0].status, StepStatus::Completed);
            assert_eq!(
                update.plan[0].path.as_deref(),
                Some("codex-rs/core/src/tools/handlers/plan.rs")
            );
            assert_eq!(update.plan[0].details.as_deref(), Some("sync state"));
            assert_eq!(update.plan[1].id, None);
            assert_eq!(update.plan[1].step, "Report results");
            assert_matches!(update.plan[1].status, StepStatus::Pending);
            false
        }
        EventMsg::TurnComplete(_) => true,
        _ => false,
    })
    .await;

    assert!(saw_plan_update, "expected PlanUpdate event");

    let req = second_mock.single_request();
    let (output_text, _success_flag) = call_output(&req, call_id);
    assert_eq!(output_text, "Plan updated");

    let active_plan = db
        .get_active_thread_plan(session_configured.session_id.to_string().as_str())
        .await?
        .expect("active plan should remain stored");
    assert_eq!(active_plan.items.len(), 1);
    assert_eq!(active_plan.items[0].row_id, "plan-1");
    assert_eq!(active_plan.items[0].status, ThreadPlanItemStatus::Completed);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_plan_tool_with_feature_persists_canonical_active_plan() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);

    let mut builder = test_codex().with_home(home.clone()).with_config(|config| {
        let _ = config.features.enable(Feature::PlanWorkflow);
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let call_id = "plan-tool-structured";
    let plan_args = json!({
        "explanation": "Structured plan",
        "plan": [
            {
                "step": "Inspect workspace",
                "status": "in_progress",
                "path": "codex-rs/core/src/tools/handlers/plan.rs",
                "details": "persist canonical csv rows",
                "inputs": ["tool args"],
                "outputs": ["active plan"],
                "acceptance": "active plan stored"
            },
            {
                "step": "Refresh UI",
                "status": "pending",
                "path": "codex-rs/tui/src/history_cell.rs",
                "details": "show metadata in history",
                "inputs": ["active plan"],
                "outputs": ["plan progress cell"],
                "depends_on": ["plan-01"],
                "acceptance": "history shows structured rows"
            }
        ]
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "update_plan", &plan_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once(&server, first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "plan acknowledged"),
        ev_completed("resp-2"),
    ]);
    let second_mock = responses::mount_sse_once(&server, second_response).await;

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "please persist the plan".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_configured.model.clone(),
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let mut saw_plan_update = false;
    wait_for_event(&codex, |event| match event {
        EventMsg::PlanUpdate(update) => {
            saw_plan_update = true;
            assert_eq!(update.explanation.as_deref(), Some("Structured plan"));
            assert_eq!(update.plan.len(), 2);
            assert_eq!(update.plan[0].id.as_deref(), Some("plan-01"));
            assert_eq!(
                update.plan[0].path.as_deref(),
                Some("codex-rs/core/src/tools/handlers/plan.rs")
            );
            assert_eq!(
                update.plan[1].depends_on.as_deref(),
                Some(["plan-01".to_string()].as_slice())
            );
            false
        }
        EventMsg::TurnComplete(_) => true,
        _ => false,
    })
    .await;

    assert!(saw_plan_update, "expected PlanUpdate event");

    let req = second_mock.single_request();
    let (output_text, _success_flag) = call_output(&req, call_id);
    assert_eq!(output_text, "Plan updated");

    let db = codex.state_db().expect("state db");
    let active_plan = db
        .get_active_thread_plan(session_configured.session_id.to_string().as_str())
        .await?
        .expect("active plan should exist");
    assert_eq!(active_plan.items.len(), 2);
    assert_eq!(active_plan.items[0].row_id, "plan-01");
    assert_eq!(active_plan.items[1].row_id, "plan-02");
    assert_eq!(
        active_plan.snapshot.raw_csv,
        "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Inspect workspace,codex-rs/core/src/tools/handlers/plan.rs,persist canonical csv rows,tool args,active plan,,active plan stored
plan-02,pending,Refresh UI,codex-rs/tui/src/history_cell.rs,show metadata in history,active plan,plan progress cell,plan-01,history shows structured rows
"
    );

    let workspace = PlanWorkspace::new(
        home.path(),
        cwd.path(),
        session_configured.session_id.to_string().as_str(),
    );
    let snapshot = workspace.snapshot().await?;
    assert_eq!(
        snapshot.active_tasks_csv.as_deref(),
        Some(active_plan.snapshot.raw_csv.as_str())
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_plan_tool_with_feature_expands_status_patch_to_full_plan() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);

    let mut builder = test_codex().with_home(home).with_config(|config| {
        let _ = config.features.enable(Feature::PlanWorkflow);
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let db = codex.state_db().expect("state db");
    db.replace_active_thread_plan(&ThreadPlanSnapshotCreateParams {
        id: "snapshot-1".to_string(),
        thread_id: session_configured.session_id.to_string(),
        source_turn_id: "turn-1".to_string(),
        source_item_id: "item-1".to_string(),
        raw_csv: "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-1,pending,Inspect workspace,codex-rs/core/src/tools/handlers/plan.rs,sync state,,,,
plan-2,pending,Refresh UI,codex-rs/tui/src/history_cell.rs,render plan,,,,history cell updated
"
        .to_string(),
    })
    .await?;

    let call_id = "plan-tool-status-patch";
    let plan_args = json!({
        "explanation": "Status update",
        "plan": [
            {"id": "plan-1", "step": "Inspect workspace", "status": "completed"}
        ],
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "update_plan", &plan_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once(&server, first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "status patch applied"),
        ev_completed("resp-2"),
    ]);
    let second_mock = responses::mount_sse_once(&server, second_response).await;

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "please patch the plan status".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_configured.model.clone(),
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let mut saw_plan_update = false;
    wait_for_event(&codex, |event| match event {
        EventMsg::PlanUpdate(update) => {
            saw_plan_update = true;
            assert_eq!(update.explanation.as_deref(), Some("Status update"));
            assert_eq!(update.plan.len(), 2);
            assert_eq!(update.plan[0].id.as_deref(), Some("plan-1"));
            assert_matches!(update.plan[0].status, StepStatus::Completed);
            assert_eq!(
                update.plan[0].path.as_deref(),
                Some("codex-rs/core/src/tools/handlers/plan.rs")
            );
            assert_eq!(update.plan[1].id.as_deref(), Some("plan-2"));
            assert_matches!(update.plan[1].status, StepStatus::Pending);
            false
        }
        EventMsg::TurnComplete(_) => true,
        _ => false,
    })
    .await;

    assert!(saw_plan_update, "expected canonicalized PlanUpdate event");

    let req = second_mock.single_request();
    let (output_text, _success_flag) = call_output(&req, call_id);
    assert_eq!(output_text, "Plan updated");

    let active_plan = db
        .get_active_thread_plan(session_configured.session_id.to_string().as_str())
        .await?
        .expect("active plan should remain stored");
    assert_eq!(active_plan.items.len(), 2);
    assert_eq!(active_plan.items[0].status, ThreadPlanItemStatus::Completed);
    assert_eq!(active_plan.items[1].status, ThreadPlanItemStatus::Pending);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_plan_tool_with_feature_rejects_unstructured_new_plan() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);

    let mut builder = test_codex().with_home(home).with_config(|config| {
        let _ = config.features.enable(Feature::PlanWorkflow);
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let call_id = "plan-tool-unstructured";
    let invalid_args = json!({
        "explanation": "Missing path",
        "plan": [
            {"step": "Inspect workspace", "status": "in_progress"}
        ]
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "update_plan", &invalid_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once(&server, first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "bad plan rejected"),
        ev_completed("resp-2"),
    ]);
    let second_mock = responses::mount_sse_once(&server, second_response).await;

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "please persist the plan".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_configured.model.clone(),
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let mut saw_plan_update = false;
    wait_for_event(&codex, |event| match event {
        EventMsg::PlanUpdate(_) => {
            saw_plan_update = true;
            false
        }
        EventMsg::TurnComplete(_) => true,
        _ => false,
    })
    .await;

    assert!(
        !saw_plan_update,
        "did not expect PlanUpdate event for unstructured payload"
    );

    let req = second_mock.single_request();
    let (output_text, _success_flag) = call_output(&req, call_id);
    assert!(output_text.contains("failed to canonicalize plan update"));
    assert!(output_text.contains("plan update row plan-01 is missing path"));

    let db = codex.state_db().expect("state db");
    let active_plan = db
        .get_active_thread_plan(session_configured.session_id.to_string().as_str())
        .await?;
    assert_eq!(active_plan, None);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_plan_tool_rejects_unknown_active_plan_ids() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);

    let mut builder = test_codex().with_home(home);
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let db = codex.state_db().expect("state db");
    db.replace_active_thread_plan(&ThreadPlanSnapshotCreateParams {
        id: "snapshot-1".to_string(),
        thread_id: session_configured.session_id.to_string(),
        source_turn_id: "turn-1".to_string(),
        source_item_id: "item-1".to_string(),
        raw_csv: "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-1,pending,Inspect workspace,codex-rs/core/src/tools/handlers/plan.rs,sync state,,,,
"
        .to_string(),
    })
    .await?;

    let call_id = "plan-tool-unknown-id";
    let plan_args = json!({
        "explanation": "Bad id",
        "plan": [
            {"id": "plan-1", "step": "Inspect workspace", "status": "completed"},
            {"id": "missing-row", "step": "Report results", "status": "pending"},
        ],
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "update_plan", &plan_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once(&server, first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "bad id rejected"),
        ev_completed("resp-2"),
    ]);
    let second_mock = responses::mount_sse_once(&server, second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "please update the active plan with a bad id".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    wait_for_event(&codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let req = second_mock.single_request();
    let (output_text, _success_flag) = call_output(&req, call_id);
    assert!(output_text.contains("failed to update active thread plan row missing-row"));

    let active_plan = db
        .get_active_thread_plan(session_configured.session_id.to_string().as_str())
        .await?
        .expect("active plan should remain stored");
    assert_eq!(active_plan.items[0].status, ThreadPlanItemStatus::Pending);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_plan_tool_rejects_malformed_payload() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let mut builder = test_codex();
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let call_id = "plan-tool-invalid";
    let invalid_args = json!({
        "explanation": "Missing plan data"
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "update_plan", &invalid_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once(&server, first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "malformed plan payload"),
        ev_completed("resp-2"),
    ]);
    let second_mock = responses::mount_sse_once(&server, second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "please update the plan".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let mut saw_plan_update = false;
    wait_for_event(&codex, |event| match event {
        EventMsg::PlanUpdate(_) => {
            saw_plan_update = true;
            false
        }
        EventMsg::TurnComplete(_) => true,
        _ => false,
    })
    .await;

    assert!(
        !saw_plan_update,
        "did not expect PlanUpdate event for malformed payload"
    );

    let req = second_mock.single_request();
    let (output_text, success_flag) = call_output(&req, call_id);
    assert!(
        output_text.contains("failed to parse function arguments"),
        "expected parse error message in output text, got {output_text:?}"
    );
    if let Some(success_flag) = success_flag {
        assert!(
            !success_flag,
            "expected tool output to mark success=false for malformed payload"
        );
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ultra_work_execution_update_plan_rejects_pending_to_completed() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let mut builder = test_codex().with_home(home).with_config(|config| {
        let _ = config.features.enable(Feature::PlanWorkflow);
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let db = codex.state_db().expect("state db");
    db.replace_active_thread_plan(&ThreadPlanSnapshotCreateParams {
        id: "snapshot-1".to_string(),
        thread_id: session_configured.session_id.to_string(),
        source_turn_id: "turn-1".to_string(),
        source_item_id: "item-1".to_string(),
        raw_csv: "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-1,pending,Guard execute,codex-rs/core/src/execute_plan_guard.rs,gate execute updates,,,,
plan-2,pending,Wire handler,codex-rs/core/src/tools/handlers/plan.rs,reject invalid execute mutations,,,plan-1,
"
        .to_string(),
    })
    .await?;

    let call_id = "execute-plan-bad-transition";
    let plan_args = json!({
        "explanation": "skip start",
        "plan": [
            {"id": "plan-1", "step": "Guard execute", "status": "completed"}
        ],
    })
    .to_string();

    responses::mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "update_plan", &plan_args),
            ev_completed("resp-1"),
        ]),
    )
    .await;
    let second_mock = responses::mount_sse_once(
        &server,
        sse(vec![
            ev_assistant_message("msg-1", "done"),
            ev_completed("resp-2"),
        ]),
    )
    .await;

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "execute the plan".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_configured.model.clone(),
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(CollaborationMode {
                mode: ModeKind::UltraWork,
                plan_phase: Some(PlanModePhase::Executing),
                settings: Settings {
                    model: session_configured.model.clone(),
                    reasoning_effort: None,
                    developer_instructions: None,
                },
            }),
            personality: None,
        })
        .await?;

    wait_for_event(&codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    let req = second_mock.single_request();
    let (output, _success_flag) = call_output(&req, call_id);
    assert!(output.contains("pending rows must first transition to in_progress"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ultra_work_execution_update_plan_allows_current_row_to_start() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let mut builder = test_codex().with_home(home).with_config(|config| {
        let _ = config.features.enable(Feature::PlanWorkflow);
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let db = codex.state_db().expect("state db");
    db.replace_active_thread_plan(&ThreadPlanSnapshotCreateParams {
        id: "snapshot-1".to_string(),
        thread_id: session_configured.session_id.to_string(),
        source_turn_id: "turn-1".to_string(),
        source_item_id: "item-1".to_string(),
        raw_csv: "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-1,pending,Guard execute,codex-rs/core/src/execute_plan_guard.rs,gate execute updates,,,,
plan-2,pending,Wire handler,codex-rs/core/src/tools/handlers/plan.rs,reject invalid execute mutations,,,plan-1,
"
        .to_string(),
    })
    .await?;

    let call_id = "execute-plan-start-transition";
    let plan_args = json!({
        "explanation": "start current row",
        "plan": [
            {"id": "plan-1", "step": "Guard execute", "status": "in_progress"}
        ],
    })
    .to_string();

    responses::mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "update_plan", &plan_args),
            ev_completed("resp-1"),
        ]),
    )
    .await;
    responses::mount_sse_once(
        &server,
        sse(vec![
            ev_assistant_message("msg-1", "done"),
            ev_completed("resp-2"),
        ]),
    )
    .await;

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "execute the plan".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_configured.model.clone(),
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(CollaborationMode {
                mode: ModeKind::UltraWork,
                plan_phase: Some(PlanModePhase::Executing),
                settings: Settings {
                    model: session_configured.model.clone(),
                    reasoning_effort: None,
                    developer_instructions: None,
                },
            }),
            personality: None,
        })
        .await?;

    wait_for_event(&codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    let plan = db
        .get_active_thread_plan(session_configured.session_id.to_string().as_str())
        .await?
        .expect("active plan should exist");
    assert_eq!(plan.items[0].status, ThreadPlanItemStatus::InProgress);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_patch_tool_executes_and_emits_patch_events() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let mut builder = test_codex().with_config(|config| {
        config
            .features
            .enable(Feature::ApplyPatchFreeform)
            .expect("test config should allow feature update");
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let file_name = "notes.txt";
    let file_path = cwd.path().join(file_name);
    let call_id = "apply-patch-call";
    let patch_content = format!(
        r#"*** Begin Patch
*** Add File: {file_name}
+Tool harness apply patch
*** End Patch"#
    );

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_apply_patch_function_call(call_id, &patch_content),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once(&server, first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "patch complete"),
        ev_completed("resp-2"),
    ]);
    let second_mock = responses::mount_sse_once(&server, second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "please apply a patch".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let mut saw_patch_begin = false;
    let mut patch_end_success = None;
    wait_for_event(&codex, |event| match event {
        EventMsg::PatchApplyBegin(begin) => {
            saw_patch_begin = true;
            assert_eq!(begin.call_id, call_id);
            false
        }
        EventMsg::PatchApplyEnd(end) => {
            assert_eq!(end.call_id, call_id);
            patch_end_success = Some(end.success);
            false
        }
        EventMsg::TurnComplete(_) => true,
        _ => false,
    })
    .await;

    assert!(saw_patch_begin, "expected PatchApplyBegin event");
    let patch_end_success =
        patch_end_success.expect("expected PatchApplyEnd event to capture success flag");
    assert!(patch_end_success);

    let req = second_mock.single_request();
    let (output_text, _success_flag) = call_output(&req, call_id);

    let expected_pattern = format!(
        r"(?s)^Exit code: 0
Wall time: [0-9]+(?:\.[0-9]+)? seconds
Output:
Success. Updated the following files:
A {file_name}
?$"
    );
    assert_regex_match(&expected_pattern, &output_text);

    let updated_contents = fs::read_to_string(file_path)?;
    assert_eq!(
        updated_contents, "Tool harness apply patch\n",
        "expected updated file content"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_patch_reports_parse_diagnostics() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let mut builder = test_codex().with_config(|config| {
        config
            .features
            .enable(Feature::ApplyPatchFreeform)
            .expect("test config should allow feature update");
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let call_id = "apply-patch-parse-error";
    let patch_content = r"*** Begin Patch
*** Update File: broken.txt
*** End Patch";

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_apply_patch_function_call(call_id, patch_content),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once(&server, first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "failed"),
        ev_completed("resp-2"),
    ]);
    let second_mock = responses::mount_sse_once(&server, second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![UserInput::Text {
                text: "please apply a patch".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    wait_for_event(&codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let req = second_mock.single_request();
    let (output_text, success_flag) = call_output(&req, call_id);

    assert!(
        output_text.contains("apply_patch verification failed"),
        "expected apply_patch verification failure message, got {output_text:?}"
    );
    assert!(
        output_text.contains("invalid hunk"),
        "expected parse diagnostics in output text, got {output_text:?}"
    );

    if let Some(success_flag) = success_flag {
        assert!(
            !success_flag,
            "expected tool output to mark success=false for parse failures"
        );
    }

    Ok(())
}
