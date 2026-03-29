use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use codex_app_server_protocol::CollabAgentTool;
use codex_app_server_protocol::CollabAgentToolCallStatus;
use codex_app_server_protocol::CommandExecutionStatus;
use codex_app_server_protocol::McpToolCallStatus;
use codex_app_server_protocol::PatchApplyStatus;
use codex_app_server_protocol::PatchChangeKind;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::ThreadTokenUsage;
use codex_app_server_protocol::TurnStatus;
use codex_core::config::Config;
use codex_protocol::models::WebSearchAction;
use codex_protocol::protocol::SessionConfiguredEvent;
use serde_json::json;

pub use crate::event_processor::CodexStatus;
use crate::event_processor::EventProcessor;
use crate::event_processor::handle_last_message;
use crate::exec_events::AgentMessageItem;
use crate::exec_events::CollabAgentState;
use crate::exec_events::CollabAgentStatus;
use crate::exec_events::CollabTool;
use crate::exec_events::CollabToolCallItem;
use crate::exec_events::CollabToolCallStatus;
use crate::exec_events::CommandExecutionItem;
use crate::exec_events::CommandExecutionStatus as ExecCommandExecutionStatus;
use crate::exec_events::ErrorItem;
use crate::exec_events::FileChangeItem;
use crate::exec_events::FileUpdateChange;
use crate::exec_events::ItemCompletedEvent;
use crate::exec_events::ItemStartedEvent;
use crate::exec_events::ItemUpdatedEvent;
use crate::exec_events::McpToolCallItem;
use crate::exec_events::McpToolCallItemError;
use crate::exec_events::McpToolCallItemResult;
use crate::exec_events::McpToolCallStatus;
use crate::exec_events::PatchApplyStatus;
use crate::exec_events::PatchChangeKind;
use crate::exec_events::PlanProgressItem;
use crate::exec_events::PlanProgressRow;
use crate::exec_events::ReasoningItem;
use crate::exec_events::ThreadErrorEvent;
use crate::exec_events::ThreadEvent;
use crate::exec_events::ThreadItem as ExecThreadItem;
use crate::exec_events::ThreadItemDetails;
use crate::exec_events::ThreadStartedEvent;
use crate::exec_events::TodoItem;
use crate::exec_events::TodoListItem;
use crate::exec_events::TurnCompletedEvent;
use crate::exec_events::TurnFailedEvent;
use crate::exec_events::TurnStartedEvent;
use crate::exec_events::Usage;
use crate::exec_events::WebSearchItem;
use codex_core::canonical_plan_csv_from_update_plan_args;
use codex_core::config::Config;
use codex_protocol::models::WebSearchAction;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::protocol;
use codex_protocol::protocol::AgentStatus as CoreAgentStatus;
use codex_protocol::protocol::CollabAgentInteractionBeginEvent;
use codex_protocol::protocol::CollabAgentInteractionEndEvent;
use codex_protocol::protocol::CollabAgentSpawnBeginEvent;
use codex_protocol::protocol::CollabAgentSpawnEndEvent;
use codex_protocol::protocol::CollabCloseBeginEvent;
use codex_protocol::protocol::CollabCloseEndEvent;
use codex_protocol::protocol::CollabWaitingBeginEvent;
use codex_protocol::protocol::CollabWaitingEndEvent;
use serde_json::Value as JsonValue;
use tracing::error;
use tracing::warn;

pub struct EventProcessorWithJsonOutput {
    last_message_path: Option<PathBuf>,
    last_proposed_plan: Option<String>,
    next_event_id: AtomicU64,
    plan_workflow_enabled: bool,
    // Tracks running commands by call_id, including the associated item id.
    running_commands: HashMap<String, RunningCommand>,
    running_patch_applies: HashMap<String, protocol::PatchApplyBeginEvent>,
    // Tracks the running plan-progress item for the current turn (at most one per turn).
    running_plan_progress: Option<RunningPlanProgress>,
    last_total_token_usage: Option<codex_protocol::protocol::TokenUsage>,
    running_mcp_tool_calls: HashMap<String, RunningMcpToolCall>,
    running_collab_tool_calls: HashMap<String, RunningCollabToolCall>,
    running_web_search_calls: HashMap<String, String>,
    last_critical_error: Option<ThreadErrorEvent>,
    final_message: Option<String>,
    emit_final_message_on_shutdown: bool,
}

