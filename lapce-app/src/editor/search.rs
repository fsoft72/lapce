//! Search and replace in the editor.

use floem::{
    keyboard::Modifiers,
    reactive::{SignalGet, SignalUpdate, SignalWith},
};
use lapce_core::{buffer::rope_text::RopeText, selection::Selection};

use super::EditorData;
use crate::command::InternalCommand;
use crate::tracing::*;

impl EditorData {
    pub(super) fn search_whole_word_forward(&self, mods: Modifiers) {
        let offset = self.cursor().with_untracked(|c| c.offset());
        let (word, buffer) = self.doc().buffer.with_untracked(|buffer| {
            let (start, end) = buffer.select_word(offset);
            (buffer.slice_to_cow(start..end).to_string(), buffer.clone())
        });
        self.common.internal_command.send(InternalCommand::Search {
            pattern: Some(word),
        });
        let next = self.common.find.next(buffer.text(), offset, false, true);

        if let Some((start, _end)) = next {
            self.run_move_command(
                &lapce_core::movement::Movement::Offset(start),
                None,
                mods,
            );
        }
    }

    pub(super) fn search_forward(&self, mods: Modifiers) {
        let offset = self.cursor().with_untracked(|c| c.offset());
        let text = self
            .doc()
            .buffer
            .with_untracked(|buffer| buffer.text().clone());
        let next = self.common.find.next(&text, offset, false, true);

        if let Some((start, _end)) = next {
            self.run_move_command(
                &lapce_core::movement::Movement::Offset(start),
                None,
                mods,
            );
        }
    }

    pub(super) fn search_backward(&self, mods: Modifiers) {
        let offset = self.cursor().with_untracked(|c| c.offset());
        let text = self
            .doc()
            .buffer
            .with_untracked(|buffer| buffer.text().clone());
        let next = self.common.find.next(&text, offset, true, true);

        if let Some((start, _end)) = next {
            self.run_move_command(
                &lapce_core::movement::Movement::Offset(start),
                None,
                mods,
            );
        }
    }

    pub(super) fn replace_next(&self, text: &str) {
        let offset = self.cursor().with_untracked(|c| c.offset());
        let buffer = self.doc().buffer.with_untracked(|buffer| buffer.clone());
        let next = self.common.find.next(buffer.text(), offset, false, true);

        if let Some((start, end)) = next {
            let selection = Selection::region(start, end);
            self.do_edit(&selection, &[(selection.clone(), text)]);
        }
    }

    pub(super) fn replace_all(&self, text: &str) {
        let offset = self.cursor().with_untracked(|c| c.offset());

        self.doc().update_find();

        let edits: Vec<(Selection, &str)> = self
            .doc()
            .find_result
            .occurrences
            .get_untracked()
            .regions()
            .iter()
            .map(|region| (Selection::region(region.start, region.end), text))
            .collect();
        if !edits.is_empty() {
            self.do_edit(&Selection::caret(offset), &edits);
        }
    }
}

impl EditorData {
    #[instrument]
    pub fn word_at_cursor(&self) -> String {
        let doc = self.doc();
        let region = self.cursor().with_untracked(|c| match &c.mode {
            lapce_core::cursor::CursorMode::Normal(offset) => {
                lapce_core::selection::SelRegion::caret(*offset)
            }
            lapce_core::cursor::CursorMode::Visual {
                start,
                end,
                mode: _,
            } => lapce_core::selection::SelRegion::new(
                *start.min(end),
                doc.buffer.with_untracked(|buffer| {
                    buffer.next_grapheme_offset(*start.max(end), 1, buffer.len())
                }),
                None,
            ),
            lapce_core::cursor::CursorMode::Insert(selection) => {
                *selection.last_inserted().unwrap()
            }
        });

        if region.is_caret() {
            doc.buffer.with_untracked(|buffer| {
                let (start, end) = buffer.select_word(region.start);
                buffer.slice_to_cow(start..end).to_string()
            })
        } else {
            doc.buffer.with_untracked(|buffer| {
                buffer.slice_to_cow(region.min()..region.max()).to_string()
            })
        }
    }

    #[instrument]
    pub fn clear_search(&self) {
        self.common.find.visual.set(false);
        self.find_focus.set(false);
    }

    #[instrument]
    pub(super) fn search(&self) {
        let pattern = self.word_at_cursor();

        let pattern = if pattern.contains('\n') || pattern.is_empty() {
            None
        } else {
            Some(pattern)
        };

        self.common
            .internal_command
            .send(InternalCommand::Search { pattern });
        self.common.find.visual.set(true);
        self.find_focus.set(true);
        self.common.find.replace_focus.set(false);
    }
}
