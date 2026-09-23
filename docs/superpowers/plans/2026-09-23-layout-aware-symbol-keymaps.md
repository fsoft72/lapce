# Layout-aware Symbol Keymap Matching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make symbol/digit keybindings (`/`, `^`, `$`, `%`, `*`, `<`, `>`, `:`, `ctrl+/`, etc.) match the character actually produced by the user's keyboard layout, instead of assuming a US QWERTY arrangement, while keeping old-style custom `keymaps.toml` entries (`shift+4` for `$`, etc.) working.

**Architecture:** `lapce-app/src/keypress/key.rs` gains a second, opt-in key representation for non-alphabetic character keys, built from the key's actual produced character (`logical`) with Shift/AltGr absorbed into it instead of tracked as modifier bits. `lapce-app/src/keypress.rs`'s matcher tries this new representation first and falls back to the pre-existing `key_without_modifiers`-based representation (unchanged) only when the new one finds no match. `defaults/keymaps-common.toml` is migrated to use literal symbol characters instead of the old `shift+<digit>` convention.

**Tech Stack:** Rust (edition 2024), `floem`/`winit` keyboard types, `indexmap`, `cargo test` / `cargo build -p lapce-app`.

**Spec:** `docs/superpowers/specs/2026-09-23-layout-aware-symbol-keymaps-design.md`

## Global Constraints

- Alphabetic key matching (`a`-`z`, `shift+<letter>`) must not change behavior - out of scope per spec.
- Multi-key chord sequences are not part of this fix - only single-keypress lookups get the new fallback behavior.
- Old-style `shift+<digit>` custom bindings must keep matching on layouts shaped like today's (backward compatibility).
- No regression to `ctrl+/`-style shortcuts: Ctrl/Meta must remain real modifiers, never absorbed into the character.

---

### Task 1: Layout-aware character representation for symbol/digit keys

**Files:**
- Modify: `lapce-app/src/keypress/key.rs`
- Modify: `lapce-app/src/keypress/press.rs`
- Test: inline `#[cfg(test)]` module in `lapce-app/src/keypress/press.rs`

**Interfaces:**
- Consumes: existing `KeyInput::keymap_key(&self) -> Option<KeyMapKey>` (unchanged, becomes the "legacy" representation), existing `KeyMapKey` enum (`lapce-app/src/keypress/keymap.rs`).
- Produces:
  - `KeyInput::logical_symbol_key(&self) -> Option<KeyMapKey>` - new method, `None` unless the key is a non-alphabetic single-ASCII-character key whose `logical` value is also a single ASCII character.
  - `KeyPress::keymap_press(&self) -> Option<KeyMapPress>` - behavior changed: now the "primary" representation, using `logical_symbol_key` when available, falling back to `legacy_keymap_press`.
  - `KeyPress::legacy_keymap_press(&self) -> Option<KeyMapPress>` - new method, exactly what `keymap_press` used to do (unchanged logic, just renamed/extracted).
  - Task 2 consumes both `keymap_press` and `legacy_keymap_press`.

- [ ] **Step 1: Add `logical_symbol_key` to `KeyInput` with its own unit tests are deferred to Step 5 (co-located with `KeyPress` tests, since `KeyPress` is what callers use) - write the method now**

Edit `lapce-app/src/keypress/key.rs`. The file currently ends with:

```rust
impl KeyInput {
    pub fn keymap_key(&self) -> Option<KeyMapKey> {
        // ... existing body, do not change ...
    }
}
```

Add a new method inside the same `impl KeyInput` block, after `keymap_key`:

```rust
    /// For a non-alphabetic character key (a digit or punctuation symbol),
    /// returns a `KeyMapKey` built from the character actually produced by
    /// the user's keyboard layout (`logical`), rather than the unshifted
    /// base character (`key_without_modifiers`) that `keymap_key` uses.
    ///
    /// This makes bindings like `/` or `^` match on any layout: on a layout
    /// where producing that character requires Shift (e.g. `/` via Shift+7
    /// on an Italian keyboard), `logical` already reflects the real
    /// character while `key_without_modifiers` would not.
    ///
    /// Returns `None` for anything this doesn't apply to (named keys,
    /// alphabetic keys, numpad keys, or a `logical` value that isn't a
    /// single ASCII character) - callers should fall back to `keymap_key`
    /// in that case.
    pub fn logical_symbol_key(&self) -> Option<KeyMapKey> {
        let KeyInput::Keyboard {
            key_without_modifiers,
            logical,
            location,
            ..
        } = self
        else {
            return None;
        };

        if matches!(location, KeyLocation::Numpad) {
            return None;
        }

        let Key::Character(base) = key_without_modifiers else {
            return None;
        };
        if !(base.len() == 1 && base.is_ascii()) {
            return None;
        }
        if base.chars().next().unwrap().is_ascii_alphabetic() {
            return None;
        }

        let Key::Character(actual) = logical else {
            return None;
        };
        if !(actual.len() == 1 && actual.is_ascii()) {
            return None;
        }

        Some(KeyMapKey::Logical(Key::Character(actual.to_lowercase().into())))
    }
```

- [ ] **Step 2: Build to check for typos**

Run: `cargo check -p lapce-app`
Expected: compiles with 0 errors (the new method is unused so far - that's fine, it's `pub`).

- [ ] **Step 3: Split `KeyPress::keymap_press` into primary + legacy**

Edit `lapce-app/src/keypress/press.rs`. Current full file:

```rust
use floem::keyboard::Modifiers;

use super::{key::KeyInput, keymap::KeyMapPress};

#[derive(Clone, Debug)]
pub struct KeyPress {
    pub(super) key: KeyInput,
    pub(super) mods: Modifiers,
}

impl KeyPress {
    pub fn keymap_press(&self) -> Option<KeyMapPress> {
        self.key.keymap_key().map(|key| KeyMapPress {
            key,
            mods: self.mods,
        })
    }
}
```

Replace the `impl KeyPress` block with:

```rust
impl KeyPress {
    /// The preferred key representation for matching against keymaps.
    ///
    /// For non-alphabetic character keys (digits, punctuation) this is
    /// built from the character actually produced by the user's keyboard
    /// layout, with Shift/AltGr absorbed into the character rather than
    /// tracked as modifiers - so e.g. `/` matches on any layout, not just
    /// ones where `/` happens to be unshifted. Everything else (letters,
    /// named keys) is unchanged from `legacy_keymap_press`.
    pub fn keymap_press(&self) -> Option<KeyMapPress> {
        if let Some(key) = self.key.logical_symbol_key() {
            let mut mods = self.mods;
            mods.set(Modifiers::SHIFT, false);
            mods.set(Modifiers::ALTGR, false);
            return Some(KeyMapPress { key, mods });
        }

        self.legacy_keymap_press()
    }

    /// The pre-existing key representation: the key's unshifted base
    /// character (or named key) plus the full modifier set. Kept as a
    /// fallback so custom `keymaps.toml` entries written with the old
    /// `shift+<digit>` convention for symbols keep matching.
    pub fn legacy_keymap_press(&self) -> Option<KeyMapPress> {
        self.key.keymap_key().map(|key| KeyMapPress {
            key,
            mods: self.mods,
        })
    }
}
```

- [ ] **Step 4: Build to check it compiles**

Run: `cargo check -p lapce-app`
Expected: compiles with 0 errors. (`legacy_keymap_press` is used by `keymap_press` itself, so no dead-code warning.)

- [ ] **Step 5: Write failing tests for both representations**

Append to `lapce-app/src/keypress/press.rs` (end of file, new module):

