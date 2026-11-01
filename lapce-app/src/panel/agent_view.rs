//! The AI agent panel: transcript, permission prompt, and input box.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use floem::{
    action::exec_after,
    event::{Event, EventListener, EventPropagation},
    keyboard::{KeyCode, PhysicalKey},
    peniko::kurbo::Rect,
    prelude::SignalTrack,
    reactive::{
        create_effect, create_memo, create_rw_signal, Memo, ReadSignal, RwSignal,
        SignalGet, SignalUpdate, SignalWith,
    },
    style::CursorStyle,
    views::{
        container, dyn_stack,
        editor::{
            text::SystemClipboard,
            text::WrapMethod,
            view::{cursor_caret, LineRegion},
            WrapProp,
        },
        empty, label, rich_text, scroll, stack, Decorators,
    },
    IntoView, View,
};
use lapce_core::{buffer::rope_text::RopeText, register::Clipboard};
use lapce_rpc::agent::{AgentConfigChoice, AgentConfigOption, AgentToolStatus};

use super::{
    data::PanelSection, kind::PanelKind, position::PanelPosition, view::PanelBuilder,
};
use crate::{
    agent::{AgentData, AgentItem, PICKER_VISIBLE_ITEMS},
    config::{color::LapceColor, LapceConfig},
    editor::view::editor_view,
    markdown::{parse_markdown, split_markdown_blocks, MarkdownContent},
    window_tab::{Focus, WindowTabData},
};

/// Vertical padding of the row that shows the agent is working.
const WORKING_ROW_PADDING_VERT: f32 = 3.0;

/// Time between two steps of the "working" animation.
const WORKING_TICK: Duration = Duration::from_millis(400);

/// Number of steps of the "working" animation (zero to three dots).
const WORKING_STEPS: usize = 4;

/// Line height factor used when rendering Markdown replies.
const MARKDOWN_LINE_HEIGHT: f64 = 1.8;

/// Height of the multi-line prompt box in pixels.
const INPUT_HEIGHT: f32 = 110.0;

/// Padding around the text inside the prompt box, as (horizontal, vertical).
const INPUT_PADDING: (f64, f64) = (10.0, 6.0);

/// Room left at the right of the text for the caret when wrapping lines.
const INPUT_WRAP_MARGIN: f64 = 10.0;

/// Narrowest width the prompt text is ever wrapped at.
const INPUT_MIN_WRAP_WIDTH: f64 = 50.0;

/// Room kept visible around the caret when the prompt box scrolls to it, as
/// (horizontal, vertical).
const CARET_SCROLL_MARGIN: (f64, f64) = (30.0, 10.0);

/// Padding around the toolbar and the prompt row.
const PANEL_PADDING: f32 = 6.0;

/// Corner radius of the prompt box, the user prompts, the Send button and
/// the toast.
const BOX_RADIUS: f32 = 6.0;

/// Padding inside the small text buttons, as (horizontal, vertical).
const BUTTON_PADDING: (f32, f32) = (8.0, 3.0);

/// Corner radius of the small text buttons.
const BUTTON_RADIUS: f32 = 4.0;

/// Space between a button and the next one or the prompt box.
const BUTTON_GAP: f32 = 6.0;

/// Horizontal padding of the Send button.
const SEND_BUTTON_PADDING_HORIZ: f32 = 18.0;

/// Padding of a transcript entry, as (horizontal, vertical).
const ITEM_PADDING: (f32, f32) = (10.0, 4.0);

/// Space above a row of buttons, and around a user prompt.
const ITEM_SPACING: f32 = 4.0;

/// Vertical padding of a user prompt, inside its tinted box.
const USER_PROMPT_PADDING_VERT: f32 = 6.0;

/// Height reserved below the prompt box for the model and effort row.
const CONFIG_ROW_HEIGHT: f32 = 30.0;

/// Vertical padding of the title line of the filter list.
const PICKER_TITLE_PADDING_VERT: f32 = 5.0;

