# Changes

## Rebrand to GoldenIDE

- Project renamed from Lapce to GoldenIDE. Executable is `golden-ide`, remote proxy is `goldenide-proxy`. Crates are now `goldenide-app`, `goldenide-core`, `goldenide-proxy`, `goldenide-rpc` (directories renamed too). Identifiers (`GoldenIdeColor`, `GoldenIdeConfig`, ...), theme color keys (`goldenide.*`), env vars (`GOLDENIDE_*`), per-project dir (`.goldenide`), app id (`dev.goldenide.goldenide`) and packaging files were renamed accordingly. The user config and data directories change name, so existing Lapce settings are not picked up.
- Kept as is on purpose: the external `lapce-xi-rope` crate, git dependencies from `github.com/lapce/*`, and the wasm plugin ABI (import module `"lapce"`), so existing wasm plugins keep working. `LICENSE` and `CHANGELOG.md` are untouched.
- Logo images (`extra/images`, icons, `.ico`, `.icns`) are still the Lapce ones and must be replaced with GoldenIDE artwork.
- README, CONTRIBUTING, About dialog, metainfo and issue templates no longer point to Lapce website, docs, Discord or Matrix. About links to the upstream Lapce repository for attribution.

## Removed online features

- Removed the auto-updater (`update.rs`, "Restart to update" command, update badge and `updater` cargo feature) and the release workflow.
- Removed the tree-sitter grammar and query download from GitHub releases. Syntax highlighting only works with grammars already present in the local grammars directory.
- Removed the automatic download and upload of the remote proxy (SSH, WSL, containers). The `goldenide-proxy` binary must be installed manually on the remote host, otherwise the connection fails with an explicit error.
- Removed the plugin marketplace: Plugins panel, plugin details tab, `toggle_plugin_*` commands and keymaps, install, remove, enable and disable RPCs, registry download code and the `reqwest` HTTP helper (`get_url`).
- The WASI runtime and local `volt.toml` plugin loading were kept: language servers are started by wasm plugins, so removing the runtime would disable LSP. Plugins are only loaded from the local plugins directory.

## Editor font zoom

- Added `editor_font_increase`, `editor_font_decrease` and `editor_font_reset` commands. They change `editor.font-size` (clamped to 6-32) and persist it in `settings.toml`.
- Added `Ctrl+=`, `Ctrl++` and `Ctrl+-` keymaps (`Cmd` on macOS), active only with `editor_focus`. The window-wide `zoom_in` / `zoom_out` keep working elsewhere.
- Added `Ctrl+mouse wheel` (`Cmd` on macOS) on the editor content and gutter to zoom the font instead of scrolling.

## Editor and terminal font zoom are independent

- Wheel direction inverted (wheel up zooms in) and each wheel event now changes the font by 2 points.
- Added `terminal_font_increase`, `terminal_font_decrease` and `terminal_font_reset` commands, with `Ctrl+=`, `Ctrl++`, `Ctrl+-` and `Ctrl+wheel` (`Cmd` on macOS) active only with terminal focus. They change `terminal.font-size`.
- Zooming the editor first pins `terminal.font-size` to its current value when it was inherited from the editor (`0`), so the terminal no longer changes with it.

## Build scripts

- Added `scripts/build-release.sh` (release build copied to `bin/golden-ide`) and `scripts/build-debug.sh` (debug build copied to `bin/golden-ide-debug`). `bin/` is git-ignored.

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
- The agent transcript now shows the user prompts in a tinted, rounded box to tell them apart from the replies.
- Agent replies in the transcript are now rendered as Markdown (headings, emphasis, lists, code blocks); images are not shown.
- Each agent reply now ends with a Copy button that puts its raw Markdown in the clipboard.
- Copying a reply now shows a short "Copied to clipboard" toast over the agent panel.
- Hid the "Restart to update" entry (and the "No update available" line) in the gear menu and the update notification dot, behind the `SHOW_UPDATE_UI` constant in `title.rs`.
- Each user prompt in the agent transcript now has an Edit button (moves the prompt back into the input box, replacing its content) and a Rerun button (sends the prompt again).

## Code quality (OPTIMIZE.md)

