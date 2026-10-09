use std::{rc::Rc, sync::Arc};

use floem::{
    ext_event::create_ext_action,
    keyboard::Modifiers,
    kurbo::{Point, Rect, Vec2},
    prelude::SignalTrack,
    reactive::{RwSignal, Scope, SignalGet, SignalUpdate, SignalWith, use_context},
    views::editor::{Editor, id::EditorId, text::Document, view::ScreenLines},
};
use lapce_core::{
    buffer::{
        InvalLines,
        diff::DiffLines,
        rope_text::{RopeText, RopeTextVal},
    },
    command::EditCommand,
    cursor::Cursor,
    editor::EditType,
    rope_text_pos::RopeTextPosition,
    selection::{InsertDrift, SelRegion, Selection},
};
use lapce_rpc::{buffer::BufferId, plugin::PluginId, proxy::ProxyResponse};
use lapce_xi_rope::{Rope, RopeDelta, Transformer};
use lsp_types::{CodeActionResponse, TextEdit};
use serde::{Deserialize, Serialize};
use view::StickyHeaderInfo;

use self::{
    diff::DiffInfo,
    location::{EditorLocation, EditorPosition},
};
use crate::{
    command::InternalCommand,
    db::LapceDb,
    doc::{Doc, DocContent},
    id::{DiffEditorId, EditorTabId},
    main_split::{Editors, MainSplitData},
    tracing::*,
    window_tab::{CommonData, WindowTabData},
};

mod commands;
mod completion;
pub mod diff;
pub mod gutter;
pub mod location;
mod navigation;
mod pointer;
mod screen_lines;
mod search;
pub mod view;

pub(crate) use self::screen_lines::compute_screen_lines;

