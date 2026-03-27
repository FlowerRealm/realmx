use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_state::THREAD_PLAN_CSV_HEADERS;
use codex_state::ThreadPlanItem;
use codex_state::ThreadPlanItemCreateParams;
use codex_state::ThreadPlanItemStatus;
use codex_state::canonicalize_thread_plan_csv;
use codex_state::canonicalize_thread_plan_csv_for_authoring;
use codex_state::parse_thread_plan_csv;
use codex_state::render_thread_plan_csv;

use crate::plan_display::render_plan_markdown;

const CSV_OPEN_FENCE: &str = "```csv";
const CSV_CLOSE_FENCE: &str = "```";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalPlanCsv {
    pub raw_csv: String,
    pub rows: Vec<ThreadPlanItemCreateParams>,
}

pub(crate) fn canonical_plan_csv_from_proposed_plan(
    markdown: &str,
) -> anyhow::Result<CanonicalPlanCsv> {
    let raw_csv = extract_csv_block(markdown)?;
    let raw_csv = canonicalize_thread_plan_csv_for_authoring(raw_csv.as_str())?;
    let rows = parse_thread_plan_csv(raw_csv.as_str())?;
    Ok(CanonicalPlanCsv { raw_csv, rows })
}

pub fn canonical_plan_csv_from_update_plan_args(
    args: &UpdatePlanArgs,
) -> anyhow::Result<CanonicalPlanCsv> {
    canonical_plan_csv_from_update_plan_args_with_mode(args, CanonicalizationMode::Compatible)
}

pub(crate) fn canonical_plan_csv_from_update_plan_args_for_authoring(
    args: &UpdatePlanArgs,
) -> anyhow::Result<CanonicalPlanCsv> {
    canonical_plan_csv_from_update_plan_args_with_mode(args, CanonicalizationMode::Authoring)
}

fn canonical_plan_csv_from_update_plan_args_with_mode(
    args: &UpdatePlanArgs,
    mode: CanonicalizationMode,
) -> anyhow::Result<CanonicalPlanCsv> {
    let rows = args
        .plan
        .iter()
        .enumerate()
        .map(|(index, item)| thread_plan_row_from_plan_item(index, item))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let raw_csv = render_thread_plan_csv(rows.as_slice())?;
    let raw_csv = match mode {
        CanonicalizationMode::Compatible => canonicalize_thread_plan_csv(raw_csv.as_str())?,
        CanonicalizationMode::Authoring => {
            canonicalize_thread_plan_csv_for_authoring(raw_csv.as_str())?
        }
    };
    let rows = parse_thread_plan_csv(raw_csv.as_str())?;
    Ok(CanonicalPlanCsv { raw_csv, rows })
}

pub(crate) fn render_empty_plan_csv() -> String {
    format!("{}\n", THREAD_PLAN_CSV_HEADERS.join(","))
}

pub(crate) fn render_plan_text(rows: &[ThreadPlanItemCreateParams]) -> String {
    let plan = UpdatePlanArgs {
        explanation: None,
        plan: rows
            .iter()
            .map(|row| PlanItemArg {
                id: Some(row.row_id.clone()),
                step: row.step.clone(),
                status: thread_plan_status_to_step_status(row.status),
                path: Some(row.path.clone()),
                details: (!row.details.is_empty()).then_some(row.details.clone()),
                inputs: (!row.inputs.is_empty()).then_some(row.inputs.clone()),
                outputs: (!row.outputs.is_empty()).then_some(row.outputs.clone()),
                depends_on: (!row.depends_on.is_empty()).then_some(row.depends_on.clone()),
                acceptance: row.acceptance.clone(),
            })
            .collect(),
    };
    render_plan_markdown(plan.plan.as_slice())
}

