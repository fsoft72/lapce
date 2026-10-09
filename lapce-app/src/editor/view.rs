use std::{cmp, path::PathBuf, rc::Rc, sync::Arc};

mod breadcrumbs;
mod changes;
mod find;
mod gutter_view;
mod sticky_header;
mod zoom;

use self::{
    breadcrumbs::editor_breadcrumbs,
    find::find_view,
    gutter_view::{editor_gutter, editor_gutter_folding_range},
    sticky_header::get_sticky_header_info,
    zoom::zoom_editor_font_on_wheel,
};
pub use self::{
    changes::{changes_colors_all, changes_colors_screen},
    zoom::zoom_font_on_wheel,
};

use floem::{
    Renderer, View, ViewId,
    action::{set_ime_allowed, set_ime_cursor_area},
    context::{PaintCx, StyleCx},
    event::{Event, EventListener, EventPropagation},
    kurbo::Stroke,
    peniko::{
        Color,
        kurbo::{Line, Point, Rect, Size},
    },
    prelude::SignalTrack,
    reactive::{
        Memo, ReadSignal, RwSignal, SignalGet, SignalUpdate, SignalWith,
        create_effect, create_memo, create_rw_signal,
    },
    style::{CursorColor, CursorStyle, Style, TextColor},
    taffy::prelude::NodeId,
    views::{
        Decorators,
        editor::{
            CurrentLineColor, CursorSurroundingLines, Editor, EditorStyle,
            IndentGuideColor, IndentStyleProp, Modal, ModalRelativeLine,
            PhantomColor, PlaceholderColor, PreeditUnderlineColor,
            RenderWhitespaceProp, ScrollBeyondLastLine, SelectionColor,
            ShowIndentGuide, SmartTab, VisibleWhitespaceColor, WrapProp,
            text::WrapMethod,
            view::{
                DiffSectionKind, EditorView as FloemEditorView, EditorViewClass,
                LineRegion, ScreenLines, cursor_caret,
            },
            visual_line::RVLine,
        },
        empty,
        scroll::{PropagatePointerWheel, scroll},
        stack,
    },
};
use lapce_core::{
    buffer::{Buffer, rope_text::RopeText},
    cursor::{CursorAffinity, CursorMode},
    selection::SelRegion,
};

use super::{DocSignal, EditorData};
use crate::{
    config::{LapceConfig, color::LapceColor, editor::WrapStyle},
    doc::DocContent,
    window_tab::WindowTabData,
    workspace::LapceWorkspace,
};

#[derive(Clone, Debug, Default)]
pub struct StickyHeaderInfo {
    pub sticky_lines: Vec<usize>,
    pub last_sticky_should_scroll: bool,
    pub y_diff: f64,
}

fn editor_wrap(config: &LapceConfig) -> WrapMethod {
    /// Minimum width that we'll allow the view to be wrapped at.
    const MIN_WRAPPED_WIDTH: f32 = 100.0;

    match config.editor.wrap_style {
        WrapStyle::None => WrapMethod::None,
        WrapStyle::EditorWidth => WrapMethod::EditorWidth,
        WrapStyle::WrapWidth => WrapMethod::WrapWidth {
            width: (config.editor.wrap_width as f32).max(MIN_WRAPPED_WIDTH),
        },
    }
}

pub fn editor_style(
    config: ReadSignal<Arc<LapceConfig>>,
    doc: DocSignal,
    s: Style,
) -> Style {
    let config = config.get();
    let doc = doc.get();

    s.set(
        IndentStyleProp,
        doc.buffer.with_untracked(Buffer::indent_style),
    )
    .set(CursorColor, config.color(LapceColor::EDITOR_CARET))
    .set(SelectionColor, config.color(LapceColor::EDITOR_SELECTION))
    .set(
        CurrentLineColor,
        config.color(LapceColor::EDITOR_CURRENT_LINE),
    )
    .set(
        VisibleWhitespaceColor,
        config.color(LapceColor::EDITOR_VISIBLE_WHITESPACE),
    )
    .set(
        IndentGuideColor,
        config.color(LapceColor::EDITOR_INDENT_GUIDE),
    )
    .set(ScrollBeyondLastLine, config.editor.scroll_beyond_last_line)
    .color(config.color(LapceColor::EDITOR_FOREGROUND))
    .set(TextColor, config.color(LapceColor::EDITOR_FOREGROUND))
    .set(PhantomColor, config.color(LapceColor::EDITOR_DIM))
    .set(PlaceholderColor, config.color(LapceColor::EDITOR_DIM))
    .set(
        PreeditUnderlineColor,
        config.color(LapceColor::EDITOR_FOREGROUND),
    )
    .set(ShowIndentGuide, config.editor.show_indent_guide)
    .set(Modal, config.core.modal)
    .set(
        ModalRelativeLine,
        config.editor.modal_mode_relative_line_numbers,
    )
    .set(SmartTab, config.editor.smart_tab)
    .set(WrapProp, editor_wrap(&config))
    .set(
        CursorSurroundingLines,
        config.editor.cursor_surrounding_lines,
    )
    .set(RenderWhitespaceProp, config.editor.render_whitespace)
}

pub struct EditorView {
    id: ViewId,
    editor: EditorData,
    is_active: Memo<bool>,
    inner_node: Option<NodeId>,
    viewport: RwSignal<Rect>,
    debug_breakline: Memo<Option<(usize, PathBuf)>>,
}