#[derive(Clone, Debug)]
pub enum InlineFindDirection {
    Left,
    Right,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct EditorInfo {
    pub content: DocContent,
    pub unsaved: Option<String>,
    pub offset: usize,
    pub scroll_offset: (f64, f64),
}

impl EditorInfo {
    pub fn to_data(
        &self,
        data: MainSplitData,
        editor_tab_id: EditorTabId,
    ) -> EditorId {
        let editors = &data.editors;
        let common = data.common.clone();
        match &self.content {
            DocContent::File { path, .. } => {
                let (doc, new_doc) =
                    data.get_doc(path.clone(), self.unsaved.clone());
                let editor = editors.make_from_doc(
                    data.scope,
                    doc,
                    Some(editor_tab_id),
                    None,
                    None,
                    common,
                );
                editor.go_to_location(
                    EditorLocation {
                        path: path.clone(),
                        position: Some(EditorPosition::Offset(self.offset)),
                        scroll_offset: Some(Vec2::new(
                            self.scroll_offset.0,
                            self.scroll_offset.1,
                        )),
                        ignore_unconfirmed: false,
                        same_editor_tab: false,
                    },
                    new_doc,
                    None,
                );

                editor.id()
            }
            DocContent::Local => editors.new_local(data.scope, common),
            DocContent::History(_) => editors.new_local(data.scope, common),
            DocContent::Scratch { name, .. } => {
                let doc = data
                    .scratch_docs
                    .try_update(|scratch_docs| {
                        if let Some(doc) = scratch_docs.get(name) {
                            return doc.clone();
                        }
                        let content = DocContent::Scratch {
                            id: BufferId::next(),
                            name: name.to_string(),
                        };
                        let doc = Doc::new_content(
                            data.scope,
                            content,
                            data.editors,
                            data.common.clone(),
                        );
                        let doc = Rc::new(doc);
                        if let Some(unsaved) = &self.unsaved {
                            doc.reload(Rope::from(unsaved), false);
                        }
                        scratch_docs.insert(name.to_string(), doc.clone());
                        doc
                    })
                    .unwrap();

                editors.new_from_doc(
                    data.scope,
                    doc,
                    Some(editor_tab_id),
                    None,
                    None,
                    common,
                )
            }
        }
    }
}

#[derive(Clone)]
pub enum EditorViewKind {
    Normal,
    Diff(DiffInfo),
}

impl EditorViewKind {
    pub fn is_normal(&self) -> bool {
        matches!(self, EditorViewKind::Normal)
    }
}

#[derive(Clone)]
pub struct OnScreenFind {
    pub active: bool,
    pub pattern: String,
    pub regions: Vec<SelRegion>,
}

pub type SnippetIndex = Vec<(usize, (usize, usize))>;

/// Shares data between cloned instances as long as the signals aren't swapped out.
#[derive(Clone, Debug)]
pub struct EditorData {
    pub scope: Scope,
    pub editor_tab_id: RwSignal<Option<EditorTabId>>,
    pub diff_editor_id: RwSignal<Option<(EditorTabId, DiffEditorId)>>,
    pub confirmed: RwSignal<bool>,
    pub snippet: RwSignal<Option<SnippetIndex>>,
    pub inline_find: RwSignal<Option<InlineFindDirection>>,
    pub on_screen_find: RwSignal<OnScreenFind>,
    pub last_inline_find: RwSignal<Option<(InlineFindDirection, String)>>,
    pub find_focus: RwSignal<bool>,
    pub editor: Rc<Editor>,
    pub kind: RwSignal<EditorViewKind>,
    pub sticky_header_height: RwSignal<f64>,
    pub common: Rc<CommonData>,
    pub sticky_header_info: RwSignal<StickyHeaderInfo>,
}

impl PartialEq for EditorData {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl EditorData {
    fn new(
        cx: Scope,
        editor: Editor,
        editor_tab_id: Option<EditorTabId>,
        diff_editor_id: Option<(EditorTabId, DiffEditorId)>,
        confirmed: Option<RwSignal<bool>>,
        common: Rc<CommonData>,
    ) -> Self {
        let cx = cx.create_child();

        let confirmed = confirmed.unwrap_or_else(|| cx.create_rw_signal(false));
        EditorData {
            scope: cx,
            editor_tab_id: cx.create_rw_signal(editor_tab_id),
            diff_editor_id: cx.create_rw_signal(diff_editor_id),
            confirmed,
            snippet: cx.create_rw_signal(None),
            inline_find: cx.create_rw_signal(None),
            on_screen_find: cx.create_rw_signal(OnScreenFind {
                active: false,
                pattern: "".to_string(),
                regions: Vec::new(),
            }),
            last_inline_find: cx.create_rw_signal(None),
            find_focus: cx.create_rw_signal(false),
            editor: Rc::new(editor),
            kind: cx.create_rw_signal(EditorViewKind::Normal),
            sticky_header_height: cx.create_rw_signal(0.0),
            common,
            sticky_header_info: cx.create_rw_signal(StickyHeaderInfo::default()),
        }
    }

    /// Create a new local editor.
    ///
    /// You should prefer calling [`Editors::make_local`] / [`Editors::new_local`] instead to
    /// register the editor.
    pub fn new_local(cx: Scope, editors: Editors, common: Rc<CommonData>) -> Self {
        Self::new_local_id(cx, EditorId::next(), editors, common)
    }

    /// Create a new local editor with the given id.
    ///
    /// You should prefer calling [`Editors::make_local`] / [`Editors::new_local`] instead to
    /// register the editor.
    pub fn new_local_id(
        cx: Scope,
        editor_id: EditorId,
        editors: Editors,
        common: Rc<CommonData>,
    ) -> Self {
        let cx = cx.create_child();
        let doc = Rc::new(Doc::new_local(cx, editors, common.clone()));
        let editor = doc.create_editor(cx, editor_id, true);
        Self::new(cx, editor, None, None, None, common)
    }

    /// Create a new editor with a specific doc.
    ///
    /// You should prefer calling [`Editors::new_editor_doc`] / [`Editors::make_from_doc`] instead.
    pub fn new_doc(
        cx: Scope,
        doc: Rc<Doc>,
        editor_tab_id: Option<EditorTabId>,
        diff_editor_id: Option<(EditorTabId, DiffEditorId)>,
        confirmed: Option<RwSignal<bool>>,
        common: Rc<CommonData>,
    ) -> Self {
        let editor = doc.create_editor(cx, EditorId::next(), false);
        Self::new(cx, editor, editor_tab_id, diff_editor_id, confirmed, common)
    }

