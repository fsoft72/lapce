//! Split operations of the main split: create, move, exchange, focus and remove splits.

use floem::{
    peniko::kurbo::{Point, Rect},
    reactive::{RwSignal, Scope, SignalGet, SignalUpdate, SignalWith},
};

use super::{
    MainSplitData, SplitContent, SplitData, SplitDirection, SplitMoveDirection,
};
use crate::{
    editor_tab::{EditorTabChild, EditorTabData},
    id::{
        DiffEditorId, EditorTabId, KeymapId, SettingsId, SplitId,
        ThemeColorSettingsId, VoltViewId,
    },
};

impl MainSplitData {
    pub fn split(
        &self,
        direction: SplitDirection,
        editor_tab_id: EditorTabId,
    ) -> Option<()> {
        let editor_tabs = self.editor_tabs.get_untracked();
        let editor_tab = editor_tabs.get(&editor_tab_id).copied()?;

        let split_id = editor_tab.with_untracked(|editor_tab| editor_tab.split);
        let splits = self.splits.get_untracked();
        let split = splits.get(&split_id).copied()?;
        let split_direction = split.with_untracked(|split| split.direction);

        let (index, children_len) = split.with_untracked(|split| {
            split
                .children
                .iter()
                .position(|(_, c)| c == &SplitContent::EditorTab(editor_tab_id))
                .map(|index| (index, split.children.len()))
        })?;

        if split_direction == direction {
            let new_editor_tab = editor_tab.with_untracked(|editor_tab| {
                self.split_editor_tab(self.scope, split_id, editor_tab)
            })?;
            let new_editor_tab_id =
                new_editor_tab.with_untracked(|editor_tab| editor_tab.editor_tab_id);
            split.update(|split| {
                for (size, _) in &split.children {
                    size.set(1.0);
                }
                split.children.insert(
                    index + 1,
                    (
                        split.scope.create_rw_signal(1.0),
                        SplitContent::EditorTab(new_editor_tab_id),
                    ),
                );
            });
        } else if children_len == 1 {
            let new_editor_tab = editor_tab.with_untracked(|editor_tab| {
                self.split_editor_tab(self.scope, split_id, editor_tab)
            })?;
            let new_editor_tab_id =
                new_editor_tab.with_untracked(|editor_tab| editor_tab.editor_tab_id);
            split.update(|split| {
                split.direction = direction;
                for (size, _) in &split.children {
                    size.set(1.0);
                }
                split.children.push((
                    split.scope.create_rw_signal(1.0),
                    SplitContent::EditorTab(new_editor_tab_id),
                ));
            });
        } else {
            let new_split_id = SplitId::next();

            editor_tab.update(|editor_tab| {
                editor_tab.split = new_split_id;
            });
            let new_editor_tab = editor_tab.with_untracked(|editor_tab| {
                self.split_editor_tab(self.scope, new_split_id, editor_tab)
            })?;
            let new_editor_tab_id =
                new_editor_tab.with_untracked(|editor_tab| editor_tab.editor_tab_id);

            let new_split = {
                let cx = self.scope.create_child();
                let new_split = SplitData {
                    scope: cx,
                    parent_split: Some(split_id),
                    split_id: new_split_id,
                    children: vec![
                        (
                            cx.create_rw_signal(1.0),
                            SplitContent::EditorTab(editor_tab_id),
                        ),
                        (
                            cx.create_rw_signal(1.0),
                            SplitContent::EditorTab(new_editor_tab_id),
                        ),
                    ],
                    direction,
                    window_origin: Point::ZERO,
                    layout_rect: Rect::ZERO,
                };
                cx.create_rw_signal(new_split)
            };
            self.splits.update(|splits| {
                splits.insert(new_split_id, new_split);
            });
            split.update(|split| {
                let size = split.children[index].0.get_untracked();
                split.children[index] = (
                    split.scope.create_rw_signal(size),
                    SplitContent::Split(new_split_id),
                );
            });
        }

        Some(())
    }

