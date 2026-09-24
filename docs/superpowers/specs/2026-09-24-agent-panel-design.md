# AI Agent Panel (ACP client) - Design

Date: 2026-09-24
Status: implemented (v1), manual agent tests pending

## Goal

Add an AI agent panel to Lapce, similar to Zed and VS Code, to chat with an external coding agent that knows the project context and can modify code in the editor. Lapce acts as an ACP (Agent Client Protocol) client. It does not contain its own LLM loop.

## Decisions

- Protocol: ACP, agent runs as a subprocess speaking JSON-RPC over stdio.
- Agent host: `lapce-proxy`, so remote, WSL and SSH workspaces work.
- Edit flow (v1): edits are applied to buffers with per-request approval. No diff review UI.
- Agents in v1: Claude Code (via ACP adapter) and Gemini CLI. Others are configurable through settings but not tested.
- Protocol implementation: official `agent-client-protocol` Rust crate. Fallback: hand-rolled JSON-RPC over `lapce-rpc/src/stdio.rs`, keeping all ACP logic behind one module boundary.

## Non-goals (v1)

- Diff review UI with accept/reject hunks.
- Multiple concurrent sessions and history persistence.
- Inline-edit mode inside the editor.
- A built-in LLM client or agent loop.

## Components

- `lapce-rpc`: new agent messages.
  - UI to proxy: `AgentStart(config)`, `AgentPrompt(text, context refs)`, `AgentCancel`, `AgentPermissionReply`.
  - Proxy to UI: `AgentMessageChunk`, `AgentToolCall`, `AgentPermissionRequest`, `AgentStatus`, plus request/response pairs for buffer read and write (see File requests).
- `lapce-proxy/src/agent/`: spawns the agent process, runs the ACP session, serves the agent's file and terminal requests. Depends on the proxy buffer and terminal layers.
- `lapce-app/src/agent/`: chat data model (messages, tool cards, pending permission). No UI code.
- `lapce-app/src/panel/agent_view.rs`: floem view. New `PanelKind::Agent` in `panel/kind.rs`, new icon, entry in the panel position defaults.
- Config: `[agent.servers.<name>]` with `command`, `args`, `env`. Claude Code and Gemini CLI ship as defaults.

## Data flow

Prompt: the user types in the panel. The UI sends `AgentPrompt` with context refs (current file, selection, `@file` mentions). The proxy converts them to ACP content blocks and streams `session/update` notifications back as `AgentMessageChunk` and `AgentToolCall`.

File requests (`fs/read_text_file`, `fs/write_text_file`):
- Unsaved buffer content lives in the UI (`doc.rs`). The proxy forwards these requests to the UI as request/response.
- Read: the UI returns buffer content if the file is open, else the proxy reads from disk.
- Write: the UI applies the change to the buffer as a single undoable edit. If the file is not open, the proxy writes to disk.
- To verify before implementation: whether `lapce-proxy/src/buffer.rs` keeps a synced buffer copy that would make forwarding unnecessary.

Permissions: `session/request_permission` becomes `AgentPermissionRequest`. The panel shows the options supplied by the agent. The proxy blocks the agent call until `AgentPermissionReply`. Writes and commands are gated. Reads are not.

Terminal requests: reuse `lapce-proxy/src/terminal.rs`. Output streams back to the agent and appears in a tool card.

Cancel: `AgentCancel` sends `session/cancel` and rejects any pending permission.

## Error handling

- Spawn failure or missing binary: status line with the exact error and the command tried.
- Agent process exit: panel shows "disconnected" with a restart button. No silent retry.
- Protocol error: logged with context and shown as an error card in the chat.

## Testing

- Unit tests for RPC message mapping and the permission state machine.
- Integration test with a mock ACP agent (stdio script replaying canned responses), no real agent in CI.
- Manual test against Claude Code and Gemini CLI.

## Open questions

- Async runtime fit of the ACP crate with the proxy threading model.
- Buffer ownership question above.
- Panel default position (bottom vs right side).

## Changes found during planning

- Terminal requests are out of v1: `terminal` capability is not advertised. The agent uses its own shell tool and asks permission through `session/request_permission`.
- ACP has no client-side gate on `fs/write_text_file`. Permission prompts come from the agent. Lapce enforces a workspace path guard on every read and write.
- `CoreRequest` is empty and the UI has no reply path, so file writes to open buffers use a `CoreNotification::AgentApplyEdit`, reads run in the dispatcher (`ProxyRequest::AgentReadFile`), writes to closed files go to disk in the proxy (`ProxyRequest::AgentWriteFile`).
- Resolved: the ACP crate is runtime agnostic (no tokio), so it runs on a dedicated proxy thread with `futures::executor::block_on`. The proxy holds synced buffers, but inside the single-threaded dispatcher.
- Default panel position: right side.
- Known limitation: right after an agent write to an open buffer, the proxy copy updates when the UI's `Update` arrives, so an immediate re-read can briefly see old text.
- Context in v1 is the active file, or its selection when there is one. `@file` mentions and context chips are deferred. Enter sends the prompt; multi-line input is not supported yet.
