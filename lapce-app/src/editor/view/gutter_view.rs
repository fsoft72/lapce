//! Views of the editor gutter: breakpoints, code lens, folding and code action markers.

use std::{collections::BTreeMap, ops::DerefMut, path::PathBuf, rc::Rc};

use floem::{
    View,
    event::{Event, EventListener, EventPropagation},
    peniko::kurbo::Rect,
    reactive::{
        Memo, RwSignal, SignalGet, SignalUpdate, SignalWith, create_memo,
        create_rw_signal,
    },
    style::CursorStyle,
    views::{
        Decorators, clip, container, dyn_stack, editor::view::ScreenLines, empty,
        label, stack, svg,
    },
};
use lapce_core::buffer::rope_text::RopeText;
use lapce_rpc::{
    dap_types::{DapId, SourceBreakpoint},
    plugin::PluginId,
};
use lsp_types::CodeLens;

use super::zoom::zoom_editor_font_on_wheel;
use crate::editor::{DocSignal, EditorData, gutter::editor_gutter_view};
use crate::{
    config::{color::LapceColor, icon::LapceIcons},
    debug::{DapData, LapceBreakpoint},
    editor::gutter::FoldingDisplayItem,
    window_tab::{CommonData, WindowTabData},
};

fn editor_gutter_breakpoint_view(
    i: usize,
    doc: DocSignal,
    daps: RwSignal<im::HashMap<DapId, DapData>>,
    breakpoints: RwSignal<BTreeMap<PathBuf, BTreeMap<usize, LapceBreakpoint>>>,
    screen_lines: RwSignal<ScreenLines>,
    common: Rc<CommonData>,
    icon_padding: f32,
) -> impl View {
    let hovered = create_rw_signal(false);
    let config = common.config;
    container(
        svg(move || config.get().ui_svg(LapceIcons::DEBUG_BREAKPOINT)).style(
            move |s| {
                let config = config.get();
                let size = config.ui.icon_size() as f32 + 2.0;
                s.size(size, size)
                    .color(config.color(LapceColor::DEBUG_BREAKPOINT_HOVER))
                    .apply_if(!hovered.get(), |s| s.hide())
            },
        ),
    )
    .on_click_stop(move |_| {
        let screen_lines = screen_lines.get_untracked();
        let line = screen_lines.lines.get(i).map(|r| r.line).unwrap_or(0);
        // let line = (viewport.get_untracked().y0
        //     / config.get_untracked().editor.line_height() as f64)
        //     .floor() as usize
        //     + i;
        let doc = doc.get_untracked();
        let offset = doc.buffer.with_untracked(|b| b.offset_of_line(line));
        if let Some(path) = doc.content.get_untracked().path() {
            let path_breakpoints = breakpoints
                .try_update(|breakpoints| {
                    let breakpoints = breakpoints.entry(path.clone()).or_default();
                    if let std::collections::btree_map::Entry::Vacant(e) =
                        breakpoints.entry(line)
                    {
                        e.insert(LapceBreakpoint {
                            id: None,
                            verified: false,
                            message: None,
                            line,
                            offset,
                            dap_line: None,
                            active: true,
                        });
                    } else {
                        let mut toggle_active = false;
                        if let Some(breakpoint) = breakpoints.get_mut(&line) {
                            if !breakpoint.active {
                                breakpoint.active = true;
                                toggle_active = true;
                            }
                        }
                        if !toggle_active {
                            breakpoints.remove(&line);
                        }
                    }
                    breakpoints.clone()
                })
                .unwrap();
            let source_breakpoints: Vec<SourceBreakpoint> = path_breakpoints
                .iter()
                .filter_map(|(_, b)| {
                    if b.active {
                        Some(SourceBreakpoint {
                            line: b.line + 1,
                            column: None,
                            condition: None,
                            hit_condition: None,
                            log_message: None,
                        })
                    } else {
                        None
                    }
                })
                .collect();
            let daps: Vec<DapId> =
                daps.with_untracked(|daps| daps.keys().cloned().collect());
            for dap_id in daps {
                common.proxy.dap_set_breakpoints(
                    dap_id,
                    path.to_path_buf(),
                    source_breakpoints.clone(),
                );
            }
        }
    })
    .on_event_stop(EventListener::PointerEnter, move |_| {
        hovered.set(true);
    })
    .on_event_stop(EventListener::PointerLeave, move |_| {
        hovered.set(false);
    })
    .style(move |s| {
        let config = config.get();
        s.width(config.ui.icon_size() as f32 + icon_padding * 2.0)
            .height(config.editor.line_height() as f32)
            .justify_center()
            .items_center()
            .cursor(CursorStyle::Pointer)
    })
}

