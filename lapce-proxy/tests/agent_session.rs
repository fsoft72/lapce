//! Integration test: a real `AgentManager` talking to the mock ACP agent.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use lapce_proxy::agent::AgentManager;
use lapce_rpc::{
    RequestId, RpcError,
    agent::{AgentEvent, AgentServerConfig, AgentStatus},
    core::{CoreNotification, CoreRpc, CoreRpcHandler},
    proxy::{
        ProxyHandler, ProxyNotification, ProxyRequest, ProxyResponse,
        ProxyRpcHandler,
    },
};
use parking_lot::Mutex;

const EVENT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long the restart test watches for a stray `Disconnected` after the
/// second session reported `Ready`.
const QUIET_WINDOW: Duration = Duration::from_secs(2);

/// Stands in for the dispatcher: records agent writes and answers file requests.
struct FakeDispatcher {
    proxy_rpc: ProxyRpcHandler,
    writes: Arc<Mutex<Vec<(PathBuf, String)>>>,
}

impl ProxyHandler for FakeDispatcher {
    /// Ignores notifications: the test drives the manager directly.
    fn handle_notification(&mut self, _rpc: ProxyNotification) {}

    /// Answers the agent file requests the session sends through the proxy RPC.
    fn handle_request(&mut self, id: RequestId, rpc: ProxyRequest) {
        let result: Result<ProxyResponse, RpcError> = match rpc {
            ProxyRequest::AgentWriteFile { path, content } => {
                self.writes.lock().push((path, content));
                Ok(ProxyResponse::Success {})
            }
            ProxyRequest::AgentReadFile { .. } => {
                Ok(ProxyResponse::AgentReadFileResponse {
                    content: String::new(),
                })
            }
            _ => Err(RpcError {
                code: 0,
                message: "unsupported in test".to_string(),
            }),
        };
        self.proxy_rpc.handle_response(id, result);
    }
}

/// Test rig: manager plus fake dispatcher plus the core notification stream.
struct Rig {
    manager: AgentManager,
    core_rpc: CoreRpcHandler,
    proxy_rpc: ProxyRpcHandler,
    writes: Arc<Mutex<Vec<(PathBuf, String)>>>,
}

impl Rig {
    /// Builds the rig and starts the fake dispatcher thread.
    fn new() -> Self {
        let core_rpc = CoreRpcHandler::new();
        let proxy_rpc = ProxyRpcHandler::new();
        let writes = Arc::new(Mutex::new(Vec::new()));
        {
            let mut dispatcher = FakeDispatcher {
                proxy_rpc: proxy_rpc.clone(),
                writes: writes.clone(),
            };
            let proxy_rpc = proxy_rpc.clone();
            std::thread::spawn(move || proxy_rpc.mainloop(&mut dispatcher));
        }
        Rig {
            manager: AgentManager::new(core_rpc.clone(), proxy_rpc.clone()),
            core_rpc,
            proxy_rpc,
            writes,
        }
    }

    /// Waits up to `timeout` for the next agent event, skipping unrelated
    /// notifications. Returns `None` on timeout.
    fn try_next_event(&self, timeout: Duration) -> Option<AgentEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.core_rpc.rx().recv_timeout(left) {
                Ok(CoreRpc::Notification(n)) => {
                    if let CoreNotification::AgentEvent { event } = *n {
                        return Some(event);
                    }
                }
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }

    /// Waits for the next agent event, skipping unrelated notifications.
    fn next_event(&self) -> AgentEvent {
        self.try_next_event(EVENT_TIMEOUT)
            .expect("timed out waiting for an agent event")
    }

    /// Reads events until `pick` returns a value.
    fn wait_for<T>(&self, mut pick: impl FnMut(&AgentEvent) -> Option<T>) -> T {
        loop {
            let event = self.next_event();
            if let Some(found) = pick(&event) {
                return found;
            }
        }
    }
}

impl Drop for Rig {
    /// Stops the session and the fake dispatcher.
    fn drop(&mut self) {
        self.manager.stop();
        self.proxy_rpc.shutdown();
    }
}