/// Space above and below a Markdown horizontal rule.
const SEPARATOR_MARGIN: f32 = 5.0;

/// Padding of the permission bar.
const PERMISSION_BAR_PADDING: f32 = 8.0;

/// Padding of an entry of the mention list, as (horizontal, vertical).
const MENTION_ITEM_PADDING: (f32, f32) = (10.0, 3.0);

/// Space between the top of the prompt row and the toast above it.
const TOAST_GAP_ABOVE_INPUT: f32 = 30.0;

/// Distance of the toast from the left edge of the panel.
const TOAST_INSET_LEFT: f32 = 10.0;

/// Padding inside the toast, as (horizontal, vertical).
const TOAST_PADDING: (f32, f32) = (10.0, 5.0);

/// Builds the agent panel.
pub fn agent_panel(
    window_tab_data: Rc<WindowTabData>,
    position: PanelPosition,
) -> impl View {
    let config = window_tab_data.common.config;
    let agent = window_tab_data.agent.clone();
    PanelBuilder::new(config, position)
        .add_with_action(
            "Agent",
            button(window_tab_data.clone(), "New Chat", {
                let agent = agent.clone();
                move || agent.new_chat()
            }),
            agent_body(window_tab_data.clone(), agent),
            window_tab_data.panel.section_open(PanelSection::Agent),
        )
        .build()
        .debug_name("Agent Panel")
}

/// The text of the activity row for animation step `step`: "Working" followed
/// by zero to three dots, so the user sees the agent has not stalled.
fn working_text(step: usize) -> String {
    format!("Working{}", ".".repeat(step % WORKING_STEPS))
}

/// Advances `step` every [`WORKING_TICK`] while `busy` is set. `generation`
/// makes a loop stop when a newer one started, so a quick busy, idle, busy
/// sequence does not leave two loops running.
fn run_working_animation(
    busy: Memo<bool>,
    step: RwSignal<usize>,
    generation: RwSignal<u64>,
    id: u64,
) {
    exec_after(WORKING_TICK, move |_| {
        if generation.get_untracked() != id || !busy.get_untracked() {
            return;
        }
        step.update(|step| *step += 1);
        run_working_animation(busy, step, generation, id);
    });
}

/// A row under the transcript that is visible while a turn runs, with an
/// animated "Working..." text.
fn working_row(window_tab_data: Rc<WindowTabData>, agent: AgentData) -> impl View {
    let config = window_tab_data.common.config;
    let busy = agent.memos.busy;
    let step = create_rw_signal(0usize);
    let generation = create_rw_signal(0u64);
    create_effect(move |_| {
        let id = generation.get_untracked() + 1;
        generation.set(id);
        if busy.get() {
            step.set(0);
            run_working_animation(busy, step, generation, id);
        }
    });
    label(move || working_text(step.get())).style(move |s| {
        s.width_pct(100.0)
            .padding_horiz(PANEL_PADDING)
            .padding_vert(WORKING_ROW_PADDING_VERT)
            .color(config.get().color(LapceColor::EDITOR_DIM))
            .apply_if(!busy.get(), |s| s.hide())
    })
}

/// Whether `key` presses a focused button: Enter, keypad Enter or Space,
/// the same keys floem turns into a click on the focused view.
fn is_activation_key(key: &PhysicalKey) -> bool {
    matches!(
        key,
        PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space)
    )
}

/// Makes `view` a button that the mouse and the keyboard can press: it is
/// reached with Tab, and Enter or Space press it while it has the focus.
/// floem already turns those keys into a click on the focused view; the key
/// is then stopped here so the panel keymap does not also see it (Enter
/// would send the prompt).
fn pressable(view: impl IntoView, on_click: impl Fn() + 'static) -> impl View {
    view.into_view()
        .keyboard_navigable()
        .on_click_stop(move |_| on_click())
        .on_event(EventListener::KeyDown, |event| match event {
            Event::KeyDown(key) if is_activation_key(&key.key.physical_key) => {
                EventPropagation::Stop
            }
            _ => EventPropagation::Continue,
        })
}

