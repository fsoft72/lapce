# TODO / FIXME triage

Triage of the `TODO` and `FIXME` markers in `lapce-app`, `lapce-core`, `lapce-proxy` and `lapce-rpc` (done on 2026-10-09, 113 markers left). Each group can be turned into tracker issues; the markers stay in the code next to what they describe. Markers are listed by file and summary, not by line number, so the list survives edits.

## Removed or rewritten

- `doc.rs` `clear_text_cache`: a `TODO: ???` over commented-out code; both removed.
- `doc/document.rs`: two `TODO: italics?` questions without an action.
- `editor/view/sticky_header.rs`: the question "won't y_diff always be scroll_offset here?" was answered (yes) and the height is now computed that way.
- `editor/commands.rs`, `main_split/diagnostics.rs`: bare `TODO` markers now say what is missing.
- `plugin/dap.rs`: the unfinished `TODO: We set this because` now explains why `CREATE_NO_WINDOW` is set.
- `syntax/mod.rs`: "can't inline this since matches borrows self.layers" was an explanation, not a task; it is now a plain comment.

## Possible bugs (verify first)

- Line wrapping and affinity: `editor/view.rs` (current line highlight, scope lines, bracket highlights, find region affinity), `window_tab/popup_origins.rs` (4 affinity questions), `completion.rs` (adjust for visual lines).
- Screen lines: `editor/screen_lines.rs` (range may iterate one line too few; vline counts in diff views).
- Syntax: `syntax/mod.rs` (`prev_sibling` can get stuck in Markdown; cursor range; no-op edit at range end; highlight errors flattened away).
- Documents: `doc/document.rs` (`has_multiline_phantom` always returns `true`), `editor/diff.rs` (left/right editors not cleaned up).
- Proxy: `plugin/catalog.rs` (did-open registration not checked), `plugin/lsp.rs` (URI scheme of document filters ignored), `plugin/wasi.rs` (empty theme lists set by default, 2 `FIXME`), `watcher.rs` (watched paths not tracked), `completion.rs` (proxy path format may differ).
- `inline_completion.rs`: two "is this correct?" checks copied from completion lens.
- `lapce-rpc/src/file.rs`: possible off-by-one (`i >= min` or `i + 1 >= min`).
- `update.rs`: the Windows updater replaces the exe instead of running the installer, and may leave a backup without a working exe.
- `window_tab.rs`: editor commands are dropped when no editor is active instead of going to the focused view.

## Missing features

- Editor: scroll commands to center/top/bottom of the window (`editor/commands.rs`), workspace edit file operations (`main_split/diagnostics.rs`), completion lens details and lens/inline completion overlap (`doc/document.rs`), `is_incomplete` completion lists and continuing the active item (`completion.rs`).
- Markdown (`markdown.rs`): inline HTML, math, strikethrough, links, more code block language names, link types for images.
- Settings and config: `WrapColumn` wrap style and readable option names (`config.rs`), theme base from the declared color preference (`config.rs`), error lens variants (`config/editor.rs`), blocking keys in inputs (`settings.rs`, `FIXME`).
- File explorer: remote files, Shift+right click "delete" instead of "move to trash", file icon from the name being typed (`file_explorer/`).
- Palette: next/previous page (`palette.rs`).
- Others: logging panel and runtime log level (`window_tab/internal_commands.rs`), variables in terminal arguments (`terminal/data.rs`), remote proxy version detection (`proxy/remote.rs`), signature information for LSP (`plugin/mod.rs`), DAP id and start method types (`lapce-rpc/src/dap_types.rs`), more trivial edits handled incrementally (`lapce-proxy/src/buffer.rs`, `plugin/psp.rs`).

## Performance

- Document: more granular invalidation of styles and caches, simple deletions and smarter inserts for inlay hints and completion lens, shared `Rc`s for the editor styling, fewer allocations in `apply_deltas` (`doc.rs`), cached styling data (`doc/styling.rs`).
- Views: vline computation in `editor/view.rs` and `panel/source_control_view.rs`, change colors as iterators with a limit (`editor/view/changes.rs`), screen line dependencies (`editor/screen_lines.rs`).
- Syntax: skip layers outside the range, pre-process local scopes (`syntax/mod.rs`).
- Others: huge pasted text in the palette and full config reload (`palette.rs`), glob recreated for each directory read (`file_explorer/data.rs`), `take` for signals (`listener.rs`), `TrustedLen` (`encoding.rs`).

## Cleanup

- `editor.rs`: `DocSignal` could be generic or a `ReadSignal` once it implements `track`.
- `editor/commands.rs` (sub-enums), `editor/screen_lines.rs` (placement, `get` vs `get_untracked`), `editor/view/sticky_header.rs` (`get` vs `get_untracked`), `editor/view.rs` (shared selection rect logic, paint recomputation comment).
- `syntax/mod.rs` (reuse `layer`), `text_input.rs` (avoid boxing), `plugin/lsp.rs` (clean up), `plugin/psp.rs` (drop the deprecated `startLspServer` notification), `config.rs` (terminal default profile hack), `file_explorer/data.rs` (pass `is_dir`), `doc/styling.rs` (wrapped line background), `markdown.rs` (build `MarkdownText` directly), `palette.rs` (filter text), `inline_completion.rs` (merge with completion lens, cancel completion), `encoding.rs` (more Unicode tests).
