//! Rich text that can be selected with the mouse and copied with Ctrl+C.
//! Floem's own `rich_text` cannot be selected, so a canvas under it paints
//! the selection and the pointer events of the wrapper drive it.

use std::mem::swap;

use floem::{
    Clipboard, Renderer, View,
    event::{Event, EventListener, EventPropagation},
    keyboard::Key,
    kurbo::{Point, Rect, Size},
    peniko::Color,
    reactive::{SignalGet, SignalUpdate, SignalWith, create_rw_signal},
    style::Position,
    text::{Cursor, TextLayout},
    views::{Decorators, canvas, rich_text, stack},
};

/// Distance the pointer must move before a press becomes a selection, so a
/// plain click does not select.
const SELECT_MIN_DISTANCE: f64 = 2.0;

/// Where the mouse selection stands, in the coordinates of the text.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SelectionState {
    None,
    /// The button is down; nothing is selected until the pointer moves.
    Ready(Point),
    Selecting(Point, Point),
    Selected(Point, Point),
}

impl SelectionState {
    /// The two ends of the selection, if there is one.
    fn ends(self) -> Option<(Point, Point)> {
        match self {
            SelectionState::Selecting(a, b) | SelectionState::Selected(a, b) => {
                Some((a, b))
            }
            _ => None,
        }
    }
}

/// The layout the text is drawn with at `width`: wrapped when wider, as
/// floem's `rich_text` does, so positions match what is on screen.
fn wrapped_layout(layout: &TextLayout, width: f32) -> TextLayout {
    let mut wrapped = layout.clone();
    if width > 0.0 && layout.size().width as f32 > width {
        wrapped.set_size(width, f32::MAX);
    }
    wrapped
}

/// The selection as a pair of cursors with the first before the second.
fn selection_cursors(
    layout: &TextLayout,
    ends: (Point, Point),
) -> Option<(Cursor, Cursor)> {
    let mut start = layout.hit(ends.0.x as f32, ends.0.y as f32)?;
    let mut end = layout.hit(ends.1.x as f32, ends.1.y as f32)?;
    if start.line > end.line || (start.line == end.line && start.index > end.index) {
        swap(&mut start, &mut end);
    }
    Some((start, end))
}

/// The text between two cursors of `layout`.
fn selected_text(layout: &TextLayout, start: Cursor, end: Cursor) -> Option<String> {
    let ranges = layout.lines_range();
    // The line texts lack their line breaks: put each line at its offset.
    let mut text = String::new();
    for (line, range) in layout.lines().iter().zip(ranges) {
        while text.len() < range.start {
            text.push('\n');
        }
        text.push_str(line.text());
    }
    let from = ranges.get(start.line)?.start + start.index;
    let to = ranges.get(end.line)?.start + end.index;
    text.get(from..to).map(str::to_string)
}

/// Shows `text_layout` as selectable text. `selection_color` gives the
/// color of the selection each time it is painted.
pub fn selectable_text(
    text_layout: TextLayout,
    selection_color: impl Fn() -> Color + 'static,
) -> impl View {
    let state = create_rw_signal(SelectionState::None);
    let width = create_rw_signal(0.0f32);

    let highlight = canvas({
        let layout = text_layout.clone();
        move |cx, _size: Size| {
            let Some(ends) = state.get().ends() else {
                return;
            };
            let wrapped = wrapped_layout(&layout, width.get_untracked());
            let Some((start, end)) = selection_cursors(&wrapped, ends) else {
                return;
            };
            let color = selection_color();
            for run in wrapped.layout_runs() {
                if let Some((start_x, run_width)) = run.highlight(start, end) {
                    let (x, y) = (start_x as f64, run.line_top as f64);
                    let rect = Rect::new(
                        x,
                        y,
                        x + run_width as f64,
                        y + run.line_height as f64,
                    );
                    cx.fill(&rect, color, 0.0);
                }
            }
        }
    })
    .style(|s| s.position(Position::Absolute).size_pct(100.0, 100.0));

    let text = rich_text({
        let layout = text_layout.clone();
        move || layout.clone()
    })
    .style(|s| s.width_full());

    let view = stack((highlight, text))
        .keyboard_navigable()
        .style(|s| s.width_full().min_width(0.0));
    let id = view.id();
    view.on_resize(move |rect| width.set(rect.width() as f32))
        .on_event(EventListener::PointerDown, move |event| {
            if let Event::PointerDown(pe) = event {
                state.set(SelectionState::Ready(pe.pos));
            }
            EventPropagation::Continue
        })
        .on_event(EventListener::PointerMove, move |event| {
            let Event::PointerMove(pme) = event else {
                return EventPropagation::Continue;
            };
            let (SelectionState::Selecting(start, _)
            | SelectionState::Ready(start)) = state.get_untracked()
            else {
                return EventPropagation::Continue;
            };
            if start.distance(pme.pos) > SELECT_MIN_DISTANCE {
                state.set(SelectionState::Selecting(start, pme.pos));
                id.request_active();
                id.request_focus();
            }
            EventPropagation::Continue
        })
        .on_event(EventListener::PointerUp, move |_| {
            state.update(|state| {
                *state = match *state {
                    SelectionState::Selecting(a, b) => SelectionState::Selected(a, b),
                    _ => SelectionState::None,
                }
            });
            id.clear_active();
            EventPropagation::Continue
        })
        .on_event(EventListener::KeyDown, {
            move |event| {
                let Event::KeyDown(key) = event else {
                    return EventPropagation::Continue;
                };
                let is_copy = (key.modifiers.control() || key.modifiers.meta())
                    && matches!(&key.key.logical_key,
                        Key::Character(ch) if ch.eq_ignore_ascii_case("c"));
                if !is_copy {
                    return EventPropagation::Continue;
                }
                let Some(ends) = state.with_untracked(|state| state.ends()) else {
                    return EventPropagation::Continue;
                };
                let wrapped = wrapped_layout(&text_layout, width.get_untracked());
                let copied = selection_cursors(&wrapped, ends)
                    .and_then(|(start, end)| selected_text(&text_layout, start, end));
                match copied {
                    Some(text) => {
                        let _ = Clipboard::set_contents(text);
                        EventPropagation::Stop
                    }
                    None => EventPropagation::Continue,
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use floem::text::{Attrs, AttrsList};

    use super::*;

    /// A layout of `text` with default attributes.
    fn layout(text: &str) -> TextLayout {
        let mut layout = TextLayout::new();
        layout.set_text(text, AttrsList::new(Attrs::new()), None);
        layout
    }

    #[test]
    fn the_text_between_two_cursors_is_selected() {
        let layout = layout("hello world\nsecond line");
        let cursor = |line, index| Cursor::new(line, index);
        assert_eq!(
            selected_text(&layout, cursor(0, 6), cursor(0, 11)).as_deref(),
            Some("world")
        );
        assert_eq!(
            selected_text(&layout, cursor(0, 6), cursor(1, 6)).as_deref(),
            Some("world\nsecond")
        );
    }

    #[test]
    fn a_selection_dragged_backwards_is_ordered() {
        let layout = layout("hello world");
        // The text is laid out from the top left: a drag from the right end
        // to the left end selects the same text as the other way round.
        let (start, end) = selection_cursors(
            &layout,
            (Point::new(500.0, 5.0), Point::new(0.0, 5.0)),
        )
        .unwrap();
        assert!(start.index <= end.index);
    }
}
