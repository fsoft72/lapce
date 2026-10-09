//! Editor tab operations of the main split: open, close and move editor tab children.

use std::rc::Rc;

use floem::{
    action::save_as,
    file::{FileDialogOptions, FileInfo},
    keyboard::Modifiers,
    peniko::kurbo::{Point, Rect},
    reactive::{RwSignal, SignalGet, SignalUpdate, SignalWith},
    views::editor::id::EditorId,
};
use lapce_core::{command::FocusCommand, cursor::Cursor};
use lapce_rpc::buffer::BufferId;

use super::{
    MainSplitData, SplitContent, SplitData, SplitMoveDirection, TabCloseKind,
};
use crate::{
    alert::AlertButton,
    command::InternalCommand,
    doc::{Doc, DocContent},
    editor::{EditorData, diff::DiffEditorData},
    editor_tab::{EditorTabChild, EditorTabChildSource, EditorTabData},
    id::{DiffEditorId, EditorTabId, KeymapId, SettingsId, SplitId, VoltViewId},
};

impl MainSplitData {
    fn new_editor_tab(
        &self,
        editor_tab_id: EditorTabId,
        split_id: SplitId,
    ) -> RwSignal<EditorTabData> {
        let editor_tab = {
            let cx = self.scope.create_child();
            let editor_tab = EditorTabData {
                scope: cx,
                split: split_id,
                active: 0,
                editor_tab_id,
                children: vec![],
                window_origin: Point::ZERO,
                layout_rect: Rect::ZERO,
                locations: cx.create_rw_signal(im::Vector::new()),
                current_location: cx.create_rw_signal(0),
            };
            cx.create_rw_signal(editor_tab)
        };
        self.editor_tabs.update(|editor_tabs| {
            editor_tabs.insert(editor_tab_id, editor_tab);
        });
        editor_tab
    }