    /// Swap out the document this editor is for
    pub fn update_doc(&self, doc: Rc<Doc>) {
        let style = doc.styling();
        self.editor.update_doc(doc, Some(style));
    }

    /// Create a new editor using the same underlying [`Doc`]
    pub fn copy(
        &self,
        cx: Scope,
        editor_tab_id: Option<EditorTabId>,
        diff_editor_id: Option<(EditorTabId, DiffEditorId)>,
        confirmed: Option<RwSignal<bool>>,
    ) -> Self {
        let cx = cx.create_child();

        let confirmed = confirmed.unwrap_or_else(|| cx.create_rw_signal(true));

        let editor = Self::new_doc(
            cx,
            self.doc(),
            editor_tab_id,
            diff_editor_id,
            Some(confirmed),
            self.common.clone(),
        );
        editor.editor.cursor.set(self.editor.cursor.get_untracked());
        editor
            .editor
            .viewport
            .set(self.editor.viewport.get_untracked());
        editor.editor.scroll_to.set(Some(
            self.editor.viewport.get_untracked().origin().to_vec2(),
        ));
        editor
            .editor
            .last_movement
            .set(self.editor.last_movement.get_untracked());

        editor
    }

    pub fn id(&self) -> EditorId {
        self.editor.id()
    }

    pub fn editor_info(&self, _data: &WindowTabData) -> EditorInfo {
        let offset = self.cursor().get_untracked().offset();
        let scroll_offset = self.viewport().get_untracked().origin();
        let doc = self.doc();
        let is_pristine = doc.is_pristine();
        let unsaved = if is_pristine {
            None
        } else {
            Some(doc.buffer.with_untracked(|b| b.to_string()))
        };
        EditorInfo {
            content: self.doc().content.get_untracked(),
            unsaved,
            offset,
            scroll_offset: (scroll_offset.x, scroll_offset.y),
        }
    }

    pub fn cursor(&self) -> RwSignal<Cursor> {
        self.editor.cursor
    }

    pub fn viewport(&self) -> RwSignal<Rect> {
        self.editor.viewport
    }

    pub fn window_origin(&self) -> RwSignal<Point> {
        self.editor.window_origin
    }

    pub fn scroll_delta(&self) -> RwSignal<Vec2> {
        self.editor.scroll_delta
    }

    pub fn scroll_to(&self) -> RwSignal<Option<Vec2>> {
        self.editor.scroll_to
    }

    pub fn active(&self) -> RwSignal<bool> {
        self.editor.active
    }

    /// Get the line information for lines on the screen.
    pub fn screen_lines(&self) -> RwSignal<ScreenLines> {
        self.editor.screen_lines
    }

    pub fn doc(&self) -> Rc<Doc> {
        let doc = self.editor.doc();
        let Ok(doc) = (doc as Rc<dyn ::std::any::Any>).downcast() else {
            panic!("doc is not Rc<Doc>");
        };

        doc
    }

    /// The signal for the editor's document.
    pub fn doc_signal(&self) -> DocSignal {
        DocSignal {
            inner: self.editor.doc_signal(),
        }
    }

    pub fn text(&self) -> Rope {
        self.editor.text()
    }

    pub fn rope_text(&self) -> RopeTextVal {
        self.editor.rope_text()
    }

    fn scroll(&self, down: bool, count: usize, mods: Modifiers) {
        self.editor.scroll(
            self.sticky_header_height.get_untracked(),
            down,
            count,
            mods,
        )
    }

    pub fn do_edit(
        &self,
        selection: &Selection,
        edits: &[(impl AsRef<Selection>, &str)],
    ) {
        let mut cursor = self.cursor().get_untracked();
        let doc = self.doc();
        let (text, delta, inval_lines) =
            match doc.do_raw_edit(edits, EditType::Completion) {
                Some(e) => e,
                None => return,
            };
        let selection = selection.apply_delta(&delta, true, InsertDrift::Default);
        let old_cursor = cursor.mode.clone();
        doc.buffer.update(|buffer| {
            cursor.update_selection(buffer, selection);
            buffer.set_cursor_before(old_cursor);
            buffer.set_cursor_after(cursor.mode.clone());
        });
        self.cursor().set(cursor);

        self.apply_deltas(&[(text, delta, inval_lines)]);
    }

