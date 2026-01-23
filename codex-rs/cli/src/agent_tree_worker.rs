use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use codex_core::AuthManager;
use codex_core::ThreadManager;
use codex_core::agent_tree::ipc::AgentTreeIpcMessage;
use codex_core::agent_tree::ipc::AgentTreeRequestKey;
use codex_core::agent_tree::ipc::ExecApprovalAnswer;
use codex_core::agent_tree::ipc::NeedExecApproval;
use codex_core::agent_tree::ipc::NeedPatchApproval;
use codex_core::agent_tree::ipc::NeedUserInput;
use codex_core::agent_tree::ipc::PatchApprovalAnswer;
use codex_core::agent_tree::ipc::UserInputAnswer;
use codex_core::agent_tree::ipc::WorkRequest;
use codex_core::agent_tree::ipc::WorkerCommandResult;
use codex_core::agent_tree::ipc::WorkerError;
use codex_core::agent_tree::ipc::WorkerLog;
use codex_core::agent_tree::ipc::WorkerLogLevel;
use codex_core::agent_tree::ipc::WorkerResult;
use codex_core::agent_tree::prompts::L2_WORKER_DEVELOPER_INSTRUCTIONS;
use codex_core::config::Config;
use codex_core::features::Feature;
use codex_core::protocol::AgentStatus;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SessionSource;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_protocol::user_input::UserInput;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::sync::Mutex;
use tokio::sync::oneshot;
use uuid::Uuid;

