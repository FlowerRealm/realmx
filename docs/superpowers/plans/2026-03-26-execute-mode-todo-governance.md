# Execute Mode Todo Governance Implementation Plan

> Historical note: this implementation plan predates the `Ultra Work` split. `Auto Plan` and `Execute` are no longer public modes; the current public modes are `Default`, `Plan`, and `Ultra Work`.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make accepted `tasks.csv` plans govern `Execute` mode by having the server choose the current executable row, inject only the plan paths plus that row into execute-time instructions, and strictly gate `update_plan` to legal single-row status transitions.

**Architecture:** Keep plan production and review in `Plan Mode`, then enforce plan-following at execute time with a dedicated `execute_plan_guard` module in `codex-core`. The guard computes the current executable row from the active thread plan, builds compact execute-mode instructions, and validates `update_plan` so the model can only advance the server-selected row without replacing or mutating the plan structure.

**Tech Stack:** Rust, `codex-core`, `codex-state`, collaboration-mode presets, thread plan persistence, `core_test_support` integration tests, `pretty_assertions`

---

### Task 1: Wire Execute Mode as a Real Collaboration Preset

**Files:**
- Modify: `codex-rs/core/src/models_manager/collaboration_mode_presets.rs`
- Modify: `codex-rs/core/src/models_manager/collaboration_mode_presets_tests.rs`
- Modify: `codex-rs/core/templates/collaboration_mode/execute.md`
- Test: `codex-rs/core/src/models_manager/collaboration_mode_presets_tests.rs`

- [ ] **Step 1: Write the failing preset tests**

Add execute-preset assertions to `codex-rs/core/src/models_manager/collaboration_mode_presets_tests.rs` so the current code fails before implementation:

```rust
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

    assert!(instructions.contains("accepted plans are absolute truth"));
    assert!(instructions.contains("only execute the server-selected row"));
    assert!(instructions.contains("do not review the plan"));
    assert!(instructions.contains("record plan-external work only in explanations"));
}

#[test]
fn builtin_presets_include_execute_for_internal_use() {
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
```

- [ ] **Step 2: Run the preset test to verify it fails**

Run:

```bash
cargo test -p codex-core execute_preset_uses_mode_display_name
```

Expected: FAIL because `execute_preset(...)` does not exist yet and the builtin preset list does not yet include `ModeKind::Execute`.

- [ ] **Step 3: Implement the execute preset and tighten the execute template**

Update `codex-rs/core/src/models_manager/collaboration_mode_presets.rs` to load the execute template and add an execute preset:

```rust
const COLLABORATION_MODE_EXECUTE: &str =
    include_str!("../../templates/collaboration_mode/execute.md");

pub fn builtin_collaboration_mode_presets(
    collaboration_modes_config: CollaborationModesConfig,
) -> Vec<CollaborationModeMask> {
    vec![
        plan_preset(collaboration_modes_config),
        auto_plan_preset(collaboration_modes_config),
        default_preset(collaboration_modes_config),
        execute_preset(collaboration_modes_config),
    ]
}

fn execute_preset(_collaboration_modes_config: CollaborationModesConfig) -> CollaborationModeMask {
    CollaborationModeMask {
        name: ModeKind::Execute.display_name().to_string(),
        mode: Some(ModeKind::Execute),
        model: None,
        reasoning_effort: None,
        developer_instructions: Some(Some(COLLABORATION_MODE_EXECUTE.to_string())),
    }
}
```

Replace the body of `codex-rs/core/templates/collaboration_mode/execute.md` with execute-governance rules that match the approved design:

```md
# Collaboration Style: Execute

You execute independently. If the current thread has an accepted active plan, that plan is absolute truth during Execute mode.

## Plan governance

- Do not review the plan in Execute mode.
- If the server provides plan workspace paths, read the provided `tasks.csv` path before acting.
- If the server provides a current executable row, only execute that row.
- Do not replace, append to, or repair the plan in Execute mode.
- Record plan-external work only in `update_plan.explanation` and the final response.

## Progress updates

- When you start the current row, update it to `in_progress`.
- When you finish the current row, update it to `completed`.
- Do not update any other row.
```

