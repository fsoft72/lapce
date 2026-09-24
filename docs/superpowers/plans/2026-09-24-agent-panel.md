# AI Agent Panel (ACP client) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an agent panel to Lapce that talks to an external ACP agent (Claude Code adapter, Gemini CLI) running in `lapce-proxy`, with edits applied to editor buffers and per-request approval.

**Architecture:** `lapce-proxy` owns the agent subprocess and the ACP session (official `agent-client-protocol` crate, driven by `futures::executor::block_on` on a dedicated thread, because the proxy is synchronous and the crate is runtime agnostic). The agent's file reads go through the dispatcher (which owns the synced buffers). Writes to open files go to the UI as a notification and are applied as one undoable edit; writes to closed files go straight to disk. The UI holds a plain `AgentState` (unit testable) inside a signal and renders it in a new `PanelKind::Agent`.

**Tech Stack:** Rust 1.98 (edition 2024), floem UI, `agent-client-protocol = "=2.2.0"` (schema v1), `futures` 0.3, crossbeam channels.

**Spec:** `docs/superpowers/specs/2026-09-24-agent-panel-design.md`

## Global Constraints

- Never use the long dash character in text, code or comments. Use `-`.
- All comments, names and messages in English.
- Every function and method gets a doc comment (`///`). Guard clauses (early returns) at the top of functions, one-liners when possible.
- Constants are `UPPER_SNAKE_CASE`. Follow existing project conventions and run `cargo fmt` and `cargo clippy` on touched crates before each commit.
- Text files end with an empty line.
- Each task appends 1-3 lines to a `## AI agent panel` section of `CHANGES.md` (create the section in Task 1) and includes `CHANGES.md` in that task's commit.
- One commit per task, only files you changed. Do not push. No AI attribution lines in commit messages. Never use `--no-verify`. Do not commit code that does not compile.
- Never disable or delete tests to make them pass. Stop after 3 failed attempts at the same problem and reassess.
- Pinned protocol crate: `agent-client-protocol = "=2.2.0"`, using the `schema::v1` types. Its schema types are `#[non_exhaustive]`, so always build them with `::new(..)` and builder methods, never with struct literals.
- The crate runs all handler callbacks on one event-loop task. A handler must never await a user action or another peer response inline. It must `cx.spawn(..)` and respond from the spawned task.
- ACP terminal capability is NOT advertised in v1 (see "Spec deltas").

## Spec deltas found during planning

These are corrections to the approved spec, discovered while reading the code. They are written into the spec at the start of Task 1.

1. **Terminal requests are out of v1.** `ClientCapabilities.terminal` stays `false`; the agent uses its own shell tool and asks for permission through `session/request_permission`. Mapping ACP `terminal/*` onto Lapce's PTY terminal is a separate feature.
2. **Permissions come from the agent.** ACP has no client-side gate on `fs/write_text_file`. The agent asks first with `session/request_permission`; Lapce shows that prompt. Lapce additionally enforces a workspace path guard on every read and write.
3. **File routing.** `CoreRequest` is an empty enum and the UI has no reply path, so a "proxy asks UI" request/response is not available. Instead: reads run in the dispatcher (which already has synced buffers via `ProxyNotification::Update`); writes to an open file send `CoreNotification::AgentApplyEdit` to the UI, writes to a closed file are written to disk by the proxy.
4. **Open questions resolved.** The ACP crate has no tokio dependency, so `futures::executor::block_on` on a dedicated thread fits the proxy. The proxy does hold synced buffer copies, but they live inside the single-threaded dispatcher, hence the new `ProxyRequest`s. Default panel position: right side (`RightTop`).
5. **Known limitation.** After an agent write to an open buffer, the proxy copy of that buffer updates when the UI's `Update` notification arrives, so an immediate re-read by the agent can briefly see the old text.
6. **Context in v1 is the active file, or its selection when there is one.** `@file` mentions and context chips are deferred. Enter sends the prompt; multi-line input is not supported yet.

## Review Focus

Inputs and conditions the spec implies but does not test directly. Each has a test in the owning task.

1. **Path escape:** agent asks to read or write `../secret`, a sibling directory sharing the workspace prefix (`/ws-evil` vs `/ws`), or a path that escapes through a symlink. Expected: request rejected with an error to the agent, nothing read or written. (Task 3)
2. **Agent dies or never starts:** command not found, or process exits mid-turn. Expected: panel shows a disconnected message with the reason, the UI is not stuck in "busy", a restart works. (Tasks 7 and 8)
3. **Permission pending when the user cancels or stops:** expected: the agent receives a cancelled outcome and does not hang. (Tasks 2 and 8)
4. **Open file with unsaved edits:** the agent must read the unsaved text, and its write must land as an undoable buffer edit, not silently on disk. (Task 6)
5. **Big or non-UTF-8 files:** a non-UTF-8 read returns an error instead of panicking, and a huge context file is truncated on a char boundary. (Tasks 4 and 6)

## File Structure

Create:
- `lapce-rpc/src/agent.rs`: shared types (`AgentServerConfig`, `AgentContext`, `AgentEvent`, ...).
- `lapce-proxy/src/agent/mod.rs`: `AgentManager` (thread and channel lifecycle).
- `lapce-proxy/src/agent/permission.rs`: `PermissionBroker` (pending permission replies).
- `lapce-proxy/src/agent/paths.rs`: workspace path guard and `slice_lines`.
- `lapce-proxy/src/agent/prompt.rs`: builds prompt text from user text plus context.
- `lapce-proxy/src/agent/mapping.rs`: ACP updates to `AgentEvent`.
- `lapce-proxy/src/agent/fs.rs`: `read_text` and `write_text` used by the dispatcher.
- `lapce-proxy/src/agent/session.rs`: the ACP client session driver.
- `lapce-proxy/examples/mock_acp_agent.rs`: mock agent for the integration test.
- `lapce-proxy/tests/agent_session.rs`: integration test.
- `lapce-app/src/config/agent.rs`: `AgentConfig` (settings).
- `lapce-app/src/agent.rs`: `AgentState`, `AgentData`.
- `lapce-app/src/panel/agent_view.rs`: floem view.

Modify: `lapce-rpc/src/{lib,proxy,core}.rs`, `lapce-proxy/{Cargo.toml,src/lib.rs,src/dispatch.rs}`, `lapce-app/src/{lib,config,window_tab,command,doc}.rs`, `lapce-app/src/panel/{kind,mod,data,view}.rs`, `lapce-app/src/config/icon.rs`, `defaults/{settings,icon-theme}.toml`, `icons/codicons/agent.svg` (new), `CHANGES.md`, and the spec.

---

### Task 1: Shared RPC types

**Files:**
- Create: `lapce-rpc/src/agent.rs`
- Modify: `lapce-rpc/src/lib.rs`, `docs/superpowers/specs/2026-09-24-agent-panel-design.md`, `CHANGES.md`

**Interfaces:**
- Produces (all in `lapce_rpc::agent`, all `Debug + Clone + Serialize + Deserialize`): `AgentRequestId = u64`, `AgentServerConfig { command: String, args: Vec<String>, env: HashMap<String, String> }`, `AgentContext { path: PathBuf, selection: Option<String> }`, `AgentPermissionKind { AllowOnce, AllowAlways, RejectOnce, RejectAlways }`, `AgentPermissionOption { id: String, name: String, kind: AgentPermissionKind }`, `AgentToolStatus { Pending, InProgress, Completed, Failed }`, `AgentStatus { Starting, Ready, Disconnected { reason: String } }`, `AgentEvent` (variants below).

- [ ] **Step 1: Update the spec with the deltas**

Append this section to the end of `docs/superpowers/specs/2026-09-24-agent-panel-design.md` (replace nothing else):

```markdown
## Changes found during planning

- Terminal requests are out of v1: `terminal` capability is not advertised. The agent uses its own shell tool and asks permission through `session/request_permission`.
- ACP has no client-side gate on `fs/write_text_file`. Permission prompts come from the agent. Lapce enforces a workspace path guard on every read and write.
- `CoreRequest` is empty and the UI has no reply path, so file writes to open buffers use a `CoreNotification::AgentApplyEdit`, reads run in the dispatcher (`ProxyRequest::AgentReadFile`), writes to closed files go to disk in the proxy (`ProxyRequest::AgentWriteFile`).
- Resolved: the ACP crate is runtime agnostic (no tokio), so it runs on a dedicated proxy thread with `futures::executor::block_on`. The proxy holds synced buffers, but inside the single-threaded dispatcher.
- Default panel position: right side.
- Known limitation: right after an agent write to an open buffer, the proxy copy updates when the UI's `Update` arrives, so an immediate re-read can briefly see old text.
- Context in v1 is the active file, or its selection when there is one. `@file` mentions and context chips are deferred. Enter sends the prompt; multi-line input is not supported yet.
```

- [ ] **Step 2: Write the failing test**

Create `lapce-rpc/src/agent.rs` containing only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes then deserializes an event and checks the JSON is stable.
    fn roundtrip(event: AgentEvent) {
        let json = serde_json::to_string(&event).unwrap();
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn permission_request_roundtrips() {
        roundtrip(AgentEvent::PermissionRequest {
            request_id: 7,
            title: "Edit main.rs".to_string(),
            options: vec![AgentPermissionOption {
                id: "allow".to_string(),
                name: "Allow".to_string(),
                kind: AgentPermissionKind::AllowOnce,
            }],
        });
    }

    #[test]
    fn status_and_tool_events_roundtrip() {
        roundtrip(AgentEvent::Status {
            status: AgentStatus::Disconnected {
                reason: "exit 1".to_string(),
            },
        });
        roundtrip(AgentEvent::ToolCallUpdate {
            id: "t1".to_string(),
            title: None,
            status: Some(AgentToolStatus::Completed),
        });
    }
}
```

Add `pub mod agent;` (alphabetical, before `pub mod buffer;`) to `lapce-rpc/src/lib.rs`.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p lapce-rpc agent::`
Expected: FAIL to compile, "cannot find type `AgentEvent`".

- [ ] **Step 4: Write the implementation**

Put this above the test module in `lapce-rpc/src/agent.rs`:

```rust
//! Types shared between the UI and the proxy for the AI agent (ACP) panel.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Identifier of a permission request raised by the agent.
pub type AgentRequestId = u64;

/// How to launch an ACP agent subprocess.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// A piece of editor context attached to a prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentContext {
    pub path: PathBuf,
    /// Selected text. When `None` the whole file is attached.
    pub selection: Option<String>,
}

/// The kind of a permission option offered by the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

/// One choice the user can pick when the agent asks for permission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentPermissionOption {
    pub id: String,
    pub name: String,
    pub kind: AgentPermissionKind,
}

/// Execution status of a tool call shown in the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// Connection status of the agent process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentStatus {
    Starting,
    Ready,
    Disconnected { reason: String },
}

/// Events streamed from the proxy to the UI while an agent session runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    MessageChunk {
        text: String,
    },
    ThoughtChunk {
        text: String,
    },
    ToolCall {
        id: String,
        title: String,
        status: AgentToolStatus,
    },
    ToolCallUpdate {
        id: String,
        title: Option<String>,
        status: Option<AgentToolStatus>,
    },
    PermissionRequest {
        request_id: AgentRequestId,
        title: String,
        options: Vec<AgentPermissionOption>,
    },
    Status {
        status: AgentStatus,
    },
    TurnEnded {
        stop_reason: String,
    },
    Error {
        message: String,
    },
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p lapce-rpc agent::`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

Append to `CHANGES.md`:

