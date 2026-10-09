//! Pointer handling of the editor: clicks, drags, context menu, hover and inlay hint links.

use std::time::Duration;

use floem::{
    action::{TimerToken, exec_after, show_context_menu},
    ext_event::create_ext_action,
    kurbo::Point,
    menu::{Menu, MenuItem},
    pointer::{MouseButton, PointerInputEvent, PointerMoveEvent},
    reactive::{SignalGet, SignalUpdate, SignalWith},
};
use itertools::Itertools;
use lapce_core::{
    buffer::rope_text::RopeText,
    command::{EditCommand, FocusCommand},
    rope_text_pos::RopeTextPosition,
};
use lapce_rpc::proxy::ProxyResponse;
use lsp_types::{
    HoverContents, InlayHint, InlayHintLabel, Location, MarkedString, MarkupKind,
};

use super::EditorData;
use super::location::{EditorLocation, EditorPosition};
use crate::tracing::*;
use crate::{
    command::{CommandKind, InternalCommand, LapceCommand, LapceWorkbenchCommand},
    config::LapceConfig,
    doc::DocContent,
    markdown::{
        MarkdownContent, from_marked_string, from_plaintext, parse_markdown,
    },
    window_tab::Focus,
};

impl EditorData {
    pub fn pointer_down(&self, pointer_event: &PointerInputEvent) {
        self.cancel_completion();
        self.cancel_inline_completion();
        if let Some(editor_tab_id) = self.editor_tab_id.get_untracked() {
            self.common
                .internal_command
                .send(InternalCommand::FocusEditorTab { editor_tab_id });
        }
        if self
            .doc()
            .content
            .with_untracked(|content| !content.is_local())
        {
            self.common.focus.set(Focus::Workbench);
            self.find_focus.set(false);
        }
        match pointer_event.button.mouse_button() {
            MouseButton::Primary => {
                self.active().set(true);
                self.left_click(pointer_event);

                let y =
                    pointer_event.pos.y - self.editor.viewport.get_untracked().y0;
                if self.sticky_header_height.get_untracked() > y {
                    let index = y as usize
                        / self.common.config.get_untracked().editor.line_height();
                    if let (Some(path), Some(line)) = (
                        self.doc().content.get_untracked().path(),
                        self.sticky_header_info
                            .get_untracked()
                            .sticky_lines
                            .get(index),
                    ) {
                        self.common.internal_command.send(
                            InternalCommand::JumpToLocation {
                                location: EditorLocation {
                                    path: path.clone(),
                                    position: Some(EditorPosition::Line(*line)),
                                    scroll_offset: None,
                                    ignore_unconfirmed: true,
                                    same_editor_tab: false,
                                },
                            },
                        );
                        return;
                    }
                }

                if (cfg!(target_os = "macos") && pointer_event.modifiers.meta())
                    || (cfg!(not(target_os = "macos"))
                        && pointer_event.modifiers.control())
                {
                    let rs = self.find_hint(pointer_event.pos);
                    match rs {
                        FindHintRs::NoMatchBreak
                        | FindHintRs::NoMatchContinue { .. } => {
                            self.common.lapce_command.send(LapceCommand {
                                kind: CommandKind::Focus(
                                    FocusCommand::GotoDefinition,
                                ),
                                data: None,
                            })
                        }
                        FindHintRs::MatchWithoutLocation => {}
                        FindHintRs::Match(location) => {
                            let Ok(path) = location.uri.to_file_path() else {
                                return;
                            };
                            self.common.internal_command.send(
                                InternalCommand::JumpToLocation {
                                    location: EditorLocation {
                                        path,
                                        position: Some(EditorPosition::Position(
                                            location.range.start,
                                        )),
                                        scroll_offset: None,
                                        ignore_unconfirmed: true,
                                        same_editor_tab: false,
                                    },
                                },
                            );
                        }
                    }
                }
            }
            MouseButton::Secondary => {
                self.right_click(pointer_event);
            }
            _ => {}
        }
    }

    fn find_hint(&self, pos: Point) -> FindHintRs {
        let rs = self.editor.line_col_of_point_with_phantom(pos);
        let line = rs.0 as u32;
        let index = rs.1 as u32;
        self.doc().inlay_hints.with_untracked(|x| {
            if let Some(hints) = x {
                let mut pre_len = 0;
                for hint in hints
                    .iter()
                    .filter_map(|(_, hint)| {
                        if hint.position.line == line {
                            Some(hint)
                        } else {
                            None
                        }
                    })
                    .sorted_by(|pre, next| {
                        pre.position.character.cmp(&next.position.character)
                    })
                {
                    match find_hint(pre_len, index, hint) {
                        FindHintRs::NoMatchContinue { pre_hint_len } => {
                            pre_len = pre_hint_len;
                        }
                        rs => return rs,
                    }
                }
                FindHintRs::NoMatchBreak
            } else {
                FindHintRs::NoMatchBreak
            }
        })
    }

