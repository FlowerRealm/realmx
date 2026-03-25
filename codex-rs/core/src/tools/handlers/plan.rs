use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::function_tool::FunctionCallError;
use crate::plan_csv::update_plan_from_thread_plan_items;
use crate::plan_workspace::PlanWorkspace;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::JsonSchema;
use async_trait::async_trait;
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
    let mut args = parse_update_plan_arguments(&arguments)?;
    if try_update_active_thread_plan(session, turn_context, &mut args).await? {
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
                match item.status {
                    codex_protocol::plan_tool::StepStatus::Pending => {
                        codex_state::ThreadPlanItemStatus::Pending
                    }
                    codex_protocol::plan_tool::StepStatus::InProgress => {
                        codex_state::ThreadPlanItemStatus::InProgress
                    }
                    codex_protocol::plan_tool::StepStatus::Completed => {
                        codex_state::ThreadPlanItemStatus::Completed
                    }
                },
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
