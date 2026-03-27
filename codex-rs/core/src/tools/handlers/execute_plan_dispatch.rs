use crate::execute_plan_dispatch::ExecuteDispatchSummary;
use crate::execute_plan_dispatch::execute_active_plan_with_subagents;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use async_trait::async_trait;

pub struct ExecutePlanDispatchHandler;

#[async_trait]
impl ToolHandler for ExecutePlanDispatchHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Self::Output, crate::function_tool::FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;

        match payload {
            ToolPayload::Function { .. } => {}
            _ => {
                return Err(crate::function_tool::FunctionCallError::RespondToModel(
                    "execute plan dispatch handler received unsupported payload".to_string(),
                ));
            }
        }

        let summary = execute_active_plan_with_subagents(&session, &turn).await?;
        Ok(FunctionToolOutput::from_text(
            render_summary(&summary),
            Some(summary.failed_rows.is_empty()),
        ))
    }
}

fn render_summary(summary: &ExecuteDispatchSummary) -> String {
    serde_json::to_string(summary).unwrap_or_else(|err| {
        format!("failed to serialize execute_active_plan_with_subagents result: {err}")
    })
}