```markdown
## AI agent panel

- Added `lapce_rpc::agent` with the shared types used by the agent panel (ACP client). Spec deltas from planning are recorded in the design spec.
```

```bash
git add lapce-rpc/src/agent.rs lapce-rpc/src/lib.rs docs/superpowers/specs/2026-09-24-agent-panel-design.md CHANGES.md
git commit -m "feat(agent): add shared RPC types for the agent panel"
```

---

### Task 2: Proxy dependencies and the permission broker

**Files:**
- Create: `lapce-proxy/src/agent/mod.rs` (only `pub mod permission;` for now), `lapce-proxy/src/agent/permission.rs`
- Modify: `lapce-proxy/Cargo.toml`, `lapce-proxy/src/lib.rs`, `CHANGES.md`

**Interfaces:**
- Produces: `PermissionBroker::new() -> Self`; `register(&self) -> (AgentRequestId, futures::channel::oneshot::Receiver<Option<String>>)`; `reply(&self, id: AgentRequestId, option_id: Option<String>) -> bool`; `cancel_all(&self)`. A dropped sender (cancel) makes the receiver return `Err(Canceled)`, which the session treats as "cancelled".

- [ ] **Step 1: Add the dependencies**

In `lapce-proxy/Cargo.toml`, under `[dependencies]` (keep the alignment style, alphabetical among the plain crates):

```toml
agent-client-protocol = "=2.2.0"
futures               = "0.3"
```

Add `pub mod agent;` to `lapce-proxy/src/lib.rs` next to the other `pub mod` lines, and create `lapce-proxy/src/agent/mod.rs`:

```rust
//! AI agent (ACP client) support.

pub mod permission;
```

Run: `cargo check -p lapce-proxy`
Expected: compiles (downloads the crate; if `cargo deny` is part of your flow, the crate is Apache-2.0).

- [ ] **Step 2: Write the failing test**

Create `lapce-proxy/src/agent/permission.rs` with the tests only:

```rust
#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;

    #[test]
    fn reply_resolves_the_matching_request() {
        let broker = PermissionBroker::new();
        let (id, rx) = broker.register();
        assert!(broker.reply(id, Some("allow".to_string())));
        assert_eq!(block_on(rx), Ok(Some("allow".to_string())));
    }

    #[test]
    fn reply_to_unknown_id_returns_false() {
        let broker = PermissionBroker::new();
        assert!(!broker.reply(99, None));
    }

    #[test]
    fn ids_are_unique() {
        let broker = PermissionBroker::new();
        let (a, _rx_a) = broker.register();
        let (b, _rx_b) = broker.register();
        assert_ne!(a, b);
    }

    #[test]
    fn cancel_all_wakes_every_waiter_with_cancelled() {
        let broker = PermissionBroker::new();
        let (_, rx1) = broker.register();
        let (_, rx2) = broker.register();
        broker.cancel_all();
        assert!(block_on(rx1).is_err());
        assert!(block_on(rx2).is_err());
    }

    #[test]
    fn a_reply_is_delivered_once() {
        let broker = PermissionBroker::new();
        let (id, _rx) = broker.register();
        assert!(broker.reply(id, None));
        assert!(!broker.reply(id, None));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p lapce-proxy agent::permission`
Expected: FAIL to compile, "cannot find type `PermissionBroker`".

- [ ] **Step 4: Write the implementation**

Put above the test module:

```rust
//! Tracks permission requests that are waiting for a user decision.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use futures::channel::oneshot;
use lapce_rpc::agent::AgentRequestId;
use parking_lot::Mutex;

/// The user's decision: `Some(option_id)` picks an option, `None` rejects.
type Decision = Option<String>;

/// Hands out request ids and delivers the user's decision to the waiting session.
#[derive(Default)]
pub struct PermissionBroker {
    next_id: AtomicU64,
    pending: Mutex<HashMap<AgentRequestId, oneshot::Sender<Decision>>>,
}

impl PermissionBroker {
    /// Creates an empty broker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new pending request and returns its id and the receiver to await.
    pub fn register(&self) -> (AgentRequestId, oneshot::Receiver<Decision>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id, tx);
        (id, rx)
    }

    /// Delivers a decision. Returns `false` if the id is unknown or already answered.
    pub fn reply(&self, id: AgentRequestId, option_id: Decision) -> bool {
        let Some(tx) = self.pending.lock().remove(&id) else {
            return false;
        };
        tx.send(option_id).is_ok()
    }

    /// Drops every pending request so all waiters see a cancellation.
    pub fn cancel_all(&self) {
        self.pending.lock().clear();
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p lapce-proxy agent::permission`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

Append to `CHANGES.md` under `## AI agent panel`: `- Added the ACP crate to lapce-proxy and a PermissionBroker for pending agent permission requests.`

```bash
git add lapce-proxy/Cargo.toml Cargo.lock lapce-proxy/src/lib.rs lapce-proxy/src/agent CHANGES.md
git commit -m "feat(agent): add ACP dependency and permission broker to the proxy"
```

---

### Task 3: Workspace path guard and line slicing

**Files:**
- Create: `lapce-proxy/src/agent/paths.rs`
- Modify: `lapce-proxy/src/agent/mod.rs` (add `pub mod paths;`), `CHANGES.md`

**Interfaces:**
- Produces: `resolve_workspace_path(workspace: &Path, path: &Path) -> anyhow::Result<PathBuf>` (returns the normalized absolute path or an error); `slice_lines(content: &str, line: Option<u32>, limit: Option<u32>) -> String` (`line` is 1-based).

Security notes for this task (recorded design answers): the agent is untrusted input; every path it sends is normalized lexically, must be absolute, must stay under the workspace root, and the real location of the deepest existing ancestor (symlinks resolved) must stay under the real workspace root.

- [ ] **Step 1: Write the failing tests**

Create `lapce-proxy/src/agent/paths.rs` with tests only:

```rust
#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    /// Creates a fresh temp directory unique to this test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("lapce-agent-paths-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accepts_a_file_inside_the_workspace() {
        let ws = temp_dir("inside");
        let ok = resolve_workspace_path(&ws, &ws.join("src/main.rs")).unwrap();
        assert_eq!(ok, ws.join("src/main.rs"));
    }

    #[test]
    fn rejects_relative_paths() {
        let ws = temp_dir("relative");
        assert!(resolve_workspace_path(&ws, Path::new("src/main.rs")).is_err());
    }

    #[test]
    fn rejects_parent_dir_escape() {
        let ws = temp_dir("dotdot");
        let escape = ws.join("../outside.txt");
        assert!(resolve_workspace_path(&ws, &escape).is_err());
    }

    #[test]
    fn rejects_sibling_directory_sharing_the_prefix() {
        let base = temp_dir("sibling");
        let ws = base.join("ws");
        let evil = base.join("ws-evil");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&evil).unwrap();
        assert!(resolve_workspace_path(&ws, &evil.join("a.txt")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let base = temp_dir("symlink");
        let ws = base.join("ws");
        let outside = base.join("outside");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("link")).unwrap();
        assert!(resolve_workspace_path(&ws, &ws.join("link/secret.txt")).is_err());
    }

    #[test]
    fn slice_lines_returns_everything_without_bounds() {
        assert_eq!(slice_lines("a\nb\nc\n", None, None), "a\nb\nc\n");
    }

    #[test]
    fn slice_lines_applies_line_and_limit() {
        assert_eq!(slice_lines("a\nb\nc\nd\n", Some(2), Some(2)), "b\nc\n");
        assert_eq!(slice_lines("a\nb\nc", Some(3), None), "c");
    }

    #[test]
    fn slice_lines_treats_zero_as_first_line_and_past_end_as_empty() {
        assert_eq!(slice_lines("a\nb\n", Some(0), Some(1)), "a\n");
        assert_eq!(slice_lines("a\nb\n", Some(9), None), "");
    }
}
```

Add `pub mod paths;` to `lapce-proxy/src/agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lapce-proxy agent::paths`
Expected: FAIL to compile, "cannot find function `resolve_workspace_path`".

- [ ] **Step 3: Write the implementation**

Put above the test module:

```rust
//! Path safety for file requests coming from the agent, and line slicing.

use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};

/// Normalizes `path` lexically (no filesystem access): drops `.` and resolves `..`.
/// Fails if a `..` would climb above the root.
fn normalize(path: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    bail!("path climbs above the root: {}", path.display());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// Returns the real location of `path`: the deepest existing ancestor is
/// canonicalized (symlinks resolved) and the missing tail is appended as is.
fn canonical_prefix(path: &Path) -> PathBuf {
    let mut tail = Vec::new();
    let mut current = path;
    loop {
        if let Ok(real) = current.canonicalize() {
            return tail.iter().rev().fold(real, |acc, name| acc.join(name));
        }
        let (Some(parent), Some(name)) = (current.parent(), current.file_name())
        else {
            return path.to_path_buf();
        };
        tail.push(name.to_owned());
        current = parent;
    }
}

/// Validates a path sent by the agent and returns its normalized absolute form.
///
/// The path must be absolute, must stay inside `workspace` after normalization,
/// and its real location (symlinks resolved) must also stay inside the real
/// workspace directory.
pub fn resolve_workspace_path(workspace: &Path, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("path must be absolute: {}", path.display());
    }
    let target = normalize(path)?;
    let root = normalize(workspace)?;
    if !target.starts_with(&root) {
        bail!("path is outside the workspace: {}", path.display());
    }
    if !canonical_prefix(&target).starts_with(canonical_prefix(&root)) {
        bail!("path escapes the workspace through a link: {}", path.display());
    }
    Ok(target)
}

/// Returns `limit` lines of `content` starting at 1-based `line`.
/// `line == 0` counts as the first line; missing bounds mean "no bound".
pub fn slice_lines(content: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return content.to_string();
    }
    let skip = line.unwrap_or(1).saturating_sub(1) as usize;
    let take = limit.map_or(usize::MAX, |l| l as usize);
    content.split_inclusive('\n').skip(skip).take(take).collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lapce-proxy agent::paths`
Expected: PASS (8 tests on Unix, 7 elsewhere).

- [ ] **Step 5: Commit**

Append to `CHANGES.md`: `- Added the workspace path guard and line slicing used to serve agent file requests.`

```bash
git add lapce-proxy/src/agent CHANGES.md
git commit -m "feat(agent): add workspace path guard for agent file requests"
```

---

### Task 4: Prompt builder

**Files:**
- Create: `lapce-proxy/src/agent/prompt.rs`
- Modify: `lapce-proxy/src/agent/mod.rs` (add `pub mod prompt;`), `CHANGES.md`

**Interfaces:**
- Produces: `MAX_CONTEXT_BYTES: usize = 64_000`; `ResolvedContext { path: PathBuf, selection: Option<String>, content: Option<String> }`; `build_prompt_text(user_text: &str, contexts: &[ResolvedContext]) -> String`.

- [ ] **Step 1: Write the failing tests**

Create `lapce-proxy/src/agent/prompt.rs` with tests only:

```rust
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn ctx(selection: Option<&str>, content: Option<&str>) -> ResolvedContext {
        ResolvedContext {
            path: PathBuf::from("/ws/a.rs"),
            selection: selection.map(str::to_string),
            content: content.map(str::to_string),
        }
    }

    #[test]
    fn no_context_returns_the_user_text_unchanged() {
        assert_eq!(build_prompt_text("fix it", &[]), "fix it");
    }

    #[test]
    fn selection_wins_over_file_content() {
        let text = build_prompt_text("why?", &[ctx(Some("let x = 1;"), Some("whole file"))]);
        assert!(text.contains("/ws/a.rs (selection)"));
        assert!(text.contains("let x = 1;"));
        assert!(!text.contains("whole file"));
        assert!(text.ends_with("why?"));
    }

    #[test]
    fn file_content_is_used_when_there_is_no_selection() {
        let text = build_prompt_text("why?", &[ctx(None, Some("whole file"))]);
        assert!(text.contains("/ws/a.rs (file)"));
        assert!(text.contains("whole file"));
    }

    #[test]
    fn context_without_any_text_is_skipped() {
        assert_eq!(build_prompt_text("hi", &[ctx(None, None)]), "hi");
    }

    #[test]
    fn huge_content_is_truncated_on_a_char_boundary() {
        // Each 'e' with an accent is 2 bytes, so the limit falls mid-character.
        let content = "\u{e9}".repeat(MAX_CONTEXT_BYTES);
        let text = build_prompt_text("q", &[ctx(None, Some(&content))]);
        assert!(text.contains("[truncated]"));
        assert!(text.len() < MAX_CONTEXT_BYTES + 200);
    }
}
```

Add `pub mod prompt;` to `lapce-proxy/src/agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lapce-proxy agent::prompt`
Expected: FAIL to compile, "cannot find type `ResolvedContext`".

- [ ] **Step 3: Write the implementation**

```rust
//! Builds the text sent to the agent from the user's message and editor context.

use std::path::PathBuf;

/// Maximum number of context bytes attached per file or selection.
pub const MAX_CONTEXT_BYTES: usize = 64_000;

/// Marker appended when context was cut to `MAX_CONTEXT_BYTES`.
const TRUNCATED_MARKER: &str = "\n[truncated]";

/// A context item whose text has already been read.
pub struct ResolvedContext {
    pub path: PathBuf,
    pub selection: Option<String>,
    pub content: Option<String>,
}

/// Cuts `text` to at most `max` bytes without splitting a UTF-8 character.
fn truncate_utf8(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Builds the prompt: one fenced block per context item, then the user text.
pub fn build_prompt_text(user_text: &str, contexts: &[ResolvedContext]) -> String {
    let mut out = String::new();
    for ctx in contexts {
        let (kind, body) = match (&ctx.selection, &ctx.content) {
            (Some(selection), _) => ("selection", selection),
            (None, Some(content)) => ("file", content),
            (None, None) => continue,
        };
        let shown = truncate_utf8(body, MAX_CONTEXT_BYTES);
        let marker = if shown.len() < body.len() {
            TRUNCATED_MARKER
        } else {
            ""
        };
        out.push_str(&format!(
            "Context from {} ({kind}):\n```\n{shown}{marker}\n```\n\n",
            ctx.path.display()
        ));
    }
    out.push_str(user_text);
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lapce-proxy agent::prompt`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

Append to `CHANGES.md`: `- Added the prompt builder that attaches selection or file context to the user message.`

```bash
git add lapce-proxy/src/agent CHANGES.md
git commit -m "feat(agent): add prompt builder with context truncation"
```

---

### Task 5: ACP update mapping

**Files:**
- Create: `lapce-proxy/src/agent/mapping.rs`
- Modify: `lapce-proxy/src/agent/mod.rs` (add `pub mod mapping;`), `CHANGES.md`

**Interfaces:**
- Consumes: `lapce_rpc::agent::{AgentEvent, AgentToolStatus, AgentPermissionOption, AgentPermissionKind}`.
- Produces: `map_update(update: SessionUpdate) -> Option<AgentEvent>`; `map_permission_options(req: &RequestPermissionRequest) -> Vec<AgentPermissionOption>`; `permission_title(req: &RequestPermissionRequest) -> String`.

This code was compile-checked against `agent-client-protocol` 2.2.0 while writing the plan.

- [ ] **Step 1: Write the failing tests**

Create `lapce-proxy/src/agent/mapping.rs` with tests only:

```rust
#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::v1::{
        ContentChunk, PermissionOption, TextContent, ToolCallUpdateFields,
    };

    use super::*;

    #[test]
    fn maps_message_chunk() {
        let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("hi")),
        ));
        assert_eq!(
            map_update(update),
            Some(AgentEvent::MessageChunk {
                text: "hi".to_string()
            })
        );
    }

    #[test]
    fn maps_tool_call_and_update() {
        let call = SessionUpdate::ToolCall(ToolCall::new("t1", "Edit foo.rs"));
        assert_eq!(
            map_update(call),
            Some(AgentEvent::ToolCall {
                id: "t1".to_string(),
                title: "Edit foo.rs".to_string(),
                status: AgentToolStatus::Pending,
            })
        );
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t1",
            ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
        ));
        assert_eq!(
            map_update(update),
            Some(AgentEvent::ToolCallUpdate {
                id: "t1".to_string(),
                title: None,
                status: Some(AgentToolStatus::Completed),
            })
        );
    }

    #[test]
    fn updates_the_panel_does_not_show_are_ignored() {
        let update = SessionUpdate::UserMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("echo of the prompt")),
        ));
        assert_eq!(map_update(update), None);
    }

    #[test]
    fn maps_permission_options_and_title() {
        let req = RequestPermissionRequest::new(
            "s1",
            ToolCallUpdate::new("t1", ToolCallUpdateFields::new().title("Run tests")),
            vec![
                PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
                PermissionOption::new("no", "Reject", PermissionOptionKind::RejectOnce),
            ],
        );
        assert_eq!(permission_title(&req), "Run tests");
        let options = map_permission_options(&req);
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].id, "allow");
        assert_eq!(options[1].kind, AgentPermissionKind::RejectOnce);
    }

    #[test]
    fn missing_title_gets_a_default() {
        let req = RequestPermissionRequest::new(
            "s1",
            ToolCallUpdate::new("t1", ToolCallUpdateFields::new()),
            vec![],
        );
        assert_eq!(permission_title(&req), "Agent action");
    }
}
```

Add `pub mod mapping;` to `lapce-proxy/src/agent/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lapce-proxy agent::mapping`
Expected: FAIL to compile, "cannot find function `map_update`".

- [ ] **Step 3: Write the implementation**

```rust
//! Converts ACP session data into the events the UI understands.

use agent_client_protocol::schema::v1::{
    ContentBlock, PermissionOptionKind, RequestPermissionRequest, SessionUpdate,
    ToolCall, ToolCallStatus, ToolCallUpdate,
};
use lapce_rpc::agent::{
    AgentEvent, AgentPermissionKind, AgentPermissionOption, AgentToolStatus,
};

/// Title shown when the agent does not name the action it asks permission for.
const DEFAULT_PERMISSION_TITLE: &str = "Agent action";

/// Maps an ACP tool status to the UI status. Unknown states count as failed.
fn map_status(status: ToolCallStatus) -> AgentToolStatus {
    match status {
        ToolCallStatus::Pending => AgentToolStatus::Pending,
        ToolCallStatus::InProgress => AgentToolStatus::InProgress,
        ToolCallStatus::Completed => AgentToolStatus::Completed,
        _ => AgentToolStatus::Failed,
    }
}

/// Returns the text of a content block, or `None` for non-text content.
fn block_text(block: ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(text) => Some(text.text),
        _ => None,
    }
}

/// Maps a session update to a UI event. Updates the panel does not show return `None`.
pub fn map_update(update: SessionUpdate) -> Option<AgentEvent> {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            block_text(chunk.content).map(|text| AgentEvent::MessageChunk { text })
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            block_text(chunk.content).map(|text| AgentEvent::ThoughtChunk { text })
        }
        SessionUpdate::ToolCall(ToolCall {
            tool_call_id,
            title,
            status,
            ..
        }) => Some(AgentEvent::ToolCall {
            id: tool_call_id.to_string(),
            title,
            status: map_status(status),
        }),
        SessionUpdate::ToolCallUpdate(ToolCallUpdate {
            tool_call_id,
            fields,
            ..
        }) => Some(AgentEvent::ToolCallUpdate {
            id: tool_call_id.to_string(),
            title: fields.title,
            status: fields.status.map(map_status),
        }),
        _ => None,
    }
}

/// Maps an ACP permission option kind. Unknown kinds count as reject-always (the safe side).
fn map_kind(kind: PermissionOptionKind) -> AgentPermissionKind {
    match kind {
        PermissionOptionKind::AllowOnce => AgentPermissionKind::AllowOnce,
        PermissionOptionKind::AllowAlways => AgentPermissionKind::AllowAlways,
        PermissionOptionKind::RejectOnce => AgentPermissionKind::RejectOnce,
        _ => AgentPermissionKind::RejectAlways,
    }
}

/// Converts the options of a permission request for display.
pub fn map_permission_options(
    req: &RequestPermissionRequest,
) -> Vec<AgentPermissionOption> {
    req.options
        .iter()
        .map(|option| AgentPermissionOption {
            id: option.option_id.to_string(),
            name: option.name.clone(),
            kind: map_kind(option.kind),
        })
        .collect()
}

/// Returns the title of the action the agent asks permission for.
pub fn permission_title(req: &RequestPermissionRequest) -> String {
    req.tool_call
        .fields
        .title
        .clone()
        .unwrap_or_else(|| DEFAULT_PERMISSION_TITLE.to_string())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lapce-proxy agent::mapping`
Expected: PASS (5 tests). If a field access such as `option.kind` is not `Copy`, add `.clone()`; the type shapes were verified when this plan was written.

- [ ] **Step 5: Commit**

Append to `CHANGES.md`: `- Added mapping from ACP session updates and permission requests to UI events.`

```bash
git add lapce-proxy/src/agent CHANGES.md
git commit -m "feat(agent): map ACP updates to UI events"
```

---

### Task 6: File read and write plumbing through the dispatcher

**Files:**
- Create: `lapce-proxy/src/agent/fs.rs`
- Modify: `lapce-proxy/src/agent/mod.rs` (add `pub mod fs;`), `lapce-rpc/src/proxy.rs`, `lapce-rpc/src/core.rs`, `lapce-proxy/src/dispatch.rs`, `CHANGES.md`

**Interfaces:**
- Consumes: `lapce_proxy::buffer::Buffer` (field `rope: Rope`), `CoreRpcHandler::notification`.
- Produces:
  - `agent::fs::read_text(buffers: &HashMap<PathBuf, Buffer>, path: &Path) -> anyhow::Result<String>`
  - `agent::fs::write_text(buffers: &HashMap<PathBuf, Buffer>, core_rpc: &CoreRpcHandler, path: &Path, content: &str) -> anyhow::Result<()>`
  - `ProxyRequest::AgentReadFile { path: PathBuf }`, `ProxyRequest::AgentWriteFile { path: PathBuf, content: String }`, `ProxyResponse::AgentReadFileResponse { content: String }` (write answers `ProxyResponse::Success {}`)
  - `ProxyRpcHandler::agent_read_file(&self, path: PathBuf) -> Result<String, RpcError>` and `agent_write_file(&self, path: PathBuf, content: String) -> Result<(), RpcError>` (both blocking; never call them from the dispatcher thread)
  - `CoreNotification::AgentApplyEdit { path: PathBuf, content: String }`

- [ ] **Step 1: Write the failing tests**

Create `lapce-proxy/src/agent/fs.rs` with tests only:

```rust
#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::PathBuf};

    use lapce_rpc::{
        buffer::BufferId,
        core::{CoreNotification, CoreRpc, CoreRpcHandler},
    };
    use lapce_xi_rope::rope::Rope;

    use super::*;
    use crate::buffer::Buffer;

    /// Creates a fresh temp directory unique to this test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("lapce-agent-fs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_prefers_the_unsaved_buffer_over_disk() {
        let dir = temp_dir("read-buffer");
        let path = dir.join("a.txt");
        fs::write(&path, "on disk").unwrap();
        let mut buffer = Buffer::new(BufferId::next(), path.clone());
        buffer.rope = Rope::from("unsaved edit");
        let buffers = HashMap::from([(path.clone(), buffer)]);
        assert_eq!(read_text(&buffers, &path).unwrap(), "unsaved edit");
    }

    #[test]
    fn read_falls_back_to_disk() {
        let dir = temp_dir("read-disk");
        let path = dir.join("a.txt");
        fs::write(&path, "on disk").unwrap();
        assert_eq!(read_text(&HashMap::new(), &path).unwrap(), "on disk");
    }

    #[test]
    fn read_of_non_utf8_file_is_an_error_not_a_panic() {
        let dir = temp_dir("non-utf8");
        let path = dir.join("bin.dat");
        fs::write(&path, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert!(read_text(&HashMap::new(), &path).is_err());
    }

    #[test]
    fn write_to_closed_file_creates_parents_and_writes_disk() {
        let dir = temp_dir("write-disk");
        let path = dir.join("new/dir/a.txt");
        let core_rpc = CoreRpcHandler::new();
        write_text(&HashMap::new(), &core_rpc, &path, "hello").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        assert!(core_rpc.rx().try_recv().is_err());
    }

    #[test]
    fn write_to_open_file_goes_to_the_ui_and_leaves_disk_alone() {
        let dir = temp_dir("write-open");
        let path = dir.join("a.txt");
        fs::write(&path, "on disk").unwrap();
        let buffers = HashMap::from([(
            path.clone(),
            Buffer::new(BufferId::next(), path.clone()),
        )]);
        let core_rpc = CoreRpcHandler::new();
        write_text(&buffers, &core_rpc, &path, "from agent").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "on disk");
        match core_rpc.rx().try_recv().unwrap() {
            CoreRpc::Notification(n) => match *n {
                CoreNotification::AgentApplyEdit {
                    path: edited,
                    content,
                } => {
                    assert_eq!(edited, path);
                    assert_eq!(content, "from agent");
                }
                other => panic!("unexpected notification: {other:?}"),
            },
            _ => panic!("expected a notification"),
        }
    }
}
```

Add `pub mod fs;` to `lapce-proxy/src/agent/mod.rs`.

- [ ] **Step 2: Add the protocol variants**

In `lapce-rpc/src/core.rs`, inside `enum CoreNotification`, add:

```rust
    /// The agent wrote a file that is open in the editor: apply the new content
    /// to the buffer as a single undoable edit.
    AgentApplyEdit {
        path: PathBuf,
        content: String,
    },
```

In `lapce-rpc/src/proxy.rs`:

- inside `enum ProxyRequest` add:

```rust
    /// Read a text file for the agent (unsaved buffer content wins over disk).
    AgentReadFile {
        path: PathBuf,
    },
    /// Write a text file for the agent (open files are edited through the UI).
    AgentWriteFile {
        path: PathBuf,
        content: String,
    },
```

- inside `enum ProxyResponse` add:

```rust
    AgentReadFileResponse {
        content: String,
    },
```

- inside `impl ProxyRpcHandler` add (next to the other request helpers):

```rust
    /// Blocking read of a text file for the agent. Must not be called from the
    /// dispatcher thread, which is the one that answers it.
    pub fn agent_read_file(&self, path: PathBuf) -> Result<String, RpcError> {
        match self.request(ProxyRequest::AgentReadFile { path })? {
            ProxyResponse::AgentReadFileResponse { content } => Ok(content),
            _ => Err(RpcError {
                code: 0,
                message: "unexpected response to AgentReadFile".to_string(),
            }),
        }
    }

    /// Blocking write of a text file for the agent. Must not be called from the
    /// dispatcher thread, which is the one that answers it.
    pub fn agent_write_file(
        &self,
        path: PathBuf,
        content: String,
    ) -> Result<(), RpcError> {
        match self.request(ProxyRequest::AgentWriteFile { path, content })? {
            ProxyResponse::Success {} => Ok(()),
            _ => Err(RpcError {
                code: 0,
                message: "unexpected response to AgentWriteFile".to_string(),
            }),
        }
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p lapce-proxy agent::fs`
Expected: FAIL to compile ("cannot find function `read_text`", and non-exhaustive match errors in `dispatch.rs`).

- [ ] **Step 4: Write the implementation**

Put above the test module in `lapce-proxy/src/agent/fs.rs`:

```rust
//! File access for the agent, backed by the dispatcher's synced buffers.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use lapce_rpc::core::{CoreNotification, CoreRpcHandler};

use crate::buffer::Buffer;

/// Reads a text file. If the file is open in the editor, the (possibly unsaved)
/// buffer content is returned; otherwise the file is read from disk.
/// Non UTF-8 files are an error.
pub fn read_text(buffers: &HashMap<PathBuf, Buffer>, path: &Path) -> Result<String> {
    if let Some(buffer) = buffers.get(path) {
        return Ok(buffer.rope.to_string());
    }
    fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))
}

/// Writes a text file. If the file is open in the editor, the UI is asked to
/// apply the content as an undoable buffer edit and the disk is left alone.
/// Otherwise parent directories are created and the file is written to disk.
pub fn write_text(
    buffers: &HashMap<PathBuf, Buffer>,
    core_rpc: &CoreRpcHandler,
    path: &Path,
    content: &str,
) -> Result<()> {
    if buffers.contains_key(path) {
        core_rpc.notification(CoreNotification::AgentApplyEdit {
            path: path.to_path_buf(),
            content: content.to_string(),
        });
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(path, content)
        .with_context(|| format!("cannot write {}", path.display()))
}
```

In `lapce-proxy/src/dispatch.rs`, inside `handle_request`'s `match rpc` (next to `NewBuffer`), add:

```rust
            AgentReadFile { path } => {
                let result = crate::agent::fs::read_text(&self.buffers, &path)
                    .map(|content| ProxyResponse::AgentReadFileResponse { content })
                    .map_err(|err| RpcError {
                        code: 0,
                        message: format!("{err:#}"),
                    });
                self.respond_rpc(id, result);
            }
            AgentWriteFile { path, content } => {
                let result = crate::agent::fs::write_text(
                    &self.buffers,
                    &self.core_rpc,
                    &path,
                    &content,
                )
                .map(|()| ProxyResponse::Success {})
                .map_err(|err| RpcError {
                    code: 0,
                    message: format!("{err:#}"),
                });
                self.respond_rpc(id, result);
            }
```

If `RpcError` is not already imported in `dispatch.rs`, add it to the existing `lapce_rpc::{...}` import.

- [ ] **Step 5: Run tests and build to verify they pass**

Run: `cargo test -p lapce-proxy agent::fs && cargo check -p lapce-proxy -p lapce-rpc`
Expected: PASS (5 tests), then a clean check. `lapce-app` will not compile yet if its `handle_core_notification` match has no wildcard: run `cargo check -p lapce-app` and, if it complains about `AgentApplyEdit`, add the arm `CoreNotification::AgentApplyEdit { .. } => {}` with the comment `// Wired to the agent state in Task 8.` (this arm is replaced in Task 8).

- [ ] **Step 6: Commit**

Append to `CHANGES.md`: `- Added ProxyRequest::AgentReadFile/AgentWriteFile and CoreNotification::AgentApplyEdit so the agent reads unsaved buffers and edits open files through the UI.`

```bash
git add lapce-proxy/src lapce-rpc/src lapce-app/src/window_tab.rs CHANGES.md
git commit -m "feat(agent): route agent file access through the dispatcher"
```

---

### Task 7: UI configuration and agent state

**Files:**
- Create: `lapce-app/src/config/agent.rs`, `lapce-app/src/agent.rs`
- Modify: `lapce-app/src/config.rs`, `lapce-app/src/lib.rs`, `defaults/settings.toml`, `CHANGES.md`

**Interfaces:**
- Consumes: `lapce_rpc::agent::*` from Task 1.
- Produces:
  - `AgentConfig { default_server: String, servers: HashMap<String, AgentServerSetting> }` with `resolve(&self) -> Result<lapce_rpc::agent::AgentServerConfig, String>` and field `LapceConfig::agent`.
  - `AgentItem { User(String), Assistant(String), Thought(String), Tool { id, title, status }, Error(String) }` with `render_key(&self) -> u64`.
  - `PendingPermission { request_id: AgentRequestId, title: String, options: Vec<AgentPermissionOption> }`.
  - `AgentState { items: im::Vector<AgentItem>, status: AgentStatus, busy: bool, pending: Option<PendingPermission> }` with `push_user(&mut self, text: &str)`, `apply(&mut self, event: &AgentEvent)`, `take_pending(&mut self) -> Option<PendingPermission>`.

- [ ] **Step 1: Write the failing tests for the config**

Create `lapce-app/src/config/agent.rs` with tests only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn config(default: &str, command: &str) -> AgentConfig {
        AgentConfig {
            default_server: default.to_string(),
            servers: HashMap::from([(
                "claude-code".to_string(),
                AgentServerSetting {
                    command: command.to_string(),
                    arguments: vec!["-y".to_string()],
                    environment: HashMap::new(),
                },
            )]),
        }
    }

    #[test]
    fn resolves_the_default_server() {
        let resolved = config("claude-code", "npx").resolve().unwrap();
        assert_eq!(resolved.command, "npx");
        assert_eq!(resolved.args, vec!["-y".to_string()]);
    }

    #[test]
    fn unknown_default_server_lists_the_available_names() {
        let err = config("nope", "npx").resolve().unwrap_err();
        assert!(err.contains("nope"));
        assert!(err.contains("claude-code"));
    }

    #[test]
    fn empty_command_is_rejected() {
        assert!(config("claude-code", "  ").resolve().is_err());
    }
}
```

Add `pub mod agent;` to the `pub mod` list in `lapce-app/src/config.rs` (next to `pub mod terminal;`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lapce-app config::agent`
Expected: FAIL to compile, "cannot find type `AgentConfig`". (The first build of `lapce-app` is slow.)

- [ ] **Step 3: Write the config implementation**

Above the test module:

```rust
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use structdesc::FieldNames;

/// How to launch one ACP agent server.
#[derive(FieldNames, Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct AgentServerSetting {
    #[field_names(desc = "Command that starts the ACP agent")]
    pub command: String,
    #[serde(default)]
    #[field_names(desc = "Arguments passed to the command")]
    pub arguments: Vec<String>,
    #[serde(default)]
    #[field_names(desc = "Extra environment variables for the agent process")]
    pub environment: HashMap<String, String>,
}

/// Settings of the AI agent panel.
#[derive(FieldNames, Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct AgentConfig {
    #[field_names(desc = "Name of the agent server started from the agent panel")]
    pub default_server: String,
    #[field_names(skip)]
    pub servers: HashMap<String, AgentServerSetting>,
}

impl AgentConfig {
    /// Resolves the default server into the launch config sent to the proxy.
    /// Returns a user-facing message when the setting is unusable.
    pub fn resolve(&self) -> Result<lapce_rpc::agent::AgentServerConfig, String> {
        let Some(server) = self.servers.get(&self.default_server) else {
            let mut names: Vec<_> = self.servers.keys().cloned().collect();
            names.sort();
            return Err(format!(
                "agent server '{}' is not configured (available: {})",
                self.default_server,
                names.join(", ")
            ));
        };
        if server.command.trim().is_empty() {
            return Err(format!(
                "agent server '{}' has an empty command",
                self.default_server
            ));
        }
        Ok(lapce_rpc::agent::AgentServerConfig {
            command: server.command.clone(),
            args: server.arguments.clone(),
            env: server.environment.clone(),
        })
    }
}
```