pub fn editor_view(
    e_data: EditorData,
    debug_breakline: Memo<Option<(usize, PathBuf)>>,
    is_active: impl Fn(bool) -> bool + 'static + Copy,
) -> EditorView {
    let id = ViewId::new();
    let is_active = create_memo(move |_| is_active(true));

    let viewport = e_data.viewport();

    let doc = e_data.doc_signal();
    let view_kind = e_data.kind;
    let screen_lines = e_data.screen_lines();
    create_effect(move |_| {
        doc.track();
        view_kind.track();
        id.request_layout();
    });

    let hide_cursor = e_data.common.window_common.hide_cursor;
    create_effect(move |_| {
        hide_cursor.track();
        let occurrences = doc.with(|doc| doc.find_result.occurrences);
        occurrences.track();
        id.request_paint();
    });

    create_effect(move |last_rev| {
        let buffer = doc.with(|doc| doc.buffer);
        let rev = buffer.with(|buffer| buffer.rev());
        if last_rev == Some(rev) {
            return rev;
        }
        id.request_layout();
        rev
    });

    let config = e_data.common.config;
    let sticky_header_height_signal = e_data.sticky_header_height;
    let editor2 = e_data.clone();
    create_effect(move |last_rev| {
        let config = config.get();
        if !config.editor.sticky_header {
            return (DocContent::Local, 0, 0, Rect::ZERO, 0, None);
        }

        let doc = doc.get();
        let rect = viewport.get();
        let (screen_lines_len, screen_lines_first) = screen_lines
            .with(|lines| (lines.lines.len(), lines.lines.first().copied()));
        let rev = (
            doc.content.get(),
            doc.buffer.with(|b| b.rev()),
            doc.cache_rev.get(),
            rect,
            screen_lines_len,
            screen_lines_first,
        );
        if last_rev.as_ref() == Some(&rev) {
            return rev;
        }

        let sticky_header_info = get_sticky_header_info(
            &editor2,
            viewport,
            sticky_header_height_signal,
            &config,
        );

        id.update_state(sticky_header_info);

        rev
    });

    let ed1 = e_data.editor.clone();
    let ed2 = ed1.clone();
    let ed3 = ed1.clone();

    let editor_window_origin = e_data.window_origin();
    let cursor = e_data.cursor();
    let find_focus = e_data.find_focus;
    let ime_allowed = e_data.common.window_common.ime_allowed;
    let editor_viewport = e_data.viewport();
    let editor_cursor = e_data.cursor();
    create_effect(move |_| {
        let active = is_active.get();
        if active && !find_focus.get() {
            if !cursor.with(|c| c.is_insert()) {
                if ime_allowed.get_untracked() {
                    ime_allowed.set(false);
                    set_ime_allowed(false);
                }
            } else {
                if !ime_allowed.get_untracked() {
                    ime_allowed.set(true);
                    set_ime_allowed(true);
                }
                let (offset, affinity) = cursor.with(|c| (c.offset(), c.affinity));
                let (_, point_below) = ed1.points_of_offset(offset, affinity);
                let window_origin = editor_window_origin.get();
                let viewport = editor_viewport.get();
                let pos = window_origin
                    + (point_below.x - viewport.x0, point_below.y - viewport.y0);
                set_ime_cursor_area(pos, Size::new(800.0, 600.0));
            }
        }
    });

    let doc = e_data.doc_signal();
    EditorView {
        id,
        editor: e_data,
        is_active,
        inner_node: None,
        viewport,
        debug_breakline,
    }
    .on_event(EventListener::ImePreedit, move |event| {
        if !is_active.get_untracked() {
            return EventPropagation::Continue;
        }

        if let Event::ImePreedit { text, cursor } = event {
            if text.is_empty() {
                ed2.clear_preedit();
            } else {
                let offset = editor_cursor.with_untracked(|c| c.offset());
                ed2.set_preedit(text.clone(), *cursor, offset);
            }
        }
        EventPropagation::Stop
    })
    .on_event(EventListener::ImeCommit, move |event| {
        if !is_active.get_untracked() {
            return EventPropagation::Continue;
        }

        if let Event::ImeCommit(text) = event {
            ed3.clear_preedit();
            ed3.receive_char(text);
        }
        EventPropagation::Stop
    })
    .class(EditorViewClass)
    .style(move |s| editor_style(config, doc, s))
}

impl EditorView {
    fn paint_diff_sections(
        &self,
        cx: &mut PaintCx,
        viewport: Rect,
        screen_lines: &ScreenLines,
        config: &LapceConfig,
    ) {
        let Some(diff_sections) = &screen_lines.diff_sections else {
            return;
        };
        for section in diff_sections.iter() {
            match section.kind {
                DiffSectionKind::NoCode => self.paint_diff_no_code(
                    cx,
                    viewport,
                    section.y_idx,
                    section.height,
                    config,
                ),
                DiffSectionKind::Added => {
                    cx.fill(
                        &Rect::ZERO
                            .with_size(Size::new(
                                viewport.width(),
                                (config.editor.line_height() * section.height)
                                    as f64,
                            ))
                            .with_origin(Point::new(
                                viewport.x0,
                                (section.y_idx * config.editor.line_height()) as f64,
                            )),
                        config
                            .color(LapceColor::SOURCE_CONTROL_ADDED)
                            .multiply_alpha(0.2),
                        0.0,
                    );
                }
                DiffSectionKind::Removed => {
                    cx.fill(
                        &Rect::ZERO
                            .with_size(Size::new(
                                viewport.width(),
                                (config.editor.line_height() * section.height)
                                    as f64,
                            ))
                            .with_origin(Point::new(
                                viewport.x0,
                                (section.y_idx * config.editor.line_height()) as f64,
                            )),
                        config
                            .color(LapceColor::SOURCE_CONTROL_REMOVED)
                            .multiply_alpha(0.2),
                        0.0,
                    );
                }
            }
        }
    }

