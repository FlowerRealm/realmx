use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::function_tool::FunctionCallError;
use crate::plan_csv::render_plan_csv_markdown;
use crate::plan_csv::update_plan_from_thread_plan_items;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::JsonSchema;
use async_trait::async_trait;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::protocol::EventMsg;
use std::collections::BTreeMap;
use std::sync::LazyLock;

pub struct PlanHandler;

pub static PLAN_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut plan_item_props = BTreeMap::new();
    plan_item_props.insert(
        "id".to_string(),
        JsonSchema::String {
            description: Some("Stable row id for reusing an existing active plan item".to_string()),
        },
    );
    plan_item_props.insert("step".to_string(), JsonSchema::String { description: None });
    plan_item_props.insert(
        "status".to_string(),
        JsonSchema::String {
            description: Some("One of: pending, in_progress, completed".to_string()),
        },
    );
    plan_item_props.insert(
        "path".to_string(),
        JsonSchema::String {
            description: Some("Repo path owned by this plan row".to_string()),
        },
    );
    plan_item_props.insert(
        "details".to_string(),
        JsonSchema::String {
            description: Some("Optional implementation notes for this row".to_string()),
        },
    );
    plan_item_props.insert(
        "inputs".to_string(),
        JsonSchema::Array {
            description: Some("Optional structured inputs consumed by this row".to_string()),
            items: Box::new(JsonSchema::String { description: None }),
        },
    );
    plan_item_props.insert(
        "outputs".to_string(),
        JsonSchema::Array {
            description: Some("Optional structured outputs produced by this row".to_string()),
            items: Box::new(JsonSchema::String { description: None }),
        },
    );
    plan_item_props.insert(
        "depends_on".to_string(),
        JsonSchema::Array {
            description: Some("Optional row ids that must complete before this row".to_string()),
            items: Box::new(JsonSchema::String { description: None }),
        },
    );
    plan_item_props.insert(
        "acceptance".to_string(),
        JsonSchema::String {
            description: Some("Optional completion criteria for this row".to_string()),
        },
    );

    let plan_items_schema = JsonSchema::Array {
        description: Some("The list of steps".to_string()),
        items: Box::new(JsonSchema::Object {
            properties: plan_item_props,
            required: Some(vec!["step".to_string(), "status".to_string()]),
            additional_properties: Some(false.into()),
        }),
    };

    let mut properties = BTreeMap::new();
    properties.insert(
        "explanation".to_string(),
        JsonSchema::String { description: None },
    );
    properties.insert("plan".to_string(), plan_items_schema);

    ToolSpec::Function(ResponsesApiTool {
        name: "update_plan".to_string(),
        description: r#"Updates the task plan.
Provide an optional explanation and a list of plan items.
Each item must include step and status, and may also include id/path/details/inputs/outputs/depends_on/acceptance.
At most one step can be in_progress at a time.
"#
        .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["plan".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
});

#[async_trait]
impl ToolHandler for PlanHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "update_plan handler received unsupported payload".to_string(),
                ));
            }
        };

        let content =
            handle_update_plan(session.as_ref(), turn.as_ref(), arguments, call_id).await?;

        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

/// This function doesn't do anything useful. However, it gives the model a structured way to record its plan that clients can read and render.
/// So it's the _inputs_ to this function that are useful to clients, not the outputs and neither are actually useful for the model other
/// than forcing it to come up and document a plan (TBD how that affects performance).
pub(crate) async fn handle_update_plan(
    session: &Session,
    turn_context: &TurnContext,
    arguments: String,
    _call_id: String,
) -> Result<String, FunctionCallError> {
    if turn_context.collaboration_mode.mode.is_plan_output_mode() {
        return Err(FunctionCallError::RespondToModel(
            "update_plan is a TODO/checklist tool and is not allowed in Plan output modes"
                .to_string(),
        ));
    }
    let args = parse_update_plan_arguments(&arguments)?;
    if let Some(active_plan) = try_update_active_thread_plan(session, &args).await? {
        session
            .send_event(turn_context, EventMsg::PlanUpdate(active_plan))
            .await;
        return Ok("Plan updated".to_string());
    }
    session
        .send_event(turn_context, EventMsg::PlanUpdate(args))
        .await;
    Ok("Plan updated".to_string())
}