In `lapce-app/src/config.rs`:
- add `agent::AgentConfig,` to the `use` list next to `terminal::TerminalConfig,`
- add the field `pub agent: AgentConfig,` to `LapceConfig` right after `pub terminal: TerminalConfig,`
- in the config update function, after `self.terminal.get_indexed_colors();` add `self.agent = new.agent;`

Append to `defaults/settings.toml` (verify the Gemini flag with `gemini --help` first: current Gemini CLI builds expose ACP mode as `--experimental-acp` or `--acp`; use whichever your installed version lists):

```toml
[agent]
default-server = "claude-code"

[agent.servers.claude-code]
command = "npx"
arguments = ["-y", "@agentclientprotocol/claude-agent-acp@latest"]

[agent.servers.gemini]
command = "gemini"
arguments = ["--experimental-acp"]
```

- [ ] **Step 4: Run the config tests**

Run: `cargo test -p lapce-app config::agent`
Expected: PASS (3 tests). Then `cargo test -p lapce-app config::` to confirm existing config tests still pass.

- [ ] **Step 5: Write the failing tests for the agent state**

Create `lapce-app/src/agent.rs` with tests only:

```rust
#[cfg(test)]
mod tests {
    use lapce_rpc::agent::{AgentStatus, AgentToolStatus};

    use super::*;

    #[test]
    fn message_chunks_extend_the_last_assistant_item() {
        let mut state = AgentState::default();
        state.apply(&AgentEvent::MessageChunk { text: "Hel".into() });
        state.apply(&AgentEvent::MessageChunk { text: "lo".into() });
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0], AgentItem::Assistant("Hello".to_string()));
    }

    #[test]
    fn a_tool_call_between_chunks_starts_a_new_assistant_item() {
        let mut state = AgentState::default();
        state.apply(&AgentEvent::MessageChunk { text: "a".into() });
        state.apply(&AgentEvent::ToolCall {
            id: "t1".into(),
            title: "Edit".into(),
            status: AgentToolStatus::Pending,
        });
        state.apply(&AgentEvent::MessageChunk { text: "b".into() });
        assert_eq!(state.items.len(), 3);
    }

    #[test]
    fn tool_update_changes_the_matching_tool() {
        let mut state = AgentState::default();
        state.apply(&AgentEvent::ToolCall {
            id: "t1".into(),
            title: "Edit".into(),
            status: AgentToolStatus::Pending,
        });
        state.apply(&AgentEvent::ToolCallUpdate {
            id: "t1".into(),
            title: None,
            status: Some(AgentToolStatus::Completed),
        });
        assert_eq!(
            state.items[0],
            AgentItem::Tool {
                id: "t1".to_string(),
                title: "Edit".to_string(),
                status: AgentToolStatus::Completed,
            }
        );
    }

    #[test]
    fn push_user_marks_the_state_busy_and_turn_end_clears_it() {
        let mut state = AgentState::default();
        state.push_user("hi");
        assert!(state.busy);
        state.apply(&AgentEvent::TurnEnded {
            stop_reason: "EndTurn".into(),
        });
        assert!(!state.busy);
    }

    #[test]
    fn disconnect_resets_busy_clears_permission_and_reports_the_reason() {
        let mut state = AgentState::default();
        state.push_user("hi");
        state.apply(&AgentEvent::PermissionRequest {
            request_id: 1,
            title: "Edit".into(),
            options: vec![],
        });
        assert!(state.pending.is_some());
        state.apply(&AgentEvent::Status {
            status: AgentStatus::Disconnected {
                reason: "exit 1".into(),
            },
        });
        assert!(!state.busy);
        assert!(state.pending.is_none());
        assert!(matches!(
            state.items.back(),
            Some(AgentItem::Error(message)) if message.contains("exit 1")
        ));
    }

    #[test]
    fn error_event_adds_an_error_item_and_stops_busy() {
        let mut state = AgentState::default();
        state.push_user("hi");
        state.apply(&AgentEvent::Error {
            message: "boom".into(),
        });
        assert!(!state.busy);
        assert_eq!(state.items.back(), Some(&AgentItem::Error("boom".to_string())));
    }

    #[test]
    fn take_pending_removes_the_request() {
        let mut state = AgentState::default();
        state.apply(&AgentEvent::PermissionRequest {
            request_id: 4,
            title: "Run".into(),
            options: vec![],
        });
        assert_eq!(state.take_pending().unwrap().request_id, 4);
        assert!(state.take_pending().is_none());
    }
}
```

Add `pub mod agent;` to `lapce-app/src/lib.rs` (alphabetical).

- [ ] **Step 6: Run tests to verify they fail**

Run: `cargo test -p lapce-app agent::tests`
Expected: FAIL to compile, "cannot find type `AgentState`".

- [ ] **Step 7: Write the state implementation**

Above the test module in `lapce-app/src/agent.rs`:

```rust
//! UI state of the AI agent panel.

use std::hash::{Hash, Hasher};

use lapce_rpc::agent::{
    AgentEvent, AgentPermissionOption, AgentRequestId, AgentStatus, AgentToolStatus,
};

/// One entry in the chat transcript.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentItem {
    User(String),
    Assistant(String),
    Thought(String),
    Tool {
        id: String,
        title: String,
        status: AgentToolStatus,
    },
    Error(String),
}

impl AgentItem {
    /// A key that changes whenever the visible content changes, so the list
    /// view re-renders items that grow while streaming.
    pub fn render_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        match self {
            AgentItem::User(text) => (0u8, text).hash(&mut hasher),
            AgentItem::Assistant(text) => (1u8, text).hash(&mut hasher),
            AgentItem::Thought(text) => (2u8, text).hash(&mut hasher),
            AgentItem::Tool { id, title, status } => {
                (3u8, id, title, *status as u8).hash(&mut hasher)
            }
            AgentItem::Error(text) => (4u8, text).hash(&mut hasher),
        }
        hasher.finish()
    }
}

/// A permission request waiting for the user's decision.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingPermission {
    pub request_id: AgentRequestId,
    pub title: String,
    pub options: Vec<AgentPermissionOption>,
}

/// Everything the panel displays. Plain data so it can be tested without a UI.
#[derive(Debug, Clone)]
pub struct AgentState {
    pub items: im::Vector<AgentItem>,
    pub status: AgentStatus,
    pub busy: bool,
    pub pending: Option<PendingPermission>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            items: im::Vector::new(),
            status: AgentStatus::Disconnected {
                reason: "not started".to_string(),
            },
            busy: false,
            pending: None,
        }
    }
}

impl AgentState {
    /// Records the user's message and marks the agent as working.
    pub fn push_user(&mut self, text: &str) {
        self.items.push_back(AgentItem::User(text.to_string()));
        self.busy = true;
    }

    /// Removes and returns the pending permission request, if any.
    pub fn take_pending(&mut self) -> Option<PendingPermission> {
        self.pending.take()
    }

    /// Appends streamed text to the last item when it has the same kind,
    /// otherwise starts a new item.
    fn append_text(&mut self, text: &str, thought: bool) {
        let merged = match self.items.back_mut() {
            Some(AgentItem::Assistant(existing)) if !thought => {
                existing.push_str(text);
                true
            }
            Some(AgentItem::Thought(existing)) if thought => {
                existing.push_str(text);
                true
            }
            _ => false,
        };
        if merged {
            return;
        }
        let item = if thought {
            AgentItem::Thought(text.to_string())
        } else {
            AgentItem::Assistant(text.to_string())
        };
        self.items.push_back(item);
    }

    /// Applies one event from the proxy to the state.
    pub fn apply(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::MessageChunk { text } => self.append_text(text, false),
            AgentEvent::ThoughtChunk { text } => self.append_text(text, true),
            AgentEvent::ToolCall { id, title, status } => {
                self.items.push_back(AgentItem::Tool {
                    id: id.clone(),
                    title: title.clone(),
                    status: *status,
                });
            }
            AgentEvent::ToolCallUpdate { id, title, status } => {
                for item in self.items.iter_mut() {
                    let AgentItem::Tool {
                        id: tool_id,
                        title: tool_title,
                        status: tool_status,
                    } = item
                    else {
                        continue;
                    };
                    if tool_id != id {
                        continue;
                    }
                    if let Some(title) = title {
                        *tool_title = title.clone();
                    }
                    if let Some(status) = status {
                        *tool_status = *status;
                    }
                }
            }
            AgentEvent::PermissionRequest {
                request_id,
                title,
                options,
            } => {
                self.pending = Some(PendingPermission {
                    request_id: *request_id,
                    title: title.clone(),
                    options: options.clone(),
                });
            }
            AgentEvent::Status { status } => {
                self.status = status.clone();
                if let AgentStatus::Disconnected { reason } = status {
                    self.busy = false;
                    self.pending = None;
                    self.items.push_back(AgentItem::Error(format!(
                        "Agent disconnected: {reason}"
                    )));
                }
            }
            AgentEvent::TurnEnded { .. } => {
                self.busy = false;
                self.pending = None;
            }
            AgentEvent::Error { message } => {
                self.busy = false;
                self.items.push_back(AgentItem::Error(message.clone()));
            }
        }
    }
}
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p lapce-app agent::tests`
Expected: PASS (7 tests). Note: `AgentToolStatus` must be `Copy` (it is, from Task 1) for `*status as u8`; if the cast is rejected, derive the key from `format!("{status:?}")` instead.

- [ ] **Step 9: Commit**

Append to `CHANGES.md`: `- Added [agent] settings (default-server, servers) and the AgentState transcript model for the panel.`

```bash
git add lapce-app/src/agent.rs lapce-app/src/lib.rs lapce-app/src/config.rs lapce-app/src/config/agent.rs defaults/settings.toml CHANGES.md
git commit -m "feat(agent): add agent settings and panel state model"
```

---

### Task 8: Session driver, manager and protocol wiring

**Files:**
- Create: `lapce-proxy/src/agent/session.rs`, `lapce-proxy/examples/mock_acp_agent.rs`, `lapce-proxy/tests/agent_session.rs`
- Modify: `lapce-proxy/src/agent/mod.rs`, `lapce-rpc/src/proxy.rs`, `lapce-rpc/src/core.rs`, `lapce-proxy/src/dispatch.rs`, `lapce-app/src/window_tab.rs`, `lapce-app/src/agent.rs` (add `AgentData`), `CHANGES.md`

**Interfaces:**
- Consumes: everything from Tasks 1-7.
- Produces:
  - `ProxyNotification::{AgentStart { config: AgentServerConfig }, AgentPrompt { text: String, contexts: Vec<AgentContext> }, AgentCancel {}, AgentPermissionReply { request_id: AgentRequestId, option_id: Option<String> }, AgentStop {}}`
  - `CoreNotification::AgentEvent { event: AgentEvent }`
  - `ProxyRpcHandler::{agent_start(config), agent_prompt(text, contexts), agent_cancel(), agent_permission_reply(request_id, option_id), agent_stop()}`
  - `AgentManager::new(core_rpc, proxy_rpc) -> Self`, `start(&mut self, config, workspace: Option<PathBuf>)`, `prompt(&self, text, contexts)`, `cancel(&self)`, `permission_reply(&self, request_id, option_id)`, `stop(&mut self)`
  - `AgentData { state: RwSignal<AgentState>, ... }` with `handle_event(&self, event: AgentEvent)` and `apply_edit(&self, path: &Path, content: &str)` (input handling and buttons are added in Task 9)

- [ ] **Step 1: Add the protocol variants and RPC helpers**