    pub fn do_text_edit(&self, edits: &[TextEdit]) {
        let (selection, edits) = self.doc().buffer.with_untracked(|buffer| {
            let selection = self.cursor().get_untracked().edit_selection(buffer);
            let edits = edits
                .iter()
                .map(|edit| {
                    let selection = lapce_core::selection::Selection::region(
                        buffer.offset_of_position(&edit.range.start),
                        buffer.offset_of_position(&edit.range.end),
                    );
                    (selection, edit.new_text.as_str())
                })
                .collect::<Vec<_>>();
            (selection, edits)
        });

        self.do_edit(&selection, &edits);
    }

    fn apply_deltas(&self, deltas: &[(Rope, RopeDelta, InvalLines)]) {
        if !deltas.is_empty() && !self.confirmed.get_untracked() {
            self.confirmed.set(true);
        }
        for (_, delta, _) in deltas {
            // self.inactive_apply_delta(delta);
            self.update_snippet_offset(delta);
            // self.update_breakpoints(delta);
        }
        // self.update_signature();
    }

    fn update_snippet_offset(&self, delta: &RopeDelta) {
        if self.snippet.with_untracked(|s| s.is_some()) {
            self.snippet.update(|snippet| {
                let mut transformer = Transformer::new(delta);
                *snippet = Some(
                    snippet
                        .as_ref()
                        .unwrap()
                        .iter()
                        .map(|(tab, (start, end))| {
                            (
                                *tab,
                                (
                                    transformer.transform(*start, false),
                                    transformer.transform(*end, true),
                                ),
                            )
                        })
                        .collect(),
                );
            });
        }
    }

    pub fn get_code_actions(&self) {
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
        let exists = doc
            .code_actions()
            .with_untracked(|c| c.contains_key(&offset));

        if exists {
            return;
        }

        // insert some empty data, so that we won't make the request again
        doc.code_actions().update(|c| {
            c.insert(offset, (PluginId(0), im::Vector::new()));
        });

        let (position, rev, diagnostics) = doc.buffer.with_untracked(|buffer| {
            let position = buffer.offset_to_position(offset);
            let rev = doc.rev();

            // Get the diagnostics for the current line, which the LSP might use to inform
            // what code actions are available (such as fixes for the diagnostics).
            let diagnostics = doc
                .diagnostics()
                .diagnostics_span
                .get_untracked()
                .iter_chunks(offset..offset)
                .filter(|(iv, _diag)| iv.start <= offset && iv.end >= offset)
                .map(|(_iv, diag)| diag)
                .cloned()
                .collect();

            (position, rev, diagnostics)
        });

        let send = create_ext_action(
            self.scope,
            move |resp: (PluginId, CodeActionResponse)| {
                if doc.rev() == rev {
                    doc.code_actions().update(|c| {
                        c.insert(offset, (resp.0, resp.1.into()));
                    });
                }
            },
        );

        self.common.proxy.get_code_actions(
            path,
            position,
            diagnostics,
            move |result| {
                if let Ok(ProxyResponse::GetCodeActionsResponse {
                    plugin_id,
                    resp,
                }) = result
                {
                    send((plugin_id, resp))
                }
            },
        );
    }

    pub fn show_code_actions(&self, mouse_click: bool) {
        let offset = self.cursor().with_untracked(|c| c.offset());
        let doc = self.doc();
        let code_actions = doc
            .code_actions()
            .with_untracked(|c| c.get(&offset).cloned());
        if let Some((plugin_id, code_actions)) = code_actions {
            if !code_actions.is_empty() {
                self.common.internal_command.send(
                    InternalCommand::ShowCodeActions {
                        offset,
                        mouse_click,
                        plugin_id,
                        code_actions,
                    },
                );
            }
        }
    }

    fn do_save(&self, after_action: impl FnOnce() + 'static) {
        self.doc().save(after_action);
    }