fn parse_update_plan_arguments(arguments: &str) -> Result<UpdatePlanArgs, FunctionCallError> {
    serde_json::from_str::<UpdatePlanArgs>(arguments).map_err(|e| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {e}"))
    })
}

async fn try_update_active_thread_plan(
    session: &Session,
    args: &UpdatePlanArgs,
) -> Result<Option<UpdatePlanArgs>, FunctionCallError> {
    let Some(state_db) = session.state_db() else {
        return Ok(None);
    };
    let thread_id = session.conversation_id.to_string();
    let Some(active_plan) = state_db
        .get_active_thread_plan(thread_id.as_str())
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to load active thread plan: {err}"))
        })?
    else {
        return Ok(None);
    };

    match classify_active_plan_update(args, &active_plan)? {
        ActivePlanUpdate::StatusOnly(status_updates) => {
            let refreshed = state_db
                .update_active_thread_plan_item_statuses(
                    thread_id.as_str(),
                    status_updates.as_slice(),
                )
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to update active thread plan: {err}"
                    ))
                })?
                .unwrap_or(active_plan);
            Ok(Some(update_plan_from_thread_plan_items(
                refreshed.items.as_slice(),
                args.explanation.clone(),
            )))
        }
        ActivePlanUpdate::Replace(items) => {
            let raw_markdown = render_plan_csv_markdown(items.as_slice()).map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to render active thread plan csv: {err}"
                ))
            })?;
            let refreshed = state_db
                .replace_active_thread_plan(
                    &codex_state::ThreadPlanSnapshotCreateParams {
                        id: uuid::Uuid::new_v4().to_string(),
                        thread_id: thread_id.clone(),
                        source_turn_id: active_plan.snapshot.source_turn_id.clone(),
                        source_item_id: active_plan.snapshot.source_item_id.clone(),
                        raw_markdown,
                    },
                    items.as_slice(),
                )
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to replace active thread plan: {err}"
                    ))
                })?;
            Ok(Some(update_plan_from_thread_plan_items(
                refreshed.items.as_slice(),
                args.explanation.clone(),
            )))
        }
        ActivePlanUpdate::Clear => {
            state_db
                .clear_active_thread_plan(thread_id.as_str())
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to clear active thread plan: {err}"
                    ))
                })?;
            Ok(Some(UpdatePlanArgs {
                explanation: args.explanation.clone(),
                plan: Vec::new(),
            }))
        }
    }
}

#[derive(Debug)]
enum ActivePlanUpdate {
    StatusOnly(Vec<(String, codex_state::ThreadPlanItemStatus)>),
    Replace(Vec<codex_state::ThreadPlanItemCreateParams>),
    Clear,
}

fn classify_active_plan_update(
    args: &UpdatePlanArgs,
    active_plan: &codex_state::ActiveThreadPlan,
) -> Result<ActivePlanUpdate, FunctionCallError> {
    if args.plan.is_empty() {
        return Ok(ActivePlanUpdate::Clear);
    }

    let replacement_items = active_plan_replacement_items(args, active_plan)?;
    if args.plan.len() != active_plan.items.len() {
        return Ok(ActivePlanUpdate::Replace(replacement_items));
    }

    let mut status_updates = Vec::with_capacity(active_plan.items.len());
    let mut in_progress_count = 0usize;

    for (item, existing) in args.plan.iter().zip(active_plan.items.iter()) {
        if replacement_item_matches_existing(item, existing) {
            let status = step_status_to_thread_plan_status(item.status.clone());
            if matches!(status, codex_state::ThreadPlanItemStatus::InProgress) {
                in_progress_count = in_progress_count.saturating_add(1);
            }

            status_updates.push((existing.row_id.clone(), status));
        } else {
            return Ok(ActivePlanUpdate::Replace(replacement_items));
        }
    }

    if in_progress_count > 1 {
        return Err(FunctionCallError::RespondToModel(
            "active thread plan may include at most one in_progress row".to_string(),
        ));
    }

    Ok(ActivePlanUpdate::StatusOnly(status_updates))
}