    pub(super) fn get_editor_tab_child(
        &self,
        source: EditorTabChildSource,
        ignore_unconfirmed: bool,
        same_editor_tab: bool,
    ) -> EditorTabChild {
        let config = self.common.config.get_untracked();

        let active_editor_tab_id = self.active_editor_tab.get_untracked();
        let editor_tabs = self.editor_tabs.get_untracked();
        let active_editor_tab = active_editor_tab_id
            .and_then(|id| editor_tabs.get(&id))
            .cloned();

        let editors = self.editors;
        let diff_editors = self.diff_editors.get_untracked();

        let active_editor_tab = if let Some(editor_tab) = active_editor_tab {
            editor_tab
        } else if editor_tabs.is_empty() {
            let editor_tab_id = EditorTabId::next();
            let editor_tab = self.new_editor_tab(editor_tab_id, self.root_split);
            let root_split = self.splits.with_untracked(|splits| {
                splits.get(&self.root_split).cloned().unwrap()
            });
            root_split.update(|root_split| {
                root_split.children = vec![(
                    root_split.scope.create_rw_signal(1.0),
                    SplitContent::EditorTab(editor_tab_id),
                )];
            });
            self.active_editor_tab.set(Some(editor_tab_id));
            editor_tab
        } else {
            let (editor_tab_id, editor_tab) = editor_tabs.iter().next().unwrap();
            self.active_editor_tab.set(Some(*editor_tab_id));
            *editor_tab
        };

        let is_same_diff_editor =
            |diff_editor_id: &DiffEditorId, left: &Rc<Doc>, right: &Rc<Doc>| {
                diff_editors
                    .get(diff_editor_id)
                    .map(|diff_editor| {
                        left.content.get_untracked()
                            == diff_editor.left.doc().content.get_untracked()
                            && right.content.get_untracked()
                                == diff_editor.right.doc().content.get_untracked()
                    })
                    .unwrap_or(false)
            };

        let selected = if !config.editor.show_tab {
            active_editor_tab.with_untracked(|editor_tab| {
                for (i, (_, _, child)) in editor_tab.children.iter().enumerate() {
                    let can_be_selected = match child {
                        EditorTabChild::Editor(editor_id) => {
                            if let Some(editor) =
                                editors.editor_untracked(*editor_id)
                            {
                                let doc = editor.doc();
                                let same_path =
                                    if let EditorTabChildSource::Editor {
                                        path,
                                        ..
                                    } = &source
                                    {
                                        doc.content.with_untracked(|content| {
                                            content
                                                .path()
                                                .map(|p| p == path)
                                                .unwrap_or(false)
                                        })
                                    } else {
                                        false
                                    };

                                same_path
                                    || doc.buffer.with_untracked(|b| b.is_pristine())
                            } else {
                                false
                            }
                        }
                        EditorTabChild::DiffEditor(diff_editor_id) => {
                            if let EditorTabChildSource::DiffEditor { left, right } =
                                &source
                            {
                                is_same_diff_editor(diff_editor_id, left, right)
                                    || diff_editors
                                        .get(diff_editor_id)
                                        .map(|diff_editor| {
                                            diff_editor.left.doc().is_pristine()
                                                && diff_editor
                                                    .right
                                                    .doc()
                                                    .is_pristine()
                                        })
                                        .unwrap_or(false)
                            } else {
                                false
                            }
                        }
                        EditorTabChild::Settings(_) => true,
                        EditorTabChild::ThemeColorSettings(_) => true,
                        EditorTabChild::Keymap(_) => true,
                        EditorTabChild::Volt(_, _) => true,
                    };

                    if can_be_selected {
                        return Some(i);
                    }
                }
                None
            })
        } else {
            match &source {
                EditorTabChildSource::Editor { path, .. } => active_editor_tab
                    .with_untracked(|editor_tab| {
                        editor_tab
                            .get_editor(editors, path)
                            .map(|(i, _)| i)
                            .or_else(|| {
                                if ignore_unconfirmed {
                                    None
                                } else {
                                    editor_tab
                                        .get_unconfirmed_editor_tab_child(
                                            editors,
                                            &diff_editors,
                                        )
                                        .map(|(i, _)| i)
                                }
                            })
                    }),
                EditorTabChildSource::DiffEditor { left, right } => {
                    if let Some(index) =
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab.children.iter().position(|(_, _, child)| {
                                if let EditorTabChild::DiffEditor(diff_editor_id) =
                                    child
                                {
                                    is_same_diff_editor(diff_editor_id, left, right)
                                } else {
                                    false
                                }
                            })
                        })
                    {
                        Some(index)
                    } else if ignore_unconfirmed {
                        None
                    } else {
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab
                                .get_unconfirmed_editor_tab_child(
                                    editors,
                                    &diff_editors,
                                )
                                .map(|(i, _)| i)
                        })
                    }
                }
                EditorTabChildSource::NewFileEditor => {
                    if ignore_unconfirmed {
                        None
                    } else {
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab
                                .get_unconfirmed_editor_tab_child(
                                    editors,
                                    &diff_editors,
                                )
                                .map(|(i, _)| i)
                        })
                    }
                }
                EditorTabChildSource::Settings => {
                    if let Some(index) =
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab.children.iter().position(|(_, _, child)| {
                                matches!(child, EditorTabChild::Settings(_))
                            })
                        })
                    {
                        Some(index)
                    } else if ignore_unconfirmed {
                        None
                    } else {
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab
                                .get_unconfirmed_editor_tab_child(
                                    editors,
                                    &diff_editors,
                                )
                                .map(|(i, _)| i)
                        })
                    }
                }
                EditorTabChildSource::ThemeColorSettings => {
                    if let Some(index) =
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab.children.iter().position(|(_, _, child)| {
                                matches!(
                                    child,
                                    EditorTabChild::ThemeColorSettings(_)
                                )
                            })
                        })
                    {
                        Some(index)
                    } else if ignore_unconfirmed {
                        None
                    } else {
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab
                                .get_unconfirmed_editor_tab_child(
                                    editors,
                                    &diff_editors,
                                )
                                .map(|(i, _)| i)
                        })
                    }
                }
                EditorTabChildSource::Keymap => {
                    if let Some(index) =
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab.children.iter().position(|(_, _, child)| {
                                matches!(child, EditorTabChild::Keymap(_))
                            })
                        })
                    {
                        Some(index)
                    } else if ignore_unconfirmed {
                        None
                    } else {
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab
                                .get_unconfirmed_editor_tab_child(
                                    editors,
                                    &diff_editors,
                                )
                                .map(|(i, _)| i)
                        })
                    }
                }
                EditorTabChildSource::Volt(id) => {
                    if let Some(index) =
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab.children.iter().position(|(_, _, child)| {
                                if let EditorTabChild::Volt(_, current_id) = child {
                                    current_id == id
                                } else {
                                    false
                                }
                            })
                        })
                    {
                        Some(index)
                    } else if ignore_unconfirmed {
                        None
                    } else {
                        active_editor_tab.with_untracked(|editor_tab| {
                            editor_tab
                                .get_unconfirmed_editor_tab_child(
                                    editors,
                                    &diff_editors,
                                )
                                .map(|(i, _)| i)
                        })
                    }
                }
            }
        };

        let new_child_from_source =
            |editor_tab_id: EditorTabId, source: &EditorTabChildSource| match source
            {
                EditorTabChildSource::Editor { doc, .. } => {
                    let editor_id = editors.new_from_doc(
                        self.scope,
                        doc.clone(),
                        Some(editor_tab_id),
                        None,
                        None,
                        self.common.clone(),
                    );

                    EditorTabChild::Editor(editor_id)
                }
                EditorTabChildSource::NewFileEditor => {
                    let name = self.get_name_for_new_file();
                    let doc_content = DocContent::Scratch {
                        id: BufferId::next(),
                        name: name.clone(),
                    };
                    let doc = Doc::new_content(
                        self.scope,
                        doc_content,
                        self.editors,
                        self.common.clone(),
                    );
                    let doc = Rc::new(doc);
                    self.scratch_docs.update(|scratch_docs| {
                        scratch_docs.insert(name, doc.clone());
                    });
                    let editor_id = editors.new_from_doc(
                        self.scope,
                        doc,
                        Some(editor_tab_id),
                        None,
                        None,
                        self.common.clone(),
                    );

                    EditorTabChild::Editor(editor_id)
                }
                EditorTabChildSource::Settings => {
                    EditorTabChild::Settings(SettingsId::next())
                }
                EditorTabChildSource::ThemeColorSettings => {
                    EditorTabChild::ThemeColorSettings(SettingsId::next())
                }
                EditorTabChildSource::Keymap => {
                    EditorTabChild::Keymap(KeymapId::next())
                }
                EditorTabChildSource::Volt(id) => {
                    EditorTabChild::Volt(VoltViewId::next(), id.to_owned())
                }
                EditorTabChildSource::DiffEditor { left, right } => {
                    let diff_editor_id = DiffEditorId::next();
                    let diff_editor = DiffEditorData::new(
                        self.scope,
                        diff_editor_id,
                        editor_tab_id,
                        left.clone(),
                        right.clone(),
                        editors,
                        self.common.clone(),
                    );
                    self.diff_editors.update(|diff_editors| {
                        diff_editors.insert(diff_editor_id, diff_editor);
                    });
                    EditorTabChild::DiffEditor(diff_editor_id)
                }
            };

        if let Some(selected) = selected {
            let (editor_tab_id, current_child) =
                active_editor_tab.with_untracked(|editor_tab| {
                    let editor_tab_id = editor_tab.editor_tab_id;
                    let (_, _, current_child) = &editor_tab.children[selected];
                    match current_child {
                        EditorTabChild::Editor(editor_id) => {
                            if let Some(editor) =
                                editors.editor_untracked(*editor_id)
                            {
                                editor.save_doc_position();
                            }
                        }
                        EditorTabChild::DiffEditor(_) => {}
                        EditorTabChild::Settings(_) => {}
                        EditorTabChild::ThemeColorSettings(_) => {}
                        EditorTabChild::Keymap(_) => {}
                        EditorTabChild::Volt(_, _) => {}
                    }
                    (editor_tab_id, current_child.clone())
                });

            // firstly, if they are the same type of child, load the new doc to the old editor
            let is_same = match (&current_child, &source) {
                (
                    EditorTabChild::Editor(editor_id),
                    EditorTabChildSource::Editor { path, doc },
                ) => {
                    if let Some(editor) = editors.editor_untracked(*editor_id) {
                        let same_path = editor
                            .doc()
                            .content
                            .with_untracked(|content| content.path() == Some(path));
                        if !same_path {
                            editor.update_doc(doc.clone());
                            editor.cursor().set(Cursor::origin(
                                self.common.config.with_untracked(|c| c.core.modal),
                            ));
                        }
                    }

                    true
                }
                (
                    EditorTabChild::DiffEditor(diff_editor_id),
                    EditorTabChildSource::DiffEditor { left, right },
                ) => {
                    if !is_same_diff_editor(diff_editor_id, left, right) {
                        if let Some(diff_editor) = diff_editors.get(diff_editor_id) {
                            diff_editor.left.update_doc(left.clone());
                            diff_editor.right.update_doc(right.clone());
                        }
                    }
                    true
                }
                (EditorTabChild::Settings(_), EditorTabChildSource::Settings) => {
                    true
                }
                _ => false,
            };
            if is_same {
                let (_, _, child) = active_editor_tab.with_untracked(|editor_tab| {
                    editor_tab.children[selected].clone()
                });
                active_editor_tab.update(|editor_tab| {
                    editor_tab.active = selected;
                });
                return child;
            }

            // We're loading a different kind of child, clean up the old resources
            match &current_child {
                EditorTabChild::Editor(editor_id) => {
                    self.remove_editor(*editor_id);
                }
                EditorTabChild::DiffEditor(diff_editor_id) => {
                    self.diff_editors.update(|diff_editors| {
                        diff_editors.remove(diff_editor_id);
                    });
                }
                EditorTabChild::Settings(_) => {}
                EditorTabChild::ThemeColorSettings(_) => {}
                EditorTabChild::Keymap(_) => {}
                EditorTabChild::Volt(_, _) => {}
            }

            // Now loading the new child
            let child = new_child_from_source(editor_tab_id, &source);
            active_editor_tab.update(|editor_tab| {
                editor_tab.children[selected] = (
                    editor_tab.scope.create_rw_signal(0),
                    editor_tab.scope.create_rw_signal(Rect::ZERO),
                    child.clone(),
                );
                editor_tab.active = selected;
            });
            return child;
        }

        // check file exists in non active editor tabs
        if config.editor.show_tab && !ignore_unconfirmed && !same_editor_tab {
            for (editor_tab_id, editor_tab) in &editor_tabs {
                if Some(*editor_tab_id) != active_editor_tab_id {
                    if let Some(index) =
                        editor_tab.with_untracked(|editor_tab| match &source {
                            EditorTabChildSource::Editor { path, .. } => editor_tab
                                .get_editor(editors, path)
                                .map(|(index, _)| index),
                            EditorTabChildSource::DiffEditor { left, right } => {
                                editor_tab.children.iter().position(
                                    |(_, _, child)| {
                                        if let EditorTabChild::DiffEditor(
                                            diff_editor_id,
                                        ) = child
                                        {
                                            is_same_diff_editor(
                                                diff_editor_id,
                                                left,
                                                right,
                                            )
                                        } else {
                                            false
                                        }
                                    },
                                )
                            }
                            EditorTabChildSource::Settings => editor_tab
                                .children
                                .iter()
                                .position(|(_, _, child)| {
                                    matches!(child, EditorTabChild::Settings(_))
                                }),
                            EditorTabChildSource::ThemeColorSettings => editor_tab
                                .children
                                .iter()
                                .position(|(_, _, child)| {
                                    matches!(
                                        child,
                                        EditorTabChild::ThemeColorSettings(_)
                                    )
                                }),
                            EditorTabChildSource::Keymap => editor_tab
                                .children
                                .iter()
                                .position(|(_, _, child)| {
                                    matches!(child, EditorTabChild::Keymap(_))
                                }),
                            EditorTabChildSource::Volt(id) => editor_tab
                                .children
                                .iter()
                                .position(|(_, _, child)| {
                                    if let EditorTabChild::Volt(_, current_id) =
                                        child
                                    {
                                        current_id == id
                                    } else {
                                        false
                                    }
                                }),
                            EditorTabChildSource::NewFileEditor => None,
                        })
                    {
                        self.active_editor_tab.set(Some(*editor_tab_id));
                        editor_tab.update(|editor_tab| {
                            editor_tab.active = index;
                        });
                        let (_, _, child) =
                            editor_tab.with_untracked(|editor_tab| {
                                editor_tab.children[index].clone()
                            });
                        return child;
                    }
                }
            }
        }

        let editor_tab_id =
            active_editor_tab.with_untracked(|editor_tab| editor_tab.editor_tab_id);
        let child = new_child_from_source(editor_tab_id, &source);

        active_editor_tab.update(|editor_tab| {
            let active = editor_tab
                .active
                .min(editor_tab.children.len().saturating_sub(1));
            let new_active = if editor_tab.children.is_empty() {
                0
            } else {
                active + 1
            };
            editor_tab.children.insert(
                new_active,
                (
                    self.scope.create_rw_signal(0),
                    self.scope.create_rw_signal(Rect::ZERO),
                    child.clone(),
                ),
            );
            editor_tab.active = new_active;
        });

        child
    }

    pub fn remove_editor(&self, editor_id: EditorId) {
        if let Some(editor) = self.editors.remove(editor_id) {
            editor.save_doc_position();

            let doc = editor.doc();
            let (content, _) = (doc.content.get_untracked(), doc.is_pristine());
            if let DocContent::Scratch { name, .. } = content {
                let doc_exists = self.editors.with_editors_untracked(|editors| {
                    editors.iter().any(|(_, editor_data)| {
                        let doc = editor_data.doc();

                        if let DocContent::Scratch {
                            name: current_name, ..
                        } = &doc.content.get_untracked()
                        {
                            current_name == &name
                        } else {
                            false
                        }
                    })
                });
                if !doc_exists {
                    self.scratch_docs.update(|scratch_docs| {
                        scratch_docs.remove(&name);
                    });
                }
            }
        }
    }
}