#[derive(Debug, Clone)]
enum RunningPlanProgress {
    Csv {
        item_id: String,
        raw_csv: String,
        rows: Vec<PlanProgressRow>,
    },
    LegacyTodo {
        item_id: String,
        items: Vec<TodoItem>,
    },
}

#[derive(Debug, PartialEq)]
pub struct CollectedThreadEvents {
    pub events: Vec<ThreadEvent>,
    pub status: CodexStatus,
}

impl EventProcessorWithJsonOutput {
    pub fn new(last_message_path: Option<PathBuf>) -> Self {
        Self::new_with_plan_workflow(last_message_path, false)
    }

    pub fn new_with_plan_workflow(
        last_message_path: Option<PathBuf>,
        plan_workflow_enabled: bool,
    ) -> Self {
        Self {
            last_message_path,
            last_proposed_plan: None,
            next_event_id: AtomicU64::new(0),
            plan_workflow_enabled,
            running_commands: HashMap::new(),
            running_patch_applies: HashMap::new(),
            running_plan_progress: None,
            last_total_token_usage: None,
            last_critical_error: None,
            final_message: None,
            emit_final_message_on_shutdown: false,
        }
    }

    pub fn final_message(&self) -> Option<&str> {
        self.final_message.as_deref()
    }

    fn next_item_id(&self) -> String {
        format!("item_{}", self.next_item_id.fetch_add(1, Ordering::SeqCst))
    }

    #[allow(clippy::print_stdout)]
    fn emit(&self, event: ThreadEvent) {
        println!(
            "{}",
            serde_json::to_string(&event).unwrap_or_else(|err| {
                json!({
                    "type": "error",
                    "message": format!("failed to serialize exec json event: {err}"),
                })
                .to_string()
            })
        );
    }

    fn usage_from_last_total(&self) -> Usage {
        let Some(usage) = self.last_total_token_usage.as_ref() else {
            return Usage::default();
        };
        Usage {
            input_tokens: usage.total.input_tokens,
            cached_input_tokens: usage.total.cached_input_tokens,
            output_tokens: usage.total.output_tokens,
        }
    }

    pub fn map_todo_items(plan: &[codex_app_server_protocol::TurnPlanStep]) -> Vec<TodoItem> {
        plan.iter()
            .map(|step| TodoItem {
                text: step.step.clone(),
                completed: matches!(
                    step.status,
                    codex_app_server_protocol::TurnPlanStepStatus::Completed
                ),
            })
            .collect()
    }

