//! The registry of the editors of a main split.

use std::{path::Path, rc::Rc};

use floem::{
    reactive::{RwSignal, Scope, SignalUpdate, SignalWith},
    views::editor::id::EditorId,
};
use tracing::{Level, event};

use crate::{
    doc::Doc,
    editor::EditorData,
    id::{DiffEditorId, EditorTabId},
    window_tab::CommonData,
};

/// All the editors in a main split
#[derive(Clone, Copy)]
pub struct Editors(pub RwSignal<im::HashMap<EditorId, EditorData>>);
impl Editors {
    pub(super) fn new(cx: Scope) -> Self {
        Self(cx.create_rw_signal(im::HashMap::new()))
    }

    /// Add an editor to the editors.
    /// Returns the id of the editor.
    pub fn insert(&self, editor: EditorData) -> EditorId {
        let id = editor.id();
        self.0.update(|editors| {
            if editors.insert(id, editor).is_some() {
                event!(Level::WARN, "Inserted EditorId that already exists");
            }
        });

        id
    }

    pub fn insert_with_id(&self, id: EditorId, editor: EditorData) {
        self.0.update(|editors| {
            editors.insert(id, editor);
        });
    }

    pub fn new_local(&self, cx: Scope, common: Rc<CommonData>) -> EditorId {
        let editor = EditorData::new_local(cx, *self, common);

        self.insert(editor)
    }

    /// Equivalent to [`Self::new_local`], but immediately gets the created editor.
    pub fn make_local(&self, cx: Scope, common: Rc<CommonData>) -> EditorData {
        let id = self.new_local(cx, common);
        self.editor_untracked(id).unwrap()
    }

    pub fn new_from_doc(
        &self,
        cx: Scope,
        doc: Rc<Doc>,
        editor_tab_id: Option<EditorTabId>,
        diff_editor_id: Option<(EditorTabId, DiffEditorId)>,
        confirmed: Option<RwSignal<bool>>,
        common: Rc<CommonData>,
    ) -> EditorId {
        let editor = EditorData::new_doc(
            cx,
            doc,
            editor_tab_id,
            diff_editor_id,
            confirmed,
            common,
        );

        self.insert(editor)
    }

    /// Equivalent to [`Self::new_editor_doc`], but immediately gets the created editor.
    pub fn make_from_doc(
        &self,
        cx: Scope,
        doc: Rc<Doc>,
        editor_tab_id: Option<EditorTabId>,
        diff_editor_id: Option<(EditorTabId, DiffEditorId)>,
        confirmed: Option<RwSignal<bool>>,
        common: Rc<CommonData>,
    ) -> EditorData {
        let id = self.new_from_doc(
            cx,
            doc,
            editor_tab_id,
            diff_editor_id,
            confirmed,
            common,
        );
        self.editor_untracked(id).unwrap()
    }

    /// Copy an existing editor which is inserted into [`Editors`]
    pub fn copy(
        &self,
        editor_id: EditorId,
        cx: Scope,
        editor_tab_id: Option<EditorTabId>,
        diff_editor_id: Option<(EditorTabId, DiffEditorId)>,
        confirmed: Option<RwSignal<bool>>,
    ) -> Option<EditorId> {
        let editor = self.editor_untracked(editor_id)?;
        let new_editor = editor.copy(cx, editor_tab_id, diff_editor_id, confirmed);

        Some(self.insert(new_editor))
    }

    pub fn make_copy(
        &self,
        editor_id: EditorId,
        cx: Scope,
        editor_tab_id: Option<EditorTabId>,
        diff_editor_id: Option<(EditorTabId, DiffEditorId)>,
        confirmed: Option<RwSignal<bool>>,
    ) -> Option<EditorData> {
        let editor_id =
            self.copy(editor_id, cx, editor_tab_id, diff_editor_id, confirmed)?;
        self.editor_untracked(editor_id)
    }

    pub fn remove(&self, id: EditorId) -> Option<EditorData> {
        self.0.try_update(|editors| editors.remove(&id)).unwrap()
    }

    pub fn get_editor_id_by_path(&self, path: &Path) -> Option<EditorId> {
        self.0.with_untracked(|x| {
            for (id, data) in x {
                if data.doc().content.with_untracked(|x| {
                    if let Some(doc_path) = x.path() {
                        doc_path == path
                    } else {
                        false
                    }
                }) {
                    return Some(*id);
                }
            }
            None
        })
    }

    pub fn contains_untracked(&self, id: EditorId) -> bool {
        self.0.with_untracked(|editors| editors.contains_key(&id))
    }

    /// Get the editor (tracking the signal)
    pub fn editor(&self, id: EditorId) -> Option<EditorData> {
        self.0.with(|editors| editors.get(&id).cloned())
    }

    /// Get the editor (not tracking the signal)
    pub fn editor_untracked(&self, id: EditorId) -> Option<EditorData> {
        self.0.with_untracked(|editors| editors.get(&id).cloned())
    }

    pub fn with_editors<O>(
        &self,
        f: impl FnOnce(&im::HashMap<EditorId, EditorData>) -> O,
    ) -> O {
        self.0.with(f)
    }

    pub fn with_editors_untracked<O>(
        &self,
        f: impl FnOnce(&im::HashMap<EditorId, EditorData>) -> O,
    ) -> O {
        self.0.with_untracked(f)
    }
}
