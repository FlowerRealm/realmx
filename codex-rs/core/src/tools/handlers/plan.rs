use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::execute_plan_guard::validate_execute_mode_plan_update;
use crate::features::Feature;
use crate::function_tool::FunctionCallError;
use crate::plan_csv::canonical_plan_csv_from_update_plan_args;
use crate::plan_csv::canonical_plan_csv_from_update_plan_args_for_authoring;
use crate::plan_csv::update_plan_from_thread_plan_items;
use crate::plan_workspace::PlanWorkspace;
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
use codex_state::ThreadPlanSnapshotCreateParams;
use std::collections::BTreeMap;
use std::sync::LazyLock;
use uuid::Uuid;

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
When creating or replacing a plan, prefer file-level rows with path/details and reuse existing ids for follow-up status updates.
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
    call_id: String,
) -> Result<String, FunctionCallError> {
    if turn_context.collaboration_mode.is_plan_mode()
        || turn_context
            .collaboration_mode
            .is_ultra_work_planning_mode()
    {
        return Err(FunctionCallError::RespondToModel(
            "update_plan is a TODO/checklist tool and is not allowed in Plan or Ultra Work planning"
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

async fn try_handle_execute_mode_active_plan_update(
    session: &Session,
    turn_context: &TurnContext,
    args: &mut UpdatePlanArgs,
) -> Result<bool, FunctionCallError> {
    if !turn_context
        .collaboration_mode
        .is_ultra_work_execution_mode()
    {
        return Ok(false);
    }
    let Some(state_db) = session.state_db() else {
        return Ok(false);
    };

    let thread_id = session.conversation_id.to_string();
    let Some(active_plan) = state_db
        .get_active_thread_plan(thread_id.as_str())
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to load active thread plan: {err}"))
        })?
    else {
        return Ok(false);
    };

    if session.enabled(Feature::PlanWorkflow) {
        return Err(FunctionCallError::RespondToModel(
            "Ultra Work execution active plan rows are managed by execute_active_plan_with_subagents while automatic dispatch is enabled".to_string(),
        ));
    }

    validate_execute_mode_plan_update(active_plan.items.as_slice(), args)
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

fn parse_update_plan_arguments(arguments: &str) -> Result<UpdatePlanArgs, FunctionCallError> {
    serde_json::from_str::<UpdatePlanArgs>(arguments).map_err(|e| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {e}"))
    })
}

async fn try_update_active_thread_plan(
    session: &Session,
    turn_context: &TurnContext,
    args: &mut UpdatePlanArgs,
) -> Result<bool, FunctionCallError> {
    let Some(state_db) = session.state_db() else {
        return Ok(false);
    };
    let thread_id = session.conversation_id.to_string();
    let Some(active_plan) = state_db
        .get_active_thread_plan(thread_id.as_str())
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to load active thread plan: {err}"))
        })?
    else {
        return Ok(false);
    };

    if let Some(missing_row_id) = args
        .plan
        .iter()
        .filter_map(|item| item.id.as_deref())
        .find(|row_id| !active_plan.items.iter().any(|item| item.row_id == *row_id))
    {
        return Err(FunctionCallError::RespondToModel(format!(
            "failed to update active thread plan row {missing_row_id}: active thread plan row not found: {missing_row_id}"
        )));
    }

    let mut updated = false;
    for item in &args.plan {
        let Some(row_id) = item.id.as_deref() else {
            continue;
        };
        state_db
            .update_active_thread_plan_item_status(
                thread_id.as_str(),
                row_id,
                step_status_to_thread_plan_status(item.status.clone()),
            )
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to update active thread plan row {row_id}: {err}"
                ))
            })?;
        updated = true;
    }
    if !updated {
        return Ok(false);
    }
    let refreshed = state_db
        .get_active_thread_plan(thread_id.as_str())
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to refresh active thread plan: {err}"
            ))
        })?
        .unwrap_or(active_plan);
    let refreshed_args =
        update_plan_from_thread_plan_items(refreshed.items.as_slice(), args.explanation.clone());
    let refreshed_rows = refreshed
        .items
        .iter()
        .map(|item| codex_state::ThreadPlanItemCreateParams {
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
        .collect::<Vec<_>>();
    let refreshed_csv =
        codex_state::render_thread_plan_csv(refreshed_rows.as_slice()).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to render refreshed active thread plan csv: {err}"
            ))
        })?;
    let codex_home = session.codex_home().await;
    let workspace = PlanWorkspace::new(
        codex_home.as_path(),
        turn_context.cwd.as_path(),
        thread_id.as_str(),
    );
    let update_public_draft = workspace.draft_matches_active().await.map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to compare plan workspace draft and active plan: {err}"
        ))
    })?;
    workspace
        .persist_active_plan(refreshed_csv.as_str(), update_public_draft)
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to sync refreshed active thread plan to workspace: {err}"
            ))
        })?;
    for item in &mut args.plan {
        let Some(row_id) = item.id.as_deref() else {
            continue;
        };
        if let Some(refreshed_item) = refreshed_args
            .plan
            .iter()
            .find(|candidate| candidate.id.as_deref() == Some(row_id))
        {
            *item = refreshed_item.clone();
        }
    }
    Ok(true)
}