```rust
#[cfg(test)]
mod tests {
    use floem::keyboard::{Key, KeyCode, KeyLocation, PhysicalKey};

    use super::*;
    use crate::keypress::keymap::KeyMapKey;

    fn char_key(
        key_without_modifiers: &str,
        logical: &str,
        mods: Modifiers,
    ) -> KeyPress {
        KeyPress {
            key: KeyInput::Keyboard {
                physical: PhysicalKey::Code(KeyCode::Digit7),
                logical: Key::Character(logical.into()),
                location: KeyLocation::Standard,
                key_without_modifiers: Key::Character(key_without_modifiers.into()),
                repeat: false,
            },
            mods,
        }
    }

    #[test]
    fn slash_matches_via_actual_character_on_shifted_layout() {
        // Italian layout: "/" is produced by Shift+7.
        let press = char_key("7", "/", Modifiers::SHIFT);

        let matched = press.keymap_press().unwrap();
        assert_eq!(matched.key, KeyMapKey::Logical(Key::Character("/".into())));
        assert!(matched.mods.is_empty());
    }

    #[test]
    fn slash_matches_directly_on_unshifted_layout() {
        // US layout: "/" is its own unshifted key.
        let press = char_key("/", "/", Modifiers::empty());

        let matched = press.keymap_press().unwrap();
        assert_eq!(matched.key, KeyMapKey::Logical(Key::Character("/".into())));
        assert!(matched.mods.is_empty());
    }

    #[test]
    fn legacy_representation_is_preserved_for_backward_compatibility() {
        // US layout event for Shift+4 ("$"), shaped like what an old
        // "shift+4" custom binding expects.
        let press = char_key("4", "$", Modifiers::SHIFT);

        let legacy = press.legacy_keymap_press().unwrap();
        assert_eq!(legacy.key, KeyMapKey::Logical(Key::Character("4".into())));
        assert_eq!(legacy.mods, Modifiers::SHIFT);

        // The primary representation is the new, layout-correct one.
        let primary = press.keymap_press().unwrap();
        assert_eq!(primary.key, KeyMapKey::Logical(Key::Character("$".into())));
        assert!(primary.mods.is_empty());
    }

    #[test]
    fn ctrl_modifier_is_preserved_for_symbol_keys() {
        // ctrl+/ on a US layout: Ctrl is a real modifier, not absorbed.
        let press = char_key("/", "/", Modifiers::CONTROL);

        let matched = press.keymap_press().unwrap();
        assert_eq!(matched.key, KeyMapKey::Logical(Key::Character("/".into())));
        assert_eq!(matched.mods, Modifiers::CONTROL);
    }

    #[test]
    fn unshifted_digit_is_unaffected() {
        // Plain "4" (e.g. vim count prefix): no Shift/AltGr involved, new
        // and legacy representations coincide.
        let press = char_key("4", "4", Modifiers::empty());

        assert_eq!(press.keymap_press(), press.legacy_keymap_press());
    }

    #[test]
    fn alphabetic_keys_use_legacy_representation() {
        // Shift+A: letters are out of scope for this fix, unchanged
        // behavior (case tracked via explicit Shift modifier).
        let press = char_key("a", "A", Modifiers::SHIFT);

        let matched = press.keymap_press().unwrap();
        assert_eq!(matched.key, KeyMapKey::Logical(Key::Character("a".into())));
        assert_eq!(matched.mods, Modifiers::SHIFT);
    }
}
```

- [ ] **Step 6: Run the new tests**

Run: `cargo test -p lapce-app --lib keypress::press::tests`
Expected: PASS - all 6 tests green. (If `KeyMapPress` or `KeyMapKey` don't derive `PartialEq`/`Debug`, this is where it'd surface; both already derive them in `lapce-app/src/keypress/keymap.rs`, no change needed there.)

- [ ] **Step 7: Commit**

```bash
cd /home/fabio/dev/ext/lapce
git add lapce-app/src/keypress/key.rs lapce-app/src/keypress/press.rs
git commit -m "feat(keypress): match symbol keys by actual produced character

Introduce a layout-aware key representation for non-alphabetic
character keys (digits, punctuation), built from the character the
keyboard actually produces instead of the US-QWERTY-shaped
key_without_modifiers + explicit shift bit. The old representation is
kept as legacy_keymap_press for the fallback added in the next commit."
```

---

### Task 2: Matcher fallback and precedence