fn editor_gutter_breakpoints(
    window_tab_data: Rc<WindowTabData>,
    e_data: RwSignal<EditorData>,
    icon_padding: f32,
) -> impl View {
    let breakpoints = window_tab_data.terminal.debug.breakpoints;
    let daps = window_tab_data.terminal.debug.daps;
    let common = window_tab_data.common.clone();

    let (ed, doc, config) = e_data
        .with_untracked(|e| (e.editor.clone(), e.doc_signal(), e.common.config));
    let viewport = ed.viewport;
    let screen_lines = ed.screen_lines;

    let num_display_lines = create_memo(move |_| {
        let screen_lines = screen_lines.get();
        screen_lines.lines.len()
        // let viewport = viewport.get();
        // let line_height = config.get().editor.line_height() as f64;
        // (viewport.height() / line_height).ceil() as usize + 1
    });

    clip(
        stack((
            dyn_stack(
                move || {
                    let num = num_display_lines.get();
                    0..num
                },
                move |i| *i,
                move |i| {
                    editor_gutter_breakpoint_view(
                        i,
                        doc,
                        daps,
                        breakpoints,
                        screen_lines,
                        common.clone(),
                        icon_padding,
                    )
                },
            )
            .style(move |s| {
                s.absolute().flex_col().margin_top(
                    -(viewport.get().y0 % config.get().editor.line_height() as f64)
                        as f32,
                )
            })
            .debug_name("Breakpoint Stack"),
            dyn_stack(
                move || {
                    let e_data = e_data.get();
                    let doc = e_data.doc_signal().get();
                    let content = doc.content.get();
                    let breakpoints = if let Some(path) = content.path() {
                        breakpoints
                            .with(|b| b.get(path).cloned())
                            .unwrap_or_default()
                    } else {
                        Default::default()
                    };
                    breakpoints.into_iter()
                },
                move |(line, b)| (*line, b.active),
                move |(line, breakpoint)| {
                    let active = breakpoint.active;
                    container(
                        svg(move || {
                            config.get().ui_svg(LapceIcons::DEBUG_BREAKPOINT)
                        })
                        .style(move |s| {
                            let config = config.get();
                            let size = config.ui.icon_size() as f32 + 2.0;
                            let color = if active {
                                LapceColor::DEBUG_BREAKPOINT
                            } else {
                                LapceColor::EDITOR_DIM
                            };
                            let color = config.color(color);
                            s.size(size, size).color(color)
                        }),
                    )
                    .style(move |s| {
                        let config = config.get();
                        let line_y = screen_lines
                            .with(|s| s.info_for_line(line))
                            .map(|l| l.y)
                            .unwrap_or_default();
                        s.absolute()
                            .width(config.ui.icon_size() as f32 + icon_padding * 2.0)
                            .height(config.editor.line_height() as f32)
                            .justify_center()
                            .items_center()
                            .margin_top(line_y as f32 - viewport.get().y0 as f32)
                    })
                },
            )
            .style(|s| s.absolute().size_pct(100.0, 100.0)),
        ))
        .style(|s| s.size_pct(100.0, 100.0)),
    )
    .style(move |s| {
        s.absolute()
            .size_pct(100.0, 100.0)
            .background(config.get().color(LapceColor::EDITOR_BACKGROUND))
    })
}

fn editor_gutter_code_lens_view(
    window_tab_data: Rc<WindowTabData>,
    line: usize,
    lens: (PluginId, usize, im::Vector<CodeLens>),
    screen_lines: RwSignal<ScreenLines>,
    viewport: RwSignal<Rect>,
    icon_padding: f32,
) -> impl View {
    let config = window_tab_data.common.config;
    let view = container(svg(move || config.get().ui_svg(LapceIcons::START)).style(
        move |s| {
            let config = config.get();
            let size = config.ui.icon_size() as f32;
            s.size(size, size)
                .color(config.color(LapceColor::LAPCE_ICON_ACTIVE))
        },
    ))
    .style(move |s| {
        let config = config.get();
        s.padding(4.0)
            .border_radius(6.0)
            .hover(|s| {
                s.cursor(CursorStyle::Pointer)
                    .background(config.color(LapceColor::PANEL_HOVERED_BACKGROUND))
            })
            .active(|s| {
                s.background(
                    config.color(LapceColor::PANEL_HOVERED_ACTIVE_BACKGROUND),
                )
            })
    })
    .on_click_stop({
        move |_| {
            let (plugin_id, offset, lens) = lens.clone();
            window_tab_data.show_code_lens(true, plugin_id, offset, lens);
        }
    });
    container(view).style(move |s| {
        let line_info = screen_lines.with(|s| s.info_for_line(line));
        let line_y = line_info.clone().map(|l| l.y).unwrap_or(-100.0);
        let rect = viewport.get();
        let config = config.get();
        let icon_size = config.ui.icon_size();
        let width = icon_size as f32 + icon_padding * 2.0;
        s.absolute()
            .width(width)
            .height(config.editor.line_height() as f32)
            .justify_center()
            .items_center()
            .margin_top(line_y as f32 - rect.y0 as f32)
    })
}

