use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_state::THREAD_PLAN_CSV_HEADERS;
use codex_state::ThreadPlanItem;
use codex_state::ThreadPlanItemCreateParams;
use codex_state::ThreadPlanItemStatus;
use codex_state::canonicalize_thread_plan_csv;
use codex_state::parse_thread_plan_csv;
use std::fmt::Write;

const CSV_OPEN_FENCE: &str = "```csv";
const CSV_CLOSE_FENCE: &str = "```";

#[derive(Debug)]
pub(crate) struct CanonicalPlanCsv {
    pub raw_csv: String,
    pub rows: Vec<ThreadPlanItemCreateParams>,
}

pub(crate) fn canonical_plan_csv_from_proposed_plan(
    markdown: &str,
) -> anyhow::Result<CanonicalPlanCsv> {
    let raw_csv = extract_csv_block(markdown)?;
    let raw_csv = canonicalize_thread_plan_csv(raw_csv.as_str())?;
    let rows = parse_thread_plan_csv(raw_csv.as_str())?;
    Ok(CanonicalPlanCsv { raw_csv, rows })
}

pub(crate) fn render_empty_plan_csv() -> String {
    format!("{}\n", THREAD_PLAN_CSV_HEADERS.join(","))
}

pub(crate) fn render_plan_text(rows: &[ThreadPlanItemCreateParams]) -> String {
    let mut out = String::from("# Plan\n\n");
    for (index, row) in rows.iter().enumerate() {
        let status = match row.status {
            ThreadPlanItemStatus::Pending => "pending",
            ThreadPlanItemStatus::InProgress => "in_progress",
            ThreadPlanItemStatus::Completed => "completed",
        };
        let _ = write!(out, "- [{status}] {} (`{}`)", row.step, row.path);
        if !row.details.is_empty() {
            let _ = write!(out, " - {}", row.details);
        }
        out.push('\n');
        append_plan_metadata_line(&mut out, "inputs", row.inputs.as_slice());
        append_plan_metadata_line(&mut out, "outputs", row.outputs.as_slice());
        append_plan_metadata_line(&mut out, "depends_on", row.depends_on.as_slice());
        if let Some(acceptance) = row.acceptance.as_deref() {
            let _ = writeln!(out, "  acceptance: {acceptance}");
        }
        if index + 1 != rows.len() {
            out.push('\n');
        }
    }
    out
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

fn append_plan_metadata_line(out: &mut String, label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    let _ = writeln!(out, "  {label}: {}", values.join(", "));
}

fn thread_plan_status_to_step_status(status: ThreadPlanItemStatus) -> StepStatus {
    match status {
        ThreadPlanItemStatus::Pending => StepStatus::Pending,
        ThreadPlanItemStatus::InProgress => StepStatus::InProgress,
        ThreadPlanItemStatus::Completed => StepStatus::Completed,
    }
}

#[cfg(test)]
mod tests {
    use super::canonical_plan_csv_from_proposed_plan;
    use super::render_plan_text;
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

- [in_progress] Parse CSV (`codex-rs/core/src/plan_csv.rs`) - extract rows
  inputs: plan markdown, csv body
  outputs: thread plan rows
  acceptance: rows persist
"
        );
    }
}
