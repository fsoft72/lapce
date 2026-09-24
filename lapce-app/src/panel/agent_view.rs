//! The AI agent panel: transcript, permission prompt, and input box.

use std::rc::Rc;

use floem::{
    View,
    event::{Event, EventListener},
    peniko::kurbo::Rect,
    prelude::SignalTrack,
    reactive::{
        SignalGet, SignalUpdate, SignalWith, create_memo, create_rw_signal,
    },
    style::CursorStyle,
    views::{
        Decorators, container, dyn_stack,
        editor::view::{LineRegion, cursor_caret},
        label, scroll, stack,
    },
};
use lapce_core::buffer::rope_text::RopeText;
use lapce_rpc::agent::AgentToolStatus;

use super::{
    data::PanelSection, kind::PanelKind, position::PanelPosition, view::PanelBuilder,
};
use crate::{
    agent::{AgentData, AgentItem},
    config::color::LapceColor,
    editor::view::editor_view,
    window_tab::{Focus, WindowTabData},
};

/// Height of the multi-line prompt box in pixels.
const INPUT_HEIGHT: f32 = 110.0;

/// Padding around the text inside the prompt box, as (horizontal, vertical).
const INPUT_PADDING: (f64, f64) = (10.0, 6.0);

/// Builds the agent panel.
pub fn agent_panel(
    window_tab_data: Rc<WindowTabData>,
    position: PanelPosition,
) -> impl View {
    let config = window_tab_data.common.config;
    let agent = window_tab_data.agent.clone();
    PanelBuilder::new(config, position)
        .add(
            "Agent",
            agent_body(window_tab_data.clone(), agent),
            window_tab_data.panel.section_open(PanelSection::Agent),
        )
        .build()
        .debug_name("Agent Panel")
}

/// A small text button.
fn button(
    window_tab_data: Rc<WindowTabData>,
    text: impl Into<String>,
    on_click: impl Fn() + 'static,
) -> impl View {
    let config = window_tab_data.common.config;
    let text: String = text.into();
    label(move || text.clone())
        .on_click_stop(move |_| on_click())
        .style(move |s| {
            let config = config.get();
            s.padding_horiz(8.0)
                .padding_vert(3.0)
                .margin_right(6.0)
                .border(1.0)
                .border_radius(4.0)
                .border_color(config.color(LapceColor::LAPCE_BORDER))
                .cursor(CursorStyle::Pointer)
        })
}

/// One transcript entry.
fn item_view(window_tab_data: Rc<WindowTabData>, item: AgentItem) -> impl View {
    let config = window_tab_data.common.config;
    let (prefix, text) = match &item {
        AgentItem::User(text) => ("You", text.clone()),
        AgentItem::Assistant(text) => ("Agent", text.clone()),
        AgentItem::Thought(text) => ("Thinking", text.clone()),
        AgentItem::Tool { title, status, .. } => {
            let mark = match status {
                AgentToolStatus::Pending => "...",
                AgentToolStatus::InProgress => ">>",
                AgentToolStatus::Completed => "ok",
                AgentToolStatus::Failed => "failed",
            };
            ("Tool", format!("[{mark}] {title}"))
        }
        AgentItem::Error(text) => ("Error", text.clone()),
        AgentItem::Note(text) => ("Note", text.clone()),
    };
    stack((
        label(move || prefix.to_string()).style(move |s| {
            s.font_bold()
                .color(config.get().color(LapceColor::EDITOR_DIM))
        }),
        label(move || text.clone()).style(|s| s.min_width(0.0).flex_grow(1.0f32)),
    ))
    .style(|s| {
        s.flex_col()
            .width_pct(100.0)
            .padding_horiz(10.0)
            .padding_vert(4.0)
    })
}