- Split `lapce-app/src/app.rs` (4344 lines) into `app/editor_tab.rs`, `app/split.rs`, `app/widgets.rs`, `app/palette.rs`, `app/popups.rs`, `app/launch.rs` and `app/menu.rs`. `app.rs` keeps `AppData`, the workbench and the window views; the public functions are re-exported, so `crate::app::*` paths are unchanged.
- Split `lapce-app/src/editor.rs` (3926 lines): the `EditorData` methods now live in `editor/commands.rs` (command handling and `KeyPressFocus`), `editor/navigation.rs`, `editor/completion.rs`, `editor/search.rs` and `editor/pointer.rs`; `compute_screen_lines` moved to `editor/screen_lines.rs` and is re-exported.
- Split `lapce-app/src/main_split.rs` (3105 lines): the split tree types moved to `main_split/split_tree.rs`, `Editors` to `main_split/editors.rs`, and the `MainSplitData` methods to `main_split/split_ops.rs`, `main_split/editor_tab_ops.rs`, `main_split/jump.rs`, `main_split/diagnostics.rs` and `main_split/files.rs`. The public types are re-exported from `main_split`.
- Split `lapce-app/src/window_tab.rs` (3064 lines): the `WindowTabData` methods moved to `window_tab/workbench_commands.rs`, `window_tab/internal_commands.rs`, `window_tab/notifications.rs`, `window_tab/popup_origins.rs` and `window_tab/panels.rs`.
- Split `lapce-app/src/editor/view.rs` (2731 lines) into `editor/view/{sticky_header,gutter_view,breadcrumbs,zoom,find,changes}.rs` and `lapce-app/src/doc.rs` (2240 lines) into `doc/lsp.rs` (language server data), `doc/document.rs` (floem document traits) and `doc/styling.rs` (`DocStyling`, re-exported).
- Agent panel: the toolbar status and the permission bar now read focused memos (`AgentStateMemos`: `status`, `busy`, `permission`) instead of the whole `AgentState` signal, so a streamed chunk no longer re-runs them.
- Agent transcript: entries are keyed by their (stable) index and each entry view follows only its own revision (`AgentState::revision`), so a streamed chunk no longer clones and hashes the whole transcript; only the growing reply re-renders. `AgentItem::render_key` was removed, `AgentState::items` is now read through `items()` / `item(index)`.
- Agent replies are now rendered block by block: `split_markdown_blocks` (in `markdown.rs`) cuts the reply at top-level blank lines outside fenced code, and each block keeps its view while unchanged, so a streamed chunk only re-parses and re-lays-out the last block instead of the whole reply.
- WASI plugin pipes: the three `RwLock::write().unwrap()` calls in `lapce-proxy/src/plugin/wasi.rs` go through `lock_pipe`, which recovers (and clears) a poisoned lock with a warning instead of panicking every plugin thread. The other shared state already uses `parking_lot`, whose locks do not poison.
- Removed the crash-prone `unwrap()` calls from the files listed in OPTIMIZE.md:
  - `lapce-proxy/src/plugin/psp.rs`: a plugin message without `params`, `id` or `method` (valid JSON-RPC, e.g. `workspace/workspaceFolders`) no longer panics the proxy; malformed messages are logged and dropped, and a request the host drops is answered with an error. The document filter checks use `match` / `is_none_or` instead of guarded unwraps.
  - `lapce-proxy/src/plugin/mod.rs`: the 21 `Url::from_file_path(path).unwrap()` calls go through `file_url`, which logs and drops the request for a non-absolute path; a broken zstd plugin archive returns an error with context.
  - `lapce-app/src/config.rs`: `export_theme` returns a `Result` with context (the command logs failures), `load_icon_theme` no longer unwraps the theme directory, and the invariants on the embedded default themes use `expect` with a message.
  - `lapce-app/src/keypress/loader.rs` and `lapce-proxy/src/cli.rs` had no production unwraps left.
- Replaced the fixed sleeps used for synchronization:
  - New `lapce_core::debounce::spawn_debouncer`: a thread that waits on a channel and handles each burst of events once it has been quiet for a while (or is too old), without losing events between bursts.
  - The config file watcher (`config/watcher.rs`) uses it instead of an `AtomicBool` plus a 500 ms sleep, whose window could drop a change made right after a reload was triggered.
  - The workspace file watcher of the proxy (`dispatch.rs`) uses it instead of a sleeping thread per burst; it no longer panics when there is no workspace.
  - The agent session `delay` (`agent/session.rs`) waits on a cancel channel instead of sleeping, so its helper thread ends as soon as the delay is dropped.
  - Kept on purpose: the 500 ms retry when Windows has not released a removed plugin folder yet (`plugin/mod.rs`) and the `try_wait` polling in the `agent/process.rs` tests, which have no event to wait on.
