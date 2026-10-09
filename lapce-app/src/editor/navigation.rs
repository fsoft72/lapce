//! Navigation of the editor: inline and on-screen find, go to definition, references, implementations and locations.

use std::{collections::HashSet, rc::Rc, sync::Arc};

use floem::{
    ViewId,
    ext_event::create_ext_action,
    keyboard::Modifiers,
    kurbo::Vec2,
    reactive::{SignalGet, SignalUpdate, SignalWith, use_context},
};
use lapce_core::{
    buffer::rope_text::RopeText,
    cursor::{Cursor, CursorMode},
    rope_text_pos::RopeTextPosition,
    selection::{SelRegion, Selection},
};
use lapce_rpc::proxy::ProxyResponse;
use lsp_types::{GotoDefinitionResponse, Location, Range, TextEdit};
use nucleo::Utf32Str;

use super::location::{EditorLocation, EditorPosition};
use super::{EditorData, InlineFindDirection};
use crate::{
    command::InternalCommand,
    db::LapceDb,
    lsp::path_from_url,
    panel::{
        call_hierarchy_view::CallHierarchyItemData,
        implementation_view::{init_implementation_root, map_to_location},
        kind::PanelKind,
    },
    window_tab::WindowTabData,
};

impl EditorData {
    /// Jump to the next/previous column on the line which matches the given text
    pub(super) fn inline_find(&self, direction: InlineFindDirection, c: &str) {
        let offset = self.cursor().with_untracked(|c| c.offset());
        let doc = self.doc();
        let (line_content, line_start_offset) =
            doc.buffer.with_untracked(|buffer| {
                let line = buffer.line_of_offset(offset);
                let line_content = buffer.line_content(line);
                let line_start_offset = buffer.offset_of_line(line);
                (line_content.to_string(), line_start_offset)
            });
        let index = offset - line_start_offset;
        if let Some(new_index) = match direction {
            InlineFindDirection::Left => line_content[..index].rfind(c),
            InlineFindDirection::Right => {
                if index + 1 >= line_content.len() {
                    None
                } else {
                    let index = index
                        + doc.buffer.with_untracked(|buffer| {
                            buffer.next_grapheme_offset(
                                offset,
                                1,
                                buffer.offset_line_end(offset, false),
                            )
                        })
                        - offset;
                    line_content[index..].find(c).map(|i| i + index)
                }
            }
        } {
            self.run_move_command(
                &lapce_core::movement::Movement::Offset(
                    new_index + line_start_offset,
                ),
                None,
                Modifiers::empty(),
            );
        }
    }

    pub(super) fn quit_on_screen_find(&self) {
        if self.on_screen_find.with_untracked(|s| s.active) {
            self.on_screen_find.update(|f| {
                f.active = false;
                f.pattern.clear();
                f.regions.clear();
            })
        }
    }

    pub(super) fn on_screen_find(&self, pattern: &str) -> Vec<SelRegion> {
        let screen_lines = self.screen_lines().get_untracked();
        let lines: HashSet<usize> =
            screen_lines.lines.iter().map(|l| l.line).collect();

        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let pattern = nucleo::pattern::Pattern::parse(
            pattern,
            nucleo::pattern::CaseMatching::Ignore,
            nucleo::pattern::Normalization::Smart,
        );
        let mut indices = Vec::new();
        let mut filter_text_buf = Vec::new();
        let mut items = Vec::new();

        let buffer = self.doc().buffer;

        for line in lines {
            filter_text_buf.clear();
            indices.clear();

            buffer.with_untracked(|buffer| {
                let start = buffer.offset_of_line(line);
                let end = buffer.offset_of_line(line + 1);
                let text = buffer.text().slice_to_cow(start..end);
                let filter_text = Utf32Str::new(&text, &mut filter_text_buf);

                if let Some(score) =
                    pattern.indices(filter_text, &mut matcher, &mut indices)
                {
                    indices.sort();
                    let left =
                        start + indices.first().copied().unwrap_or(0) as usize;
                    let right =
                        start + indices.last().copied().unwrap_or(0) as usize + 1;
                    let right = if right == left { left + 1 } else { right };
                    items.push((score, left, right));
                }
            });
        }

        items.sort_by_key(|(score, _, _)| -(*score as i64));
        if let Some((_, offset, _)) = items.first().copied() {
            self.run_move_command(
                &lapce_core::movement::Movement::Offset(offset),
                None,
                Modifiers::empty(),
            );
        }

        items
            .into_iter()
            .map(|(_, start, end)| SelRegion::new(start, end, None))
            .collect()
    }