/// A small text button.
fn button(
    window_tab_data: Rc<WindowTabData>,
    text: impl Into<String>,
    on_click: impl Fn() + 'static,
) -> impl View {
    let config = window_tab_data.common.config;
    let text: String = text.into();
    pressable(label(move || text.clone()), on_click).style(move |s| {
        let config = config.get();
        s.padding_horiz(BUTTON_PADDING.0)
            .padding_vert(BUTTON_PADDING.1)
            .margin_right(BUTTON_GAP)
            .border(1.0)
            .border_radius(BUTTON_RADIUS)
            .border_color(config.color(LapceColor::LAPCE_BORDER))
            .cursor(CursorStyle::Pointer)
            .focus_visible(|s| {
                s.border_color(config.color(LapceColor::EDITOR_FOCUS))
            })
    })
}

/// Renders the Markdown returned by `text`, which may grow while a reply
/// streams in. The text is cut into top-level blocks and each block keeps
/// its view while its content stays the same, so a new chunk only parses and
/// lays out the last block again. Images are not shown.
fn markdown_view(
    config: ReadSignal<Arc<LapceConfig>>,
    text: impl Fn() -> String + 'static,
) -> impl View {
    dyn_stack(
        move || {
            let text = text();
            split_markdown_blocks(&text)
                .into_iter()
                .map(str::to_string)
                .enumerate()
                .collect::<Vec<_>>()
        },
        |(index, block)| {
            let mut hasher = DefaultHasher::new();
            block.hash(&mut hasher);
            (*index, hasher.finish())
        },
        move |(_, block)| markdown_block_view(config, block),
    )
    .style(|s| s.flex_col().width_full().min_width(0.0))
}

/// Renders one top-level Markdown block, again when the config changes.
fn markdown_block_view(
    config: ReadSignal<Arc<LapceConfig>>,
    block: String,
) -> impl View {
    let id = AtomicU64::new(0);
    dyn_stack(
        move || parse_markdown(&block, MARKDOWN_LINE_HEIGHT, &config.get()),
        move |_| id.fetch_add(1, Ordering::Relaxed),
        move |content| match content {
            MarkdownContent::Text(text_layout) => container(
                rich_text(move || text_layout.clone()).style(|s| s.width_full()),
            )
            .style(|s| s.width_full()),
            MarkdownContent::Image { .. } => container(empty()),
            MarkdownContent::Separator => container(empty().style(move |s| {
                s.width_full()
                    .margin_vert(SEPARATOR_MARGIN)
                    .height(1.0)
                    .background(config.get().color(LapceColor::LAPCE_BORDER))
            })),
        },
    )
    .style(|s| s.flex_col().width_full().min_width(0.0))
}

/// The label and the text shown for a transcript entry.
fn item_display(item: &AgentItem) -> (&'static str, String) {
    match item {
        AgentItem::User { text, .. } => ("You", text.clone()),
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
    }
}