pub(crate) fn update_plan_from_thread_plan_items(
    items: &[ThreadPlanItem],
    explanation: Option<String>,
) -> UpdatePlanArgs {
    UpdatePlanArgs {
        explanation,
        plan: items
            .iter()
            .map(|item| PlanItemArg {
                id: Some(item.row_id.clone()),
                step: item.step.clone(),
                status: thread_plan_status_to_step_status(item.status),
                path: Some(item.path.clone()),
                details: (!item.details.is_empty()).then_some(item.details.clone()),
                inputs: (!item.inputs.is_empty()).then_some(item.inputs.clone()),
                outputs: (!item.outputs.is_empty()).then_some(item.outputs.clone()),
                depends_on: (!item.depends_on.is_empty()).then_some(item.depends_on.clone()),
                acceptance: item.acceptance.clone(),
            })
            .collect(),
    }
}

fn extract_csv_block(markdown: &str) -> anyhow::Result<String> {
    let open_index = markdown
        .find(CSV_OPEN_FENCE)
        .ok_or_else(|| anyhow::anyhow!("missing csv block in proposed plan"))?;
    let body_start = open_index + CSV_OPEN_FENCE.len();
    let body = markdown[body_start..]
        .strip_prefix('\n')
        .unwrap_or(&markdown[body_start..]);
    let close_index = body
        .find(&format!("\n{CSV_CLOSE_FENCE}"))
        .ok_or_else(|| anyhow::anyhow!("unterminated csv block in proposed plan"))?;
    let csv = &body[..close_index];
    if body[close_index + 1 + CSV_CLOSE_FENCE.len()..].contains(CSV_OPEN_FENCE) {
        return Err(anyhow::anyhow!(
            "multiple csv blocks found in proposed plan"
        ));
    }
    Ok(csv.to_string())
}

fn thread_plan_row_from_plan_item(
    index: usize,
    item: &PlanItemArg,
) -> anyhow::Result<ThreadPlanItemCreateParams> {
    let row_number = index + 1;
    let row_id = item
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("plan-{row_number:02}"));
    let path = item
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("plan update row {row_id} is missing path"))?;
    Ok(ThreadPlanItemCreateParams {
        row_id,
        row_index: index as i64,
        status: step_status_to_thread_plan_status(item.status.clone()),
        step: item.step.trim().to_string(),
        path,
        details: item
            .details
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string(),
        inputs: normalize_plan_values(item.inputs.as_deref().unwrap_or_default()),
        outputs: normalize_plan_values(item.outputs.as_deref().unwrap_or_default()),
        depends_on: normalize_plan_values(item.depends_on.as_deref().unwrap_or_default()),
        acceptance: item
            .acceptance
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    })
}