    pub(super) fn go_to_definition(&self) {
        let doc = self.doc();
        let path = match if doc.loaded() {
            doc.content.with_untracked(|c| c.path().cloned())
        } else {
            None
        } {
            Some(path) => path,
            None => return,
        };

        let offset = self.cursor().with_untracked(|c| c.offset());
        let (start_position, position) = doc.buffer.with_untracked(|buffer| {
            let start_offset = buffer.prev_code_boundary(offset);
            let start_position = buffer.offset_to_position(start_offset);
            let position = buffer.offset_to_position(offset);
            (start_position, position)
        });

        enum DefinitionOrReference {
            Location(EditorLocation),
            References(Vec<Location>),
        }

        let internal_command = self.common.internal_command;
        let cursor = self.cursor().read_only();
        let send = create_ext_action(self.scope, move |d| {
            let current_offset = cursor.with_untracked(|c| c.offset());
            if current_offset != offset {
                return;
            }

            match d {
                DefinitionOrReference::Location(location) => {
                    internal_command
                        .send(InternalCommand::JumpToLocation { location });
                }
                DefinitionOrReference::References(locations) => {
                    internal_command.send(InternalCommand::PaletteReferences {
                        references: locations
                            .into_iter()
                            .map(|l| EditorLocation {
                                path: path_from_url(&l.uri),
                                position: Some(EditorPosition::Position(
                                    l.range.start,
                                )),
                                scroll_offset: None,
                                ignore_unconfirmed: false,
                                same_editor_tab: false,
                            })
                            .collect(),
                    });
                }
            }
        });
        let proxy = self.common.proxy.clone();
        self.common.proxy.get_definition(
            offset,
            path.clone(),
            position,
            move |result| {
                if let Ok(ProxyResponse::GetDefinitionResponse {
                    definition, ..
                }) = result
                {
                    if let Some(location) = match definition {
                        GotoDefinitionResponse::Scalar(location) => Some(location),
                        GotoDefinitionResponse::Array(locations) => {
                            if !locations.is_empty() {
                                Some(locations[0].clone())
                            } else {
                                None
                            }
                        }
                        GotoDefinitionResponse::Link(location_links) => {
                            let location_link = location_links[0].clone();
                            Some(Location {
                                uri: location_link.target_uri,
                                range: location_link.target_selection_range,
                            })
                        }
                    } {
                        if location.range.start == start_position {
                            proxy.get_references(
                                path.clone(),
                                position,
                                move |result| {
                                    if let Ok(
                                        ProxyResponse::GetReferencesResponse {
                                            references,
                                        },
                                    ) = result
                                    {
                                        if references.is_empty() {
                                            return;
                                        }
                                        if references.len() == 1 {
                                            let location = &references[0];
                                            send(DefinitionOrReference::Location(
                                                EditorLocation {
                                                    path: path_from_url(
                                                        &location.uri,
                                                    ),
                                                    position: Some(
                                                        EditorPosition::Position(
                                                            location.range.start,
                                                        ),
                                                    ),
                                                    scroll_offset: None,
                                                    ignore_unconfirmed: false,
                                                    same_editor_tab: false,
                                                },
                                            ));
                                        } else {
                                            send(DefinitionOrReference::References(
                                                references,
                                            ));
                                        }
                                    }
                                },
                            );
                        } else {
                            let path = path_from_url(&location.uri);
                            send(DefinitionOrReference::Location(EditorLocation {
                                path,
                                position: Some(EditorPosition::Position(
                                    location.range.start,
                                )),
                                scroll_offset: None,
                                ignore_unconfirmed: false,
                                same_editor_tab: false,
                            }));
                        }
                    }
                }
            },
        );
    }

    pub fn call_hierarchy(&self, window_tab_data: WindowTabData) {
        let doc = self.doc();
        let path = match if doc.loaded() {
            doc.content.with_untracked(|c| c.path().cloned())
        } else {
            None
        } {
            Some(path) => path,
            None => return,
        };

        let offset = self.cursor().with_untracked(|c| c.offset());
        let (_start_position, position) = doc.buffer.with_untracked(|buffer| {
            let start_offset = buffer.prev_code_boundary(offset);
            let start_position = buffer.offset_to_position(start_offset);
            let position = buffer.offset_to_position(offset);
            (start_position, position)
        });
        let scope = window_tab_data.scope;
        let range = Range {
            start: _start_position,
            end: position,
        };
        self.common.proxy.show_call_hierarchy(
            path,
            position,
            create_ext_action(self.scope, move |result| {
                if let Ok(ProxyResponse::ShowCallHierarchyResponse {
                    items, ..
                }) = result
                {
                    if let Some(item) = items.and_then(|x| x.into_iter().next()) {
                        let root = scope.create_rw_signal(CallHierarchyItemData {
                            view_id: ViewId::new(),
                            item: Rc::new(item),
                            from_range: range,
                            init: false,
                            open: scope.create_rw_signal(true),
                            children: scope.create_rw_signal(Vec::with_capacity(0)),
                        });
                        let item = root;
                        window_tab_data.call_hierarchy_data.root.update(|x| {
                            *x = Some(root);
                        });
                        window_tab_data.show_panel(PanelKind::CallHierarchy);
                        window_tab_data.common.internal_command.send(
                            InternalCommand::CallHierarchyIncoming {
                                item_id: item.get_untracked().view_id,
                            },
                        );
                    }
                }
            }),
        );
    }