    fn map_item_with_id(
        item: ThreadItem,
        make_id: impl FnOnce() -> String,
    ) -> Option<ExecThreadItem> {
        match item {
            ThreadItem::AgentMessage { text, .. } => Some(ExecThreadItem {
                id: make_id(),
                details: ThreadItemDetails::AgentMessage(AgentMessageItem { text }),
            }),
            ThreadItem::Reasoning { summary, .. } => {
                let text = summary.join("\n");
                if text.trim().is_empty() {
                    return None;
                }
                Some(ExecThreadItem {
                    id: make_id(),
                    details: ThreadItemDetails::Reasoning(ReasoningItem { text }),
                })
            }
            ThreadItem::CommandExecution {
                command,
                aggregated_output,
                exit_code,
                status,
                ..
            } => Some(ExecThreadItem {
                id: make_id(),
                details: ThreadItemDetails::CommandExecution(CommandExecutionItem {
                    command,
                    aggregated_output: aggregated_output.unwrap_or_default(),
                    exit_code,
                    status: match status {
                        CommandExecutionStatus::InProgress => ExecCommandExecutionStatus::InProgress,
                        CommandExecutionStatus::Completed => ExecCommandExecutionStatus::Completed,
                        CommandExecutionStatus::Failed => ExecCommandExecutionStatus::Failed,
                        CommandExecutionStatus::Declined => ExecCommandExecutionStatus::Declined,
                    },
                }),
            }),
            ThreadItem::FileChange {
                changes, status, ..
            } => Some(ExecThreadItem {
                id: make_id(),
                details: ThreadItemDetails::FileChange(FileChangeItem {
                    changes: changes
                        .into_iter()
                        .map(|change| FileUpdateChange {
                            path: change.path,
                            kind: match change.kind {
                                PatchChangeKind::Add => ExecPatchChangeKind::Add,
                                PatchChangeKind::Delete => ExecPatchChangeKind::Delete,
                                PatchChangeKind::Update { .. } => ExecPatchChangeKind::Update,
                            },
                        })
                        .collect(),
                    status: match status {
                        PatchApplyStatus::InProgress => ExecPatchApplyStatus::InProgress,
                        PatchApplyStatus::Completed => ExecPatchApplyStatus::Completed,
                        PatchApplyStatus::Failed | PatchApplyStatus::Declined => {
                            ExecPatchApplyStatus::Failed
                        }
                    },
                }),
            }),
            ThreadItem::McpToolCall {
                server,
                tool,
                status,
                arguments,
                result,
                error,
                ..
            } => Some(ExecThreadItem {
                id: make_id(),
                details: ThreadItemDetails::McpToolCall(McpToolCallItem {
                    server,
                    tool,
                    status: match status {
                        McpToolCallStatus::InProgress => ExecMcpToolCallStatus::InProgress,
                        McpToolCallStatus::Completed => ExecMcpToolCallStatus::Completed,
                        McpToolCallStatus::Failed => ExecMcpToolCallStatus::Failed,
                    },
                    arguments,
                    result: result.map(|result| McpToolCallItemResult {
                        content: result.content,
                        structured_content: result.structured_content,
                    }),
                    error: error.map(|error| McpToolCallItemError {
                        message: error.message,
                    }),
                }),
            }),
            ThreadItem::CollabAgentToolCall {
                tool,
                sender_thread_id,
                receiver_thread_ids,
                prompt,
                agents_states,
                status,
                ..
            } => Some(ExecThreadItem {
                id: make_id(),
                details: ThreadItemDetails::CollabToolCall(CollabToolCallItem {
                    tool: match tool {
                        CollabAgentTool::SpawnAgent => CollabTool::SpawnAgent,
                        CollabAgentTool::SendInput => CollabTool::SendInput,
                        CollabAgentTool::ResumeAgent => CollabTool::Wait,
                        CollabAgentTool::Wait => CollabTool::Wait,
                        CollabAgentTool::CloseAgent => CollabTool::CloseAgent,
                    },
                    sender_thread_id,
                    receiver_thread_ids,
                    prompt,
                    agents_states: agents_states
                        .into_iter()
                        .map(|(thread_id, state)| {
                            (
                                thread_id,
                                CollabAgentState {
                                    status: match state.status {
                                        codex_app_server_protocol::CollabAgentStatus::PendingInit => {
                                            CollabAgentStatus::PendingInit
                                        }
                                        codex_app_server_protocol::CollabAgentStatus::Running => {
                                            CollabAgentStatus::Running
                                        }
                                        codex_app_server_protocol::CollabAgentStatus::Interrupted => {
                                            CollabAgentStatus::Interrupted
                                        }
                                        codex_app_server_protocol::CollabAgentStatus::Completed => {
                                            CollabAgentStatus::Completed
                                        }
                                        codex_app_server_protocol::CollabAgentStatus::Errored => {
                                            CollabAgentStatus::Errored
                                        }
                                        codex_app_server_protocol::CollabAgentStatus::Shutdown => {
                                            CollabAgentStatus::Shutdown
                                        }
                                        codex_app_server_protocol::CollabAgentStatus::NotFound => {
                                            CollabAgentStatus::NotFound
                                        }
                                    },
                                    message: state.message,
                                },
                            )
                        })
                        .collect(),
                    status: match status {
                        CollabAgentToolCallStatus::InProgress => CollabToolCallStatus::InProgress,
                        CollabAgentToolCallStatus::Completed => CollabToolCallStatus::Completed,
                        CollabAgentToolCallStatus::Failed => CollabToolCallStatus::Failed,
                    },
                }),
            }),
            ThreadItem::WebSearch {
                id: raw_id,
                query,
                action,
            } => Some(ExecThreadItem {
                id: make_id(),
                details: ThreadItemDetails::WebSearch(WebSearchItem {
                    id: raw_id,
                    query,
                    action: match action {
                        Some(action) => serde_json::from_value(
                            serde_json::to_value(action).unwrap_or_else(|_| json!("other")),
                        )
                        .unwrap_or(WebSearchAction::Other),
                        None => WebSearchAction::Other,
                    },
                }),
            }),
            _ => None,
        }
    }

