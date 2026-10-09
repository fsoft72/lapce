//! Command handling of the editor: edit, move, scroll and focus commands and the key press focus.

use std::collections::HashSet;

use floem::{
    keyboard::Modifiers,
    reactive::{SignalGet, SignalUpdate, SignalWith, batch},
    views::editor::{command::CommandExecuted, movement},
};
use lapce_core::{
    buffer::{InvalLines, rope_text::RopeText},
    command::{
        EditCommand, FocusCommand, MotionModeCommand, MultiSelectionCommand,
        ScrollCommand,
    },
    cursor::CursorMode,
    mode::{Mode, MotionMode},
    selection::SelRegion,
};
use lapce_xi_rope::{Rope, RopeDelta};
use lsp_types::InlineCompletionTriggerKind;

use super::{EditorData, InlineFindDirection};
use crate::tracing::*;
use crate::{
    command::{CommandKind, InternalCommand},
    editor_tab::EditorTabChild,
    keypress::{KeyPressFocus, condition::Condition},
    main_split::{SplitDirection, SplitMoveDirection},
};

impl EditorData {
    pub(super) fn run_edit_command(&self, cmd: &EditCommand) -> CommandExecuted {
        let doc = self.doc();
        let text = self.editor.rope_text();
        let is_local = doc.content.with_untracked(|content| content.is_local());
        let modal = self.editor.es.with_untracked(|s| s.modal()) && !is_local;
        let smart_tab = self
            .common
            .config
            .with_untracked(|config| config.editor.smart_tab);
        let doc_before_edit = text.text().clone();
        let mut cursor = self.editor.cursor.get_untracked();
        let mut register = self.common.register.get_untracked();

        let yank_data =
            if let lapce_core::cursor::CursorMode::Visual { .. } = &cursor.mode {
                Some(cursor.yank(&text))
            } else {
                None
            };

        let deltas =
            batch(|| doc.do_edit(&mut cursor, cmd, modal, &mut register, smart_tab));

        if !deltas.is_empty() {
            if let Some(data) = yank_data {
                register.add_delete(data);
            }
        }

        self.editor.cursor.set(cursor);
        self.editor.register.set(register);

        if show_completion(cmd, &doc_before_edit, &deltas) {
            self.update_completion(false);
        } else {
            self.cancel_completion();
        }

        if *cmd == EditCommand::InsertNewLine {
            // Cancel so that there's no flickering
            self.cancel_inline_completion();
            self.update_inline_completion(InlineCompletionTriggerKind::Automatic);
            self.quit_on_screen_find();
        } else if show_inline_completion(cmd) {
            self.update_inline_completion(InlineCompletionTriggerKind::Automatic);
        } else {
            self.cancel_inline_completion();
        }

        self.apply_deltas(&deltas);
        if let EditCommand::NormalMode = cmd {
            self.snippet.set(None);
            self.quit_on_screen_find();
        }

        CommandExecuted::Yes
    }

    fn run_motion_mode_command(
        &self,
        cmd: &MotionModeCommand,
        count: Option<usize>,
    ) -> CommandExecuted {
        let count = count.unwrap_or(1);
        let motion_mode = match cmd {
            MotionModeCommand::MotionModeDelete => MotionMode::Delete { count },
            MotionModeCommand::MotionModeIndent => MotionMode::Indent,
            MotionModeCommand::MotionModeOutdent => MotionMode::Outdent,
            MotionModeCommand::MotionModeYank => MotionMode::Yank { count },
        };
        let mut cursor = self.editor.cursor.get_untracked();
        let mut register = self.common.register.get_untracked();

        movement::do_motion_mode(
            &self.editor,
            &*self.doc(),
            &mut cursor,
            motion_mode,
            &mut register,
        );

        self.editor.cursor.set(cursor);
        self.common.register.set(register);

        CommandExecuted::Yes
    }

