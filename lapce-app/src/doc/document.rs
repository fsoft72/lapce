//! The floem editor document traits implemented by `Doc`.

use std::ops::Range;

use floem::{
    keyboard::Modifiers,
    reactive::{RwSignal, SignalGet, SignalUpdate, SignalWith},
    views::editor::{
        Editor, EditorStyle,
        actions::CommonAction,
        command::{Command, CommandExecuted},
        id::EditorId,
        phantom_text::{PhantomText, PhantomTextKind, PhantomTextLine},
        text::{Document, DocumentPhantom, PreeditData},
        view::{ScreenLines, ScreenLinesBase},
    },
};
use itertools::Itertools;
use lapce_core::{
    buffer::rope_text::RopeText,
    char_buffer::CharBuffer,
    command::EditCommand,
    cursor::{Cursor, CursorAffinity},
    editor::{Action, EditType},
    mode::MotionMode,
    register::Register,
    selection::Selection,
    word::{CharClassification, WordCursor, get_char_property},
};
use lapce_xi_rope::Rope;
use lsp_types::{DiagnosticSeverity, InlayHintLabel};
use smallvec::SmallVec;

use super::Doc;
use crate::{
    command::{CommandKind, LapceCommand},
    config::color::LapceColor,
    editor::compute_screen_lines,
    keypress::KeyPressFocus,
};

impl Document for Doc {
    fn text(&self) -> Rope {
        self.buffer.with_untracked(|buffer| buffer.text().clone())
    }

    fn cache_rev(&self) -> RwSignal<u64> {
        self.cache_rev
    }

    fn find_unmatched(&self, offset: usize, previous: bool, ch: char) -> usize {
        self.syntax().with_untracked(|syntax| {
            if syntax.layers.is_some() {
                syntax
                    .find_tag(offset, previous, &CharBuffer::from(ch))
                    .unwrap_or(offset)
            } else {
                let text = self.text();
                let mut cursor = WordCursor::new(&text, offset);
                let new_offset = if previous {
                    cursor.previous_unmatched(ch)
                } else {
                    cursor.next_unmatched(ch)
                };

                new_offset.unwrap_or(offset)
            }
        })
    }

    fn find_matching_pair(&self, offset: usize) -> usize {
        self.syntax().with_untracked(|syntax| {
            if syntax.layers.is_some() {
                syntax.find_matching_pair(offset).unwrap_or(offset)
            } else {
                let text = self.text();
                WordCursor::new(&text, offset)
                    .match_pairs()
                    .unwrap_or(offset)
            }
        })
    }

    fn preedit(&self) -> PreeditData {
        self.preedit.clone()
    }

    fn compute_screen_lines(
        &self,
        editor: &Editor,
        base: RwSignal<ScreenLinesBase>,
    ) -> ScreenLines {
        let Some(editor_data) = self.editor_data(editor.id()) else {
            return ScreenLines {
                lines: Default::default(),
                info: Default::default(),
                diff_sections: Default::default(),
                base,
            };
        };

        compute_screen_lines(
            self.common.config,
            base,
            editor_data.kind.read_only(),
            &editor_data.doc_signal().get(),
            editor.lines(),
            editor.text_prov(),
            editor.config_id(),
        )
    }

    fn run_command(
        &self,
        ed: &Editor,
        cmd: &Command,
        count: Option<usize>,
        modifiers: Modifiers,
    ) -> CommandExecuted {
        let Some(editor_data) = self.editor_data(ed.id()) else {
            return CommandExecuted::No;
        };

        let cmd = CommandKind::from(cmd.clone());
        let cmd = LapceCommand {
            kind: cmd,
            data: None,
        };
        editor_data.run_command(&cmd, count, modifiers)
    }

    fn receive_char(&self, ed: &Editor, c: &str) {
        let Some(editor_data) = self.editor_data(ed.id()) else {
            return;
        };

        editor_data.receive_char(c);
    }

    fn edit(
        &self,
        iter: &mut dyn Iterator<Item = (Selection, &str)>,
        edit_type: EditType,
    ) {
        let delta = self
            .buffer
            .try_update(|buffer| buffer.edit(iter, edit_type))
            .unwrap();
        self.apply_deltas(&[delta]);
    }
}

