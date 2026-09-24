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