/// The transcript entry at `index`. It only follows the revision of its own
/// entry, so a streamed chunk re-renders the growing reply and nothing else,
/// and the other entries are never copied out of the state again.
fn item_view(window_tab_data: Rc<WindowTabData>, index: usize) -> impl View {
    let config = window_tab_data.common.config;
    let state = window_tab_data.agent.state;
    let revision = create_memo(move |_| state.with(|s| s.revision(index)));
    let item = create_memo(move |_| {
        revision.track();
        state.with_untracked(|s| s.item(index).cloned())
    });
    // The kind of an entry never changes: only its text and status do.
    let (prefix, is_user, is_assistant) = item.with_untracked(|item| match item {
        Some(item) => (
            item_display(item).0,
            matches!(item, AgentItem::User { .. }),
            matches!(item, AgentItem::Assistant(_)),
        ),
        None => ("", false, false),
    });
    let text = move || {
        item.with(|item| item.as_ref().map(|i| item_display(i).1))
            .unwrap_or_default()
    };
    let text_untracked = move || {
        item.with_untracked(|item| item.as_ref().map(|i| item_display(i).1))
            .unwrap_or_default()
    };
    // Replies are Markdown; everything else is plain text.
    let body = if is_assistant {
        let agent = window_tab_data.agent.clone();
        stack((
            markdown_view(config, text),
            button(window_tab_data.clone(), "Copy", move || {
                SystemClipboard::new().put_string(text_untracked());
                agent.show_toast("Copied to clipboard");
            })
            .style(|s| s.margin_top(ITEM_SPACING)),
        ))
        .style(|s| s.flex_col().items_start().width_full().min_width(0.0))
        .into_any()
    } else if is_user {
        let edit_agent = window_tab_data.agent.clone();
        let rerun_agent = window_tab_data.agent.clone();
        let stop_agent = window_tab_data.agent.clone();
        let busy = window_tab_data.agent.memos.busy;
        // Only the prompt whose turn is running offers Stop instead of Edit and Rerun.
        let running = create_memo(move |_| {
            busy.get() && state.with(|s| s.last_user_index()) == Some(index)
        });
        stack((
            label(text).style(|s| s.min_width(0.0).flex_grow(1.0f32)),
            stack((
                button(window_tab_data.clone(), "Edit", move || {
                    edit_agent.edit_in_input(&text_untracked());
                })
                .style(move |s| s.apply_if(running.get(), |s| s.hide())),
                button(window_tab_data.clone(), "Rerun", move || {
                    rerun_agent.rerun(index);
                })
                .style(move |s| s.apply_if(running.get(), |s| s.hide())),
                button(window_tab_data.clone(), "Stop", move || {
                    stop_agent.cancel();
                })
                .style(move |s| s.apply_if(!running.get(), |s| s.hide())),
            ))
            .style(|s| s.margin_top(ITEM_SPACING)),
        ))
        .style(|s| s.flex_col().items_start().width_full().min_width(0.0))
        .into_any()
    } else {
        label(text)
            .style(|s| s.min_width(0.0).flex_grow(1.0f32))
            .into_any()
    };
    stack((
        label(move || prefix.to_string()).style(move |s| {
            s.font_bold()
                .color(config.get().color(LapceColor::EDITOR_DIM))
        }),
        body,
    ))
    .style(move |s| {
        let config = config.get();
        // The user's prompts sit in a tinted box so they stand out from the replies.
        s.flex_col()
            .width_pct(100.0)
            .padding_horiz(ITEM_PADDING.0)
            .padding_vert(ITEM_PADDING.1)
            .apply_if(is_user, |s| {
                s.margin_vert(ITEM_SPACING)
                    .padding_vert(USER_PROMPT_PADDING_VERT)
                    .border_radius(BOX_RADIUS)
                    .background(config.color(LapceColor::PANEL_HOVERED_BACKGROUND))
            })
    })
}