**Files:**
- Modify: `lapce-app/src/keypress.rs:473-522` (the `match_keymap` method)
- Test: inline `#[cfg(test)]` module in `lapce-app/src/keypress.rs`

**Interfaces:**
- Consumes: `KeyPress::keymap_press()` and `KeyPress::legacy_keymap_press()` from Task 1; existing `KeyMap`, `KeyMapPress`, `KeymapMatch`, `KeyPressFocus`, `Modes` types (all pre-existing in this file/`keymap.rs`/`condition.rs`).
- Produces: `KeyPressData::resolve_keymap(keymaps: &IndexMap<Vec<KeyMapPress>, Vec<KeyMap>>, keypresses: &[KeyPress], check: &T) -> KeymapMatch` - a private associated function factored out of `match_keymap` so it can be tested without constructing a full `KeyPressData`. `match_keymap` itself keeps its existing signature and is used unchanged by `key_down` and the Insert-mode fallback elsewhere in this file.

- [ ] **Step 1: Read current `match_keymap` to confirm line numbers before editing**

Run: `sed -n '473,522p' lapce-app/src/keypress.rs`
Expected output (confirm it matches before editing - if the file has drifted, locate the method by name instead of by line number):

```rust
    fn match_keymap<T: KeyPressFocus + ?Sized>(
        &self,
        keypresses: &[KeyPress],
        check: &T,
    ) -> KeymapMatch {
        let keypresses: Vec<KeyMapPress> =
            keypresses.iter().filter_map(|k| k.keymap_press()).collect();
        let matches: Vec<_> = self
            .keymaps
            .get(&keypresses)
            .map(|keymaps| {
                keymaps
                    .iter()
                    .filter(|keymap| {
                        if check.expect_char()
                            && keypresses.len() == 1
                            && keypresses[0].is_char()
                        {
                            return false;
                        }
                        if !keymap.modes.is_empty()
                            && !keymap.modes.contains(check.get_mode().into())
                        {
                            return false;
                        }
                        if let Some(condition) = &keymap.when {
                            if !Self::check_condition(condition, check) {
                                return false;
                            }
                        }
                        true
                    })
                    .collect()
            })
            .unwrap_or_default();

        if matches.is_empty() {
            KeymapMatch::None
        } else if matches.len() == 1 && matches[0].key == keypresses {
            KeymapMatch::Full(matches[0].command.clone())
        } else if matches.len() > 1
            && matches.iter().filter(|m| m.key != keypresses).count() == 0
        {
            KeymapMatch::Multiple(
                matches.iter().rev().map(|m| m.command.clone()).collect(),
            )
        } else {
            KeymapMatch::Prefix
        }
    }
```

- [ ] **Step 2: Replace it with the split primary/fallback version**

Replace the whole method (the block confirmed in Step 1) with:

```rust
    fn match_keymap<T: KeyPressFocus + ?Sized>(
        &self,
        keypresses: &[KeyPress],
        check: &T,
    ) -> KeymapMatch {
        Self::resolve_keymap(&self.keymaps, keypresses, check)
    }

    /// Resolves a sequence of keypresses against `keymaps`. Takes the map
    /// explicitly (rather than reading `self.keymaps`) so it can be
    /// exercised directly in tests without constructing a full
    /// `KeyPressData` (which needs a reactive `Scope`).
    fn resolve_keymap<T: KeyPressFocus + ?Sized>(
        keymaps: &IndexMap<Vec<KeyMapPress>, Vec<KeyMap>>,
        keypresses: &[KeyPress],
        check: &T,
    ) -> KeymapMatch {
        let primary: Vec<KeyMapPress> =
            keypresses.iter().filter_map(|k| k.keymap_press()).collect();
        let keymatch = Self::match_keymap_presses(keymaps, &primary, check);
        if !matches!(keymatch, KeymapMatch::None) {
            return keymatch;
        }

        // Fall back to the pre-existing key_without_modifiers-based
        // representation, but only for a single keypress: no shipped or
        // known custom keymap uses a symbol/digit key inside a multi-key
        // chord, and extending the fallback to chords would multiply the
        // number of candidate sequences to look up for no known benefit.
        if let [keypress] = keypresses {
            if let Some(legacy) = keypress.legacy_keymap_press() {
                let legacy = vec![legacy];
                if legacy != primary {
                    return Self::match_keymap_presses(keymaps, &legacy, check);
                }
            }
        }

        KeymapMatch::None
    }

    fn match_keymap_presses<T: KeyPressFocus + ?Sized>(
        keymaps: &IndexMap<Vec<KeyMapPress>, Vec<KeyMap>>,
        keypresses: &[KeyMapPress],
        check: &T,
    ) -> KeymapMatch {
        let matches: Vec<_> = keymaps
            .get(keypresses)
            .map(|keymaps| {
                keymaps
                    .iter()
                    .filter(|keymap| {
                        if check.expect_char()
                            && keypresses.len() == 1
                            && keypresses[0].is_char()
                        {
                            return false;
                        }
                        if !keymap.modes.is_empty()
                            && !keymap.modes.contains(check.get_mode().into())
                        {
                            return false;
                        }
                        if let Some(condition) = &keymap.when {
                            if !Self::check_condition(condition, check) {
                                return false;
                            }
                        }
                        true
                    })
                    .collect()
            })
            .unwrap_or_default();

        if matches.is_empty() {
            KeymapMatch::None
        } else if matches.len() == 1 && matches[0].key.as_slice() == keypresses {
            KeymapMatch::Full(matches[0].command.clone())
        } else if matches.len() > 1
            && matches.iter().filter(|m| m.key.as_slice() != keypresses).count() == 0
        {
            KeymapMatch::Multiple(
                matches.iter().rev().map(|m| m.command.clone()).collect(),
            )
        } else {
            KeymapMatch::Prefix
        }
    }
```

- [ ] **Step 3: Build**

Run: `cargo check -p lapce-app`
Expected: compiles with 0 errors. `match_keymap_presses` takes `keypresses: &[KeyMapPress]`; `keymaps.get(keypresses)` works because `IndexMap<Vec<KeyMapPress>, _>::get` accepts `&[KeyMapPress]` via `Borrow<[KeyMapPress]>`, same as it did implicitly before.

- [ ] **Step 4: Write failing tests for the fallback behavior**

Append to `lapce-app/src/keypress.rs` (end of file, new module):

