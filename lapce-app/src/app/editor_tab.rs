//! Views of an editor tab: header with the tab strip, content and drag and drop targets.

use std::rc::Rc;

use floem::{
    IntoView, View,
    action::show_context_menu,
    event::{Event, EventListener, EventPropagation},
    menu::{Menu, MenuItem},
    peniko::kurbo::{Rect, Size},
    reactive::{
        ReadSignal, RwSignal, SignalGet, SignalUpdate, SignalWith, create_effect,
        create_memo, create_rw_signal,
    },
    style::{AlignItems, CursorStyle, Style},
    taffy::{
        Line,
        style_helpers::{self, auto, fr},
    },
    text::Style as FontStyle,
    unit::PxPctAuto,
    views::{
        Decorators, clip, container, dyn_stack, empty, label,
        scroll::{VerticalScrollAsHorizontal, scroll},
        stack, svg, tab, text, tooltip,
    },
};

use super::widgets::{clickable_icon, tooltip_tip};
use crate::{
    command::{InternalCommand, LapceWorkbenchCommand},
    config::{color::LapceColor, icon::LapceIcons, ui::TabSeparatorHeight},
    editor::{diff::diff_show_more_section_view, view::editor_container_view},
    editor_tab::{EditorTabChild, EditorTabData},
    id::EditorTabId,
    keymap::keymap_view,
    listener::Listener,
    main_split::{SplitDirection, SplitMoveDirection, TabCloseKind},
    plugin::{PluginData, plugin_info_view},
    settings::{settings_view, theme_color_settings_view},
    window_tab::{Focus, WindowTabData},
};

