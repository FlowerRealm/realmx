use crate::agent::status::is_final;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::error::CodexErr;
use crate::features::Feature;
use crate::function_tool::FunctionCallError;
use crate::plan_csv::update_plan_from_thread_plan_items;
use crate::plan_workspace::PlanWorkspace;
use crate::tools::handlers::multi_agents::build_agent_spawn_config;
use crate::tools::handlers::multi_agents::build_wait_agent_statuses;
use codex_protocol::ThreadId;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::CollabAgentRef;
use codex_protocol::protocol::CollabAgentSpawnBeginEvent;
use codex_protocol::protocol::CollabAgentSpawnEndEvent;
use codex_protocol::protocol::CollabAgentStatusEntry;
use codex_protocol::protocol::CollabCloseBeginEvent;
use codex_protocol::protocol::CollabCloseEndEvent;
use codex_protocol::protocol::CollabWaitingBeginEvent;
use codex_protocol::protocol::CollabWaitingEndEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::user_input::UserInput;
use codex_state::ActiveThreadPlan;
use codex_state::ThreadPlanItem;
use codex_state::ThreadPlanItemCreateParams;
use codex_state::ThreadPlanItemStatus;
use codex_state::render_thread_plan_csv;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecuteReadyRow {
    pub(crate) row_id: String,
    pub(crate) step: String,
    pub(crate) path: String,
    pub(crate) details: String,
    pub(crate) depends_on: Vec<String>,
    pub(crate) acceptance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveWorkerAssignment {
    pub(crate) row_id: String,
    pub(crate) thread_id: ThreadId,
    pub(crate) agent_nickname: Option<String>,
    pub(crate) agent_role: Option<String>,
}

#[derive(Debug, Default)]
struct SpawnReadyWorkersResult {
    assignments: Vec<ActiveWorkerAssignment>,
    failures: Vec<DispatchFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DispatchFailure {
    pub(crate) row_id: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ExecuteDispatchSummary {
    pub(crate) completed_rows: Vec<String>,
    pub(crate) failed_rows: Vec<DispatchFailure>,
}

pub(crate) fn resolve_dependency_ready_rows(items: &[ThreadPlanItem]) -> Vec<ExecuteReadyRow> {
    let completed = items
        .iter()
        .filter(|item| matches!(item.status, ThreadPlanItemStatus::Completed))
        .map(|item| item.row_id.as_str())
        .collect::<HashSet<_>>();
    let in_progress = items
        .iter()
        .filter(|item| matches!(item.status, ThreadPlanItemStatus::InProgress))
        .map(|item| item.row_id.as_str())
        .collect::<HashSet<_>>();

    items
        .iter()
        .filter(|item| matches!(item.status, ThreadPlanItemStatus::Pending))
        .filter(|item| {
            item.depends_on
                .iter()
                .all(|dependency| completed.contains(dependency.as_str()))
        })
        .filter(|item| !in_progress.contains(item.row_id.as_str()))
        .map(|item| ExecuteReadyRow {
            row_id: item.row_id.clone(),
            step: item.step.clone(),
            path: item.path.clone(),
            details: item.details.clone(),
            depends_on: item.depends_on.clone(),
            acceptance: item.acceptance.clone(),
        })
        .collect()
}

pub(crate) fn conflicting_active_rows(items: &[ThreadPlanItem], row_id: &str) -> Vec<String> {
    let Some(target) = items.iter().find(|item| item.row_id == row_id) else {
        return Vec::new();
    };

    items
        .iter()
        .filter(|item| item.row_id != row_id)
        .filter(|item| matches!(item.status, ThreadPlanItemStatus::InProgress))
        .filter(|item| item.path == target.path)
        .map(|item| item.row_id.clone())
        .collect()
}

pub(crate) async fn execute_active_plan_with_subagents(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
) -> Result<ExecuteDispatchSummary, FunctionCallError> {
    if !session.enabled(Feature::PlanWorkflow) {
        return Err(FunctionCallError::RespondToModel(
            "execute active plan dispatch feature is disabled".to_string(),
        ));
    }
    let state_db = session
        .state_db()
        .ok_or_else(|| FunctionCallError::RespondToModel("state db unavailable".to_string()))?;
    let thread_id = session.conversation_id.to_string();
    let Some(mut active_plan) = state_db
        .get_active_thread_plan(thread_id.as_str())
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to load active thread plan: {err}"))
        })?
    else {
        return Err(FunctionCallError::RespondToModel(
            "no active plan exists for this thread".to_string(),
        ));
    };

    let mut completed_rows = Vec::new();
    let mut failed_rows = Vec::new();
    let mut stop_dispatch = false;

    loop {
        let ready_rows = resolve_dependency_ready_rows(active_plan.items.as_slice());
        if ready_rows.is_empty() {
            break;
        }

        let updates = ready_rows
            .iter()
            .map(|row| (row.row_id.clone(), ThreadPlanItemStatus::InProgress))
            .collect::<Vec<_>>();
        let refreshed = state_db
            .update_active_thread_plan_item_statuses(thread_id.as_str(), updates.as_slice())
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to mark active plan rows in_progress: {err}"
                ))
            })?
            .ok_or_else(|| {
                FunctionCallError::RespondToModel("active plan disappeared during dispatch".into())
            })?;
        active_plan = refreshed;
        emit_plan_update(session, turn, active_plan.items.as_slice(), None).await;

        let SpawnReadyWorkersResult {
            assignments,
            failures: spawn_failures,
        } = spawn_ready_workers(session, turn, active_plan.items.as_slice(), &ready_rows).await?;
        let statuses = if assignments.is_empty() {
            HashMap::new()
        } else {
            let (statuses, _) = wait_for_assignments(session, turn, &assignments).await;
            close_assignments(session, turn, &assignments, &statuses).await;
            statuses
        };

        let mut batch_updates = Vec::new();
        for failure in spawn_failures {
            batch_updates.push((failure.row_id.clone(), ThreadPlanItemStatus::Pending));
            failed_rows.push(failure);
            stop_dispatch = true;
        }
        for assignment in &assignments {
            match statuses.get(&assignment.thread_id) {
                Some(AgentStatus::Completed(_)) => {
                    completed_rows.push(assignment.row_id.clone());
                    batch_updates
                        .push((assignment.row_id.clone(), ThreadPlanItemStatus::Completed));
                }
                Some(status) => {
                    failed_rows.push(DispatchFailure {
                        row_id: assignment.row_id.clone(),
                        message: format!("worker ended with status {status:?}"),
                    });
                    batch_updates.push((assignment.row_id.clone(), ThreadPlanItemStatus::Pending));
                    stop_dispatch = true;
                }
                None => {
                    failed_rows.push(DispatchFailure {
                        row_id: assignment.row_id.clone(),
                        message: "worker status missing after wait".to_string(),
                    });
                    batch_updates.push((assignment.row_id.clone(), ThreadPlanItemStatus::Pending));
                    stop_dispatch = true;
                }
            }
        }

        if !batch_updates.is_empty() {
            let refreshed = state_db
                .update_active_thread_plan_item_statuses(
                    thread_id.as_str(),
                    batch_updates.as_slice(),
                )
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to persist dispatched active plan updates: {err}"
                    ))
                })?
                .ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "active plan disappeared while persisting dispatch updates".to_string(),
                    )
                })?;
            active_plan = refreshed;
            let explanation = if failed_rows.is_empty() {
                None
            } else {
                Some(format!(
                    "DAG dispatch paused after failures in rows: {}",
                    failed_rows
                        .iter()
                        .map(|failure| failure.row_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            };
            emit_plan_update(session, turn, active_plan.items.as_slice(), explanation).await;
        }

        if stop_dispatch {
            break;
        }
    }

    Ok(ExecuteDispatchSummary {
        completed_rows,
        failed_rows,
    })
}

