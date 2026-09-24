# Changes

## Editor font zoom

- Added `editor_font_increase`, `editor_font_decrease` and `editor_font_reset` commands. They change `editor.font-size` (clamped to 6-32) and persist it in `settings.toml`.
- Added `Ctrl+=`, `Ctrl++` and `Ctrl+-` keymaps (`Cmd` on macOS), active only with `editor_focus`. The window-wide `zoom_in` / `zoom_out` keep working elsewhere.
- Added `Ctrl+mouse wheel` (`Cmd` on macOS) on the editor content and gutter to zoom the font instead of scrolling.