/// The top bar of an Editor tab. Includes the tab forward/back buttons, the tab scroll bar and the new split and tab close all button.
fn editor_tab_header(
    window_tab_data: Rc<WindowTabData>,
    active_editor_tab: ReadSignal<Option<EditorTabId>>,
    editor_tab: RwSignal<EditorTabData>,
    dragging: RwSignal<Option<(RwSignal<usize>, EditorTabId)>>,
) -> impl View {
    let main_split = window_tab_data.main_split.clone();
    let plugin = window_tab_data.plugin.clone();
    let editors = window_tab_data.main_split.editors;
    let diff_editors = window_tab_data.main_split.diff_editors;
    let focus = window_tab_data.common.focus;
    let config = window_tab_data.common.config;
    let internal_command = window_tab_data.common.internal_command;
    let workbench_command = window_tab_data.common.workbench_command;
    let editor_tab_id =
        editor_tab.with_untracked(|editor_tab| editor_tab.editor_tab_id);

    let editor_tab_active =
        create_memo(move |_| editor_tab.with(|editor_tab| editor_tab.active));
    let items = move || {
        let editor_tab = editor_tab.get();
        for (i, (index, _, _)) in editor_tab.children.iter().enumerate() {
            if index.get_untracked() != i {
                index.set(i);
            }
        }
        editor_tab.children
    };
    let key = |(_, _, child): &(RwSignal<usize>, RwSignal<Rect>, EditorTabChild)| {
        child.id()
    };
    let is_focused = move || {
        if let Focus::Workbench = focus.get() {
            editor_tab.with_untracked(|e| Some(e.editor_tab_id))
                == active_editor_tab.get()
        } else {
            false
        }
    };

    let view_fn = move |(i, layout_rect, child): (
        RwSignal<usize>,
        RwSignal<Rect>,
        EditorTabChild,
    )| {
        let local_child = child.clone();
        let child_for_close = child.clone();
        let child_for_mouse_close = child.clone();
        let child_for_mouse_close_2 = child.clone();
        let main_split = main_split.clone();
        let plugin = plugin.clone();
        let child_view = {
            let info = child.view_info(editors, diff_editors, plugin, config);
            let hovered = create_rw_signal(false);

            use crate::config::ui::TabCloseButton;

            let tab_icon = container({
                svg("")
                    .update_value(move || info.with(|info| info.icon.clone()))
                    .style(move |s| {
                        let config = config.get();
                        let size = config.ui.icon_size() as f32;
                        s.size(size, size)
                            .apply_opt(info.with(|info| info.color), |s, c| {
                                s.color(c)
                            })
                            .apply_if(
                                !info.with(|info| info.is_pristine)
                                    && config.ui.tab_close_button
                                        == TabCloseButton::Off,
                                |s| s.color(config.color(LapceColor::LAPCE_WARN)),
                            )
                    })
            })
            .style(|s| s.padding(4.));

            let tab_content = tooltip(
                label(move || info.with(|info| info.name.clone())).style(move |s| {
                    s.apply_if(
                        !info
                            .with(|info| info.confirmed)
                            .map(|confirmed| confirmed.get())
                            .unwrap_or(true),
                        |s| s.font_style(FontStyle::Italic),
                    )
                    .selectable(false)
                }),
                move || {
                    tooltip_tip(
                        config,
                        text(info.with(|info| {
                            info.path
                                .clone()
                                .map(|path| path.display().to_string())
                                .unwrap_or("local".to_string())
                        })),
                    )
                },
            );

            let tab_close_button = clickable_icon(
                move || {
                    if hovered.get() || info.with(|info| info.is_pristine) {
                        LapceIcons::CLOSE
                    } else {
                        LapceIcons::UNSAVED
                    }
                },
                move || {
                    let editor_tab_id =
                        editor_tab.with_untracked(|t| t.editor_tab_id);
                    internal_command.send(InternalCommand::EditorTabChildClose {
                        editor_tab_id,
                        child: child_for_close.clone(),
                    });
                },
                || false,
                || false,
                || "Close",
                config,
            )
            .on_event_stop(EventListener::PointerDown, |_| {})
            .on_event_stop(EventListener::PointerEnter, move |_| {
                hovered.set(true);
            })
            .on_event_stop(EventListener::PointerLeave, move |_| {
                hovered.set(false);
            });

            stack((
                tab_icon.style(move |s| {
                    let tab_close_button = config.get().ui.tab_close_button;
                    s.apply_if(tab_close_button == TabCloseButton::Left, |s| {
                        s.grid_column(Line {
                            start: style_helpers::line(3),
                            end: style_helpers::span(1),
                        })
                    })
                }),
                tab_content.style(move |s| {
                    let tab_close_button = config.get().ui.tab_close_button;
                    s.apply_if(tab_close_button == TabCloseButton::Left, |s| {
                        s.grid_column(Line {
                            start: style_helpers::line(2),
                            end: style_helpers::span(1),
                        })
                    })
                    .apply_if(tab_close_button == TabCloseButton::Off, |s| {
                        s.padding_right(4.)
                    })
                }),
                tab_close_button.style(move |s| {
                    let tab_close_button = config.get().ui.tab_close_button;
                    s.apply_if(tab_close_button == TabCloseButton::Left, |s| {
                        s.grid_column(Line {
                            start: style_helpers::line(1),
                            end: style_helpers::span(1),
                        })
                    })
                    .apply_if(tab_close_button == TabCloseButton::Off, |s| s.hide())
                }),
            ))
            .style(move |s| {
                s.items_center()
                    .justify_center()
                    .border_left(if i.get() == 0 { 1.0 } else { 0.0 })
                    .border_right(1.0)
                    .border_color(config.get().color(LapceColor::LAPCE_BORDER))
                    .padding_horiz(6.)
                    .gap(6.)
                    .grid()
                    .grid_template_columns(vec![auto(), fr(1_f32), auto()])
                    .apply_if(
                        config.get().ui.tab_separator_height
                            == TabSeparatorHeight::Full,
                        |s| s.height_full(),
                    )
            })
        };

        let confirmed = match local_child {
            EditorTabChild::Editor(editor_id) => {
                editors.editor_untracked(editor_id).map(|e| e.confirmed)
            }
            EditorTabChild::DiffEditor(diff_editor_id) => diff_editors
                .with_untracked(|diff_editors| {
                    diff_editors
                        .get(&diff_editor_id)
                        .map(|diff_editor_data| diff_editor_data.confirmed)
                }),
            _ => None,
        };

        let header_content_size = create_rw_signal(Size::ZERO);
        let drag_over_left: RwSignal<Option<bool>> = create_rw_signal(None);
        stack((
            child_view
                .on_double_click_stop(move |_| {
                    if let Some(confirmed) = confirmed {
                        confirmed.set(true);
                    }
                })
                .on_event(EventListener::PointerDown, move |event| {
                    if let Event::PointerDown(pointer_event) = event {
                        if pointer_event.button.is_auxiliary() {
                            let editor_tab_id =
                                editor_tab.with_untracked(|t| t.editor_tab_id);
                            internal_command.send(
                                InternalCommand::EditorTabChildClose {
                                    editor_tab_id,
                                    child: child_for_mouse_close.clone(),
                                },
                            );
                            EventPropagation::Stop
                        } else {
                            editor_tab.update(|editor_tab| {
                                editor_tab.active = i.get_untracked();
                            });
                            EventPropagation::Continue
                        }
                    } else {
                        EventPropagation::Continue
                    }
                })
                .on_secondary_click_stop(move |_| {
                    let editor_tab_id =
                        editor_tab.with_untracked(|t| t.editor_tab_id);

                    tab_secondary_click(
                        internal_command,
                        editor_tab_id,
                        child_for_mouse_close_2.clone(),
                    );
                })
                .on_event_stop(EventListener::DragStart, move |_| {
                    dragging.set(Some((i, editor_tab_id)));
                })
                .on_event_stop(EventListener::DragEnd, move |_| {
                    dragging.set(None);
                })
                .on_resize(move |rect| {
                    header_content_size.set(rect.size());
                })
                .draggable()
                .dragging_style(move |s| {
                    let config = config.get();
                    s.border(1.0)
                        .border_radius(6.0)
                        .background(
                            config
                                .color(LapceColor::PANEL_BACKGROUND)
                                .multiply_alpha(0.7),
                        )
                        .border_color(config.color(LapceColor::LAPCE_BORDER))
                })
                .style(|s| {
                    s.align_items(Some(AlignItems::Center)).flex_grow(1.0f32)
                }),
            empty()
                .style(move |s| {
                    s.size_full()
                        .border_bottom(if editor_tab_active.get() == i.get() {
                            2.0
                        } else {
                            0.0
                        })
                        .border_color(config.get().color(if is_focused() {
                            LapceColor::LAPCE_TAB_ACTIVE_UNDERLINE
                        } else {
                            LapceColor::LAPCE_TAB_INACTIVE_UNDERLINE
                        }))
                })
                .style(|s| {
                    s.absolute()
                        .padding_horiz(3.0)
                        .size_full()
                        .pointer_events_none()
                })
                .debug_name("Drop Indicator"),
            empty()
                .style(move |s| {
                    let i = i.get();
                    let drag_over_left = drag_over_left.get();
                    s.absolute()
                        .margin_left(if i == 0 { 0.0 } else { -2.0 })
                        .height_full()
                        .width(
                            header_content_size.get().width as f32
                                + if i == 0 { 1.0 } else { 3.0 },
                        )
                        .apply_if(drag_over_left.is_none(), |s| s.hide())
                        .apply_if(drag_over_left.is_some(), |s| {
                            if let Some(drag_over_left) = drag_over_left {
                                if drag_over_left {
                                    s.border_left(3.0)
                                } else {
                                    s.border_right(3.0)
                                }
                            } else {
                                s
                            }
                        })
                        .border_color(
                            config
                                .get()
                                .color(LapceColor::LAPCE_TAB_ACTIVE_UNDERLINE)
                                .multiply_alpha(0.5),
                        )
                })
                .debug_name("Active Tab Indicator"),
        ))
        .on_resize(move |rect| {
            layout_rect.set(rect);
        })
        .style(move |s| {
            let config = config.get();
            s.height_full()
                .flex_col()
                .items_center()
                .justify_center()
                .cursor(CursorStyle::Pointer)
                .hover(|s| s.background(config.color(LapceColor::HOVER_BACKGROUND)))
        })
        .debug_name("Tab and Active Indicator")
        .on_event_stop(EventListener::DragOver, move |event| {
            if dragging.with_untracked(|dragging| dragging.is_some()) {
                if let Event::PointerMove(pointer_event) = event {
                    let new_left = pointer_event.pos.x
                        < header_content_size.get_untracked().width / 2.0;
                    if drag_over_left.get_untracked() != Some(new_left) {
                        drag_over_left.set(Some(new_left));
                    }
                }
            }
        })
        .on_event(EventListener::Drop, move |event| {
            if let Some((from_index, from_editor_tab_id)) = dragging.get_untracked()
            {
                drag_over_left.set(None);
                if let Event::PointerUp(pointer_event) = event {
                    let left = pointer_event.pos.x
                        < header_content_size.get_untracked().width / 2.0;
                    let index = i.get_untracked();
                    let new_index = if left { index } else { index + 1 };
                    main_split.move_editor_tab_child(
                        from_editor_tab_id,
                        editor_tab_id,
                        from_index.get_untracked(),
                        new_index,
                    );
                }
                EventPropagation::Stop
            } else {
                EventPropagation::Continue
            }
        })
        .on_event_stop(EventListener::DragLeave, move |_| {
            drag_over_left.set(None);
        })
    };

    let content_size = create_rw_signal(Size::ZERO);
    let scroll_offset = create_rw_signal(Rect::ZERO);
    stack((
        stack({
            let size = create_rw_signal(Size::ZERO);
            (
                clip(empty().style(move |s| {
                    let config = config.get();
                    s.absolute()
                        .height_full()
                        .width(size.get().width as f32)
                        .background(config.color(LapceColor::PANEL_BACKGROUND))
                        .box_shadow_blur(3.0)
                        .box_shadow_color(
                            config.color(LapceColor::LAPCE_DROPDOWN_SHADOW),
                        )
                }))
                .style(move |s| {
                    let scroll_offset = scroll_offset.get();
                    s.absolute()
                        .width(size.get().width as f32 + 30.0)
                        .height_full()
                        .apply_if(scroll_offset.x0 == 0.0, |s| s.hide())
                }),
                stack((
                    clickable_icon(
                        || LapceIcons::TAB_PREVIOUS,
                        move || {
                            workbench_command
                                .send(LapceWorkbenchCommand::PreviousEditorTab);
                        },
                        || false,
                        || false,
                        || "Previous Tab",
                        config,
                    )
                    .style(|s| s.margin_horiz(6.0).margin_vert(7.0)),
                    clickable_icon(
                        || LapceIcons::TAB_NEXT,
                        move || {
                            workbench_command
                                .send(LapceWorkbenchCommand::NextEditorTab);
                        },
                        || false,
                        || false,
                        || "Next Tab",
                        config,
                    )
                    .style(|s| s.margin_right(6.0)),
                ))
                .on_resize(move |rect| {
                    size.set(rect.size());
                })
                .debug_name("Next/Previoius Tab Buttons")
                .style(move |s| s.items_center()),
            )
        })
        .style(|s| s.flex_shrink(0f32)),
        container(
            scroll({
                dyn_stack(items, key, view_fn)
                    .on_resize(move |rect| {
                        let size = rect.size();
                        if content_size.get_untracked() != size {
                            content_size.set(size);
                        }
                    })
                    .debug_name("Horizontal Tab Stack")
                    .style(|s| s.height_full().items_center())
            })
            .on_scroll(move |rect| {
                scroll_offset.set(rect);
            })
            .ensure_visible(move || {
                let active = editor_tab_active.get();
                editor_tab
                    .with_untracked(|editor_tab| editor_tab.children[active].1)
                    .get_untracked()
            })
            .scroll_style(|s| s.hide_bars(true))
            .style(|s| {
                s.set(VerticalScrollAsHorizontal, true)
                    .absolute()
                    .size_full()
            }),
        )
        .style(|s| {
            s.height_full()
                .flex_grow(1.0f32)
                .flex_basis(0.)
                .min_width(10.)
        })
        .debug_name("Tab scroll"),
        stack({
            let size = create_rw_signal(Size::ZERO);
            (
                clip({
                    empty().style(move |s| {
                        let config = config.get();
                        s.absolute()
                            .height_full()
                            .margin_left(30.0)
                            .width(size.get().width as f32)
                            .background(config.color(LapceColor::PANEL_BACKGROUND))
                            .box_shadow_blur(3.0)
                            .box_shadow_color(
                                config.color(LapceColor::LAPCE_DROPDOWN_SHADOW),
                            )
                    })
                })
                .style(move |s| {
                    let content_size = content_size.get();
                    let scroll_offset = scroll_offset.get();
                    s.absolute()
                        .margin_left(-30.0)
                        .width(size.get().width as f32 + 30.0)
                        .height_full()
                        .apply_if(scroll_offset.x1 >= content_size.width, |s| {
                            s.hide()
                        })
                }),
                stack((
                    clickable_icon(
                        || LapceIcons::SPLIT_HORIZONTAL,
                        move || {
                            let editor_tab_id =
                                editor_tab.with_untracked(|t| t.editor_tab_id);
                            internal_command.send(InternalCommand::Split {
                                direction: SplitDirection::Vertical,
                                editor_tab_id,
                            });
                        },
                        || false,
                        || false,
                        || "Split Horizontally",
                        config,
                    )
                    .style(|s| s.margin_left(6.0)),
                    clickable_icon(
                        || LapceIcons::CLOSE,
                        move || {
                            let editor_tab_id =
                                editor_tab.with_untracked(|t| t.editor_tab_id);
                            internal_command.send(InternalCommand::EditorTabClose {
                                editor_tab_id,
                            });
                        },
                        || false,
                        || false,
                        || "Close All",
                        config,
                    )
                    .style(|s| s.margin_horiz(6.0)),
                ))
                .on_resize(move |rect| {
                    size.set(rect.size());
                })
                .style(|s| s.items_center().height_full()),
            )
        })
        .debug_name("Split/Close Panel Buttons")
        .style(move |s| {
            let content_size = content_size.get();
            let scroll_offset = scroll_offset.get();
            s.height_full()
                .flex_shrink(0f32)
                .margin_left(PxPctAuto::Auto)
                .apply_if(scroll_offset.x1 < content_size.width, |s| {
                    s.margin_left(0.)
                })
        }),
    ))
    .style(move |s| {
        let config = config.get();
        s.items_center()
            .max_width_full()
            .border_bottom(1.0)
            .border_color(config.color(LapceColor::LAPCE_BORDER))
            .background(config.color(LapceColor::PANEL_BACKGROUND))
            .height(config.ui.header_height() as i32)
    })
    .debug_name("Editor Tab Header")
}

