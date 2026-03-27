use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use std::collections::HashMap;
use std::fmt::Write;

const FLAT_LIST_NOTE: &str =
    "Legacy dependency layout detected; showing a flat list instead of a dependency tree.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanDisplayMode {
    Tree,
    Flat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDisplayProjection {
    pub mode: PlanDisplayMode,
    pub compatibility_note: Option<String>,
    pub rows: Vec<PlanDisplayRow>,
    pub layers: Vec<PlanDisplayLayer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDisplayRow {
    pub row_id: Option<String>,
    pub effective_row_id: String,
    pub display_number: Option<String>,
    pub depth: usize,
    pub layer: Option<usize>,
    pub primary_parent_id: Option<String>,
    pub additional_dependencies: Vec<String>,
    pub step: String,
    pub status: StepStatus,
    pub path: Option<String>,
    pub details: Option<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub depends_on: Vec<String>,
    pub acceptance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDisplayLayer {
    pub layer_index: usize,
    pub rows: Vec<PlanDisplayLayerRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDisplayLayerRow {
    pub effective_row_id: String,
    pub row_id: Option<String>,
    pub display_number: Option<String>,
    pub step: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedPlanRow {
    row_id: Option<String>,
    effective_row_id: String,
    step: String,
    status: StepStatus,
    path: Option<String>,
    details: Option<String>,
    inputs: Vec<String>,
    outputs: Vec<String>,
    depends_on: Vec<String>,
    acceptance: Option<String>,
}

pub fn project_plan_items(items: &[PlanItemArg]) -> PlanDisplayProjection {
    let normalized = items
        .iter()
        .enumerate()
        .map(|(index, item)| normalize_plan_row(index, item))
        .collect::<Vec<_>>();

    match build_tree_projection(normalized.as_slice()) {
        Some(projection) => projection,
        None => build_flat_projection(normalized.as_slice()),
    }
}

pub fn render_plan_markdown(items: &[PlanItemArg]) -> String {
    let projection = project_plan_items(items);
    let mut out = String::from("# Plan\n\n");

    if let Some(note) = projection.compatibility_note.as_deref() {
        let _ = writeln!(out, "> {note}\n");
    }

    match projection.mode {
        PlanDisplayMode::Tree => {
            out.push_str("## Dependency Tree\n\n");
            for row in &projection.rows {
                append_tree_row(&mut out, row);
            }
            out.push('\n');
            out.push_str("## Parallel Layers\n\n");
            for layer in &projection.layers {
                append_layer_row(&mut out, layer);
            }
        }
        PlanDisplayMode::Flat => {
            out.push_str("## Plan Steps\n\n");
            for row in &projection.rows {
                append_flat_row(&mut out, row);
            }
        }
    }

    out
}

fn normalize_plan_row(index: usize, item: &PlanItemArg) -> NormalizedPlanRow {
    let row_id = item
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let path = item
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let details = item
        .details
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let acceptance = item
        .acceptance
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    NormalizedPlanRow {
        effective_row_id: row_id
            .clone()
            .unwrap_or_else(|| format!("__row-{:02}", index + 1)),
        row_id,
        step: item.step.trim().to_string(),
        status: item.status.clone(),
        path,
        details,
        inputs: normalize_list(item.inputs.as_deref().unwrap_or_default()),
        outputs: normalize_list(item.outputs.as_deref().unwrap_or_default()),
        depends_on: normalize_list(item.depends_on.as_deref().unwrap_or_default()),
        acceptance,
    }
}

fn normalize_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn build_tree_projection(rows: &[NormalizedPlanRow]) -> Option<PlanDisplayProjection> {
    let mut id_to_index = HashMap::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        if id_to_index
            .insert(row.effective_row_id.as_str(), index)
            .is_some()
        {
            return None;
        }
    }

    let mut root_count = 0usize;
    let mut child_counts: HashMap<&str, usize> = HashMap::new();
    let mut numbers = vec![None; rows.len()];
    let mut depths = vec![0usize; rows.len()];
    let mut layers = vec![None; rows.len()];
    let mut primary_parents = vec![None; rows.len()];

    for (index, row) in rows.iter().enumerate() {
        let Some(primary_dependency) = row.depends_on.first() else {
            root_count += 1;
            numbers[index] = Some(root_count.to_string());
            layers[index] = Some(0);
            continue;
        };

        let parent_index = *id_to_index.get(primary_dependency.as_str())?;
        if parent_index >= index {
            return None;
        }
        let parent_number = numbers[parent_index].clone()?;
        let parent_layer = layers[parent_index]?;

        let max_dependency_layer = row.depends_on.iter().try_fold(parent_layer, |acc, dep| {
            let dep_index = *id_to_index.get(dep.as_str())?;
            if dep_index >= index {
                return None;
            }
            Some(acc.max(layers[dep_index]?))
        })?;

        let child_position = child_counts
            .entry(primary_dependency.as_str())
            .and_modify(|count| *count += 1)
            .or_insert(1);
        numbers[index] = Some(format!("{parent_number}.{child_position}"));
        depths[index] = depths[parent_index] + 1;
        layers[index] = Some(max_dependency_layer + 1);
        primary_parents[index] = Some(primary_dependency.clone());
    }

    let projected_rows = rows
        .iter()
        .enumerate()
        .map(|(index, row)| PlanDisplayRow {
            row_id: row.row_id.clone(),
            effective_row_id: row.effective_row_id.clone(),
            display_number: numbers[index].clone(),
            depth: depths[index],
            layer: layers[index],
            primary_parent_id: primary_parents[index].clone(),
            additional_dependencies: match primary_parents[index].as_deref() {
                Some(primary_parent) => row
                    .depends_on
                    .iter()
                    .filter(|dep| dep.as_str() != primary_parent)
                    .cloned()
                    .collect(),
                None => Vec::new(),
            },
            step: row.step.clone(),
            status: row.status.clone(),
            path: row.path.clone(),
            details: row.details.clone(),
            inputs: row.inputs.clone(),
            outputs: row.outputs.clone(),
            depends_on: row.depends_on.clone(),
            acceptance: row.acceptance.clone(),
        })
        .collect::<Vec<_>>();

    Some(PlanDisplayProjection {
        mode: PlanDisplayMode::Tree,
        compatibility_note: None,
        layers: build_layers(projected_rows.as_slice()),
        rows: projected_rows,
    })
}

fn build_layers(rows: &[PlanDisplayRow]) -> Vec<PlanDisplayLayer> {
    let Some(max_layer) = rows.iter().filter_map(|row| row.layer).max() else {
        return Vec::new();
    };

    (0..=max_layer)
        .map(|layer_index| PlanDisplayLayer {
            layer_index,
            rows: rows
                .iter()
                .filter(|row| row.layer == Some(layer_index))
                .map(|row| PlanDisplayLayerRow {
                    effective_row_id: row.effective_row_id.clone(),
                    row_id: row.row_id.clone(),
                    display_number: row.display_number.clone(),
                    step: row.step.clone(),
                })
                .collect(),
        })
        .filter(|layer| !layer.rows.is_empty())
        .collect()
}

fn build_flat_projection(rows: &[NormalizedPlanRow]) -> PlanDisplayProjection {
    PlanDisplayProjection {
        mode: PlanDisplayMode::Flat,
        compatibility_note: Some(FLAT_LIST_NOTE.to_string()),
        rows: rows
            .iter()
            .map(|row| PlanDisplayRow {
                row_id: row.row_id.clone(),
                effective_row_id: row.effective_row_id.clone(),
                display_number: None,
                depth: 0,
                layer: None,
                primary_parent_id: None,
                additional_dependencies: Vec::new(),
                step: row.step.clone(),
                status: row.status.clone(),
                path: row.path.clone(),
                details: row.details.clone(),
                inputs: row.inputs.clone(),
                outputs: row.outputs.clone(),
                depends_on: row.depends_on.clone(),
                acceptance: row.acceptance.clone(),
            })
            .collect(),
        layers: Vec::new(),
    }
}

fn append_tree_row(out: &mut String, row: &PlanDisplayRow) {
    let indent = "  ".repeat(row.depth);
    let number = row.display_number.as_deref().unwrap_or("?");
    let _ = write!(
        out,
        "{indent}- [{}] {number} {}",
        status_label(&row.status),
        row.step
    );
    append_inline_identity(out, row);
    append_details_suffix(out, row.details.as_deref());
    out.push('\n');
    append_metadata_lines(
        out,
        indent.as_str(),
        row.inputs.as_slice(),
        row.outputs.as_slice(),
        if row.additional_dependencies.is_empty() {
            None
        } else {
            Some(row.depends_on.as_slice())
        },
        row.acceptance.as_deref(),
    );
}

fn append_flat_row(out: &mut String, row: &PlanDisplayRow) {
    let _ = write!(out, "- [{}] {}", status_label(&row.status), row.step);
    append_inline_identity(out, row);
    append_details_suffix(out, row.details.as_deref());
    out.push('\n');
    append_metadata_lines(
        out,
        "",
        row.inputs.as_slice(),
        row.outputs.as_slice(),
        (!row.depends_on.is_empty()).then_some(row.depends_on.as_slice()),
        row.acceptance.as_deref(),
    );
}

fn append_layer_row(out: &mut String, layer: &PlanDisplayLayer) {
    let entries = layer
        .rows
        .iter()
        .map(|row| {
            let number = row.display_number.as_deref().unwrap_or("?");
            let row_id = row
                .row_id
                .as_deref()
                .unwrap_or(row.effective_row_id.as_str());
            format!("{number} (`{row_id}`)")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(out, "- L{}: {entries}", layer.layer_index);
}

fn append_inline_identity(out: &mut String, row: &PlanDisplayRow) {
    let row_id = row.row_id.as_deref();
    match (row_id, row.path.as_deref()) {
        (Some(row_id), Some(path)) => {
            let _ = write!(out, " (`{row_id}`; `{path}`)");
        }
        (Some(row_id), None) => {
            let _ = write!(out, " (`{row_id}`)");
        }
        (None, Some(path)) => {
            let _ = write!(out, " (`{path}`)");
        }
        (None, None) => {}
    }
}

fn append_details_suffix(out: &mut String, details: Option<&str>) {
    if let Some(details) = details {
        let _ = write!(out, " - {details}");
    }
}

fn append_metadata_lines(
    out: &mut String,
    indent: &str,
    inputs: &[String],
    outputs: &[String],
    depends_on: Option<&[String]>,
    acceptance: Option<&str>,
) {
    append_metadata_line(out, indent, "inputs", inputs);
    append_metadata_line(out, indent, "outputs", outputs);
    if let Some(depends_on) = depends_on {
        append_metadata_line(out, indent, "depends_on", depends_on);
    }
    if let Some(acceptance) = acceptance {
        let _ = writeln!(out, "{indent}  acceptance: {acceptance}");
    }
}

fn append_metadata_line(out: &mut String, indent: &str, label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    let _ = writeln!(out, "{indent}  {label}: {}", values.join(", "));
}

fn status_label(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::Pending => "pending",
        StepStatus::InProgress => "in_progress",
        StepStatus::Completed => "completed",
    }
}

#[cfg(test)]
mod tests {
    use super::PlanDisplayMode;
    use super::project_plan_items;
    use super::render_plan_markdown;
    use codex_protocol::plan_tool::PlanItemArg;
    use codex_protocol::plan_tool::StepStatus;
    use pretty_assertions::assert_eq;

    #[test]
    fn projects_tree_numbers_and_layers() {
        let projection = project_plan_items(&[
            sample_item("plan-01", "Root A", StepStatus::Completed, &[]),
            sample_item("plan-02", "Child A", StepStatus::InProgress, &["plan-01"]),
            sample_item("plan-03", "Root B", StepStatus::Pending, &[]),
            sample_item("plan-04", "Child B", StepStatus::Pending, &["plan-03"]),
        ]);

        assert_eq!(projection.mode, PlanDisplayMode::Tree);
        assert_eq!(
            projection
                .rows
                .iter()
                .map(|row| row.display_number.clone())
                .collect::<Vec<_>>(),
            vec![
                Some("1".to_string()),
                Some("1.1".to_string()),
                Some("2".to_string()),
                Some("2.1".to_string()),
            ]
        );
        assert_eq!(
            projection
                .rows
                .iter()
                .map(|row| row.layer)
                .collect::<Vec<_>>(),
            vec![Some(0), Some(1), Some(0), Some(1)]
        );
        assert_eq!(projection.layers.len(), 2);
        assert_eq!(projection.layers[0].layer_index, 0);
        assert_eq!(projection.layers[1].layer_index, 1);
    }

    #[test]
    fn projects_multi_parent_rows_using_first_dependency_as_display_parent() {
        let projection = project_plan_items(&[
            sample_item("plan-01", "Root A", StepStatus::Completed, &[]),
            sample_item("plan-02", "Root B", StepStatus::Completed, &[]),
            sample_item(
                "plan-03",
                "Join work",
                StepStatus::Pending,
                &["plan-01", "plan-02"],
            ),
        ]);

        assert_eq!(projection.mode, PlanDisplayMode::Tree);
        assert_eq!(projection.rows[2].display_number.as_deref(), Some("1.1"));
        assert_eq!(
            projection.rows[2].primary_parent_id.as_deref(),
            Some("plan-01")
        );
        assert_eq!(projection.rows[2].additional_dependencies, vec!["plan-02"]);
        assert_eq!(projection.rows[2].depends_on, vec!["plan-01", "plan-02"]);
        assert_eq!(projection.rows[2].layer, Some(1));
    }

    #[test]
    fn falls_back_to_flat_projection_when_dependencies_are_not_row_ordered() {
        let projection = project_plan_items(&[
            sample_item(
                "plan-01",
                "Late dependency",
                StepStatus::Pending,
                &["plan-02"],
            ),
            sample_item("plan-02", "Future row", StepStatus::Pending, &[]),
        ]);

        assert_eq!(projection.mode, PlanDisplayMode::Flat);
        assert_eq!(
            projection.compatibility_note.as_deref(),
            Some(
                "Legacy dependency layout detected; showing a flat list instead of a dependency tree."
            )
        );
        assert!(projection.layers.is_empty());
        assert_eq!(projection.rows[0].display_number, None);
    }

    #[test]
    fn renders_markdown_tree_and_parallel_layers() {
        let markdown = render_plan_markdown(&[
            sample_item("plan-01", "Root A", StepStatus::Completed, &[]),
            sample_item("plan-02", "Child A", StepStatus::InProgress, &["plan-01"]),
        ]);

        assert_eq!(
            markdown,
            "\
# Plan

## Dependency Tree

- [completed] 1 Root A (`plan-01`; `src/root.rs`)
  inputs: input for plan-01
  outputs: output for plan-01
  acceptance: done for plan-01
  - [in_progress] 1.1 Child A (`plan-02`; `src/root.rs`)
    inputs: input for plan-02
    outputs: output for plan-02
    acceptance: done for plan-02

## Parallel Layers

- L0: 1 (`plan-01`)
- L1: 1.1 (`plan-02`)
"
        );
    }

    fn sample_item(
        row_id: &str,
        step: &str,
        status: StepStatus,
        depends_on: &[&str],
    ) -> PlanItemArg {
        PlanItemArg {
            id: Some(row_id.to_string()),
            step: step.to_string(),
            status,
            path: Some("src/root.rs".to_string()),
            details: None,
            inputs: Some(vec![format!("input for {row_id}")]),
            outputs: Some(vec![format!("output for {row_id}")]),
            depends_on: (!depends_on.is_empty())
                .then(|| depends_on.iter().map(|value| value.to_string()).collect()),
            acceptance: Some(format!("done for {row_id}")),
        }
    }
}
