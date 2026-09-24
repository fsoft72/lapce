# Changes

## Editor font zoom

- Added `editor_font_increase`, `editor_font_decrease` and `editor_font_reset` commands. They change `editor.font-size` (clamped to 6-32) and persist it in `settings.toml`.
- Added `Ctrl+=`, `Ctrl++` and `Ctrl+-` keymaps (`Cmd` on macOS), active only with `editor_focus`. The window-wide `zoom_in` / `zoom_out` keep working elsewhere.
- Added `Ctrl+mouse wheel` (`Cmd` on macOS) on the editor content and gutter to zoom the font instead of scrolling.

## Editor and terminal font zoom are independent

- Wheel direction inverted (wheel up zooms in) and each wheel event now changes the font by 2 points.
- Added `terminal_font_increase`, `terminal_font_decrease` and `terminal_font_reset` commands, with `Ctrl+=`, `Ctrl++`, `Ctrl+-` and `Ctrl+wheel` (`Cmd` on macOS) active only with terminal focus. They change `terminal.font-size`.
- Zooming the editor first pins `terminal.font-size` to its current value when it was inherited from the editor (`0`), so the terminal no longer changes with it.

## Build scripts

- Added `scripts/build-release.sh` (release build copied to `bin/lapcie`) and `scripts/build-debug.sh` (debug build copied to `bin/lapcie-debug`). `bin/` is git-ignored.

## AI agent panel

- Added `lapce_rpc::agent` with the shared types used by the agent panel (ACP client). Spec deltas from planning are recorded in the design spec.
- Added the ACP crate to lapce-proxy and a PermissionBroker for pending agent permission requests.
- Added the workspace path guard and line slicing used to serve agent file requests.
- Added the prompt builder that attaches selection or file context to the user message.
- Added mapping from ACP session updates and permission requests to UI events.
- Added ProxyRequest::AgentReadFile/AgentWriteFile and CoreNotification::AgentApplyEdit so the agent reads unsaved buffers and edits open files through the UI.
- Added [agent] settings (default-server, servers) and the AgentState transcript model for the panel.
- Added the ACP session driver (AgentManager), the mock agent integration tests, and the proxy/UI protocol wiring. Terminal capability is not advertised in v1.
- Stopping or restarting the agent now interrupts a handshake that never completes, and events from a replaced session are dropped (its permission prompts are rejected).
- Added the Agent panel (right side by default) with transcript, permission prompt and input, plus toggle_agent_visual and toggle_agent_focus commands.
- Agent server settings (`[agent]`) are now read only from the user settings file; a workspace `.lapce/settings.toml` cannot change the command Lapce launches.
- Added a regression test that the agent process is killed when a ready session is stopped; security review of the agent surface done, spec marked implemented (manual tests with real agents pending).
