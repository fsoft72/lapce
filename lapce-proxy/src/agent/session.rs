//! One ACP client session: spawns the agent, serves its requests, runs prompts.

use std::{collections::VecDeque, path::PathBuf, sync::Arc, time::Duration};

use agent_client_protocol::{
    AcpAgent, Agent, ByteStreams, Client, ConnectionTo, Error as AcpError,
    schema::{
        ProtocolVersion,
        v1::{
            CancelNotification, ClientCapabilities, ContentBlock,
            FileSystemCapabilities, InitializeRequest, NewSessionRequest,
            PromptRequest, ReadTextFileRequest, ReadTextFileResponse,
            RequestPermissionOutcome, RequestPermissionRequest,
            RequestPermissionResponse, SelectedPermissionOutcome,
            SessionNotification, StopReason, TextContent, WriteTextFileRequest,
            WriteTextFileResponse,
        },
    },
};
use anyhow::{Result, anyhow};
use futures::{
    AsyncRead, AsyncReadExt, FutureExt, StreamExt,
    channel::{mpsc, oneshot},
    select,
};
use lapce_rpc::{
    agent::{
        AgentContext, AgentEvent, AgentPermissionOption, AgentServerConfig,
        AgentStatus,
    },
    core::{CoreNotification, CoreRpcHandler},
    proxy::ProxyRpcHandler,
};
use parking_lot::Mutex;

use super::{
    mapping::{map_permission_options, map_update, permission_title},
    paths::{resolve_workspace_path, slice_lines},
    permission::PermissionBroker,
    process::AgentProcess,
    prompt::{ResolvedContext, build_prompt_text},
};

/// How many trailing bytes of the agent's stderr are kept for error messages.
const STDERR_TAIL_LIMIT: usize = 2048;

/// How long to wait for the rest of the agent's stderr after it exited.
const STDERR_GRACE: Duration = Duration::from_millis(500);

/// Stop reason reported when a prompt queued during the handshake is cancelled.
const CANCELLED_STOP_REASON: &str = "Cancelled";

/// Commands the manager sends to a running session.
pub enum SessionCommand {
    /// Runs one turn with the user text and the editor context.
    Prompt {
        text: String,
        contexts: Vec<AgentContext>,
    },
    /// Cancels the running turn and any pending permission request.
    Cancel,
}

/// Everything a session needs from the proxy.
pub struct SessionEnv {
    /// Channel to the UI.
    pub core_rpc: CoreRpcHandler,
    /// Channel to the dispatcher, used for file access.
    pub proxy_rpc: ProxyRpcHandler,
    /// Pending permission requests awaiting a user decision.
    pub broker: Arc<PermissionBroker>,
    /// Root the agent is confined to.
    pub workspace: PathBuf,
    /// Generation of the current session, shared with the manager.
    pub generation: Arc<Mutex<u64>>,
    /// Generation this session was started with.
    pub session_generation: u64,
    /// The agent process, shared with the manager so stop can kill it at once.
    pub process: AgentProcess,
}

impl SessionEnv {
    /// Sends an event to the UI, unless a newer session replaced this one.
    /// The check and the send happen under the generation lock, so a stale
    /// event cannot land after the new session's `Starting`. Returns whether
    /// the event was sent.
    pub fn emit(&self, event: AgentEvent) -> bool {
        let current = self.generation.lock();
        if *current != self.session_generation {
            return false;
        }
        self.core_rpc
            .notification(CoreNotification::AgentEvent { event });
        true
    }

    /// Registers a permission request with the broker and shows it in the UI.
    /// If this session is stale the request is rejected at once, so the agent
    /// sees a cancellation instead of waiting on a prompt nobody can answer.
    fn open_permission(
        &self,
        title: String,
        options: Vec<AgentPermissionOption>,
    ) -> oneshot::Receiver<Option<String>> {
        let (request_id, decision) = self.broker.register();
        let shown = self.emit(AgentEvent::PermissionRequest {
            request_id,
            title,
            options,
        });
        if !shown {
            self.broker.reply(request_id, None);
        }
        decision
    }