async fn emit_plan_update(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    items: &[ThreadPlanItem],
    explanation: Option<String>,
) {
    session
        .send_event(
            turn,
            EventMsg::PlanUpdate(update_plan_from_thread_plan_items(items, explanation)),
        )
        .await;
}

async fn spawn_ready_workers(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    items: &[ThreadPlanItem],
    ready_rows: &[ExecuteReadyRow],
) -> Result<SpawnReadyWorkersResult, FunctionCallError> {
    let mut assignments = Vec::with_capacity(ready_rows.len());
    let mut failures = Vec::new();
    let config = build_agent_spawn_config(&session.get_base_instructions().await, turn.as_ref())?;

    for row in ready_rows {
        let call_id = Uuid::new_v4().to_string();
        let prompt = build_worker_prompt(items, row);
        session
            .send_event(
                turn,
                CollabAgentSpawnBeginEvent {
                    call_id: call_id.clone(),
                    sender_thread_id: session.conversation_id,
                    prompt: prompt.clone(),
                    model: String::new(),
                    reasoning_effort: turn.reasoning_effort.unwrap_or_default(),
                }
                .into(),
            )
            .await;

        let result = session
            .services
            .agent_control
            .spawn_agent(
                config.clone(),
                vec![UserInput::Text {
                    text: prompt.clone(),
                    text_elements: Vec::new(),
                }],
                Some(SessionSource::SubAgent(SubAgentSource::Other(format!(
                    "execute_plan_dispatch:{}:{}",
                    session.conversation_id, row.row_id
                )))),
            )
            .await;
        let (new_thread_id, status) = match &result {
            Ok(thread_id) => (
                Some(*thread_id),
                session.services.agent_control.get_status(*thread_id).await,
            ),
            Err(_) => (None, AgentStatus::NotFound),
        };
        let (agent_nickname, agent_role) = match new_thread_id {
            Some(thread_id) => session
                .services
                .agent_control
                .get_agent_nickname_and_role(thread_id)
                .await
                .unwrap_or((None, None)),
            None => (None, None),
        };
        session
            .send_event(
                turn,
                CollabAgentSpawnEndEvent {
                    call_id,
                    sender_thread_id: session.conversation_id,
                    new_thread_id,
                    new_agent_nickname: agent_nickname.clone(),
                    new_agent_role: agent_role.clone(),
                    prompt,
                    model: String::new(),
                    reasoning_effort: turn.reasoning_effort.unwrap_or_default(),
                    status,
                }
                .into(),
            )
            .await;

        let thread_id = match result {
            Ok(thread_id) => thread_id,
            Err(CodexErr::UnsupportedOperation(_)) => {
                failures.push(DispatchFailure {
                    row_id: row.row_id.clone(),
                    message: "collab manager unavailable".to_string(),
                });
                continue;
            }
            Err(other) => {
                failures.push(DispatchFailure {
                    row_id: row.row_id.clone(),
                    message: format!("failed to spawn worker: {other}"),
                });
                continue;
            }
        };
        assignments.push(ActiveWorkerAssignment {
            row_id: row.row_id.clone(),
            thread_id,
            agent_nickname,
            agent_role,
        });
    }

    Ok(SpawnReadyWorkersResult {
        assignments,
        failures,
    })
}

