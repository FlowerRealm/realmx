use std::path::Path;

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

pub(crate) fn build_execute_plan_guard_instructions(
    workspace_root: &Path,
    items: &[ThreadPlanItem],
) -> anyhow::Result<String> {
    let tasks_csv = workspace_root.join("tasks.csv");
    let tasks_md = workspace_root.join("tasks.md");
    let Some(target) = resolve_current_executable_target(items)? else {
        return Ok(format!(
            "Execute-mode plan workspace: `{}`.\nRead `{}` before acting.\nDerived plan text lives at `{}`.\nNo executable plan row is currently available.",
            workspace_root.display(),
            tasks_csv.display(),
            tasks_md.display(),
        ));
    };

    let acceptance = target.acceptance.as_deref().unwrap_or("");
    Ok(format!(
        "Execute-mode plan workspace: `{}`.\nRead `{}` before acting. Derived plan text lives at `{}`.\nThe accepted active plan is absolute truth in Execute mode.\nOnly execute the server-selected row below.\nCurrent executable row:\n- id: `{}`\n- step: {}\n- path: `{}`\n- details: {}\n- acceptance: {}\nDo not review or modify the plan. Record plan-external work only in `update_plan.explanation` and the final response.",
        workspace_root.display(),
        tasks_csv.display(),
        tasks_md.display(),
        target.row_id,
        target.step,
        target.path,
        target.details,
        acceptance,
    ))
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
    let row_id = update.id.as_deref().ok_or_else(|| {
        anyhow::anyhow!("Execute mode updates must include the current plan row id")
    })?;
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

fn target_from_item(item: &ThreadPlanItem) -> ExecutePlanTarget {
    ExecutePlanTarget {
        row_id: item.row_id.clone(),
        step: item.step.clone(),
        path: item.path.clone(),
        details: item.details.clone(),
        acceptance: item.acceptance.clone(),
    }
}

fn reject_metadata_mutation(update: &PlanItemArg, current: &ThreadPlanItem) -> anyhow::Result<()> {
    if update.step != current.step {
        anyhow::bail!("Execute mode may not change the current plan row step");
    }
    reject_optional_string_mutation(
        update.path.as_deref(),
        Some(current.path.as_str()),
        "Execute mode may not change the current plan row path",
    )?;
    reject_optional_string_mutation(
        update.details.as_deref(),
        Some(current.details.as_str()),
        "Execute mode may not change the current plan row details",
    )?;
    reject_optional_vec_mutation(
        update.inputs.as_deref(),
        current.inputs.as_slice(),
        "Execute mode may not change the current plan row inputs",
    )?;
    reject_optional_vec_mutation(
        update.outputs.as_deref(),
        current.outputs.as_slice(),
        "Execute mode may not change the current plan row outputs",
    )?;
    reject_optional_vec_mutation(
        update.depends_on.as_deref(),
        current.depends_on.as_slice(),
        "Execute mode may not change the current plan row dependencies",
    )?;
    reject_optional_string_mutation(
        update.acceptance.as_deref(),
        current.acceptance.as_deref(),
        "Execute mode may not change the current plan row acceptance",
    )?;
    Ok(())
}

fn reject_optional_string_mutation(
    update: Option<&str>,
    current: Option<&str>,
    message: &str,
) -> anyhow::Result<()> {
    if let Some(update) = update
        && Some(update) != current
    {
        anyhow::bail!("{message}");
    }
    Ok(())
}

fn reject_optional_vec_mutation(
    update: Option<&[String]>,
    current: &[String],
    message: &str,
) -> anyhow::Result<()> {
    if let Some(update) = update
        && update != current
    {
        anyhow::bail!("{message}");
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
        (ThreadPlanItemStatus::Pending, StepStatus::Pending) => {
            anyhow::bail!("Execute mode pending rows must first transition to in_progress")
        }
        (ThreadPlanItemStatus::Pending, StepStatus::Completed) => {
            anyhow::bail!("Execute mode pending rows must first transition to in_progress")
        }
        (ThreadPlanItemStatus::InProgress, StepStatus::Pending)
        | (ThreadPlanItemStatus::Completed, StepStatus::Pending)
        | (ThreadPlanItemStatus::Completed, StepStatus::InProgress) => {
            anyhow::bail!("Execute mode received an invalid plan status transition")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ExecutePlanTarget;
    use super::build_execute_plan_guard_instructions;
    use super::resolve_current_executable_target;
    use super::validate_execute_mode_plan_update;
    use codex_protocol::plan_tool::PlanItemArg;
    use codex_protocol::plan_tool::StepStatus;
    use codex_protocol::plan_tool::UpdatePlanArgs;
    use codex_state::ThreadPlanItem;
    use codex_state::ThreadPlanItemStatus;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

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
    fn selects_first_pending_row_without_dependencies() {
        let rows = sample_rows(&[
            ("plan-01", ThreadPlanItemStatus::Pending, vec![]),
            ("plan-02", ThreadPlanItemStatus::Pending, vec!["plan-01"]),
        ]);

        let target =
            resolve_current_executable_target(rows.as_slice()).expect("target selection succeeds");
        assert_eq!(target, Some(sample_target("plan-01")));
    }

    #[test]
    fn returns_none_when_no_dependency_ready_pending_row_exists() {
        let rows = sample_rows(&[
            ("plan-01", ThreadPlanItemStatus::Completed, vec![]),
            (
                "plan-02",
                ThreadPlanItemStatus::Pending,
                vec!["missing-plan"],
            ),
        ]);

        let target =
            resolve_current_executable_target(rows.as_slice()).expect("target selection succeeds");
        assert_eq!(target, None);
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
                sample_update("plan-01", StepStatus::InProgress),
                sample_update("plan-02", StepStatus::Pending),
            ],
        };

        let err = validate_execute_mode_plan_update(rows.as_slice(), &args)
            .expect_err("multi-row execute update should fail");
        assert_eq!(
            err.to_string(),
            "Execute mode may only update the server-selected current plan row"
        );
    }

    #[test]
    fn rejects_updates_without_row_id() {
        let rows = sample_rows(&[("plan-01", ThreadPlanItemStatus::Pending, vec![])]);
        let mut update = sample_update("plan-01", StepStatus::InProgress);
        update.id = None;
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![update],
        };

        let err = validate_execute_mode_plan_update(rows.as_slice(), &args)
            .expect_err("missing id should fail");
        assert_eq!(
            err.to_string(),
            "Execute mode updates must include the current plan row id"
        );
    }

    #[test]
    fn rejects_non_current_row_updates() {
        let rows = sample_rows(&[
            ("plan-01", ThreadPlanItemStatus::Pending, vec![]),
            ("plan-02", ThreadPlanItemStatus::Pending, vec!["plan-01"]),
        ]);
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![sample_update("plan-02", StepStatus::InProgress)],
        };

        let err = validate_execute_mode_plan_update(rows.as_slice(), &args)
            .expect_err("non-current row should fail");
        assert_eq!(
            err.to_string(),
            "Execute mode may only update the server-selected current plan row"
        );
    }

    #[test]
    fn rejects_metadata_mutation() {
        let rows = sample_rows(&[("plan-01", ThreadPlanItemStatus::Pending, vec![])]);
        let mut update = sample_update("plan-01", StepStatus::InProgress);
        update.details = Some("changed details".to_string());
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![update],
        };

        let err = validate_execute_mode_plan_update(rows.as_slice(), &args)
            .expect_err("details mutation should fail");
        assert_eq!(
            err.to_string(),
            "Execute mode may not change the current plan row details"
        );
    }

    #[test]
    fn rejects_pending_to_completed_transition() {
        let rows = sample_rows(&[("plan-01", ThreadPlanItemStatus::Pending, vec![])]);
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![sample_update("plan-01", StepStatus::Completed)],
        };

        let err = validate_execute_mode_plan_update(rows.as_slice(), &args)
            .expect_err("pending to completed should fail");
        assert_eq!(
            err.to_string(),
            "Execute mode pending rows must first transition to in_progress"
        );
    }

    #[test]
    fn allows_current_row_to_start() {
        let rows = sample_rows(&[("plan-01", ThreadPlanItemStatus::Pending, vec![])]);
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![sample_update("plan-01", StepStatus::InProgress)],
        };

        validate_execute_mode_plan_update(rows.as_slice(), &args)
            .expect("pending to in_progress should pass");
    }

    #[test]
    fn allows_in_progress_row_to_complete() {
        let rows = sample_rows(&[("plan-01", ThreadPlanItemStatus::InProgress, vec![])]);
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![sample_update("plan-01", StepStatus::Completed)],
        };

        validate_execute_mode_plan_update(rows.as_slice(), &args)
            .expect("in_progress to completed should pass");
    }

    #[test]
    fn allows_idempotent_progress_updates() {
        let rows = sample_rows(&[("plan-01", ThreadPlanItemStatus::InProgress, vec![])]);
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![sample_update("plan-01", StepStatus::InProgress)],
        };

        validate_execute_mode_plan_update(rows.as_slice(), &args)
            .expect("in_progress to in_progress should pass");
    }

    #[test]
    fn rejects_updates_when_plan_has_no_current_executable_row() {
        let rows = sample_rows(&[("plan-01", ThreadPlanItemStatus::Completed, vec![])]);
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![sample_update("plan-01", StepStatus::InProgress)],
        };

        let err = validate_execute_mode_plan_update(rows.as_slice(), &args)
            .expect_err("completed-only plan should reject execute-mode updates");
        assert_eq!(
            err.to_string(),
            "Execute mode has no current executable plan row"
        );
    }

    #[test]
    fn build_instructions_only_includes_current_row() {
        let rows = sample_rows(&[
            ("plan-01", ThreadPlanItemStatus::Pending, vec![]),
            ("plan-02", ThreadPlanItemStatus::Pending, vec!["plan-01"]),
        ]);
        let tmp = TempDir::new().expect("tmp dir");

        let text = build_execute_plan_guard_instructions(tmp.path(), rows.as_slice())
            .expect("instructions should build");

        assert!(text.contains("tasks.csv"));
        assert!(text.contains("tasks.md"));
        assert!(text.contains("plan-01"));
        assert!(!text.contains("plan-02"));
    }

    fn sample_rows(rows: &[(&str, ThreadPlanItemStatus, Vec<&str>)]) -> Vec<ThreadPlanItem> {
        rows.iter()
            .enumerate()
            .map(|(index, (row_id, status, depends_on))| ThreadPlanItem {
                snapshot_id: "snapshot-1".to_string(),
                row_id: (*row_id).to_string(),
                row_index: index as i64,
                status: *status,
                step: sample_step(row_id),
                path: sample_path(row_id),
                details: sample_details(row_id),
                inputs: vec!["input".to_string()],
                outputs: vec!["output".to_string()],
                depends_on: depends_on
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                acceptance: Some(sample_acceptance(row_id)),
            })
            .collect()
    }

    fn sample_update(row_id: &str, status: StepStatus) -> PlanItemArg {
        PlanItemArg {
            id: Some(row_id.to_string()),
            step: sample_step(row_id),
            status,
            path: Some(sample_path(row_id)),
            details: Some(sample_details(row_id)),
            inputs: Some(vec!["input".to_string()]),
            outputs: Some(vec!["output".to_string()]),
            depends_on: None,
            acceptance: Some(sample_acceptance(row_id)),
        }
    }

    fn sample_target(row_id: &str) -> ExecutePlanTarget {
        ExecutePlanTarget {
            row_id: row_id.to_string(),
            step: sample_step(row_id),
            path: sample_path(row_id),
            details: sample_details(row_id),
            acceptance: Some(sample_acceptance(row_id)),
        }
    }

    fn sample_step(row_id: &str) -> String {
        format!("step for {row_id}")
    }

    fn sample_path(row_id: &str) -> String {
        format!("codex-rs/core/src/{row_id}.rs")
    }

    fn sample_details(row_id: &str) -> String {
        format!("details for {row_id}")
    }

    fn sample_acceptance(row_id: &str) -> String {
        format!("acceptance for {row_id}")
    }
}