    fn paint_diff_no_code(
        &self,
        cx: &mut PaintCx,
        viewport: Rect,
        start_line: usize,
        height: usize,
        config: &LapceConfig,
    ) {
        let line_height = config.editor.line_height();
        let height = (height * line_height) as f64;
        let y = (start_line * line_height) as f64;
        let y_end = y + height;

        if y_end < viewport.y0 || y > viewport.y1 {
            return;
        }

        let y = y.max(viewport.y0 - 10.0);
        let y_end = y_end.min(viewport.y1 + 10.0);
        let height = y_end - y;

        let start_x = viewport.x0.floor() as usize;
        let start_x = start_x - start_x % 8;

        for x in (start_x..viewport.x1.ceil() as usize + 1 + height.ceil() as usize)
            .step_by(8)
        {
            let p0 = if x as f64 > viewport.x1.ceil() {
                Point::new(viewport.x1.ceil(), y + (x as f64 - viewport.x1.ceil()))
            } else {
                Point::new(x as f64, y)
            };

            let height = if x as f64 - height < viewport.x0.floor() {
                x as f64 - viewport.x0.floor()
            } else {
                height
            };
            if height > 0.0 {
                let p1 = Point::new(x as f64 - height, y + height);
                cx.stroke(
                    &Line::new(p0, p1),
                    config.color(LapceColor::EDITOR_DIM),
                    &Stroke::new(1.0),
                );
            }
        }
    }

    fn paint_current_line(
        &self,
        cx: &mut PaintCx,
        is_local: bool,
        screen_lines: &ScreenLines,
    ) {
        let e_data = &self.editor;
        let ed = &e_data.editor;
        let cursor = self.editor.cursor();
        let config = self.editor.common.config;

        let config = config.get_untracked();
        let line_height = config.editor.line_height() as f64;
        let viewport = self.viewport.get_untracked();

        let current_line_color = ed.es.with_untracked(EditorStyle::current_line);

        let breakline = self.debug_breakline.with_untracked(|breakline| {
            let (breakline, breakline_path) = breakline.as_ref()?;
            e_data
                .doc()
                .content
                .with_untracked(|c| c.path() == Some(breakline_path))
                .then_some(*breakline)
        });

        // TODO: check if this is correct
        if let Some(breakline) = breakline {
            if let Some(info) = screen_lines.info_for_line(breakline) {
                let rect = Rect::from_origin_size(
                    (viewport.x0, info.vline_y),
                    (viewport.width(), line_height),
                );

                cx.fill(
                    &rect,
                    config.color(LapceColor::EDITOR_DEBUG_BREAK_LINE),
                    0.0,
                );
            }
        }

        cursor.with_untracked(|cursor| {
            let highlight_current_line = match cursor.mode {
                CursorMode::Normal(_) | CursorMode::Insert(_) => true,
                CursorMode::Visual { .. } => false,
            };

            if let Some(current_line_color) = current_line_color {
                // Highlight the current line
                if !is_local && highlight_current_line {
                    for (_, end) in cursor.regions_iter() {
                        // TODO: unsure if this is correct for wrapping lines
                        let rvline = ed.rvline_of_offset(end, cursor.affinity);

                        if let Some(info) = screen_lines
                            .info(rvline)
                            .filter(|_| Some(rvline.line) != breakline)
                        {
                            let rect = Rect::from_origin_size(
                                (viewport.x0, info.vline_y),
                                (viewport.width(), line_height),
                            );

                            cx.fill(&rect, current_line_color, 0.0);
                        }
                    }
                }
            }
        });
    }

    fn paint_find(&self, cx: &mut PaintCx, screen_lines: &ScreenLines) {
        let find_visual = self.editor.common.find.visual.get_untracked();
        if !find_visual && self.editor.on_screen_find.with_untracked(|f| !f.active) {
            return;
        }
        if screen_lines.lines.is_empty() {
            return;
        }

        let min_vline = *screen_lines.lines.first().unwrap();
        let max_vline = *screen_lines.lines.last().unwrap();
        let min_line = screen_lines.info(min_vline).unwrap().vline_info.rvline.line;
        let max_line = screen_lines.info(max_vline).unwrap().vline_info.rvline.line;

        let e_data = &self.editor;
        let ed = &e_data.editor;
        let doc = e_data.doc();

        let config = self.editor.common.config;
        let occurrences = doc.find_result.occurrences;

        let config = config.get_untracked();
        let line_height = config.editor.line_height() as f64;
        let color = config.color(LapceColor::EDITOR_FOREGROUND);

        let start = ed.offset_of_line(min_line);
        let end = ed.offset_of_line(max_line + 1);

        // TODO: The selection rect creation logic for find is quite similar to the version
        // within insert cursor. It would be good to deduplicate it.
        if find_visual {
            doc.update_find();
            occurrences.with_untracked(|selection| {
                for region in selection.regions_in_range(start, end) {
                    self.paint_find_region(
                        cx,
                        ed,
                        region,
                        color,
                        screen_lines,
                        line_height,
                    );
                }
            });
        }

        self.editor.on_screen_find.with_untracked(|find| {
            if find.active {
                for region in &find.regions {
                    self.paint_find_region(
                        cx,
                        ed,
                        region,
                        color,
                        screen_lines,
                        line_height,
                    );
                }
            }
        });
    }

