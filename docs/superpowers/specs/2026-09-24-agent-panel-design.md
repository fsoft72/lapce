# AI Agent Panel (ACP client) - Design

Date: 2026-09-24
Status: draft, awaiting review

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
