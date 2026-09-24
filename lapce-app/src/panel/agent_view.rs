//! The AI agent panel: transcript, permission prompt, and input box.

use std::rc::Rc;

use floem::{
    View,
    event::EventListener,
    reactive::{SignalGet, SignalUpdate, SignalWith},
    style::CursorStyle,
    views::{Decorators, container, dyn_stack, label, scroll, stack},
};
use lapce_rpc::agent::AgentToolStatus;

use super::{
    data::PanelSection, kind::PanelKind, position::PanelPosition, view::PanelBuilder,
};
use crate::{
    agent::{AgentData, AgentItem},
    config::color::LapceColor,
    text_input::TextInputBuilder,
    window_tab::{Focus, WindowTabData},
};

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

/// The permission prompt shown while the agent waits for a decision.
fn permission_bar(
    window_tab_data: Rc<WindowTabData>,
    agent: AgentData,
) -> impl View {
    let config = window_tab_data.common.config;
    let pending = {
        let agent = agent.clone();
        move || agent.state.with(|state| state.pending.clone())
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
            move || agent.state.with(|state| format!("{:?}", state.status))
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

    let input = stack((
        TextInputBuilder::new()
            .is_focused(is_focused)
            .build_editor(agent.input.clone())
            .placeholder(|| "Ask the agent (Enter to send)".to_string())
            .style(|s| s.padding_vert(4.0).padding_horiz(10.0).width_pct(100.0))
            .on_event_cont(EventListener::PointerDown, move |_| {
                focus.set(Focus::Panel(PanelKind::Agent));
            }),
        button(window_tab_data.clone(), "Send", {
            let agent = agent.clone();
            move || agent.send_input()
        }),
    ))
    .style(move |s| {
        s.items_center()
            .width_pct(100.0)
            .border_top(1.0)
            .border_color(config.get().color(LapceColor::LAPCE_BORDER))
    });

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