pub(crate) async fn run(config: Config) -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let mut stdin_lines = BufReader::new(stdin).lines();

    let stdout = tokio::io::stdout();
    let stdout = Arc::new(Mutex::new(stdout));

    let Some(first_line) = stdin_lines
        .next_line()
        .await
        .context("read WorkRequest line")?
    else {
        send(
            &stdout,
            AgentTreeIpcMessage::Error(WorkerError {
                message: "missing WorkRequest".to_string(),
            }),
        )
        .await?;
        anyhow::bail!("missing WorkRequest");
    };

    let request: WorkRequest = match serde_json::from_str::<AgentTreeIpcMessage>(&first_line)
        .context("parse WorkRequest JSON")?
    {
        AgentTreeIpcMessage::WorkRequest(req) => req,
        other => {
            send(
                &stdout,
                AgentTreeIpcMessage::Error(WorkerError {
                    message: format!("expected work_request, got {other:?}"),
                }),
            )
            .await?;
            anyhow::bail!("expected WorkRequest");
        }
    };

    let pending_user_input: Arc<Mutex<HashMap<AgentTreeRequestKey, oneshot::Sender<_>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let pending_exec_approval: Arc<Mutex<HashMap<AgentTreeRequestKey, oneshot::Sender<_>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let pending_patch_approval: Arc<Mutex<HashMap<AgentTreeRequestKey, oneshot::Sender<_>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let stdin_task = {
        let stdout = stdout.clone();
        let pending_user_input = pending_user_input.clone();
        let pending_exec_approval = pending_exec_approval.clone();
        let pending_patch_approval = pending_patch_approval.clone();
        tokio::spawn(async move {
            while let Some(line) = stdin_lines.next_line().await.context("read IPC line")? {
                let msg =
                    serde_json::from_str::<AgentTreeIpcMessage>(&line).context("parse IPC JSON")?;
                match msg {
                    AgentTreeIpcMessage::UserInputAnswer(answer) => {
                        deliver_user_input_answer(&pending_user_input, answer).await;
                    }
                    AgentTreeIpcMessage::ExecApprovalAnswer(answer) => {
                        deliver_exec_approval_answer(&pending_exec_approval, answer).await;
                    }
                    AgentTreeIpcMessage::PatchApprovalAnswer(answer) => {
                        deliver_patch_approval_answer(&pending_patch_approval, answer).await;
                    }
                    AgentTreeIpcMessage::Log(_) => {}
                    AgentTreeIpcMessage::Error(err) => {
                        send(&stdout, AgentTreeIpcMessage::Error(err)).await?;
                        break;
                    }
                    other => {
                        send(
                            &stdout,
                            AgentTreeIpcMessage::Log(WorkerLog {
                                level: WorkerLogLevel::Warn,
                                message: format!("ignored IPC message: {other:?}"),
                            }),
                        )
                        .await?;
                    }
                }
            }
            Ok::<(), anyhow::Error>(())
        })
    };

    let repo_root = git_repo_root(&config.cwd)
        .await
        .context("resolve git root")?;

    let worktree_path = create_worktree(&config, &request, &repo_root)
        .await
        .context("create worktree")?;

    let mut worker_config = config.clone();
    worker_config.cwd = worktree_path.clone();
    worker_config
        .features
        .enable(Feature::Collab)
        .enable(Feature::ApplyPatchFreeform);
    worker_config.developer_instructions = Some(L2_WORKER_DEVELOPER_INSTRUCTIONS.to_string());

    let auth_manager = AuthManager::shared(
        worker_config.codex_home.clone(),
        true,
        worker_config.cli_auth_credentials_store_mode,
    );
    let thread_manager = Arc::new(ThreadManager::new(
        worker_config.codex_home.clone(),
        auth_manager.clone(),
        SessionSource::Exec,
    ));

    let mut thread_created_rx = thread_manager.subscribe_thread_created();
    let new_thread = thread_manager
        .start_thread(worker_config.clone())
        .await
        .context("start L2 thread")?;
    let l2_thread_id = new_thread.thread_id;
    let l2_thread = new_thread.thread;

    let commands: Arc<Mutex<Vec<WorkerCommandResult>>> = Arc::new(Mutex::new(Vec::new()));

    {
        let stdout = stdout.clone();
        send(
            &stdout,
            AgentTreeIpcMessage::Log(WorkerLog {
                level: WorkerLogLevel::Info,
                message: format!("L2 thread started: {l2_thread_id}"),
            }),
        )
        .await?;
    }

    // Spawn event drainers for existing threads and any subsequently created threads.
    let event_loop_cancel = tokio_util::sync::CancellationToken::new();
    let drain_task = {
        let stdout = stdout.clone();
        let pending_user_input = pending_user_input.clone();
        let pending_exec_approval = pending_exec_approval.clone();
        let pending_patch_approval = pending_patch_approval.clone();
        let commands = commands.clone();
        let thread_manager = thread_manager.clone();
        let cancel = event_loop_cancel.clone();
        tokio::spawn(async move {
            // Drain any existing threads (at minimum, the L2 thread).
            let existing = thread_manager.list_thread_ids().await;
            for id in existing {
                if let Ok(thread) = thread_manager.get_thread(id).await {
                    spawn_thread_event_drain(
                        id,
                        thread,
                        stdout.clone(),
                        pending_user_input.clone(),
                        pending_exec_approval.clone(),
                        pending_patch_approval.clone(),
                        commands.clone(),
                        cancel.clone(),
                    );
                }
            }

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    created = thread_created_rx.recv() => {
                        let Ok(id) = created else { break };
                        if let Ok(thread) = thread_manager.get_thread(id).await {
                            spawn_thread_event_drain(
                                id,
                                thread,
                                stdout.clone(),
                                pending_user_input.clone(),
                                pending_exec_approval.clone(),
                                pending_patch_approval.clone(),
                                commands.clone(),
                                cancel.clone(),
                            );
                        }
                    }
                }
            }
        })
    };

    let prompt = build_l2_prompt(&request);
    l2_thread
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: prompt,
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
        })
        .await
        .context("submit L2 prompt")?;

    let summary = wait_for_final_message(l2_thread.as_ref())
        .await
        .context("wait for L2 completion")?;

    event_loop_cancel.cancel();
    let _ = drain_task.await;

    let diff = generate_worktree_diff(&worktree_path)
        .await
        .context("generate worktree diff")?;
    let commands = commands.lock().await.clone();

    let result = WorkerResult {
        summary,
        diff,
        commands,
        worktree_path,
    };
    send(&stdout, AgentTreeIpcMessage::WorkerResult(result)).await?;

    // Ensure we drain stdin task to avoid broken pipe surprises; cancel by closing stdin.
    drop(stdin_task);

    Ok(())
}