    fn run_multi_selection_command(
        &self,
        cmd: &MultiSelectionCommand,
    ) -> CommandExecuted {
        let mut cursor = self.editor.cursor.get_untracked();
        let rope_text = self.rope_text();
        let doc = self.doc();
        let config = self.common.config.get_untracked();

        // This is currently special-cased in Lapce because floem editor does not have 'find'
        match cmd {
            MultiSelectionCommand::SelectAllCurrent => {
                if let CursorMode::Insert(mut selection) = cursor.mode.clone() {
                    if !selection.is_empty() {
                        let find = doc.find();

                        let first = selection.first().unwrap();
                        let (start, end) = if first.is_caret() {
                            rope_text.select_word(first.start)
                        } else {
                            (first.min(), first.max())
                        };
                        let search_str = rope_text.slice_to_cow(start..end);
                        let case_sensitive = find.case_sensitive(false);
                        let multicursor_case_sensitive =
                            config.editor.multicursor_case_sensitive;
                        let case_sensitive =
                            multicursor_case_sensitive || case_sensitive;
                        // let search_whole_word = config.editor.multicursor_whole_words;
                        find.set_case_sensitive(case_sensitive);
                        find.set_find(&search_str);
                        let mut offset = 0;
                        while let Some((start, end)) =
                            find.next(rope_text.text(), offset, false, false)
                        {
                            offset = end;
                            selection.add_region(SelRegion::new(start, end, None));
                        }
                    }
                    cursor.set_insert(selection);
                }
            }
            MultiSelectionCommand::SelectNextCurrent => {
                if let CursorMode::Insert(mut selection) = cursor.mode.clone() {
                    if !selection.is_empty() {
                        let mut had_caret = false;
                        for region in selection.regions_mut() {
                            if region.is_caret() {
                                had_caret = true;
                                let (start, end) =
                                    rope_text.select_word(region.start);
                                region.start = start;
                                region.end = end;
                            }
                        }
                        if !had_caret {
                            let find = doc.find();

                            let r = selection.last_inserted().unwrap();
                            let search_str =
                                rope_text.slice_to_cow(r.min()..r.max());
                            let case_sensitive = find.case_sensitive(false);
                            let case_sensitive =
                                config.editor.multicursor_case_sensitive
                                    || case_sensitive;
                            // let search_whole_word =
                            // config.editor.multicursor_whole_words;
                            find.set_case_sensitive(case_sensitive);
                            find.set_find(&search_str);
                            let mut offset = r.max();
                            let mut seen = HashSet::new();
                            while let Some((start, end)) =
                                find.next(rope_text.text(), offset, false, true)
                            {
                                if !selection
                                    .regions()
                                    .iter()
                                    .any(|r| r.min() == start && r.max() == end)
                                {
                                    selection.add_region(SelRegion::new(
                                        start, end, None,
                                    ));
                                    break;
                                }
                                if seen.contains(&end) {
                                    break;
                                }
                                offset = end;
                                seen.insert(offset);
                            }
                        }
                    }
                    cursor.set_insert(selection);
                }
            }
            MultiSelectionCommand::SelectSkipCurrent => {
                if let CursorMode::Insert(mut selection) = cursor.mode.clone() {
                    if !selection.is_empty() {
                        let r = selection.last_inserted().unwrap();
                        if r.is_caret() {
                            let (start, end) = rope_text.select_word(r.start);
                            selection.replace_last_inserted_region(SelRegion::new(
                                start, end, None,
                            ));
                        } else {
                            let find = doc.find();

                            let search_str =
                                rope_text.slice_to_cow(r.min()..r.max());
                            find.set_find(&search_str);
                            let mut offset = r.max();
                            let mut seen = HashSet::new();
                            while let Some((start, end)) =
                                find.next(rope_text.text(), offset, false, true)
                            {
                                if !selection
                                    .regions()
                                    .iter()
                                    .any(|r| r.min() == start && r.max() == end)
                                {
                                    selection.replace_last_inserted_region(
                                        SelRegion::new(start, end, None),
                                    );
                                    break;
                                }
                                if seen.contains(&end) {
                                    break;
                                }
                                offset = end;
                                seen.insert(offset);
                            }
                        }
                    }
                    cursor.set_insert(selection);
                }
            }
            _ => movement::do_multi_selection(&self.editor, &mut cursor, cmd),
        };

        self.editor.cursor.set(cursor);
        // self.cancel_signature();
        self.cancel_completion();
        self.cancel_inline_completion();
        CommandExecuted::Yes
    }