    fn paint_find_region(
        &self,
        cx: &mut PaintCx,
        ed: &Editor,
        region: &SelRegion,
        color: Color,
        screen_lines: &ScreenLines,
        line_height: f64,
    ) {
        let start = region.min();
        let end = region.max();

        // TODO(minor): the proper affinity here should probably be tracked by selregion
        let (start_rvline, start_col) =
            ed.rvline_col_of_offset(start, CursorAffinity::Forward);
        let (end_rvline, end_col) =
            ed.rvline_col_of_offset(end, CursorAffinity::Backward);

        for line_info in screen_lines.iter_line_info() {
            let rvline_info = line_info.vline_info;
            let rvline = rvline_info.rvline;
            let line = rvline.line;

            if rvline < start_rvline {
                continue;
            }

            if rvline > end_rvline {
                break;
            }

            let left_col = if rvline == start_rvline { start_col } else { 0 };
            let (right_col, _vline_end) = if rvline == end_rvline {
                let max_col = ed.last_col(rvline_info, true);
                (end_col.min(max_col), false)
            } else {
                (ed.last_col(rvline_info, true), true)
            };

            // TODO(minor): sel region should have the affinity of the start/end
            let x0 = ed
                .line_point_of_line_col(
                    line,
                    left_col,
                    CursorAffinity::Forward,
                    true,
                )
                .x;
            let x1 = ed
                .line_point_of_line_col(
                    line,
                    right_col,
                    CursorAffinity::Backward,
                    true,
                )
                .x;

            if !rvline_info.is_empty() && start != end && left_col != right_col {
                let rect = Size::new(x1 - x0, line_height)
                    .to_rect()
                    .with_origin(Point::new(x0, line_info.vline_y));
                cx.stroke(&rect, color, &Stroke::new(1.0));
            }
        }
    }

    fn paint_sticky_headers(
        &self,
        cx: &mut PaintCx,
        viewport: Rect,
        screen_lines: &ScreenLines,
    ) {
        let config = self.editor.common.config.get_untracked();
        if !config.editor.sticky_header {
            return;
        }
        if !self.editor.kind.get_untracked().is_normal() {
            return;
        }

        let line_height = config.editor.line_height();
        let Some(start_vline) = screen_lines.lines.first() else {
            return;
        };
        let start_info = screen_lines.vline_info(*start_vline).unwrap();
        let start_line = start_info.rvline.line;

        self.editor
            .sticky_header_info
            .with_untracked(|sticky_header_info| {
                self.paint_sticky_lines(
                    cx,
                    viewport,
                    line_height,
                    start_line,
                    sticky_header_info,
                );
            });
    }

    /// Paints the sticky header lines of `sticky_header_info` over the top of the viewport.
    fn paint_sticky_lines(
        &self,
        cx: &mut PaintCx,
        viewport: Rect,
        line_height: usize,
        start_line: usize,
        sticky_header_info: &StickyHeaderInfo,
    ) {
        let config = self.editor.common.config.get_untracked();
        let total_sticky_lines = sticky_header_info.sticky_lines.len();

        let paint_last_line = total_sticky_lines > 0
            && (sticky_header_info.last_sticky_should_scroll
                || sticky_header_info.y_diff != 0.0
                || start_line + total_sticky_lines - 1
                    != *sticky_header_info.sticky_lines.last().unwrap());

        let total_sticky_lines = if paint_last_line {
            total_sticky_lines
        } else {
            total_sticky_lines.saturating_sub(1)
        };

        if total_sticky_lines == 0 {
            return;
        }

        let scroll_offset = if sticky_header_info.last_sticky_should_scroll {
            sticky_header_info.y_diff
        } else {
            0.0
        };

        // Clear background

        let area_height = sticky_header_info
            .sticky_lines
            .iter()
            .copied()
            .map(|line| {
                let layout = self.editor.editor.text_layout(line);
                layout.line_count() * line_height
            })
            .sum::<usize>() as f64
            - scroll_offset;

        let sticky_area_rect = Size::new(viewport.x1, area_height)
            .to_rect()
            .with_origin(Point::new(0.0, viewport.y0))
            .inflate(10.0, 0.0);

        cx.fill(
            &sticky_area_rect,
            config.color(LapceColor::LAPCE_DROPDOWN_SHADOW),
            3.0,
        );
        cx.fill(
            &sticky_area_rect,
            config.color(LapceColor::EDITOR_STICKY_HEADER_BACKGROUND),
            0.0,
        );
        // Paint lines
        let mut y_accum = 0.0;
        for (i, line) in sticky_header_info.sticky_lines.iter().copied().enumerate()
        {
            let y_diff = if i == total_sticky_lines - 1 {
                scroll_offset
            } else {
                0.0
            };

            let text_layout = self.editor.editor.text_layout(line);

            let text_height = (text_layout.line_count() * line_height) as f64;
            let height = text_height - y_diff;

            cx.save();

            let line_area_rect = Size::new(viewport.width(), height)
                .to_rect()
                .with_origin(Point::new(viewport.x0, viewport.y0 + y_accum));

            cx.clip(&line_area_rect);

            let y = viewport.y0 - y_diff + y_accum;
            cx.draw_text(&text_layout.text, Point::new(viewport.x0, y));

            y_accum += text_height;

            cx.restore();
        }
    }

