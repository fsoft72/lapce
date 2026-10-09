//! Screen origins of the hover, completion, code action and rename popups.

use floem::{
    peniko::kurbo::{Point, Vec2},
    reactive::SignalGet,
};
use lapce_core::cursor::CursorAffinity;

use super::WindowTabData;
use crate::{code_action::CodeActionStatus, completion::CompletionStatus};

impl WindowTabData {
    pub fn hover_origin(&self) -> Option<Point> {
        if !self.common.hover.active.get_untracked() {
            return None;
        }

        let editor_id = self.common.hover.editor_id.get_untracked();
        let editor_data = self.main_split.editors.editor(editor_id)?;

        let (window_origin, viewport, editor) = (
            editor_data.window_origin(),
            editor_data.viewport(),
            &editor_data.editor,
        );

        // TODO(minor): affinity should be gotten from where the hover was started at.
        let (point_above, point_below) = editor.points_of_offset(
            self.common.hover.offset.get_untracked(),
            CursorAffinity::Forward,
        );

        let window_origin =
            window_origin.get() - self.common.window_origin.get().to_vec2();
        let viewport = viewport.get();
        let hover_size = self.common.hover.layout_rect.get().size();
        let tab_size = self.layout_rect.get().size();

        let mut origin = window_origin
            + Vec2::new(
                point_below.x - viewport.x0,
                (point_above.y - viewport.y0) - hover_size.height,
            );
        if origin.y < 0.0 {
            origin.y = window_origin.y + point_below.y - viewport.y0;
        }
        if origin.x + hover_size.width + 1.0 > tab_size.width {
            origin.x = tab_size.width - hover_size.width - 1.0;
        }
        if origin.x <= 0.0 {
            origin.x = 0.0;
        }

        Some(origin)
    }

    pub fn completion_origin(&self) -> Point {
        let completion = self.common.completion.get();
        if completion.status == CompletionStatus::Inactive {
            return Point::ZERO;
        }
        let config = self.common.config.get();
        let editor_data =
            if let Some(editor) = self.main_split.active_editor.get_untracked() {
                editor
            } else {
                return Point::ZERO;
            };

        let (window_origin, viewport, editor) = (
            editor_data.window_origin(),
            editor_data.viewport(),
            &editor_data.editor,
        );

        // TODO(minor): What affinity should we use for this? Probably just use the cursor's
        // original affinity..
        let (point_above, point_below) =
            editor.points_of_offset(completion.offset, CursorAffinity::Forward);

        let window_origin =
            window_origin.get() - self.common.window_origin.get().to_vec2();
        let viewport = viewport.get();
        let completion_size = completion.layout_rect.size();
        let tab_size = self.layout_rect.get().size();

        let mut origin = window_origin
            + Vec2::new(
                point_below.x
                    - viewport.x0
                    - config.editor.line_height() as f64
                    - 5.0,
                point_below.y - viewport.y0,
            );
        if origin.y + completion_size.height > tab_size.height {
            origin.y = window_origin.y + (point_above.y - viewport.y0)
                - completion_size.height;
        }
        if origin.x + completion_size.width + 1.0 > tab_size.width {
            origin.x = tab_size.width - completion_size.width - 1.0;
        }
        if origin.x <= 0.0 {
            origin.x = 0.0;
        }

        origin
    }

    pub fn code_action_origin(&self) -> Point {
        let code_action = self.code_action.get();
        let config = self.common.config.get();
        if code_action.status.get_untracked() == CodeActionStatus::Inactive {
            return Point::ZERO;
        }

        let tab_size = self.layout_rect.get().size();
        let code_action_size = code_action.layout_rect.size();

        let editor_data =
            if let Some(editor) = self.main_split.active_editor.get_untracked() {
                editor
            } else {
                return Point::ZERO;
            };

        let (window_origin, viewport, editor) = (
            editor_data.window_origin(),
            editor_data.viewport(),
            &editor_data.editor,
        );

        // TODO(minor): What affinity should we use for this?
        let (_point_above, point_below) =
            editor.points_of_offset(code_action.offset, CursorAffinity::Forward);

        let window_origin =
            window_origin.get() - self.common.window_origin.get().to_vec2();
        let viewport = viewport.get();

        let mut origin = window_origin
            + Vec2::new(
                if code_action.mouse_click {
                    0.0
                } else {
                    point_below.x - viewport.x0
                },
                point_below.y - viewport.y0,
            );

        if origin.y + code_action_size.height > tab_size.height {
            origin.y = origin.y
                - config.editor.line_height() as f64
                - code_action_size.height;
        }
        if origin.x + code_action_size.width + 1.0 > tab_size.width {
            origin.x = tab_size.width - code_action_size.width - 1.0;
        }
        if origin.x <= 0.0 {
            origin.x = 0.0;
        }

        origin
    }

    pub fn rename_origin(&self) -> Point {
        let config = self.common.config.get();
        if !self.rename.active.get() {
            return Point::ZERO;
        }

        let tab_size = self.layout_rect.get().size();
        let rename_size = self.rename.layout_rect.get().size();

        let editor_data =
            if let Some(editor) = self.main_split.active_editor.get_untracked() {
                editor
            } else {
                return Point::ZERO;
            };

        let (window_origin, viewport, editor) = (
            editor_data.window_origin(),
            editor_data.viewport(),
            &editor_data.editor,
        );

        // TODO(minor): What affinity should we use for this?
        let (_point_above, point_below) = editor.points_of_offset(
            self.rename.start.get_untracked(),
            CursorAffinity::Forward,
        );

        let window_origin =
            window_origin.get() - self.common.window_origin.get().to_vec2();
        let viewport = viewport.get();

        let mut origin = window_origin
            + Vec2::new(point_below.x - viewport.x0, point_below.y - viewport.y0);

        if origin.y + rename_size.height > tab_size.height {
            origin.y =
                origin.y - config.editor.line_height() as f64 - rename_size.height;
        }
        if origin.x + rename_size.width + 1.0 > tab_size.width {
            origin.x = tab_size.width - rename_size.width - 1.0;
        }
        if origin.x <= 0.0 {
            origin.x = 0.0;
        }

        origin
    }
}