/// Path of the mock agent example binary built by `cargo test`. Panics with a
/// hint when it is missing (a filtered `--test` run does not build examples).
fn mock_agent_command() -> String {
    let exe = std::env::current_exe().unwrap();
    let debug_dir = exe.parent().unwrap().parent().unwrap();
    let path = debug_dir
        .join("examples")
        .join(format!("mock_acp_agent{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.exists(),
        "mock agent not found at {}: run `cargo build -p lapce-proxy --example mock_acp_agent`",
        path.display()
    );
    path.to_string_lossy().to_string()
}

/// Agent server configuration that launches the mock agent.
fn mock_config() -> AgentServerConfig {
    AgentServerConfig {
        command: mock_agent_command(),
        args: vec![],
        env: Default::default(),
    }
}

/// True if the event is a `Disconnected` status.
fn is_disconnected(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::Status {
            status: AgentStatus::Disconnected { .. }
        }
    )
}

/// True if the event is a `Ready` status.
fn is_ready(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::Status {
            status: AgentStatus::Ready
        }
    )
}

#[test]
fn allowed_permission_lets_the_agent_write_and_finish() {
    let mut rig = Rig::new();
    rig.manager
        .start(mock_config(), Some(PathBuf::from("/mock")));
    rig.manager.prompt("go".to_string(), vec![]);

    let request_id = rig.wait_for(|event| match event {
        AgentEvent::PermissionRequest { request_id, .. } => Some(*request_id),
        _ => None,
    });
    rig.manager
        .permission_reply(request_id, Some("allow".to_string()));
    rig.wait_for(|event| {
        matches!(event, AgentEvent::TurnEnded { .. }).then_some(())
    });

    let writes = rig.writes.lock();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, PathBuf::from("/mock/out.txt"));
    assert_eq!(writes[0].1, "written by mock");
}

#[test]
fn rejected_permission_prevents_the_write() {
    let mut rig = Rig::new();
    rig.manager
        .start(mock_config(), Some(PathBuf::from("/mock")));
    rig.manager.prompt("go".to_string(), vec![]);

    let request_id = rig.wait_for(|event| match event {
        AgentEvent::PermissionRequest { request_id, .. } => Some(*request_id),
        _ => None,
    });
    rig.manager.permission_reply(request_id, None);
    rig.wait_for(|event| {
        matches!(event, AgentEvent::TurnEnded { .. }).then_some(())
    });

    assert!(rig.writes.lock().is_empty());
}

#[test]
fn stopping_with_a_pending_permission_does_not_hang_the_agent() {
    let mut rig = Rig::new();
    rig.manager
        .start(mock_config(), Some(PathBuf::from("/mock")));
    rig.manager.prompt("go".to_string(), vec![]);
    rig.wait_for(|event| {
        matches!(event, AgentEvent::PermissionRequest { .. }).then_some(())
    });

    rig.manager.stop();

    rig.wait_for(|event| is_disconnected(event).then_some(()));
    assert!(rig.writes.lock().is_empty());
}

#[test]
fn restarting_does_not_report_the_replaced_session_as_disconnected() {
    let mut rig = Rig::new();
    rig.manager
        .start(mock_config(), Some(PathBuf::from("/mock")));
    rig.wait_for(|event| is_ready(event).then_some(()));

    rig.manager
        .start(mock_config(), Some(PathBuf::from("/mock")));
    // Everything up to the second Ready must be free of Disconnected.
    loop {
        let event = rig.next_event();
        assert!(
            !is_disconnected(&event),
            "replaced session reported Disconnected before the new one was ready"
        );
        if is_ready(&event) {
            break;
        }
    }
    // And nothing stale may arrive shortly after either.
    let deadline = Instant::now() + QUIET_WINDOW;
    while let Some(event) =
        rig.try_next_event(deadline.saturating_duration_since(Instant::now()))
    {
        assert!(
            !is_disconnected(&event),
            "replaced session reported Disconnected after the new one was ready"
        );
    }
}

