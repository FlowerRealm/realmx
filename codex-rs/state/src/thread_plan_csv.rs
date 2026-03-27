use crate::ThreadPlanItemCreateParams;
use crate::ThreadPlanItemStatus;
use std::collections::HashMap;
use std::collections::HashSet;

pub const THREAD_PLAN_CSV_HEADERS: [&str; 9] = [
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

pub fn parse_thread_plan_csv(raw_csv: &str) -> anyhow::Result<Vec<ThreadPlanItemCreateParams>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .from_reader(raw_csv.as_bytes());
    let headers = reader.headers()?;
    let headers = headers.iter().collect::<Vec<_>>();
    if headers != THREAD_PLAN_CSV_HEADERS {
        let expected = THREAD_PLAN_CSV_HEADERS.join(",");
        let found = headers.join(",");
        return Err(anyhow::anyhow!(
            "plan csv headers must be {expected}; found {found}"
        ));
    }

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
        let inputs = split_pipe_list(record.get(5).unwrap_or_default());
        let outputs = split_pipe_list(record.get(6).unwrap_or_default());
        let depends_on = split_pipe_list(record.get(7).unwrap_or_default());
        let acceptance = parse_optional_string(record.get(8).unwrap_or_default());

        rows.push(ThreadPlanItemCreateParams {
            row_id,
            row_index: rows.len() as i64,
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

pub fn render_thread_plan_csv(rows: &[ThreadPlanItemCreateParams]) -> anyhow::Result<String> {
    let mut writer = csv::WriterBuilder::new().from_writer(Vec::new());
    writer.write_record(THREAD_PLAN_CSV_HEADERS)?;
    for row in rows {
        let inputs = join_pipe_list(row.inputs.as_slice());
        let outputs = join_pipe_list(row.outputs.as_slice());
        let depends_on = join_pipe_list(row.depends_on.as_slice());
        writer.write_record([
            row.row_id.as_str(),
            row.status.as_str(),
            row.step.as_str(),
            row.path.as_str(),
            row.details.as_str(),
            inputs.as_str(),
            outputs.as_str(),
            depends_on.as_str(),
            row.acceptance.as_deref().unwrap_or_default(),
        ])?;
    }
    let bytes = writer
        .into_inner()
        .map_err(csv::IntoInnerError::into_error)?;
    String::from_utf8(bytes).map_err(|err| anyhow::anyhow!("invalid utf8 in rendered csv: {err}"))
}

pub fn canonicalize_thread_plan_csv(raw_csv: &str) -> anyhow::Result<String> {
    let rows = parse_thread_plan_csv(raw_csv)?;
    render_thread_plan_csv(rows.as_slice())
}

pub fn canonicalize_thread_plan_csv_for_authoring(raw_csv: &str) -> anyhow::Result<String> {
    let rows = parse_thread_plan_csv(raw_csv)?;
    validate_thread_plan_rows_for_authoring(rows.as_slice())?;
    render_thread_plan_csv(rows.as_slice())
}

fn parse_status(value: &str) -> anyhow::Result<ThreadPlanItemStatus> {
    match value.trim() {
        "pending" => Ok(ThreadPlanItemStatus::Pending),
        "in_progress" => Ok(ThreadPlanItemStatus::InProgress),
        "completed" => Ok(ThreadPlanItemStatus::Completed),
        other => Err(anyhow::anyhow!("invalid plan csv status: {other}")),
    }
}

fn parse_optional_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then_some(value.to_string())
}

fn split_pipe_list(value: &str) -> Vec<String> {
    value
        .split('|')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn join_pipe_list(values: &[String]) -> String {
    values.join("|")
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

pub fn validate_thread_plan_rows_for_authoring(
    rows: &[ThreadPlanItemCreateParams],
) -> anyhow::Result<()> {
    let id_to_index = rows
        .iter()
        .enumerate()
        .map(|(index, row)| (row.row_id.as_str(), index))
        .collect::<HashMap<_, _>>();

    for (index, row) in rows.iter().enumerate() {
        for dependency in &row.depends_on {
            let dependency_index = id_to_index.get(dependency.as_str()).ok_or_else(|| {
                anyhow::anyhow!(
                    "plan csv row {} depends on unknown id: {}",
                    row.row_id,
                    dependency
                )
            })?;
            if *dependency_index >= index {
                return Err(anyhow::anyhow!(
                    "plan csv row {} depends on a later row: {}",
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
    use super::canonicalize_thread_plan_csv;
    use super::canonicalize_thread_plan_csv_for_authoring;
    use super::parse_thread_plan_csv;
    use super::render_thread_plan_csv;
    use super::validate_thread_plan_rows_for_authoring;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_strict_plan_csv() {
        let raw_csv = "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Parse CSV,codex-rs/core/src/plan_csv.rs,extract rows,plan markdown|csv body,thread plan rows,,rows persist
";

        let rows = parse_thread_plan_csv(raw_csv).expect("csv should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row_id, "plan-01");
        assert_eq!(
            rows[0].inputs,
            vec!["plan markdown".to_string(), "csv body".to_string()]
        );
        assert_eq!(rows[0].acceptance.as_deref(), Some("rows persist"));
    }

    #[test]
    fn rejects_legacy_headers() {
        let raw_csv = "\
id,status,step,path,details
plan-01,pending,Parse CSV,codex-rs/core/src/plan_csv.rs,extract rows
";

        let err = parse_thread_plan_csv(raw_csv).expect_err("legacy csv should fail");
        assert_eq!(
            err.to_string(),
            "plan csv headers must be id,status,step,path,details,inputs,outputs,depends_on,acceptance; found id,status,step,path,details"
        );
    }

    #[test]
    fn renders_canonical_csv() {
        let raw_csv = "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Parse CSV,codex-rs/core/src/plan_csv.rs,extract rows, plan markdown | csv body , thread plan rows ,, rows persist
";

        let canonical = canonicalize_thread_plan_csv(raw_csv).expect("csv should canonicalize");
        assert_eq!(
            canonical,
            "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Parse CSV,codex-rs/core/src/plan_csv.rs,extract rows,plan markdown|csv body,thread plan rows,,rows persist
"
        );
    }

    #[test]
    fn render_round_trip_preserves_rows() {
        let raw_csv = "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,pending,Parse CSV,codex-rs/core/src/plan_csv.rs,extract rows,,,,
plan-02,completed,Render Plan,codex-rs/core/src/codex.rs,emit final item,thread plan rows,rendered plan text,plan-01,plan item text renders
";

        let rows = parse_thread_plan_csv(raw_csv).expect("csv should parse");
        let rendered = render_thread_plan_csv(rows.as_slice()).expect("csv should render");
        assert_eq!(rendered, raw_csv);
    }

    #[test]
    fn authoring_validation_accepts_dependency_safe_row_order() {
        let raw_csv = "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,pending,Root A,src/root_a.rs,,,,,
plan-02,pending,Child A,src/child_a.rs,,,,plan-01,
plan-03,pending,Join work,src/join.rs,,,,plan-01|plan-02,
";

        let rows = parse_thread_plan_csv(raw_csv).expect("csv should parse");
        validate_thread_plan_rows_for_authoring(rows.as_slice())
            .expect("authoring validation should pass");
        let canonical = canonicalize_thread_plan_csv_for_authoring(raw_csv)
            .expect("authoring canonicalization should pass");
        assert_eq!(canonical, raw_csv);
    }

    #[test]
    fn authoring_validation_rejects_dependency_on_later_row() {
        let raw_csv = "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,pending,Root A,src/root_a.rs,,,,plan-02,
plan-02,pending,Root B,src/root_b.rs,,,,,
";

        let rows = parse_thread_plan_csv(raw_csv).expect("csv should parse");
        let err = validate_thread_plan_rows_for_authoring(rows.as_slice())
            .expect_err("later-row dependency should fail");
        assert_eq!(
            err.to_string(),
            "plan csv row plan-01 depends on a later row: plan-02"
        );
    }
}
