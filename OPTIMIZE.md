# Optimization TODO

> Generated on 2026-10-09. Items sorted by importance. Based on a static scan (file sizes, grep for recurring patterns, a read of the agent panel code); nothing was profiled, so performance items are hypotheses to measure first.

## Critical

- [x] **Split the god files** — the app/UI layer has several 3000-4300 line files that mix view building, state and command handling, which makes every change risky and slows incremental builds. Split by responsibility (e.g. `app.rs` into window/menu/layout/launch modules, `editor.rs` into data/commands/gutter-like parts, `main_split.rs` into split tree and editor-tab operations).
  - File(s): `lapce-app/src/app.rs` (4344), `lapce-app/src/editor.rs` (3926), `lapce-app/src/main_split.rs` (3105), `lapce-app/src/window_tab.rs` (3064), `lapce-app/src/editor/view.rs` (2731), `lapce-app/src/doc.rs` (2240)

## High

- [x] **Re-parse of the whole Markdown reply on every streamed chunk** — `markdown_view` calls `parse_markdown` inside the `dyn_stack` source closure and the item key is a hash of the full text, so each chunk of a streaming reply re-parses and re-lays-out the entire reply (cost grows quadratically with reply length). Parse once per `render_key` change in a memo, or only re-render the trailing block while the turn is busy.
  - File(s): `lapce-app/src/panel/agent_view.rs`, `lapce-app/src/agent.rs` (`render_key`)

- [x] **Whole transcript cloned on every state change** — the transcript `dyn_stack` source does `state.items.iter().cloned().enumerate().collect()` each time the state signal fires (including every streamed chunk), cloning every message string. Keep per-item signals, or `Rc<str>`/`Arc<str>` text in `AgentItem`, so unchanged items are not copied.
  - File(s): `lapce-app/src/panel/agent_view.rs`, `lapce-app/src/agent.rs`

- [x] **Single wide `state` signal drives the whole agent panel** — toolbar status, permission bar and transcript all depend on one `AgentState` signal, so any change (e.g. status text) re-evaluates every consumer. Split into focused memos (`status`, `busy`, `current_permission`, `items`).
  - File(s): `lapce-app/src/agent.rs`, `lapce-app/src/panel/agent_view.rs`

- [x] **`.lock().unwrap()` / `.read().unwrap()` on shared state** — a panic in one thread poisons the lock and cascades into panics in every other thread that touches it, including the proxy. Handle poisoning explicitly (`parking_lot`, or `unwrap_or_else(PoisonError::into_inner)`) and fail with context.
  - File(s): `lapce-app/src`, `lapce-proxy/src` (3 sites found by grep: locate with `rg "\.(lock|read|write)\(\)\.unwrap"`)

- [x] **Production `unwrap()` calls** — about 400 `unwrap()` and 19 `expect()` in the crates; many are in tests, but the rest can crash the editor on bad config, keymap or plugin data. Audit non-test code first, starting from the config and keymap loaders, and return errors with context.
  - File(s): `lapce-app/src/config.rs`, `lapce-app/src/keypress/loader.rs`, `lapce-proxy/src/plugin/mod.rs`, `lapce-proxy/src/plugin/psp.rs`, `lapce-proxy/src/cli.rs`

## Medium

- [x] **Fixed `thread::sleep` for synchronization** — sleeps of 20 ms, 500 ms and similar are used for polling or retry in the proxy; they add latency and hide races. Replace with channel receives with timeout, condition variables or event-driven wakeups.
  - File(s): `lapce-proxy/src/agent/process.rs` (line ~130), `lapce-proxy/src/agent/session.rs` (line ~189), `lapce-proxy/src/plugin/mod.rs` (line ~1646), `lapce-app/src/config/watcher.rs` (line ~28)

- [x] **Rerun and send duplicate prompt bookkeeping** — `send_input`, `rerun` and `send_text` in the agent data share the busy check but each repeats it; a single `try_send(text)` entry point would keep the "ignore while busy" rule in one place. Also decide whether Rerun should reuse the original file context instead of recomputing the active one.
  - File(s): `lapce-app/src/agent.rs`

- [x] **Agent buttons are not keyboard reachable** — `button()` is a plain label with an `on_click`, so Copy/Edit/Rerun/Restart/Cancel have no focus or key activation. Add a focusable button view shared by the panel.
  - File(s): `lapce-app/src/panel/agent_view.rs`

- [x] **`too_many_arguments` suppressed in several constructors** — the `#[allow(clippy::too_many_arguments)]` markers (window_tab, catalog, find, settings, terminal view) point at missing parameter structs; introduce context/builder structs instead of silencing the lint.
  - File(s): `lapce-app/src/window_tab.rs`, `lapce-proxy/src/plugin/catalog.rs`, `lapce-app/src/find.rs`, `lapce-app/src/settings.rs`, `lapce-app/src/terminal/view.rs`

- [x] **Heavy `.clone()` use** — about 1500 `.clone()` calls across the crates, many on `String`, `PathBuf` and `Vec` in UI closures. Review hot closures (per-frame style and view closures) and prefer `Rc`/`Arc` or borrowed data there.
  - File(s): `lapce-app/src/editor/view.rs`, `lapce-app/src/doc.rs`, `lapce-app/src/main_split.rs`

- [x] **Unresolved TODO/FIXME backlog** — 124 TODO/FIXME markers, concentrated in the editor core. Triage them into issues or delete the stale ones so the remaining markers mean something.
  - File(s): `lapce-app/src/doc.rs` (16), `lapce-app/src/editor/view.rs` (14), `lapce-app/src/editor.rs` (13), `lapce-core/src/syntax/mod.rs` (9), `lapce-app/src/markdown.rs` (9)

- [x] **Silently ignored results** — about 20 `let _ =` sites discard `Result` values (channel sends, file writes). Log failures at least at `debug` level so lost events are traceable.
  - File(s): `rg "let _ ="` over `lapce-app/src` and `lapce-proxy/src`

## Low / Nice to have

- [x] **Mention file list loaded eagerly and unbounded** — `load_files` fetches every workspace file path and sorts them in memory on load; for large repos use a cap or incremental fuzzy search in the proxy.
  - File(s): `lapce-app/src/agent.rs`, `lapce-app/src/agent_mention.rs`

- [x] **Toast has no cancellation** — `show_toast` schedules a clear after a fixed delay; two quick copies let the first timer hide the second toast early. Track a generation counter and clear only if it still matches.
  - File(s): `lapce-app/src/agent.rs`

- [ ] **Magic numbers in the agent panel layout** — values like `margin_top(4.0)`, `padding_vert(6.0)`, `inset_bottom(INPUT_HEIGHT + 30.0)` are inline; extract named constants like the existing `INPUT_*` ones.
  - File(s): `lapce-app/src/panel/agent_view.rs`

- [ ] **Local `lapce-agent.log` in the repo root** — it is gitignored, but the log path is hard-wired by name; make the directory configurable or place it in the data dir so the working tree stays clean.
  - File(s): `lapce-app/src/app/logging.rs`

- [ ] **Agent UI has no automated tests** — `AgentState` has unit tests, but `edit_in_input`, `rerun` and `send_text` (which touch the editor and the proxy) have none; extract the pure parts or inject a proxy trait so they can be tested.
  - File(s): `lapce-app/src/agent.rs`