    /// Serves `fs/read_text_file`: guard the path, read through the dispatcher, slice lines.
    fn read_file(&self, req: &ReadTextFileRequest) -> Result<String> {
        let path = resolve_workspace_path(&self.workspace, &req.path)?;
        let content = self
            .proxy_rpc
            .agent_read_file(path)
            .map_err(|err| anyhow!(err.message))?;
        Ok(slice_lines(&content, req.line, req.limit))
    }

    /// Serves `fs/write_text_file`: guard the path, then write through the dispatcher.
    fn write_file(&self, req: &WriteTextFileRequest) -> Result<()> {
        let path = resolve_workspace_path(&self.workspace, &req.path)?;
        self.proxy_rpc
            .agent_write_file(path, req.content.clone())
            .map_err(|err| anyhow!(err.message))
    }

    /// Reads the text of each context item. Items that cannot be read are skipped.
    fn resolve_contexts(&self, contexts: Vec<AgentContext>) -> Vec<ResolvedContext> {
        contexts
            .into_iter()
            .map(|ctx| {
                let content = match ctx.selection {
                    Some(_) => None,
                    None => resolve_workspace_path(&self.workspace, &ctx.path)
                        .ok()
                        .and_then(|path| self.proxy_rpc.agent_read_file(path).ok()),
                };
                ResolvedContext {
                    path: ctx.path,
                    selection: ctx.selection,
                    content,
                }
            })
            .collect()
    }
}

/// Converts an internal error into an ACP internal error carrying the message.
fn to_acp_error(err: anyhow::Error) -> AcpError {
    AcpError::internal_error().data(format!("{err:#}"))
}

/// Reads the agent's stderr until it closes, keeping only the last
/// [`STDERR_TAIL_LIMIT`] bytes. Draining also keeps a chatty agent from
/// blocking on a full pipe.
async fn drain_stderr(
    mut stderr: impl AsyncRead + Unpin,
    tail: Arc<Mutex<VecDeque<u8>>>,
) {
    let mut buf = [0u8; 1024];
    loop {
        let read = match stderr.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        let mut tail = tail.lock();
        tail.extend(&buf[..read]);
        let excess = tail.len().saturating_sub(STDERR_TAIL_LIMIT);
        tail.drain(..excess);
    }
}

/// A future that resolves after `duration`, driven by a helper thread
/// because the session runs on a plain executor without timers.
fn delay(duration: Duration) -> oneshot::Receiver<()> {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(duration);
        let _ = tx.send(());
    });
    rx
}

/// Builds the error reported when the agent process exits while the
/// connection is still open, including the end of its stderr.
fn exit_error(
    status: std::io::Result<std::process::ExitStatus>,
    tail: &Mutex<VecDeque<u8>>,
) -> AcpError {
    let status = match status {
        Ok(status) => format!("the agent exited ({status})"),
        Err(err) => format!("cannot wait for the agent: {err}"),
    };
    let bytes = tail.lock().iter().copied().collect::<Vec<_>>();
    let stderr = String::from_utf8_lossy(&bytes).trim().to_string();
    let message = if stderr.is_empty() {
        status
    } else {
        format!("{status}: {stderr}")
    };
    AcpError::internal_error().data(message)
}

/// Runs one agent session until the command channel closes or the agent dies.
///
/// The process is spawned here rather than by the ACP crate so its pid can be
/// handed to [`AgentProcess`]: stop then kills the tree synchronously instead
/// of relying on this thread to notice and drop the crate's guard.
pub async fn run_session(
    config: AgentServerConfig,
    env: Arc<SessionEnv>,
    cmds: mpsc::UnboundedReceiver<SessionCommand>,
) -> Result<(), AcpError> {
    let agent = AcpAgent::new(
        agent_client_protocol::AcpAgentConfig::new(config.command)
            .args(config.args)
            .envs(config.env),
    );
    let (stdin, stdout, stderr, mut child) = agent.spawn_process()?;
    if !env.process.attach(child.id()) {
        return Ok(());
    }
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let drain = drain_stderr(stderr, tail.clone()).fuse();
    let connection =
        connect(ByteStreams::new(stdin, stdout), env.clone(), cmds).fuse();
    let exit = child.status().fuse();
    futures::pin_mut!(drain, connection, exit);
    let result = loop {
        select! {
            res = connection => break res,
            status = exit => {
                // Kill what is left of the tree so stderr closes, then give
                // the drain a moment to collect the last lines.
                env.process.release();
                let grace = delay(STDERR_GRACE).fuse();
                futures::pin_mut!(grace);
                select! {
                    () = drain => {},
                    _ = grace => {},
                }
                break Err(exit_error(status, &tail));
            }
            () = drain => {},
        }
    };
    env.process.release();
    result
}

