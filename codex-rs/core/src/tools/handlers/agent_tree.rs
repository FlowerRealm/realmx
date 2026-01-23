use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;

use crate::agent_tree::ipc::AgentTreeIpcMessage;
use crate::agent_tree::ipc::ExecApprovalAnswer;
use crate::agent_tree::ipc::PatchApprovalAnswer;
use crate::agent_tree::ipc::UserInputAnswer;
use crate::agent_tree::ipc::WorkRequest;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct AgentTreeHandler;

#[derive(Debug, Deserialize)]
struct AgentTreeDelegateArgs {
    task: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    tests: Vec<String>,
    #[serde(default)]
    base_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentTreeApplyDiffArgs {
    diff: String,
}

#[derive(Debug, Serialize)]
struct ApplyDiffResult {
    applied: bool,
    stdout: String,
    stderr: String,
}

#[async_trait]
impl ToolHandler for AgentTreeHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    async fn is_mutating(&self, invocation: &ToolInvocation) -> bool {
        invocation.tool_name == "agent_tree_apply_diff"
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            tool_name,
            payload,
            call_id,
            ..
        } = invocation;

        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "agent_tree handler received unsupported payload".to_string(),
            ));
        };

        match tool_name.as_str() {
            "agent_tree_delegate" => {
                let args: AgentTreeDelegateArgs = parse_arguments(&arguments)?;
                delegate::handle(session, turn, call_id, args).await
            }
            "agent_tree_apply_diff" => {
                let args: AgentTreeApplyDiffArgs = parse_arguments(&arguments)?;
                apply_diff::handle(turn, args).await
            }
            other => Err(FunctionCallError::RespondToModel(format!(
                "unsupported agent_tree tool {other}"
            ))),
        }
    }
}

mod delegate {
    use super::*;

    pub async fn handle(
        session: Arc<crate::codex::Session>,
        turn: Arc<crate::codex::TurnContext>,
        call_id: String,
        args: AgentTreeDelegateArgs,
    ) -> Result<ToolOutput, FunctionCallError> {
        let exe = std::env::current_exe().map_err(|err| {
            FunctionCallError::Fatal(format!("failed to resolve current_exe for worker: {err}"))
        })?;

        let mut child = tokio::process::Command::new(exe)
            .arg("agent-tree-worker")
            .current_dir(&turn.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| FunctionCallError::Fatal(format!("failed to spawn worker: {err}")))?;

        let mut child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| FunctionCallError::Fatal("worker stdin was not captured".to_string()))?;
        let child_stdout = child.stdout.take().ok_or_else(|| {
            FunctionCallError::Fatal("worker stdout was not captured".to_string())
        })?;
        let child_stderr = child.stderr.take();

        // Send initial WorkRequest.
        let work = WorkRequest {
            task: args.task,
            context: args.context,
            tests: args.tests,
            base_ref: args.base_ref,
        };
        write_msg(&mut child_stdin, &AgentTreeIpcMessage::WorkRequest(work)).await?;

