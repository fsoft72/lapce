# Layout-aware symbol keybinding matching

## Problem

Lapce's keymap matcher resolves character keybindings using
`key_without_modifiers` (the physical key's unshifted base character) plus a
separately tracked Shift modifier bit, instead of the key's actual produced
character (`logical`, which already accounts for Shift/AltGr/layout).

This works for US QWERTY, where most punctuation keys are unshifted (`/`) and
shifted symbols sit on digit keys in a US-specific arrangement (`shift+4` =
`$`). It breaks on other layouts. On Italian keyboards, for example, `/` is
produced by Shift+7 and `^` is not on a digit key at all. Any binding that
relies on `key_without_modifiers` + an explicit `shift` bit for a symbol only
matches on layouts that happen to share the US arrangement.

Confirmed affected today: the vim-mode `/` (search), `^` (line start
non-blank), `$` (line end), `%` (match pairs), `*` (search whole word
forward), `<`/`>` (indent/outdent), `:` (command palette), and non-vim
shortcuts such as `ctrl+/` (toggle line comment).

## Goals

- Symbol/digit keybindings match based on the character actually produced by
  the user's keyboard, regardless of layout.
- No regression for existing behavior on US layouts.
- Existing user-authored `keymaps.toml` overrides using the old `shift+<digit>`
  convention keep working (backward compatibility).
- Fix applies globally (all commands, all modes) since the matching engine is
  shared app-wide, and the same root cause affects non-vim shortcuts.

## Non-goals

- Changing behavior for alphabetic keys (`a`-`z` matching via
  `key_without_modifiers` + explicit `shift` bit for uppercase) is unaffected
  and out of scope - that convention is already layout-portable for
  Latin-alphabet layouts.
- Multi-key chord sequences (e.g. `g g`) are not part of this fix; no
  shipped keymap uses a symbol/digit key inside a multi-key chord today.
- Dead-key/compose-sequence input is out of scope beyond "whatever character
  the toolkit reports as `logical`" - if the OS/toolkit correctly composes a
  dead-key sequence into a final character, it is handled like any other
  character; composing intermediate dead-key state is not something this
  change introduces or fixes.

## Design

### Matching engine change

File: `lapce-app/src/keypress/key.rs`, `KeyInput::keymap_key()`.

Today, for `Key::Character(c)` where `c.len() == 1 && c.is_ascii()`, the
function always returns `KeyMapKey::Logical(Character(key_without_modifiers))`
(lowercased), relying on the separately-tracked Shift bit in `KeyMapPress.mods`
for disambiguation.

New behavior: split this case by whether the base character is alphabetic:

- **Alphabetic** (`a`-`z` after lowercasing): unchanged. Continue using
  `key_without_modifiers` lowercased, with Shift tracked as a real modifier
  bit. This preserves the existing, already-portable `shift+<letter>`
  convention.
- **Non-alphabetic** (digits, punctuation): compute a *second* candidate
  representation from `logical` (the actual produced character) instead of
  `key_without_modifiers`, with Shift and AltGr cleared from the modifier set
  used for that candidate (they're absorbed into the character, exactly as
  the existing `handle_keymatch` typed-character fallback at
  `keypress.rs:419-428` already does for a different purpose). Ctrl and Meta
  remain real modifiers on this candidate, so `ctrl+/`-style shortcuts keep
  working as modifier+character combinations.

  The legacy representation (`key_without_modifiers` + full modifier set,
  today's behavior) is still computed and kept as a fallback candidate.

  Note: for a plain digit/punctuation key pressed with no Shift/AltGr (e.g.
  `4`, `0` used for vim count prefixes), `logical` and `key_without_modifiers`
  are identical and no Shift/AltGr bit needs absorbing, so the new and legacy
  candidates coincide - no behavior change for unshifted digit/punctuation
  keys.

### Lookup order (backward compatibility)

File: `lapce-app/src/keypress.rs`, `KeyPressData::match_keymap()`.

For a single-keypress lookup (`keypresses.len() == 1`) where the keypress is a
non-alphabetic character key with Shift or AltGr involved, the matcher now has
two candidate `KeyMapPress` values instead of one:

1. **New candidate** (`logical`-based, Shift/AltGr absorbed) - tried first.
2. **Legacy candidate** (`key_without_modifiers`-based, today's behavior) -
   tried if (1) finds no match.

This order means:
- Shipped defaults, migrated to literal-character syntax (see below), match
  immediately via candidate 1 on every layout.
- A user's pre-existing custom `keymaps.toml` entry using the old
  `shift+<digit>` syntax still matches via candidate 2, unchanged from today,
  as long as their layout matches the assumption baked into that entry (same
  as today - no regression, no new guarantee for old-style entries on other
  layouts).
- If both a new-style and an old-style binding happen to match the same
  physical event, the new-style (more specific, layout-correct) binding wins.

Multi-keypress sequences (chords) are unaffected: only the `len() == 1` case
gains the second candidate, since no shipped chord uses a symbol/digit key.

### Default keymap migration

File: `defaults/keymaps-common.toml`. Replace `shift+<digit/punct>` entries
with the literal character they represent:

| Current `key` | New `key` | Command(s) |
|---|---|---|
| `shift+6` | `^` | `line_start_non_blank` |
| `shift+4` | `$` | `line_end` |
| `shift+8` | `*` | `search_whole_word_forward` |
| `shift+5` | `%` | `match_pairs` |
| `shift+.` (2 occurrences) | `>` | `motion_mode_indent`, `indent_line` |
| `shift+,` (2 occurrences) | `<` | `motion_mode_outdent`, `outdent_line` |
| `shift+;` | `:` | `palette.command` |

`/` (already bound to `search` in vim Normal mode from the prior change)
requires no further edit - it already uses literal-character syntax and will
now match via the new candidate on every layout.

`ctrl+/` (`keymaps-nonmacos.toml`) and `meta+/` (`keymaps-macos.toml`) also
require no edit: they already use literal-character syntax; the engine change
alone makes them layout-correct.

### Testing

- Unit tests in `lapce-app/src/keypress/` (new or extended `#[cfg(test)]`
  module) simulating keyboard events where `key_without_modifiers` and
  `logical` diverge, e.g. `key_without_modifiers = "7"`, `logical = "/"`,
  `mods = SHIFT` matching a `key = "/"` binding.
- Regression test: a legacy-style custom binding (`shift+4`) still matches an
  event shaped like today's US-layout event (`key_without_modifiers = "4"`,
  `logical = "$"`, `mods = SHIFT`).
- Regression test: `ctrl+/` still requires Ctrl - an event with `logical =
  "/"`, `mods = SHIFT` only (no Ctrl) must not match a `ctrl+/` binding.
- Manual verification (rebuild + restart Lapce) on the Italian keyboard
  layout: `/`, `^`, `$`, `%`, `*`, `<`, `>`, `:` in vim Normal mode, and
  `ctrl+/` for toggle line comment.

## Files touched

- `lapce-app/src/keypress/key.rs` - new candidate representation for
  non-alphabetic character keys.
- `lapce-app/src/keypress.rs` - two-candidate lookup order in
  `match_keymap`.
- `defaults/keymaps-common.toml` - migrate symbol bindings to literal-character
  syntax (table above).
- `CHANGELOG.md` - note the fix under Unreleased.