async fn wait_for_assignments(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    assignments: &[ActiveWorkerAssignment],
) -> (HashMap<ThreadId, AgentStatus>, Vec<CollabAgentStatusEntry>) {
    let receiver_thread_ids = assignments
        .iter()
        .map(|assignment| assignment.thread_id)
        .collect::<Vec<_>>();
    let receiver_agents = assignments
        .iter()
        .map(|assignment| CollabAgentRef {
            thread_id: assignment.thread_id,
            agent_nickname: assignment.agent_nickname.clone(),
            agent_role: assignment.agent_role.clone(),
        })
        .collect::<Vec<_>>();
    let call_id = Uuid::new_v4().to_string();
    session
        .send_event(
            turn,
            CollabWaitingBeginEvent {
                sender_thread_id: session.conversation_id,
                receiver_thread_ids,
                receiver_agents: receiver_agents.clone(),
                call_id: call_id.clone(),
            }
            .into(),
        )
        .await;

    let mut statuses = HashMap::with_capacity(assignments.len());
    for assignment in assignments {
        let status = wait_for_final_status(session, assignment.thread_id).await;
        statuses.insert(assignment.thread_id, status);
    }
    let agent_statuses = build_wait_agent_statuses(&statuses, &receiver_agents);
    session
        .send_event(
            turn,
            CollabWaitingEndEvent {
                sender_thread_id: session.conversation_id,
                call_id,
                agent_statuses: agent_statuses.clone(),
                statuses: statuses.clone(),
            }
            .into(),
        )
        .await;

    (statuses, agent_statuses)
}

async fn wait_for_final_status(session: &Arc<Session>, thread_id: ThreadId) -> AgentStatus {
    match session
        .services
        .agent_control
        .subscribe_status(thread_id)
        .await
    {
        Ok(mut rx) => loop {
            let status = rx.borrow().clone();
            if is_final(&status) {
                return status;
            }
            if rx.changed().await.is_err() {
                return session.services.agent_control.get_status(thread_id).await;
            }
        },
        Err(_) => session.services.agent_control.get_status(thread_id).await,
    }
}