/// The permission prompt shown while the agent waits for a decision. Only
/// the oldest pending request is shown; answering it reveals the next.
fn permission_bar(
    window_tab_data: Rc<WindowTabData>,
    agent: AgentData,
) -> impl View {
    let config = window_tab_data.common.config;
    let pending = {
        let agent = agent.clone();
        move || {
            agent
                .state
                .with(|state| state.current_permission().cloned())
        }
    };
    dyn_stack(
        move || pending().into_iter().collect::<Vec<_>>(),
        |pending| pending.request_id,
        move |pending| {
            let agent = agent.clone();
            let window_tab_data = window_tab_data.clone();
            let buttons = pending
                .options
                .iter()
                .map(|option| {
                    let agent = agent.clone();
                    let id = option.id.clone();
                    button(window_tab_data.clone(), option.name.clone(), move || {
                        agent.reply_permission(Some(id.clone()));
                    })
                })
                .collect::<Vec<_>>();
            let reject_agent = agent.clone();
            stack((
                label(move || format!("Agent asks: {}", pending.title)),
                stack((
                    floem::views::stack_from_iter(buttons),
                    button(window_tab_data, "Reject", move || {
                        reject_agent.reply_permission(None);
                    }),
                ))
                .style(|s| s.margin_top(4.0)),
            ))
            .style(move |s| {
                s.flex_col()
                    .width_pct(100.0)
                    .padding(8.0)
                    .border_top(1.0)
                    .border_color(config.get().color(LapceColor::LAPCE_BORDER))
            })
        },
    )
    .style(|s| s.flex_col().width_pct(100.0))
}

/// The tall "Send" button next to the prompt box.
fn send_button(
    window_tab_data: Rc<WindowTabData>,
    on_click: impl Fn() + 'static,
) -> impl View {
    let config = window_tab_data.common.config;
    label(|| "Send".to_string())
        .on_click_stop(move |_| on_click())
        .style(move |s| {
            let config = config.get();
            s.padding_horiz(18.0)
                .margin_left(6.0)
                .items_center()
                .justify_center()
                .border(1.0)
                .border_radius(6.0)
                .border_color(config.color(LapceColor::LAPCE_BORDER))
                .cursor(CursorStyle::Pointer)
                .hover(|s| {
                    s.background(config.color(LapceColor::PANEL_HOVERED_BACKGROUND))
                })
                .active(|s| {
                    s.background(
                        config.color(LapceColor::PANEL_HOVERED_ACTIVE_BACKGROUND),
                    )
                })
        })
}