    pub(super) fn run_move_command(
        &self,
        movement: &lapce_core::movement::Movement,
        count: Option<usize>,
        mods: Modifiers,
    ) -> CommandExecuted {
        self.common.hover.active.set(false);
        if movement.is_jump()
            && movement != &self.editor.last_movement.get_untracked()
        {
            let path = self
                .doc()
                .content
                .with_untracked(|content| content.path().cloned());
            if let Some(path) = path {
                let offset = self.cursor().with_untracked(|c| c.offset());
                let scroll_offset =
                    self.viewport().get_untracked().origin().to_vec2();
                self.common.internal_command.send(
                    InternalCommand::SaveJumpLocation {
                        path,
                        offset,
                        scroll_offset,
                    },
                );
            }
        }
        self.editor.last_movement.set(movement.clone());

        let mut cursor = self.cursor().get_untracked();
        self.common.register.update(|register| {
            movement::move_cursor(
                &self.editor,
                &*self.doc(),
                &mut cursor,
                movement,
                count.unwrap_or(1),
                mods.shift(),
                register,
            )
        });

        self.editor.cursor.set(cursor);

        if self.snippet.with_untracked(|s| s.is_some()) {
            self.snippet.update(|snippet| {
                let offset = self.editor.cursor.get_untracked().offset();
                let mut within_region = false;
                for (_, (start, end)) in snippet.as_mut().unwrap() {
                    if offset >= *start && offset <= *end {
                        within_region = true;
                        break;
                    }
                }
                if !within_region {
                    *snippet = None;
                }
            })
        }
        self.cancel_completion();
        CommandExecuted::Yes
    }

    pub fn run_scroll_command(
        &self,
        cmd: &ScrollCommand,
        count: Option<usize>,
        mods: Modifiers,
    ) -> CommandExecuted {
        let prev_completion_index = self
            .common
            .completion
            .with_untracked(|c| c.active.get_untracked());

        match cmd {
            ScrollCommand::PageUp => {
                self.editor.page_move(false, mods);
            }
            ScrollCommand::PageDown => {
                self.editor.page_move(true, mods);
            }
            ScrollCommand::ScrollUp => {
                self.scroll(false, count.unwrap_or(1), mods);
            }
            ScrollCommand::ScrollDown => {
                self.scroll(true, count.unwrap_or(1), mods);
            }
            // TODO: move the cursor line to the center, top or bottom of the window.
            ScrollCommand::CenterOfWindow => {}
            ScrollCommand::TopOfWindow => {}
            ScrollCommand::BottomOfWindow => {}
        }

        let current_completion_index = self
            .common
            .completion
            .with_untracked(|c| c.active.get_untracked());

        if prev_completion_index != current_completion_index {
            self.common.completion.with_untracked(|c| {
                let cursor_offset = self.cursor().with_untracked(|c| c.offset());
                c.update_document_completion(self, cursor_offset);
            });
        }

        CommandExecuted::Yes
    }