- [ ] **Step 4: Run the preset tests to verify they pass**

Run:

```bash
cargo test -p codex-core execute_preset_uses_mode_display_name
cargo test -p codex-core execute_preset_includes_execute_instructions
cargo test -p codex-core builtin_presets_include_execute_for_internal_use
```

Expected: PASS for all three tests.

- [ ] **Step 5: Commit**

```bash
git add codex-rs/core/src/models_manager/collaboration_mode_presets.rs codex-rs/core/src/models_manager/collaboration_mode_presets_tests.rs codex-rs/core/templates/collaboration_mode/execute.md
git commit -m "feat: add execute collaboration preset"
```

### Task 2: Add the Execute Plan Guard Module

**Files:**
- Create: `codex-rs/core/src/execute_plan_guard.rs`
- Modify: `codex-rs/core/src/lib.rs`
- Test: `codex-rs/core/src/execute_plan_guard.rs`

- [ ] **Step 1: Write the guard module tests first**

Create unit tests inside `codex-rs/core/src/execute_plan_guard.rs` that define the target-selection and transition-validation contract:

```rust
#[cfg(test)]
mod tests {
    use super::resolve_current_executable_target;
    use super::validate_execute_mode_plan_update;
    use codex_protocol::plan_tool::PlanItemArg;
    use codex_protocol::plan_tool::StepStatus;
    use codex_protocol::plan_tool::UpdatePlanArgs;
    use codex_state::ThreadPlanItem;
    use codex_state::ThreadPlanItemStatus;
    use pretty_assertions::assert_eq;

    #[test]
    fn selects_existing_in_progress_row() {
        let rows = sample_rows(&[
            ("plan-01", ThreadPlanItemStatus::Completed, vec![]),
            ("plan-02", ThreadPlanItemStatus::InProgress, vec!["plan-01"]),
            ("plan-03", ThreadPlanItemStatus::Pending, vec!["plan-02"]),
        ]);

        let target = resolve_current_executable_target(rows.as_slice())
            .expect("target selection should succeed")
            .expect("target should exist");
        assert_eq!(target.row_id, "plan-02");
    }

    #[test]
    fn selects_first_dependency_ready_pending_row() {
        let rows = sample_rows(&[
            ("plan-01", ThreadPlanItemStatus::Completed, vec![]),
            ("plan-02", ThreadPlanItemStatus::Pending, vec!["plan-01"]),
            ("plan-03", ThreadPlanItemStatus::Pending, vec!["plan-02"]),
        ]);

        let target = resolve_current_executable_target(rows.as_slice())
            .expect("target selection should succeed")
            .expect("target should exist");
        assert_eq!(target.row_id, "plan-02");
    }

    #[test]
    fn rejects_execute_mode_multi_row_updates() {
        let rows = sample_rows(&[
            ("plan-01", ThreadPlanItemStatus::Pending, vec![]),
            ("plan-02", ThreadPlanItemStatus::Pending, vec!["plan-01"]),
        ]);
        let args = UpdatePlanArgs {
            explanation: Some("bad".to_string()),
            plan: vec![
                PlanItemArg {
                    id: Some("plan-01".to_string()),
                    step: "one".to_string(),
                    status: StepStatus::InProgress,
                    path: None,
                    details: None,
                    inputs: None,
                    outputs: None,
                    depends_on: None,
                    acceptance: None,
                },
                PlanItemArg {
                    id: Some("plan-02".to_string()),
                    step: "two".to_string(),
                    status: StepStatus::Pending,
                    path: None,
                    details: None,
                    inputs: None,
                    outputs: None,
                    depends_on: None,
                    acceptance: None,
                },
            ],
        };

        let err = validate_execute_mode_plan_update(rows.as_slice(), &args)
            .expect_err("multi-row execute update should fail");
        assert_eq!(
            err.to_string(),
            "Execute mode may only update the server-selected current plan row"
        );
    }
}
```

- [ ] **Step 2: Run the guard tests to verify they fail**

Run:

