//! The AI agent panel: transcript, permission prompt, and input box.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use floem::{
    IntoView, View,
    action::exec_after,
    event::{Event, EventListener, EventPropagation},
    keyboard::{KeyCode, PhysicalKey},
    peniko::Color,
    peniko::kurbo::Rect,
    prelude::SignalTrack,
    reactive::{
        Memo, ReadSignal, RwSignal, SignalGet, SignalUpdate, SignalWith,
        create_effect, create_memo, create_rw_signal,
    },
    style::{CursorStyle, Style},
    taffy::JustifyContent,
    views::{
        Decorators, container, dyn_stack,
        editor::{
            WrapProp,
            text::SystemClipboard,
            text::WrapMethod,
            view::{LineRegion, cursor_caret},
        },
        empty, label, scroll, stack,
    },
};
use lapce_core::{buffer::rope_text::RopeText, register::Clipboard};
use lapce_rpc::agent::{AgentConfigChoice, AgentConfigOption, AgentToolStatus};

use super::{
    data::PanelSection, kind::PanelKind, position::PanelPosition,
    selectable_text::selectable_text, view::PanelBuilder,
};
use crate::{
    agent::{AgentData, AgentItem, PICKER_VISIBLE_ITEMS},
    config::{LapceConfig, color::LapceColor},
    editor::view::editor_view,
    markdown::{MarkdownContent, parse_markdown, split_markdown_blocks},
    window_tab::{Focus, WindowTabData},
};

/// How far from the end of the transcript still counts as "at the end", so
/// the transcript keeps following the new content.
const FOLLOW_END_MARGIN: f64 = 20.0;

/// Space between the text of a message and its buttons.
const BUTTONS_MARGIN_TOP: f32 = 10.0;

/// Vertical padding of the boxes of thoughts and tool calls.
const TINTED_BOX_PADDING_VERT: f32 = 6.0;

/// Vertical padding of the box of an answer of the agent.
const ASSISTANT_PADDING_VERT: f32 = 8.0;

/// Width of the frame of the box of an answer of the agent.
const ASSISTANT_BORDER_WIDTH: f32 = 1.5;

/// Padding of a row of the changed files (horizontal, vertical).
const CHANGES_ROW_PADDING: (f32, f32) = (12.0, 5.0);

/// Space between the parts of a row of the changed files.
const CHANGES_ROW_GAP: f32 = 10.0;

/// Font size of the title of the changed files box.
const CHANGES_TITLE_FONT_SIZE: f32 = 14.0;

/// Vertical padding of the title of the changed files box.
const CHANGES_TITLE_PADDING_VERT: f32 = 8.0;

/// Padding of the Approve and Revert buttons (horizontal, vertical).
const REVIEW_BUTTON_PADDING: (f32, f32) = (16.0, 6.0);

/// Tallest the list of changed files grows before it scrolls.
const CHANGES_LIST_MAX_HEIGHT: f32 = 140.0;

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
            stack((
                button(window_tab_data.clone(), "Clear", {
                    let agent = agent.clone();
                    move || agent.clear_messages()
                }),
                button(window_tab_data.clone(), "New Chat", {
                    let agent = agent.clone();
                    move || agent.new_chat()
                }),
            )),
            agent_body(window_tab_data.clone(), agent),
            window_tab_data.panel.section_open(PanelSection::Agent),
        )
        .build()
        .debug_name("Agent Panel")
}