impl MainSplitData {
    fn editor_tab_remove(&self, editor_tab_id: EditorTabId) -> Option<()> {
        let editor_tabs = self.editor_tabs.get_untracked();
        let editor_tab = editor_tabs.get(&editor_tab_id).copied()?;
        self.editor_tabs.update(|editor_tabs| {
            editor_tabs.remove(&editor_tab_id);
        });

        let split_id = editor_tab.with_untracked(|editor_tab| editor_tab.split);
        let splits = self.splits.get_untracked();
        let split = splits.get(&split_id).copied()?;
        let parent_split_id = split.with_untracked(|split| split.parent_split);

        let is_active = self
            .active_editor_tab
            .with_untracked(|active| active == &Some(editor_tab_id));

        let index = split.with_untracked(|split| {
            split
                .children
                .iter()
                .position(|(_, c)| c == &SplitContent::EditorTab(editor_tab_id))
        })?;
        split.update(|split| {
            split.children.remove(index);
            for (size, _) in &split.children {
                size.set(1.0);
            }
        });
        let split_children = split.with_untracked(|split| split.children.clone());

        if is_active {
            if split_children.is_empty() {
                let new_focus = parent_split_id
                    .and_then(|split_id| splits.get(&split_id))
                    .and_then(|split| {
                        let index = split.with_untracked(|split| {
                            split.children.iter().position(|(_, c)| {
                                c == &SplitContent::Split(split_id)
                            })
                        })?;
                        Some((index, split))
                    })
                    .and_then(|(index, split)| {
                        split.with_untracked(|split| {
                            if index == split.children.len() - 1 {
                                if index > 0 {
                                    Some(split.children[index - 1])
                                } else {
                                    None
                                }
                            } else {
                                Some(split.children[index + 1])
                            }
                        })
                    });
                if let Some((_, content)) = new_focus {
                    self.split_content_focus(&content);
                }
            } else {
                let (_, content) =
                    split_children[index.min(split_children.len() - 1)];
                self.split_content_focus(&content);
            }
        }

        if split_children.is_empty() {
            self.split_remove(split_id);
        } else if split_children.len() == 1 {
            if let Some(parent_split_id) = parent_split_id {
                let parent_split = splits.get(&parent_split_id).copied()?;
                let split_index = parent_split.with_untracked(|split| {
                    split.content_index(&SplitContent::Split(split_id))
                })?;
                let (_, orphan) =
                    split.try_update(|split| split.children.remove(0)).unwrap();
                self.split_content_set_parent(&orphan, parent_split_id);
                parent_split.update(|parent_split| {
                    let size = parent_split.children[split_index].0.get_untracked();
                    parent_split.children[split_index] =
                        (parent_split.scope.create_rw_signal(size), orphan);
                });
                self.split_remove(split_id);
            }
        }

        Some(())
    }