    fn started_item_id(&mut self, raw_id: &str) -> String {
        if let Some(existing) = self.raw_to_exec_item_id.get(raw_id) {
            return existing.clone();
        }
        let exec_id = self.next_item_id();
        self.raw_to_exec_item_id
            .insert(raw_id.to_string(), exec_id.clone());
        exec_id
    }

    fn completed_item_id(&mut self, raw_id: &str) -> String {
        self.raw_to_exec_item_id
            .remove(raw_id)
            .unwrap_or_else(|| self.next_item_id())
    }

    fn map_started_item(&mut self, item: ThreadItem) -> Option<ExecThreadItem> {
        match item {
            ThreadItem::AgentMessage { .. } | ThreadItem::Reasoning { .. } => None,
            other => {
                let raw_id = other.id().to_string();
                Self::map_item_with_id(other, || self.started_item_id(&raw_id))
            }
        }
    }

    fn map_completed_item_mut(&mut self, item: ThreadItem) -> Option<ExecThreadItem> {
        if let ThreadItem::Reasoning { summary, .. } = &item
            && summary.join("\n").trim().is_empty()
        {
            return None;
        }
        match &item {
            ThreadItem::AgentMessage { .. } | ThreadItem::Reasoning { .. } => {
                Self::map_item_with_id(item, || self.next_item_id())
            }
            other => {
                let raw_id = other.id().to_string();
                Self::map_item_with_id(item, || self.completed_item_id(&raw_id))
            }
        }
    }

    fn reconcile_unfinished_started_items(
        &mut self,
        turn_items: &[ThreadItem],
    ) -> Vec<ThreadEvent> {
        turn_items
            .iter()
            .filter_map(|item| {
                let raw_id = item.id().to_string();
                if !self.raw_to_exec_item_id.contains_key(&raw_id) {
                    return None;
                }
                self.map_completed_item_mut(item.clone())
                    .map(|item| ThreadEvent::ItemCompleted(ItemCompletedEvent { item }))
            })
            .collect()
    }

    fn plan_progress_item_from_plan(&self, args: &UpdatePlanArgs) -> Option<PlanProgressItem> {
        let canonical_plan = canonical_plan_csv_from_update_plan_args(args).ok()?;
        Some(PlanProgressItem {
            raw_csv: canonical_plan.raw_csv,
            rows: canonical_plan
                .rows
                .into_iter()
                .map(|row| PlanProgressRow {
                    id: row.row_id,
                    step: row.step,
                    status: match row.status.as_str() {
                        "pending" => StepStatus::Pending,
                        "in_progress" => StepStatus::InProgress,
                        "completed" => StepStatus::Completed,
                        other => {
                            warn!("unexpected thread plan item status in exec event processor: {other}");
                            StepStatus::Pending
                        }
                    },
                    path: row.path,
                    details: row.details,
                    inputs: row.inputs,
                    outputs: row.outputs,
                    depends_on: row.depends_on,
                    acceptance: row.acceptance,
                })
                .collect(),
        })
    }