    fn paint_scroll_bar(
        &self,
        cx: &mut PaintCx,
        viewport: Rect,
        is_local: bool,
        config: Arc<LapceConfig>,
    ) {
        const BAR_WIDTH: f64 = 10.0;

        if is_local {
            return;
        }

        cx.fill(
            &Rect::ZERO
                .with_size(Size::new(1.0, viewport.height()))
                .with_origin(Point::new(
                    viewport.x0 + viewport.width() - BAR_WIDTH,
                    viewport.y0,
                ))
                .inflate(0.0, 10.0),
            config.color(LapceColor::LAPCE_SCROLL_BAR),
            0.0,
        );

        if !self.editor.kind.get_untracked().is_normal() {
            return;
        }

        let doc = self.editor.doc();
        let total_len = doc.buffer.with_untracked(|buffer| buffer.last_line());
        let changes = doc.head_changes().get_untracked();
        let total_height = viewport.height();
        let total_width = viewport.width();
        let line_height = config.editor.line_height();
        let content_height = if config.editor.scroll_beyond_last_line {
            (total_len * line_height) as f64 + total_height - line_height as f64
        } else {
            (total_len * line_height) as f64
        };

        let colors = changes_colors_all(&config, &self.editor.editor, changes);
        for (y, height, _, color) in colors {
            let y = y / content_height * total_height;
            let height = ((height * line_height) as f64 / content_height
                * total_height)
                .max(3.0);
            let rect = Rect::ZERO.with_size(Size::new(3.0, height)).with_origin(
                Point::new(
                    viewport.x0 + total_width - BAR_WIDTH + 1.0,
                    y + viewport.y0,
                ),
            );
            cx.fill(&rect, color, 0.0);
        }
    }

    /// Paint a highlight around the characters at the given positions.
    fn paint_char_highlights(
        &self,
        cx: &mut PaintCx,
        screen_lines: &ScreenLines,
        highlight_line_cols: impl Iterator<Item = (RVLine, usize)>,
    ) {
        let editor = &self.editor.editor;
        let config = self.editor.common.config.get_untracked();
        let line_height = config.editor.line_height() as f64;

        for (rvline, col) in highlight_line_cols {
            // Is the given line on screen?
            if let Some(line_info) = screen_lines.info(rvline) {
                let x0 = editor
                    .line_point_of_line_col(
                        rvline.line,
                        col,
                        CursorAffinity::Forward,
                        true,
                    )
                    .x;
                let x1 = editor
                    .line_point_of_line_col(
                        rvline.line,
                        col + 1,
                        CursorAffinity::Backward,
                        true,
                    )
                    .x;

                let y0 = line_info.vline_y;
                let y1 = y0 + line_height;

                let rect = Rect::new(x0, y0, x1, y1);

                cx.stroke(
                    &rect,
                    config.color(LapceColor::EDITOR_FOREGROUND),
                    &Stroke::new(1.0),
                );
            }
        }
    }

    /// Paint scope lines between `(start_rvline, start_line, start_col)` and
    /// `(end_rvline, end_line end_col)`.
    fn paint_scope_lines(
        &self,
        cx: &mut PaintCx,
        viewport: Rect,
        screen_lines: &ScreenLines,
        (start, start_col): (RVLine, usize),
        (end, end_col): (RVLine, usize),
    ) {
        let editor = &self.editor.editor;
        let doc = self.editor.doc();
        let config = self.editor.common.config.get_untracked();
        let line_height = config.editor.line_height() as f64;
        let brush = config.color(LapceColor::EDITOR_FOREGROUND);

        if start == end {
            if let Some(line_info) = screen_lines.info(start) {
                // TODO: Due to line wrapping the y positions of these two spots could be different, do we need to change it?
                let x0 = editor
                    .line_point_of_line_col(
                        start.line,
                        start_col + 1,
                        CursorAffinity::Forward,
                        true,
                    )
                    .x;
                let x1 = editor
                    .line_point_of_line_col(
                        end.line,
                        end_col,
                        CursorAffinity::Backward,
                        true,
                    )
                    .x;

                if x0 < x1 {
                    let y = line_info.vline_y + line_height;

                    let p0 = Point::new(x0, y);
                    let p1 = Point::new(x1, y);
                    let line = Line::new(p0, p1);

                    cx.stroke(&line, brush, &Stroke::new(1.0));
                }
            }
        } else {
            // Are start_line and end_line on screen?
            let start_line_y = screen_lines
                .info(start)
                .map(|line_info| line_info.vline_y + line_height);
            let end_line_y = screen_lines
                .info(end)
                .map(|line_info| line_info.vline_y + line_height);

            // We only need to draw anything if start_line is on or before the visible section and
            // end_line is on or after the visible section.
            let y0 = start_line_y.or_else(|| {
                screen_lines
                    .lines
                    .first()
                    .is_some_and(|&first_vline| first_vline > start)
                    .then(|| viewport.min_y())
            });
            let y1 = end_line_y.or_else(|| {
                screen_lines
                    .lines
                    .last()
                    .is_some_and(|&last_line| last_line < end)
                    .then(|| viewport.max_y())
            });

            if let [Some(y0), Some(y1)] = [y0, y1] {
                let start_x = editor
                    .line_point_of_line_col(
                        start.line,
                        start_col + 1,
                        CursorAffinity::Forward,
                        true,
                    )
                    .x;
                let end_x = editor
                    .line_point_of_line_col(
                        end.line,
                        end_col,
                        CursorAffinity::Backward,
                        true,
                    )
                    .x;

                // TODO(minor): is this correct with line wrapping?
                // The vertical line should be drawn to the left of any non-whitespace characters
                // in the enclosed section.
                let min_text_x = doc.buffer.with_untracked(|buffer| {
                    ((start.line + 1)..=end.line)
                        .filter(|&line| !buffer.is_line_whitespace(line))
                        .map(|line| {
                            let non_blank_offset =
                                buffer.first_non_blank_character_on_line(line);
                            let (_, col) =
                                editor.offset_to_line_col(non_blank_offset);

                            editor
                                .line_point_of_line_col(
                                    line,
                                    col,
                                    CursorAffinity::Backward,
                                    true,
                                )
                                .x
                        })
                        .min_by(f64::total_cmp)
                });

                let min_x = min_text_x.map_or(start_x, |min_text_x| {
                    cmp::min_by(min_text_x, start_x, f64::total_cmp)
                });

                // Is start_line on screen, and is the vertical line to the left of the opening
                // bracket?
                if let Some(y) = start_line_y.filter(|_| start_x > min_x) {
                    let p0 = Point::new(min_x, y);
                    let p1 = Point::new(start_x, y);
                    let line = Line::new(p0, p1);

                    cx.stroke(&line, brush, &Stroke::new(1.0));
                }

                // Is end_line on screen, and is the vertical line to the left of the closing
                // bracket?
                if let Some(y) = end_line_y.filter(|_| end_x > min_x) {
                    let p0 = Point::new(min_x, y);
                    let p1 = Point::new(end_x, y);
                    let line = Line::new(p0, p1);

                    cx.stroke(&line, brush, &Stroke::new(1.0));
                }

                let p0 = Point::new(min_x, y0);
                let p1 = Point::new(min_x, y1);
                let line = Line::new(p0, p1);

                cx.stroke(&line, brush, &Stroke::new(1.0));
            }
        }
    }