fn active_plan_replacement_items(
    args: &UpdatePlanArgs,
    active_plan: &codex_state::ActiveThreadPlan,
) -> Result<Vec<codex_state::ThreadPlanItemCreateParams>, FunctionCallError> {
    let existing_rows = active_plan
        .items
        .iter()
        .map(|item| (item.row_id.as_str(), item))
        .collect::<std::collections::HashMap<_, _>>();
    let mut replacement_items = Vec::with_capacity(args.plan.len());
    let mut in_progress_count = 0usize;
    let mut seen_row_ids = std::collections::HashSet::with_capacity(args.plan.len());

    for (index, item) in args.plan.iter().enumerate() {
        let status = step_status_to_thread_plan_status(item.status.clone());
        if matches!(status, codex_state::ThreadPlanItemStatus::InProgress) {
            in_progress_count = in_progress_count.saturating_add(1);
        }

        let existing = matching_existing_plan_item(item, index, active_plan, &existing_rows);
        let row_id = item
            .id
            .clone()
            .or_else(|| existing.map(|value| value.row_id.clone()))
            .unwrap_or_else(|| format!("plan-{:02}", index + 1));
        if !seen_row_ids.insert(row_id.clone()) {
            return Err(FunctionCallError::RespondToModel(format!(
                "duplicate active thread plan row id: {row_id}"
            )));
        }

        let path = item
            .path
            .clone()
            .or_else(|| existing.map(|value| value.path.clone()))
            .unwrap_or_default();
        if path.is_empty() {
            return Err(FunctionCallError::RespondToModel(format!(
                "active thread plan row {row_id} is missing path"
            )));
        }

        replacement_items.push(codex_state::ThreadPlanItemCreateParams {
            row_id,
            row_index: index as i64,
            status,
            step: item.step.clone(),
            path,
            details: item
                .details
                .clone()
                .or_else(|| existing.map(|value| value.details.clone()))
                .unwrap_or_default(),
            inputs: item
                .inputs
                .clone()
                .or_else(|| existing.map(|value| value.inputs.clone()))
                .unwrap_or_default(),
            outputs: item
                .outputs
                .clone()
                .or_else(|| existing.map(|value| value.outputs.clone()))
                .unwrap_or_default(),
            depends_on: item
                .depends_on
                .clone()
                .or_else(|| existing.map(|value| value.depends_on.clone()))
                .unwrap_or_default(),
            acceptance: item
                .acceptance
                .clone()
                .or_else(|| existing.and_then(|value| value.acceptance.clone())),
        });
    }

    if in_progress_count > 1 {
        return Err(FunctionCallError::RespondToModel(
            "active thread plan may include at most one in_progress row".to_string(),
        ));
    }
    validate_dependencies(replacement_items.as_slice())?;

    Ok(replacement_items)
}

fn validate_dependencies(
    items: &[codex_state::ThreadPlanItemCreateParams],
) -> Result<(), FunctionCallError> {
    let row_ids = items
        .iter()
        .map(|item| item.row_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    for item in items {
        for dependency in &item.depends_on {
            if dependency == &item.row_id {
                return Err(FunctionCallError::RespondToModel(format!(
                    "active thread plan row {} cannot depend on itself",
                    item.row_id
                )));
            }
            if !row_ids.contains(dependency.as_str()) {
                return Err(FunctionCallError::RespondToModel(format!(
                    "active thread plan row {} depends on unknown id: {}",
                    item.row_id, dependency
                )));
            }
        }
    }
    Ok(())
}

fn replacement_item_matches_existing(
    item: &codex_protocol::plan_tool::PlanItemArg,
    existing: &codex_state::ThreadPlanItem,
) -> bool {
    item.id
        .as_deref()
        .is_none_or(|row_id| row_id == existing.row_id.as_str())
        && item.step == existing.step
        && metadata_matches(item.path.as_ref(), existing.path.as_str())
        && metadata_matches(item.details.as_ref(), existing.details.as_str())
        && collection_matches(item.inputs.as_ref(), existing.inputs.as_slice())
        && collection_matches(item.outputs.as_ref(), existing.outputs.as_slice())
        && collection_matches(item.depends_on.as_ref(), existing.depends_on.as_slice())
        && optional_string_matches(item.acceptance.as_ref(), existing.acceptance.as_ref())
}

fn matching_existing_plan_item<'a>(
    item: &codex_protocol::plan_tool::PlanItemArg,
    index: usize,
    active_plan: &'a codex_state::ActiveThreadPlan,
    existing_rows: &std::collections::HashMap<&str, &'a codex_state::ThreadPlanItem>,
) -> Option<&'a codex_state::ThreadPlanItem> {
    if let Some(row_id) = item.id.as_deref() {
        return existing_rows.get(row_id).copied();
    }

    active_plan
        .items
        .get(index)
        .filter(|existing| existing.step == item.step)
}