    #[instrument]
    fn left_click(&self, pointer_event: &PointerInputEvent) {
        match pointer_event.count {
            1 => {
                self.single_click(pointer_event);
            }
            2 => {
                self.double_click(pointer_event);
            }
            3 => {
                self.triple_click(pointer_event);
            }
            _ => {}
        }
    }

    #[instrument]
    fn single_click(&self, pointer_event: &PointerInputEvent) {
        self.editor.single_click(pointer_event);
    }

    #[instrument]
    fn double_click(&self, pointer_event: &PointerInputEvent) {
        self.editor.double_click(pointer_event);
    }

    #[instrument]
    fn triple_click(&self, pointer_event: &PointerInputEvent) {
        self.editor.triple_click(pointer_event);
    }

    #[instrument]
    pub fn pointer_move(&self, pointer_event: &PointerMoveEvent) {
        let mode = self.cursor().with_untracked(|c| c.get_mode());
        let (offset, is_inside) =
            self.editor.offset_of_point(mode, pointer_event.pos);
        if self.active().get_untracked()
            && self.cursor().with_untracked(|c| c.offset()) != offset
        {
            self.cursor().update(|cursor| {
                cursor.set_offset(offset, true, pointer_event.modifiers.alt())
            });
        }
        if self.common.hover.active.get_untracked() {
            let hover_editor_id = self.common.hover.editor_id.get_untracked();
            if hover_editor_id != self.id() {
                self.common.hover.active.set(false);
            } else {
                let current_offset = self.common.hover.offset.get_untracked();
                let start_offset = self
                    .doc()
                    .buffer
                    .with_untracked(|buffer| buffer.prev_code_boundary(offset));
                if current_offset != start_offset {
                    self.common.hover.active.set(false);
                }
            }
        }
        let hover_delay = self.common.config.get_untracked().editor.hover_delay;
        if hover_delay > 0 {
            if is_inside {
                let start_offset = self
                    .doc()
                    .buffer
                    .with_untracked(|buffer| buffer.prev_code_boundary(offset));

                let editor = self.clone();
                let mouse_hover_timer = self.common.mouse_hover_timer;
                let timer_token =
                    exec_after(Duration::from_millis(hover_delay), move |token| {
                        if mouse_hover_timer.try_get_untracked() == Some(token)
                            && editor.editor_tab_id.try_get_untracked().is_some()
                        {
                            editor.update_hover(start_offset);
                        }
                    });
                mouse_hover_timer.set(timer_token);
            } else {
                self.common.mouse_hover_timer.set(TimerToken::INVALID);
            }
        }
    }

    #[instrument]
    pub fn pointer_up(&self, pointer_event: &PointerInputEvent) {
        self.editor.pointer_up(pointer_event);
    }

    #[instrument]
    pub fn pointer_leave(&self) {
        self.common.mouse_hover_timer.set(TimerToken::INVALID);
    }

