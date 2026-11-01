//! Jump location history of the main split.

use std::path::PathBuf;

use floem::{
    peniko::kurbo::Vec2,
    reactive::{SignalGet, SignalUpdate, SignalWith},
};
use lsp_types::TextEdit;

use super::MainSplitData;
use crate::editor::location::{EditorLocation, EditorPosition};

impl MainSplitData {
    fn save_current_jump_location(&self) -> bool {
        if let Some(editor) = self.active_editor.get_untracked() {
            let (cursor, viewport) = (editor.cursor(), editor.viewport());
            let path = editor
                .doc()
                .content
                .with_untracked(|content| content.path().cloned());
            if let Some(path) = path {
                let offset = cursor.with_untracked(|c| c.offset());
                let scroll_offset = viewport.get_untracked().origin().to_vec2();
                return self.save_jump_location(path, offset, scroll_offset);
            }
        }
        false
    }

    pub fn save_jump_location(
        &self,
        path: PathBuf,
        offset: usize,
        scroll_offset: Vec2,
    ) -> bool {
        let mut locations = self.locations.get_untracked();
        if let Some(last_location) = locations.last() {
            if last_location.path == path
                && last_location.position == Some(EditorPosition::Offset(offset))
                && last_location.scroll_offset == Some(scroll_offset)
            {
                return false;
            }
        }
        let location = EditorLocation {
            path,
            position: Some(EditorPosition::Offset(offset)),
            scroll_offset: Some(scroll_offset),
            ignore_unconfirmed: false,
            same_editor_tab: false,
        };
        locations.push_back(location.clone());
        let current_location = locations.len();
        self.locations.set(locations);
        self.current_location.set(current_location);

        let active_editor_tab_id = self.active_editor_tab.get_untracked();
        let editor_tabs = self.editor_tabs.get_untracked();
        if let Some((locations, current_location)) = active_editor_tab_id
            .and_then(|id| editor_tabs.get(&id))
            .map(|editor_tab| {
                editor_tab.with_untracked(|editor_tab| {
                    (editor_tab.locations, editor_tab.current_location)
                })
            })
        {
            let mut l = locations.get_untracked();
            l.push_back(location);
            let new_current_location = l.len();
            locations.set(l);
            current_location.set(new_current_location);
        }

        true
    }

    pub fn jump_to_location(
        &self,
        location: EditorLocation,
        edits: Option<Vec<TextEdit>>,
    ) {
        self.save_current_jump_location();
        self.go_to_location(location, edits);
    }
}

impl MainSplitData {
    pub fn jump_location_backward(&self, local: bool) {
        let (locations, current_location) = if local {
            let active_editor_tab_id = self.active_editor_tab.get_untracked();
            let editor_tabs = self.editor_tabs.get_untracked();
            if let Some((locations, current_location)) = active_editor_tab_id
                .and_then(|id| editor_tabs.get(&id))
                .map(|editor_tab| {
                    editor_tab.with_untracked(|editor_tab| {
                        (editor_tab.locations, editor_tab.current_location)
                    })
                })
            {
                (locations, current_location)
            } else {
                return;
            }
        } else {
            (self.locations, self.current_location)
        };

        let locations_value = locations.get_untracked();
        let current_location_value = current_location.get_untracked();
        if current_location_value < 1 {
            return;
        }

        if current_location_value >= locations_value.len() {
            // if we are at the head of the locations, save the current location
            // before jump back
            if self.save_current_jump_location() {
                current_location.update(|l| {
                    *l -= 1;
                });
            }
        }

        current_location.update(|l| {
            *l -= 1;
        });
        let current_location_value = current_location.get_untracked();
        current_location.set(current_location_value);
        let mut location = locations_value[current_location_value].clone();
        // for local jumps, we keep on the same editor tab
        // because we only jump on the same split
        location.same_editor_tab = local;

        self.go_to_location(location, None);
    }

    pub fn jump_location_forward(&self, local: bool) {
        let (locations, current_location) = if local {
            let active_editor_tab_id = self.active_editor_tab.get_untracked();
            let editor_tabs = self.editor_tabs.get_untracked();
            if let Some((locations, current_location)) = active_editor_tab_id
                .and_then(|id| editor_tabs.get(&id))
                .map(|editor_tab| {
                    editor_tab.with_untracked(|editor_tab| {
                        (editor_tab.locations, editor_tab.current_location)
                    })
                })
            {
                (locations, current_location)
            } else {
                return;
            }
        } else {
            (self.locations, self.current_location)
        };

        let locations_value = locations.get_untracked();
        let current_location_value = current_location.get_untracked();
        if locations_value.is_empty() {
            return;
        }
        if current_location_value >= locations_value.len() - 1 {
            return;
        }
        current_location.set(current_location_value + 1);
        let mut location = locations_value[current_location_value + 1].clone();
        // for local jumps, we keep on the same editor tab
        // because we only jump on the same split
        location.same_editor_tab = local;
        self.go_to_location(location, None);
    }
}

impl MainSplitData {
    pub fn can_jump_location_backward(&self, tracked: bool) -> bool {
        if tracked {
            self.current_location.get() >= 1
        } else {
            self.current_location.get_untracked() >= 1
        }
    }

    pub fn can_jump_location_forward(&self, tracked: bool) -> bool {
        if tracked {
            !(self.locations.with(|l| l.is_empty())
                || self.current_location.get()
                    >= self.locations.with(|l| l.len()) - 1)
        } else {
            !(self.locations.with_untracked(|l| l.is_empty())
                || self.current_location.get_untracked()
                    >= self.locations.with_untracked(|l| l.len()) - 1)
        }
    }
}