```bash
cargo test -p codex-core selects_existing_in_progress_row
```

Expected: FAIL because `execute_plan_guard.rs` and its exported functions do not exist yet.

- [ ] **Step 3: Implement the new guard module with minimal production code**

Create `codex-rs/core/src/execute_plan_guard.rs` with focused data types and functions:

```rust
use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_state::ThreadPlanItem;
use codex_state::ThreadPlanItemStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutePlanTarget {
    pub(crate) row_id: String,
    pub(crate) step: String,
    pub(crate) path: String,
    pub(crate) details: String,
    pub(crate) acceptance: Option<String>,
}

pub(crate) fn resolve_current_executable_target(
    items: &[ThreadPlanItem],
) -> anyhow::Result<Option<ExecutePlanTarget>> {
    if let Some(item) = items
        .iter()
        .find(|item| matches!(item.status, ThreadPlanItemStatus::InProgress))
    {
        return Ok(Some(target_from_item(item)));
    }

    for item in items {
        if !matches!(item.status, ThreadPlanItemStatus::Pending) {
            continue;
        }
        let ready = item.depends_on.iter().all(|dependency| {
            items.iter().any(|candidate| {
                candidate.row_id == *dependency
                    && matches!(candidate.status, ThreadPlanItemStatus::Completed)
            })
        });
        if ready {
            return Ok(Some(target_from_item(item)));
        }
    }

    Ok(None)
}

pub(crate) fn validate_execute_mode_plan_update(
    items: &[ThreadPlanItem],
    args: &UpdatePlanArgs,
) -> anyhow::Result<()> {
    if args.plan.len() != 1 {
        anyhow::bail!("Execute mode may only update the server-selected current plan row");
    }

    let target = resolve_current_executable_target(items)?
        .ok_or_else(|| anyhow::anyhow!("Execute mode has no current executable plan row"))?;
    let update = &args.plan[0];
    let row_id = update
        .id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Execute mode updates must include the current plan row id"))?;
    if row_id != target.row_id {
        anyhow::bail!("Execute mode may only update the server-selected current plan row");
    }

    let current = items
        .iter()
        .find(|item| item.row_id == row_id)
        .ok_or_else(|| anyhow::anyhow!("active thread plan row not found: {row_id}"))?;
    reject_metadata_mutation(update, current)?;
    validate_status_transition(current.status, update.status.clone())?;
    Ok(())
}
```

Expose the module from `codex-rs/core/src/lib.rs`:

```rust
mod execute_plan_guard;
```

- [ ] **Step 4: Run the guard tests to verify they pass**

Run:

```bash
cargo test -p codex-core selects_existing_in_progress_row
cargo test -p codex-core selects_first_dependency_ready_pending_row
cargo test -p codex-core rejects_execute_mode_multi_row_updates
```

Expected: PASS for the new guard tests.

- [ ] **Step 5: Commit**

```bash
git add codex-rs/core/src/execute_plan_guard.rs codex-rs/core/src/lib.rs
git commit -m "feat: add execute plan guard"
```

### Task 3: Inject Execute-Time Plan Paths and the Server-Selected Row

**Files:**
- Modify: `codex-rs/core/src/codex.rs`
- Modify: `codex-rs/core/tests/suite/collaboration_instructions.rs`
- Test: `codex-rs/core/tests/suite/collaboration_instructions.rs`

- [ ] **Step 1: Write the failing collaboration-instructions test**