- Agent prompts go through a single `AgentData::try_send` entry point, which owns the "ignore while a turn is running" rule. `send_input` reads the input box, and clears it only when `try_send` accepted the prompt.
- Rerun now sends the prompt again with the context that was attached to it the first time (`AgentItem::User` stores its `contexts`), instead of the file that happens to be active when Rerun is clicked.
- Agent panel buttons (Copy, Edit, Rerun, Restart, Cancel, the permission options, Reject and Send) are now keyboard reachable: the shared `pressable` wrapper makes them Tab-focusable, Enter or Space press them, and the focused button shows the focus color on its border. The activation key is stopped at the button so the panel keymap does not also send the prompt. (Not verified by hand in the running app.)
- Removed the `too_many_arguments` suppressions of the files listed in OPTIMIZE.md: `Find::find` takes a `FindOptions`, `terminal_view` takes the `&TerminalData`, the settings `dropdown_scroll` takes a `DropdownSignals`, and the plugin catalog receives a `ServerRequest` struct (also the payload of `PluginCatalogRpc::ServerRequest`). `WindowTabData::new` and `handle_server_notification` were already under the limit. The suppressions left in `plugin/mod.rs`, `plugin/lsp.rs` and `plugin/psp.rs` were not part of the item.
- Fixed a race in `AgentManager::start` (found through a flaky `restarting_does_not_report_the_replaced_session_as_disconnected` test): the replaced session was killed before the generation was bumped, so its final `Disconnected` could still be reported during a restart. The generation is now bumped first.
- Removed clones from the per-frame and per-line paths: `paint_current_line` no longer clones the whole `EditorData` and the debug break line path on every frame, `paint_sticky_headers` borrows the sticky lines instead of cloning them (twice, one copy being thrown away), `paint_find` paints the visible matches without collecting them into a `Vec`, and `phantom_text` (run for every line laid out) clones the completion lens and inline completion texts only for the line that shows them, once instead of twice. `main_split.rs` has no per-frame closures, so it was left alone.
- Triaged the TODO/FIXME markers into `docs/todo-triage.md` (possible bugs, missing features, performance, cleanup), ready to be filed as issues. Stale or empty markers were removed or rewritten, and the sticky header height question was answered in code.
- Stopped discarding `Result`s with `let _ =`: `update::restart` returns the `exec` error with context (it used to report success when the restart failed; the caller already logs it), `error_notification` logs a failed `notify-send` at debug level, and a cancel sent after the agent session ended is logged at debug level. The remaining `let _ =` sites outside tests do not hold a `Result` (cursor moves, signal tracking, map inserts, a byte count).
- The `@file` mention list is now built on the proxy response thread instead of the UI thread, and capped at `MAX_MENTION_FILES` (50 000, the shallowest files win), so huge workspaces no longer freeze the UI or slow down filtering on every keystroke. A file left out by the cap can't be attached as a mention.
- OPTIMIZE.md "Toast has no cancellation": already handled by the `toast_seq` generation counter of `AgentData::show_toast` (an old timer only hides its own toast); marked as done without code changes.
- The agent panel layout values (paddings, gaps, radii, toast position, caret scroll margin) are now named constants next to the existing `INPUT_*` ones; the rendered layout is unchanged.
- The agent communication log `lapce-agent.log` is now written to the Lapce logs directory instead of the current working directory, so the working tree stays clean. Set `LAPCE_AGENT_LOG_DIR` to write it elsewhere (`LAPCE_AGENT_LOG_DIR=.` restores the old location).
- Agent UI tests: the logic of `try_send` is split into pure, tested pieces: `plan_prompt` (ignore while busy, send, or start the agent first), `send_prompt` (records the prompt and sends it, tested against a real `ProxyRpcHandler` channel) and `agent_mention::mention_contexts` (which `@mentions` become contexts; it also no longer clones the whole workspace file list on every send). Rerun is covered through `AgentState::user_prompt`. `edit_in_input` only edits the input editor and is still untested: it needs an editor with a `MainSplitData`, which tests cannot build cheaply.
- The chat box of the agent panel now has Model and Effort buttons. They appear when the agent advertises ACP session config options with the `model` or `thought_level` category, show the current value, and open a menu to pick another one (`session/set_config_option`). The choice is applied between turns; the panel updates from the agent's answer or `config_option_update`. New `AgentEvent::ConfigOptions` and `ProxyNotification::AgentSetConfig`; the mock agent offers a model setting for the integration test.
- The Model and Effort buttons now open a filter list above the prompt box instead of a menu: type to filter (every word must match the name or value, any case), Up/Down move the highlight, Enter picks, Esc closes, and a click picks too. The list shows 10 entries at a time and scrolls. The button label no longer repeats the setting name when the value already starts with it ("Thinking: high" instead of "Thinking: Thinking: high").
- A turn that ends with `EndTurn` and no answer (no assistant text and no tool call since the last prompt) now adds an error to the transcript telling the user to check the agent credentials and model. Some agents, such as `pi-acp` with an expired login, end the turn silently on a model error, which used to leave the panel looking frozen.
- The agent panel now shows feedback during and after a turn: an animated "Working..." row under the transcript while a turn runs (driven by a generation-guarded timer loop that stops when the turn ends), and a "Task completed" note when a turn ends normally after the agent answered. (Not verified by hand in the running app.)
- While a turn runs, the prompt being answered shows a single "Stop" button (cancels the turn) instead of "Edit" and "Rerun"; the two come back when the turn ends. New `AgentState::last_user_index`. (Not verified by hand in the running app.)
- New "New Chat" button at the right end of the Agent panel header (`PanelBuilder::add_with_action`): it clears the transcript and, if an agent is running, replaces its session with a fresh one, so the agent keeps no memory or context of the old chat. The input draft is kept. New `AgentState::clear_chat` and `AgentData::new_chat`. (Not verified by hand in the running app.)
- Removed the "Cancel" button from the agent toolbar: the running prompt has its own "Stop" button.
- New "Clear" button next to "New Chat" in the Agent panel header: it empties the transcript but keeps the agent session, so the agent still remembers the chat. It is refused with a toast while a turn runs.
- Review of the agent changes. The proxy forwards the ACP diff blocks of completed edit tools as `AgentEvent::FileChange` (pi writes files itself and never uses `fs/write_text_file`, so the diff blocks are the only source). The new `agent_changes` module keeps, per file, the text before the first change and the latest text; a file back to its original leaves the list. The panel shows a "Changed files" box above the prompt with `+added -removed` line counts (skipped for very large diffs). Clicking a file opens a diff tab (original on the left, the file on the right, `MainSplitData::open_text_diff`). "Approve changes" empties the list; "Revert changes" writes the original text back (through the open buffer when the file is open) and moves files the agent created to the trash. Both are refused while a turn runs; New Chat also empties the list. Changes made by shell commands of the agent have no diff block and are not tracked. (Not verified by hand in the running app.)
- The changed files box of the agent panel is now easier to see: a 2px accent border and a bigger bold title, rows with the file name in bold, its folder dimmed and the line counts in green (+) and red (-), a filled green "Approve changes" button and a red outlined "Revert changes" button. (Not verified by hand in the running app.)
- Agent panel transcript styling: Stop, Edit, Rerun and Copy sit at the right of the box with more space above them; "Thinking" entries are boxed on a yellow tint and "Tool" entries on a cyan tint (translucent theme colors, so both themes work); the answers of the agent get a framed box on the editor background with an accent border, so they stand out. Also ran `cargo fmt` with the project config, which fixed the import order of the files edited with a plain rustfmt before. (Not verified by hand in the running app.)
- The Markdown answers of the agent can now be selected with the mouse and copied with Ctrl+C (Cmd+C on macOS). Floem's `rich_text` has no selection, so the new `panel::selectable_text` view draws the selection on a canvas under the text and handles the pointer and copy events; the text is still laid out by `rich_text`. Only the Markdown text blocks are selectable (not code images or separators), and a selection stays inside one block. (Not verified by hand in the running app.)
- The agent transcript scrolls down by itself as content is added (streamed text, tool calls), unless the user scrolled up; scrolling back to the end, or sending a new prompt, makes it follow again. The rule is the pure `follow_after_scroll`. (Not verified by hand in the running app.)
- The boxes of the agent transcript have a small left and right margin, so they no longer touch the sides of the panel.
- The title of each transcript box ("Agent", "Thinking"...) has 6px of space below it.
- When a turn of the agent ends while the Lapce window has no focus, a desktop notification is shown ("Agent finished" with "Task completed" or a note that the agent ended without any response; "Agent stopped" with the stop reason for other stops; nothing when the user cancelled). New `desktop_notification` module (runs `notify-send`, unix only, a missing `notify-send` is only logged) and a `window_focused` signal in `WindowCommonData`, set from the window got and lost focus events. (Not verified by hand in the running app.)
- Two new agent settings, both on by default: `agent.show-thinking` and `agent.show-tools` (checkboxes in the Settings, "Agent" section). When off, the "Thinking" or "Tool" boxes are hidden in the chat; the entries are still recorded, so turning a setting back on shows them again.
- Selection in the agent answers: releasing the mouse button now keeps a finished selection even if the release event reaches the view twice (`after_pointer_up`), and the press, release and state changes are logged at debug level (`lapce_app::panel::selectable_text`) to find why a selection was lost on release. Not confirmed as the cause yet.

## Publish script

- Added `scripts/publish.sh <version>`: runs only on a clean `master`, bumps the version (`Cargo.toml`, `Cargo.lock`, macOS `Info.plist`, wix `goldenide.wxs`, `goldenide.spec`), commits, pushes `master`, merges it into `publish`, pushes `publish` and returns to `master`.
- CI now runs on `master` and `publish`. Executables are built and uploaded only on `publish`; other branches just run `cargo check`.