    pub fn editor_tab_close(&self, editor_tab_id: EditorTabId) -> Option<()> {
        let editor_tabs = self.editor_tabs.get_untracked();
        let editor_tab = editor_tabs.get(&editor_tab_id).copied()?;
        let editor_tab = editor_tab.get_untracked();
        for (_, _, child) in editor_tab.children {
            self.editor_tab_child_close(editor_tab_id, child, false);
        }

        Some(())
    }

    fn editor_tab_child_close_warning(
        &self,
        child: &EditorTabChild,
    ) -> Option<(String, Rc<Doc>, EditorData)> {
        match child {
            EditorTabChild::Editor(editor_id) => {
                let editor = self.editors.editor_untracked(*editor_id)?;
                let doc = editor.doc();
                let doc_content = doc.content.get_untracked();
                let is_dirty = !doc.is_pristine();
                if is_dirty {
                    let exists = self.editors.with_editors_untracked(|editors| {
                        editors.iter().any(|(id, editor)| {
                            let doc = editor.doc();
                            id != editor_id
                                && doc.content.with_untracked(|content| {
                                    content == &doc_content
                                })
                        })
                    });
                    if !exists {
                        return match doc_content {
                            DocContent::File { path, .. } => Some((
                                path.file_name()?.to_str()?.to_string(),
                                doc,
                                editor,
                            )),
                            DocContent::Local => None,
                            DocContent::History(_) => None,
                            DocContent::Scratch { name, .. } => {
                                Some((name, doc, editor))
                            }
                        };
                    }
                }
                None
            }
            EditorTabChild::DiffEditor(_) => None,
            EditorTabChild::Settings(_) => None,
            EditorTabChild::ThemeColorSettings(_) => None,
            EditorTabChild::Keymap(_) => None,
            EditorTabChild::Volt(_, _) => None,
        }
    }
}