fn normalize_plan_values(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn thread_plan_status_to_step_status(status: ThreadPlanItemStatus) -> StepStatus {
    match status {
        ThreadPlanItemStatus::Pending => StepStatus::Pending,
        ThreadPlanItemStatus::InProgress => StepStatus::InProgress,
        ThreadPlanItemStatus::Completed => StepStatus::Completed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanonicalizationMode {
    Compatible,
    Authoring,
}

fn step_status_to_thread_plan_status(status: StepStatus) -> ThreadPlanItemStatus {
    match status {
        StepStatus::Pending => ThreadPlanItemStatus::Pending,
        StepStatus::InProgress => ThreadPlanItemStatus::InProgress,
        StepStatus::Completed => ThreadPlanItemStatus::Completed,
    }
}

#[cfg(test)]
mod tests {
    use super::canonical_plan_csv_from_proposed_plan;
    use super::canonical_plan_csv_from_update_plan_args;
    use super::render_plan_text;
    use codex_protocol::plan_tool::PlanItemArg;
    use codex_protocol::plan_tool::StepStatus;
    use codex_protocol::plan_tool::UpdatePlanArgs;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_embedded_csv_block() {
        let markdown = r#"
# Plan

```csv
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Touch state,codex-rs/state/src/runtime.rs,add runtime hook,plan markdown,active plan rows,,active plan reloads
plan-02,pending,Touch tui,codex-rs/tui/src/chatwidget.rs,,active plan rows,history cell update,plan-01,
```
"#;
        let plan = canonical_plan_csv_from_proposed_plan(markdown).expect("csv should parse");
        assert_eq!(plan.rows.len(), 2);
        assert_eq!(plan.rows[0].row_id, "plan-01");
        assert_eq!(plan.rows[0].path, "codex-rs/state/src/runtime.rs");
        assert_eq!(
            plan.raw_csv,
            "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Touch state,codex-rs/state/src/runtime.rs,add runtime hook,plan markdown,active plan rows,,active plan reloads
plan-02,pending,Touch tui,codex-rs/tui/src/chatwidget.rs,,active plan rows,history cell update,plan-01,
"
        );
    }

    #[test]
    fn rejects_missing_csv_block() {
        let err = canonical_plan_csv_from_proposed_plan("# Plan").expect_err("csv should fail");
        assert_eq!(err.to_string(), "missing csv block in proposed plan");
    }

    #[test]
    fn rejects_legacy_headers() {
        let markdown = r#"
```csv
id,status,step,path,details
plan-01,pending,Touch state,codex-rs/state/src/runtime.rs,add runtime hook
```
"#;
        let err = canonical_plan_csv_from_proposed_plan(markdown).expect_err("csv should fail");
        assert_eq!(
            err.to_string(),
            "plan csv headers must be id,status,step,path,details,inputs,outputs,depends_on,acceptance; found id,status,step,path,details"
        );
    }

    #[test]
    fn renders_plan_text_from_rows() {
        let markdown = r#"
```csv
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Parse CSV,codex-rs/core/src/plan_csv.rs,extract rows,plan markdown|csv body,thread plan rows,,rows persist
```
"#;
        let plan = canonical_plan_csv_from_proposed_plan(markdown).expect("csv should parse");
        assert_eq!(
            render_plan_text(plan.rows.as_slice()),
            "\
# Plan

## Dependency Tree

- [in_progress] 1 Parse CSV (`plan-01`; `codex-rs/core/src/plan_csv.rs`) - extract rows
  inputs: plan markdown, csv body
  outputs: thread plan rows
  acceptance: rows persist

## Parallel Layers

- L0: 1 (`plan-01`)
"
        );
    }

    #[test]
    fn canonicalizes_update_plan_args() {
        let args = UpdatePlanArgs {
            explanation: Some("structured".to_string()),
            plan: vec![
                PlanItemArg {
                    id: None,
                    step: "Touch state".to_string(),
                    status: StepStatus::InProgress,
                    path: Some("codex-rs/state/src/runtime/thread_plans.rs".to_string()),
                    details: Some("persist canonical rows".to_string()),
                    inputs: Some(vec![" update args ".to_string(), "".to_string()]),
                    outputs: Some(vec![" active plan ".to_string()]),
                    depends_on: None,
                    acceptance: Some(" active plan stored ".to_string()),
                },
                PlanItemArg {
                    id: Some("plan-custom".to_string()),
                    step: "Refresh UI".to_string(),
                    status: StepStatus::Pending,
                    path: Some("codex-rs/tui/src/history_cell.rs".to_string()),
                    details: None,
                    inputs: None,
                    outputs: None,
                    depends_on: Some(vec!["plan-01".to_string()]),
                    acceptance: None,
                },
            ],
        };

        let plan = canonical_plan_csv_from_update_plan_args(&args).expect("csv should parse");
        assert_eq!(
            plan.raw_csv,
            "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Touch state,codex-rs/state/src/runtime/thread_plans.rs,persist canonical rows,update args,active plan,,active plan stored
plan-custom,pending,Refresh UI,codex-rs/tui/src/history_cell.rs,,,,plan-01,
"
        );
        assert_eq!(plan.rows[0].row_id, "plan-01");
        assert_eq!(plan.rows[1].row_id, "plan-custom");
    }

    #[test]
    fn rejects_update_plan_args_without_path() {
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![PlanItemArg {
                id: None,
                step: "Touch state".to_string(),
                status: StepStatus::InProgress,
                path: None,
                details: None,
                inputs: None,
                outputs: None,
                depends_on: None,
                acceptance: None,
            }],
        };

        let err = canonical_plan_csv_from_update_plan_args(&args).expect_err("path should fail");
        assert_eq!(err.to_string(), "plan update row plan-01 is missing path");
    }
}