/// Serves the ACP connection to a spawned agent over `transport`.
async fn connect(
    transport: impl agent_client_protocol::ConnectTo<Client> + 'static,
    env: Arc<SessionEnv>,
    mut cmds: mpsc::UnboundedReceiver<SessionCommand>,
) -> Result<(), AcpError> {
    Client
        .builder()
        .on_receive_notification(
            {
                let env = env.clone();
                async move |notification: SessionNotification, _cx| {
                    if let Some(event) = map_update(notification.update) {
                        env.emit(event);
                    }
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            {
                let env = env.clone();
                async move |req: ReadTextFileRequest, responder, _cx| {
                    match env.read_file(&req) {
                        Ok(content) => responder.respond(ReadTextFileResponse::new(content)),
                        Err(err) => responder.respond_with_error(to_acp_error(err)),
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let env = env.clone();
                async move |req: WriteTextFileRequest, responder, _cx| {
                    match env.write_file(&req) {
                        Ok(()) => responder.respond(WriteTextFileResponse::new()),
                        Err(err) => responder.respond_with_error(to_acp_error(err)),
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let env = env.clone();
                async move |req: RequestPermissionRequest, responder, cx| {
                    // Never await the user inside a handler: it would freeze the event loop.
                    let decision = env.open_permission(
                        permission_title(&req),
                        map_permission_options(&req),
                    );
                    cx.spawn(async move {
                        let outcome = match decision.await {
                            Ok(Some(option_id)) => RequestPermissionOutcome::Selected(
                                SelectedPermissionOutcome::new(option_id),
                            ),
                            _ => RequestPermissionOutcome::Cancelled,
                        };
                        responder.respond(RequestPermissionResponse::new(outcome))
                    })
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, {
            let env = env.clone();
            async move |connection: ConnectionTo<Agent>| {
                let handshake = async {
                    connection
                        .send_request(
                            InitializeRequest::new(ProtocolVersion::V1)
                                .client_capabilities(ClientCapabilities::new().fs(
                                    FileSystemCapabilities::new()
                                        .read_text_file(true)
                                        .write_text_file(true),
                                )),
                        )
                        .block_task()
                        .await?;
                    let session = connection
                        .send_request(NewSessionRequest::new(env.workspace.clone()))
                        .block_task()
                        .await?;
                    Ok::<_, AcpError>(session.session_id)
                }
                .fuse();
                futures::pin_mut!(handshake);
                // Watch the commands during the handshake too, so stop can
                // interrupt an agent that never answers. There is deliberately
                // no timeout: a first `npx` download can be legitimately slow.
                let mut queued = VecDeque::new();
                let session_id = loop {
                    select! {
                        res = handshake => break res?,
                        cmd = cmds.next() => match cmd {
                            Some(prompt @ SessionCommand::Prompt { .. }) => {
                                queued.push_back(prompt);
                            }
                            Some(SessionCommand::Cancel) => {
                                // The UI marked itself busy when it sent the
                                // prompt; end that turn or it never clears.
                                if !queued.is_empty() {
                                    queued.clear();
                                    env.emit(AgentEvent::TurnEnded {
                                        stop_reason: CANCELLED_STOP_REASON.to_string(),
                                    });
                                }
                            }
                            None => return Ok(()),
                        },
                    }
                };
                env.emit(AgentEvent::Status {
                    status: AgentStatus::Ready,
                });

                loop {
                    let cmd = match queued.pop_front() {
                        Some(cmd) => cmd,
                        None => match cmds.next().await {
                            Some(cmd) => cmd,
                            None => break,
                        },
                    };
                    let SessionCommand::Prompt { text, contexts } = cmd else {
                        continue;
                    };
                    let contexts = env.resolve_contexts(contexts);
                    let prompt = build_prompt_text(&text, &contexts);
                    let turn = connection
                        .send_request(PromptRequest::new(
                            session_id.clone(),
                            vec![ContentBlock::Text(TextContent::new(prompt))],
                        ))
                        .block_task()
                        .fuse();
                    futures::pin_mut!(turn);
                    let outcome: Result<StopReason, AcpError> = loop {
                        select! {
                            res = turn => break res.map(|response| response.stop_reason),
                            cmd = cmds.next() => match cmd {
                                Some(SessionCommand::Cancel) => {
                                    env.broker.cancel_all();
                                    connection.send_notification(
                                        CancelNotification::new(session_id.clone()),
                                    )?;
                                }
                                Some(SessionCommand::Prompt { .. }) => {}
                                None => return Ok(()),
                            },
                        }
                    };
                    let event = match outcome {
                        Ok(stop) => AgentEvent::TurnEnded {
                            stop_reason: format!("{stop:?}"),
                        },
                        Err(err) => AgentEvent::Error {
                            message: format!("Prompt failed: {err}"),
                        },
                    };
                    env.emit(event);
                }
                Ok(())
            }
        })
        .await
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use lapce_rpc::core::CoreRpc;

    use super::*;

    /// Builds a session env of generation 1 with the given shared generation.
    fn env_with(generation: Arc<Mutex<u64>>) -> SessionEnv {
        SessionEnv {
            core_rpc: CoreRpcHandler::new(),
            proxy_rpc: ProxyRpcHandler::new(),
            broker: Arc::new(PermissionBroker::new()),
            workspace: PathBuf::from("/ws"),
            generation,
            session_generation: 1,
            process: AgentProcess::new(),
        }
    }

    /// Returns the next agent event already queued for the UI, if any.
    fn queued_event(env: &SessionEnv) -> Option<AgentEvent> {
        match env.core_rpc.rx().try_recv().ok()? {
            CoreRpc::Notification(n) => match *n {
                CoreNotification::AgentEvent { event } => Some(event),
                _ => None,
            },
            _ => None,
        }
    }

    #[test]
    fn current_session_emits_events() {
        let env = env_with(Arc::new(Mutex::new(1)));
        assert!(env.emit(AgentEvent::TurnEnded {
            stop_reason: "EndTurn".to_string(),
        }));
        assert!(matches!(
            queued_event(&env),
            Some(AgentEvent::TurnEnded { .. })
        ));
    }

    #[test]
    fn replaced_session_events_are_dropped() {
        let generation = Arc::new(Mutex::new(1));
        let env = env_with(generation.clone());
        *generation.lock() = 2;
        assert!(!env.emit(AgentEvent::TurnEnded {
            stop_reason: "EndTurn".to_string(),
        }));
        assert!(queued_event(&env).is_none());
    }

    #[test]
    fn current_session_permission_is_shown_and_awaits_the_user() {
        let env = env_with(Arc::new(Mutex::new(1)));
        let mut decision = env.open_permission("Edit".to_string(), vec![]);
        let Some(AgentEvent::PermissionRequest { request_id, .. }) =
            queued_event(&env)
        else {
            panic!("the permission request was not shown");
        };
        assert_eq!(decision.try_recv(), Ok(None), "must still be pending");
        assert!(env.broker.reply(request_id, Some("allow".to_string())));
        assert_eq!(block_on(decision), Ok(Some("allow".to_string())));
    }

    #[test]
    fn replaced_session_permission_is_rejected_without_a_prompt() {
        let generation = Arc::new(Mutex::new(1));
        let env = env_with(generation.clone());
        *generation.lock() = 2;
        let decision = env.open_permission("Edit".to_string(), vec![]);
        assert!(queued_event(&env).is_none());
        assert_eq!(block_on(decision), Ok(None));
    }
}