fn metadata_matches(value: Option<&String>, existing: &str) -> bool {
    value.is_none_or(|value| value == existing)
}

fn collection_matches(value: Option<&Vec<String>>, existing: &[String]) -> bool {
    value.is_none_or(|value| value.as_slice() == existing)
}

fn optional_string_matches(value: Option<&String>, existing: Option<&String>) -> bool {
    value.is_none_or(|value| Some(value) == existing)
}

fn step_status_to_thread_plan_status(status: StepStatus) -> codex_state::ThreadPlanItemStatus {
    match status {
        StepStatus::Pending => codex_state::ThreadPlanItemStatus::Pending,
        StepStatus::InProgress => codex_state::ThreadPlanItemStatus::InProgress,
        StepStatus::Completed => codex_state::ThreadPlanItemStatus::Completed,
    }
}

#[cfg(test)]
mod tests {
    use super::ActivePlanUpdate;
    use super::classify_active_plan_update;
    use crate::function_tool::FunctionCallError;
    use codex_protocol::plan_tool::PlanItemArg;
    use codex_protocol::plan_tool::StepStatus;
    use codex_protocol::plan_tool::UpdatePlanArgs;
    use codex_state::ActiveThreadPlan;
    use codex_state::ThreadPlanItem;
    use codex_state::ThreadPlanItemStatus;
    use codex_state::ThreadPlanSnapshot;
    use pretty_assertions::assert_eq;

    fn active_plan() -> ActiveThreadPlan {
        ActiveThreadPlan {
            snapshot: ThreadPlanSnapshot {
                id: "snapshot-1".to_string(),
                thread_id: "thread-1".to_string(),
                source_turn_id: "turn-1".to_string(),
                source_item_id: "item-1".to_string(),
                raw_markdown: "plan".to_string(),
                created_at: chrono::Utc::now(),
                superseded_at: None,
            },
            items: vec![
                ThreadPlanItem {
                    snapshot_id: "snapshot-1".to_string(),
                    row_id: "plan-01".to_string(),
                    row_index: 0,
                    status: ThreadPlanItemStatus::InProgress,
                    step: "First".to_string(),
                    path: "a.rs".to_string(),
                    details: String::new(),
                    inputs: Vec::new(),
                    outputs: vec!["out".to_string()],
                    depends_on: Vec::new(),
                    acceptance: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    completed_at: None,
                },
                ThreadPlanItem {
                    snapshot_id: "snapshot-1".to_string(),
                    row_id: "plan-02".to_string(),
                    row_index: 1,
                    status: ThreadPlanItemStatus::Pending,
                    step: "Second".to_string(),
                    path: "b.rs".to_string(),
                    details: "details".to_string(),
                    inputs: vec!["in".to_string()],
                    outputs: Vec::new(),
                    depends_on: vec!["plan-01".to_string()],
                    acceptance: Some("done".to_string()),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    completed_at: None,
                },
            ],
        }
    }

    #[test]
    fn status_only_updates_accept_unchanged_metadata() {
        let active_plan = active_plan();
        let args = UpdatePlanArgs {
            explanation: Some("update".to_string()),
            plan: vec![
                PlanItemArg {
                    id: Some("plan-01".to_string()),
                    step: "First".to_string(),
                    status: StepStatus::Completed,
                    path: Some("a.rs".to_string()),
                    details: None,
                    inputs: None,
                    outputs: Some(vec!["out".to_string()]),
                    depends_on: None,
                    acceptance: None,
                },
                PlanItemArg {
                    id: Some("plan-02".to_string()),
                    step: "Second".to_string(),
                    status: StepStatus::InProgress,
                    path: Some("b.rs".to_string()),
                    details: Some("details".to_string()),
                    inputs: Some(vec!["in".to_string()]),
                    outputs: None,
                    depends_on: Some(vec!["plan-01".to_string()]),
                    acceptance: Some("done".to_string()),
                },
            ],
        };

        let ActivePlanUpdate::StatusOnly(updated) =
            classify_active_plan_update(&args, &active_plan)
                .expect("status-only update should validate")
        else {
            panic!("status-only update should be recognized");
        };
        assert_eq!(updated.len(), 2);
        assert_eq!(
            updated,
            vec![
                ("plan-01".to_string(), ThreadPlanItemStatus::Completed),
                ("plan-02".to_string(), ThreadPlanItemStatus::InProgress),
            ]
        );
    }

