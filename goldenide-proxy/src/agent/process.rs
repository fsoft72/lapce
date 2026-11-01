//! Kills the agent's process tree synchronously, from any thread.

use std::sync::Arc;

use parking_lot::Mutex;

/// What the handle knows about the process it guards.
#[derive(Default)]
struct ProcessSlot {
    /// Pid of the spawned agent, which is also its process group id on unix.
    /// `None` before the spawn and after the session released the process.
    pid: Option<u32>,
    /// Set by [`AgentProcess::kill`]; a process attached later is killed at once.
    killed: bool,
}

/// A shared handle to the agent process of one session.
///
/// The session attaches the pid right after spawning; the manager kills the
/// tree on stop without waiting for the session thread, which may be busy.
/// On unix the agent leads its own process group, so the whole tree
/// (`npx` and the `node` it starts) dies; on Windows `taskkill /T` does the same.
#[derive(Clone, Default)]
pub struct AgentProcess {
    slot: Arc<Mutex<ProcessSlot>>,
}

impl AgentProcess {
    /// Creates a handle with no process attached.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the pid of the freshly spawned agent. If the handle was
    /// already killed, the process is killed at once and `false` is returned.
    pub fn attach(&self, pid: u32) -> bool {
        let mut slot = self.slot.lock();
        if slot.killed {
            kill_tree(pid);
            return false;
        }
        slot.pid = Some(pid);
        true
    }

    /// Kills the attached process tree now and forgets the pid, so a later
    /// call cannot hit a recycled pid. Returns whether a process was attached.
    pub fn release(&self) -> bool {
        let Some(pid) = self.slot.lock().pid.take() else {
            return false;
        };
        kill_tree(pid);
        true
    }

    /// Marks the handle killed and kills the attached process tree, if any.
    pub fn kill(&self) {
        let mut slot = self.slot.lock();
        slot.killed = true;
        if let Some(pid) = slot.pid.take() {
            kill_tree(pid);
        }
    }

    /// True once [`AgentProcess::kill`] was called.
    pub fn was_killed(&self) -> bool {
        self.slot.lock().killed
    }
}

/// Kills the process group led by `pid`. Errors (the group is already gone)
/// are ignored on purpose: the goal is only that nothing survives.
#[cfg(unix)]
fn kill_tree(pid: u32) {
    let Ok(pgid) = libc::pid_t::try_from(pid) else {
        return;
    };
    // SAFETY: killpg only sends a signal; no memory is shared with the callee.
    unsafe {
        libc::killpg(pgid, libc::SIGKILL);
    }
}

/// Kills the process `pid` and its children with `taskkill`, waiting for it
/// to finish. A failure is logged: the tree may be gone already.
#[cfg(windows)]
fn kill_tree(pid: u32) {
    use std::os::windows::process::CommandExt;

    /// Keeps `taskkill` from flashing a console window.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let result = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if let Err(err) = result {
        tracing::warn!("cannot run taskkill for agent process {pid}: {err}");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    use super::*;

    /// Spawns `sleep` as the leader of its own process group.
    fn spawn_group_leader() -> std::process::Child {
        use std::os::unix::process::CommandExt;
        Command::new("sleep")
            .arg("30")
            .stdout(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap()
    }

    /// Waits up to five seconds for the child to exit.
    fn exits(child: &mut std::process::Child) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn kill_terminates_the_attached_process() {
        let mut child = spawn_group_leader();
        let process = AgentProcess::new();
        assert!(process.attach(child.id()));
        process.kill();
        assert!(process.was_killed());
        assert!(exits(&mut child));
    }

    #[test]
    fn a_process_attached_after_kill_dies_at_once() {
        let mut child = spawn_group_leader();
        let process = AgentProcess::new();
        process.kill();
        assert!(!process.attach(child.id()));
        assert!(exits(&mut child));
    }

    #[test]
    fn release_kills_once_and_forgets_the_pid() {
        let mut child = spawn_group_leader();
        let process = AgentProcess::new();
        assert!(process.attach(child.id()));
        assert!(process.release());
        assert!(!process.release());
        assert!(!process.was_killed());
        assert!(exits(&mut child));
    }
}
