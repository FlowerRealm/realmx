use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_state::ThreadPlanItem;
use codex_state::ThreadPlanItemCreateParams;
use codex_state::ThreadPlanItemStatus;
use codex_state::render_thread_plan_csv;
use std::collections::HashSet;
use std::fmt::Write;

const CSV_OPEN_FENCE: &str = "```csv";
const CSV_CLOSE_FENCE: &str = "```";
const LEGACY_HEADERS: [&str; 5] = ["id", "status", "step", "path", "details"];
const STRUCTURED_HEADERS: [&str; 9] = [
    "id",
    "status",
    "step",
    "path",
    "details",
    "inputs",
    "outputs",
    "depends_on",
    "acceptance",
];

#[derive(Debug)]
pub(crate) struct CanonicalPlanCsv {
    pub raw_csv: String,
    pub rows: Vec<ThreadPlanItemCreateParams>,
}

pub(crate) fn canonical_plan_csv_from_proposed_plan(
    markdown: &str,
) -> anyhow::Result<CanonicalPlanCsv> {
    let rows = parse_plan_csv(markdown)?;
    let raw_csv = render_thread_plan_csv(rows.as_slice())?;
    Ok(CanonicalPlanCsv { raw_csv, rows })
}

pub(crate) fn parse_plan_csv(markdown: &str) -> anyhow::Result<Vec<ThreadPlanItemCreateParams>> {
    let csv = extract_csv_block(markdown)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .from_reader(csv.as_bytes());
    let headers = reader.headers()?;
    let headers = headers.iter().collect::<Vec<_>>();
    let format = if headers == LEGACY_HEADERS {
        PlanCsvFormat::Legacy
    } else if headers == STRUCTURED_HEADERS {
        PlanCsvFormat::Structured
    } else {
        let found = headers.join(",");
        let expected = STRUCTURED_HEADERS.join(",");
        return Err(anyhow::anyhow!(
            "plan csv headers must be {expected} or {}; found {found}",
            LEGACY_HEADERS.join(",")
        ));
    };

    let mut rows = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut in_progress_count = 0usize;
    for (index, record) in reader.records().enumerate() {
        let record = record?;
        if record.iter().all(str::is_empty) {
            continue;
        }

        let row_id = record.get(0).unwrap_or_default().trim().to_string();
        if row_id.is_empty() {
            let row_number = index + 2;
            return Err(anyhow::anyhow!("plan csv row {row_number} is missing id"));
        }
        if !seen_ids.insert(row_id.clone()) {
            return Err(anyhow::anyhow!("duplicate plan csv id: {row_id}"));
        }

        let status = parse_status(record.get(1).unwrap_or_default())?;
        if matches!(status, ThreadPlanItemStatus::InProgress) {
            in_progress_count = in_progress_count.saturating_add(1);
        }

        let step = record.get(2).unwrap_or_default().trim().to_string();
        if step.is_empty() {
            return Err(anyhow::anyhow!("plan csv row {row_id} is missing step"));
        }

        let path = record.get(3).unwrap_or_default().trim().to_string();
        if path.is_empty() {
            return Err(anyhow::anyhow!("plan csv row {row_id} is missing path"));
        }

        rows.push(ThreadPlanItemCreateParams {
            row_id,
            row_index: rows.len() as i64,
            status,
            step,
            path,
            details: record.get(4).unwrap_or_default().trim().to_string(),
            inputs: format.parse_list(&record, /*index*/ 5),
            outputs: format.parse_list(&record, /*index*/ 6),
            depends_on: format.parse_list(&record, /*index*/ 7),
            acceptance: format.parse_optional_string(&record, /*index*/ 8),
        });
    }

    if rows.is_empty() {
        return Err(anyhow::anyhow!(
            "plan csv block must include at least one row"
        ));
    }
    if in_progress_count > 1 {
        return Err(anyhow::anyhow!(
            "plan csv may include at most one in_progress row"
        ));
    }
    validate_dependencies(rows.as_slice())?;
    Ok(rows)
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

pub(crate) fn render_plan_csv_markdown(
    items: &[ThreadPlanItemCreateParams],
) -> anyhow::Result<String> {
    let csv = render_thread_plan_csv(items)?;
    let csv = csv.trim_end_matches('\n');
    Ok(format!(
        "<proposed_plan>\n# Plan\n\n```csv\n{csv}\n```\n</proposed_plan>\n"
    ))
}

fn extract_csv_block(markdown: &str) -> anyhow::Result<&str> {
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
    Ok(csv)
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

fn parse_status(value: &str) -> anyhow::Result<ThreadPlanItemStatus> {
    match value.trim() {
        "pending" => Ok(ThreadPlanItemStatus::Pending),
        "in_progress" => Ok(ThreadPlanItemStatus::InProgress),
        "completed" => Ok(ThreadPlanItemStatus::Completed),
        other => Err(anyhow::anyhow!("invalid plan csv status: {other}")),
    }
}

#[derive(Clone, Copy)]
enum PlanCsvFormat {
    Legacy,
    Structured,
}

impl PlanCsvFormat {
    fn parse_list(self, record: &csv::StringRecord, index: usize) -> Vec<String> {
        if matches!(self, Self::Legacy) {
            return Vec::new();
        }

        split_pipe_list(record.get(index).unwrap_or_default())
    }

    fn parse_optional_string(self, record: &csv::StringRecord, index: usize) -> Option<String> {
        if matches!(self, Self::Legacy) {
            return None;
        }

        let value = record.get(index).unwrap_or_default().trim();
        (!value.is_empty()).then_some(value.to_string())
    }
}

fn split_pipe_list(value: &str) -> Vec<String> {
    value
        .split('|')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn validate_dependencies(rows: &[ThreadPlanItemCreateParams]) -> anyhow::Result<()> {
    let ids = rows
        .iter()
        .map(|row| row.row_id.as_str())
        .collect::<HashSet<_>>();
    for row in rows {
        for dependency in &row.depends_on {
            if dependency == &row.row_id {
                return Err(anyhow::anyhow!(
                    "plan csv row {} cannot depend on itself",
                    row.row_id
                ));
            }
            if !ids.contains(dependency.as_str()) {
                return Err(anyhow::anyhow!(
                    "plan csv row {} depends on unknown id: {}",
                    row.row_id,
                    dependency
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::canonical_plan_csv_from_proposed_plan;
    use super::parse_plan_csv;
    use super::render_plan_csv_markdown;
    use super::render_plan_text;
    use codex_state::ThreadPlanItemCreateParams;
    use codex_state::ThreadPlanItemStatus;
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
    fn canonicalizes_legacy_headers() {
        let markdown = r#"
```csv
id,status,step,path,details
plan-01,pending,Touch state,codex-rs/state/src/runtime.rs,add runtime hook
```
"#;
        let plan = canonical_plan_csv_from_proposed_plan(markdown).expect("csv should parse");
        assert_eq!(
            plan.raw_csv,
            "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,pending,Touch state,codex-rs/state/src/runtime.rs,add runtime hook,,,,
"
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

    #[test]
    fn renders_structured_plan_markdown() {
        let markdown = render_plan_csv_markdown(&[ThreadPlanItemCreateParams {
            row_id: "plan-01".to_string(),
            row_index: 0,
            status: ThreadPlanItemStatus::InProgress,
            step: "Parse CSV".to_string(),
            path: "codex-rs/core/src/plan_csv.rs".to_string(),
            details: "extract rows".to_string(),
            inputs: vec!["plan markdown".to_string()],
            outputs: vec!["thread plan rows".to_string()],
            depends_on: Vec::new(),
            acceptance: Some("rows persist".to_string()),
        }])
        .expect("markdown should render");

        assert!(markdown.starts_with("<proposed_plan>\n# Plan\n\n```csv\n"));
        assert!(
            markdown.contains("id,status,step,path,details,inputs,outputs,depends_on,acceptance")
        );
        assert!(markdown.contains("plan-01,in_progress,Parse CSV"));
    }

    #[test]
    fn parse_plan_csv_rejects_multiple_in_progress_rows() {
        let markdown = r#"
```csv
id,status,step,path,details
plan-01,in_progress,Touch state,codex-rs/state/src/runtime.rs,first row
plan-02,in_progress,Touch tui,codex-rs/tui/src/chatwidget.rs,second row
```
"#;

        let err = parse_plan_csv(markdown).expect_err("csv should reject duplicate in_progress");
        assert_eq!(
            err.to_string(),
            "plan csv may include at most one in_progress row"
        );
    }

    #[test]
    fn parse_plan_csv_rejects_unknown_dependency() {
        let markdown = r#"
```csv
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,pending,Parse CSV,codex-rs/core/src/plan_csv.rs,,,,plan-99,
```
"#;

        let err = parse_plan_csv(markdown).expect_err("unknown dependency should fail");
        assert_eq!(
            err.to_string(),
            "plan csv row plan-01 depends on unknown id: plan-99"
        );
    }
}