/// Whether the transcript keeps following new content after the viewport
/// moved from `previous_top` to `top..bottom` over `content_height` of
/// content. Reaching the end turns it on; scrolling up turns it off; any
/// other move (the content grew under the viewport) keeps it as it was.
fn follow_after_scroll(
    following: bool,
    previous_top: f64,
    top: f64,
    bottom: f64,
    content_height: f64,
) -> bool {
    if bottom >= content_height - FOLLOW_END_MARGIN {
        return true;
    }
    if top < previous_top - 0.5 {
        return false;
    }
    following
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
            MarkdownContent::Text(text_layout) => {
                container(selectable_text(text_layout, move || {
                    config.get_untracked().color(LapceColor::EDITOR_SELECTION)
                }))
                .style(|s| s.width_full())
            }
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

/// Styles a transcript entry as a box tinted with `color`: a translucent
/// background and a frame of the same color, so it reads well in the dark
/// and in the light theme.
fn tinted_box(style: Style, color: Color, background_alpha: f32) -> Style {
    style
        .margin_vert(ITEM_SPACING)
        .padding_vert(TINTED_BOX_PADDING_VERT)
        .border(1.0)
        .border_radius(BOX_RADIUS)
        .border_color(color.multiply_alpha(0.5))
        .background(color.multiply_alpha(background_alpha))
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
    let (prefix, is_user, is_assistant, is_thought, is_tool) =
        item.with_untracked(|item| match item {
            Some(item) => (
                item_display(item).0,
                matches!(item, AgentItem::User { .. }),
                matches!(item, AgentItem::Assistant(_)),
                matches!(item, AgentItem::Thought(_)),
                matches!(item, AgentItem::Tool { .. }),
            ),
            None => ("", false, false, false, false),
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
            .style(|s| s.margin_top(BUTTONS_MARGIN_TOP).margin_right(0.0)),
        ))
        .style(|s| s.flex_col().items_end().width_full().min_width(0.0))
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
            label(text).style(|s| s.min_width(0.0).width_full()),
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
            .style(|s| {
                s.margin_top(BUTTONS_MARGIN_TOP)
                    .width_full()
                    .justify_content(Some(JustifyContent::FlexEnd))
            }),
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
            .apply_if(is_thought, |s| {
                tinted_box(s, config.color(LapceColor::TERMINAL_YELLOW), 0.18)
            })
            .apply_if(is_tool, |s| {
                tinted_box(s, config.color(LapceColor::TERMINAL_CYAN), 0.16)
            })
            .apply_if(is_assistant, |s| {
                // The answers are what the user reads: a framed box on the
                // editor background, brighter than the other entries.
                s.margin_vert(ITEM_SPACING)
                    .padding_vert(ASSISTANT_PADDING_VERT)
                    .border(ASSISTANT_BORDER_WIDTH)
                    .border_radius(BOX_RADIUS)
                    .border_color(
                        config
                            .color(LapceColor::LAPCE_TAB_ACTIVE_UNDERLINE)
                            .multiply_alpha(0.8),
                    )
                    .background(config.color(LapceColor::EDITOR_BACKGROUND))
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

/// What a row of the changed files shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ChangeRow {
    path: PathBuf,
    /// The file name.
    name: String,
    /// The folder of the file, relative to the workspace; empty at its root.
    folder: String,
    /// Lines added and removed, when they could be counted.
    counts: Option<(usize, usize)>,
}

/// Builds the row of the file at `path`, showing it relative to `root`.
fn change_row(
    path: &Path,
    root: Option<&Path>,
    counts: Option<(usize, usize)>,
) -> ChangeRow {
    let shown = root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    ChangeRow {
        path: path.to_path_buf(),
        name: shown
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| shown.display().to_string()),
        folder: shown
            .parent()
            .map(|folder| folder.display().to_string())
            .unwrap_or_default(),
        counts,
    }
}

/// One row of the changed files: the name, its folder and the line counts,
/// green for added and red for removed. A click opens the diff.
fn change_row_view(
    window_tab_data: Rc<WindowTabData>,
    agent: AgentData,
    row: ChangeRow,
) -> impl View {
    let config = window_tab_data.common.config;
    let path = row.path.clone();
    let (added, removed) = row.counts.unwrap_or_default();
    let has_counts = row.counts.is_some();
    stack((
        label(move || row.name.clone()).style(|s| s.font_bold()),
        label(move || row.folder.clone()).style(move |s| {
            s.margin_left(CHANGES_ROW_GAP)
                .color(config.get().color(LapceColor::EDITOR_DIM))
        }),
        empty().style(|s| s.flex_grow(1.0f32)),
        label(move || format!("+{added}")).style(move |s| {
            s.margin_left(CHANGES_ROW_GAP)
                .font_bold()
                .color(config.get().color(LapceColor::SOURCE_CONTROL_ADDED))
                .apply_if(!has_counts, |s| s.hide())
        }),
        label(move || format!("-{removed}")).style(move |s| {
            s.margin_left(CHANGES_ROW_GAP)
                .font_bold()
                .color(config.get().color(LapceColor::SOURCE_CONTROL_REMOVED))
                .apply_if(!has_counts, |s| s.hide())
        }),
    ))
    .on_click_stop(move |_| agent.open_change_diff(&path))
    .style(move |s| {
        let config = config.get();
        s.width_pct(100.0)
            .items_center()
            .padding_horiz(CHANGES_ROW_PADDING.0)
            .padding_vert(CHANGES_ROW_PADDING.1)
            .cursor(CursorStyle::Pointer)
            .hover(|s| {
                s.background(config.color(LapceColor::PANEL_HOVERED_BACKGROUND))
            })
    })
}

/// A large button of the changed files box: `accent` colors its border and
/// text, and fills it when `filled`.
fn review_button(
    window_tab_data: Rc<WindowTabData>,
    text: &'static str,
    accent: &'static str,
    filled: bool,
    on_click: impl Fn() + 'static,
) -> impl View {
    let config = window_tab_data.common.config;
    pressable(label(move || text.to_string()), on_click).style(move |s| {
        let config = config.get();
        let color = config.color(accent);
        s.padding_horiz(REVIEW_BUTTON_PADDING.0)
            .padding_vert(REVIEW_BUTTON_PADDING.1)
            .margin_right(BUTTON_GAP)
            .border(1.5)
            .border_radius(BUTTON_RADIUS)
            .border_color(color)
            .font_bold()
            .cursor(CursorStyle::Pointer)
            .color(if filled {
                config.color(LapceColor::LAPCE_BUTTON_PRIMARY_FOREGROUND)
            } else {
                color
            })
            .apply_if(filled, |s| s.background(color))
            .hover(|s| s.border_color(config.color(LapceColor::EDITOR_FOCUS)))
            .focus_visible(|s| {
                s.border_color(config.color(LapceColor::EDITOR_FOCUS))
            })
    })
}

/// The files the agent changed, above the prompt box: a highlighted box with
/// one row per file (a click opens its diff in a tab), then the Approve and
/// Revert buttons. Hidden while the agent changed nothing.
fn changes_bar(window_tab_data: Rc<WindowTabData>, agent: AgentData) -> impl View {
    let config = window_tab_data.common.config;
    let root = window_tab_data.common.workspace.path.clone();
    let revision = agent.memos.changes_revision;
    let state = agent.state;
    let rows = dyn_stack(
        move || {
            revision.track();
            state.with_untracked(|state| {
                state
                    .changes()
                    .files()
                    .iter()
                    .map(|file| {
                        change_row(&file.path, root.as_deref(), file.line_counts())
                    })
                    .collect::<Vec<_>>()
            })
        },
        |row| row.clone(),
        {
            let agent = agent.clone();
            let window_tab_data = window_tab_data.clone();
            move |row| change_row_view(window_tab_data.clone(), agent.clone(), row)
        },
    )
    .style(|s| s.flex_col().width_pct(100.0));
    let buttons = stack((
        review_button(
            window_tab_data.clone(),
            "Approve changes",
            LapceColor::LAPCE_BUTTON_PRIMARY_BACKGROUND,
            true,
            {
                let agent = agent.clone();
                move || agent.approve_changes()
            },
        ),
        review_button(
            window_tab_data.clone(),
            "Revert changes",
            LapceColor::LAPCE_ERROR,
            false,
            {
                let agent = agent.clone();
                move || agent.revert_changes()
            },
        ),
    ))
    .style(|s| s.padding(PERMISSION_BAR_PADDING));
    stack((
        label(move || {
            revision.track();
            let count = state.with_untracked(|state| state.changes().files().len());
            format!("Changed files ({count})")
        })
        .style(move |s| {
            s.font_bold()
                .font_size(CHANGES_TITLE_FONT_SIZE)
                .padding_horiz(CHANGES_ROW_PADDING.0)
                .padding_vert(CHANGES_TITLE_PADDING_VERT)
                .color(config.get().color(LapceColor::SOURCE_CONTROL_MODIFIED))
        }),
        scroll(rows)
            .style(|s| s.max_height(CHANGES_LIST_MAX_HEIGHT).width_pct(100.0)),
        buttons,
    ))
    .style(move |s| {
        let config = config.get();
        s.flex_col()
            .width_pct(100.0)
            .border_top(2.0)
            .border_color(config.color(LapceColor::SOURCE_CONTROL_MODIFIED))
            .background(config.color(LapceColor::EDITOR_BACKGROUND))
            .apply_if(
                {
                    revision.track();
                    state.with_untracked(|state| state.changes().is_empty())
                },
                |s| s.hide(),
            )
    })
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

    // The transcript follows the new content unless the user scrolled up.
    let follow = create_rw_signal(true);
    let scroll_to_end = create_rw_signal(0u64);
    let content_height = create_rw_signal(0.0f64);
    let last_top = create_rw_signal(0.0f64);
    // A new prompt brings the transcript back to the end.
    create_effect({
        let busy = agent.memos.busy;
        move |_| {
            if busy.get() {
                follow.set(true);
                scroll_to_end.update(|ticks| *ticks += 1);
            }
        }
    });
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
        .on_resize(move |rect| {
            content_height.set(rect.height());
            if follow.get_untracked() {
                scroll_to_end.update(|ticks| *ticks += 1);
            }
        })
        .style(|s| s.flex_col().width_pct(100.0)),
    )
    .on_scroll(move |rect| {
        let previous_top = last_top.get_untracked();
        last_top.set(rect.y0);
        follow.set(follow_after_scroll(
            follow.get_untracked(),
            previous_top,
            rect.y0,
            rect.y1,
            content_height.get_untracked(),
        ));
    })
    .scroll_to_percent(move || {
        scroll_to_end.track();
        100.0
    })
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
                changes_bar(window_tab_data.clone(), agent.clone()),
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

    use std::path::Path;

    use super::{change_row, follow_after_scroll, is_activation_key, working_text};

    #[test]
    fn a_change_row_splits_the_name_from_the_relative_folder() {
        let root = Path::new("/w");
        let row = change_row(Path::new("/w/src/a.rs"), Some(root), Some((3, 1)));
        assert_eq!((row.name.as_str(), row.folder.as_str()), ("a.rs", "src"));
        assert_eq!(row.counts, Some((3, 1)));
        let root_file = change_row(Path::new("/w/b.rs"), Some(root), None);
        assert_eq!(root_file.folder, "");
        let outside = change_row(Path::new("/other/c.rs"), Some(root), None);
        assert_eq!(outside.folder, "/other");
    }

    #[test]
    fn the_transcript_follows_until_the_user_scrolls_up() {
        // At the end: follows.
        assert!(follow_after_scroll(false, 0.0, 500.0, 900.0, 900.0));
        // Scrolled up from the end: stops following.
        assert!(!follow_after_scroll(true, 500.0, 300.0, 700.0, 900.0));
        // The content grew under a viewport that did not move: unchanged.
        assert!(follow_after_scroll(true, 500.0, 500.0, 900.0, 1200.0));
        assert!(!follow_after_scroll(false, 300.0, 300.0, 700.0, 1200.0));
        // Scrolling back down to the end follows again.
        assert!(follow_after_scroll(false, 300.0, 800.0, 1190.0, 1200.0));
    }

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