In `lapce-rpc/src/proxy.rs`, add to the imports `use crate::agent::{AgentContext, AgentRequestId, AgentServerConfig};` and inside `enum ProxyNotification`:

```rust
    AgentStart {
        config: AgentServerConfig,
    },
    AgentPrompt {
        text: String,
        contexts: Vec<AgentContext>,
    },
    AgentCancel {},
    AgentPermissionReply {
        request_id: AgentRequestId,
        option_id: Option<String>,
    },
    AgentStop {},
```

Inside `impl ProxyRpcHandler`:

```rust
    /// Starts (or restarts) the agent session.
    pub fn agent_start(&self, config: AgentServerConfig) {
        self.notification(ProxyNotification::AgentStart { config });
    }

    /// Sends a user prompt with editor context to the agent.
    pub fn agent_prompt(&self, text: String, contexts: Vec<AgentContext>) {
        self.notification(ProxyNotification::AgentPrompt { text, contexts });
    }

    /// Cancels the running agent turn.
    pub fn agent_cancel(&self) {
        self.notification(ProxyNotification::AgentCancel {});
    }

    /// Answers a pending permission request. `None` rejects it.
    pub fn agent_permission_reply(
        &self,
        request_id: AgentRequestId,
        option_id: Option<String>,
    ) {
        self.notification(ProxyNotification::AgentPermissionReply {
            request_id,
            option_id,
        });
    }

    /// Stops the agent session and its process.
    pub fn agent_stop(&self) {
        self.notification(ProxyNotification::AgentStop {});
    }
```

In `lapce-rpc/src/core.rs` add `agent::AgentEvent,` to the crate imports and inside `enum CoreNotification`:

```rust
    /// An event from the running agent session.
    AgentEvent {
        event: AgentEvent,
    },
```

- [ ] **Step 2: Write the mock agent**

Add to `lapce-proxy/Cargo.toml`:

```toml
[[example]]
name = "mock_acp_agent"
path = "examples/mock_acp_agent.rs"
```

Create `lapce-proxy/examples/mock_acp_agent.rs` (compile-checked against the crate):

```rust
//! Mock ACP agent used by the integration tests. On every prompt it streams a
//! message, asks for permission, writes `/mock/out.txt` if allowed, then ends the turn.

use agent_client_protocol::{
    Agent, Result, Stdio,
    schema::v1::{
        AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest,
        InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
        PermissionOptionKind, PromptRequest, PromptResponse, RequestPermissionOutcome,
        RequestPermissionRequest, SessionNotification, SessionUpdate, StopReason,
        TextContent, ToolCallUpdate, ToolCallUpdateFields, WriteTextFileRequest,
    },
};

fn main() -> Result<()> {
    futures::executor::block_on(
        Agent
            .builder()
            .name("mock-acp-agent")
            .on_receive_request(
                async move |init: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(init.protocol_version)
                            .agent_capabilities(AgentCapabilities::new()),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_req: NewSessionRequest, responder, _cx| {
                    responder.respond(NewSessionResponse::new("mock-session"))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |req: PromptRequest, responder, cx| {
                    let connection = cx.clone();
                    cx.spawn(async move {
                        let session_id = req.session_id.clone();
                        connection.send_notification(SessionNotification::new(
                            session_id.clone(),
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new("hello")),
                            )),
                        ))?;
                        let permission = connection
                            .send_request(RequestPermissionRequest::new(
                                session_id.clone(),
                                ToolCallUpdate::new(
                                    "tool-1",
                                    ToolCallUpdateFields::new(),
                                ),
                                vec![
                                    PermissionOption::new(
                                        "allow",
                                        "Allow",
                                        PermissionOptionKind::AllowOnce,
                                    ),
                                    PermissionOption::new(
                                        "reject",
                                        "Reject",
                                        PermissionOptionKind::RejectOnce,
                                    ),
                                ],
                            ))
                            .block_task()
                            .await?;
                        let allowed = matches!(
                            permission.outcome,
                            RequestPermissionOutcome::Selected(ref selected)
                                if selected.option_id.to_string() == "allow"
                        );
                        if allowed {
                            connection
                                .send_request(WriteTextFileRequest::new(
                                    session_id,
                                    "/mock/out.txt",
                                    "written by mock",
                                ))
                                .block_task()
                                .await?;
                        }
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    })
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(Stdio::new()),
    )
}
```

- [ ] **Step 3: Write the failing integration test**

Create `lapce-proxy/tests/agent_session.rs`:

```rust
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
        ProxyHandler, ProxyNotification, ProxyRequest, ProxyResponse, ProxyRpcHandler,
    },
};
use parking_lot::Mutex;

const EVENT_TIMEOUT: Duration = Duration::from_secs(20);

/// Stands in for the dispatcher: records agent writes and answers file requests.
struct FakeDispatcher {
    proxy_rpc: ProxyRpcHandler,
    writes: Arc<Mutex<Vec<(PathBuf, String)>>>,
}

impl ProxyHandler for FakeDispatcher {
    fn handle_notification(&mut self, _rpc: ProxyNotification) {}

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

    /// Waits for the next agent event, skipping unrelated notifications.
    fn next_event(&self) -> AgentEvent {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.core_rpc.rx().recv_timeout(left) {
                Ok(CoreRpc::Notification(n)) => {
                    if let CoreNotification::AgentEvent { event } = *n {
                        return event;
                    }
                }
                Ok(_) => {}
                Err(_) => panic!("timed out waiting for an agent event"),
            }
        }
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
    fn drop(&mut self) {
        self.manager.stop();
        self.proxy_rpc.shutdown();
    }
}

/// Path of the mock agent example binary built by `cargo test`.
fn mock_agent_command() -> String {
    let exe = std::env::current_exe().unwrap();
    let debug_dir = exe.parent().unwrap().parent().unwrap();
    debug_dir
        .join("examples")
        .join(format!("mock_acp_agent{}", std::env::consts::EXE_SUFFIX))
        .to_string_lossy()
        .to_string()
}

fn mock_config() -> AgentServerConfig {
    AgentServerConfig {
        command: mock_agent_command(),
        args: vec![],
        env: Default::default(),
    }
}

#[test]
fn allowed_permission_lets_the_agent_write_and_finish() {
    let mut rig = Rig::new();
    rig.manager.start(mock_config(), Some(PathBuf::from("/mock")));
    rig.manager.prompt("go".to_string(), vec![]);

    let request_id = rig.wait_for(|event| match event {
        AgentEvent::PermissionRequest { request_id, .. } => Some(*request_id),
        _ => None,
    });
    rig.manager
        .permission_reply(request_id, Some("allow".to_string()));
    rig.wait_for(|event| matches!(event, AgentEvent::TurnEnded { .. }).then_some(()));

    let writes = rig.writes.lock();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, PathBuf::from("/mock/out.txt"));
    assert_eq!(writes[0].1, "written by mock");
}

#[test]
fn rejected_permission_prevents_the_write() {
    let mut rig = Rig::new();
    rig.manager.start(mock_config(), Some(PathBuf::from("/mock")));
    rig.manager.prompt("go".to_string(), vec![]);

    let request_id = rig.wait_for(|event| match event {
        AgentEvent::PermissionRequest { request_id, .. } => Some(*request_id),
        _ => None,
    });
    rig.manager.permission_reply(request_id, None);
    rig.wait_for(|event| matches!(event, AgentEvent::TurnEnded { .. }).then_some(()));

    assert!(rig.writes.lock().is_empty());
}

#[test]
fn stopping_with_a_pending_permission_does_not_hang_the_agent() {
    let mut rig = Rig::new();
    rig.manager.start(mock_config(), Some(PathBuf::from("/mock")));
    rig.manager.prompt("go".to_string(), vec![]);
    rig.wait_for(|event| {
        matches!(event, AgentEvent::PermissionRequest { .. }).then_some(())
    });

    rig.manager.stop();

    rig.wait_for(|event| {
        matches!(
            event,
            AgentEvent::Status {
                status: AgentStatus::Disconnected { .. }
            }
        )
        .then_some(())
    });
    assert!(rig.writes.lock().is_empty());
}

#[test]
fn write_outside_the_workspace_is_refused_by_the_path_guard() {
    let mut rig = Rig::new();
    // The mock writes /mock/out.txt, which is outside this workspace.
    rig.manager.start(mock_config(), Some(PathBuf::from("/other")));
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
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p lapce-proxy --test agent_session`
Expected: FAIL to compile, "unresolved import `lapce_proxy::agent::AgentManager`".

- [ ] **Step 5: Write the session driver**

Create `lapce-proxy/src/agent/session.rs` (the ACP wiring was compile-checked against the crate):

```rust
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
            RequestPermissionResponse, SelectedPermissionOutcome, SessionNotification,
            StopReason, TextContent, WriteTextFileRequest, WriteTextFileResponse,
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
    Prompt {
        text: String,
        contexts: Vec<AgentContext>,
    },
    Cancel,
}

/// Everything a session needs from the proxy.
pub struct SessionEnv {
    pub core_rpc: CoreRpcHandler,
    pub proxy_rpc: ProxyRpcHandler,
    pub broker: Arc<PermissionBroker>,
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
```

- [ ] **Step 6: Write the manager**

Replace `lapce-proxy/src/agent/mod.rs` with:

```rust
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
    agent::{AgentContext, AgentEvent, AgentRequestId, AgentServerConfig, AgentStatus},
    core::{CoreNotification, CoreRpcHandler},
    proxy::ProxyRpcHandler,
};
use permission::PermissionBroker;
use session::{SessionCommand, SessionEnv, run_session};

/// Owns the agent session thread and forwards UI commands to it.
pub struct AgentManager {
    core_rpc: CoreRpcHandler,
    proxy_rpc: ProxyRpcHandler,
    broker: Arc<PermissionBroker>,
    cmd_tx: Option<mpsc::UnboundedSender<SessionCommand>>,
}

impl AgentManager {
    /// Creates a manager with no running session.
    pub fn new(core_rpc: CoreRpcHandler, proxy_rpc: ProxyRpcHandler) -> Self {
        Self {
            core_rpc,
            proxy_rpc,
            broker: Arc::new(PermissionBroker::new()),
            cmd_tx: None,
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
        self.emit(AgentEvent::Status {
            status: AgentStatus::Starting,
        });
        let (cmd_tx, cmd_rx) = mpsc::unbounded();
        self.cmd_tx = Some(cmd_tx);
        let env = Arc::new(SessionEnv {
            core_rpc: self.core_rpc.clone(),
            proxy_rpc: self.proxy_rpc.clone(),
            broker: self.broker.clone(),
            workspace,
        });
        let core_rpc = self.core_rpc.clone();
        std::thread::Builder::new()
            .name("AgentSession".to_owned())
            .spawn(move || {
                let result = futures::executor::block_on(run_session(config, env, cmd_rx));
                let reason = match result {
                    Ok(()) => "session closed".to_string(),
                    Err(err) => format!("{err}"),
                };
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
    pub fn permission_reply(&self, request_id: AgentRequestId, option_id: Option<String>) {
        self.broker.reply(request_id, option_id);
    }

    /// Stops the session: rejects pending permissions and closes the command channel.
    pub fn stop(&mut self) {
        self.broker.cancel_all();
        self.cmd_tx = None;
    }
}
```

- [ ] **Step 7: Wire the dispatcher**

In `lapce-proxy/src/dispatch.rs`:
- add a field `agent: crate::agent::AgentManager,` to `Dispatcher`
- in `Dispatcher::new` (near line 1212), build it before the `Self { .. }` literal, because `core_rpc` and `proxy_rpc` are moved into the literal: add `let agent = crate::agent::AgentManager::new(core_rpc.clone(), proxy_rpc.clone());` right after the `file_watcher` line, and add `agent,` to the literal
- in `handle_notification`'s `match`, inside the `Shutdown {}` arm add `self.agent.stop();` as the first statement, and add these arms next to it:

```rust
            AgentStart { config } => {
                self.agent.start(config, self.workspace.clone());
            }
            AgentPrompt { text, contexts } => {
                self.agent.prompt(text, contexts);
            }
            AgentCancel {} => {
                self.agent.cancel();
            }
            AgentPermissionReply {
                request_id,
                option_id,
            } => {
                self.agent.permission_reply(request_id, option_id);
            }
            AgentStop {} => {
                self.agent.stop();
            }
```

- [ ] **Step 8: Run the integration tests**

Run: `cargo test -p lapce-proxy --test agent_session -- --test-threads=1`
Expected: PASS (5 tests). If the mock path is not found, run `cargo build -p lapce-proxy --example mock_acp_agent` first (cargo builds examples for `cargo test`, but a filtered run may skip it). If `missing_agent_binary...` hangs, the spawn error is not surfacing through `run_session`'s result; in that case log the error and return `Err` from `connect_with` before the handshake (do not add a retry).

Then run `cargo test -p lapce-proxy` to confirm all unit tests still pass.

- [ ] **Step 9: Add `AgentData` and the UI arms**

In `lapce-app/src/agent.rs`, above the tests, add `AgentData` (imports at the top of the file: `use std::{path::Path, rc::Rc};`, `use floem::reactive::{RwSignal, Scope, SignalGet, SignalUpdate, SignalWith};`, `use lapce_core::{buffer::EditType, selection::Selection};`, `use crate::{main_split::MainSplitData, window_tab::CommonData};`):

```rust
/// Reactive wrapper around [`AgentState`] plus the pieces needed to edit buffers.
#[derive(Clone)]
pub struct AgentData {
    pub state: RwSignal<AgentState>,
    pub main_split: MainSplitData,
    pub common: Rc<CommonData>,
}

impl AgentData {
    /// Creates the agent data with an empty transcript.
    pub fn new(cx: Scope, main_split: MainSplitData, common: Rc<CommonData>) -> Self {
        Self {
            state: cx.create_rw_signal(AgentState::default()),
            main_split,
            common,
        }
    }

    /// Applies an event coming from the proxy.
    pub fn handle_event(&self, event: AgentEvent) {
        self.state.update(|state| state.apply(&event));
    }

    /// Replaces the whole content of an open document as one undoable edit.
    /// Does nothing if the document is not open or the text is unchanged.
    pub fn apply_edit(&self, path: &Path, content: &str) {
        let Some(doc) = self.main_split.docs.with_untracked(|docs| docs.get(path).cloned())
        else {
            return;
        };
        let (len, same) = doc
            .buffer
            .with_untracked(|buffer| (buffer.len(), buffer.to_string() == content));
        if same {
            return;
        }
        doc.do_raw_edit(&[(Selection::region(0, len), content)], EditType::Other);
    }
}
```

Create it in `WindowTabData::new` right after the `plugin` is created (near `lapce-app/src/window_tab.rs:515`): `let agent = AgentData::new(cx, main_split.clone(), common.clone());`, add the field `pub agent: AgentData,` to `WindowTabData` next to `pub plugin: PluginData,`, and add `agent,` to the struct construction. In `handle_core_notification` replace the temporary `AgentApplyEdit` arm from Task 6 (if you added one) and add:

```rust
            CoreNotification::AgentEvent { event } => {
                self.agent.handle_event(event.clone());
            }
            CoreNotification::AgentApplyEdit { path, content } => {
                self.agent.apply_edit(path, content);
            }
```

Fix import paths as the compiler directs (`Selection` is `lapce_core::selection::Selection`, `EditType` is `lapce_core::buffer::EditType`, `Buffer::len` comes from the `BufferContent`/rope-text trait already used in `doc.rs`).

- [ ] **Step 10: Build and run all touched tests**

Run: `cargo check --workspace && cargo test -p lapce-rpc -p lapce-proxy -p lapce-app`
Expected: clean build and all tests pass.

- [ ] **Step 11: Commit**

Append to `CHANGES.md`: `- Added the ACP session driver (AgentManager), the mock agent integration tests, and the proxy/UI protocol wiring. Terminal capability is not advertised in v1.`

```bash
git add lapce-proxy lapce-rpc lapce-app/src/agent.rs lapce-app/src/window_tab.rs Cargo.lock CHANGES.md
git commit -m "feat(agent): add ACP session driver and protocol wiring"
```

---

### Task 9: The panel

**Files:**
- Create: `lapce-app/src/panel/agent_view.rs`, `icons/codicons/agent.svg`
- Modify: `lapce-app/src/panel/{kind,mod,data,view}.rs`, `lapce-app/src/config/icon.rs`, `defaults/icon-theme.toml`, `lapce-app/src/{command,window_tab,doc,agent}.rs`, `CHANGES.md`

**Interfaces:**
- Consumes: `AgentData`, `AgentState`, `AgentItem`, `PendingPermission`, `ProxyRpcHandler::agent_*`, `LapceConfig::agent.resolve()`.
- Produces: `PanelKind::Agent` (default position `RightTop`, focusable), `PanelSection::Agent`, `agent_panel(window_tab_data, position) -> impl View`, `AgentData::{input, send_input, cancel, reply_permission, restart}`, commands `toggle_agent_visual` and `toggle_agent_focus`.

This task is UI code that follows `lapce-app/src/panel/problem_view.rs` and `plugin_view.rs` (search input). Rust's exhaustive matches drive the wiring: after adding `PanelKind::Agent`, run `cargo check -p lapce-app` and fix each non-exhaustive match, using the guidance below. Import paths that the compiler flags are expected.

- [ ] **Step 1: Add the icon**

Find the icon folder: `find icons -name source-control.svg` (it is `icons/codicons/`). Create `icons/codicons/agent.svg`:

```svg
<svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" xmlns="http://www.w3.org/2000/svg"><path d="M2 3a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H8.4L5 14v-3H3a1 1 0 0 1-1-1V3zm1 0v7h3v1.9L7.9 10H13V3H3zm2 2h6v1H5V5zm0 2h4v1H5V7z"/></svg>
```

Add to `lapce-app/src/config/icon.rs` next to `SCM`: `pub const AGENT: &'static str = "agent";` and to `defaults/icon-theme.toml` next to the `scm.icon` line: `"agent" = "agent.svg"`. Confirm `icon-theme.toml` entries in the same table use plain names like `"file_explorer" = "files.svg"` (they do).

- [ ] **Step 2: Register the panel kind**

In `lapce-app/src/panel/kind.rs`:
- add `Agent,` to `enum PanelKind`
- `svg_name`: `PanelKind::Agent => LapceIcons::AGENT,`
- `default_position`: `PanelKind::Agent => PanelPosition::RightTop,`

In `lapce-app/src/panel/data.rs`: in `default_panel_order`, change the `RightTop` entry to `im::vector![PanelKind::Agent, PanelKind::DocumentSymbol,]`, and add `Agent` to `enum PanelSection` and to wherever `PanelSection` defaults are initialized (search for `PanelSection::Problem` / `PanelSection::Error` and follow the pattern; default open = `true`). Existing users' saved panel order gets the new kind appended automatically by `LapceDb::get_panel_orders`.

In `lapce-app/src/panel/mod.rs` add `pub mod agent_view;`.

- [ ] **Step 3: Input, send, cancel and permission handling in `AgentData`**

In `lapce-app/src/agent.rs` extend `AgentData` with an input editor and actions. Add the field `pub input: EditorData,` (import `crate::editor::EditorData`), create it in `AgentData::new` with `main_split.editors.make_local(cx, common.clone())`, then add these methods:

```rust
    /// Reads and clears the input box. Returns `None` when it only has whitespace.
    fn take_input_text(&self) -> Option<String> {
        let doc = self.input.doc();
        let text = doc.buffer.with_untracked(|buffer| buffer.to_string());
        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let len = doc.buffer.with_untracked(|buffer| buffer.len());
        doc.do_raw_edit(&[(Selection::region(0, len), "")], EditType::Other);
        Some(text)
    }

    /// Builds the context attached to a prompt: the active file, and its selection if any.
    fn active_context(&self) -> Vec<AgentContext> {
        let Some(editor) = self.main_split.active_editor.get_untracked() else {
            return Vec::new();
        };
        let doc = editor.doc();
        let Some(path) = doc.content.with_untracked(|content| content.path().cloned())
        else {
            return Vec::new();
        };
        let selection = editor
            .cursor()
            .with_untracked(|cursor| cursor.get_selection())
            .filter(|(start, end)| start != end)
            .map(|(start, end)| {
                doc.buffer
                    .with_untracked(|buffer| buffer.slice_to_cow(start..end).to_string())
            });
        vec![AgentContext { path, selection }]
    }

    /// Starts the configured agent server. Reports configuration problems in the transcript.
    pub fn restart(&self) {
        match self.common.config.get_untracked().agent.resolve() {
            Ok(config) => self.common.proxy.agent_start(config),
            Err(message) => self.handle_event(AgentEvent::Error { message }),
        }
    }

    /// Sends the input box content as a prompt, starting the agent first if needed.
    pub fn send_input(&self) {
        if self.state.with_untracked(|state| state.busy) {
            return;
        }
        let Some(text) = self.take_input_text() else {
            return;
        };
        let connected = self
            .state
            .with_untracked(|state| matches!(state.status, AgentStatus::Ready | AgentStatus::Starting));
        if !connected {
            self.restart();
        }
        self.state.update(|state| state.push_user(&text));
        self.common.proxy.agent_prompt(text, self.active_context());
    }

    /// Cancels the running turn.
    pub fn cancel(&self) {
        self.common.proxy.agent_cancel();
    }

    /// Answers the pending permission request with `option_id`, or rejects it with `None`.
    pub fn reply_permission(&self, option_id: Option<String>) {
        let Some(pending) = self.state.try_update(|state| state.take_pending()).flatten()
        else {
            return;
        };
        self.common
            .proxy
            .agent_permission_reply(pending.request_id, option_id);
    }
```

Implement `KeyPressFocus` for `AgentData` by copying the `impl KeyPressFocus for PluginData` block from `lapce-app/src/plugin.rs:112` and changing: `query_editor` to `self.input`, and the `InsertNewLine` arm to send instead of ignoring it:

```rust
                    CommandKind::Edit(EditCommand::InsertNewLine) => {
                        self.send_input();
                        return CommandExecuted::Yes;
                    }
```

so Enter sends the prompt. In `receive_char` forward to `self.input.receive_char(c)`.

- [ ] **Step 4: The view**

Create `lapce-app/src/panel/agent_view.rs`. Use the structure below; use `LapceColor` values that exist (`EDITOR_FOREGROUND`, `LAPCE_BORDER`, `EDITOR_DIM`, and `LAPCE_ERROR` if present, else `EDITOR_FOREGROUND`). The compiler will point out any import that differs:

```rust
//! The AI agent panel: transcript, permission prompt, and input box.

use std::rc::Rc;

use floem::{
    View,
    reactive::{SignalGet, SignalWith},
    style::CursorStyle,
    views::{Decorators, container, dyn_stack, label, scroll, stack},
};
use lapce_rpc::agent::AgentToolStatus;

use super::{data::PanelSection, position::PanelPosition, view::PanelBuilder};
use crate::{
    agent::{AgentData, AgentItem},
    config::color::LapceColor,
    text_input::TextInputBuilder,
    window_tab::{Focus, WindowTabData},
};
use crate::panel::kind::PanelKind;

/// Builds the agent panel.
pub fn agent_panel(
    window_tab_data: Rc<WindowTabData>,
    position: PanelPosition,
) -> impl View {
    let config = window_tab_data.common.config;
    let agent = window_tab_data.agent.clone();
    PanelBuilder::new(config, position)
        .add(
            "Agent",
            agent_body(window_tab_data.clone(), agent),
            window_tab_data.panel.section_open(PanelSection::Agent),
        )
        .build()
        .debug_name("Agent Panel")
}

/// A small text button.
fn button(
    window_tab_data: Rc<WindowTabData>,
    text: impl Into<String>,
    on_click: impl Fn() + 'static,
) -> impl View {
    let config = window_tab_data.common.config;
    let text: String = text.into();
    label(move || text.clone())
        .on_click_stop(move |_| on_click())
        .style(move |s| {
            let config = config.get();
            s.padding_horiz(8.0)
                .padding_vert(3.0)
                .margin_right(6.0)
                .border(1.0)
                .border_radius(4.0)
                .border_color(config.color(LapceColor::LAPCE_BORDER))
                .cursor(CursorStyle::Pointer)
        })
}

/// One transcript entry.
fn item_view(window_tab_data: Rc<WindowTabData>, item: AgentItem) -> impl View {
    let config = window_tab_data.common.config;
    let (prefix, text) = match &item {
        AgentItem::User(text) => ("You", text.clone()),
        AgentItem::Assistant(text) => ("Agent", text.clone()),
        AgentItem::Thought(text) => ("Thinking", text.clone()),
        AgentItem::Tool { title, status, .. } => {
            let mark = match status {
                AgentToolStatus::Pending => "...",
                AgentToolStatus::InProgress => ">>",
                AgentToolStatus::Completed => "ok",
                AgentToolStatus::Failed => "failed",
            };
            ("Tool", format!("[{mark}] {title}"))
        }
        AgentItem::Error(text) => ("Error", text.clone()),
    };
    stack((
        label(move || prefix.to_string()).style(move |s| {
            s.font_bold()
                .color(config.get().color(LapceColor::EDITOR_DIM))
        }),
        label(move || text.clone()).style(|s| s.min_width(0.0).flex_grow(1.0)),
    ))
    .style(|s| s.flex_col().width_pct(100.0).padding_horiz(10.0).padding_vert(4.0))
}

/// The permission prompt shown while the agent waits for a decision.
fn permission_bar(window_tab_data: Rc<WindowTabData>, agent: AgentData) -> impl View {
    let config = window_tab_data.common.config;
    let pending = {
        let agent = agent.clone();
        move || agent.state.with(|state| state.pending.clone())
    };
    dyn_stack(
        move || pending().into_iter().collect::<Vec<_>>(),
        |pending| pending.request_id,
        move |pending| {
            let agent = agent.clone();
            let window_tab_data = window_tab_data.clone();
            let buttons = pending
                .options
                .iter()
                .map(|option| {
                    let agent = agent.clone();
                    let id = option.id.clone();
                    button(window_tab_data.clone(), option.name.clone(), move || {
                        agent.reply_permission(Some(id.clone()));
                    })
                })
                .collect::<Vec<_>>();
            let reject_agent = agent.clone();
            stack((
                label(move || format!("Agent asks: {}", pending.title)),
                stack((
                    floem::views::stack_from_iter(buttons),
                    button(window_tab_data, "Reject", move || {
                        reject_agent.reply_permission(None);
                    }),
                ))
                .style(|s| s.margin_top(4.0)),
            ))
            .style(move |s| {
                s.flex_col()
                    .width_pct(100.0)
                    .padding(8.0)
                    .border_top(1.0)
                    .border_color(config.get().color(LapceColor::LAPCE_BORDER))
            })
        },
    )
    .style(|s| s.flex_col().width_pct(100.0))
}

/// The whole panel body: toolbar, transcript, permission bar, input box.
fn agent_body(window_tab_data: Rc<WindowTabData>, agent: AgentData) -> impl View {
    let config = window_tab_data.common.config;
    let focus = window_tab_data.common.focus;
    let is_focused = move || focus.get() == Focus::Panel(PanelKind::Agent);

    let toolbar = stack((
        button(window_tab_data.clone(), "Restart", {
            let agent = agent.clone();
            move || agent.restart()
        }),
        button(window_tab_data.clone(), "Cancel", {
            let agent = agent.clone();
            move || agent.cancel()
        }),
        label({
            let agent = agent.clone();
            move || agent.state.with(|state| format!("{:?}", state.status))
        })
        .style(move |s| s.color(config.get().color(LapceColor::EDITOR_DIM))),
    ))
    .style(|s| s.padding(6.0).items_center());

    let transcript = scroll(
        dyn_stack(
            {
                let agent = agent.clone();
                move || {
                    agent
                        .state
                        .with(|state| state.items.iter().cloned().enumerate().collect::<Vec<_>>())
                }
            },
            |(index, item)| (*index, item.render_key()),
            {
                let window_tab_data = window_tab_data.clone();
                move |(_, item)| item_view(window_tab_data.clone(), item)
            },
        )
        .style(|s| s.flex_col().width_pct(100.0)),
    )
    .style(|s| s.flex_grow(1.0).flex_basis(0.0).min_height(0.0).width_pct(100.0));

    let input = stack((
        TextInputBuilder::new()
            .is_focused(is_focused)
            .build_editor(agent.input.clone())
            .placeholder(|| "Ask the agent (Enter to send)".to_string())
            .style(|s| s.padding_vert(4.0).padding_horiz(10.0).width_pct(100.0)),
        button(window_tab_data.clone(), "Send", {
            let agent = agent.clone();
            move || agent.send_input()
        }),
    ))
    .style(move |s| {
        s.items_center()
            .width_pct(100.0)
            .border_top(1.0)
            .border_color(config.get().color(LapceColor::LAPCE_BORDER))
    });

    container(
        stack((
            toolbar,
            transcript,
            permission_bar(window_tab_data.clone(), agent),
            input,
        ))
        .style(|s| s.flex_col().size_pct(100.0, 100.0)),
    )
    .style(|s| s.size_pct(100.0, 100.0))
}
```

Compile-driven adjustments to expect here: `floem::views::stack_from_iter` may live under a different path in this floem version (search `stack_from_iter` in the floem source), and `EditorData::cursor` is a signal field, so use `editor.cursor` (not `editor.cursor()`) in `active_context` if the compiler rejects the call.

- [ ] **Step 5: Wire the panel into the existing matches**

Run `cargo check -p lapce-app` and fix every non-exhaustive match on `PanelKind`:

- `lapce-app/src/panel/view.rs`, view match (near line 473): `PanelKind::Agent => agent_panel(window_tab_data.clone(), position).into_any(),` (import `agent_view::agent_panel`).
- `lapce-app/src/panel/view.rs`, tooltip match (near line 553): `PanelKind::Agent => "Agent",`.
- `lapce-app/src/window_tab.rs` `toggle_panel_focus` (near line 2680): add `PanelKind::Agent` to the arm `PanelKind::Terminal | PanelKind::SourceControl | PanelKind::Search` (it accepts focus).
- `lapce-app/src/window_tab.rs` key routing (near line 2404): add `Focus::Panel(PanelKind::Agent) => Some(keypress.key_down(event, &self.agent)),`.
- `lapce-app/src/doc.rs` (near line 2179): add `| Focus::Panel(PanelKind::Agent)` to the `matches!` list so keyboard events reach the panel.

Add two commands in `lapce-app/src/command.rs` following the exact pattern of `ToggleProblemVisual` / `ToggleProblemFocus` (same `#[strum(serialize = ..)]` and `#[strum(message = ..)]` attributes): `ToggleAgentVisual` (`toggle_agent_visual`, "Toggle Agent Visual") and `ToggleAgentFocus` (`toggle_agent_focus`, "Toggle Agent Focus"). Handle them in `window_tab.rs` next to the Problem ones: `ToggleAgentVisual => self.toggle_panel_visual(PanelKind::Agent),` and `ToggleAgentFocus => self.toggle_panel_focus(PanelKind::Agent),`. Do not add default keybindings (users bind them).

- [ ] **Step 6: Build, lint, test**

Run: `cargo fmt --all && cargo clippy -p lapce-rpc -p lapce-proxy -p lapce-app -- -D warnings && cargo test -p lapce-rpc -p lapce-proxy -p lapce-app`
Expected: clean, all tests pass.

- [ ] **Step 7: Manual smoke test with the mock agent**

Run `scripts/build-debug.sh` (or `cargo run -p lapce-app`). In Settings (`settings.toml`) point `default-server` at a temporary server entry that runs the mock: 

```toml
[agent]
default-server = "mock"
[agent.servers.mock]
command = "/home/fabio/dev/projects/lapce/target/debug/examples/mock_acp_agent"
```

Open the folder `/mock` (create it first: `mkdir -p /mock` or change the mock's path constant locally, do not commit that). Open the command palette, run "Toggle Agent Visual". Type a message, press Enter. Expected: "hello" streams in, a permission bar appears with Allow and Reject, Allow writes `/mock/out.txt`, transcript ends the turn, and Send is enabled again. Revert the `settings.toml` change.

- [ ] **Step 8: Commit**

Append to `CHANGES.md`: `- Added the Agent panel (right side by default) with transcript, permission prompt and input, plus toggle_agent_visual and toggle_agent_focus commands.`

```bash
git add lapce-app defaults icons CHANGES.md
git commit -m "feat(agent): add the agent panel"
```

---

### Task 10: Real agents, security review, docs

**Files:**
- Modify: `CHANGES.md`, `docs/superpowers/specs/2026-09-24-agent-panel-design.md` (status line)

- [ ] **Step 1: Manual test with Claude Code**

Set `default-server = "claude-code"`. Open a real project, ask "add a doc comment to the main function", approve the edit. Expected: the file open in the editor changes as one undoable edit (press undo once to revert), the transcript shows tool cards, no crash. Also try an unsaved edit first: type a character without saving, ask the agent to read the file, and confirm its answer reflects the unsaved text (Review Focus item 4).

- [ ] **Step 2: Manual test with Gemini CLI**

Set `default-server = "gemini"`. Ask a question about the open file and confirm a streamed answer. If Gemini rejects the ACP flag, run `gemini --help`, fix the `arguments` in `defaults/settings.toml`, and commit that as a fix.

- [ ] **Step 3: Process cleanup check**

Start a session, then close the Lapce window. Run `pgrep -fa "claude-agent-acp|mock_acp_agent|gemini"`. Expected: no leftover agent process. If one remains, the `AcpAgent` child is not killed on drop: in `AgentManager::stop` and on proxy shutdown that is a bug to fix before merging (spawn the child with kill-on-drop or keep its handle and kill it explicitly), with a regression test in `agent_session.rs` that asserts the mock process is gone after `stop()`.

- [ ] **Step 4: Security review**

Invoke the `rafter-code-review` skill on the branch diff. The surface is: agent-controlled file paths (Task 3 guard), agent-controlled file contents written to disk, subprocess launch from user settings, and permission prompts. Address every finding or record why it does not apply. Then run `rafter run` on the same diff if the CLI is installed.

- [ ] **Step 5: Finalize**

Change the spec status line to `Status: implemented (v1)` and commit:

```bash
git add CHANGES.md docs/superpowers/specs/2026-09-24-agent-panel-design.md
git commit -m "docs(agent): mark the agent panel spec as implemented"
```

Remind the maintainer that `docs/superpowers/` is dropped before upstreaming (see commit `e4d7679f`).