impl MainSplitData {
    pub fn editor_tab_child_close_active(&self) -> Option<()> {
        let active_editor_tab = self.active_editor_tab.get_untracked()?;
        let editor_tab = self.editor_tabs.with_untracked(|editor_tabs| {
            editor_tabs.get(&active_editor_tab).copied()
        })?;
        let (_, _, child) = editor_tab.with_untracked(|editor_tab| {
            editor_tab.children.get(editor_tab.active).cloned()
        })?;
        self.editor_tab_child_close(active_editor_tab, child, false);
        Some(())
    }

    pub fn editor_tab_child_close_by_kind(
        &self,
        editor_tab_id: EditorTabId,
        child: EditorTabChild,
        kind: TabCloseKind,
    ) -> Option<()> {
        let tabs_to_close: Vec<EditorTabChild> = {
            let editor_tabs = self.editor_tabs.get_untracked();

            let editor_tab = editor_tabs.get(&editor_tab_id).copied()?;
            let editor_tab = editor_tab.get_untracked();
            match kind {
                TabCloseKind::CloseOther => editor_tab
                    .children
                    .iter()
                    .filter_map(|x| {
                        if x.2 != child {
                            Some(x.2.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                TabCloseKind::CloseToLeft => {
                    let mut tabs_to_close = Vec::new();
                    for child_tab in &editor_tab.children {
                        if child_tab.2 != child {
                            tabs_to_close.push(child_tab.2.clone());
                        } else {
                            break;
                        }
                    }
                    tabs_to_close
                }
                TabCloseKind::CloseToRight => {
                    let mut tabs_to_close = Vec::new();
                    let mut add_to_tabs = false;
                    for child_tab in &editor_tab.children {
                        if child_tab.2 != child && add_to_tabs {
                            tabs_to_close.push(child_tab.2.clone());
                        } else {
                            add_to_tabs = true;
                        }
                    }
                    tabs_to_close
                }
            }
        };
        for child_tab in tabs_to_close {
            self.editor_tab_child_close(editor_tab_id, child_tab, false);
        }
        Some(())
    }

    pub fn editor_tab_child_close(
        &self,
        editor_tab_id: EditorTabId,
        child: EditorTabChild,
        force: bool,
    ) -> Option<()> {
        if !force {
            if let Some((name, doc, editor)) =
                self.editor_tab_child_close_warning(&child)
            {
                let internal_command = self.common.internal_command;
                let main_split = self.clone();

                let doc_content = doc.content.get_untracked();
                let save_button = match doc_content {
                    DocContent::Scratch { .. } => {
                        let child = child.clone();
                        let doc = doc.clone();
                        let save_action = Rc::new(move || {
                            let child = child.clone();
                            let main_split = main_split.clone();
                            let doc = doc.clone();
                            internal_command.send(InternalCommand::HideAlert);
                            save_as(
                                FileDialogOptions::new().title("Save File"),
                                move |file: Option<FileInfo>| {
                                    let main_split = main_split.clone();
                                    let child = child.clone();
                                    let local_main_split = main_split.clone();
                                    if let Some(mut file) = file {
                                        main_split.save_as(
                                            doc.clone(),
                                            if let Some(path) = file.path.pop() {
                                                path
                                            } else {
                                                tracing::error!("No path");
                                                return;
                                            },
                                            move || {
                                                local_main_split
                                                    .clone()
                                                    .editor_tab_child_close(
                                                        editor_tab_id,
                                                        child.clone(),
                                                        false,
                                                    );
                                            },
                                        );
                                    }
                                },
                            );
                        });
                        Some(AlertButton {
                            text: "Save".to_string(),
                            action: save_action,
                        })
                    }
                    DocContent::File { .. } => {
                        let editor = editor.clone();
                        let editors = self.editors;
                        let editor_id = editor.id();
                        let save_action = Rc::new(move || {
                            internal_command.send(InternalCommand::HideAlert);
                            editor.save(false, move || {
                                if let Some(editor) =
                                    editors.editor_untracked(editor_id)
                                {
                                    editor.clone().run_focus_command(
                                        &FocusCommand::SplitClose,
                                        None,
                                        Modifiers::empty(),
                                    );
                                }
                            });
                        });
                        Some(AlertButton {
                            text: "Save".to_string(),
                            action: save_action,
                        })
                    }
                    DocContent::Local => None,
                    DocContent::History(_) => None,
                };
                if let Some(save_button) = save_button {
                    let main_split = self.clone();
                    let child = child.clone();
                    self.common
                        .internal_command
                        .send(InternalCommand::ShowAlert {
                            title: format!(
                                "Do you want to save the changes you made to {name}?"
                            ),
                            msg: "Your changes will be lost if you don't save them."
                                .to_string(),
                            buttons: vec![
                                save_button,
                                AlertButton {
                                    text: "Don't Save".to_string(),
                                    action: Rc::new(move || {
                                        internal_command
                                            .send(InternalCommand::HideAlert);
                                        main_split.editor_tab_child_close(
                                            editor_tab_id,
                                            child.clone(),
                                            true,
                                        );
                                    }),
                                },
                            ],
                        });
                }

                return Some(());
            }
        }

        let editor_tabs = self.editor_tabs.get_untracked();
        let editor_tab = editor_tabs.get(&editor_tab_id).copied()?;

        let index = editor_tab.with_untracked(|editor_tab| {
            editor_tab.children.iter().position(|(_, _, c)| c == &child)
        })?;

        let editor_tab_children_len = editor_tab
            .try_update(|editor_tab| {
                editor_tab.children.remove(index);
                editor_tab.active =
                    index.min(editor_tab.children.len().saturating_sub(1));
                editor_tab.children.len()
            })
            .unwrap();

        match child {
            EditorTabChild::Editor(editor_id) => {
                self.remove_editor(editor_id);
            }
            EditorTabChild::DiffEditor(diff_editor_id) => {
                let removed_diff_editor = self
                    .diff_editors
                    .try_update(|diff_editors| diff_editors.remove(&diff_editor_id))
                    .unwrap();
                if let Some(diff_editor) = removed_diff_editor {
                    diff_editor.right.save_doc_position();
                    self.editors.remove(diff_editor.right.editor.id());
                    self.editors.remove(diff_editor.left.editor.id());
                }
            }
            EditorTabChild::Settings(_) => {}
            EditorTabChild::ThemeColorSettings(_) => {}
            EditorTabChild::Keymap(_) => {}
            EditorTabChild::Volt(_, _) => {}
        }

        if editor_tab_children_len == 0 {
            self.editor_tab_remove(editor_tab_id);
        }

        Some(())
    }

    pub fn editor_tab_update_layout(
        &self,
        editor_tab_id: &EditorTabId,
        window_origin: Option<Point>,
        rect: Option<Rect>,
    ) -> Option<()> {
        let editor_tabs = self.editor_tabs.get_untracked();
        let editor_tab = editor_tabs.get(editor_tab_id).copied()?;
        editor_tab.update(|editor_tab| {
            if let Some(window_origin) = window_origin {
                editor_tab.window_origin = window_origin;
            }
            if let Some(rect) = rect {
                editor_tab.layout_rect = rect;
            }
        });
        Some(())
    }
}

impl MainSplitData {
    pub fn move_editor_tab_child(
        &self,
        from_tab: EditorTabId,
        to_tab: EditorTabId,
        from_index: usize,
        to_index: usize,
    ) -> Option<()> {
        let from_editor_tab = self
            .editor_tabs
            .with_untracked(|editor_tabs| editor_tabs.get(&from_tab).cloned())?;

        if from_tab == to_tab {
            if from_index == to_index {
                return Some(());
            }

            let to_index = if from_index < to_index {
                to_index - 1
            } else {
                to_index
            };

            from_editor_tab.update(|tab| {
                let child = tab.children.remove(from_index);
                tab.children.insert(to_index, child);
                tab.active = to_index;
            });
        } else {
            let to_editor_tab = self
                .editor_tabs
                .with_untracked(|editor_tabs| editor_tabs.get(&to_tab).cloned())?;

            let (_, _, child) = from_editor_tab
                .try_update(|tab| {
                    let child = tab.children.remove(from_index);
                    tab.active =
                        tab.active.min(tab.children.len().saturating_sub(1));
                    child
                })
                .unwrap();

            self.editor_tab_child_set_parent(&child, to_tab);
            to_editor_tab.update(|tab| {
                tab.children.insert(
                    to_index,
                    (
                        tab.scope.create_rw_signal(to_index),
                        tab.scope.create_rw_signal(Rect::ZERO),
                        child,
                    ),
                );
                tab.active = to_index;
            });
            self.active_editor_tab.set(Some(to_tab));

            if from_editor_tab.with_untracked(|tab| tab.children.is_empty()) {
                self.editor_tab_remove(from_tab);
            }
        }

        Some(())
    }

    pub(super) fn split_content_set_parent(
        &self,
        content: &SplitContent,
        parent_split_id: SplitId,
    ) -> Option<()> {
        match content {
            SplitContent::EditorTab(editor_tab_id) => {
                let editor_tab = self.editor_tabs.with_untracked(|editor_tabs| {
                    editor_tabs.get(editor_tab_id).copied()
                })?;
                editor_tab.update(|editor_tab| {
                    editor_tab.split = parent_split_id;
                });
            }
            SplitContent::Split(split_id) => {
                let split = self
                    .splits
                    .with_untracked(|splits| splits.get(split_id).copied())?;
                split.update(|split| {
                    split.parent_split = Some(parent_split_id);
                });
            }
        }
        Some(())
    }

    fn editor_tab_child_set_parent(
        &self,
        child: &EditorTabChild,
        editor_tab_id: EditorTabId,
    ) -> Option<()> {
        match child {
            EditorTabChild::Editor(editor_id) => {
                let editor = self.editors.editor(*editor_id)?;
                editor.editor_tab_id.set(Some(editor_tab_id));
            }
            EditorTabChild::DiffEditor(diff_editor_id) => {
                let diff_editor =
                    self.diff_editors.with_untracked(|diff_editors| {
                        diff_editors.get(diff_editor_id).cloned()
                    })?;
                diff_editor.editor_tab_id.set(editor_tab_id);
                diff_editor
                    .left
                    .diff_editor_id
                    .set(Some((editor_tab_id, *diff_editor_id)));
                diff_editor
                    .right
                    .diff_editor_id
                    .set(Some((editor_tab_id, *diff_editor_id)));
            }
            EditorTabChild::Settings(_) => {}
            EditorTabChild::ThemeColorSettings(_) => {}
            EditorTabChild::Keymap(_) => {}
            EditorTabChild::Volt(_, _) => {}
        }
        Some(())
    }

    pub fn move_editor_tab_child_to_new_split(
        &self,
        from_tab: EditorTabId,
        from_index: usize,
        to_tab: EditorTabId,
        split: SplitMoveDirection,
    ) -> Option<()> {
        let from_editor_tab = self
            .editor_tabs
            .with_untracked(|editor_tabs| editor_tabs.get(&from_tab).cloned())?;
        let to_editor_tab = self
            .editor_tabs
            .with_untracked(|editor_tabs| editor_tabs.get(&to_tab).cloned())?;
        let to_split_id =
            to_editor_tab.with_untracked(|editor_tab| editor_tab.split);
        let to_split = self
            .splits
            .with_untracked(|splits| splits.get(&to_split_id).cloned())?;
        {
            let to_split_direction =
                to_split.with_untracked(|split| split.direction);
            if split.direction() != to_split_direction {
                let children_len =
                    to_split.with_untracked(|split| split.children.len());
                if children_len <= 1 {
                    to_split.update(|to_split| {
                        to_split.direction = split.direction();
                    });
                }
            }
        }

        let to_split_direction = to_split.with_untracked(|split| split.direction);
        if split.direction() == to_split_direction {
            let index =
                to_split.with_untracked(|split| split.editor_tab_index(to_tab))?;
            let index = match split {
                SplitMoveDirection::Up => index,
                SplitMoveDirection::Down => index + 1,
                SplitMoveDirection::Right => index + 1,
                SplitMoveDirection::Left => index,
            };
            let new_editor_tab_id = EditorTabId::next();

            let (_, _, child) = from_editor_tab
                .try_update(|tab| {
                    let child = tab.children.remove(from_index);
                    tab.active =
                        tab.active.min(tab.children.len().saturating_sub(1));
                    child
                })
                .unwrap();

            self.editor_tab_child_set_parent(&child, new_editor_tab_id);

            let cx = self.scope.create_child();
            let new_editor_tab = EditorTabData {
                scope: cx,
                split: to_split_id,
                editor_tab_id: new_editor_tab_id,
                active: 0,
                children: vec![(
                    cx.create_rw_signal(0),
                    cx.create_rw_signal(Rect::ZERO),
                    child,
                )],
                window_origin: Point::ZERO,
                layout_rect: Rect::ZERO,
                locations: cx.create_rw_signal(im::Vector::new()),
                current_location: cx.create_rw_signal(0),
            };
            self.editor_tabs.update(|editor_tabs| {
                editor_tabs.insert(
                    new_editor_tab.editor_tab_id,
                    new_editor_tab.scope.create_rw_signal(new_editor_tab),
                );
            });
            to_split.update(|split| {
                for (size, _) in &split.children {
                    size.set(1.0);
                }
                split.children.insert(
                    index,
                    (
                        split.scope.create_rw_signal(1.0),
                        SplitContent::EditorTab(new_editor_tab_id),
                    ),
                );
            });
            self.active_editor_tab.set(Some(new_editor_tab_id));
        } else {
            let index =
                to_split.with_untracked(|split| split.editor_tab_index(to_tab))?;
            let (size, existing_editor_tab) = to_split
                .try_update(|split| split.children.remove(index))
                .unwrap();

            let new_split_id = SplitId::next();

            if let SplitContent::EditorTab(existing_editor_tab_id) =
                existing_editor_tab
            {
                let editor_tab = self.editor_tabs.with_untracked(|editor_tabs| {
                    editor_tabs.get(&existing_editor_tab_id).cloned()
                })?;
                editor_tab.update(|editor_tab| {
                    editor_tab.split = new_split_id;
                });
            }

            let new_editor_tab_id = EditorTabId::next();

            let (_, _, child) = from_editor_tab
                .try_update(|tab| {
                    let child = tab.children.remove(from_index);
                    tab.active =
                        tab.active.min(tab.children.len().saturating_sub(1));
                    child
                })
                .unwrap();
            self.editor_tab_child_set_parent(&child, new_editor_tab_id);

            let new_editor_tab = {
                let cx = self.scope.create_child();
                EditorTabData {
                    scope: cx,
                    split: new_split_id,
                    editor_tab_id: new_editor_tab_id,
                    active: 0,
                    children: vec![(
                        cx.create_rw_signal(0),
                        cx.create_rw_signal(Rect::ZERO),
                        child,
                    )],
                    window_origin: Point::ZERO,
                    layout_rect: Rect::ZERO,
                    locations: cx.create_rw_signal(im::Vector::new()),
                    current_location: cx.create_rw_signal(0),
                }
            };
            self.editor_tabs.update(|editor_tabs| {
                editor_tabs.insert(
                    new_editor_tab.editor_tab_id,
                    new_editor_tab.scope.create_rw_signal(new_editor_tab),
                );
            });

            let scope = self.scope.create_child();
            let new_split_children = match split {
                SplitMoveDirection::Up | SplitMoveDirection::Left => {
                    vec![
                        (
                            scope.create_rw_signal(1.0),
                            SplitContent::EditorTab(new_editor_tab_id),
                        ),
                        (scope.create_rw_signal(1.0), existing_editor_tab),
                    ]
                }
                SplitMoveDirection::Down | SplitMoveDirection::Right => {
                    vec![
                        (scope.create_rw_signal(1.0), existing_editor_tab),
                        (
                            scope.create_rw_signal(1.0),
                            SplitContent::EditorTab(new_editor_tab_id),
                        ),
                    ]
                }
            };
            let new_split = SplitData {
                scope,
                split_id: new_split_id,
                parent_split: Some(to_split_id),
                children: new_split_children,
                direction: split.direction(),
                window_origin: Point::ZERO,
                layout_rect: Rect::ZERO,
            };
            self.splits.update(|splits| {
                splits.insert(
                    new_split_id,
                    new_split.scope.create_rw_signal(new_split),
                );
            });
            to_split.update(|split| {
                split.children.insert(
                    index,
                    (
                        split.scope.create_rw_signal(size.get_untracked()),
                        SplitContent::Split(new_split_id),
                    ),
                );
            });
            self.active_editor_tab.set(Some(new_editor_tab_id));
        }

        if from_editor_tab.with_untracked(|tab| tab.children.is_empty()) {
            self.editor_tab_remove(from_tab);
        }

        Some(())
    }
}