    fn handle_plan_update(&mut self, args: &UpdatePlanArgs) -> Vec<ThreadEvent> {
        if self.plan_workflow_enabled
            && let Some(plan_progress) = self.plan_progress_item_from_plan(args)
        {
            if let Some(RunningPlanProgress::Csv {
                item_id,
                raw_csv,
                rows,
            }) = &mut self.running_plan_progress
            {
                *raw_csv = plan_progress.raw_csv.clone();
                *rows = plan_progress.rows.clone();
                let item = ThreadItem {
                    id: item_id.clone(),
                    details: ThreadItemDetails::PlanProgress(plan_progress),
                };
                return vec![ThreadEvent::ItemUpdated(ItemUpdatedEvent { item })];
            }

            let item_id = self.get_next_item_id();
            self.running_plan_progress = Some(RunningPlanProgress::Csv {
                item_id: item_id.clone(),
                raw_csv: plan_progress.raw_csv.clone(),
                rows: plan_progress.rows.clone(),
            });
            let item = ThreadItem {
                id: item_id,
                details: ThreadItemDetails::PlanProgress(plan_progress),
            };
            return vec![ThreadEvent::ItemStarted(ItemStartedEvent { item })];
        }

        let items = self.todo_items_from_plan(args);
        if let Some(RunningPlanProgress::LegacyTodo {
            item_id,
            items: running_items,
        }) = &mut self.running_plan_progress
        {
            *running_items = items.clone();
            let item = ThreadItem {
                id: item_id.clone(),
                details: ThreadItemDetails::TodoList(TodoListItem { items }),
            };
            return vec![ThreadEvent::ItemUpdated(ItemUpdatedEvent { item })];
        }

        let item_id = self.get_next_item_id();
        self.running_plan_progress = Some(RunningPlanProgress::LegacyTodo {
            item_id: item_id.clone(),
            items: items.clone(),
        });
        let item = ThreadItem {
            id: item_id,
            details: ThreadItemDetails::TodoList(TodoListItem { items }),
        };
        vec![ThreadEvent::ItemStarted(ItemStartedEvent { item })]
    }

    fn handle_task_started(&mut self, _: &protocol::TurnStartedEvent) -> Vec<ThreadEvent> {
        self.last_critical_error = None;
        vec![ThreadEvent::TurnStarted(TurnStartedEvent {})]
    }

    fn handle_task_complete(&mut self) -> Vec<ThreadEvent> {
        let usage = if let Some(u) = &self.last_total_token_usage {
            Usage {
                input_tokens: u.input_tokens,
                cached_input_tokens: u.cached_input_tokens,
                output_tokens: u.output_tokens,
            }
        } else {
            Usage::default()
        };

        let mut items = Vec::new();

        if let Some(running) = self.running_plan_progress.take() {
            let item = match running {
                RunningPlanProgress::Csv {
                    item_id,
                    raw_csv,
                    rows,
                } => ThreadItem {
                    id: item_id,
                    details: ThreadItemDetails::PlanProgress(PlanProgressItem { raw_csv, rows }),
                },
                RunningPlanProgress::LegacyTodo { item_id, items } => ThreadItem {
                    id: item_id,
                    details: ThreadItemDetails::TodoList(TodoListItem { items }),
                },
            };
            items.push(ThreadEvent::ItemCompleted(ItemCompletedEvent { item }));
        }

        if !self.running_commands.is_empty() {
            for (_, running) in self.running_commands.drain() {
                let item = ThreadItem {
                    id: running.item_id,
                    details: ThreadItemDetails::CommandExecution(CommandExecutionItem {
                        command: running.command,
                        aggregated_output: running.aggregated_output,
                        exit_code: None,
                        status: CommandExecutionStatus::Completed,
                    }),
                };
                items.push(ThreadEvent::ItemCompleted(ItemCompletedEvent { item }));
            }
        }

        if let Some(error) = self.last_critical_error.take() {
            items.push(ThreadEvent::TurnFailed(TurnFailedEvent { error }));
        } else {
            items.push(ThreadEvent::TurnCompleted(TurnCompletedEvent { usage }));
        }

        items
            .iter()
            .rev()
            .find_map(|item| match item {
                ThreadItem::AgentMessage { text, .. } => Some(text.clone()),
                _ => None,
            })
            .or_else(|| {
                items.iter().rev().find_map(|item| match item {
                    ThreadItem::Plan { text, .. } => Some(text.clone()),
                    _ => None,
                })
            })
    }

    pub fn thread_started_event(session_configured: &SessionConfiguredEvent) -> ThreadEvent {
        ThreadEvent::ThreadStarted(ThreadStartedEvent {
            thread_id: session_configured.session_id.to_string(),
        })
    }

    pub fn collect_warning(&mut self, message: String) -> CollectedThreadEvents {
        CollectedThreadEvents {
            events: vec![ThreadEvent::ItemCompleted(ItemCompletedEvent {
                item: ExecThreadItem {
                    id: self.next_item_id(),
                    details: ThreadItemDetails::Error(ErrorItem { message }),
                },
            })],
            status: CodexStatus::Running,
        }
    }