/// The multi-line prompt box (Enter sends, Shift+Enter adds a line) and the Send button.
fn input_box(window_tab_data: Rc<WindowTabData>, agent: AgentData) -> impl View {
    let config = window_tab_data.common.config;
    let focus = window_tab_data.common.focus;
    let editor = agent.input.clone();
    let doc = editor.doc_signal();
    let cursor = editor.cursor();
    let viewport = editor.viewport();
    let window_origin = editor.window_origin();
    let editor = create_rw_signal(editor);
    let is_active = move |tracked: bool| {
        let focus = if tracked {
            focus.get()
        } else {
            focus.get_untracked()
        };
        focus == Focus::Panel(PanelKind::Agent)
    };
    let is_empty = create_memo(move |_| {
        let doc = doc.get();
        doc.buffer.with(|buffer| buffer.len() == 0)
    });
    let debug_breakline = create_memo(move |_| None);
    let (pad_x, pad_y) = INPUT_PADDING;

    let text_area = container({
        scroll({
            let view = stack((
                editor_view(editor.get_untracked(), debug_breakline, is_active),
                label(|| "Ask the agent (Enter to send, Shift+Enter for a new line)".to_string())
                    .style(move |s| {
                        let config = config.get();
                        s.absolute()
                            .items_center()
                            .height(config.editor.line_height() as f32)
                            .color(config.color(LapceColor::EDITOR_DIM))
                            .apply_if(!is_empty.get(), |s| s.hide())
                            .selectable(false)
                    }),
            ))
            .style(move |s| {
                s.absolute()
                    .min_size_pct(100.0, 100.0)
                    .padding_left(pad_x as f32)
                    .padding_vert(pad_y as f32)
                    .hover(|s| s.cursor(CursorStyle::Text))
            });
            let id = view.id();
            view.on_event_cont(EventListener::PointerDown, move |event| {
                focus.set(Focus::Panel(PanelKind::Agent));
                let event = event.clone().offset((pad_x, pad_y));
                if let Event::PointerDown(pointer_event) = event {
                    id.request_active();
                    editor.get_untracked().pointer_down(&pointer_event);
                }
            })
            .on_event_stop(EventListener::PointerMove, move |event| {
                let event = event.clone().offset((pad_x, pad_y));
                if let Event::PointerMove(pointer_event) = event {
                    editor.get_untracked().pointer_move(&pointer_event);
                }
            })
            .on_event_stop(EventListener::PointerUp, move |event| {
                let event = event.clone().offset((pad_x, pad_y));
                if let Event::PointerUp(pointer_event) = event {
                    editor.get_untracked().pointer_up(&pointer_event);
                }
            })
        })
        .on_move(move |pos| {
            window_origin.set(pos + (pad_x, pad_y));
        })
        .on_scroll(move |rect| {
            viewport.set(rect);
        })
        .ensure_visible(move || {
            let cursor = cursor.get();
            let offset = cursor.offset();
            let e_data = editor.get_untracked();
            e_data.doc_signal().track();
            e_data.kind.track();
            let LineRegion { x, width, rvline } = cursor_caret(
                &e_data.editor,
                offset,
                !cursor.is_insert(),
                cursor.affinity,
            );
            let line_height = config.get_untracked().editor.line_height();
            let vline = e_data.editor.vline_of_rvline(rvline);
            Rect::from_origin_size(
                (x, (vline.get() * line_height) as f64),
                (width, line_height as f64),
            )
            .inflate(30.0, 10.0)
        })
        .style(|s| s.absolute().size_pct(100.0, 100.0))
    })
    .style(move |s| {
        let config = config.get();
        s.flex_grow(1.0f32)
            .min_width(0.0)
            .height(INPUT_HEIGHT)
            .border(1.0)
            .padding(-1.0)
            .border_radius(6.0)
            .border_color(config.color(LapceColor::LAPCE_BORDER))
            .background(config.color(LapceColor::EDITOR_BACKGROUND))
    });

    stack((
        text_area,
        send_button(window_tab_data, move || agent.send_input()),
    ))
    .style(move |s| {
        s.items_stretch()
            .width_pct(100.0)
            .padding(6.0)
            .border_top(1.0)
            .border_color(config.get().color(LapceColor::LAPCE_BORDER))
    })
}

/// The whole panel body: toolbar, transcript, permission bar, input box.
fn agent_body(window_tab_data: Rc<WindowTabData>, agent: AgentData) -> impl View {
    let config = window_tab_data.common.config;
    let focus = window_tab_data.common.focus;
    let is_focused = move || focus.get() == Focus::Panel(PanelKind::Agent);

    let toolbar = stack((
        button(window_tab_data.clone(), "Restart", {
            let agent = agent.clone();
            move || agent.restart()
        }),
        button(window_tab_data.clone(), "Cancel", {
            let agent = agent.clone();
            move || agent.cancel()
        }),
        label({
            let agent = agent.clone();
            move || agent.state.with(|state| state.status.to_string())
        })
        .style(move |s| s.color(config.get().color(LapceColor::EDITOR_DIM))),
    ))
    .style(|s| s.padding(6.0).items_center());

    let transcript = scroll(
        dyn_stack(
            {
                let agent = agent.clone();
                move || {
                    agent.state.with(|state| {
                        state.items.iter().cloned().enumerate().collect::<Vec<_>>()
                    })
                }
            },
            |(index, item)| (*index, item.render_key()),
            {
                let window_tab_data = window_tab_data.clone();
                move |(_, item)| item_view(window_tab_data.clone(), item)
            },
        )
        .style(|s| s.flex_col().width_pct(100.0)),
    )
    .style(|s| {
        s.flex_grow(1.0f32)
            .flex_basis(0.0)
            .min_height(0.0)
            .width_pct(100.0)
    });

    let input = input_box(window_tab_data.clone(), agent.clone());

    container(
        stack((
            toolbar,
            transcript,
            permission_bar(window_tab_data.clone(), agent),
            input,
        ))
        .style(|s| s.flex_col().size_pct(100.0, 100.0)),
    )
    .style(|s| s.size_pct(100.0, 100.0))
}