    pub fn run_focus_command(
        &self,
        cmd: &FocusCommand,
        _count: Option<usize>,
        mods: Modifiers,
    ) -> CommandExecuted {
        // TODO(minor): Evaluate whether we should split this into subenums,
        // such as actions specific to the actual editor pane, movement, and list movement.
        let prev_completion_index = self
            .common
            .completion
            .with_untracked(|c| c.active.get_untracked());

        match cmd {
            FocusCommand::ModalClose => {
                self.cancel_completion();
            }
            FocusCommand::SplitVertical => {
                if let Some(editor_tab_id) =
                    self.editor_tab_id.read_only().get_untracked()
                {
                    self.common.internal_command.send(InternalCommand::Split {
                        direction: SplitDirection::Vertical,
                        editor_tab_id,
                    });
                } else if let Some((editor_tab_id, _)) =
                    self.diff_editor_id.get_untracked()
                {
                    self.common.internal_command.send(InternalCommand::Split {
                        direction: SplitDirection::Vertical,
                        editor_tab_id,
                    });
                } else {
                    return CommandExecuted::No;
                }
            }
            FocusCommand::SplitHorizontal => {
                if let Some(editor_tab_id) = self.editor_tab_id.get_untracked() {
                    self.common.internal_command.send(InternalCommand::Split {
                        direction: SplitDirection::Horizontal,
                        editor_tab_id,
                    });
                } else if let Some((editor_tab_id, _)) =
                    self.diff_editor_id.get_untracked()
                {
                    self.common.internal_command.send(InternalCommand::Split {
                        direction: SplitDirection::Horizontal,
                        editor_tab_id,
                    });
                } else {
                    return CommandExecuted::No;
                }
            }
            FocusCommand::SplitRight => {
                if let Some(editor_tab_id) = self.editor_tab_id.get_untracked() {
                    self.common
                        .internal_command
                        .send(InternalCommand::SplitMove {
                            direction: SplitMoveDirection::Right,
                            editor_tab_id,
                        });
                } else if let Some((editor_tab_id, _)) =
                    self.diff_editor_id.get_untracked()
                {
                    self.common
                        .internal_command
                        .send(InternalCommand::SplitMove {
                            direction: SplitMoveDirection::Right,
                            editor_tab_id,
                        });
                } else {
                    return CommandExecuted::No;
                }
            }
            FocusCommand::SplitLeft => {
                if let Some(editor_tab_id) = self.editor_tab_id.get_untracked() {
                    self.common
                        .internal_command
                        .send(InternalCommand::SplitMove {
                            direction: SplitMoveDirection::Left,
                            editor_tab_id,
                        });
                } else if let Some((editor_tab_id, _)) =
                    self.diff_editor_id.get_untracked()
                {
                    self.common
                        .internal_command
                        .send(InternalCommand::SplitMove {
                            direction: SplitMoveDirection::Left,
                            editor_tab_id,
                        });
                } else {
                    return CommandExecuted::No;
                }
            }
            FocusCommand::SplitUp => {
                if let Some(editor_tab_id) = self.editor_tab_id.get_untracked() {
                    self.common
                        .internal_command
                        .send(InternalCommand::SplitMove {
                            direction: SplitMoveDirection::Up,
                            editor_tab_id,
                        });
                } else if let Some((editor_tab_id, _)) =
                    self.diff_editor_id.get_untracked()
                {
                    self.common
                        .internal_command
                        .send(InternalCommand::SplitMove {
                            direction: SplitMoveDirection::Up,
                            editor_tab_id,
                        });
                } else {
                    return CommandExecuted::No;
                }
            }
            FocusCommand::SplitDown => {
                if let Some(editor_tab_id) = self.editor_tab_id.get_untracked() {
                    self.common
                        .internal_command
                        .send(InternalCommand::SplitMove {
                            direction: SplitMoveDirection::Down,
                            editor_tab_id,
                        });
                } else if let Some((editor_tab_id, _)) =
                    self.diff_editor_id.get_untracked()
                {
                    self.common
                        .internal_command
                        .send(InternalCommand::SplitMove {
                            direction: SplitMoveDirection::Down,
                            editor_tab_id,
                        });
                } else {
                    return CommandExecuted::No;
                }
            }
            FocusCommand::SplitExchange => {
                if let Some(editor_tab_id) = self.editor_tab_id.get_untracked() {
                    self.common
                        .internal_command
                        .send(InternalCommand::SplitExchange { editor_tab_id });
                } else if let Some((editor_tab_id, _)) =
                    self.diff_editor_id.get_untracked()
                {
                    self.common
                        .internal_command
                        .send(InternalCommand::SplitExchange { editor_tab_id });
                } else {
                    return CommandExecuted::No;
                }
            }
            FocusCommand::SplitClose => {
                if let Some(editor_tab_id) = self.editor_tab_id.get_untracked() {
                    self.common.internal_command.send(
                        InternalCommand::EditorTabChildClose {
                            editor_tab_id,
                            child: EditorTabChild::Editor(self.id()),
                        },
                    );
                } else if let Some((editor_tab_id, diff_editor_id)) =
                    self.diff_editor_id.get_untracked()
                {
                    self.common.internal_command.send(
                        InternalCommand::EditorTabChildClose {
                            editor_tab_id,
                            child: EditorTabChild::DiffEditor(diff_editor_id),
                        },
                    );
                } else {
                    return CommandExecuted::No;
                }
            }
            FocusCommand::ListNext => {
                self.common.completion.update(|c| {
                    c.next();
                });
            }
            FocusCommand::ListPrevious => {
                self.common.completion.update(|c| {
                    c.previous();
                });
            }
            FocusCommand::ListNextPage => {
                self.common.completion.update(|c| {
                    c.next_page();
                });
            }
            FocusCommand::ListPreviousPage => {
                self.common.completion.update(|c| {
                    c.previous_page();
                });
            }
            FocusCommand::ListSelect => {
                self.select_completion();
                self.cancel_inline_completion();
            }
            FocusCommand::JumpToNextSnippetPlaceholder => {
                self.snippet.update(|snippet| {
                    if let Some(snippet_mut) = snippet.as_mut() {
                        let mut current = 0;
                        let offset = self.cursor().get_untracked().offset();
                        for (i, (_, (start, end))) in snippet_mut.iter().enumerate()
                        {
                            if *start <= offset && offset <= *end {
                                current = i;
                                break;
                            }
                        }

                        let last_placeholder = current + 1 >= snippet_mut.len() - 1;

                        if let Some((_, (start, end))) = snippet_mut.get(current + 1)
                        {
                            let mut selection =
                                lapce_core::selection::Selection::new();
                            let region = lapce_core::selection::SelRegion::new(
                                *start, *end, None,
                            );
                            selection.add_region(region);
                            self.cursor().update(|cursor| {
                                cursor.set_insert(selection);
                            });
                        }

                        if last_placeholder {
                            *snippet = None;
                        }
                        // self.update_signature();
                        self.cancel_completion();
                        self.cancel_inline_completion();
                    }
                });
            }
            FocusCommand::JumpToPrevSnippetPlaceholder => {
                self.snippet.update(|snippet| {
                    if let Some(snippet_mut) = snippet.as_mut() {
                        let mut current = 0;
                        let offset = self.cursor().get_untracked().offset();
                        for (i, (_, (start, end))) in snippet_mut.iter().enumerate()
                        {
                            if *start <= offset && offset <= *end {
                                current = i;
                                break;
                            }
                        }

                        if current > 0 {
                            if let Some((_, (start, end))) =
                                snippet_mut.get(current - 1)
                            {
                                let mut selection =
                                    lapce_core::selection::Selection::new();
                                let region = lapce_core::selection::SelRegion::new(
                                    *start, *end, None,
                                );
                                selection.add_region(region);
                                self.cursor().update(|cursor| {
                                    cursor.set_insert(selection);
                                });
                            }
                            // self.update_signature();
                            self.cancel_completion();
                            self.cancel_inline_completion();
                        }
                    }
                });
            }
            FocusCommand::GotoDefinition => {
                self.go_to_definition();
            }
            FocusCommand::ShowCodeActions => {
                self.show_code_actions(false);
            }
            FocusCommand::SearchWholeWordForward => {
                self.search_whole_word_forward(mods);
            }
            FocusCommand::SearchForward => {
                self.search_forward(mods);
            }
            FocusCommand::SearchBackward => {
                self.search_backward(mods);
            }
            FocusCommand::Save => {
                self.save(true, || {});
            }
            FocusCommand::SaveWithoutFormatting => {
                self.save(false, || {});
            }
            FocusCommand::FormatDocument => {
                self.format();
            }
            FocusCommand::InlineFindLeft => {
                self.inline_find.set(Some(InlineFindDirection::Left));
            }
            FocusCommand::InlineFindRight => {
                self.inline_find.set(Some(InlineFindDirection::Right));
            }
            FocusCommand::OnScreenFind => {
                self.on_screen_find.update(|find| {
                    find.active = true;
                    find.pattern.clear();
                    find.regions.clear();
                });
            }
            FocusCommand::RepeatLastInlineFind => {
                if let Some((direction, c)) = self.last_inline_find.get_untracked() {
                    self.inline_find(direction, &c);
                }
            }
            FocusCommand::Rename => {
                self.rename();
            }
            FocusCommand::ClearSearch => {
                self.clear_search();
            }
            FocusCommand::Search => {
                self.search();
            }
            FocusCommand::FocusFindEditor => {
                self.common.find.replace_focus.set(false);
            }
            FocusCommand::FocusReplaceEditor => {
                if self.common.find.replace_active.get_untracked() {
                    self.common.find.replace_focus.set(true);
                }
            }
            FocusCommand::InlineCompletionSelect => {
                self.select_inline_completion();
            }
            FocusCommand::InlineCompletionNext => {
                self.next_inline_completion();
            }
            FocusCommand::InlineCompletionPrevious => {
                self.previous_inline_completion();
            }
            FocusCommand::InlineCompletionCancel => {
                self.cancel_inline_completion();
            }
            FocusCommand::InlineCompletionInvoke => {
                self.update_inline_completion(InlineCompletionTriggerKind::Invoked);
            }
            FocusCommand::ShowHover => {
                let start_offset = self.doc().buffer.with_untracked(|b| {
                    b.prev_code_boundary(self.cursor().get_untracked().offset())
                });
                self.update_hover(start_offset);
            }
            _ => {}
        }

        let current_completion_index = self
            .common
            .completion
            .with_untracked(|c| c.active.get_untracked());

        if prev_completion_index != current_completion_index {
            self.common.completion.with_untracked(|c| {
                let cursor_offset = self.cursor().with_untracked(|c| c.offset());
                c.update_document_completion(self, cursor_offset);
            });
        }

        CommandExecuted::Yes
    }
}

