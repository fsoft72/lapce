//! One ACP client session: spawns the agent, serves its requests, runs prompts.

use std::{path::PathBuf, sync::Arc};

use agent_client_protocol::{
    AcpAgent, Agent, Client, ConnectionTo, Error as AcpError,
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
use futures::{FutureExt, StreamExt, channel::mpsc, select};
use lapce_rpc::{
    agent::{AgentContext, AgentEvent, AgentServerConfig, AgentStatus},
    core::{CoreNotification, CoreRpcHandler},
    proxy::ProxyRpcHandler,
};

use super::{
    mapping::{map_permission_options, map_update, permission_title},
    paths::{resolve_workspace_path, slice_lines},
    permission::PermissionBroker,
    prompt::{ResolvedContext, build_prompt_text},
};

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
}

impl SessionEnv {
    /// Sends an event to the UI.
    fn emit(&self, event: AgentEvent) {
        self.core_rpc
            .notification(CoreNotification::AgentEvent { event });
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

/// Runs one agent session until the command channel closes or the agent dies.
pub async fn run_session(
    config: AgentServerConfig,
    env: Arc<SessionEnv>,
    mut cmds: mpsc::UnboundedReceiver<SessionCommand>,
) -> Result<(), AcpError> {
    let agent = AcpAgent::new(
        agent_client_protocol::AcpAgentConfig::new(config.command)
            .args(config.args)
            .envs(config.env),
    );
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
                    let (request_id, decision) = env.broker.register();
                    env.emit(AgentEvent::PermissionRequest {
                        request_id,
                        title: permission_title(&req),
                        options: map_permission_options(&req),
                    });
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
        .connect_with(agent, {
            let env = env.clone();
            async move |connection: ConnectionTo<Agent>| {
                connection
                    .send_request(
                        InitializeRequest::new(ProtocolVersion::V1).client_capabilities(
                            ClientCapabilities::new().fs(FileSystemCapabilities::new()
                                .read_text_file(true)
                                .write_text_file(true)),
                        ),
                    )
                    .block_task()
                    .await?;
                let session = connection
                    .send_request(NewSessionRequest::new(env.workspace.clone()))
                    .block_task()
                    .await?;
                let session_id = session.session_id;
                env.emit(AgentEvent::Status {
                    status: AgentStatus::Ready,
                });

                while let Some(cmd) = cmds.next().await {
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
                    match outcome {
                        Ok(stop) => env.emit(AgentEvent::TurnEnded {
                            stop_reason: format!("{stop:?}"),
                        }),
                        Err(err) => env.emit(AgentEvent::Error {
                            message: format!("Prompt failed: {err}"),
                        }),
                    }
                }
                Ok(())
            }
        })
        .await
}
