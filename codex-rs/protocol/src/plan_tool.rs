use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

// Types for the TODO tool arguments matching codex-vscode/todo-mcp/src/main.rs
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PlanItemArg {
    #[serde(default)]
    pub id: Option<String>,
    pub step: String,
    pub status: StepStatus,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    #[serde(default)]
    pub inputs: Option<Vec<String>>,
    #[serde(default)]
    pub outputs: Option<Vec<String>>,
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
    #[serde(default)]
    pub acceptance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlanArgs {
    /// Arguments for the `update_plan` todo/checklist tool (not plan mode).
    #[serde(default)]
    pub explanation: Option<String>,
    pub plan: Vec<PlanItemArg>,
}