async fn try_sync_active_thread_plan(
    session: &Session,
    turn_context: &TurnContext,
    args: &mut UpdatePlanArgs,
    call_id: &str,
) -> Result<bool, FunctionCallError> {
    if !session.enabled(Feature::PlanWorkflow) {
        return try_update_active_thread_plan(session, turn_context, args).await;
    }

    let Some(state_db) = session.state_db() else {
        return Ok(false);
    };
    let thread_id = session.conversation_id.to_string();
    let active_plan = state_db
        .get_active_thread_plan(thread_id.as_str())
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to load active thread plan: {err}"))
        })?;

    if let Some(active_plan) = active_plan.filter(|plan| update_plan_is_status_patch(plan, args)) {
        update_existing_active_thread_plan(
            session,
            turn_context,
            args,
            thread_id.as_str(),
            active_plan,
        )
        .await?;
        return Ok(true);
    }

    replace_active_thread_plan(session, turn_context, args, thread_id.as_str(), call_id).await?;
    Ok(true)
}

fn update_plan_is_status_patch(
    active_plan: &codex_state::ActiveThreadPlan,
    args: &UpdatePlanArgs,
) -> bool {
    !args.plan.is_empty()
        && args.plan.iter().all(|item| {
            item.id
                .as_deref()
                .is_some_and(|row_id| active_plan.items.iter().any(|row| row.row_id == row_id))
        })
}

async fn update_existing_active_thread_plan(
    session: &Session,
    turn_context: &TurnContext,
    args: &mut UpdatePlanArgs,
    thread_id: &str,
    active_plan: codex_state::ActiveThreadPlan,
) -> Result<(), FunctionCallError> {
    let state_db = session
        .state_db()
        .ok_or_else(|| FunctionCallError::RespondToModel("state db unavailable".to_string()))?;
    for item in &args.plan {
        let row_id = item.id.as_deref().ok_or_else(|| {
            FunctionCallError::RespondToModel("expected row id for active plan update".to_string())
        })?;
        state_db
            .update_active_thread_plan_item_status(
                thread_id,
                row_id,
                step_status_to_thread_plan_status(item.status.clone()),
            )
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to update active thread plan row {row_id}: {err}"
                ))
            })?;
    }

    let refreshed = state_db
        .get_active_thread_plan(thread_id)
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to refresh active thread plan: {err}"
            ))
        })?
        .unwrap_or(active_plan);
    persist_active_plan_to_workspace(session, turn_context, thread_id, refreshed.items.as_slice())
        .await?;
    *args =
        update_plan_from_thread_plan_items(refreshed.items.as_slice(), args.explanation.clone());
    Ok(())
}

async fn replace_active_thread_plan(
    session: &Session,
    turn_context: &TurnContext,
    args: &mut UpdatePlanArgs,
    thread_id: &str,
    call_id: &str,
) -> Result<(), FunctionCallError> {
    let canonical_plan =
        canonical_plan_csv_from_update_plan_args_for_authoring(args).map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to canonicalize plan update: {err}"))
        })?;
    let state_db = session
        .state_db()
        .ok_or_else(|| FunctionCallError::RespondToModel("state db unavailable".to_string()))?;
    let active_plan = state_db
        .replace_active_thread_plan(&ThreadPlanSnapshotCreateParams {
            id: Uuid::new_v4().to_string(),
            thread_id: thread_id.to_string(),
            source_turn_id: turn_context.sub_id.clone(),
            source_item_id: call_id.to_string(),
            raw_csv: canonical_plan.raw_csv,
        })
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to replace active thread plan: {err}"
            ))
        })?;
    persist_active_plan_to_workspace(
        session,
        turn_context,
        thread_id,
        active_plan.items.as_slice(),
    )
    .await?;
    *args =
        update_plan_from_thread_plan_items(active_plan.items.as_slice(), args.explanation.clone());
    Ok(())
}

async fn persist_active_plan_to_workspace(
    session: &Session,
    turn_context: &TurnContext,
    thread_id: &str,
    items: &[codex_state::ThreadPlanItem],
) -> Result<(), FunctionCallError> {
    let refreshed_args = update_plan_from_thread_plan_items(items, None);
    let refreshed_csv = canonical_plan_csv_from_update_plan_args(&refreshed_args)
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to render refreshed active thread plan csv: {err}"
            ))
        })?
        .raw_csv;
    let codex_home = session.codex_home().await;
    let workspace = PlanWorkspace::new(codex_home.as_path(), turn_context.cwd.as_path(), thread_id);
    let update_public_draft = workspace.draft_matches_active().await.map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to compare plan workspace draft and active plan: {err}"
        ))
    })?;
    workspace
        .persist_active_plan(refreshed_csv.as_str(), update_public_draft)
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to sync refreshed active thread plan to workspace: {err}"
            ))
        })?;
    Ok(())
}

fn step_status_to_thread_plan_status(status: StepStatus) -> codex_state::ThreadPlanItemStatus {
    match status {
        StepStatus::Pending => codex_state::ThreadPlanItemStatus::Pending,
        StepStatus::InProgress => codex_state::ThreadPlanItemStatus::InProgress,
        StepStatus::Completed => codex_state::ThreadPlanItemStatus::Completed,
    }
}