```rust
#[cfg(test)]
mod resolve_keymap_tests {
    use floem::keyboard::{Key, KeyCode, KeyLocation, PhysicalKey};

    use super::*;
    use crate::keypress::{condition::Condition, key::KeyInput};

    #[derive(Debug)]
    struct TestFocus;

    impl KeyPressFocus for TestFocus {
        fn get_mode(&self) -> Mode {
            Mode::Normal
        }

        fn check_condition(&self, _condition: Condition) -> bool {
            false
        }

        fn run_command(
            &self,
            _command: &LapceCommand,
            _count: Option<usize>,
            _mods: Modifiers,
        ) -> CommandExecuted {
            CommandExecuted::Yes
        }
    }

    fn char_press(
        key_without_modifiers: &str,
        logical: &str,
        mods: Modifiers,
    ) -> KeyPress {
        KeyPress {
            key: KeyInput::Keyboard {
                physical: PhysicalKey::Code(KeyCode::Digit7),
                logical: Key::Character(logical.into()),
                location: KeyLocation::Standard,
                key_without_modifiers: Key::Character(key_without_modifiers.into()),
                repeat: false,
            },
            mods,
        }
    }

    fn insert_keymap(
        keymaps: &mut IndexMap<Vec<KeyMapPress>, Vec<KeyMap>>,
        key: &str,
        command: &str,
    ) {
        let key_presses = KeyMapPress::parse(key);
        keymaps.insert(
            key_presses.clone(),
            vec![KeyMap {
                key: key_presses,
                modes: Modes::empty(),
                when: None,
                command: command.to_string(),
            }],
        );
    }

    #[test]
    fn new_style_binding_matches_shifted_layout_event() {
        let mut keymaps = IndexMap::new();
        insert_keymap(&mut keymaps, "/", "search");
        // Italian layout: "/" is produced by Shift+7.
        let press = char_press("7", "/", Modifiers::SHIFT);

        let result = KeyPressData::resolve_keymap(&keymaps, &[press], &TestFocus);
        assert_eq!(result, KeymapMatch::Full("search".to_string()));
    }

    #[test]
    fn old_style_binding_still_matches_via_legacy_fallback() {
        let mut keymaps = IndexMap::new();
        insert_keymap(&mut keymaps, "shift+4", "line_end");
        // US layout event for Shift+4 ("$"), shaped like what the old
        // "shift+4" convention expects.
        let press = char_press("4", "$", Modifiers::SHIFT);

        let result = KeyPressData::resolve_keymap(&keymaps, &[press], &TestFocus);
        assert_eq!(result, KeymapMatch::Full("line_end".to_string()));
    }

    #[test]
    fn new_style_binding_takes_precedence_when_both_would_match() {
        let mut keymaps = IndexMap::new();
        insert_keymap(&mut keymaps, "$", "line_end");
        insert_keymap(&mut keymaps, "shift+4", "some_other_command");
        let press = char_press("4", "$", Modifiers::SHIFT);

        let result = KeyPressData::resolve_keymap(&keymaps, &[press], &TestFocus);
        assert_eq!(result, KeymapMatch::Full("line_end".to_string()));
    }

    #[test]
    fn ctrl_symbol_shortcut_requires_ctrl_modifier() {
        let mut keymaps = IndexMap::new();
        insert_keymap(&mut keymaps, "ctrl+/", "toggle_line_comment");
        // Plain "/" (no ctrl) must not trigger the ctrl+/ binding.
        let press = char_press("/", "/", Modifiers::empty());

        let result = KeyPressData::resolve_keymap(&keymaps, &[press], &TestFocus);
        assert_eq!(result, KeymapMatch::None);
    }

    #[test]
    fn ctrl_symbol_shortcut_matches_on_shifted_layout() {
        let mut keymaps = IndexMap::new();
        insert_keymap(&mut keymaps, "ctrl+/", "toggle_line_comment");
        // Italian layout: ctrl+/ is physically Ctrl+Shift+7.
        let press = char_press("7", "/", Modifiers::CONTROL | Modifiers::SHIFT);

        let result = KeyPressData::resolve_keymap(&keymaps, &[press], &TestFocus);
        assert_eq!(result, KeymapMatch::Full("toggle_line_comment".to_string()));
    }
}
```

- [ ] **Step 5: Run the new tests**

Run: `cargo test -p lapce-app --lib keypress::resolve_keymap_tests`
Expected: PASS - all 5 tests green.

- [ ] **Step 6: Run the full existing keypress test suite plus a full workspace build to check for regressions**

Run: `cargo test -p lapce-app --lib keypress` && `cargo build -p lapce-app`
Expected: all existing tests still pass, 0 build errors.

- [ ] **Step 7: Commit**

```bash
cd /home/fabio/dev/ext/lapce
git add lapce-app/src/keypress.rs
git commit -m "feat(keypress): try layout-aware match before legacy fallback

match_keymap now tries the new logical-character-based representation
first and only falls back to the old key_without_modifiers-based one
(single keypress only) when that finds nothing, so old custom
keymaps.toml entries keep working."
```

---

### Task 3: Migrate default keymaps and verify on the real app

**Files:**
- Modify: `defaults/keymaps-common.toml`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: the matcher from Task 2 (already handles literal-character bindings correctly on every layout).
- Produces: nothing consumed by later tasks - this is the last task.

- [ ] **Step 1: Replace `shift+<digit/punct>` entries with literal characters**

In `defaults/keymaps-common.toml`, make these exact replacements (search for each `key = "..."` line and change only the `key` value on that line; do not change `command` or `mode`):

