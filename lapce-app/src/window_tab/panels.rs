//! Panel visibility and focus toggling of a window tab.

use floem::reactive::{SignalGet, SignalUpdate, SignalWith};

use super::{Focus, WindowTabData};
use crate::panel::{kind::PanelKind, position::PanelContainerPosition};

impl WindowTabData {
    pub fn toggle_panel_visual(&self, kind: PanelKind) {
        if self.panel.is_panel_visible(&kind) {
            self.hide_panel(kind);
        } else {
            self.show_panel(kind);
        }
    }

    /// Toggle a specific kind of panel.
    pub(super) fn toggle_panel_focus(&self, kind: PanelKind) {
        let should_hide = match kind {
            PanelKind::FileExplorer
            | PanelKind::Plugin
            | PanelKind::Problem
            | PanelKind::Debug
            | PanelKind::CallHierarchy
            | PanelKind::DocumentSymbol
            | PanelKind::References
            | PanelKind::Implementation => {
                // Some panels don't accept focus (yet). Fall back to visibility check
                // in those cases.
                self.panel.is_panel_visible(&kind)
            }
            PanelKind::Terminal
            | PanelKind::SourceControl
            | PanelKind::Search
            | PanelKind::Agent => self.is_panel_focused(kind),
        };
        if should_hide {
            self.hide_panel(kind);
        } else {
            self.show_panel(kind);
        }
    }

    /// Toggle a panel on one of the sides.
    pub(super) fn toggle_container_visual(&self, position: &PanelContainerPosition) {
        let shown = !self.panel.is_container_shown(position, false);
        self.panel.set_shown(&position.first(), shown);
        self.panel.set_shown(&position.second(), shown);

        if shown {
            if let Some((kind, _)) = self
                .panel
                .active_panel_at_position(&position.second(), false)
            {
                self.show_panel(kind);
            }

            if let Some((kind, _)) = self
                .panel
                .active_panel_at_position(&position.first(), false)
            {
                self.show_panel(kind);
            }
        } else {
            if let Some((kind, _)) = self
                .panel
                .active_panel_at_position(&position.second(), false)
            {
                self.hide_panel(kind);
            }

            if let Some((kind, _)) = self
                .panel
                .active_panel_at_position(&position.first(), false)
            {
                self.hide_panel(kind);
            }
        }
    }

    fn is_panel_focused(&self, kind: PanelKind) -> bool {
        // Moving between e.g. Search and Problems doesn't affect focus, so we need to also check
        // visibility.
        self.common.focus.get_untracked() == Focus::Panel(kind)
            && self.panel.is_panel_visible(&kind)
    }

    pub(super) fn hide_panel(&self, kind: PanelKind) {
        self.panel.hide_panel(&kind);
        self.common.focus.set(Focus::Workbench);
    }

    pub fn show_panel(&self, kind: PanelKind) {
        if kind == PanelKind::Terminal
            && self
                .terminal
                .tab_info
                .with_untracked(|info| info.tabs.is_empty())
        {
            self.terminal.new_tab(
                self.common
                    .config
                    .get_untracked()
                    .terminal
                    .get_default_profile(),
            );
        }
        self.panel.show_panel(&kind);
        if kind == PanelKind::Search
            && self.common.focus.get_untracked() == Focus::Workbench
        {
            let active_editor = self.main_split.active_editor.get_untracked();
            let word = active_editor.map(|editor| editor.word_at_cursor());
            if let Some(word) = word {
                if !word.is_empty() {
                    self.global_search.set_pattern(word);
                }
            }
        }
        self.common.focus.set(Focus::Panel(kind));
    }
}
