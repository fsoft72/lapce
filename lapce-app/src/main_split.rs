use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

mod diagnostics;
mod editor_tab_ops;
mod editors;
mod files;
mod jump;
mod split_ops;
mod split_tree;

pub use self::{
    editors::Editors,
    split_tree::{
        SplitContent, SplitContentInfo, SplitData, SplitDirection, SplitInfo,
        SplitMoveDirection,
    },
};

use floem::{
    ext_event::create_ext_action,
    reactive::{Memo, RwSignal, Scope, SignalGet, SignalUpdate, SignalWith},
};
use itertools::Itertools;
use lapce_core::{buffer::rope_text::RopeText, selection::Selection};
use lapce_rpc::{core::FileChanged, plugin::VoltID, proxy::ProxyResponse};
use lapce_xi_rope::Rope;
use lsp_types::TextEdit;

use crate::{
    code_lens::CodeLensData,
    doc::{DiagnosticData, Doc, DocContent, DocHistory},
    editor::{EditorData, diff::DiffEditorData, location::EditorLocation},
    editor_tab::{EditorTabChild, EditorTabChildSource, EditorTabData},
    id::{DiffEditorId, EditorTabId, SplitId},
    keypress::{EventRef, KeyPressData, KeyPressHandle},
    panel::implementation_view::ReferencesRoot,
    window_tab::{CommonData, Focus},
};

#[derive(Clone)]
pub struct MainSplitData {
    pub scope: Scope,
    pub root_split: SplitId,
    pub active_editor_tab: RwSignal<Option<EditorTabId>>,
    pub splits: RwSignal<im::HashMap<SplitId, RwSignal<SplitData>>>,
    pub editor_tabs: RwSignal<im::HashMap<EditorTabId, RwSignal<EditorTabData>>>,
    pub editors: Editors,
    pub diff_editors: RwSignal<im::HashMap<DiffEditorId, DiffEditorData>>,
    pub docs: RwSignal<im::HashMap<PathBuf, Rc<Doc>>>,
    pub scratch_docs: RwSignal<im::HashMap<String, Rc<Doc>>>,
    pub diagnostics: RwSignal<im::HashMap<PathBuf, DiagnosticData>>,
    pub references: RwSignal<ReferencesRoot>,
    pub implementations: RwSignal<crate::panel::implementation_view::ReferencesRoot>,
    pub active_editor: Memo<Option<EditorData>>,
    pub find_editor: EditorData,
    pub replace_editor: EditorData,
    pub locations: RwSignal<im::Vector<EditorLocation>>,
    pub current_location: RwSignal<usize>,
    pub width: RwSignal<f64>,
    pub code_lens: RwSignal<CodeLensData>,
    pub common: Rc<CommonData>,
}

impl std::fmt::Debug for MainSplitData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MainSplitData")
            .field("root_split", &self.root_split)
            .finish()
    }
}

impl MainSplitData {
    pub fn new(cx: Scope, common: Rc<CommonData>) -> Self {
        let splits = cx.create_rw_signal(im::HashMap::new());
        let active_editor_tab = cx.create_rw_signal(None);
        let editor_tabs: RwSignal<
            im::HashMap<EditorTabId, RwSignal<EditorTabData>>,
        > = cx.create_rw_signal(im::HashMap::new());
        let editors = Editors::new(cx);
        let diff_editors: RwSignal<im::HashMap<DiffEditorId, DiffEditorData>> =
            cx.create_rw_signal(im::HashMap::new());
        let docs: RwSignal<im::HashMap<PathBuf, Rc<Doc>>> =
            cx.create_rw_signal(im::HashMap::new());
        let scratch_docs = cx.create_rw_signal(im::HashMap::new());
        let locations = cx.create_rw_signal(im::Vector::new());
        let references = cx.create_rw_signal(ReferencesRoot::default());
        let implementations = cx.create_rw_signal(
            crate::panel::implementation_view::ReferencesRoot::default(),
        );
        let current_location = cx.create_rw_signal(0);
        let diagnostics = cx.create_rw_signal(im::HashMap::new());
        let find_editor = editors.make_local(cx, common.clone());
        let replace_editor = editors.make_local(cx, common.clone());

        let active_editor = cx.create_memo(move |_| -> Option<EditorData> {
            let active_editor_tab = active_editor_tab.get()?;
            let editor_tab = editor_tabs
                .with(|editor_tabs| editor_tabs.get(&active_editor_tab).copied())?;
            let (_, _, child) = editor_tab.with(|editor_tab| {
                editor_tab.children.get(editor_tab.active).cloned()
            })?;

            let editor = match child {
                EditorTabChild::Editor(editor_id) => editors.editor(editor_id)?,
                EditorTabChild::DiffEditor(diff_editor_id) => {
                    let diff_editor = diff_editors.with(|diff_editors| {
                        diff_editors.get(&diff_editor_id).cloned()
                    })?;
                    if diff_editor.focus_right.get() {
                        diff_editor.right
                    } else {
                        diff_editor.left
                    }
                }
                _ => return None,
            };

            Some(editor)
        });

        {
            let buffer = find_editor.doc().buffer;
            let find = common.find.clone();
            cx.create_effect(move |_| {
                let content = buffer.with(|buffer| buffer.to_string());
                find.set_find(&content);
            });
        }

        Self {
            scope: cx,
            root_split: SplitId::next(),
            splits,
            active_editor_tab,
            editor_tabs,
            editors,
            diff_editors,
            docs,
            scratch_docs,
            active_editor,
            find_editor,
            replace_editor,
            diagnostics,
            locations,
            current_location,
            width: cx.create_rw_signal(0.0),
            code_lens: cx.create_rw_signal(CodeLensData::new(common.clone())),
            common,
            references,
            implementations,
        }
    }