    /// Paint enclosing bracket highlights and scope lines if the corresponding settings are
    /// enabled.
    fn paint_bracket_highlights_scope_lines(
        &self,
        cx: &mut PaintCx,
        viewport: Rect,
        screen_lines: &ScreenLines,
    ) {
        let config = self.editor.common.config.get_untracked();

        if config.editor.highlight_matching_brackets
            || config.editor.highlight_scope_lines
        {
            let e_data = &self.editor;
            let ed = &e_data.editor;
            let offset = ed.cursor.with_untracked(|cursor| cursor.mode.offset());

            let bracket_offsets = e_data
                .doc_signal()
                .with_untracked(|doc| doc.find_enclosing_brackets(offset))
                .map(|(start, end)| [start, end]);

            let bracket_line_cols = bracket_offsets.map(|bracket_offsets| {
                bracket_offsets.map(|offset| {
                    let (rvline, col) =
                        ed.rvline_col_of_offset(offset, CursorAffinity::Forward);
                    (rvline, col)
                })
            });

            if config.editor.highlight_matching_brackets {
                self.paint_char_highlights(
                    cx,
                    screen_lines,
                    bracket_line_cols.into_iter().flatten(),
                );
            }

            if config.editor.highlight_scope_lines {
                if let Some([start_line_col, end_line_col]) = bracket_line_cols {
                    self.paint_scope_lines(
                        cx,
                        viewport,
                        screen_lines,
                        start_line_col,
                        end_line_col,
                    );
                }
            }
        }
    }
}

impl View for EditorView {
    fn id(&self) -> ViewId {
        self.id
    }

    fn style_pass(&mut self, cx: &mut StyleCx<'_>) {
        let editor = &self.editor.editor;
        if editor.es.try_update(|s| s.read(cx)).unwrap() {
            editor.floem_style_id.update(|val| *val += 1);
            cx.app_state_mut().request_paint(self.id());
        }
    }