impl KeyPressFocus for EditorData {
    fn get_mode(&self) -> Mode {
        if self.common.find.visual.get_untracked() && self.find_focus.get_untracked()
        {
            Mode::Insert
        } else {
            self.cursor().with_untracked(|c| c.get_mode())
        }
    }

    #[instrument]
    fn check_condition(&self, condition: Condition) -> bool {
        match condition {
            Condition::InputFocus => {
                self.common.find.visual.get_untracked()
                    && self.find_focus.get_untracked()
            }
            Condition::ListFocus => self.has_completions(),
            Condition::CompletionFocus => self.has_completions(),
            Condition::InlineCompletionVisible => self.has_inline_completions(),
            Condition::OnScreenFindActive => {
                self.on_screen_find.with_untracked(|f| f.active)
            }
            Condition::InSnippet => self.snippet.with_untracked(|s| s.is_some()),
            Condition::EditorFocus => self
                .doc()
                .content
                .with_untracked(|content| !content.is_local()),
            Condition::SearchFocus => {
                self.common.find.visual.get_untracked()
                    && self.find_focus.get_untracked()
                    && !self.common.find.replace_focus.get_untracked()
            }
            Condition::ReplaceFocus => {
                self.common.find.visual.get_untracked()
                    && self.find_focus.get_untracked()
                    && self.common.find.replace_focus.get_untracked()
            }
            Condition::SearchActive => {
                if self.common.config.get_untracked().core.modal
                    && self.cursor().with_untracked(|c| !c.is_normal())
                {
                    false
                } else {
                    self.common.find.visual.get_untracked()
                }
            }
            _ => false,
        }
    }

