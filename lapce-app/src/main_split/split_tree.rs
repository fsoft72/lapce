//! The split tree: split directions, split contents and their persisted form.

use floem::{
    peniko::kurbo::{Point, Rect},
    reactive::{RwSignal, Scope, SignalGet, SignalUpdate, SignalWith},
};
use serde::{Deserialize, Serialize};

use super::MainSplitData;
use crate::{
    editor_tab::EditorTabInfo,
    id::{EditorTabId, SplitId},
    window_tab::WindowTabData,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDirection {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug)]
pub enum SplitMoveDirection {
    Up,
    Down,
    Right,
    Left,
}

impl SplitMoveDirection {
    pub fn direction(&self) -> SplitDirection {
        match self {
            SplitMoveDirection::Up | SplitMoveDirection::Down => {
                SplitDirection::Horizontal
            }
            SplitMoveDirection::Left | SplitMoveDirection::Right => {
                SplitDirection::Vertical
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitContent {
    EditorTab(EditorTabId),
    Split(SplitId),
}

impl SplitContent {
    pub fn id(&self) -> u64 {
        match self {
            SplitContent::EditorTab(id) => id.to_raw(),
            SplitContent::Split(id) => id.to_raw(),
        }
    }

    pub fn content_info(&self, data: &WindowTabData) -> SplitContentInfo {
        match &self {
            SplitContent::EditorTab(editor_tab_id) => {
                let editor_tab_data = data
                    .main_split
                    .editor_tabs
                    .get_untracked()
                    .get(editor_tab_id)
                    .cloned()
                    .unwrap();
                SplitContentInfo::EditorTab(
                    editor_tab_data.get_untracked().tab_info(data),
                )
            }
            SplitContent::Split(split_id) => {
                let split_data = data
                    .main_split
                    .splits
                    .get_untracked()
                    .get(split_id)
                    .cloned()
                    .unwrap();
                SplitContentInfo::Split(split_data.get_untracked().split_info(data))
            }
        }
    }
}

#[derive(Clone)]
pub struct SplitData {
    pub scope: Scope,
    pub parent_split: Option<SplitId>,
    pub split_id: SplitId,
    pub children: Vec<(RwSignal<f64>, SplitContent)>,
    pub direction: SplitDirection,
    pub window_origin: Point,
    pub layout_rect: Rect,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SplitInfo {
    pub children: Vec<SplitContentInfo>,
    pub direction: SplitDirection,
}

impl SplitInfo {
    pub fn to_data(
        &self,
        data: MainSplitData,
        parent_split: Option<SplitId>,
        split_id: SplitId,
    ) -> RwSignal<SplitData> {
        let split_data = {
            let cx = data.scope.create_child();
            let split_data = SplitData {
                scope: cx,
                split_id,
                direction: self.direction,
                parent_split,
                children: self
                    .children
                    .iter()
                    .map(|child| {
                        (
                            cx.create_rw_signal(1.0),
                            child.to_data(data.clone(), split_id),
                        )
                    })
                    .collect(),
                window_origin: Point::ZERO,
                layout_rect: Rect::ZERO,
            };
            cx.create_rw_signal(split_data)
        };
        data.splits.update(|splits| {
            splits.insert(split_id, split_data);
        });
        split_data
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum SplitContentInfo {
    EditorTab(EditorTabInfo),
    Split(SplitInfo),
}

impl SplitContentInfo {
    pub fn to_data(
        &self,
        data: MainSplitData,
        parent_split: SplitId,
    ) -> SplitContent {
        match &self {
            SplitContentInfo::EditorTab(tab_info) => {
                let tab_data = tab_info.to_data(data, parent_split);
                SplitContent::EditorTab(
                    tab_data.with_untracked(|tab_data| tab_data.editor_tab_id),
                )
            }
            SplitContentInfo::Split(split_info) => {
                let split_id = SplitId::next();
                split_info.to_data(data, Some(parent_split), split_id);
                SplitContent::Split(split_id)
            }
        }
    }
}

impl SplitData {
    pub fn split_info(&self, data: &WindowTabData) -> SplitInfo {
        SplitInfo {
            direction: self.direction,
            children: self
                .children
                .iter()
                .map(|(_, child)| child.content_info(data))
                .collect(),
        }
    }

    pub fn editor_tab_index(&self, editor_tab_id: EditorTabId) -> Option<usize> {
        self.children
            .iter()
            .position(|(_, c)| c == &SplitContent::EditorTab(editor_tab_id))
    }

    pub fn content_index(&self, content: &SplitContent) -> Option<usize> {
        self.children.iter().position(|(_, c)| c == content)
    }
}