fn editor_tab_content(
    window_tab_data: Rc<WindowTabData>,
    plugin: PluginData,
    active_editor_tab: ReadSignal<Option<EditorTabId>>,
    editor_tab: RwSignal<EditorTabData>,
) -> impl View {
    let main_split = window_tab_data.main_split.clone();
    let common = main_split.common.clone();
    let workspace = common.workspace.clone();
    let editors = main_split.editors;
    let diff_editors = main_split.diff_editors;
    let config = common.config;
    let focus = common.focus;
    let items = move || {
        editor_tab
            .get()
            .children
            .into_iter()
            .map(|(_, _, child)| child)
    };
    let key = |child: &EditorTabChild| child.id();
    let view_fn = move |child| {
        let common = common.clone();
        let child = match child {
            EditorTabChild::Editor(editor_id) => {
                if let Some(editor_data) = editors.editor_untracked(editor_id) {
                    let editor_scope = editor_data.scope;
                    let editor_tab_id = editor_data.editor_tab_id;
                    let is_active = move |tracked: bool| {
                        editor_scope.track();
                        let focus = if tracked {
                            focus.get()
                        } else {
                            focus.get_untracked()
                        };
                        if let Focus::Workbench = focus {
                            let active_editor_tab = if tracked {
                                active_editor_tab.get()
                            } else {
                                active_editor_tab.get_untracked()
                            };
                            let editor_tab = if tracked {
                                editor_tab_id.get()
                            } else {
                                editor_tab_id.get_untracked()
                            };
                            editor_tab.is_some() && editor_tab == active_editor_tab
                        } else {
                            false
                        }
                    };
                    let editor_data = create_rw_signal(editor_data);
                    editor_container_view(
                        window_tab_data.clone(),
                        workspace.clone(),
                        is_active,
                        editor_data,
                    )
                    .into_any()
                } else {
                    text("empty editor").into_any()
                }
            }
            EditorTabChild::DiffEditor(diff_editor_id) => {
                let diff_editor_data = diff_editors.with_untracked(|diff_editors| {
                    diff_editors.get(&diff_editor_id).cloned()
                });
                if let Some(diff_editor_data) = diff_editor_data {
                    let focus_right = diff_editor_data.focus_right;
                    let diff_editor_tab_id = diff_editor_data.editor_tab_id;
                    let diff_editor_scope = diff_editor_data.scope;
                    let is_active = move |tracked: bool| {
                        let focus = if tracked {
                            focus.get()
                        } else {
                            focus.get_untracked()
                        };
                        if let Focus::Workbench = focus {
                            let active_editor_tab = if tracked {
                                active_editor_tab.get()
                            } else {
                                active_editor_tab.get_untracked()
                            };
                            let diff_editor_tab_id = if tracked {
                                diff_editor_tab_id.get()
                            } else {
                                diff_editor_tab_id.get_untracked()
                            };
                            Some(diff_editor_tab_id) == active_editor_tab
                        } else {
                            false
                        }
                    };
                    let left_viewport = diff_editor_data.left.viewport();
                    let left_scroll_to = diff_editor_data.left.scroll_to();
                    let right_viewport = diff_editor_data.right.viewport();
                    let right_scroll_to = diff_editor_data.right.scroll_to();
                    create_effect(move |_| {
                        let left_viewport = left_viewport.get();
                        if right_viewport.get_untracked() != left_viewport {
                            right_scroll_to
                                .set(Some(left_viewport.origin().to_vec2()));
                        }
                    });
                    create_effect(move |_| {
                        let right_viewport = right_viewport.get();
                        if left_viewport.get_untracked() != right_viewport {
                            left_scroll_to
                                .set(Some(right_viewport.origin().to_vec2()));
                        }
                    });
                    let left_editor =
                        create_rw_signal(diff_editor_data.left.clone());
                    let right_editor =
                        create_rw_signal(diff_editor_data.right.clone());
                    stack((
                        container(
                            editor_container_view(
                                window_tab_data.clone(),
                                workspace.clone(),
                                move |track| {
                                    is_active(track)
                                        && if track {
                                            !focus_right.get()
                                        } else {
                                            !focus_right.get_untracked()
                                        }
                                },
                                left_editor,
                            )
                            .debug_name("Left Editor"),
                        )
                        .on_event_cont(EventListener::PointerDown, move |_| {
                            focus_right.set(false);
                        })
                        .style(move |s| {
                            s.height_full()
                                .flex_grow(1.0f32)
                                .flex_basis(0.0)
                                .border_right(1.0)
                                .border_color(
                                    config.get().color(LapceColor::LAPCE_BORDER),
                                )
                        }),
                        container(
                            editor_container_view(
                                window_tab_data.clone(),
                                workspace.clone(),
                                move |track| {
                                    is_active(track)
                                        && if track {
                                            focus_right.get()
                                        } else {
                                            focus_right.get_untracked()
                                        }
                                },
                                right_editor,
                            )
                            .debug_name("Right Editor"),
                        )
                        .on_event_cont(EventListener::PointerDown, move |_| {
                            focus_right.set(true);
                        })
                        .style(|s| {
                            s.height_full().flex_grow(1.0f32).flex_basis(0.0)
                        }),
                        diff_show_more_section_view(
                            &diff_editor_data.left,
                            &diff_editor_data.right,
                        ),
                    ))
                    .style(|s: Style| s.size_full())
                    .on_cleanup(move || {
                        diff_editor_scope.dispose();
                    })
                    .into_any()
                } else {
                    text("empty diff editor").into_any()
                }
            }
            EditorTabChild::Settings(_) => {
                settings_view(plugin.installed, editors, common).into_any()
            }
            EditorTabChild::ThemeColorSettings(_) => {
                theme_color_settings_view(editors, common).into_any()
            }
            EditorTabChild::Keymap(_) => keymap_view(editors, common).into_any(),
            EditorTabChild::Volt(_, id) => {
                plugin_info_view(plugin.clone(), id).into_any()
            }
        };
        child.style(|s| s.size_full())
    };
    let active = move || editor_tab.with(|t| t.active);

    tab(active, items, key, view_fn)
        .style(|s| s.size_full())
        .debug_name("Editor Tab Content")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DragOverPosition {
    Top,
    Bottom,
    Left,
    Right,
    Middle,
}

pub(super) fn editor_tab(
    window_tab_data: Rc<WindowTabData>,
    plugin: PluginData,
    active_editor_tab: ReadSignal<Option<EditorTabId>>,
    editor_tab: RwSignal<EditorTabData>,
    dragging: RwSignal<Option<(RwSignal<usize>, EditorTabId)>>,
) -> impl View {
    let main_split = window_tab_data.main_split.clone();
    let common = main_split.common.clone();
    let editor_tabs = main_split.editor_tabs;
    let editor_tab_id =
        editor_tab.with_untracked(|editor_tab| editor_tab.editor_tab_id);
    let config = common.config;
    let focus = common.focus;
    let internal_command = main_split.common.internal_command;
    let tab_size = create_rw_signal(Size::ZERO);
    let drag_over: RwSignal<Option<DragOverPosition>> = create_rw_signal(None);
    stack((
        editor_tab_header(
            window_tab_data.clone(),
            active_editor_tab,
            editor_tab,
            dragging,
        ),
        stack((
            editor_tab_content(
                window_tab_data.clone(),
                plugin.clone(),
                active_editor_tab,
                editor_tab,
            ),
            empty()
                .style(move |s| {
                    let pos = drag_over.get();
                    let width = match pos {
                        Some(pos) => match pos {
                            DragOverPosition::Top => 100.0,
                            DragOverPosition::Bottom => 100.0,
                            DragOverPosition::Left => 50.0,
                            DragOverPosition::Right => 50.0,
                            DragOverPosition::Middle => 100.0,
                        },
                        None => 100.0,
                    };
                    let height = match pos {
                        Some(pos) => match pos {
                            DragOverPosition::Top => 50.0,
                            DragOverPosition::Bottom => 50.0,
                            DragOverPosition::Left => 100.0,
                            DragOverPosition::Right => 100.0,
                            DragOverPosition::Middle => 100.0,
                        },
                        None => 100.0,
                    };
                    let size = tab_size.get_untracked();
                    let margin_left = match pos {
                        Some(pos) => match pos {
                            DragOverPosition::Top => 0.0,
                            DragOverPosition::Bottom => 0.0,
                            DragOverPosition::Left => 0.0,
                            DragOverPosition::Right => size.width / 2.0,
                            DragOverPosition::Middle => 0.0,
                        },
                        None => 0.0,
                    };
                    let margin_top = match pos {
                        Some(pos) => match pos {
                            DragOverPosition::Top => 0.0,
                            DragOverPosition::Bottom => size.height / 2.0,
                            DragOverPosition::Left => 0.0,
                            DragOverPosition::Right => 0.0,
                            DragOverPosition::Middle => 0.0,
                        },
                        None => 0.0,
                    };
                    s.absolute()
                        .size_pct(width, height)
                        .margin_top(margin_top as f32)
                        .margin_left(margin_left as f32)
                        .apply_if(pos.is_none(), |s| s.hide())
                        .background(
                            config
                                .get()
                                .color(LapceColor::EDITOR_DRAG_DROP_BACKGROUND),
                        )
                })
                .debug_name("Drag Over Handle"),
            empty()
                .on_event_stop(EventListener::DragOver, move |event| {
                    if dragging.with_untracked(|dragging| dragging.is_some()) {
                        if let Event::PointerMove(pointer_event) = event {
                            let size = tab_size.get_untracked();
                            let pos = pointer_event.pos;
                            let new_drag_over = if pos.x < size.width / 4.0 {
                                DragOverPosition::Left
                            } else if pos.x > size.width * 3.0 / 4.0 {
                                DragOverPosition::Right
                            } else if pos.y < size.height / 4.0 {
                                DragOverPosition::Top
                            } else if pos.y > size.height * 3.0 / 4.0 {
                                DragOverPosition::Bottom
                            } else {
                                DragOverPosition::Middle
                            };
                            if drag_over.get_untracked() != Some(new_drag_over) {
                                drag_over.set(Some(new_drag_over));
                            }
                        }
                    }
                })
                .on_event_stop(EventListener::DragLeave, move |_| {
                    drag_over.set(None);
                })
                .on_event(EventListener::Drop, move |_| {
                    if let Some((from_index, from_editor_tab_id)) =
                        dragging.get_untracked()
                    {
                        if let Some(pos) = drag_over.get_untracked() {
                            match pos {
                                DragOverPosition::Top => {
                                    main_split.move_editor_tab_child_to_new_split(
                                        from_editor_tab_id,
                                        from_index.get_untracked(),
                                        editor_tab_id,
                                        SplitMoveDirection::Up,
                                    );
                                }
                                DragOverPosition::Bottom => {
                                    main_split.move_editor_tab_child_to_new_split(
                                        from_editor_tab_id,
                                        from_index.get_untracked(),
                                        editor_tab_id,
                                        SplitMoveDirection::Down,
                                    );
                                }
                                DragOverPosition::Left => {
                                    main_split.move_editor_tab_child_to_new_split(
                                        from_editor_tab_id,
                                        from_index.get_untracked(),
                                        editor_tab_id,
                                        SplitMoveDirection::Left,
                                    );
                                }
                                DragOverPosition::Right => {
                                    main_split.move_editor_tab_child_to_new_split(
                                        from_editor_tab_id,
                                        from_index.get_untracked(),
                                        editor_tab_id,
                                        SplitMoveDirection::Right,
                                    );
                                }
                                DragOverPosition::Middle => {
                                    main_split.move_editor_tab_child(
                                        from_editor_tab_id,
                                        editor_tab_id,
                                        from_index.get_untracked(),
                                        editor_tab.with_untracked(|editor_tab| {
                                            editor_tab.active + 1
                                        }),
                                    );
                                }
                            }
                        }
                        drag_over.set(None);
                        EventPropagation::Stop
                    } else {
                        EventPropagation::Continue
                    }
                })
                .on_resize(move |rect| {
                    tab_size.set(rect.size());
                })
                .style(move |s| {
                    s.absolute()
                        .size_full()
                        .apply_if(dragging.get().is_none(), |s| {
                            s.pointer_events_none()
                        })
                }),
        ))
        .debug_name("Editor Content and Drag Over")
        .style(|s| s.size_full()),
    ))
    .on_event_cont(EventListener::PointerDown, move |_| {
        if focus.get_untracked() != Focus::Workbench {
            focus.set(Focus::Workbench);
        }
        let editor_tab_id = editor_tab.with_untracked(|t| t.editor_tab_id);
        internal_command.send(InternalCommand::FocusEditorTab { editor_tab_id });
    })
    .on_cleanup(move || {
        if editor_tabs
            .with_untracked(|editor_tabs| editor_tabs.contains_key(&editor_tab_id))
        {
            return;
        }
        editor_tab
            .with_untracked(|editor_tab| editor_tab.scope)
            .dispose();
    })
    .style(|s| s.flex_col().size_full())
    .debug_name("Editor Tab (Content + Header)")
}

fn tab_secondary_click(
    internal_command: Listener<InternalCommand>,
    editor_tab_id: EditorTabId,
    child: EditorTabChild,
) {
    let mut menu = Menu::new("");
    let child_other = child.clone();
    let child_right = child.clone();
    let child_left = child.clone();
    menu = menu
        .entry(MenuItem::new("Close").action(move || {
            internal_command.send(InternalCommand::EditorTabChildClose {
                editor_tab_id,
                child: child.clone(),
            });
        }))
        .entry(MenuItem::new("Close Other Tabs").action(move || {
            internal_command.send(InternalCommand::EditorTabCloseByKind {
                editor_tab_id,
                child: child_other.clone(),
                kind: TabCloseKind::CloseOther,
            });
        }))
        .entry(MenuItem::new("Close All Tabs").action(move || {
            internal_command.send(InternalCommand::EditorTabClose { editor_tab_id });
        }))
        .entry(MenuItem::new("Close Tabs to the Right").action(move || {
            internal_command.send(InternalCommand::EditorTabCloseByKind {
                editor_tab_id,
                child: child_right.clone(),
                kind: TabCloseKind::CloseToRight,
            });
        }))
        .entry(MenuItem::new("Close Tabs to the Left").action(move || {
            internal_command.send(InternalCommand::EditorTabCloseByKind {
                editor_tab_id,
                child: child_left.clone(),
                kind: TabCloseKind::CloseToLeft,
            });
        }));
    show_context_menu(menu, None);
}