    #[instrument]
    fn run_command(
        &self,
        command: &crate::command::LapceCommand,
        count: Option<usize>,
        mods: Modifiers,
    ) -> CommandExecuted {
        if self.common.find.visual.get_untracked() && self.find_focus.get_untracked()
        {
            match &command.kind {
                CommandKind::Edit(_)
                | CommandKind::Move(_)
                | CommandKind::MultiSelection(_) => {
                    if self.common.find.replace_focus.get_untracked() {
                        self.common.internal_command.send(
                            InternalCommand::ReplaceEditorCommand {
                                command: command.clone(),
                                count,
                                mods,
                            },
                        );
                    } else {
                        self.common.internal_command.send(
                            InternalCommand::FindEditorCommand {
                                command: command.clone(),
                                count,
                                mods,
                            },
                        );
                    }
                    return CommandExecuted::Yes;
                }
                _ => {}
            }
        }

        match &command.kind {
            crate::command::CommandKind::Workbench(_) => CommandExecuted::No,
            crate::command::CommandKind::Edit(cmd) => self.run_edit_command(cmd),
            crate::command::CommandKind::Move(cmd) => {
                let movement = cmd.to_movement(count);
                self.run_move_command(&movement, count, mods)
            }
            crate::command::CommandKind::Scroll(cmd) => {
                if self
                    .doc()
                    .content
                    .with_untracked(|content| content.is_local())
                {
                    return CommandExecuted::No;
                }
                self.run_scroll_command(cmd, count, mods)
            }
            crate::command::CommandKind::Focus(cmd) => {
                if self
                    .doc()
                    .content
                    .with_untracked(|content| content.is_local())
                {
                    return CommandExecuted::No;
                }
                self.run_focus_command(cmd, count, mods)
            }
            crate::command::CommandKind::MotionMode(cmd) => {
                self.run_motion_mode_command(cmd, count)
            }
            crate::command::CommandKind::MultiSelection(cmd) => {
                self.run_multi_selection_command(cmd)
            }
        }
    }