fn editor_gutter_folding_view(
    window_tab_data: Rc<WindowTabData>,
    screen_lines: RwSignal<ScreenLines>,
    viewport: RwSignal<Rect>,
    folding_display_item: FoldingDisplayItem,
) -> impl View {
    let config = window_tab_data.common.config;
    let view = container(
        svg(move || {
            let icon_str = match folding_display_item {
                FoldingDisplayItem::UnfoldStart(_) => LapceIcons::FOLD_DOWN,
                FoldingDisplayItem::Folded(_) => LapceIcons::FOLD,
                FoldingDisplayItem::UnfoldEnd(_) => LapceIcons::FOLD_UP,
            };
            config.get().ui_svg(icon_str)
        })
        .style(move |s| {
            let config = config.get();
            let size = config.ui.icon_size() as f32;
            s.size(size, size)
                .color(config.color(LapceColor::LAPCE_ICON_ACTIVE))
        }),
    )
    .style(move |s| {
        let config = config.get();
        s.padding(4.0)
            .border_radius(6.0)
            .hover(|s| {
                s.cursor(CursorStyle::Pointer)
                    .background(config.color(LapceColor::PANEL_HOVERED_BACKGROUND))
            })
            .active(|s| {
                s.background(
                    config.color(LapceColor::PANEL_HOVERED_ACTIVE_BACKGROUND),
                )
            })
    });
    container(view).style(move |s| {
        let line = folding_display_item.position().line;
        let line_info = screen_lines.with(|s| s.info_for_line(line as usize));
        let line_y = line_info.clone().map(|l| l.y).unwrap_or(-100.0);
        let rect = viewport.get();
        let config = config.get();
        let icon_size = config.ui.icon_size();
        let width = icon_size as f32 + 4.0;
        s.absolute()
            .width(width / 2.0)
            .height(config.editor.line_height() as f32)
            .justify_center()
            .items_center()
            .margin_top(line_y as f32 - rect.y0 as f32)
    })
}

fn editor_gutter_code_lens(
    window_tab_data: Rc<WindowTabData>,
    doc: DocSignal,
    screen_lines: RwSignal<ScreenLines>,
    viewport: RwSignal<Rect>,
    icon_padding: f32,
) -> impl View {
    let config = window_tab_data.common.config;

    dyn_stack(
        move || {
            let doc = doc.get();
            doc.code_lens.get()
        },
        move |(line, _)| (*line, doc.with_untracked(|doc| doc.rev())),
        move |(line, lens)| {
            editor_gutter_code_lens_view(
                window_tab_data.clone(),
                line,
                lens,
                screen_lines,
                viewport,
                icon_padding,
            )
        },
    )
    .style(move |s| {
        let config = config.get();
        let width = config.ui.icon_size() as f32 + icon_padding * 2.0;
        s.absolute()
            .width(width)
            .height_full()
            .margin_left(width - 8.0)
    })
    .debug_name("CodeLens Stack")
}

pub(super) fn editor_gutter_folding_range(
    window_tab_data: Rc<WindowTabData>,
    doc: DocSignal,
    screen_lines: RwSignal<ScreenLines>,
    viewport: RwSignal<Rect>,
) -> impl View {
    let config = window_tab_data.common.config;
    dyn_stack(
        move || doc.get().folding_ranges.get().to_display_items(),
        move |item| *item,
        move |item| {
            editor_gutter_folding_view(
                window_tab_data.clone(),
                screen_lines,
                viewport,
                item,
            )
            .on_click_stop({
                move |_| {
                    doc.get_untracked().folding_ranges.update(|x| match item {
                        FoldingDisplayItem::UnfoldStart(pos)
                        | FoldingDisplayItem::Folded(pos) => {
                            x.0.iter_mut().find_map(|mut range| {
                                let range = range.deref_mut();
                                if range.start == pos {
                                    range.status.click();
                                    Some(())
                                } else {
                                    None
                                }
                            });
                        }
                        FoldingDisplayItem::UnfoldEnd(pos) => {
                            x.0.iter_mut().find_map(|mut range| {
                                let range = range.deref_mut();
                                if range.end == pos {
                                    range.status.click();
                                    Some(())
                                } else {
                                    None
                                }
                            });
                        }
                    })
                }
            })
        },
    )
    .style(move |s| {
        let config = config.get();
        let width = config.ui.icon_size() as f32;
        // hide for now
        s.width(width)
            .height_full()
            .margin_left(-width / 2.0)
            .hide()
    })
    .debug_name("Folding Range Stack")
}