        // Best-effort stderr drain (avoid deadlock if worker is chatty on stderr).
        if let Some(stderr) = child_stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut buf = String::new();
                loop {
                    buf.clear();
                    let read = reader.read_line(&mut buf).await;
                    match read {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            });
        }

        let mut lines = BufReader::new(child_stdout).lines();
        while let Some(line) = lines.next_line().await.map_err(|err| {
            FunctionCallError::Fatal(format!("failed to read worker stdout: {err}"))
        })? {
            let msg: AgentTreeIpcMessage = serde_json::from_str(&line).map_err(|err| {
                FunctionCallError::Fatal(format!("failed to parse worker IPC message: {err}"))
            })?;

            match msg {
                AgentTreeIpcMessage::NeedUserInput(need) => {
                    let response = session
                        .request_user_input(&turn, call_id.clone(), need.args)
                        .await
                        .unwrap_or_else(|| {
                            codex_protocol::request_user_input::RequestUserInputResponse {
                                answers: HashMap::new(),
                            }
                        });
                    let answer = UserInputAnswer {
                        request_key: need.request_key,
                        response,
                    };
                    write_msg(
                        &mut child_stdin,
                        &AgentTreeIpcMessage::UserInputAnswer(answer),
                    )
                    .await?;
                }
                AgentTreeIpcMessage::NeedExecApproval(need) => {
                    let decision = session
                        .request_command_approval(
                            &turn,
                            call_id.clone(),
                            need.event.command,
                            need.event.cwd,
                            need.event.reason,
                            need.event.proposed_execpolicy_amendment,
                        )
                        .await;
                    let answer = ExecApprovalAnswer {
                        request_key: need.request_key,
                        decision,
                    };
                    write_msg(
                        &mut child_stdin,
                        &AgentTreeIpcMessage::ExecApprovalAnswer(answer),
                    )
                    .await?;
                }
                AgentTreeIpcMessage::NeedPatchApproval(need) => {
                    let decision_rx = session
                        .request_patch_approval(
                            &turn,
                            call_id.clone(),
                            need.event.changes,
                            need.event.reason,
                            need.event.grant_root,
                        )
                        .await;
                    let decision = decision_rx.await.unwrap_or_default();
                    let answer = PatchApprovalAnswer {
                        request_key: need.request_key,
                        decision,
                    };
                    write_msg(
                        &mut child_stdin,
                        &AgentTreeIpcMessage::PatchApprovalAnswer(answer),
                    )
                    .await?;
                }
                AgentTreeIpcMessage::WorkerResult(result) => {
                    let content = serde_json::to_string(&result).map_err(|err| {
                        FunctionCallError::Fatal(format!(
                            "failed to serialize agent_tree_delegate result: {err}"
                        ))
                    })?;
                    return Ok(ToolOutput::Function {
                        content,
                        success: Some(true),
                        content_items: None,
                    });
                }
                AgentTreeIpcMessage::Log(_log) => {}
                AgentTreeIpcMessage::Error(err) => {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "agent_tree worker error: {}",
                        err.message
                    )));
                }
                AgentTreeIpcMessage::WorkRequest(_) => {}
                AgentTreeIpcMessage::UserInputAnswer(_)
                | AgentTreeIpcMessage::ExecApprovalAnswer(_)
                | AgentTreeIpcMessage::PatchApprovalAnswer(_) => {}
            }
        }

        let status = child.wait().await.map_err(|err| {
            FunctionCallError::Fatal(format!("failed to wait on agent_tree worker: {err}"))
        })?;
        Err(FunctionCallError::RespondToModel(format!(
            "agent_tree worker exited unexpectedly (status={status})"
        )))
    }

    async fn write_msg(
        stdin: &mut tokio::process::ChildStdin,
        msg: &AgentTreeIpcMessage,
    ) -> Result<(), FunctionCallError> {
        let line = serde_json::to_string(msg).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize IPC message: {err}"))
        })?;
        stdin.write_all(line.as_bytes()).await.map_err(|err| {
            FunctionCallError::Fatal(format!("failed to write to worker stdin: {err}"))
        })?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|err| FunctionCallError::Fatal(format!("failed to write newline: {err}")))?;
        stdin.flush().await.map_err(|err| {
            FunctionCallError::Fatal(format!("failed to flush worker stdin: {err}"))
        })?;
        Ok(())
    }
}

mod apply_diff {
    use super::*;

    pub async fn handle(
        turn: Arc<crate::codex::TurnContext>,
        args: AgentTreeApplyDiffArgs,
    ) -> Result<ToolOutput, FunctionCallError> {
        let mut child = tokio::process::Command::new("git")
            .arg("apply")
            .arg("--whitespace=nowarn")
            .arg("-")
            .current_dir(&turn.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to spawn git: {err}"))
            })?;

        let mut stdin = child.stdin.take().ok_or_else(|| {
            FunctionCallError::Fatal("git apply stdin was not captured".to_string())
        })?;
        stdin.write_all(args.diff.as_bytes()).await.map_err(|err| {
            FunctionCallError::Fatal(format!("failed to write diff to git: {err}"))
        })?;
        stdin
            .shutdown()
            .await
            .map_err(|err| FunctionCallError::Fatal(format!("failed to close git stdin: {err}")))?;

        let output = child.wait_with_output().await.map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to wait on git apply: {err}"))
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.status.success() {
            return Err(FunctionCallError::RespondToModel(format!(
                "git apply failed: {stderr}"
            )));
        }

        let content = serde_json::to_string(&ApplyDiffResult {
            applied: true,
            stdout,
            stderr,
        })
        .map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize agent_tree_apply_diff result: {err}"
            ))
        })?;

        Ok(ToolOutput::Function {
            content,
            success: Some(true),
            content_items: None,
        })
    }
}
