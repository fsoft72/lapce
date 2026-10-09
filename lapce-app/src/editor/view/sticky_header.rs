//! Computes the sticky header lines shown at the top of the editor.

use floem::{
    peniko::kurbo::Rect,
    reactive::{RwSignal, SignalGet, SignalUpdate},
};

use super::StickyHeaderInfo;
use crate::config::LapceConfig;
use crate::editor::EditorData;

pub(super) fn get_sticky_header_info(
    editor_data: &EditorData,
    viewport: RwSignal<Rect>,
    sticky_header_height_signal: RwSignal<f64>,
    config: &LapceConfig,
) -> StickyHeaderInfo {
    let editor = &editor_data.editor;
    let doc = editor_data.doc();

    let viewport = viewport.get();
    // TODO(minor): should this be a `get`
    let screen_lines = editor.screen_lines.get();
    let line_height = config.editor.line_height() as f64;
    // let start_line = (viewport.y0 / line_height).floor() as usize;
    let Some(start) = screen_lines.lines.first() else {
        return StickyHeaderInfo {
            sticky_lines: Vec::new(),
            last_sticky_should_scroll: false,
            y_diff: 0.0,
        };
    };
    let start_info = screen_lines.info(*start).unwrap();
    let start_line = start_info.vline_info.rvline.line;

    let y_diff = viewport.y0 - start_info.vline_y;

    let mut last_sticky_should_scroll = false;
    let mut sticky_lines = Vec::new();
    if let Some(lines) = doc.sticky_headers(start_line) {
        let total_lines = lines.len();
        if total_lines > 0 {
            let line = start_line + total_lines;
            if let Some(new_lines) = doc.sticky_headers(line) {
                if new_lines.len() > total_lines {
                    sticky_lines = new_lines;
                } else {
                    sticky_lines = lines;
                    last_sticky_should_scroll = new_lines.len() < total_lines;
                    if new_lines.len() < total_lines {
                        if let Some(new_new_lines) =
                            doc.sticky_headers(start_line + total_lines - 1)
                        {
                            if new_new_lines.len() < total_lines {
                                sticky_lines.pop();
                                last_sticky_should_scroll = false;
                            }
                        } else {
                            sticky_lines.pop();
                            last_sticky_should_scroll = false;
                        }
                    }
                }
            } else {
                sticky_lines = lines;
                last_sticky_should_scroll = true;
            }
        }
    }

    let total_sticky_lines = sticky_lines.len();

    let paint_last_line = total_sticky_lines > 0
        && (last_sticky_should_scroll
            || y_diff != 0.0
            || start_line + total_sticky_lines - 1 != *sticky_lines.last().unwrap());

    // Fix up the line count in case we don't need to paint the last one.
    let total_sticky_lines = if paint_last_line {
        total_sticky_lines
    } else {
        total_sticky_lines.saturating_sub(1)
    };

    if total_sticky_lines == 0 {
        sticky_header_height_signal.set(0.0);
        return StickyHeaderInfo {
            sticky_lines: Vec::new(),
            last_sticky_should_scroll: false,
            y_diff: 0.0,
        };
    }

    let scroll_offset = if last_sticky_should_scroll {
        y_diff
    } else {
        0.0
    };

    let sticky_header_height = sticky_lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            // TODO(question): won't y_diff always be scroll_offset here? so we should just sub on
            // the outside
            let y_diff = if i == total_sticky_lines - 1 {
                scroll_offset
            } else {
                0.0
            };

            let layout = editor.text_layout(*line);
            layout.line_count() as f64 * line_height - y_diff
        })
        .sum();

    sticky_header_height_signal.set(sticky_header_height);
    StickyHeaderInfo {
        sticky_lines,
        last_sticky_should_scroll,
        y_diff,
    }
}
