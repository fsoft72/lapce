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
- Final review fixes: restart/cancel never leave the panel busy, permission prompts queue, agent edits to clean open files are saved (dirty ones get a note, read-only ones an error), reads see pending agent writes, and the agent process tree is killed synchronously on stop and on window/app exit.
- The agent command is resolved with PATHEXT on Windows (`npx` -> `npx.cmd`), errors name the command line and the agent's stderr, the status label is readable, and session errors are logged.
- Added a small agent icon at the bottom right of the status bar to open / close the agent chat panel.
- Added an "Agent" section to Settings: default server dropdown plus command and arguments of the selected server (saved in `agent.servers.<name>` of the user settings).
- Replaced the default `gemini` agent server with `agy` in `defaults/settings.toml` (no ACP flag: `agy` exposes none in `--help`).
- Fixed unused variable warnings in `logging.rs` and `agent_view.rs`.
- Added ACP logging in lapce-proxy: lifecycle events at `info` (spawn, initialize, session, prompt, turn end, permissions, exit) and every raw JSON-RPC line plus agent stderr at `debug` (target `lapce_proxy::agent::wire`). Lines go to the Lapce log file; on the console use `LAPCE_LOG=lapce_proxy=debug`.
- The agent communication log is now also written to `lapce-agent.log` in the current working directory (appended, debug level, only `lapce_proxy::agent` targets).
- Added a `pi` agent server (`npx -y pi-acp`) to the default settings; initialize and session/new verified by hand against the adapter.
- Fixed a crash (xi-rope panic) when typing a second prompt: sending a prompt now resets the input box cursor along with its content.
- Config tests now use the `agy` default server (they broke when it replaced the old default).
- Removed the `gemini` and `agy` agent servers everywhere (they do not work over ACP); `pi` is the second default server. Config tests and the design docs updated accordingly.
- Fixed the prompt box wrapping every letter onto a new line: it now wraps at the width of the box instead of the width of its own text.
- Added `@file` mentions to the agent prompt box: typing `@` plus a name lists the matching workspace files (`.git` and `node_modules` excluded); Up/Down/Enter/Tab/Esc drive the list, and the mentioned files are attached to the prompt as context.