    pub fn save(
        &self,
        allow_formatting: bool,
        after_action: impl FnOnce() + 'static,
    ) {
        let doc = self.doc();
        let is_pristine = doc.is_pristine();
        let content = doc.content.get_untracked();

        if let DocContent::Scratch { .. } = &content {
            self.common
                .internal_command
                .send(InternalCommand::SaveScratchDoc2 { doc });
            return;
        }

        if content.path().is_some() && is_pristine {
            return;
        }

        let config = self.common.config.get_untracked();
        let DocContent::File { path, .. } = content else {
            return;
        };

        // If we are disallowing formatting (such as due to a manual save without formatting),
        // then we skip normalizing line endings as a common reason for that is large files.
        // (but if the save is typical, even if config format_on_save is false, we normalize)
        if allow_formatting && config.editor.normalize_line_endings {
            self.run_edit_command(&EditCommand::NormalizeLineEndings);
        }

        let rev = doc.rev();
        let format_on_save = allow_formatting && config.editor.format_on_save;
        if format_on_save {
            let editor = self.clone();
            let send = create_ext_action(self.scope, move |result| {
                if let Ok(Ok(ProxyResponse::GetDocumentFormatting { edits })) =
                    result
                {
                    let current_rev = editor.doc().rev();
                    if current_rev == rev {
                        editor.do_text_edit(&edits);
                    }
                }
                editor.do_save(after_action);
            });

            let (tx, rx) = crossbeam_channel::bounded(1);
            let proxy = self.common.proxy.clone();
            std::thread::spawn(move || {
                proxy.get_document_formatting(path, move |result| {
                    if let Err(err) = tx.send(result) {
                        tracing::error!("{:?}", err);
                    }
                });
                let result = rx.recv_timeout(std::time::Duration::from_secs(1));
                send(result);
            });
        } else {
            self.do_save(after_action);
        }
    }

    pub fn format(&self) {
        let doc = self.doc();
        let rev = doc.rev();
        let content = doc.content.get_untracked();

        if let DocContent::File { path, .. } = content {
            let editor = self.clone();
            let send = create_ext_action(self.scope, move |result| {
                if let Ok(Ok(ProxyResponse::GetDocumentFormatting { edits })) =
                    result
                {
                    let current_rev = editor.doc().rev();
                    if current_rev == rev {
                        editor.do_text_edit(&edits);
                    }
                }
            });

            let (tx, rx) = crossbeam_channel::bounded(1);
            let proxy = self.common.proxy.clone();
            std::thread::spawn(move || {
                proxy.get_document_formatting(path, move |result| {
                    if let Err(err) = tx.send(result) {
                        tracing::error!("{:?}", err);
                    }
                });
                let result = rx.recv_timeout(std::time::Duration::from_secs(1));
                send(result);
            });
        }
    }

    pub fn save_doc_position(&self) {
        let doc = self.doc();
        let path = match if doc.loaded() {
            doc.content.with_untracked(|c| c.path().cloned())
        } else {
            None
        } {
            Some(path) => path,
            None => return,
        };

        let cursor_offset = self.cursor().with_untracked(|c| c.offset());
        let scroll_offset = self.viewport().with_untracked(|v| v.origin().to_vec2());

        let db: Arc<LapceDb> = use_context().unwrap();
        db.save_doc_position(
            &self.common.workspace,
            path,
            cursor_offset,
            scroll_offset,
        );
    }

    fn rename(&self) {
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
        let (position, rev) = doc
            .buffer
            .with_untracked(|buffer| (buffer.offset_to_position(offset), doc.rev()));

        let cursor = self.cursor();
        let buffer = doc.buffer;
        let internal_command = self.common.internal_command;
        let local_path = path.clone();
        let send = create_ext_action(self.scope, move |result| {
            if let Ok(ProxyResponse::PrepareRename { resp }) = result {
                if buffer.with_untracked(|buffer| buffer.rev()) != rev {
                    return;
                }

                if cursor.with_untracked(|c| c.offset()) != offset {
                    return;
                }

                let (start, _end, position, placeholder) =
                    buffer.with_untracked(|buffer| match resp {
                        lsp_types::PrepareRenameResponse::Range(range) => (
                            buffer.offset_of_position(&range.start),
                            buffer.offset_of_position(&range.end),
                            range.start,
                            None,
                        ),
                        lsp_types::PrepareRenameResponse::RangeWithPlaceholder {
                            range,
                            placeholder,
                        } => (
                            buffer.offset_of_position(&range.start),
                            buffer.offset_of_position(&range.end),
                            range.start,
                            Some(placeholder),
                        ),
                        lsp_types::PrepareRenameResponse::DefaultBehavior {
                            ..
                        } => {
                            let start = buffer.prev_code_boundary(offset);
                            let position = buffer.offset_to_position(start);
                            (
                                start,
                                buffer.next_code_boundary(offset),
                                position,
                                None,
                            )
                        }
                    });
                let placeholder = placeholder.unwrap_or_else(|| {
                    buffer.with_untracked(|buffer| {
                        let (start, end) = buffer.select_word(offset);
                        buffer.slice_to_cow(start..end).to_string()
                    })
                });
                internal_command.send(InternalCommand::StartRename {
                    path: local_path.clone(),
                    placeholder,
                    start,
                    position,
                });
            }
        });
        self.common
            .proxy
            .prepare_rename(path, position, move |result| {
                send(result);
            });
    }