/// Lists the pids of processes whose full command line matches `pattern`.
#[cfg(unix)]
fn pids_matching(pattern: &str) -> Vec<String> {
    let out = std::process::Command::new("pgrep")
        .args(["-f", pattern])
        .output()
        .expect("pgrep must be available to run this test");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Polls until `check` holds or `timeout` elapses. Returns the last result.
#[cfg(unix)]
fn eventually(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if check() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(unix)]
#[test]
fn stopping_an_agent_that_never_answers_the_handshake_disconnects_and_kills_it() {
    // A unique argument so pgrep only sees the child of this test.
    const SILENT_ARG: &str = "3600.4242";
    let pattern = format!("sleep {SILENT_ARG}");
    let mut rig = Rig::new();
    rig.manager.start(
        AgentServerConfig {
            command: "sleep".to_string(),
            args: vec![SILENT_ARG.to_string()],
            env: Default::default(),
        },
        Some(PathBuf::from("/mock")),
    );
    assert!(
        eventually(EVENT_TIMEOUT, || !pids_matching(&pattern).is_empty()),
        "the silent agent was never spawned"
    );

    rig.manager.stop();

    rig.wait_for(|event| {
        assert!(!is_ready(event), "a silent agent cannot become ready");
        is_disconnected(event).then_some(())
    });
    assert!(
        eventually(Duration::from_secs(5), || pids_matching(&pattern)
            .is_empty()),
        "the silent agent process is still running after stop"
    );
}

#[cfg(unix)]
#[test]
fn stopping_a_ready_session_kills_the_agent_process() {
    // The mock ignores its arguments; this one only tags the process for pgrep.
    // No leading dashes: pgrep would parse the pattern as an option.
    const TAG_ARG: &str = "cleanup-tag-5151";
    let mut rig = Rig::new();
    let mut config = mock_config();
    config.args = vec![TAG_ARG.to_string()];
    rig.manager.start(config, Some(PathBuf::from("/mock")));
    rig.wait_for(|event| is_ready(event).then_some(()));
    assert!(
        !pids_matching(TAG_ARG).is_empty(),
        "the mock agent is not running after Ready"
    );

    rig.manager.stop();

    rig.wait_for(|event| is_disconnected(event).then_some(()));
    assert!(
        eventually(Duration::from_secs(5), || pids_matching(TAG_ARG).is_empty()),
        "the mock agent process is still running after stop"
    );
}

#[test]
fn write_outside_the_workspace_is_refused_by_the_path_guard() {
    let mut rig = Rig::new();
    // The mock writes /mock/out.txt, which is outside this workspace.
    rig.manager
        .start(mock_config(), Some(PathBuf::from("/other")));
    rig.manager.prompt("go".to_string(), vec![]);

    let request_id = rig.wait_for(|event| match event {
        AgentEvent::PermissionRequest { request_id, .. } => Some(*request_id),
        _ => None,
    });
    rig.manager
        .permission_reply(request_id, Some("allow".to_string()));
    // The refused write makes the mock fail its prompt, so the turn may end,
    // report an error, or the whole session may disconnect. All are acceptable;
    // what matters is that nothing was written.
    rig.wait_for(|event| {
        matches!(
            event,
            AgentEvent::TurnEnded { .. }
                | AgentEvent::Error { .. }
                | AgentEvent::Status {
                    status: AgentStatus::Disconnected { .. }
                }
        )
        .then_some(())
    });

    assert!(rig.writes.lock().is_empty());
}

#[test]
fn missing_agent_binary_reports_disconnected_with_a_reason() {
    let mut rig = Rig::new();
    rig.manager.start(
        AgentServerConfig {
            command: "definitely-not-a-real-agent-binary".to_string(),
            args: vec![],
            env: Default::default(),
        },
        Some(PathBuf::from("/mock")),
    );

    let reason = rig.wait_for(|event| match event {
        AgentEvent::Status {
            status: AgentStatus::Disconnected { reason },
        } => Some(reason.clone()),
        _ => None,
    });
    assert!(!reason.is_empty());
}