| Find | Replace with |
|---|---|
| `key = "shift+6"` (near `line_start_non_blank`) | `key = "^"` |
| `key = "shift+4"` (near `line_end`) | `key = "$"` |
| `key = "shift+8"` (near `search_whole_word_forward`) | `key = "*"` |
| `key = "shift+5"` (near `match_pairs`) | `key = "%"` |
| `key = "shift+."` (both occurrences: `motion_mode_indent` and `indent_line`) | `key = ">"` |
| `key = "shift+,"` (both occurrences: `motion_mode_outdent` and `outdent_line`) | `key = "<"` |
| `key = "shift+;"` (near `palette.command`) | `key = ":"` |

Verify each replacement targets the right occurrence by checking the `command` line immediately below it matches the table above. Use this to confirm no entry was missed or wrongly changed:

Run: `grep -n 'key = "shift+[,.4568;]"' defaults/keymaps-common.toml`
Expected: no output (all matching entries have been replaced).

- [ ] **Step 2: Add a CHANGELOG entry**

In `CHANGELOG.md`, under `## Unreleased` / `### Features/Changes`, the existing line from the earlier `/` fix is:

```
- In modal (Vim) mode, bind `/` to open the search bar (was previously bound to go-to-line)
```

Replace that single line with two lines:

```
- In modal (Vim) mode, bind `/` to open the search bar (was previously bound to go-to-line)
- Fix vim-mode symbol keybindings (`/`, `^`, `$`, `%`, `*`, `<`, `>`, `:`) and `ctrl+/` (toggle line comment) not matching on keyboard layouts where those symbols require Shift/AltGr on a different key than on US QWERTY (e.g. Italian keyboards)
```

- [ ] **Step 3: Full workspace build**

Run: `cargo build -p lapce-app`
Expected: 0 errors.

- [ ] **Step 4: Full test suite**

Run: `cargo test -p lapce-app --lib keypress`
Expected: all tests pass, including the ones added in Tasks 1 and 2.

- [ ] **Step 5: Manual verification on the real app (Italian keyboard layout)**

This step needs a human at the keyboard - hand off to the user rather than attempting it programmatically.

1. Quit any running `./target/debug/lapce` instance (the keymaps are compiled in via `include_str!`, so a stale running binary won't reflect the changes).
2. Run `./target/debug/lapce .` (or the user's usual way of launching the freshly built debug binary) to open a fresh instance.
3. Enable modal (Vim) editing if not already on (`core.modal` setting).
4. In Normal mode, in an open file, verify each of these opens/executes the expected action:
   - `/` opens the search bar.
   - `^` moves to the first non-blank character of the line.
   - `$` moves to the end of the line.
   - `%` jumps to the matching bracket (cursor on a bracket character).
   - `*` searches the word under the cursor.
   - `<`/`>` (in Visual mode, or as an operator) outdent/indent.
   - `:` opens the command palette.
5. Verify `ctrl+/` toggles a line comment.
6. Report back which of these worked and which didn't, if any.

- [ ] **Step 6: Commit**

```bash
cd /home/fabio/dev/ext/lapce
git add defaults/keymaps-common.toml CHANGELOG.md
git commit -m "fix(keymaps): use literal symbol characters in default vim bindings

Migrates shift+<digit> style bindings (line_end, line_start_non_blank,
match_pairs, search_whole_word_forward, indent/outdent, command
palette) to literal characters (\$, ^, %, *, <, >, :), which the
layout-aware matcher now resolves correctly on any keyboard layout."
```

## Self-Review Notes

- **Spec coverage:** matching engine change (Task 1), lookup order/backward compatibility (Task 2), default keymap migration (Task 3 Step 1), testing (Tasks 1-2 unit tests + Task 3 Step 5 manual check), CHANGELOG (Task 3 Step 2) - all spec sections have a task.
- **Placeholders:** none - every step has literal code or literal grep/build/test commands.
- **Type consistency:** `KeyPressData::resolve_keymap` and `match_keymap_presses` signatures are defined once (Task 2 Step 2) and reused identically in the Task 2 tests; `KeyPress::keymap_press`/`legacy_keymap_press` defined once (Task 1 Step 3) and reused as-is in Task 2.