    #[test]
    fn status_only_updates_reject_multiple_in_progress_rows() {
        let active_plan = active_plan();
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![
                PlanItemArg {
                    id: Some("plan-01".to_string()),
                    step: "First".to_string(),
                    status: StepStatus::InProgress,
                    path: Some("a.rs".to_string()),
                    details: None,
                    inputs: None,
                    outputs: Some(vec!["out".to_string()]),
                    depends_on: None,
                    acceptance: None,
                },
                PlanItemArg {
                    id: Some("plan-02".to_string()),
                    step: "Second".to_string(),
                    status: StepStatus::InProgress,
                    path: Some("b.rs".to_string()),
                    details: Some("details".to_string()),
                    inputs: Some(vec!["in".to_string()]),
                    outputs: None,
                    depends_on: Some(vec!["plan-01".to_string()]),
                    acceptance: Some("done".to_string()),
                },
            ],
        };

        let err = classify_active_plan_update(&args, &active_plan)
            .expect_err("multiple in_progress rows should fail");
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "active thread plan may include at most one in_progress row".to_string()
            )
        );
    }

    #[test]
    fn status_only_updates_fall_back_when_metadata_changes() {
        let active_plan = active_plan();
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![
                PlanItemArg {
                    id: Some("plan-01".to_string()),
                    step: "First".to_string(),
                    status: StepStatus::Completed,
                    path: Some("renamed.rs".to_string()),
                    details: None,
                    inputs: None,
                    outputs: Some(vec!["out".to_string()]),
                    depends_on: None,
                    acceptance: None,
                },
                PlanItemArg {
                    id: Some("plan-02".to_string()),
                    step: "Second".to_string(),
                    status: StepStatus::Pending,
                    path: Some("b.rs".to_string()),
                    details: Some("details".to_string()),
                    inputs: Some(vec!["in".to_string()]),
                    outputs: None,
                    depends_on: Some(vec!["plan-01".to_string()]),
                    acceptance: Some("done".to_string()),
                },
            ],
        };

        let ActivePlanUpdate::Replace(updated) = classify_active_plan_update(&args, &active_plan)
            .expect("metadata change should not error")
        else {
            panic!("metadata change should replace the active plan");
        };
        assert_eq!(updated[0].path, "renamed.rs");
        assert_eq!(updated[1].depends_on, vec!["plan-01".to_string()]);
    }

    #[test]
    fn status_only_updates_allow_omitted_optional_metadata() {
        let active_plan = active_plan();
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![
                PlanItemArg {
                    id: Some("plan-01".to_string()),
                    step: "First".to_string(),
                    status: StepStatus::Completed,
                    path: None,
                    details: None,
                    inputs: None,
                    outputs: None,
                    depends_on: None,
                    acceptance: None,
                },
                PlanItemArg {
                    id: Some("plan-02".to_string()),
                    step: "Second".to_string(),
                    status: StepStatus::InProgress,
                    path: None,
                    details: None,
                    inputs: None,
                    outputs: None,
                    depends_on: None,
                    acceptance: None,
                },
            ],
        };

        let ActivePlanUpdate::StatusOnly(updated) =
            classify_active_plan_update(&args, &active_plan)
                .expect("omitted metadata should still validate")
        else {
            panic!("omitted metadata should stay on the status-only path");
        };
        assert_eq!(
            updated,
            vec![
                ("plan-01".to_string(), ThreadPlanItemStatus::Completed),
                ("plan-02".to_string(), ThreadPlanItemStatus::InProgress),
            ]
        );
    }

    #[test]
    fn status_only_updates_allow_omitted_ids_for_legacy_calls() {
        let active_plan = active_plan();
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![
                PlanItemArg {
                    id: None,
                    step: "First".to_string(),
                    status: StepStatus::Completed,
                    path: None,
                    details: None,
                    inputs: None,
                    outputs: None,
                    depends_on: None,
                    acceptance: None,
                },
                PlanItemArg {
                    id: None,
                    step: "Second".to_string(),
                    status: StepStatus::InProgress,
                    path: None,
                    details: None,
                    inputs: None,
                    outputs: None,
                    depends_on: None,
                    acceptance: None,
                },
            ],
        };

        let ActivePlanUpdate::StatusOnly(updated) =
            classify_active_plan_update(&args, &active_plan)
                .expect("legacy status-only update should validate without ids")
        else {
            panic!("legacy status-only update should stay on the status-only path");
        };
        assert_eq!(
            updated,
            vec![
                ("plan-01".to_string(), ThreadPlanItemStatus::Completed),
                ("plan-02".to_string(), ThreadPlanItemStatus::InProgress),
            ]
        );
    }

    #[test]
    fn structural_changes_replace_active_plan_while_reusing_known_metadata() {
        let active_plan = active_plan();
        let args = UpdatePlanArgs {
            explanation: Some("restructured".to_string()),
            plan: vec![
                PlanItemArg {
                    id: Some("plan-01".to_string()),
                    step: "First".to_string(),
                    status: StepStatus::Completed,
                    path: None,
                    details: None,
                    inputs: None,
                    outputs: None,
                    depends_on: None,
                    acceptance: None,
                },
                PlanItemArg {
                    id: Some("plan-03".to_string()),
                    step: "Third".to_string(),
                    status: StepStatus::InProgress,
                    path: Some("c.rs".to_string()),
                    details: Some("new".to_string()),
                    inputs: Some(vec!["handoff".to_string()]),
                    outputs: None,
                    depends_on: Some(vec!["plan-01".to_string()]),
                    acceptance: Some("done".to_string()),
                },
            ],
        };

        let ActivePlanUpdate::Replace(updated) = classify_active_plan_update(&args, &active_plan)
            .expect("structural change should validate")
        else {
            panic!("structural change should replace the active plan");
        };
        assert_eq!(updated.len(), 2);
        assert_eq!(updated[0].row_id, "plan-01");
        assert_eq!(updated[0].path, "a.rs");
        assert_eq!(updated[0].outputs, vec!["out".to_string()]);
        assert_eq!(updated[1].row_id, "plan-03");
        assert_eq!(updated[1].path, "c.rs");
        assert_eq!(updated[1].depends_on, vec!["plan-01".to_string()]);
    }

    #[test]
    fn structural_changes_without_ids_do_not_reuse_mismatched_row_metadata() {
        let active_plan = active_plan();
        let args = UpdatePlanArgs {
            explanation: Some("restructured".to_string()),
            plan: vec![
                PlanItemArg {
                    id: None,
                    step: "Second".to_string(),
                    status: StepStatus::Completed,
                    path: None,
                    details: None,
                    inputs: None,
                    outputs: None,
                    depends_on: None,
                    acceptance: None,
                },
                PlanItemArg {
                    id: None,
                    step: "Third".to_string(),
                    status: StepStatus::InProgress,
                    path: Some("c.rs".to_string()),
                    details: Some("new".to_string()),
                    inputs: Some(vec!["handoff".to_string()]),
                    outputs: None,
                    depends_on: Some(vec!["plan-01".to_string()]),
                    acceptance: Some("done".to_string()),
                },
            ],
        };

        let err = classify_active_plan_update(&args, &active_plan)
            .expect_err("reordered rows without ids should not inherit mismatched metadata");
        assert_eq!(
            err,
            FunctionCallError::RespondToModel(
                "active thread plan row plan-01 is missing path".to_string()
            )
        );
    }

    #[test]
    fn empty_plan_clears_active_plan() {
        let active_plan = active_plan();
        let args = UpdatePlanArgs {
            explanation: Some("done".to_string()),
            plan: Vec::new(),
        };

        assert!(matches!(
            classify_active_plan_update(&args, &active_plan)
                .expect("empty plan should clear the active plan"),
            ActivePlanUpdate::Clear
        ));
    }
}