    pub fn key_down<'a>(
        &self,
        event: impl Into<EventRef<'a>>,
        keypress: &KeyPressData,
    ) -> Option<KeyPressHandle> {
        let active_editor_tab = self.active_editor_tab.get_untracked()?;
        let editor_tab = self.editor_tabs.with_untracked(|editor_tabs| {
            editor_tabs.get(&active_editor_tab).copied()
        })?;
        let (_, _, child) = editor_tab.with_untracked(|editor_tab| {
            editor_tab.children.get(editor_tab.active).cloned()
        })?;
        match child {
            EditorTabChild::Editor(editor_id) => {
                let editor = self.editors.editor_untracked(editor_id)?;
                let handle = keypress.key_down(event, &editor);
                editor.get_code_actions();
                Some(handle)
            }
            EditorTabChild::DiffEditor(diff_editor_id) => {
                let diff_editor =
                    self.diff_editors.with_untracked(|diff_editors| {
                        diff_editors.get(&diff_editor_id).cloned()
                    })?;
                let editor = if diff_editor.focus_right.get_untracked() {
                    &diff_editor.right
                } else {
                    &diff_editor.left
                };
                let handle = keypress.key_down(event, editor);
                editor.get_code_actions();
                Some(handle)
            }
            EditorTabChild::Settings(_) => None,
            EditorTabChild::ThemeColorSettings(_) => None,
            EditorTabChild::Keymap(_) => None,
            EditorTabChild::Volt(_, _) => None,
        }
    }

    pub fn get_doc(
        &self,
        path: PathBuf,
        unsaved: Option<String>,
    ) -> (Rc<Doc>, bool) {
        let cx = self.scope;
        let doc = self.docs.with_untracked(|docs| docs.get(&path).cloned());
        if let Some(doc) = doc {
            (doc, false)
        } else {
            let diagnostic_data = self.get_diagnostic_data(&path);

            let doc = Doc::new(
                cx,
                path.clone(),
                diagnostic_data,
                self.editors,
                self.common.clone(),
            );
            let doc = Rc::new(doc);
            self.docs.update(|docs| {
                docs.insert(path.clone(), doc.clone());
            });

            {
                let doc = doc.clone();
                let local_doc = doc.clone();
                let send = create_ext_action(cx, move |result| {
                    if let Ok(ProxyResponse::NewBufferResponse {
                        content,
                        read_only,
                    }) = result
                    {
                        local_doc.init_content(Rope::from(content));
                        if read_only {
                            local_doc.content.update(|content| {
                                if let DocContent::File { read_only, .. } = content {
                                    *read_only = true;
                                }
                            });
                        } else if let Some(unsaved) = unsaved {
                            local_doc.reload(Rope::from(unsaved), false);
                        }
                    }
                });

                self.common
                    .proxy
                    .new_buffer(doc.buffer_id, path, move |result| {
                        send(result);
                    });
            }
            doc.get_code_lens();
            doc.get_folding_range();
            doc.get_document_symbol();
            (doc, true)
        }
    }

    pub fn go_to_location(
        &self,
        location: EditorLocation,
        edits: Option<Vec<TextEdit>>,
    ) {
        if self.common.focus.get_untracked() != Focus::Workbench {
            self.common.focus.set(Focus::Workbench);
        }
        let path = location.path.clone();
        let (doc, new_doc) = self.get_doc(path.clone(), None);

        let child = self.get_editor_tab_child(
            EditorTabChildSource::Editor { path, doc },
            location.ignore_unconfirmed,
            location.same_editor_tab,
        );
        if let EditorTabChild::Editor(editor_id) = child {
            if let Some(editor) = self.editors.editor_untracked(editor_id) {
                editor.go_to_location(location, new_doc, edits);
            }
        }
    }

    pub fn open_file_changes(&self, path: PathBuf) {
        let (right, _) = self.get_doc(path.clone(), None);
        let left = Doc::new_history(
            self.scope,
            DocContent::History(DocHistory {
                path: path.clone(),
                version: "head".to_string(),
            }),
            self.editors,
            self.common.clone(),
        );
        let left = Rc::new(left);

        let send = {
            let left = left.clone();
            create_ext_action(self.scope, move |result| {
                if let Ok(ProxyResponse::BufferHeadResponse { content, .. }) = result
                {
                    left.init_content(Rope::from(content));
                }
            })
        };
        self.common.proxy.get_buffer_head(path, move |result| {
            send(result);
        });

        self.get_editor_tab_child(
            EditorTabChildSource::DiffEditor { left, right },
            false,
            false,
        );
    }

    pub fn open_diff_files(&self, left_path: PathBuf, right_path: PathBuf) {
        let [left, right] =
            [left_path, right_path].map(|path| self.get_doc(path, None).0);

        self.get_editor_tab_child(
            EditorTabChildSource::DiffEditor { left, right },
            false,
            false,
        );
    }

    pub fn open_file_changed(&self, path: &Path, content: &FileChanged) {
        tracing::debug!("open_file_changed {:?}", path);
        match content {
            FileChanged::Change(content) => {
                let doc = self.docs.with_untracked(|docs| docs.get(path).cloned());
                let doc = match doc {
                    Some(doc) => doc,
                    None => return,
                };
                doc.handle_file_changed(Rope::from(content));
            }
            FileChanged::Delete => {
                if self.docs.with_untracked(|x| x.get(path).is_none()) {
                    return;
                }
                let Some(editor_id) = self.editors.get_editor_id_by_path(path)
                else {
                    return;
                };
                let id = editor_id.to_raw();
                if let Some(tab_id) = self.editor_tabs.with_untracked(|x| {
                    for (tab_id, tab_data) in x {
                        if tab_data.with_untracked(|x| {
                            x.children.iter().any(|(_, _, child)| child.id() == id)
                        }) {
                            return Some(*tab_id);
                        }
                    }
                    None
                }) {
                    self.editor_tab_close(tab_id);
                }
                self.editors.remove(editor_id);
                self.docs.update(|x| {
                    x.remove(path);
                });
            }
        }
    }

    pub fn set_find_pattern(&self, pattern: Option<String>) {
        if let Some(pattern) = pattern {
            self.find_editor.doc().reload(Rope::from(pattern), true);
        }
        let pattern_len = self
            .find_editor
            .doc()
            .buffer
            .with_untracked(|buffer| buffer.len());
        self.find_editor
            .cursor()
            .update(|cursor| cursor.set_insert(Selection::region(0, pattern_len)));
    }

    pub fn open_volt_view(&self, id: VoltID) {
        self.get_editor_tab_child(EditorTabChildSource::Volt(id), false, false);
    }

    pub fn open_settings(&self) {
        self.get_editor_tab_child(EditorTabChildSource::Settings, false, false);
    }

    pub fn open_theme_color_settings(&self) {
        self.get_editor_tab_child(
            EditorTabChildSource::ThemeColorSettings,
            false,
            false,
        );
    }

    pub fn open_keymap(&self) {
        self.get_editor_tab_child(EditorTabChildSource::Keymap, false, false);
    }

    pub fn new_file(&self) -> EditorTabChild {
        self.get_editor_tab_child(EditorTabChildSource::NewFileEditor, false, false)
    }

    /// Opens a new file holding the current theme in TOML.
    pub fn export_theme(&self) {
        let theme = match self.common.config.get_untracked().export_theme() {
            Ok(theme) => theme,
            Err(err) => {
                tracing::error!("failed to export the theme: {err:?}");
                return;
            }
        };
        let child = self.new_file();
        if let EditorTabChild::Editor(id) = child {
            if let Some(editor) = self.editors.editor_untracked(id) {
                editor.doc().reload(Rope::from(theme), true);
            }
        }
    }

    pub fn show_env(&self) {
        let child = self.new_file();
        if let EditorTabChild::Editor(id) = child {
            if let Some(editor) = self.editors.editor_untracked(id) {
                let doc = editor.doc();
                doc.reload(
                    Rope::from(
                        std::env::vars().map(|(k, v)| format!("{k}={v}")).join("\n"),
                    ),
                    true,
                );
            }
        }
    }

    pub fn get_active_editor(&self) -> Option<EditorData> {
        let active_editor_tab = self.active_editor_tab.get()?;
        let editor_tabs = self.editor_tabs;
        let editor_tab = editor_tabs
            .with(|editor_tabs| editor_tabs.get(&active_editor_tab).copied())?;
        let (_, _, child) = editor_tab.with(|editor_tab| {
            editor_tab.children.get(editor_tab.active).cloned()
        })?;
        match child {
            EditorTabChild::Editor(editor_id) => self.editors.editor(editor_id),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TabCloseKind {
    CloseOther,
    CloseToLeft,
    CloseToRight,
}