fn editor_gutter_code_actions(
    e_data: RwSignal<EditorData>,
    gutter_width: Memo<f64>,
    icon_padding: f32,
) -> impl View {
    let (ed, doc, config) = e_data
        .with_untracked(|e| (e.editor.clone(), e.doc_signal(), e.common.config));
    let viewport = ed.viewport;
    let cursor = ed.cursor;

    let code_action_vline = create_memo(move |_| {
        let doc = doc.get();
        let (offset, affinity) =
            cursor.with(|cursor| (cursor.offset(), cursor.affinity));
        let has_code_actions = doc
            .code_actions()
            .with(|c| c.get(&offset).map(|c| !c.1.is_empty()).unwrap_or(false));
        if has_code_actions {
            let vline = ed.vline_of_offset(offset, affinity);
            Some(vline)
        } else {
            None
        }
    });

    container(
        container(
            svg(move || config.get().ui_svg(LapceIcons::LIGHTBULB)).style(
                move |s| {
                    let config = config.get();
                    let size = config.ui.icon_size() as f32;
                    s.size(size, size)
                        .color(config.color(LapceColor::LAPCE_WARN))
                },
            ),
        )
        .on_click_stop(move |_| {
            e_data.get_untracked().show_code_actions(true);
        })
        .style(move |s| {
            let config = config.get();
            s.padding(4.0)
                .border_radius(6.0)
                .hover(|s| {
                    s.cursor(CursorStyle::Pointer).background(
                        config.color(LapceColor::PANEL_HOVERED_BACKGROUND),
                    )
                })
                .active(|s| {
                    s.background(
                        config.color(LapceColor::PANEL_HOVERED_ACTIVE_BACKGROUND),
                    )
                })
        }),
    )
    .style(move |s| {
        let config = config.get();
        let viewport = viewport.get();
        let gutter_width = gutter_width.get();
        let code_action_vline = code_action_vline.get();
        let size = config.ui.icon_size() as f32;
        let line_height = config.editor.line_height();
        let margin_top = if let Some(vline) = code_action_vline {
            (vline.get() * line_height) as f32 - viewport.y0 as f32
        } else {
            0.0
        };
        let width = size + icon_padding * 2.0;
        s.absolute()
            .items_center()
            .justify_center()
            .margin_left(gutter_width as f32 - width + 1.0)
            .margin_top(margin_top)
            .width(width)
            .height(line_height as f32)
            .apply_if(code_action_vline.is_none(), |s| s.hide())
    })
    .debug_name("Code Action LightBulb")
}

pub(super) fn editor_gutter(
    window_tab_data: Rc<WindowTabData>,
    e_data: RwSignal<EditorData>,
) -> impl View {
    let icon_padding = 6.0;

    let (ed, doc, config) = e_data
        .with_untracked(|e| (e.editor.clone(), e.doc_signal(), e.common.config));
    let viewport = ed.viewport;
    let scroll_delta = ed.scroll_delta;
    let screen_lines = ed.screen_lines;

    let gutter_rect = create_rw_signal(Rect::ZERO);
    let gutter_width = create_memo(move |_| gutter_rect.get().width());

    let icon_total_width = move || {
        let icon_size = config.get().ui.icon_size() as f32;
        icon_size + icon_padding * 2.0
    };

    let gutter_padding_right = create_memo(move |_| icon_total_width() + 6.0);

    stack((
        stack((
            empty().style(move |s| s.width(icon_total_width() * 2.0 - 8.0)),
            label(move || {
                let doc = doc.get();
                doc.buffer.with(|b| b.last_line() + 1).to_string()
            }),
            empty().style(move |s| s.width(gutter_padding_right.get())),
        ))
        .debug_name("Centered Last Line Count")
        .style(|s| s.height_pct(100.0)),
        editor_gutter_breakpoints(window_tab_data.clone(), e_data, icon_padding),
        clip(
            stack((
                editor_gutter_code_lens(
                    window_tab_data.clone(),
                    doc,
                    screen_lines,
                    viewport,
                    icon_padding,
                ),
                editor_gutter_view(e_data.get_untracked(), gutter_padding_right)
                    .on_resize(move |rect| {
                        gutter_rect.set(rect);
                    })
                    .on_event(EventListener::PointerWheel, move |event| {
                        if let Event::PointerWheel(pointer_event) = event {
                            if !zoom_editor_font_on_wheel(
                                &e_data.get_untracked(),
                                pointer_event,
                            ) {
                                scroll_delta.set(pointer_event.delta);
                            }
                        }
                        EventPropagation::Stop
                    })
                    .style(|s| s.size_pct(100.0, 100.0)),
                editor_gutter_code_actions(e_data, gutter_width, icon_padding),
            ))
            .style(|s| s.size_pct(100.0, 100.0)),
        )
        .style(move |s| s.absolute().size_pct(100.0, 100.0)),
    ))
    .style(|s| s.height_pct(100.0))
    .debug_name("Editor Gutter")
}
