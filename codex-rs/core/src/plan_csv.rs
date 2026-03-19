use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_state::ThreadPlanItem;
use codex_state::ThreadPlanItemCreateParams;
use codex_state::ThreadPlanItemStatus;
use std::collections::HashSet;

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
        let details = record.get(4).unwrap_or_default().trim().to_string();
        let inputs = format.parse_list(&record, 5);
        let outputs = format.parse_list(&record, 6);
        let depends_on = format.parse_list(&record, 7);
        let acceptance = format.parse_optional_string(&record, 8);
        rows.push(ThreadPlanItemCreateParams {
            row_id,
            row_index: index as i64,
            status,
            step,
            path,
            details,
            inputs,
            outputs,
            depends_on,
            acceptance,
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

fn parse_status(value: &str) -> anyhow::Result<ThreadPlanItemStatus> {
    match value.trim() {
        "pending" => Ok(ThreadPlanItemStatus::Pending),
        "in_progress" => Ok(ThreadPlanItemStatus::InProgress),
        "completed" => Ok(ThreadPlanItemStatus::Completed),
        other => Err(anyhow::anyhow!("invalid plan csv status: {other}")),
    }
}

fn thread_plan_status_to_step_status(status: ThreadPlanItemStatus) -> StepStatus {
    match status {
        ThreadPlanItemStatus::Pending => StepStatus::Pending,
        ThreadPlanItemStatus::InProgress => StepStatus::InProgress,
        ThreadPlanItemStatus::Completed => StepStatus::Completed,
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
    use super::parse_plan_csv;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_embedded_csv_block() {
        let markdown = r#"
# Plan

```csv
id,status,step,path,details
plan-01,in_progress,Touch state,codex-rs/state/src/runtime.rs,add runtime hook
plan-02,pending,Touch tui,codex-rs/tui/src/chatwidget.rs,
```
"#;
        let rows = parse_plan_csv(markdown).expect("csv should parse");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].row_id, "plan-01");
        assert_eq!(rows[0].path, "codex-rs/state/src/runtime.rs");
        assert_eq!(rows[1].details, "");
        assert!(rows[0].inputs.is_empty());
    }

    #[test]
    fn rejects_missing_path() {
        let markdown = r#"
```csv
id,status,step,path,details
plan-01,pending,Touch state,,missing file path
```
"#;
        let err = parse_plan_csv(markdown).expect_err("csv should reject missing path");
        assert_eq!(err.to_string(), "plan csv row plan-01 is missing path");
    }

    #[test]
    fn rejects_multiple_in_progress_rows() {
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
    fn parses_structured_rows() {
        let markdown = r#"
```csv
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Parse CSV,codex-rs/core/src/plan_csv.rs,extract rows,plan markdown|csv body,thread plan rows,,rows persist
plan-02,pending,Sync protocol,codex-rs/protocol/src/plan_tool.rs,add fields,thread plan rows,protocol rows,plan-01,notifications include fields
```
"#;
        let rows = parse_plan_csv(markdown).expect("structured csv should parse");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].inputs,
            vec!["plan markdown".to_string(), "csv body".to_string()]
        );
        assert_eq!(rows[1].depends_on, vec!["plan-01".to_string()]);
        assert_eq!(
            rows[1].acceptance.as_deref(),
            Some("notifications include fields")
        );
    }

    #[test]
    fn rejects_unknown_dependency() {
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

    #[test]
    fn rejects_self_dependency() {
        let markdown = r#"
```csv
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,pending,Parse CSV,codex-rs/core/src/plan_csv.rs,,,,plan-01,
```
"#;
        let err = parse_plan_csv(markdown).expect_err("self dependency should fail");
        assert_eq!(
            err.to_string(),
            "plan csv row plan-01 cannot depend on itself"
        );
    }
}