fn build_l2_prompt(request: &WorkRequest) -> String {
    let mut prompt = String::new();
    prompt.push_str(&request.task);
    if let Some(context) = &request.context {
        if !context.trim().is_empty() {
            prompt.push_str("\n\n---\n\n");
            prompt.push_str("Context:\n");
            prompt.push_str(context);
        }
    }
    if !request.tests.is_empty() {
        prompt.push_str("\n\n---\n\n");
        prompt.push_str("Preferred tests:\n");
        for cmd in &request.tests {
            prompt.push_str("- ");
            prompt.push_str(cmd);
            prompt.push('\n');
        }
    }
    prompt
}

fn spawn_thread_event_drain(
    thread_id: codex_protocol::ThreadId,
    thread: Arc<codex_core::CodexThread>,
    stdout: Arc<Mutex<tokio::io::Stdout>>,
    pending_user_input: Arc<
        Mutex<HashMap<AgentTreeRequestKey, oneshot::Sender<RequestUserInputResponse>>>,
    >,
    pending_exec_approval: Arc<
        Mutex<
            HashMap<AgentTreeRequestKey, oneshot::Sender<codex_protocol::protocol::ReviewDecision>>,
        >,
    >,
    pending_patch_approval: Arc<
        Mutex<
            HashMap<AgentTreeRequestKey, oneshot::Sender<codex_protocol::protocol::ReviewDecision>>,
        >,
    >,
    commands: Arc<Mutex<Vec<WorkerCommandResult>>>,
    cancel: tokio_util::sync::CancellationToken,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                event = thread.next_event() => {
                    let event = match event {
                        Ok(event) => event,
                        Err(_) => break,
                    };
                    match event.msg {
                        EventMsg::RequestUserInput(ev) => {
                            let request_key = AgentTreeRequestKey::new(thread_id, event.id.clone());
                            if let Err(err) = handle_request_user_input(
                                &thread,
                                stdout.as_ref(),
                                &pending_user_input,
                                request_key,
                                ev,
                            )
                            .await
                            {
                                let _ = send(stdout.as_ref(), AgentTreeIpcMessage::Log(WorkerLog {
                                    level: WorkerLogLevel::Error,
                                    message: format!("request_user_input failed: {err:#}"),
                                }))
                                .await;
                            }
                        }
                        EventMsg::ExecApprovalRequest(ev) => {
                            let request_key = AgentTreeRequestKey::new(thread_id, event.id.clone());
                            if let Err(err) = handle_exec_approval(
                                &thread,
                                stdout.as_ref(),
                                &pending_exec_approval,
                                request_key,
                                ev,
                            )
                            .await
                            {
                                let _ = send(stdout.as_ref(), AgentTreeIpcMessage::Log(WorkerLog {
                                    level: WorkerLogLevel::Error,
                                    message: format!("exec approval failed: {err:#}"),
                                }))
                                .await;
                            }
                        }
                        EventMsg::ApplyPatchApprovalRequest(ev) => {
                            let request_key = AgentTreeRequestKey::new(thread_id, event.id.clone());
                            if let Err(err) = handle_patch_approval(
                                &thread,
                                stdout.as_ref(),
                                &pending_patch_approval,
                                request_key,
                                ev,
                            )
                            .await
                            {
                                let _ = send(stdout.as_ref(), AgentTreeIpcMessage::Log(WorkerLog {
                                    level: WorkerLogLevel::Error,
                                    message: format!("patch approval failed: {err:#}"),
                                }))
                                .await;
                            }
                        }
                        EventMsg::ExecCommandEnd(ev) => {
                            let cmd = ev.command.join(" ");
                            let output = truncate_output(&ev.formatted_output, 16_384);
                            commands.lock().await.push(WorkerCommandResult {
                                command: cmd,
                                exit_code: Some(ev.exit_code),
                                output,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
    });
}

async fn handle_request_user_input(
    thread: &codex_core::CodexThread,
    stdout: &Mutex<tokio::io::Stdout>,
    pending: &Mutex<HashMap<AgentTreeRequestKey, oneshot::Sender<RequestUserInputResponse>>>,
    request_key: AgentTreeRequestKey,
    ev: codex_protocol::protocol::RequestUserInputEvent,
) -> anyhow::Result<()> {
    let event_id = request_key.event_id.clone();
    let (tx, rx) = oneshot::channel();
    pending.lock().await.insert(request_key.clone(), tx);

    let args = codex_protocol::request_user_input::RequestUserInputArgs {
        questions: ev.questions,
    };
    send(
        stdout,
        AgentTreeIpcMessage::NeedUserInput(NeedUserInput { request_key, args }),
    )
    .await?;

    let response = rx.await.unwrap_or_else(|_| RequestUserInputResponse {
        answers: HashMap::new(),
    });
    thread
        .submit(Op::UserInputAnswer {
            id: event_id,
            response,
        })
        .await?;

    Ok(())
}

async fn handle_exec_approval(
    thread: &codex_core::CodexThread,
    stdout: &Mutex<tokio::io::Stdout>,
    pending: &Mutex<
        HashMap<AgentTreeRequestKey, oneshot::Sender<codex_protocol::protocol::ReviewDecision>>,
    >,
    request_key: AgentTreeRequestKey,
    ev: codex_protocol::protocol::ExecApprovalRequestEvent,
) -> anyhow::Result<()> {
    let event_id = request_key.event_id.clone();
    let (tx, rx) = oneshot::channel();
    pending.lock().await.insert(request_key.clone(), tx);

    send(
        stdout,
        AgentTreeIpcMessage::NeedExecApproval(NeedExecApproval {
            request_key,
            event: ev,
        }),
    )
    .await?;

    let decision = rx.await.unwrap_or_default();
    thread
        .submit(Op::ExecApproval {
            id: event_id,
            decision,
        })
        .await?;
    Ok(())
}

async fn handle_patch_approval(
    thread: &codex_core::CodexThread,
    stdout: &Mutex<tokio::io::Stdout>,
    pending: &Mutex<
        HashMap<AgentTreeRequestKey, oneshot::Sender<codex_protocol::protocol::ReviewDecision>>,
    >,
    request_key: AgentTreeRequestKey,
    ev: codex_protocol::protocol::ApplyPatchApprovalRequestEvent,
) -> anyhow::Result<()> {
    let event_id = request_key.event_id.clone();
    let (tx, rx) = oneshot::channel();
    pending.lock().await.insert(request_key.clone(), tx);

    send(
        stdout,
        AgentTreeIpcMessage::NeedPatchApproval(NeedPatchApproval {
            request_key,
            event: ev,
        }),
    )
    .await?;

    let decision = rx.await.unwrap_or_default();
    thread
        .submit(Op::PatchApproval {
            id: event_id,
            decision,
        })
        .await?;
    Ok(())
}

async fn deliver_user_input_answer(
    pending: &Mutex<HashMap<AgentTreeRequestKey, oneshot::Sender<RequestUserInputResponse>>>,
    answer: UserInputAnswer,
) {
    if let Some(tx) = pending.lock().await.remove(&answer.request_key) {
        let _ = tx.send(answer.response);
    }
}

async fn deliver_exec_approval_answer(
    pending: &Mutex<
        HashMap<AgentTreeRequestKey, oneshot::Sender<codex_protocol::protocol::ReviewDecision>>,
    >,
    answer: ExecApprovalAnswer,
) {
    if let Some(tx) = pending.lock().await.remove(&answer.request_key) {
        let _ = tx.send(answer.decision);
    }
}

async fn deliver_patch_approval_answer(
    pending: &Mutex<
        HashMap<AgentTreeRequestKey, oneshot::Sender<codex_protocol::protocol::ReviewDecision>>,
    >,
    answer: PatchApprovalAnswer,
) {
    if let Some(tx) = pending.lock().await.remove(&answer.request_key) {
        let _ = tx.send(answer.decision);
    }
}

async fn wait_for_final_message(thread: &codex_core::CodexThread) -> anyhow::Result<String> {
    loop {
        let status = thread.agent_status().await;
        if is_final_status(&status) {
            return Ok(match status {
                AgentStatus::Completed(Some(msg)) => msg,
                AgentStatus::Completed(None) => String::new(),
                AgentStatus::Errored(msg) => msg,
                AgentStatus::Shutdown => "shutdown".to_string(),
                AgentStatus::NotFound => "not found".to_string(),
                AgentStatus::PendingInit | AgentStatus::Running => unreachable!(),
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

fn is_final_status(status: &AgentStatus) -> bool {
    matches!(
        status,
        AgentStatus::Completed(_)
            | AgentStatus::Errored(_)
            | AgentStatus::Shutdown
            | AgentStatus::NotFound
    )
}

async fn create_worktree(
    config: &Config,
    request: &WorkRequest,
    repo_root: &Path,
) -> anyhow::Result<PathBuf> {
    let repo_name = repo_root
        .file_name()
        .map(|os| os.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    let task_id = Uuid::new_v4().to_string();
    let worktrees_root = config.codex_home.join("agent-tree").join("worktrees");
    let worktree_path = worktrees_root.join(repo_name).join(task_id);

    tokio::fs::create_dir_all(worktree_path.parent().unwrap()).await?;

    let base_ref = request.base_ref.as_deref().unwrap_or("HEAD");
    let worktree_path_arg = worktree_path.to_string_lossy().to_string();
    run_git_strict(
        repo_root,
        &["worktree", "add", "--detach", &worktree_path_arg, base_ref],
    )
    .await?;

    Ok(worktree_path)
}

async fn git_repo_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let out = run_git_strict(cwd, &["rev-parse", "--show-toplevel"])
        .await
        .context("git rev-parse --show-toplevel")?;
    Ok(PathBuf::from(out.trim()))
}

async fn generate_worktree_diff(worktree_path: &Path) -> anyhow::Result<String> {
    let mut combined = String::new();
    let tracked = run_git_allow_diff_exit_code(worktree_path, &["diff", "--no-color"]).await?;
    combined.push_str(&tracked);

    let untracked = run_git_strict(
        worktree_path,
        &["ls-files", "--others", "--exclude-standard"],
    )
    .await?;
    let dev_null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    for rel in untracked.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let diff = run_git_allow_diff_exit_code(
            worktree_path,
            &["diff", "--no-color", "--no-index", "--", dev_null, rel],
        )
        .await?;
        combined.push_str(&diff);
    }

    Ok(combined)
}

async fn run_git_strict(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = tokio::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .await
        .with_context(|| format!("run git {args:?}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn run_git_allow_diff_exit_code(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = tokio::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .await
        .with_context(|| format!("run git {args:?}"))?;

    let exit_code = output.status.code();
    let is_ok = matches!(exit_code, Some(0) | Some(1));
    if !is_ok {
        anyhow::bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn send(stdout: &Mutex<tokio::io::Stdout>, msg: AgentTreeIpcMessage) -> anyhow::Result<()> {
    let mut out = stdout.lock().await;
    let line = serde_json::to_string(&msg).context("serialize IPC message")?;
    out.write_all(line.as_bytes()).await?;
    out.write_all(b"\n").await?;
    out.flush().await?;
    Ok(())
}

fn truncate_output(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut out = text[..max_bytes].to_string();
    out.push_str("\n…(truncated)…\n");
    out
}