/// The permission prompt shown while the agent waits for a decision. Only
/// the oldest pending request is shown; answering it reveals the next.
fn permission_bar(
    window_tab_data: Rc<WindowTabData>,
    agent: AgentData,
) -> impl View {
    let config = window_tab_data.common.config;
    let pending = agent.memos.permission;
    dyn_stack(
        move || pending.get().into_iter().collect::<Vec<_>>(),
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
                .style(|s| s.margin_top(ITEM_SPACING)),
            ))
            .style(move |s| {
                s.flex_col()
                    .width_pct(100.0)
                    .padding(PERMISSION_BAR_PADDING)
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
    pressable(label(|| "Send".to_string()), on_click).style(move |s| {
        let config = config.get();
        s.padding_horiz(SEND_BUTTON_PADDING_HORIZ)
            .margin_left(BUTTON_GAP)
            .items_center()
            .justify_center()
            .border(1.0)
            .border_radius(BOX_RADIUS)
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
            .focus_visible(|s| {
                s.border_color(config.color(LapceColor::EDITOR_FOCUS))
            })
    })
}

/// The list of files matching the `@mention` being typed, shown above the prompt box.
fn mention_list(window_tab_data: Rc<WindowTabData>, agent: AgentData) -> impl View {
    let config = window_tab_data.common.config;
    let items = agent.mention_items;
    let selected = agent.mention_selected;
    let visible = create_memo({
        let agent = agent.clone();
        move |_| {
            agent.mention.with(|mention| mention.is_some())
                && items.with(|items| !items.is_empty())
        }
    });
    dyn_stack(
        move || items.get().into_iter().enumerate().collect::<Vec<_>>(),
        |(index, file)| (*index, file.clone()),
        move |(index, file)| {
            let agent = agent.clone();
            label(move || file.clone())
                .on_click_stop(move |_| {
                    agent.mention_selected.set(index);
                    agent.accept_mention();
                })
                .style(move |s| {
                    let config = config.get();
                    s.width_pct(100.0)
                        .padding_horiz(MENTION_ITEM_PADDING.0)
                        .padding_vert(MENTION_ITEM_PADDING.1)
                        .cursor(CursorStyle::Pointer)
                        .apply_if(selected.get() == index, |s| {
                            s.background(
                                config.color(LapceColor::PANEL_HOVERED_BACKGROUND),
                            )
                        })
                })
        },
    )
    .style(move |s| {
        let config = config.get();
        s.flex_col()
            .width_pct(100.0)
            .border_top(1.0)
            .border_color(config.color(LapceColor::LAPCE_BORDER))
            .background(config.color(LapceColor::PANEL_BACKGROUND))
            .apply_if(!visible.get(), |s| s.hide())
    })
}

/// A button showing the current value of `option`. Clicking it opens the
/// filter list of the values. It is hidden when the agent does not offer the
/// setting, and does nothing while a turn runs.
fn config_selector(
    window_tab_data: Rc<WindowTabData>,
    agent: AgentData,
    option: Memo<Option<AgentConfigOption>>,
) -> impl View {
    let config = window_tab_data.common.config;
    let busy = agent.memos.busy;
    let text = move || {
        option.with(|option| {
            option
                .as_ref()
                .map_or(String::new(), |option| option.label())
        })
    };
    let open_menu = move || {
        let Some(option) = option.get_untracked() else {
            return;
        };
        if busy.get_untracked() {
            return agent.show_toast("Wait for the agent to finish its turn");
        }
        agent.open_picker(option);
    };
    pressable(label(text), open_menu).style(move |s| {
        let config = config.get();
        s.padding_horiz(BUTTON_PADDING.0)
            .padding_vert(BUTTON_PADDING.1)
            .margin_right(BUTTON_GAP)
            .border(1.0)
            .border_radius(BUTTON_RADIUS)
            .border_color(config.color(LapceColor::LAPCE_BORDER))
            .cursor(CursorStyle::Pointer)
            .apply_if(option.with(|option| option.is_none()), |s| s.hide())
            .apply_if(busy.get(), |s| {
                s.color(config.color(LapceColor::EDITOR_DIM))
            })
            .focus_visible(|s| {
                s.border_color(config.color(LapceColor::EDITOR_FOCUS))
            })
    })
}

/// The filter list of a setting (model, effort), shown above the prompt box:
/// the text typed so far, then the matching values. Up and Down move the
/// highlight, Enter picks it and Esc closes the list.
fn picker_list(window_tab_data: Rc<WindowTabData>, agent: AgentData) -> impl View {
    let config = window_tab_data.common.config;
    let (picker, query) = (agent.picker, agent.picker_query);
    let (items, selected, top) =
        (agent.picker_items, agent.picker_selected, agent.picker_top);
    let title = move || {
        let name = picker.with(|picker| {
            picker
                .as_ref()
                .map(|picker| picker.name.clone())
                .unwrap_or_default()
        });
        format!("{name}: {}|", query.get())
    };
    let rows = dyn_stack(
        move || {
            let top = top.get();
            items
                .get()
                .into_iter()
                .enumerate()
                .skip(top)
                .take(PICKER_VISIBLE_ITEMS)
                .collect::<Vec<_>>()
        },
        |(index, choice)| (*index, choice.value.clone()),
        {
            let agent = agent.clone();
            move |(index, choice): (usize, AgentConfigChoice)| {
                let agent = agent.clone();
                let current = picker.with_untracked(|picker| {
                    picker
                        .as_ref()
                        .is_some_and(|picker| picker.current == choice.value)
                });
                let text =
                    format!("{}{}", if current { "* " } else { "  " }, choice.name);
                label(move || text.clone())
                    .on_click_stop(move |_| {
                        agent.set_picker_selected(index);
                        agent.accept_picker();
                    })
                    .style(move |s| {
                        let config = config.get();
                        s.width_pct(100.0)
                            .padding_horiz(MENTION_ITEM_PADDING.0)
                            .padding_vert(MENTION_ITEM_PADDING.1)
                            .cursor(CursorStyle::Pointer)
                            .apply_if(selected.get() == index, |s| {
                                s.background(
                                    config
                                        .color(LapceColor::PANEL_HOVERED_BACKGROUND),
                                )
                            })
                    })
            }
        },
    )
    .style(|s| s.flex_col().width_pct(100.0));
    stack((
        label(title).style(move |s| {
            s.width_pct(100.0)
                .padding_horiz(MENTION_ITEM_PADDING.0)
                .padding_vert(PICKER_TITLE_PADDING_VERT)
                .font_bold()
                .border_bottom(1.0)
                .border_color(config.get().color(LapceColor::LAPCE_BORDER))
        }),
        rows,
        label(|| "No match".to_string()).style(move |s| {
            s.padding_horiz(MENTION_ITEM_PADDING.0)
                .padding_vert(MENTION_ITEM_PADDING.1)
                .color(config.get().color(LapceColor::EDITOR_DIM))
                .apply_if(items.with(|items| !items.is_empty()), |s| s.hide())
        }),
    ))
    .style(move |s| {
        let config = config.get();
        s.flex_col()
            .width_pct(100.0)
            .border_top(1.0)
            .border_color(config.color(LapceColor::LAPCE_BORDER))
            .background(config.color(LapceColor::PANEL_BACKGROUND))
            .apply_if(picker.with(|picker| picker.is_none()), |s| s.hide())
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
    // A local editor is as wide as its text, and the editor copies that width
    // into its viewport, so the default "wrap at the editor width" would wrap
    // every letter. Wrap at the width of the box instead.
    let wrap_width = create_rw_signal(0.0f32);

    let text_area = container({
        scroll({
            let view = stack((
                editor_view(editor.get_untracked(), debug_breakline, is_active)
                    .style(move |s| match wrap_width.get() {
                        width if width > 0.0 => {
                            s.set(WrapProp, WrapMethod::WrapWidth { width })
                        }
                        _ => s,
                    }),
                label(|| {
                    "Ask the agent (Enter to send, Shift+Enter for a new line)"
                        .to_string()
                })
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
            let text_width = rect.width() - pad_x - INPUT_WRAP_MARGIN;
            wrap_width.set(text_width.max(INPUT_MIN_WRAP_WIDTH) as f32);
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
            .inflate(CARET_SCROLL_MARGIN.0, CARET_SCROLL_MARGIN.1)
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
            .border_radius(BOX_RADIUS)
            .border_color(config.color(LapceColor::LAPCE_BORDER))
            .background(config.color(LapceColor::EDITOR_BACKGROUND))
    });

    let selectors = stack((
        config_selector(window_tab_data.clone(), agent.clone(), agent.memos.model),
        config_selector(window_tab_data.clone(), agent.clone(), agent.memos.effort),
    ))
    .style(|s| s.items_center().height(CONFIG_ROW_HEIGHT));

    stack((
        stack((
            text_area,
            send_button(window_tab_data, move || agent.send_input()),
        ))
        .style(|s| s.items_stretch().width_pct(100.0)),
        selectors,
    ))
    .style(move |s| {
        s.flex_col()
            .width_pct(100.0)
            .padding(PANEL_PADDING)
            .border_top(1.0)
            .border_color(config.get().color(LapceColor::LAPCE_BORDER))
    })
}

/// The whole panel body: toolbar, transcript, permission bar, input box.
fn agent_body(window_tab_data: Rc<WindowTabData>, agent: AgentData) -> impl View {
    let config = window_tab_data.common.config;

    let toolbar = stack((
        button(window_tab_data.clone(), "Restart", {
            let agent = agent.clone();
            move || agent.restart()
        }),
        label({
            let status = agent.memos.status;
            move || status.with(|status| status.to_string())
        })
        .style(move |s| s.color(config.get().color(LapceColor::EDITOR_DIM))),
    ))
    .style(|s| s.padding(PANEL_PADDING).items_center());

    let transcript = scroll(
        dyn_stack(
            {
                let item_count = agent.memos.item_count;
                move || (0..item_count.get()).collect::<Vec<_>>()
            },
            |index| *index,
            {
                let window_tab_data = window_tab_data.clone();
                move |index| item_view(window_tab_data.clone(), index)
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

    let toast = {
        let toast = agent.toast;
        label(move || toast.get().unwrap_or_default()).style(move |s| {
            let config = config.get();
            s.absolute()
                .inset_bottom(
                    INPUT_HEIGHT + CONFIG_ROW_HEIGHT + TOAST_GAP_ABOVE_INPUT,
                )
                .inset_left(TOAST_INSET_LEFT)
                .padding_horiz(TOAST_PADDING.0)
                .padding_vert(TOAST_PADDING.1)
                .border(1.0)
                .border_radius(BOX_RADIUS)
                .border_color(config.color(LapceColor::LAPCE_BORDER))
                .background(config.color(LapceColor::PANEL_BACKGROUND))
                .apply_if(toast.with(|toast| toast.is_none()), |s| s.hide())
        })
    };

    container(
        stack((
            stack((
                toolbar,
                transcript,
                working_row(window_tab_data.clone(), agent.clone()),
                permission_bar(window_tab_data.clone(), agent.clone()),
                mention_list(window_tab_data.clone(), agent.clone()),
                picker_list(window_tab_data.clone(), agent),
                input,
            ))
            .style(|s| s.flex_col().size_pct(100.0, 100.0)),
            toast,
        ))
        .style(|s| s.size_pct(100.0, 100.0)),
    )
    .style(|s| s.size_pct(100.0, 100.0))
}

#[cfg(test)]
mod tests {
    use floem::keyboard::{KeyCode, PhysicalKey};

    use super::{is_activation_key, working_text};

    #[test]
    fn the_working_text_cycles_through_zero_to_three_dots() {
        let texts: Vec<_> = (0..5).map(working_text).collect();
        assert_eq!(
            texts,
            ["Working", "Working.", "Working..", "Working...", "Working"]
        );
    }

    #[test]
    fn enter_and_space_press_a_focused_button() {
        for code in [KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space] {
            assert!(is_activation_key(&PhysicalKey::Code(code)));
        }
    }

    #[test]
    fn tab_and_letters_do_not_press_a_button() {
        for code in [KeyCode::Tab, KeyCode::KeyA, KeyCode::Escape] {
            assert!(!is_activation_key(&PhysicalKey::Code(code)));
        }
    }
}