    fn debug_name(&self) -> std::borrow::Cow<'static, str> {
        "Editor View".into()
    }

    fn update(
        &mut self,
        _cx: &mut floem::context::UpdateCx,
        state: Box<dyn std::any::Any>,
    ) {
        if let Ok(state) = state.downcast() {
            self.editor.sticky_header_info.set(*state);
            self.id.request_layout();
        }
    }

    fn layout(
        &mut self,
        cx: &mut floem::context::LayoutCx,
    ) -> floem::taffy::prelude::NodeId {
        cx.layout_node(self.id, true, |_cx| {
            if self.inner_node.is_none() {
                self.inner_node = Some(self.id.new_taffy_node());
            }

            let e_data = &self.editor;
            let editor = &e_data.editor;

            let viewport_size = self.viewport.get_untracked().size();

            let screen_lines = e_data.screen_lines().get_untracked();
            for (line, _) in screen_lines.iter_lines_y() {
                // fill in text layout cache so that max width is correct.
                editor.text_layout(line);
            }

            let inner_node = self.inner_node.unwrap();

            let config = self.editor.common.config.get_untracked();
            let line_height = config.editor.line_height() as f64;

            let is_local = e_data.doc().content.with_untracked(|c| c.is_local());

            let width = editor.max_line_width() + 10.0;
            let width = if !is_local {
                width.max(viewport_size.width)
            } else {
                width
            };
            let last_vline = editor.last_vline().get();
            let last_vline = e_data.visual_line(last_vline);
            let last_line_height = line_height * (last_vline + 1) as f64;
            let height = last_line_height.max(line_height);
            let height = if !is_local {
                height.max(viewport_size.height)
            } else {
                height
            };

            let margin_bottom = if !is_local
                && editor
                    .es
                    .with_untracked(EditorStyle::scroll_beyond_last_line)
            {
                viewport_size.height.min(last_line_height) - line_height
            } else {
                0.0
            };

            let style = Style::new()
                .width(width)
                .height(height)
                .margin_bottom(margin_bottom)
                .to_taffy_style();
            self.id.set_taffy_style(inner_node, style);

            vec![inner_node]
        })
    }

    fn compute_layout(
        &mut self,
        cx: &mut floem::context::ComputeLayoutCx,
    ) -> Option<Rect> {
        let viewport = cx.current_viewport();
        if self.viewport.with_untracked(|v| v != &viewport) {
            self.viewport.set(viewport);
        }
        None
    }

    fn paint(&mut self, cx: &mut PaintCx) {
        let viewport = self.viewport.get_untracked();
        let e_data = &self.editor;
        let ed = &e_data.editor;
        let config = e_data.common.config.get_untracked();
        let doc = e_data.doc();
        let is_local = doc.content.with_untracked(|content| content.is_local());
        let find_focus = self.editor.find_focus;
        let is_active =
            self.is_active.get_untracked() && !find_focus.get_untracked();

        // We repeatedly get the screen lines because we don't currently carefully manage the
        // paint functions to avoid potentially needing to recompute them, which could *maybe*
        // make them invalid.
        // TODO: One way to get around the above issue would be to more careful, since we
        // technically don't need to stop it from *recomputing* just stop any possible changes, but
        // avoiding recomputation seems easiest/clearest.
        // I expect that most/all of the paint functions could restrict themselves to only what is
        // within the active screen lines without issue.
        let screen_lines = ed.screen_lines.get_untracked();
        self.paint_current_line(cx, is_local, &screen_lines);
        FloemEditorView::paint_selection(cx, ed, &screen_lines);
        let screen_lines = ed.screen_lines.get_untracked();
        self.paint_diff_sections(cx, viewport, &screen_lines, &config);
        let screen_lines = ed.screen_lines.get_untracked();
        self.paint_find(cx, &screen_lines);
        let screen_lines = ed.screen_lines.get_untracked();
        self.paint_bracket_highlights_scope_lines(cx, viewport, &screen_lines);
        let screen_lines = ed.screen_lines.get_untracked();
        FloemEditorView::paint_text(
            cx,
            None,
            ed,
            viewport,
            is_active,
            &screen_lines,
        );
        let screen_lines = ed.screen_lines.get_untracked();
        self.paint_sticky_headers(cx, viewport, &screen_lines);
        self.paint_scroll_bar(cx, viewport, is_local, config);
    }
}

pub fn editor_container_view(
    window_tab_data: Rc<WindowTabData>,
    workspace: Arc<LapceWorkspace>,
    is_active: impl Fn(bool) -> bool + 'static + Copy,
    editor: RwSignal<EditorData>,
) -> impl View {
    let (editor_id, find_focus, sticky_header_height, editor_view, config, doc, ed) =
        editor.with_untracked(|editor| {
            (
                editor.id(),
                editor.find_focus,
                editor.sticky_header_height,
                editor.kind,
                editor.common.config,
                editor.doc_signal(),
                editor.editor.clone(),
            )
        });

    let main_split = window_tab_data.main_split.clone();
    let editors = main_split.editors;
    let scratch_docs = main_split.scratch_docs;
    let find_editor = main_split.find_editor;
    let replace_editor = main_split.replace_editor;
    let replace_active = main_split.common.find.replace_active;
    let replace_focus = main_split.common.find.replace_focus;
    let debug_breakline = window_tab_data.terminal.breakline;

    let viewport = ed.viewport;
    let screen_lines = ed.screen_lines;

    stack((
        editor_breadcrumbs(workspace, editor.get_untracked(), config),
        stack((
            editor_gutter(window_tab_data.clone(), editor),
            editor_gutter_folding_range(
                window_tab_data.clone(),
                doc,
                screen_lines,
                viewport,
            ),
            editor_content(editor, debug_breakline, is_active),
            empty().style(move |s| {
                let config = config.get();
                s.absolute()
                    .width_pct(100.0)
                    .height(sticky_header_height.get() as f32)
                    // .box_shadow_blur(5.0)
                    // .border_bottom(1.0)
                    // .border_color(
                    //     config.get_color(LapceColor::LAPCE_BORDER),
                    // )
                    .apply_if(
                        !config.editor.sticky_header
                            || sticky_header_height.get() == 0.0
                            || !editor_view.get().is_normal(),
                        |s| s.hide(),
                    )
            }),
            find_view(
                editor,
                find_editor,
                find_focus,
                replace_editor,
                replace_active,
                replace_focus,
                is_active,
            )
            .debug_name("find view"),
        ))
        .style(|s| s.width_full().flex_basis(0).flex_grow(1.0f32)),
    ))
    .on_cleanup(move || {
        let editor = editor.get_untracked();
        editor.cancel_completion();
        editor.cancel_inline_completion();
        if editors.contains_untracked(editor_id) {
            // editor still exist, so it might be moved to a different editor tab
            return;
        }
        let doc = editor.doc();
        editor.scope.dispose();

        let scratch_doc_name =
            if let DocContent::Scratch { name, .. } = doc.content.get_untracked() {
                Some(name.to_string())
            } else {
                None
            };
        if let Some(name) = scratch_doc_name {
            if !scratch_docs
                .with_untracked(|scratch_docs| scratch_docs.contains_key(&name))
            {
                doc.scope.dispose();
            }
        }
    })
    .style(|s| s.flex_col().absolute().size_pct(100.0, 100.0))
    .debug_name("Editor Container")
}