async fn close_assignments(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    assignments: &[ActiveWorkerAssignment],
    statuses: &HashMap<ThreadId, AgentStatus>,
) {
    for assignment in assignments {
        let call_id = Uuid::new_v4().to_string();
        session
            .send_event(
                turn,
                CollabCloseBeginEvent {
                    call_id: call_id.clone(),
                    sender_thread_id: session.conversation_id,
                    receiver_thread_id: assignment.thread_id,
                }
                .into(),
            )
            .await;
        let shutdown_status = match session
            .services
            .agent_control
            .shutdown_agent(assignment.thread_id)
            .await
        {
            Ok(_) => statuses
                .get(&assignment.thread_id)
                .cloned()
                .unwrap_or(AgentStatus::Shutdown),
            Err(_) => statuses
                .get(&assignment.thread_id)
                .cloned()
                .unwrap_or(AgentStatus::NotFound),
        };
        session
            .send_event(
                turn,
                CollabCloseEndEvent {
                    call_id,
                    sender_thread_id: session.conversation_id,
                    receiver_thread_id: assignment.thread_id,
                    receiver_agent_nickname: assignment.agent_nickname.clone(),
                    receiver_agent_role: assignment.agent_role.clone(),
                    status: shutdown_status,
                }
                .into(),
            )
            .await;
    }
}