    // reset the doc inside and move cursor back
    pub fn reset(&self) {
        let doc = self.doc();
        doc.reload(Rope::from(""), true);
        self.cursor()
            .update(|cursor| cursor.set_offset(0, false, false));
    }

    pub fn visual_line(&self, line: usize) -> usize {
        self.kind.with_untracked(|kind| match kind {
            EditorViewKind::Normal => line,
            EditorViewKind::Diff(diff) => {
                let is_right = diff.is_right;
                let mut last_change: Option<&DiffLines> = None;
                let mut visual_line = 0;
                let mut changes = diff.changes.iter().peekable();
                while let Some(change) = changes.next() {
                    match (is_right, change) {
                        (true, DiffLines::Left(range)) => {
                            if let Some(DiffLines::Right(_)) = changes.peek() {
                            } else {
                                visual_line += range.len();
                            }
                        }
                        (false, DiffLines::Right(range)) => {
                            let len = if let Some(DiffLines::Left(r)) = last_change {
                                range.len() - r.len().min(range.len())
                            } else {
                                range.len()
                            };
                            if len > 0 {
                                visual_line += len;
                            }
                        }
                        (true, DiffLines::Right(range))
                        | (false, DiffLines::Left(range)) => {
                            if line < range.end {
                                return visual_line + line - range.start;
                            }
                            visual_line += range.len();
                            if is_right {
                                if let Some(DiffLines::Left(r)) = last_change {
                                    let len = r.len() - r.len().min(range.len());
                                    if len > 0 {
                                        visual_line += len;
                                    }
                                }
                            }
                        }
                        (_, DiffLines::Both(info)) => {
                            let end = if is_right {
                                info.right.end
                            } else {
                                info.left.end
                            };
                            if line >= end {
                                visual_line += info.right.len()
                                    - info
                                        .skip
                                        .as_ref()
                                        .map(|skip| skip.len().saturating_sub(1))
                                        .unwrap_or(0);
                                last_change = Some(change);
                                continue;
                            }

                            let start = if is_right {
                                info.right.start
                            } else {
                                info.left.start
                            };
                            if let Some(skip) = info.skip.as_ref() {
                                if start + skip.start > line {
                                    return visual_line + line - start;
                                } else if start + skip.end > line {
                                    return visual_line + skip.start;
                                } else {
                                    return visual_line
                                        + (line - start - skip.len() + 1);
                                }
                            } else {
                                return visual_line + line - start;
                            }
                        }
                    }
                    last_change = Some(change);
                }
                visual_line
            }
        })
    }

