//! Views of the command palette: input, item list and preview.

use std::{ops::Range, rc::Rc, sync::Arc};

use floem::{
    View,
    event::EventListener,
    peniko::kurbo::{Point, Rect, Size},
    reactive::{ReadSignal, SignalGet, SignalUpdate, SignalWith, create_rw_signal},
    style::{AlignItems, CursorStyle, Display, Position, Style},
    unit::PxPctAuto,
    views::{
        Decorators, VirtualVector, container, dyn_stack, label,
        scroll::{PropagatePointerWheel, scroll},
        stack, svg, text, virtual_stack,
    },
};

use crate::{
    command::{CommandKind, LapceCommand},
    config::{LapceConfig, color::LapceColor, icon::LapceIcons},
    debug::RunDebugMode,
    editor::view::editor_container_view,
    focus_text::focus_text,
    keypress::keymap::KeyMap,
    palette::{
        PaletteStatus,
        item::{PaletteItem, PaletteItemContent},
    },
    text_input::TextInputBuilder,
    window_tab::{Focus, WindowTabData},
    workspace::LapceWorkspace,
};

fn palette_item(
    workspace: Arc<LapceWorkspace>,
    i: usize,
    item: PaletteItem,
    index: ReadSignal<usize>,
    palette_item_height: f64,
    config: ReadSignal<Arc<LapceConfig>>,
    keymap: Option<&KeyMap>,
) -> impl View + use<> {
    match &item.content {
        PaletteItemContent::File { path, .. }
        | PaletteItemContent::Reference { path, .. } => {
            let file_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            // let (file_name, _) = create_signal(cx.scope, file_name);
            let folder = path
                .parent()
                .unwrap_or("".as_ref())
                .to_string_lossy()
                .into_owned();
            // let (folder, _) = create_signal(cx.scope, folder);
            let folder_len = folder.len();

            let file_name_indices = item
                .indices
                .iter()
                .filter_map(|&i| {
                    if folder_len > 0 {
                        if i > folder_len {
                            Some(i - folder_len - 1)
                        } else {
                            None
                        }
                    } else {
                        Some(i)
                    }
                })
                .collect::<Vec<_>>();
            let folder_indices = item
                .indices
                .iter()
                .filter_map(|&i| if i < folder_len { Some(i) } else { None })
                .collect::<Vec<_>>();

            let path = path.to_path_buf();
            let style_path = path.clone();
            container(
                stack((
                    svg(move || config.get().file_svg(&path).0).style(move |s| {
                        let config = config.get();
                        let size = config.ui.icon_size() as f32;
                        let color = config.file_svg(&style_path).1;
                        s.min_width(size)
                            .size(size, size)
                            .margin_right(5.0)
                            .apply_opt(color, Style::color)
                    }),
                    focus_text(
                        move || file_name.clone(),
                        move || file_name_indices.clone(),
                        move || config.get().color(LapceColor::EDITOR_FOCUS),
                    )
                    .style(|s| s.margin_right(6.0).max_width_full()),
                    focus_text(
                        move || folder.clone(),
                        move || folder_indices.clone(),
                        move || config.get().color(LapceColor::EDITOR_FOCUS),
                    )
                    .style(move |s| {
                        s.color(config.get().color(LapceColor::EDITOR_DIM))
                            .min_width(0.0)
                            .flex_grow(1.0f32)
                            .flex_basis(0.0)
                    }),
                ))
                .style(|s| s.align_items(Some(AlignItems::Center)).max_width_full()),
            )
        }
        PaletteItemContent::DocumentSymbol {
            kind,
            name,
            container_name,
            ..
        } => {
            let kind = *kind;
            let text = name.to_string();
            let hint = container_name.clone().unwrap_or_default();
            let text_indices: Vec<usize> = item
                .indices
                .iter()
                .filter_map(|i| {
                    let i = *i;
                    if i < text.len() { Some(i) } else { None }
                })
                .collect();
            let hint_indices: Vec<usize> = item
                .indices
                .iter()
                .filter_map(|i| {
                    let i = *i;
                    if i >= text.len() {
                        Some(i - text.len())
                    } else {
                        None
                    }
                })
                .collect();
            container(
                stack((
                    svg(move || {
                        let config = config.get();
                        config
                            .symbol_svg(&kind)
                            .unwrap_or_else(|| config.ui_svg(LapceIcons::FILE))
                    })
                    .style(move |s| {
                        let config = config.get();
                        let size = config.ui.icon_size() as f32;
                        s.min_width(size).size(size, size).margin_right(5.0).color(
                            config.symbol_color(&kind).unwrap_or_else(|| {
                                config.color(LapceColor::LAPCE_ICON_ACTIVE)
                            }),
                        )
                    }),
                    focus_text(
                        move || text.clone(),
                        move || text_indices.clone(),
                        move || config.get().color(LapceColor::EDITOR_FOCUS),
                    )
                    .style(|s| s.margin_right(6.0).max_width_full()),
                    focus_text(
                        move || hint.clone(),
                        move || hint_indices.clone(),
                        move || config.get().color(LapceColor::EDITOR_FOCUS),
                    )
                    .style(move |s| {
                        s.color(config.get().color(LapceColor::EDITOR_DIM))
                            .min_width(0.0)
                            .flex_grow(1.0f32)
                            .flex_basis(0.0)
                    }),
                ))
                .style(|s| s.align_items(Some(AlignItems::Center)).max_width_full()),
            )
        }
        PaletteItemContent::WorkspaceSymbol {
            kind,
            name,
            location,
            ..
        } => {
            let text = name.to_string();
            let kind = *kind;

            let path = location.path.clone();
            let full_path = location.path.clone();
            let path = if let Some(workspace_path) = workspace.path.as_ref() {
                path.strip_prefix(workspace_path)
                    .unwrap_or(&full_path)
                    .to_path_buf()
            } else {
                path
            };

            let hint = path.to_string_lossy().to_string();
            let text_indices: Vec<usize> = item
                .indices
                .iter()
                .filter_map(|i| {
                    let i = *i;
                    if i < text.len() { Some(i) } else { None }
                })
                .collect();
            let hint_indices: Vec<usize> = item
                .indices
                .iter()
                .filter_map(|i| {
                    let i = *i;
                    if i >= text.len() {
                        Some(i - text.len())
                    } else {
                        None
                    }
                })
                .collect();
            container(
                stack((
                    svg(move || {
                        let config = config.get();
                        config
                            .symbol_svg(&kind)
                            .unwrap_or_else(|| config.ui_svg(LapceIcons::FILE))
                    })
                    .style(move |s| {
                        let config = config.get();
                        let size = config.ui.icon_size() as f32;
                        s.min_width(size)
                            .size(size, size)
                            .margin_right(5.0)
                            .color(config.color(LapceColor::LAPCE_ICON_ACTIVE))
                    }),
                    focus_text(
                        move || text.clone(),
                        move || text_indices.clone(),
                        move || config.get().color(LapceColor::EDITOR_FOCUS),
                    )
                    .style(|s| s.margin_right(6.0).max_width_full()),
                    focus_text(
                        move || hint.clone(),
                        move || hint_indices.clone(),
                        move || config.get().color(LapceColor::EDITOR_FOCUS),
                    )
                    .style(move |s| {
                        s.color(config.get().color(LapceColor::EDITOR_DIM))
                            .min_width(0.0)
                            .flex_grow(1.0f32)
                            .flex_basis(0.0)
                    }),
                ))
                .style(|s| s.align_items(Some(AlignItems::Center)).max_width_full()),
            )
        }
        PaletteItemContent::RunAndDebug {
            mode,
            config: run_config,
        } => {
            let mode = *mode;
            let text = format!("{mode} {}", run_config.name);
            let hint = format!(
                "{} {}",
                run_config.program,
                run_config.args.clone().unwrap_or_default().join(" ")
            );
            let text_indices: Vec<usize> = item
                .indices
                .iter()
                .filter_map(|i| {
                    let i = *i;
                    if i < text.len() { Some(i) } else { None }
                })
                .collect();
            let hint_indices: Vec<usize> = item
                .indices
                .iter()
                .filter_map(|i| {
                    let i = *i;
                    if i >= text.len() {
                        Some(i - text.len())
                    } else {
                        None
                    }
                })
                .collect();
            container(
                stack((
                    svg(move || {
                        let config = config.get();
                        match mode {
                            RunDebugMode::Run => config.ui_svg(LapceIcons::START),
                            RunDebugMode::Debug => config.ui_svg(LapceIcons::DEBUG),
                        }
                    })
                    .style(move |s| {
                        let config = config.get();
                        let size = config.ui.icon_size() as f32;
                        s.min_width(size)
                            .size(size, size)
                            .margin_right(5.0)
                            .color(config.color(LapceColor::LAPCE_ICON_ACTIVE))
                    }),
                    focus_text(
                        move || text.clone(),
                        move || text_indices.clone(),
                        move || config.get().color(LapceColor::EDITOR_FOCUS),
                    )
                    .style(|s| s.margin_right(6.0).max_width_full()),
                    focus_text(
                        move || hint.clone(),
                        move || hint_indices.clone(),
                        move || config.get().color(LapceColor::EDITOR_FOCUS),
                    )
                    .style(move |s| {
                        s.color(config.get().color(LapceColor::EDITOR_DIM))
                            .min_width(0.0)
                            .flex_grow(1.0f32)
                            .flex_basis(0.0)
                    }),
                ))
                .style(|s| s.align_items(Some(AlignItems::Center)).max_width_full()),
            )
        }
        PaletteItemContent::PaletteHelp { .. }
        | PaletteItemContent::Command { .. } => {
            let text = item.filter_text;
            let indices = item.indices;
            let keys = if let Some(keymap) = keymap {
                keymap
                    .key
                    .iter()
                    .map(|key| key.label().trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            } else {
                vec![]
            };
            container(
                stack((
                    focus_text(
                        move || text.clone(),
                        move || indices.clone(),
                        move || config.get().color(LapceColor::EDITOR_FOCUS),
                    )
                    .style(|s| {
                        s.flex_row()
                            .flex_grow(1.0f32)
                            .align_items(Some(AlignItems::Center))
                    }),
                    stack((dyn_stack(
                        move || keys.clone(),
                        |k| k.clone(),
                        move |key| {
                            label(move || key.clone()).style(move |s| {
                                s.padding_horiz(5.0)
                                    .padding_vert(1.0)
                                    .margin_right(5.0)
                                    .border(1.0)
                                    .border_radius(3.0)
                                    .border_color(
                                        config.get().color(LapceColor::LAPCE_BORDER),
                                    )
                                    .selectable(false)
                            })
                        },
                    ),)),
                ))
                .style(|s| s.width_full().items_center()),
            )
        }
        PaletteItemContent::Line { .. }
        | PaletteItemContent::Workspace { .. }
        | PaletteItemContent::SshHost { .. }
        | PaletteItemContent::Language { .. }
        | PaletteItemContent::LineEnding { .. }
        | PaletteItemContent::ColorTheme { .. }
        | PaletteItemContent::SCMReference { .. }
        | PaletteItemContent::TerminalProfile { .. }
        | PaletteItemContent::IconTheme { .. } => {
            let text = item.filter_text;
            let indices = item.indices;
            container(
                focus_text(
                    move || text.clone(),
                    move || indices.clone(),
                    move || config.get().color(LapceColor::EDITOR_FOCUS),
                )
                .style(|s| s.align_items(Some(AlignItems::Center)).max_width_full()),
            )
        }
        #[cfg(windows)]
        PaletteItemContent::WslHost { .. } => {
            let text = item.filter_text;
            let indices = item.indices;
            container(
                focus_text(
                    move || text.clone(),
                    move || indices.clone(),
                    move || config.get().color(LapceColor::EDITOR_FOCUS),
                )
                .style(|s| s.align_items(Some(AlignItems::Center)).max_width_full()),
            )
        }
    }
    .style(move |s| {
        s.width_full()
            .height(palette_item_height as f32)
            .padding_horiz(10.0)
            .apply_if(index.get() == i, |style| {
                style.background(
                    config.get().color(LapceColor::PALETTE_CURRENT_BACKGROUND),
                )
            })
    })
}

fn palette_input(window_tab_data: Rc<WindowTabData>) -> impl View {
    let editor = window_tab_data.palette.input_editor.clone();
    let config = window_tab_data.common.config;
    let focus = window_tab_data.common.focus;
    let is_focused = move || focus.get() == Focus::Palette;

    let input = TextInputBuilder::new()
        .is_focused(is_focused)
        .build_editor(editor)
        .placeholder(move || window_tab_data.palette.placeholder_text().to_owned())
        .style(|s| s.width_full());

    container(container(input).style(move |s| {
        let config = config.get();
        s.width_full()
            .height(25.0)
            .items_center()
            .border_bottom(1.0)
            .border_color(config.color(LapceColor::LAPCE_BORDER))
            .background(config.color(LapceColor::EDITOR_BACKGROUND))
    }))
    .style(|s| s.padding_bottom(5.0))
}

struct PaletteItems(im::Vector<PaletteItem>);

impl VirtualVector<(usize, PaletteItem)> for PaletteItems {
    fn total_len(&self) -> usize {
        self.0.len()
    }

    fn slice(
        &mut self,
        range: Range<usize>,
    ) -> impl Iterator<Item = (usize, PaletteItem)> {
        let start = range.start;
        Box::new(
            self.0
                .slice(range)
                .into_iter()
                .enumerate()
                .map(move |(i, item)| (i + start, item)),
        )
    }
}

fn palette_content(
    window_tab_data: Rc<WindowTabData>,
    layout_rect: ReadSignal<Rect>,
) -> impl View {
    let items = window_tab_data.palette.filtered_items;
    let keymaps = window_tab_data
        .palette
        .keypress
        .get_untracked()
        .command_keymaps;
    let index = window_tab_data.palette.index.read_only();
    let clicked_index = window_tab_data.palette.clicked_index.write_only();
    let config = window_tab_data.common.config;
    let run_id = window_tab_data.palette.run_id;
    let input = window_tab_data.palette.input.read_only();
    let palette_item_height = 25.0;
    let workspace = window_tab_data.workspace.clone();
    stack((
        scroll({
            let workspace = workspace.clone();
            virtual_stack(
                move || PaletteItems(items.get()),
                move |(i, _item)| {
                    (run_id.get_untracked(), *i, input.get_untracked().input)
                },
                move |(i, item)| {
                    let workspace = workspace.clone();
                    let keymap = {
                        let cmd_kind = match &item.content {
                            PaletteItemContent::PaletteHelp { cmd } => {
                                Some(CommandKind::Workbench(cmd.clone()))
                            }
                            PaletteItemContent::Command {
                                cmd: LapceCommand { kind, .. },
                            } => Some(kind.clone()),
                            _ => None,
                        };

                        cmd_kind
                            .and_then(|kind| keymaps.get(kind.str()))
                            .and_then(|maps| maps.first())
                    };
                    container(palette_item(
                        workspace,
                        i,
                        item,
                        index,
                        palette_item_height,
                        config,
                        keymap,
                    ))
                    .on_click_stop(move |_| {
                        clicked_index.set(Some(i));
                    })
                    .style(move |s| {
                        s.width_full().cursor(CursorStyle::Pointer).hover(|s| {
                            s.background(
                                config
                                    .get()
                                    .color(LapceColor::PANEL_HOVERED_BACKGROUND),
                            )
                        })
                    })
                },
            )
            .item_size_fixed(move || palette_item_height)
            .style(|s| s.width_full().flex_col())
        })
        .ensure_visible(move || {
            Size::new(1.0, palette_item_height)
                .to_rect()
                .with_origin(Point::new(
                    0.0,
                    index.get() as f64 * palette_item_height,
                ))
        })
        .style(|s| {
            s.width_full()
                .min_height(0.0)
                .set(PropagatePointerWheel, false)
        }),
        text("No matching results").style(move |s| {
            s.display(if items.with(|items| items.is_empty()) {
                Display::Flex
            } else {
                Display::None
            })
            .padding_horiz(10.0)
            .align_items(Some(AlignItems::Center))
            .height(palette_item_height as f32)
        }),
    ))
    .style(move |s| {
        s.flex_col()
            .width_full()
            .min_height(0.0)
            .max_height((layout_rect.get().height() * 0.45 - 36.0).round() as f32)
            .padding_bottom(5.0)
            .padding_bottom(5.0)
    })
}

fn palette_preview(window_tab_data: Rc<WindowTabData>) -> impl View {
    let palette_data = window_tab_data.palette.clone();
    let workspace = palette_data.workspace.clone();
    let preview_editor = palette_data.preview_editor;
    let has_preview = palette_data.has_preview;
    let config = palette_data.common.config;
    let preview_editor = create_rw_signal(preview_editor);
    container(
        container(editor_container_view(
            window_tab_data,
            workspace,
            |_tracked: bool| true,
            preview_editor,
        ))
        .style(move |s| {
            let config = config.get();
            s.position(Position::Absolute)
                .border_top(1.0)
                .border_color(config.color(LapceColor::LAPCE_BORDER))
                .size_full()
                .background(config.color(LapceColor::EDITOR_BACKGROUND))
        }),
    )
    .style(move |s| {
        s.display(if has_preview.get() {
            Display::Flex
        } else {
            Display::None
        })
        .flex_grow(1.0f32)
    })
}

pub(super) fn palette(window_tab_data: Rc<WindowTabData>) -> impl View {
    let layout_rect = window_tab_data.layout_rect.read_only();
    let palette_data = window_tab_data.palette.clone();
    let status = palette_data.status.read_only();
    let config = palette_data.common.config;
    let has_preview = palette_data.has_preview.read_only();
    container(
        stack((
            palette_input(window_tab_data.clone()),
            palette_content(window_tab_data.clone(), layout_rect),
            palette_preview(window_tab_data.clone()),
        ))
        .on_event_stop(EventListener::PointerDown, move |_| {})
        .style(move |s| {
            let config = config.get();
            s.width(config.ui.palette_width() as f64)
                .max_width_full()
                .max_height(if has_preview.get() {
                    PxPctAuto::Auto
                } else {
                    PxPctAuto::Pct(100.0)
                })
                .height(if has_preview.get() {
                    PxPctAuto::Px(layout_rect.get().height() - 10.0)
                } else {
                    PxPctAuto::Auto
                })
                .margin_top(4.0)
                .border(1.0)
                .border_radius(6.0)
                .border_color(config.color(LapceColor::LAPCE_BORDER))
                .flex_col()
                .background(config.color(LapceColor::PALETTE_BACKGROUND))
                .pointer_events_auto()
        }),
    )
    .style(move |s| {
        s.display(if status.get() == PaletteStatus::Inactive {
            Display::None
        } else {
            Display::Flex
        })
        .position(Position::Absolute)
        .size_full()
        .flex_col()
        .items_center()
        .pointer_events_none()
    })
    .debug_name("Palette Layer")
}