impl DocumentPhantom for Doc {
    fn phantom_text(
        &self,
        _: EditorId,
        _: &EditorStyle,
        line: usize,
    ) -> PhantomTextLine {
        let config = &self.common.config.get_untracked();

        let (start_offset, end_offset) = self.buffer.with_untracked(|buffer| {
            (buffer.offset_of_line(line), buffer.offset_of_line(line + 1))
        });

        let inlay_hints = self.inlay_hints.get_untracked();
        // If hints are enabled, and the hints field is filled, then get the hints for this line
        // and convert them into PhantomText instances
        let hints = config
            .editor
            .enable_inlay_hints
            .then_some(())
            .and(inlay_hints.as_ref())
            .map(|hints| hints.iter_chunks(start_offset..end_offset))
            .into_iter()
            .flatten()
            .filter(|(interval, _)| {
                interval.start >= start_offset && interval.start < end_offset
            })
            .map(|(interval, inlay_hint)| {
                let (col, affinity) = self.buffer.with_untracked(|b| {
                    let mut cursor =
                        lapce_xi_rope::Cursor::new(b.text(), interval.start);

                    let next_char = cursor.peek_next_codepoint();
                    let prev_char = cursor.prev_codepoint();

                    let mut affinity = None;
                    if let Some(prev_char) = prev_char {
                        let c = get_char_property(prev_char);
                        if c == CharClassification::Other {
                            affinity = Some(CursorAffinity::Backward)
                        } else if matches!(
                            c,
                            CharClassification::Lf
                                | CharClassification::Cr
                                | CharClassification::Space
                        ) {
                            affinity = Some(CursorAffinity::Forward)
                        }
                    };
                    if affinity.is_none() {
                        if let Some(next_char) = next_char {
                            let c = get_char_property(next_char);
                            if c == CharClassification::Other {
                                affinity = Some(CursorAffinity::Forward)
                            } else if matches!(
                                c,
                                CharClassification::Lf
                                    | CharClassification::Cr
                                    | CharClassification::Space
                            ) {
                                affinity = Some(CursorAffinity::Backward)
                            }
                        }
                    }

                    let (_, col) = b.offset_to_line_col(interval.start);
                    (col, affinity)
                });
                let text = match &inlay_hint.label {
                    InlayHintLabel::String(label) => label.to_string(),
                    InlayHintLabel::LabelParts(parts) => {
                        parts.iter().map(|p| &p.value).join("")
                    }
                };
                PhantomText {
                    kind: PhantomTextKind::InlayHint,
                    col,
                    text,
                    affinity,
                    fg: Some(config.color(LapceColor::INLAY_HINT_FOREGROUND)),
                    // font_family: Some(config.editor.inlay_hint_font_family()),
                    font_size: Some(config.editor.inlay_hint_font_size()),
                    bg: Some(config.color(LapceColor::INLAY_HINT_BACKGROUND)),
                    under_line: None,
                }
            });
        // You're quite unlikely to have more than six hints on a single line
        // this later has the diagnostics added onto it, but that's still likely to be below six
        // overall.
        let mut text: SmallVec<[PhantomText; 6]> = hints.collect();

        // If error lens is enabled, and the diagnostics field is filled, then get the diagnostics
        // that end on this line which have a severity worse than HINT and convert them into
        // PhantomText instances

        let mut diag_text: SmallVec<[PhantomText; 6]> =
            self.buffer.with_untracked(|buffer| {
                config
                    .editor
                    .enable_error_lens
                    .then_some(())
                    .map(|_| self.diagnostics.diagnostics_span.get_untracked())
                    .map(|diags| {
                        diags
                            .iter_chunks(start_offset..end_offset)
                            .filter_map(|(iv, diag)| {
                                let end = iv.end();
                                let end_line = buffer.line_of_offset(end);
                                if end_line == line
                                    && diag.severity < Some(DiagnosticSeverity::HINT)
                                {
                                    let fg = {
                                        let severity = diag
                                            .severity
                                            .unwrap_or(DiagnosticSeverity::WARNING);
                                        let theme_prop = if severity
                                            == DiagnosticSeverity::ERROR
                                        {
                                            LapceColor::ERROR_LENS_ERROR_FOREGROUND
                                        } else if severity
                                            == DiagnosticSeverity::WARNING
                                        {
                                            LapceColor::ERROR_LENS_WARNING_FOREGROUND
                                        } else {
                                            // information + hint (if we keep that) + things without a severity
                                            LapceColor::ERROR_LENS_OTHER_FOREGROUND
                                        };

                                        config.color(theme_prop)
                                    };

                                    let text =
                                        if config.editor.only_render_error_styling {
                                            "".to_string()
                                        } else if config.editor.error_lens_multiline
                                        {
                                            format!("    {}", diag.message)
                                        } else {
                                            format!(
                                                "    {}",
                                                diag.message.lines().join(" ")
                                            )
                                        };
                                    Some(PhantomText {
                                        kind: PhantomTextKind::Diagnostic,
                                        col: end_offset - start_offset,
                                        affinity: Some(CursorAffinity::Backward),
                                        text,
                                        fg: Some(fg),
                                        font_size: Some(
                                            config.editor.error_lens_font_size(),
                                        ),
                                        bg: None,
                                        under_line: None,
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect::<SmallVec<[PhantomText; 6]>>()
                    })
                    .unwrap_or_default()
            });

        text.append(&mut diag_text);

        // The completion texts are only cloned for the line that shows them:
        // this runs for every line laid out.
        let (completion_line, completion_col) = self.completion_pos.get_untracked();
        let completion_text = (config.editor.enable_completion_lens
            && line == completion_line)
            .then(|| self.completion_lens.get_untracked())
            .flatten()
            // TODO: We're probably missing on various useful completion things to include here!
            .map(|completion| PhantomText {
                kind: PhantomTextKind::Completion,
                col: completion_col,
                text: completion,
                fg: Some(config.color(LapceColor::COMPLETION_LENS_FOREGROUND)),
                font_size: Some(config.editor.completion_lens_font_size()),
                affinity: Some(CursorAffinity::Backward),
                // font_family: Some(config.editor.completion_lens_font_family()),
                bg: None,
                under_line: None,
            });
        if let Some(completion_text) = completion_text {
            text.push(completion_text);
        }

        // TODO: don't display completion lens and inline completion at the same time
        // and/or merge them so that they can be shifted between like multiple inline completions
        // can
        let (inline_completion_line, inline_completion_col) =
            self.inline_completion_pos.get_untracked();
        let inline_completion_text = (config.editor.enable_inline_completion
            && line == inline_completion_line)
            .then(|| self.inline_completion.get_untracked())
            .flatten()
            .map(|completion| PhantomText {
                kind: PhantomTextKind::Completion,
                col: inline_completion_col,
                text: completion,
                affinity: Some(CursorAffinity::Backward),
                fg: Some(config.color(LapceColor::COMPLETION_LENS_FOREGROUND)),
                font_size: Some(config.editor.completion_lens_font_size()),
                // font_family: Some(config.editor.completion_lens_font_family()),
                bg: None,
                under_line: None,
            });
        if let Some(inline_completion_text) = inline_completion_text {
            text.push(inline_completion_text);
        }

        if let Some(preedit) = self
            .preedit_phantom(Some(config.color(LapceColor::EDITOR_FOREGROUND)), line)
        {
            text.push(preedit)
        }

        text.sort_by(|a, b| {
            if a.col == b.col {
                a.kind.cmp(&b.kind)
            } else {
                a.col.cmp(&b.col)
            }
        });

        PhantomTextLine { text }
    }

    fn has_multiline_phantom(&self, _: EditorId, _: &EditorStyle) -> bool {
        // TODO: actually check
        true
    }
}
impl CommonAction for Doc {
    fn exec_motion_mode(
        &self,
        _ed: &Editor,
        cursor: &mut Cursor,
        motion_mode: MotionMode,
        range: Range<usize>,
        is_vertical: bool,
        register: &mut Register,
    ) {
        let deltas = self
            .buffer
            .try_update(move |buffer| {
                Action::execute_motion_mode(
                    cursor,
                    buffer,
                    motion_mode,
                    range,
                    is_vertical,
                    register,
                )
            })
            .unwrap();
        self.apply_deltas(&deltas);
    }

    fn do_edit(
        &self,
        _ed: &Editor,
        cursor: &mut Cursor,
        cmd: &EditCommand,
        modal: bool,
        register: &mut Register,
        smart_tab: bool,
    ) -> bool {
        let deltas = Doc::do_edit(self, cursor, cmd, modal, register, smart_tab);
        !deltas.is_empty()
    }
}