    fn split_editor_tab(
        &self,
        cx: Scope,
        split_id: SplitId,
        editor_tab: &EditorTabData,
    ) -> Option<RwSignal<EditorTabData>> {
        let (_, _, child) = editor_tab.children.get(editor_tab.active)?;

        let editor_tab_id = EditorTabId::next();

        let new_child = match child {
            EditorTabChild::Editor(editor_id) => {
                let editor_id = self
                    .editors
                    .copy(*editor_id, cx, Some(editor_tab_id), None, None)
                    .unwrap();

                EditorTabChild::Editor(editor_id)
            }
            EditorTabChild::DiffEditor(diff_editor_id) => {
                let new_diff_editor_id = DiffEditorId::next();
                let diff_editor = self
                    .diff_editors
                    .get_untracked()
                    .get(diff_editor_id)?
                    .copy(cx, editor_tab_id, new_diff_editor_id, self.editors);
                self.diff_editors.update(|diff_editors| {
                    diff_editors.insert(new_diff_editor_id, diff_editor);
                });
                EditorTabChild::DiffEditor(new_diff_editor_id)
            }
            EditorTabChild::Settings(_) => {
                EditorTabChild::Settings(SettingsId::next())
            }
            EditorTabChild::ThemeColorSettings(_) => {
                EditorTabChild::ThemeColorSettings(ThemeColorSettingsId::next())
            }
            EditorTabChild::Keymap(_) => EditorTabChild::Keymap(KeymapId::next()),
            EditorTabChild::Volt(_, id) => {
                EditorTabChild::Volt(VoltViewId::next(), id.to_owned())
            }
        };

        let editor_tab = {
            let cx = self.scope.create_child();
            let editor_tab = EditorTabData {
                scope: cx,
                split: split_id,
                editor_tab_id,
                active: 0,
                children: vec![(
                    cx.create_rw_signal(0),
                    cx.create_rw_signal(Rect::ZERO),
                    new_child,
                )],
                window_origin: Point::ZERO,
                layout_rect: Rect::ZERO,
                locations: cx.create_rw_signal(editor_tab.locations.get_untracked()),
                current_location: cx
                    .create_rw_signal(editor_tab.current_location.get_untracked()),
            };
            cx.create_rw_signal(editor_tab)
        };
        self.editor_tabs.update(|editor_tabs| {
            editor_tabs.insert(editor_tab_id, editor_tab);
        });
        Some(editor_tab)
    }

    pub fn split_move(
        &self,
        direction: SplitMoveDirection,
        editor_tab_id: EditorTabId,
    ) -> Option<()> {
        let editor_tabs = self.editor_tabs.get_untracked();
        let editor_tab = editor_tabs.get(&editor_tab_id).copied()?;

        let rect = editor_tab.with_untracked(|editor_tab| {
            editor_tab.layout_rect.with_origin(editor_tab.window_origin)
        });

        match direction {
            SplitMoveDirection::Up => {
                for (_, e) in editor_tabs.iter() {
                    let current_rect = e.with_untracked(|e| {
                        e.layout_rect.with_origin(e.window_origin)
                    });
                    if (current_rect.y1 - rect.y0).abs() < 3.0
                        && current_rect.x0 <= rect.x0
                        && rect.x0 < current_rect.x1
                    {
                        let new_editor_tab_id =
                            e.with_untracked(|e| e.editor_tab_id);
                        self.active_editor_tab.set(Some(new_editor_tab_id));
                        return Some(());
                    }
                }
            }
            SplitMoveDirection::Down => {
                for (_, e) in editor_tabs.iter() {
                    let current_rect = e.with_untracked(|e| {
                        e.layout_rect.with_origin(e.window_origin)
                    });
                    if (current_rect.y0 - rect.y1).abs() < 3.0
                        && current_rect.x0 <= rect.x0
                        && rect.x0 < current_rect.x1
                    {
                        let new_editor_tab_id =
                            e.with_untracked(|e| e.editor_tab_id);
                        self.active_editor_tab.set(Some(new_editor_tab_id));
                        return Some(());
                    }
                }
            }
            SplitMoveDirection::Right => {
                for (_, e) in editor_tabs.iter() {
                    let current_rect = e.with_untracked(|e| {
                        e.layout_rect.with_origin(e.window_origin)
                    });
                    if (rect.x1 - current_rect.x0).abs() < 3.0
                        && current_rect.y0 <= rect.y0
                        && rect.y0 < current_rect.y1
                    {
                        let new_editor_tab_id =
                            e.with_untracked(|e| e.editor_tab_id);
                        self.active_editor_tab.set(Some(new_editor_tab_id));
                        return Some(());
                    }
                }
            }
            SplitMoveDirection::Left => {
                for (_, e) in editor_tabs.iter() {
                    let current_rect = e.with_untracked(|e| {
                        e.layout_rect.with_origin(e.window_origin)
                    });
                    if (current_rect.x1 - rect.x0).abs() < 3.0
                        && current_rect.y0 <= rect.y0
                        && rect.y0 < current_rect.y1
                    {
                        let new_editor_tab_id =
                            e.with_untracked(|e| e.editor_tab_id);
                        self.active_editor_tab.set(Some(new_editor_tab_id));
                        return Some(());
                    }
                }
            }
        }

        Some(())
    }