    fn expect_char(&self) -> bool {
        if self.common.find.visual.get_untracked() && self.find_focus.get_untracked()
        {
            false
        } else {
            self.inline_find.with_untracked(|f| f.is_some())
                || self.on_screen_find.with_untracked(|f| f.active)
        }
    }

    fn receive_char(&self, c: &str) {
        if self.common.find.visual.get_untracked() && self.find_focus.get_untracked()
        {
            // find/relace editor receive char
            if self.common.find.replace_focus.get_untracked() {
                self.common.internal_command.send(
                    InternalCommand::ReplaceEditorReceiveChar { s: c.to_string() },
                );
            } else {
                self.common.internal_command.send(
                    InternalCommand::FindEditorReceiveChar { s: c.to_string() },
                );
            }
        } else {
            // normal editor receive char
            if self.get_mode() == Mode::Insert {
                let mut cursor = self.cursor().get_untracked();
                let deltas = self.doc().do_insert(
                    &mut cursor,
                    c,
                    &self.common.config.get_untracked(),
                );
                self.cursor().set(cursor);

                if !c
                    .chars()
                    .all(|c| c.is_whitespace() || c.is_ascii_whitespace())
                {
                    self.update_completion(false);
                } else {
                    self.cancel_completion();
                }

                self.update_inline_completion(
                    InlineCompletionTriggerKind::Automatic,
                );

                self.apply_deltas(&deltas);
            } else if let Some(direction) = self.inline_find.get_untracked() {
                self.inline_find(direction.clone(), c);
                self.last_inline_find.set(Some((direction, c.to_string())));
                self.inline_find.set(None);
            } else if self.on_screen_find.with_untracked(|f| f.active) {
                self.on_screen_find.update(|find| {
                    let pattern = format!("{}{c}", find.pattern);
                    find.regions = self.on_screen_find(&pattern);
                    find.pattern = pattern;
                });
            }
        }
    }
}

/// Checks if completion should be triggered if the received command
/// is one that inserts whitespace or deletes whitespace
fn show_completion(
    cmd: &EditCommand,
    doc: &Rope,
    deltas: &[(Rope, RopeDelta, InvalLines)],
) -> bool {
    match cmd {
        EditCommand::DeleteBackward
        | EditCommand::DeleteForward
        | EditCommand::DeleteWordBackward
        | EditCommand::DeleteWordForward
        | EditCommand::DeleteForwardAndInsert => {
            let start = match deltas.first().and_then(|delta| delta.1.els.first()) {
                Some(lapce_xi_rope::DeltaElement::Copy(_, start)) => *start,
                _ => 0,
            };

            let end = match deltas.first().and_then(|delta| delta.1.els.get(1)) {
                Some(lapce_xi_rope::DeltaElement::Copy(end, _)) => *end,
                _ => 0,
            };

            if start > 0 && end > start {
                !doc.slice_to_cow(start..end)
                    .chars()
                    .all(|c| c.is_whitespace() || c.is_ascii_whitespace())
            } else {
                true
            }
        }
        _ => false,
    }
}

fn show_inline_completion(cmd: &EditCommand) -> bool {
    matches!(
        cmd,
        EditCommand::DeleteBackward
            | EditCommand::DeleteForward
            | EditCommand::DeleteWordBackward
            | EditCommand::DeleteWordForward
            | EditCommand::DeleteForwardAndInsert
            | EditCommand::IndentLine
            | EditCommand::InsertMode
    )
}