Add a new integration test to `codex-rs/core/tests/suite/collaboration_instructions.rs` that seeds an active thread plan and verifies the request input for `ModeKind::Execute` includes only the plan paths plus the current row:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_mode_injects_plan_paths_and_current_row_only() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let req = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let home = Arc::new(tempfile::TempDir::new()?);
    let test = test_codex().with_home(home).build(&server).await?;
    let db = test.codex.state_db().expect("state db");
    db.replace_active_thread_plan(&ThreadPlanSnapshotCreateParams {
        id: "snapshot-1".to_string(),
        thread_id: test.session_configured.session_id.to_string(),
        source_turn_id: "turn-1".to_string(),
        source_item_id: "item-1".to_string(),
        raw_csv: "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,pending,Guard execute,codex-rs/core/src/execute_plan_guard.rs,add execute plan guard,,, ,guard selects current row
plan-02,pending,Wire handler,codex-rs/core/src/tools/handlers/plan.rs,gate update_plan,,,plan-01,handler rejects invalid updates
"
        .replace(",,, ,", ",,,,"),
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
                mode: ModeKind::Execute,
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
    assert!(!joined.contains("plan-02,pending,Wire handler"));
}
```

- [ ] **Step 2: Run the collaboration-instructions test to verify it fails**

Run:

```bash
cargo test -p codex-core execute_mode_injects_plan_paths_and_current_row_only
```

Expected: FAIL because `codex.rs` does not yet inject execute-time plan paths or the server-selected row.

- [ ] **Step 3: Implement the execute-time developer-instruction injection**

In `codex-rs/core/src/codex.rs`, append a governed-execute developer section while building request inputs:

```rust
if collaboration_mode.mode == ModeKind::Execute
    && let Some(state_db) = self.state_db()
{
    let thread_id = self.conversation_id.to_string();
    if let Some(active_plan) = state_db.get_active_thread_plan(thread_id.as_str()).await? {
        let codex_home = self.codex_home().await;
        let workspace = PlanWorkspace::new(
            codex_home.as_path(),
            turn_context.cwd.as_path(),
            thread_id.as_str(),
        );
        let execute_instructions = crate::execute_plan_guard::build_execute_plan_guard_instructions(
            workspace.root(),
            active_plan.items.as_slice(),
        )?;
        developer_sections.push(execute_instructions);
    }
}
```

Implement `build_execute_plan_guard_instructions(...)` in `codex-rs/core/src/execute_plan_guard.rs` to emit compact text:

```rust
pub(crate) fn build_execute_plan_guard_instructions(
    workspace_root: &Path,
    items: &[ThreadPlanItem],
) -> anyhow::Result<String> {
    let tasks_csv = workspace_root.join("tasks.csv");
    let tasks_md = workspace_root.join("tasks.md");
    let Some(target) = resolve_current_executable_target(items)? else {
        return Ok(format!(
            "Execute-mode plan workspace: `{}`.\nRead `{}` before acting.\nNo executable plan row is currently available.",
            workspace_root.display(),
            tasks_csv.display(),
        ));
    };
    Ok(format!(
        "Execute-mode plan workspace: `{}`.\nRead `{}` before acting. Derived plan text lives at `{}`.\nThe active plan is absolute truth in Execute mode.\nOnly execute the server-selected row below.\nCurrent executable row:\n- id: `{}`\n- step: {}\n- path: `{}`\n- details: {}\n- acceptance: {}\nDo not review or modify the plan. Record plan-external work only in `update_plan.explanation` and the final response.",
        workspace_root.display(),
        tasks_csv.display(),
        tasks_md.display(),
        target.row_id,
        target.step,
        target.path,
        target.details,
        target.acceptance.as_deref().unwrap_or(""),
    ))
}
```

- [ ] **Step 4: Run the collaboration-instructions test to verify it passes**

Run:

```bash
cargo test -p codex-core execute_mode_injects_plan_paths_and_current_row_only
```

Expected: PASS, with the request input containing the plan paths and `plan-01` but not a full dump of the whole plan.

- [ ] **Step 5: Commit**

```bash
git add codex-rs/core/src/codex.rs codex-rs/core/src/execute_plan_guard.rs codex-rs/core/tests/suite/collaboration_instructions.rs
git commit -m "feat: inject execute plan guard instructions"
```

### Task 4: Gate `update_plan` in Execute Mode

**Files:**
- Modify: `codex-rs/core/src/tools/handlers/plan.rs`
- Modify: `codex-rs/core/tests/suite/tool_harness.rs`
- Test: `codex-rs/core/tests/suite/tool_harness.rs`

- [ ] **Step 1: Write the failing execute-mode handler tests**

Add integration tests to `codex-rs/core/tests/suite/tool_harness.rs` that seed an active plan, run the model in `ModeKind::Execute`, and assert that invalid updates are rejected while legal single-row transitions succeed:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_mode_update_plan_rejects_pending_to_completed() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let mut builder = test_codex().with_home(home).with_config(|config| {
        let _ = config.features.enable(Feature::PlanProgressCsv);
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
        sse(vec![ev_assistant_message("msg-1", "done"), ev_completed("resp-2")]),
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
                mode: ModeKind::Execute,
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
    let output = call_output(&req, call_id);
    assert!(output.contains("pending rows must first transition to in_progress"));
    Ok(())
}
```

Add a second test that allows a legal `pending -> in_progress` update for the current row and checks that the active plan persisted that transition:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_mode_update_plan_allows_current_row_to_start() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let mut builder = test_codex().with_home(home).with_config(|config| {
        let _ = config.features.enable(Feature::PlanProgressCsv);
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
        sse(vec![ev_assistant_message("msg-1", "done"), ev_completed("resp-2")]),
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
                mode: ModeKind::Execute,
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
```

- [ ] **Step 2: Run the execute-mode handler tests to verify they fail**

Run:

```bash
cargo test -p codex-core execute_mode_update_plan_rejects_pending_to_completed
```

Expected: FAIL because `update_plan` does not yet distinguish governed `Execute` mode from general active-plan updates.

- [ ] **Step 3: Implement execute-mode gating in the plan handler**

Add a guarded branch near the start of `handle_update_plan(...)` in `codex-rs/core/src/tools/handlers/plan.rs`:

```rust
pub(crate) async fn handle_update_plan(
    session: &Session,
    turn_context: &TurnContext,
    arguments: String,
    call_id: String,
) -> Result<String, FunctionCallError> {
    if turn_context.collaboration_mode.mode.is_plan_output_mode() {
        return Err(FunctionCallError::RespondToModel(
            "update_plan is a TODO/checklist tool and is not allowed in Plan output modes"
                .to_string(),
        ));
    }

    let mut args = parse_update_plan_arguments(&arguments)?;
    if try_handle_execute_mode_active_plan_update(session, turn_context, &mut args).await? {
        session
            .send_event(turn_context, EventMsg::PlanUpdate(args))
            .await;
        return Ok("Plan updated".to_string());
    }

    if try_sync_active_thread_plan(session, turn_context, &mut args, call_id.as_str()).await? {
        session
            .send_event(turn_context, EventMsg::PlanUpdate(args))
            .await;
        return Ok("Plan updated".to_string());
    }
    session
        .send_event(turn_context, EventMsg::PlanUpdate(args))
        .await;
    Ok("Plan updated".to_string())
}
```

Implement `try_handle_execute_mode_active_plan_update(...)` so it:

```rust
async fn try_handle_execute_mode_active_plan_update(
    session: &Session,
    turn_context: &TurnContext,
    args: &mut UpdatePlanArgs,
) -> Result<bool, FunctionCallError> {
    if turn_context.collaboration_mode.mode != ModeKind::Execute {
        return Ok(false);
    }
    let Some(state_db) = session.state_db() else {
        return Ok(false);
    };
    let thread_id = session.conversation_id.to_string();
    let Some(active_plan) = state_db
        .get_active_thread_plan(thread_id.as_str())
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!(
            "failed to load active thread plan: {err}"
        )))?
    else {
        return Ok(false);
    };

    crate::execute_plan_guard::validate_execute_mode_plan_update(
        active_plan.items.as_slice(),
        args,
    )
    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;

    update_existing_active_thread_plan(
        session,
        turn_context,
        args,
        thread_id.as_str(),
        active_plan,
    )
    .await?;
    Ok(true)
}
```

Also add stricter validation inside `execute_plan_guard.rs` so metadata changes are rejected:

```rust
fn reject_metadata_mutation(update: &PlanItemArg, current: &ThreadPlanItem) -> anyhow::Result<()> {
    if update.step != current.step {
        anyhow::bail!("Execute mode may not change the current plan row step");
    }
    if update.path.as_deref().is_some_and(|path| path != current.path) {
        anyhow::bail!("Execute mode may not change the current plan row path");
    }
    if update.details.as_deref().is_some_and(|details| details != current.details) {
        anyhow::bail!("Execute mode may not change the current plan row details");
    }
    Ok(())
}

fn validate_status_transition(
    current: ThreadPlanItemStatus,
    next: StepStatus,
) -> anyhow::Result<()> {
    match (current, next) {
        (ThreadPlanItemStatus::Pending, StepStatus::InProgress)
        | (ThreadPlanItemStatus::InProgress, StepStatus::Completed)
        | (ThreadPlanItemStatus::InProgress, StepStatus::InProgress)
        | (ThreadPlanItemStatus::Completed, StepStatus::Completed) => Ok(()),
        (ThreadPlanItemStatus::Pending, StepStatus::Completed) => {
            anyhow::bail!("Execute mode pending rows must first transition to in_progress")
        }
        _ => anyhow::bail!("Execute mode received an invalid plan status transition"),
    }
}
```

- [ ] **Step 4: Run the execute-mode handler tests to verify they pass**

Run:

```bash
cargo test -p codex-core execute_mode_update_plan_rejects_pending_to_completed
cargo test -p codex-core execute_mode_update_plan_allows_current_row_to_start
```

Expected: PASS, with the bad transition rejected and the valid start transition persisted.

- [ ] **Step 5: Commit**

```bash
git add codex-rs/core/src/tools/handlers/plan.rs codex-rs/core/src/execute_plan_guard.rs codex-rs/core/tests/suite/tool_harness.rs
git commit -m "feat: gate execute mode plan updates"
```

### Task 5: Run Focused Verification and Finish the Branch

**Files:**
- Modify: `codex-rs/core/src/models_manager/collaboration_mode_presets.rs`
- Modify: `codex-rs/core/templates/collaboration_mode/execute.md`
- Modify: `codex-rs/core/src/execute_plan_guard.rs`
- Modify: `codex-rs/core/src/codex.rs`
- Modify: `codex-rs/core/src/tools/handlers/plan.rs`
- Modify: `codex-rs/core/src/models_manager/collaboration_mode_presets_tests.rs`
- Modify: `codex-rs/core/tests/suite/collaboration_instructions.rs`
- Modify: `codex-rs/core/tests/suite/tool_harness.rs`
- Test: `codex-rs/core`

- [ ] **Step 1: Run the focused codex-core test targets**

Run:

```bash
cargo test -p codex-core execute_preset_uses_mode_display_name
cargo test -p codex-core execute_preset_includes_execute_instructions
cargo test -p codex-core builtin_presets_include_execute_for_internal_use
cargo test -p codex-core execute_mode_injects_plan_paths_and_current_row_only
cargo test -p codex-core execute_mode_update_plan_rejects_pending_to_completed
cargo test -p codex-core execute_mode_update_plan_allows_current_row_to_start
```

Expected: PASS for all focused tests.

- [ ] **Step 2: Run the crate test suite**

Run:

```bash
cargo test -p codex-core
```

Expected: PASS with the full `codex-core` crate test suite green.

- [ ] **Step 3: Run formatting**

Run:

```bash
just fmt
```

Expected: PASS with the workspace formatter making no unexpected semantic changes.

- [ ] **Step 4: Run scoped lint fixes**

Run:

```bash
just fix -p core
```

Expected: PASS with any clippy-driven rewrites applied in `codex-core`.

- [ ] **Step 5: Commit**

```bash
git add codex-rs/core/src/models_manager/collaboration_mode_presets.rs codex-rs/core/templates/collaboration_mode/execute.md codex-rs/core/src/execute_plan_guard.rs codex-rs/core/src/codex.rs codex-rs/core/src/tools/handlers/plan.rs codex-rs/core/src/models_manager/collaboration_mode_presets_tests.rs codex-rs/core/tests/suite/collaboration_instructions.rs codex-rs/core/tests/suite/tool_harness.rs
git commit -m "feat: govern execute mode with active todo plan"
```
