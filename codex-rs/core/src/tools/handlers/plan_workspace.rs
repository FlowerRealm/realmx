use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::features::Feature;
use crate::function_tool::FunctionCallError;
use crate::plan_workspace::PlanWorkspace;
use crate::plan_workspace::PlanWorkspaceFile;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::JsonSchema;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::LazyLock;

pub struct PlanWorkspaceReadHandler;
pub struct PlanWorkspaceWriteHandler;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PlanWorkspaceFileArg {
    Requirements,
    Design,
    TasksCsv,
    TasksMd,
}

impl PlanWorkspaceFileArg {
    fn into_core(self) -> PlanWorkspaceFile {
        match self {
            Self::Requirements => PlanWorkspaceFile::Requirements,
            Self::Design => PlanWorkspaceFile::Design,
            Self::TasksCsv => PlanWorkspaceFile::TasksCsv,
            Self::TasksMd => PlanWorkspaceFile::TasksMd,
        }
    }
}

#[derive(Debug, Deserialize)]
struct PlanWorkspaceReadArgs {
    file: PlanWorkspaceFileArg,
}

#[derive(Debug, Deserialize)]
struct PlanWorkspaceWriteArgs {
    file: PlanWorkspaceFileArg,
    content: String,
}

pub static PLAN_WORKSPACE_READ_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "file".to_string(),
        JsonSchema::String {
            description: Some("One of: requirements, design, tasks_csv, tasks_md".to_string()),
        },
    );
    ToolSpec::Function(ResponsesApiTool {
        name: "plan_workspace_read".to_string(),
        description:
            "Read a plan workspace file from the current thread's file-first plan workspace."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["file".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
});

pub static PLAN_WORKSPACE_WRITE_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "file".to_string(),
        JsonSchema::String {
            description: Some(
                "One of: requirements, design, tasks_csv. tasks_md is derived and cannot be written directly."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "content".to_string(),
        JsonSchema::String {
            description: Some("Full file content to write.".to_string()),
        },
    );
    ToolSpec::Function(ResponsesApiTool {
        name: "plan_workspace_write".to_string(),
        description: "Write a plan workspace file for the current thread. Use this in Plan/Auto Plan modes to update requirements.md, design.md, or tasks.csv before emitting the final proposed_plan block.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["file".to_string(), "content".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
});

#[async_trait]
impl ToolHandler for PlanWorkspaceReadHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let session = invocation.session.clone();
        let turn = invocation.turn.clone();
        let arguments = match invocation.payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "plan_workspace_read handler received unsupported payload".to_string(),
                ));
            }
        };

        ensure_plan_workspace_available(session.enabled(Feature::PlanModeWorkspace), &turn)?;
        let args: PlanWorkspaceReadArgs = parse_arguments(&arguments)?;
        let codex_home = session.codex_home().await;
        let thread_id = session.conversation_id.to_string();
        let workspace = PlanWorkspace::new(codex_home.as_path(), turn.cwd.as_path(), &thread_id);
        let file = args.file.into_core();
        let content = workspace.read_file(file).await.map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to read plan workspace {}: {err}",
                file.file_name()
            ))
        })?;
        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

#[async_trait]
impl ToolHandler for PlanWorkspaceWriteHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let session = invocation.session.clone();
        let turn = invocation.turn.clone();
        let arguments = match invocation.payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "plan_workspace_write handler received unsupported payload".to_string(),
                ));
            }
        };

        ensure_plan_workspace_available(session.enabled(Feature::PlanModeWorkspace), &turn)?;
        let args: PlanWorkspaceWriteArgs = parse_arguments(&arguments)?;
        let file = args.file.into_core();
        let codex_home = session.codex_home().await;
        let thread_id = session.conversation_id.to_string();
        let workspace = PlanWorkspace::new(codex_home.as_path(), turn.cwd.as_path(), &thread_id);
        workspace
            .write_file(file, args.content.as_str())
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to write plan workspace {}: {err}",
                    file.file_name()
                ))
            })?;
        Ok(FunctionToolOutput::from_text(
            format!("Wrote {}", file.file_name()),
            Some(true),
        ))
    }
}

fn ensure_plan_workspace_available(
    feature_enabled: bool,
    turn: &crate::codex::TurnContext,
) -> Result<(), FunctionCallError> {
    if !feature_enabled {
        return Err(FunctionCallError::RespondToModel(
            "plan workspace tools are disabled".to_string(),
        ));
    }
    if !turn.collaboration_mode.mode.is_plan_output_mode() {
        return Err(FunctionCallError::RespondToModel(
            "plan workspace tools are only available in Plan and Auto Plan modes".to_string(),
        ));
    }
    Ok(())
}