    pub fn find_reference(&self, window_tab_data: WindowTabData) {
        let doc = self.doc();
        let path = match if doc.loaded() {
            doc.content.with_untracked(|c| c.path().cloned())
        } else {
            None
        } {
            Some(path) => path,
            None => return,
        };

        let offset = self.cursor().with_untracked(|c| c.offset());
        let (_start_position, position) = doc.buffer.with_untracked(|buffer| {
            let start_offset = buffer.prev_code_boundary(offset);
            let start_position = buffer.offset_to_position(start_offset);
            let position = buffer.offset_to_position(offset);
            (start_position, position)
        });
        let scope = window_tab_data.scope;
        let update_implementation = create_ext_action(self.scope, {
            let window_tab_data = window_tab_data.clone();
            move |result| {
                if let Ok(ProxyResponse::ReferencesResolveResponse { items }) =
                    result
                {
                    window_tab_data
                        .main_split
                        .references
                        .update(|x| *x = init_implementation_root(items, scope));
                    window_tab_data.show_panel(PanelKind::References);
                }
            }
        });
        let proxy = self.common.proxy.clone();
        self.common.proxy.get_references(
            path,
            position,
            create_ext_action(self.scope, move |result| {
                if let Ok(ProxyResponse::GetReferencesResponse { references }) =
                    result
                {
                    {
                        if !references.is_empty() {
                            proxy.references_resolve(
                                references,
                                update_implementation,
                            );
                        } else {
                            window_tab_data.show_panel(PanelKind::References);
                        }
                    }
                }
            }),
        );
    }

    pub fn go_to_implementation(&self, window_tab_data: WindowTabData) {
        let doc = self.doc();
        let path = match if doc.loaded() {
            doc.content.with_untracked(|c| c.path().cloned())
        } else {
            None
        } {
            Some(path) => path,
            None => return,
        };

        let offset = self.cursor().with_untracked(|c| c.offset());
        let (_start_position, position) = doc.buffer.with_untracked(|buffer| {
            let start_offset = buffer.prev_code_boundary(offset);
            let start_position = buffer.offset_to_position(start_offset);
            let position = buffer.offset_to_position(offset);
            (start_position, position)
        });
        let scope = window_tab_data.scope;
        let update_implementation = create_ext_action(self.scope, {
            let window_tab_data = window_tab_data.clone();
            move |result| {
                if let Ok(ProxyResponse::ReferencesResolveResponse { items }) =
                    result
                {
                    window_tab_data
                        .main_split
                        .implementations
                        .update(|x| *x = init_implementation_root(items, scope));
                    window_tab_data.show_panel(PanelKind::Implementation);
                }
            }
        });
        let proxy = self.common.proxy.clone();
        self.common.proxy.go_to_implementation(
            path,
            position,
            create_ext_action(self.scope, {
                move |result| {
                    if let Ok(ProxyResponse::GotoImplementationResponse {
                        resp,
                        ..
                    }) = result
                    {
                        let locations = map_to_location(resp);
                        if !locations.is_empty() {
                            proxy.references_resolve(
                                locations,
                                update_implementation,
                            );
                        } else {
                            window_tab_data.show_panel(PanelKind::Implementation);
                        }
                    }
                }
            }),
        );
    }
}

impl EditorData {
    fn do_go_to_location(
        &self,
        location: EditorLocation,
        edits: Option<Vec<TextEdit>>,
    ) {
        if let Some(position) = location.position {
            self.go_to_position(position, location.scroll_offset, edits);
        } else if let Some(edits) = edits.as_ref() {
            self.do_text_edit(edits);
        } else {
            let db: Arc<LapceDb> = use_context().unwrap();
            if let Ok(info) = db.get_doc_info(&self.common.workspace, &location.path)
            {
                self.go_to_position(
                    EditorPosition::Offset(info.cursor_offset),
                    Some(Vec2::new(info.scroll_offset.0, info.scroll_offset.1)),
                    edits,
                );
            }
        }
    }

    pub fn go_to_location(
        &self,
        location: EditorLocation,
        new_doc: bool,
        edits: Option<Vec<TextEdit>>,
    ) {
        if !new_doc {
            self.do_go_to_location(location, edits);
        } else {
            let loaded = self.doc().loaded;
            let editor = self.clone();
            self.scope.create_effect(move |prev_loaded| {
                if prev_loaded == Some(true) {
                    return true;
                }

                let loaded = loaded.get();
                if loaded {
                    editor.do_go_to_location(location.clone(), edits.clone());
                }
                loaded
            });
        }
    }

    pub fn go_to_position(
        &self,
        position: EditorPosition,
        scroll_offset: Option<Vec2>,
        edits: Option<Vec<TextEdit>>,
    ) {
        let offset = self
            .doc()
            .buffer
            .with_untracked(|buffer| position.to_offset(buffer));
        let config = self.common.config.get_untracked();
        self.cursor().set(if config.core.modal {
            Cursor::new(CursorMode::Normal(offset), None, None)
        } else {
            Cursor::new(CursorMode::Insert(Selection::caret(offset)), None, None)
        });
        if let Some(scroll_offset) = scroll_offset {
            self.editor.scroll_to.set(Some(scroll_offset));
        }
        if let Some(edits) = edits.as_ref() {
            self.do_text_edit(edits);
        }
    }
}