fn editor_content(
    e_data: RwSignal<EditorData>,
    debug_breakline: Memo<Option<(usize, PathBuf)>>,
    is_active: impl Fn(bool) -> bool + 'static + Copy,
) -> impl View {
    let (
        cursor,
        scroll_delta,
        scroll_to,
        window_origin,
        viewport,
        sticky_header_height,
        config,
        editor,
    ) = e_data.with_untracked(|editor| {
        (
            editor.cursor().read_only(),
            editor.scroll_delta().read_only(),
            editor.scroll_to(),
            editor.window_origin(),
            editor.viewport(),
            editor.sticky_header_height,
            editor.common.config,
            editor.editor.clone(),
        )
    });

    {
        create_effect(move |_| {
            is_active(true);
            let e_data = e_data.get_untracked();
            e_data.cancel_completion();
            e_data.cancel_inline_completion();
        });
    }

    let current_scroll = create_rw_signal(Rect::ZERO);

    scroll({
        let editor_content_view =
            editor_view(e_data.get_untracked(), debug_breakline, is_active).style(
                move |s| {
                    s.absolute()
                        .margin_left(1.0)
                        .min_size_full()
                        .cursor(CursorStyle::Text)
                },
            );

        let id = editor_content_view.id();
        editor.editor_view_id.set(Some(id));

        let editor2 = editor.clone();
        editor_content_view
            .on_event_cont(EventListener::FocusGained, move |_| {
                editor.editor_view_focused.notify();
            })
            .on_event_cont(EventListener::FocusLost, move |_| {
                editor2.editor_view_focus_lost.notify();
            })
            .on_event_cont(EventListener::PointerDown, move |event| {
                if let Event::PointerDown(pointer_event) = event {
                    id.request_active();
                    e_data.get_untracked().pointer_down(pointer_event);
                }
            })
            .on_event(EventListener::PointerWheel, move |event| {
                if let Event::PointerWheel(pointer_event) = event {
                    if zoom_editor_font_on_wheel(
                        &e_data.get_untracked(),
                        pointer_event,
                    ) {
                        return EventPropagation::Stop;
                    }
                }
                EventPropagation::Continue
            })
            .on_event_stop(EventListener::PointerMove, move |event| {
                if let Event::PointerMove(pointer_event) = event {
                    e_data.get_untracked().pointer_move(pointer_event);
                }
            })
            .on_event_stop(EventListener::PointerUp, move |event| {
                if let Event::PointerUp(pointer_event) = event {
                    e_data.get_untracked().pointer_up(pointer_event);
                }
            })
            .on_event_stop(EventListener::PointerLeave, move |event| {
                if let Event::PointerLeave = event {
                    e_data.get_untracked().pointer_leave();
                }
            })
    })
    .on_move(move |point| {
        window_origin.set(point);
    })
    .on_scroll(move |rect| {
        if rect.y0 != current_scroll.get_untracked().y0 {
            // only cancel completion if scrolled vertically
            let e_data = e_data.get_untracked();
            e_data.cancel_completion();
            e_data.cancel_inline_completion();
        }
        current_scroll.set(rect);
    })
    .scroll_to(move || scroll_to.get().map(|s| s.to_point()))
    .scroll_delta(move || scroll_delta.get())
    .ensure_visible(move || {
        let e_data = e_data.get_untracked();
        let cursor = cursor.get();
        let offset = cursor.offset();
        e_data.doc_signal().track();
        e_data.kind.track();

        let LineRegion { x, width, rvline } = cursor_caret(
            &e_data.editor,
            offset,
            !cursor.is_insert(),
            cursor.affinity,
        );
        let config = config.get_untracked();
        let line_height = config.editor.line_height();
        // TODO: is there a good way to avoid the calculation of the vline here?
        let vline = e_data.editor.vline_of_rvline(rvline);
        let vline = e_data.visual_line(vline.get());
        let rect = Rect::from_origin_size(
            (x, (vline * line_height) as f64),
            (width, line_height as f64),
        )
        .inflate(10.0, 0.0);

        let viewport = viewport.get_untracked();
        let smallest_distance = (viewport.y0 - rect.y0)
            .abs()
            .min((viewport.y1 - rect.y0).abs())
            .min((viewport.y0 - rect.y1).abs())
            .min((viewport.y1 - rect.y1).abs());
        let biggest_distance = (viewport.y0 - rect.y0)
            .abs()
            .max((viewport.y1 - rect.y0).abs())
            .max((viewport.y0 - rect.y1).abs())
            .max((viewport.y1 - rect.y1).abs());
        let jump_to_middle = biggest_distance > viewport.height()
            && smallest_distance > viewport.height() / 2.0;

        if jump_to_middle {
            rect.inflate(0.0, viewport.height() / 2.0)
        } else {
            let cursor_surrounding_lines =
                e_data.editor.es.with(|s| s.cursor_surrounding_lines());
            let mut rect = rect;
            rect.y0 -= (cursor_surrounding_lines * line_height) as f64
                + sticky_header_height.get_untracked();
            rect.y1 += (cursor_surrounding_lines * line_height) as f64;
            rect
        }
    })
    .style(|s| s.size_full().set(PropagatePointerWheel, false))
    .debug_name("Editor Content")
}