fn build_worker_prompt(items: &[ThreadPlanItem], row: &ExecuteReadyRow) -> String {
    let mut prompt = String::new();
    let conflicts = conflicting_active_rows(items, row.row_id.as_str());
    let full_plan_csv = render_thread_plan_csv(
        &items
            .iter()
            .map(|item| ThreadPlanItemCreateParams {
                row_id: item.row_id.clone(),
                row_index: item.row_index,
                status: item.status,
                step: item.step.clone(),
                path: item.path.clone(),
                details: item.details.clone(),
                inputs: item.inputs.clone(),
                outputs: item.outputs.clone(),
                depends_on: item.depends_on.clone(),
                acceptance: item.acceptance.clone(),
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|err| format!("failed to render active plan csv: {err}"));
    let _ = writeln!(
        prompt,
        "You are a worker subagent executing one accepted active plan row."
    );
    let _ = writeln!(prompt, "Own only this row:");
    let _ = writeln!(prompt, "- id: {}", row.row_id);
    let _ = writeln!(prompt, "- step: {}", row.step);
    let _ = writeln!(prompt, "- path: {}", row.path);
    if !row.details.is_empty() {
        let _ = writeln!(prompt, "- details: {}", row.details);
    }
    if !row.depends_on.is_empty() {
        let _ = writeln!(prompt, "- depends_on: {}", row.depends_on.join(", "));
    }
    if let Some(acceptance) = row.acceptance.as_deref() {
        let _ = writeln!(prompt, "- acceptance: {acceptance}");
    }
    let _ = writeln!(prompt);
    let _ = writeln!(prompt, "Full active tasks.csv:");
    let _ = writeln!(prompt, "{full_plan_csv}");
    if conflicts.is_empty() {
        let _ = writeln!(prompt, "Conflicting active rows on the same path: none");
    } else {
        let _ = writeln!(
            prompt,
            "Conflicting active rows on the same path: {}",
            conflicts.join(", ")
        );
    }
    let _ = writeln!(prompt);
    let _ = writeln!(prompt, "Rules:");
    let _ = writeln!(prompt, "- Read the full accepted plan before acting.");
    let _ = writeln!(
        prompt,
        "- You may edit code directly in the shared workspace."
    );
    let _ = writeln!(
        prompt,
        "- Do not update the plan or change ownership of any other row."
    );
    let _ = writeln!(
        prompt,
        "- If another active row touches the same path, coordinate by minimizing overlap and avoid reverting others."
    );
    let _ = writeln!(
        prompt,
        "- Finish by completing only the assigned implementation work."
    );
    prompt
}

pub(crate) fn build_execute_dispatch_guard_instructions(
    workspace: &PlanWorkspace,
    plan: &ActiveThreadPlan,
) -> anyhow::Result<String> {
    let tasks_csv = workspace.root().join("tasks.csv");
    let tasks_md = workspace.root().join("tasks.md");
    let ready_rows = resolve_dependency_ready_rows(plan.items.as_slice());
    let ready_preview = if ready_rows.is_empty() {
        "none".to_string()
    } else {
        ready_rows
            .iter()
            .map(|row| format!("{} ({})", row.row_id, row.path))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Ok(format!(
        "Execute-mode plan workspace: `{}`.\nRead `{}` before acting. Derived plan text lives at `{}`.\nThe accepted active plan is absolute truth in Execute mode.\nWhen active plan dispatch is enabled, use `execute_active_plan_with_subagents` to advance dependency-ready rows automatically instead of manually updating plan rows.\nCurrent dependency-ready rows: {}\nDo not review or modify the plan manually. Record plan-external work only in the final response.",
        workspace.root().display(),
        tasks_csv.display(),
        tasks_md.display(),
        ready_preview,
    ))
}

#[cfg(test)]
mod tests {
    use super::build_execute_dispatch_guard_instructions;
    use super::conflicting_active_rows;
    use super::resolve_dependency_ready_rows;
    use crate::plan_workspace::PlanWorkspace;
    use chrono::Utc;
    use codex_state::ActiveThreadPlan;
    use codex_state::ThreadPlanItem;
    use codex_state::ThreadPlanItemStatus;
    use codex_state::ThreadPlanSnapshot;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn item(
        row_id: &str,
        status: ThreadPlanItemStatus,
        path: &str,
        depends_on: &[&str],
    ) -> ThreadPlanItem {
        ThreadPlanItem {
            snapshot_id: "snapshot".to_string(),
            row_id: row_id.to_string(),
            row_index: 0,
            status,
            step: format!("step-{row_id}"),
            path: path.to_string(),
            details: String::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            depends_on: depends_on.iter().map(|value| value.to_string()).collect(),
            acceptance: None,
        }
    }

    #[test]
    fn resolves_all_dependency_ready_rows() {
        let items = vec![
            item("plan-01", ThreadPlanItemStatus::Completed, "a.rs", &[]),
            item(
                "plan-02",
                ThreadPlanItemStatus::Pending,
                "b.rs",
                &["plan-01"],
            ),
            item(
                "plan-03",
                ThreadPlanItemStatus::Pending,
                "c.rs",
                &["plan-01"],
            ),
            item(
                "plan-04",
                ThreadPlanItemStatus::Pending,
                "d.rs",
                &["plan-02"],
            ),
        ];

        let ready = resolve_dependency_ready_rows(items.as_slice());
        assert_eq!(
            ready
                .iter()
                .map(|row| row.row_id.as_str())
                .collect::<Vec<_>>(),
            vec!["plan-02", "plan-03"]
        );
    }

    #[test]
    fn finds_conflicting_active_rows_by_path() {
        let items = vec![
            item("plan-01", ThreadPlanItemStatus::InProgress, "same.rs", &[]),
            item("plan-02", ThreadPlanItemStatus::InProgress, "same.rs", &[]),
            item("plan-03", ThreadPlanItemStatus::InProgress, "other.rs", &[]),
        ];

        let conflicts = conflicting_active_rows(items.as_slice(), "plan-01");
        assert_eq!(conflicts, vec!["plan-02".to_string()]);
    }

    #[test]
    fn execute_dispatch_guard_instructions_reference_dispatch_tool() {
        let codex_home = TempDir::new().expect("tempdir");
        let repo = TempDir::new().expect("tempdir");
        let workspace = PlanWorkspace::new(codex_home.path(), repo.path(), "thread-1");
        let items = vec![
            item("plan-01", ThreadPlanItemStatus::Completed, "a.rs", &[]),
            item(
                "plan-02",
                ThreadPlanItemStatus::Pending,
                "b.rs",
                &["plan-01"],
            ),
        ];
        let plan = ActiveThreadPlan {
            snapshot: ThreadPlanSnapshot {
                id: "snapshot-1".to_string(),
                thread_id: "thread-1".to_string(),
                source_turn_id: "turn-1".to_string(),
                source_item_id: "item-1".to_string(),
                raw_csv: String::new(),
                created_at: Utc::now(),
                superseded_at: None,
            },
            items,
        };

        let instructions =
            build_execute_dispatch_guard_instructions(&workspace, &plan).expect("instructions");
        assert!(instructions.contains("execute_active_plan_with_subagents"));
        assert!(instructions.contains("dependency-ready rows"));
    }
}
