use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;

use super::{data::PanelOrder, position::PanelPosition};
use crate::config::icon::GoldenIdeIcons;

#[derive(
    Clone, Copy, PartialEq, Serialize, Deserialize, Hash, Eq, Debug, EnumIter,
)]
pub enum PanelKind {
    Terminal,
    FileExplorer,
    SourceControl,
    Search,
    Problem,
    Debug,
    CallHierarchy,
    DocumentSymbol,
    References,
    Implementation,
    Agent,
}

impl PanelKind {
    pub fn svg_name(&self) -> &'static str {
        match &self {
            PanelKind::Terminal => GoldenIdeIcons::TERMINAL,
            PanelKind::FileExplorer => GoldenIdeIcons::FILE_EXPLORER,
            PanelKind::SourceControl => GoldenIdeIcons::SCM,
            PanelKind::Search => GoldenIdeIcons::SEARCH,
            PanelKind::Problem => GoldenIdeIcons::PROBLEM,
            PanelKind::Debug => GoldenIdeIcons::DEBUG,
            PanelKind::CallHierarchy => GoldenIdeIcons::TYPE_HIERARCHY,
            PanelKind::DocumentSymbol => GoldenIdeIcons::DOCUMENT_SYMBOL,
            PanelKind::References => GoldenIdeIcons::REFERENCES,
            PanelKind::Implementation => GoldenIdeIcons::IMPLEMENTATION,
            PanelKind::Agent => GoldenIdeIcons::AGENT,
        }
    }

    pub fn position(&self, order: &PanelOrder) -> Option<(usize, PanelPosition)> {
        for (pos, panels) in order.iter() {
            let index = panels.iter().position(|k| k == self);
            if let Some(index) = index {
                return Some((index, *pos));
            }
        }
        None
    }

    pub fn default_position(&self) -> PanelPosition {
        match self {
            PanelKind::Terminal => PanelPosition::BottomLeft,
            PanelKind::FileExplorer => PanelPosition::LeftTop,
            PanelKind::SourceControl => PanelPosition::LeftTop,
            PanelKind::Search => PanelPosition::BottomLeft,
            PanelKind::Problem => PanelPosition::BottomLeft,
            PanelKind::Debug => PanelPosition::LeftTop,
            PanelKind::CallHierarchy => PanelPosition::BottomLeft,
            PanelKind::DocumentSymbol => PanelPosition::RightTop,
            PanelKind::References => PanelPosition::BottomLeft,
            PanelKind::Implementation => PanelPosition::BottomLeft,
            PanelKind::Agent => PanelPosition::RightTop,
        }
    }
}
