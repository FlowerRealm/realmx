use std::path::PathBuf;

use codex_protocol::ThreadId;
use codex_protocol::protocol::ApplyPatchApprovalRequestEvent;
use codex_protocol::protocol::ExecApprovalRequestEvent;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputResponse;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AgentTreeRequestKey {
    pub thread_id: ThreadId,
    pub event_id: String,
}

impl AgentTreeRequestKey {
    pub fn new(thread_id: ThreadId, event_id: impl Into<String>) -> Self {
        Self {
            thread_id,
            event_id: event_id.into(),
        }
    }

    pub fn as_string(&self) -> String {
        format!("{}:{}", self.thread_id, self.event_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkRequest {
    pub task: String,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub tests: Vec<String>,
    #[serde(default)]
    pub base_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerCommandResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerResult {
    pub summary: String,
    pub diff: String,
    pub commands: Vec<WorkerCommandResult>,
    pub worktree_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeedUserInput {
    pub request_key: AgentTreeRequestKey,
    pub args: RequestUserInputArgs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInputAnswer {
    pub request_key: AgentTreeRequestKey,
    pub response: RequestUserInputResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeedExecApproval {
    pub request_key: AgentTreeRequestKey,
    pub event: ExecApprovalRequestEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecApprovalAnswer {
    pub request_key: AgentTreeRequestKey,
    pub decision: ReviewDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeedPatchApproval {
    pub request_key: AgentTreeRequestKey,
    pub event: ApplyPatchApprovalRequestEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchApprovalAnswer {
    pub request_key: AgentTreeRequestKey,
    pub decision: ReviewDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerLog {
    pub level: WorkerLogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerError {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentTreeIpcMessage {
    WorkRequest(WorkRequest),
    NeedUserInput(NeedUserInput),
    UserInputAnswer(UserInputAnswer),
    NeedExecApproval(NeedExecApproval),
    ExecApprovalAnswer(ExecApprovalAnswer),
    NeedPatchApproval(NeedPatchApproval),
    PatchApprovalAnswer(PatchApprovalAnswer),
    WorkerResult(WorkerResult),
    Log(WorkerLog),
    Error(WorkerError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::request_user_input::RequestUserInputQuestion;
    use pretty_assertions::assert_eq;

    #[test]
    fn work_request_json_round_trip() {
        let msg = AgentTreeIpcMessage::WorkRequest(WorkRequest {
            task: "do thing".to_string(),
            context: Some("ctx".to_string()),
            tests: vec!["cargo test -p codex-core".to_string()],
            base_ref: Some("HEAD".to_string()),
        });

        let json = serde_json::to_value(&msg).expect("serialize");
        assert_eq!(json["type"], "work_request");

        let parsed: AgentTreeIpcMessage = serde_json::from_value(json).expect("deserialize");
        let AgentTreeIpcMessage::WorkRequest(req) = parsed else {
            panic!("expected WorkRequest");
        };
        assert_eq!(req.task, "do thing");
        assert_eq!(req.context.as_deref(), Some("ctx"));
        assert_eq!(req.tests, vec!["cargo test -p codex-core"]);
        assert_eq!(req.base_ref.as_deref(), Some("HEAD"));
    }

    #[test]
    fn need_user_input_json_round_trip() {
        let msg = AgentTreeIpcMessage::NeedUserInput(NeedUserInput {
            request_key: AgentTreeRequestKey::new(ThreadId::new(), "0"),
            args: RequestUserInputArgs {
                questions: vec![RequestUserInputQuestion {
                    id: "q1".to_string(),
                    header: "Q1".to_string(),
                    question: "Pick one".to_string(),
                    options: None,
                }],
            },
        });

        let json = serde_json::to_value(&msg).expect("serialize");
        assert_eq!(json["type"], "need_user_input");

        let parsed: AgentTreeIpcMessage = serde_json::from_value(json).expect("deserialize");
        let AgentTreeIpcMessage::NeedUserInput(need) = parsed else {
            panic!("expected NeedUserInput");
        };
        assert_eq!(need.args.questions.len(), 1);
        assert_eq!(need.args.questions[0].id, "q1");
    }
}
