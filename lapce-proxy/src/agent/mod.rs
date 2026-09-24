//! AI agent (ACP client) support.

pub mod fs;
pub mod mapping;
pub mod paths;
pub mod permission;
pub mod prompt;
pub mod session;

use std::{path::PathBuf, sync::Arc};

use futures::channel::mpsc;
use lapce_rpc::{
    agent::{
        AgentContext, AgentEvent, AgentRequestId, AgentServerConfig, AgentStatus,
    },
    core::{CoreNotification, CoreRpcHandler},
    proxy::ProxyRpcHandler,
};
use parking_lot::Mutex;
use permission::PermissionBroker;
use session::{SessionCommand, SessionEnv, run_session};

/// Owns the agent session thread and forwards UI commands to it.
pub struct AgentManager {
    core_rpc: CoreRpcHandler,
    proxy_rpc: ProxyRpcHandler,
    broker: Arc<PermissionBroker>,
    cmd_tx: Option<mpsc::UnboundedSender<SessionCommand>>,
    /// Generation of the current session. Only `start` bumps it, so a session
    /// replaced by a restart stays silent when it ends, while a stopped one
    /// still reports `Disconnected`. The lock also orders the old session's
    /// final event against the new session's `Starting`.
    generation: Arc<Mutex<u64>>,
}

impl AgentManager {
    /// Creates a manager with no running session.
    pub fn new(core_rpc: CoreRpcHandler, proxy_rpc: ProxyRpcHandler) -> Self {
        Self {
            core_rpc,
            proxy_rpc,
            broker: Arc::new(PermissionBroker::new()),
            cmd_tx: None,
            generation: Arc::new(Mutex::new(0)),
        }
    }

    /// Sends an event to the UI.
    fn emit(&self, event: AgentEvent) {
        self.core_rpc
            .notification(CoreNotification::AgentEvent { event });
    }

    /// Starts a session, replacing any running one. The session works on `workspace`.
    pub fn start(&mut self, config: AgentServerConfig, workspace: Option<PathBuf>) {
        self.stop();
        let Some(workspace) = workspace else {
            self.emit(AgentEvent::Error {
                message: "Open a folder before starting the agent".to_string(),
            });
            return;
        };
        let session_generation = {
            let mut generation = self.generation.lock();
            *generation += 1;
            // Emitted under the lock so a replaced session cannot slip its
            // final event in after this one.
            self.emit(AgentEvent::Status {
                status: AgentStatus::Starting,
            });
            *generation
        };
        let (cmd_tx, cmd_rx) = mpsc::unbounded();
        self.cmd_tx = Some(cmd_tx);
        let env = Arc::new(SessionEnv {
            core_rpc: self.core_rpc.clone(),
            proxy_rpc: self.proxy_rpc.clone(),
            broker: self.broker.clone(),
            workspace,
        });
        let core_rpc = self.core_rpc.clone();
        let generation = self.generation.clone();
        std::thread::Builder::new()
            .name("AgentSession".to_owned())
            .spawn(move || {
                let result =
                    futures::executor::block_on(run_session(config, env, cmd_rx));
                let reason = match result {
                    Ok(()) => "session closed".to_string(),
                    Err(err) => format!("{err}"),
                };
                let current = generation.lock();
                if *current != session_generation {
                    return;
                }
                core_rpc.notification(CoreNotification::AgentEvent {
                    event: AgentEvent::Status {
                        status: AgentStatus::Disconnected { reason },
                    },
                });
            })
            .expect("failed to spawn the agent session thread");
    }

    /// Sends a prompt to the running session.
    pub fn prompt(&self, text: String, contexts: Vec<AgentContext>) {
        let Some(tx) = &self.cmd_tx else {
            return self.emit(AgentEvent::Error {
                message: "The agent is not running".to_string(),
            });
        };
        if tx
            .unbounded_send(SessionCommand::Prompt { text, contexts })
            .is_err()
        {
            self.emit(AgentEvent::Error {
                message: "The agent is not running".to_string(),
            });
        }
    }

    /// Cancels the running turn.
    pub fn cancel(&self) {
        let Some(tx) = &self.cmd_tx else { return };
        let _ = tx.unbounded_send(SessionCommand::Cancel);
    }

    /// Answers a pending permission request. `None` rejects it.
    pub fn permission_reply(
        &self,
        request_id: AgentRequestId,
        option_id: Option<String>,
    ) {
        self.broker.reply(request_id, option_id);
    }

    /// Stops the session: rejects pending permissions and closes the command channel.
    pub fn stop(&mut self) {
        self.broker.cancel_all();
        self.cmd_tx = None;
    }
}