    pub fn collect_thread_events(
        &mut self,
        notification: ServerNotification,
    ) -> CollectedThreadEvents {
        let mut events = Vec::new();
        let status = match notification {
            ServerNotification::ConfigWarning(notification) => {
                let message = match notification.details {
                    Some(details) if !details.is_empty() => {
                        format!("{} ({details})", notification.summary)
                    }
                    _ => notification.summary,
                };
                events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent {
                    item: ExecThreadItem {
                        id: self.next_item_id(),
                        details: ThreadItemDetails::Error(ErrorItem { message }),
                    },
                }));
                CodexStatus::Running
            }
            ServerNotification::Error(notification) => {
                let message = match notification.error.additional_details {
                    Some(details) if !details.is_empty() => {
                        format!("{} ({details})", notification.error.message)
                    }
                    _ => notification.error.message,
                };
                let error = ThreadErrorEvent { message };
                self.last_critical_error = Some(error.clone());
                events.push(ThreadEvent::Error(error));
                CodexStatus::Running
            }
            ServerNotification::DeprecationNotice(notification) => {
                let message = match notification.details {
                    Some(details) if !details.is_empty() => {
                        format!("{} ({details})", notification.summary)
                    }
                    _ => notification.summary,
                };
                events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent {
                    item: ExecThreadItem {
                        id: self.next_item_id(),
                        details: ThreadItemDetails::Error(ErrorItem { message }),
                    },
                }));
                CodexStatus::Running
            }
            ServerNotification::HookStarted(_) | ServerNotification::HookCompleted(_) => {
                CodexStatus::Running
            }
            ServerNotification::ItemStarted(notification) => {
                if let Some(item) = self.map_started_item(notification.item) {
                    events.push(ThreadEvent::ItemStarted(ItemStartedEvent { item }));
                }
                CodexStatus::Running
            }
            ServerNotification::ItemCompleted(notification) => {
                if let Some(item) = self.map_completed_item_mut(notification.item) {
                    if let ThreadItemDetails::AgentMessage(AgentMessageItem { text }) =
                        &item.details
                    {
                        self.final_message = Some(text.clone());
                    }
                    events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent { item }));
                }
                CodexStatus::Running
            }
            ServerNotification::ModelRerouted(notification) => {
                events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent {
                    item: ExecThreadItem {
                        id: self.next_item_id(),
                        details: ThreadItemDetails::Error(ErrorItem {
                            message: format!(
                                "model rerouted: {} -> {} ({:?})",
                                notification.from_model, notification.to_model, notification.reason
                            ),
                        }),
                    },
                }));
                CodexStatus::Running
            }
            ServerNotification::ThreadTokenUsageUpdated(notification) => {
                self.last_total_token_usage = Some(notification.token_usage);
                CodexStatus::Running
            }
            ServerNotification::TurnCompleted(notification) => {
                if let Some(running) = self.running_todo_list.take() {
                    events.push(ThreadEvent::ItemCompleted(ItemCompletedEvent {
                        item: ExecThreadItem {
                            id: running.item_id,
                            details: ThreadItemDetails::TodoList(TodoListItem {
                                items: running.items,
                            }),
                        },
                    }));
                }
                events.extend(self.reconcile_unfinished_started_items(&notification.turn.items));
                match notification.turn.status {
                    TurnStatus::Completed => {
                        if let Some(final_message) =
                            Self::final_message_from_turn_items(notification.turn.items.as_slice())
                        {
                            self.final_message = Some(final_message);
                        }
                        self.emit_final_message_on_shutdown = true;
                        events.push(ThreadEvent::TurnCompleted(TurnCompletedEvent {
                            usage: self.usage_from_last_total(),
                        }));
                        CodexStatus::InitiateShutdown
                    }
                    TurnStatus::Failed => {
                        self.final_message = None;
                        self.emit_final_message_on_shutdown = false;
                        let error = notification
                            .turn
                            .error
                            .map(|error| ThreadErrorEvent {
                                message: match error.additional_details {
                                    Some(details) if !details.is_empty() => {
                                        format!("{} ({details})", error.message)
                                    }
                                    _ => error.message,
                                },
                            })
                            .or_else(|| self.last_critical_error.clone())
                            .unwrap_or_else(|| ThreadErrorEvent {
                                message: "turn failed".to_string(),
                            });
                        events.push(ThreadEvent::TurnFailed(TurnFailedEvent { error }));
                        CodexStatus::InitiateShutdown
                    }
                    TurnStatus::Interrupted => {
                        self.final_message = None;
                        self.emit_final_message_on_shutdown = false;
                        CodexStatus::InitiateShutdown
                    }
                    TurnStatus::InProgress => CodexStatus::Running,
                }
            }
            ServerNotification::TurnDiffUpdated(_) => CodexStatus::Running,
            ServerNotification::TurnPlanUpdated(notification) => {
                let items = Self::map_todo_items(&notification.plan);
                if let Some(running) = self.running_todo_list.as_mut() {
                    running.items = items.clone();
                    let item_id = running.item_id.clone();
                    events.push(ThreadEvent::ItemUpdated(ItemUpdatedEvent {
                        item: ExecThreadItem {
                            id: item_id,
                            details: ThreadItemDetails::TodoList(TodoListItem { items }),
                        },
                    }));
                } else {
                    let item_id = self.next_item_id();
                    self.running_todo_list = Some(RunningTodoList {
                        item_id: item_id.clone(),
                        items: items.clone(),
                    });
                    events.push(ThreadEvent::ItemStarted(ItemStartedEvent {
                        item: ExecThreadItem {
                            id: item_id,
                            details: ThreadItemDetails::TodoList(TodoListItem { items }),
                        },
                    }));
                }
                CodexStatus::Running
            }
            ServerNotification::TurnStarted(_) => {
                events.push(ThreadEvent::TurnStarted(TurnStartedEvent {}));
                CodexStatus::Running
            }
            _ => CodexStatus::Running,
        };

        CollectedThreadEvents { events, status }
    }
}