    pub fn actual_line(&self, visual_line: usize, bottom_affinity: bool) -> usize {
        self.kind.with_untracked(|kind| match kind {
            EditorViewKind::Normal => visual_line,
            EditorViewKind::Diff(diff) => {
                let is_right = diff.is_right;
                let mut actual_line: usize = 0;
                let mut current_visual_line = 0;
                let mut last_change: Option<&DiffLines> = None;
                let mut changes = diff.changes.iter().peekable();
                while let Some(change) = changes.next() {
                    match (is_right, change) {
                        (true, DiffLines::Left(range)) => {
                            if let Some(DiffLines::Right(_)) = changes.peek() {
                            } else {
                                current_visual_line += range.len();
                                if current_visual_line >= visual_line {
                                    return if bottom_affinity {
                                        actual_line
                                    } else {
                                        actual_line.saturating_sub(1)
                                    };
                                }
                            }
                        }
                        (false, DiffLines::Right(range)) => {
                            let len = if let Some(DiffLines::Left(r)) = last_change {
                                range.len() - r.len().min(range.len())
                            } else {
                                range.len()
                            };
                            if len > 0 {
                                current_visual_line += len;
                                if current_visual_line >= visual_line {
                                    return actual_line;
                                }
                            }
                        }
                        (true, DiffLines::Right(range))
                        | (false, DiffLines::Left(range)) => {
                            let len = range.len();
                            if current_visual_line + len > visual_line {
                                return range.start
                                    + (visual_line - current_visual_line);
                            }
                            current_visual_line += len;
                            actual_line += len;
                            if is_right {
                                if let Some(DiffLines::Left(r)) = last_change {
                                    let len = r.len() - r.len().min(range.len());
                                    if len > 0 {
                                        current_visual_line += len;
                                        if current_visual_line > visual_line {
                                            return if bottom_affinity {
                                                actual_line
                                            } else {
                                                actual_line - range.len()
                                            };
                                        }
                                    }
                                }
                            }
                        }
                        (_, DiffLines::Both(info)) => {
                            let len = info.right.len();
                            let start = if is_right {
                                info.right.start
                            } else {
                                info.left.start
                            };

                            if let Some(skip) = info.skip.as_ref() {
                                if current_visual_line + skip.start == visual_line {
                                    return if bottom_affinity {
                                        actual_line + skip.end
                                    } else {
                                        (actual_line + skip.start).saturating_sub(1)
                                    };
                                } else if current_visual_line + skip.start + 1
                                    > visual_line
                                {
                                    return actual_line + visual_line
                                        - current_visual_line;
                                } else if current_visual_line + len - skip.len() + 1
                                    >= visual_line
                                {
                                    return actual_line
                                        + skip.end
                                        + (visual_line
                                            - current_visual_line
                                            - skip.start
                                            - 1);
                                }
                                actual_line += len;
                                current_visual_line += len - skip.len() + 1;
                            } else {
                                if current_visual_line + len > visual_line {
                                    return start
                                        + (visual_line - current_visual_line);
                                }
                                current_visual_line += len;
                                actual_line += len;
                            }
                        }
                    }
                    last_change = Some(change);
                }
                actual_line
            }
        })
    }
}

/// Custom signal wrapper for [`Doc`], because [`Editor`] only knows it as a
/// `Rc<dyn Document>`, and there is currently no way to have an `RwSignal<Rc<Doc>>` and
/// an `RwSignal<Rc<dyn Document>>`.
///
// FIXME: This could possibly be swapped with a generic impl?
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocSignal {
    // TODO: replace with ReadSignal once that impls `track`
    inner: RwSignal<Rc<dyn Document>>,
}
impl DocSignal {
    pub fn get(&self) -> Rc<Doc> {
        let doc = self.inner.get();
        (doc as Rc<dyn ::std::any::Any>)
            .downcast()
            .expect("doc is not Rc<Doc>")
    }

    pub fn get_untracked(&self) -> Rc<Doc> {
        let doc = self.inner.get_untracked();
        (doc as Rc<dyn ::std::any::Any>)
            .downcast()
            .expect("doc is not Rc<Doc>")
    }

    pub fn with<O>(&self, f: impl FnOnce(&Rc<Doc>) -> O) -> O {
        self.inner.with(|doc| {
            let doc = doc.clone();
            let doc: Rc<Doc> = (doc as Rc<dyn ::std::any::Any>)
                .downcast()
                .expect("doc is not Rc<Doc>");
            f(&doc)
        })
    }

    pub fn with_untracked<O>(&self, f: impl FnOnce(&Rc<Doc>) -> O) -> O {
        self.inner.with_untracked(|doc| {
            let doc = doc.clone();
            let doc: Rc<Doc> = (doc as Rc<dyn ::std::any::Any>)
                .downcast()
                .expect("doc is not Rc<Doc>");
            f(&doc)
        })
    }

    pub fn track(&self) {
        self.inner.track();
    }
}
