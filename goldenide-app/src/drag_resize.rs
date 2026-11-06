//! Shared helpers for resize handles (panel borders, split borders).
//!
//! Two things made a border drag jerky:
//!
//! 1. floem runs a full style and layout pass after every pointer event and a
//!    mouse reports hundreds of events per second, so the resize is applied at
//!    most once per [`DRAG_APPLY_INTERVAL`] ([`DragThrottle`]).
//! 2. floem reports `PointerMove` positions relative to the handle, and the
//!    handle moves while the border follows the pointer. A shift computed from
//!    such positions depends on when the last layout ran, so the border
//!    oscillated. The shift is computed from window coordinates instead
//!    ([`window_pos`]) against a size measured when the drag started.

use std::{cell::Cell, rc::Rc, time::Duration};

use floem::{ViewId, action::exec_after, kurbo::Point};

/// Minimum time between two applications of a resize while a border is
/// dragged: about one frame at 60 Hz.
pub const DRAG_APPLY_INTERVAL: Duration = Duration::from_millis(16);

/// Converts `pos`, a pointer position local to the handle `view_id`, into
/// window coordinates. It is stable while the handle moves, unlike `pos`.
pub fn window_pos(view_id: ViewId, pos: Point) -> Point {
    let origin = view_id.layout_rect().origin();
    Point::new(pos.x + origin.x, pos.y + origin.y)
}

/// The size a pane gets when its border moved from `start_pos` to `pos`
/// (window coordinates, along one axis) and the pane measured `start_size`
/// when the drag started. `grows_with_pointer` is false for a pane that sits
/// after the border (right and bottom panels), which shrinks when the border
/// moves forward.
pub fn dragged_size(
    start_size: f64,
    start_pos: f64,
    pos: f64,
    grows_with_pointer: bool,
) -> f64 {
    let shift = pos - start_pos;
    if grows_with_pointer {
        start_size + shift
    } else {
        start_size - shift
    }
}

/// Keeps only the last pointer position received and hands it to `apply` at
/// most once per [`DRAG_APPLY_INTERVAL`].
#[derive(Clone)]
pub struct DragThrottle {
    pending: Rc<Cell<Option<Point>>>,
    timer_set: Rc<Cell<bool>>,
    apply: Rc<dyn Fn(Point)>,
}

impl DragThrottle {
    /// Creates a throttle that calls `apply` with the last stored position.
    pub fn new(apply: impl Fn(Point) + 'static) -> Self {
        Self {
            pending: Rc::new(Cell::new(None)),
            timer_set: Rc::new(Cell::new(false)),
            apply: Rc::new(apply),
        }
    }

    /// Stores `pos` and schedules the application if none is scheduled yet.
    pub fn push(&self, pos: Point) {
        self.pending.set(Some(pos));
        if self.timer_set.replace(true) {
            return;
        }
        let this = self.clone();
        exec_after(DRAG_APPLY_INTERVAL, move |_| {
            this.timer_set.set(false);
            this.flush();
        });
    }

    /// Applies the stored position now, if any. Call it when the drag ends.
    pub fn flush(&self) {
        if let Some(pos) = self.pending.take() {
            (self.apply)(pos);
        }
    }

    /// Drops the stored position without applying it.
    pub fn cancel(&self) {
        self.pending.set(None);
    }
}

#[cfg(test)]
mod tests {
    use super::dragged_size;

    #[test]
    fn a_pane_before_the_border_grows_with_the_pointer() {
        assert_eq!(dragged_size(200.0, 100.0, 130.0, true), 230.0);
        assert_eq!(dragged_size(200.0, 100.0, 70.0, true), 170.0);
    }

    #[test]
    fn a_pane_after_the_border_shrinks_when_the_pointer_moves_forward() {
        assert_eq!(dragged_size(200.0, 100.0, 130.0, false), 170.0);
        assert_eq!(dragged_size(200.0, 100.0, 70.0, false), 230.0);
    }
}