impl EventProcessor for EventProcessorWithJsonOutput {
    fn print_config_summary(
        &mut self,
        _: &Config,
        _: &str,
        session_configured: &SessionConfiguredEvent,
    ) {
        self.emit(Self::thread_started_event(session_configured));
    }

    fn process_server_notification(&mut self, notification: ServerNotification) -> CodexStatus {
        let collected = self.collect_thread_events(notification);
        for event in collected.events {
            self.emit(event);
        }
        collected.status
    }

    fn process_warning(&mut self, message: String) -> CodexStatus {
        let collected = self.collect_warning(message);
        for event in collected.events {
            self.emit(event);
        }
        collected.status
    }

    fn print_final_output(&mut self) {
        if self.emit_final_message_on_shutdown
            && let Some(path) = self.last_message_path.as_deref()
        {
            handle_last_message(self.final_message.as_deref(), path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn failed_turn_does_not_overwrite_output_last_message_file() {
        let tempdir = tempdir().expect("create tempdir");
        let output_path = tempdir.path().join("last-message.txt");
        std::fs::write(&output_path, "keep existing contents").expect("seed output file");

        let mut processor = EventProcessorWithJsonOutput::new(Some(output_path.clone()));

        let collected = processor.collect_thread_events(ServerNotification::ItemCompleted(
            codex_app_server_protocol::ItemCompletedNotification {
                item: ThreadItem::AgentMessage {
                    id: "msg-1".to_string(),
                    text: "partial answer".to_string(),
                    phase: None,
                    memory_citation: None,
                },
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
        ));

        assert_eq!(collected.status, CodexStatus::Running);
        assert_eq!(processor.final_message(), Some("partial answer"));

        let status = processor.process_server_notification(ServerNotification::TurnCompleted(
            codex_app_server_protocol::TurnCompletedNotification {
                thread_id: "thread-1".to_string(),
                turn: codex_app_server_protocol::Turn {
                    id: "turn-1".to_string(),
                    items: Vec::new(),
                    status: TurnStatus::Failed,
                    error: Some(codex_app_server_protocol::TurnError {
                        message: "turn failed".to_string(),
                        additional_details: None,
                        codex_error_info: None,
                    }),
                },
            },
        ));

        assert_eq!(status, CodexStatus::InitiateShutdown);
        assert_eq!(processor.final_message(), None);

        EventProcessor::print_final_output(&mut processor);

        assert_eq!(
            std::fs::read_to_string(&output_path).expect("read output file"),
            "keep existing contents"
        );
    }
}