    pub fn split_exchange(&self, editor_tab_id: EditorTabId) -> Option<()> {
        let editor_tabs = self.editor_tabs.get_untracked();
        let editor_tab = editor_tabs.get(&editor_tab_id).copied()?;

        let split_id = editor_tab.with_untracked(|editor_tab| editor_tab.split);
        let splits = self.splits.get_untracked();
        let split = splits.get(&split_id).copied()?;

        split.update(|split| {
            let index = split
                .children
                .iter()
                .position(|(_, c)| c == &SplitContent::EditorTab(editor_tab_id));
            if let Some(index) = index {
                if index < split.children.len() - 1 {
                    split.children.swap(index, index + 1);
                }
                self.split_content_focus(&split.children[index].1);
            }
        });

        Some(())
    }

    pub(super) fn split_content_focus(&self, content: &SplitContent) {
        match content {
            SplitContent::EditorTab(editor_tab_id) => {
                self.active_editor_tab.set(Some(*editor_tab_id));
            }
            SplitContent::Split(split_id) => {
                self.split_focus(*split_id);
            }
        }
    }

    fn split_focus(&self, split_id: SplitId) -> Option<()> {
        let splits = self.splits.get_untracked();
        let split = splits.get(&split_id).copied()?;

        let split_children = split.with_untracked(|split| split.children.clone());
        let content = split_children.first()?;
        self.split_content_focus(&content.1);

        Some(())
    }

    pub(super) fn split_remove(&self, split_id: SplitId) -> Option<()> {
        if split_id == self.root_split {
            return Some(());
        }

        let splits = self.splits.get_untracked();
        let split = splits.get(&split_id).copied()?;
        self.splits.update(|splits| {
            splits.remove(&split_id);
        });
        let parent_split_id = split.with_untracked(|split| split.parent_split)?;
        let parent_split = splits.get(&parent_split_id).copied()?;

        let split_len = parent_split
            .try_update(|split| {
                split
                    .children
                    .retain(|(_, c)| c != &SplitContent::Split(split_id));
                split.children.len()
            })
            .unwrap();
        if split_len == 0 {
            self.split_remove(parent_split_id);
        } else if split_len == 1 {
            let parent_parent_split_id =
                parent_split.with_untracked(|split| split.parent_split)?;
            let parent_parent_split =
                splits.get(&parent_parent_split_id).copied()?;
            let parent_split_index =
                parent_parent_split.with_untracked(|split| {
                    split.content_index(&SplitContent::Split(parent_split_id))
                })?;
            let orphan = parent_split
                .try_update(|split| split.children.remove(0))
                .unwrap();
            self.split_content_set_parent(&orphan.1, parent_parent_split_id);
            parent_parent_split.update(|parent_parent_split| {
                parent_parent_split.children[parent_split_index] = orphan;
            });
            self.split_remove(parent_split_id);
        }

        Some(())
    }
}

impl MainSplitData {
    pub fn split_exchange_active(&self) -> Option<()> {
        let active_editor_tab = self.active_editor_tab.get_untracked()?;
        self.split_exchange(active_editor_tab)?;
        Some(())
    }

    pub fn split_move_active(&self, direction: SplitMoveDirection) -> Option<()> {
        let active_editor_tab = self.active_editor_tab.get_untracked()?;
        self.split_move(direction, active_editor_tab)?;
        Some(())
    }

    pub fn split_active(&self, direction: SplitDirection) -> Option<()> {
        let active_editor_tab = self.active_editor_tab.get_untracked()?;
        self.split(direction, active_editor_tab)?;
        Some(())
    }
}