    #[instrument]
    fn right_click(&self, pointer_event: &PointerInputEvent) {
        let mode = self.cursor().with_untracked(|c| c.get_mode());
        let (offset, _) = self.editor.offset_of_point(mode, pointer_event.pos);
        let doc = self.doc();
        let pointer_inside_selection = doc.buffer.with_untracked(|buffer| {
            self.cursor()
                .with_untracked(|c| c.edit_selection(buffer).contains(offset))
        });
        if !pointer_inside_selection {
            // move cursor to pointer position if outside current selection
            self.single_click(pointer_event);
        }

        let (path, is_file) = doc.content.with_untracked(|content| match content {
            DocContent::File { path, .. } => {
                (Some(path.to_path_buf()), path.is_file())
            }
            DocContent::Local
            | DocContent::History(_)
            | DocContent::Scratch { .. } => (None, false),
        });
        let mut menu = Menu::new("");
        let mut cmds = if is_file {
            if path
                .as_ref()
                .and_then(|x| x.file_name().and_then(|x| x.to_str()))
                .map(|x| x == "run.toml")
                .unwrap_or_default()
            {
                vec![
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::RevealInPanel,
                    )),
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::RevealInFileExplorer,
                    )),
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::SourceControlOpenActiveFileRemoteUrl,
                    )),
                    None,
                    Some(CommandKind::Edit(EditCommand::ClipboardCut)),
                    Some(CommandKind::Edit(EditCommand::ClipboardCopy)),
                    Some(CommandKind::Edit(EditCommand::ClipboardPaste)),
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::AddRunDebugConfig,
                    )),
                    None,
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::PaletteCommand,
                    )),
                ]
            } else {
                vec![
                    Some(CommandKind::Focus(FocusCommand::GotoDefinition)),
                    Some(CommandKind::Focus(FocusCommand::GotoTypeDefinition)),
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::ShowCallHierarchy,
                    )),
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::FindReferences,
                    )),
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::GoToImplementation,
                    )),
                    Some(CommandKind::Focus(FocusCommand::Rename)),
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::RunInTerminal,
                    )),
                    None,
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::RevealInPanel,
                    )),
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::RevealInFileExplorer,
                    )),
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::SourceControlOpenActiveFileRemoteUrl,
                    )),
                    None,
                    Some(CommandKind::Edit(EditCommand::ClipboardCut)),
                    Some(CommandKind::Edit(EditCommand::ClipboardCopy)),
                    Some(CommandKind::Edit(EditCommand::ClipboardPaste)),
                    None,
                    Some(CommandKind::Workbench(
                        LapceWorkbenchCommand::PaletteCommand,
                    )),
                ]
            }
        } else {
            vec![
                Some(CommandKind::Edit(EditCommand::ClipboardCut)),
                Some(CommandKind::Edit(EditCommand::ClipboardCopy)),
                Some(CommandKind::Edit(EditCommand::ClipboardPaste)),
                None,
                Some(CommandKind::Workbench(
                    LapceWorkbenchCommand::PaletteCommand,
                )),
            ]
        };
        if self.diff_editor_id.get_untracked().is_some() && is_file {
            cmds.push(Some(CommandKind::Workbench(
                LapceWorkbenchCommand::GoToLocation,
            )));
        }
        let lapce_command = self.common.lapce_command;
        for cmd in cmds {
            if let Some(cmd) = cmd {
                menu = menu.entry(
                    MenuItem::new(cmd.desc().unwrap_or_else(|| cmd.str())).action(
                        move || {
                            lapce_command.send(LapceCommand {
                                kind: cmd.clone(),
                                data: None,
                            })
                        },
                    ),
                );
            } else {
                menu = menu.separator();
            }
        }
        show_context_menu(menu, None);
    }

    #[instrument]
    pub(super) fn update_hover(&self, offset: usize) {
        let doc = self.doc();
        let path = doc
            .content
            .with_untracked(|content| content.path().cloned());
        let position = doc
            .buffer
            .with_untracked(|buffer| buffer.offset_to_position(offset));
        let path = match path {
            Some(path) => path,
            None => return,
        };
        let config = self.common.config;
        let hover_data = self.common.hover.clone();
        let editor_id = self.id();
        let send = create_ext_action(self.scope, move |resp| {
            if let Ok(ProxyResponse::HoverResponse { hover, .. }) = resp {
                let content = parse_hover_resp(hover, &config.get_untracked());
                hover_data.content.set(content);
                hover_data.offset.set(offset);
                hover_data.editor_id.set(editor_id);
                hover_data.active.set(true);
            }
        });
        self.common.proxy.get_hover(0, path, position, |resp| {
            send(resp);
        });
    }
}

fn parse_hover_resp(
    hover: lsp_types::Hover,
    config: &LapceConfig,
) -> Vec<MarkdownContent> {
    match hover.contents {
        HoverContents::Scalar(text) => match text {
            MarkedString::String(text) => parse_markdown(&text, 1.8, config),
            MarkedString::LanguageString(code) => parse_markdown(
                &format!("```{}\n{}\n```", code.language, code.value),
                1.8,
                config,
            ),
        },
        HoverContents::Array(array) => array
            .into_iter()
            .map(|t| from_marked_string(t, config))
            .rev()
            .reduce(|mut contents, more| {
                contents.push(MarkdownContent::Separator);
                contents.extend(more);
                contents
            })
            .unwrap_or_default(),
        HoverContents::Markup(content) => match content.kind {
            MarkupKind::PlainText => from_plaintext(&content.value, 1.8, config),
            MarkupKind::Markdown => parse_markdown(&content.value, 1.8, config),
        },
    }
}

#[derive(Debug)]
enum FindHintRs {
    NoMatchBreak,
    NoMatchContinue { pre_hint_len: u32 },
    MatchWithoutLocation,
    Match(Location),
}

fn find_hint(
    mut pre_hint_len: u32,
    index: u32,
    hint: &InlayHint,
) -> FindHintRs {
    use FindHintRs::*;
    match &hint.label {
        InlayHintLabel::String(text) => {
            let actual_col = pre_hint_len + hint.position.character;
            let actual_col_end = actual_col + (text.len() as u32);
            if actual_col > index {
                NoMatchBreak
            } else if actual_col <= index && index < actual_col_end {
                MatchWithoutLocation
            } else {
                pre_hint_len += text.len() as u32;
                NoMatchContinue { pre_hint_len }
            }
        }
        InlayHintLabel::LabelParts(parts) => {
            for part in parts {
                let actual_col = pre_hint_len + hint.position.character;
                let actual_col_end = actual_col + part.value.len() as u32;
                if index < actual_col {
                    return NoMatchBreak;
                } else if actual_col <= index && index < actual_col_end {
                    if let Some(location) = &part.location {
                        return Match(location.clone());
                    } else {
                        return MatchWithoutLocation;
                    }
                } else {
                    pre_hint_len += part.value.len() as u32;
                }
            }
            NoMatchContinue { pre_hint_len }
        }
    }
}
