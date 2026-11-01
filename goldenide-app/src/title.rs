use std::{rc::Rc, sync::Arc};

use floem::{
    View,
    event::EventListener,
    menu::{Menu, MenuItem},
    peniko::Color,
    reactive::{Memo, ReadSignal, RwSignal, SignalGet, SignalUpdate, create_memo},
    style::{AlignItems, CursorStyle, JustifyContent},
    views::{Decorators, container, drag_window_area, empty, label, stack, svg},
};
use goldenide_rpc::proxy::ProxyStatus;

use crate::{
    app::{clickable_icon, not_clickable_icon, tooltip_label, window_menu},
    command::{GoldenIdeCommand, GoldenIdeWorkbenchCommand, WindowCommand},
    config::{GoldenIdeConfig, color::GoldenIdeColor, icon::GoldenIdeIcons},
    listener::Listener,
    main_split::MainSplitData,
    window_tab::WindowTabData,
    workspace::GoldenIdeWorkspace,
};

fn left(
    workspace: Arc<GoldenIdeWorkspace>,
    goldenide_command: Listener<GoldenIdeCommand>,
    workbench_command: Listener<GoldenIdeWorkbenchCommand>,
    config: ReadSignal<Arc<GoldenIdeConfig>>,
    proxy_status: RwSignal<Option<ProxyStatus>>,
    num_window_tabs: Memo<usize>,
) -> impl View {
    let is_local = workspace.kind.is_local();
    let is_macos = cfg!(target_os = "macos");
    stack((
        empty().style(move |s| {
            let should_hide = if is_macos {
                num_window_tabs.get() > 1
            } else {
                true
            };
            s.width(75.0).apply_if(should_hide, |s| s.hide())
        }),
        container(
            svg(move || config.get().ui_svg(GoldenIdeIcons::LOGO)).style(move |s| {
                let config = config.get();
                s.size(16.0, 16.0)
                    .color(config.color(GoldenIdeColor::GOLDENIDE_ICON_ACTIVE))
            }),
        )
        .style(move |s| s.margin_horiz(10.0).apply_if(is_macos, |s| s.hide())),
        not_clickable_icon(
            || GoldenIdeIcons::MENU,
            || false,
            || false,
            || "Menu",
            config,
        )
        .popout_menu(move || window_menu(goldenide_command, workbench_command))
        .style(move |s| {
            s.margin_left(4.0)
                .margin_right(6.0)
                .apply_if(is_macos, |s| s.hide())
        }),
        tooltip_label(
            config,
            container(
                svg(move || config.get().ui_svg(GoldenIdeIcons::REMOTE)).style(
                    move |s| {
                        let config = config.get();
                        let size = (config.ui.icon_size() as f32 + 2.0).min(30.0);
                        s.size(size, size).color(if is_local {
                            config.color(GoldenIdeColor::GOLDENIDE_ICON_ACTIVE)
                        } else {
                            match proxy_status.get() {
                                Some(_) => Color::WHITE,
                                None => config
                                    .color(GoldenIdeColor::GOLDENIDE_ICON_ACTIVE),
                            }
                        })
                    },
                ),
            ),
            || "Connect to Remote",
        )
        .popout_menu(move || {
            #[allow(unused_mut)]
            let mut menu = Menu::new("").entry(
                MenuItem::new("Connect to SSH Host").action(move || {
                    workbench_command
                        .send(GoldenIdeWorkbenchCommand::ConnectSshHost);
                }),
            );
            if !is_local
                && proxy_status.get().is_some_and(|p| {
                    matches!(p, ProxyStatus::Connecting | ProxyStatus::Connected)
                })
            {
                menu = menu.entry(MenuItem::new("Disconnect remote").action(
                    move || {
                        workbench_command
                            .send(GoldenIdeWorkbenchCommand::DisconnectRemote);
                    },
                ));
            }
            #[cfg(windows)]
            {
                menu = menu.entry(MenuItem::new("Connect to WSL Host").action(
                    move || {
                        workbench_command
                            .send(GoldenIdeWorkbenchCommand::ConnectWslHost);
                    },
                ));
            }
            menu
        })
        .style(move |s| {
            let config = config.get();
            let color = if is_local {
                Color::TRANSPARENT
            } else {
                match proxy_status.get() {
                    Some(ProxyStatus::Connected) => {
                        config.color(GoldenIdeColor::GOLDENIDE_REMOTE_CONNECTED)
                    }
                    Some(ProxyStatus::Connecting) => {
                        config.color(GoldenIdeColor::GOLDENIDE_REMOTE_CONNECTING)
                    }
                    Some(ProxyStatus::Disconnected) => {
                        config.color(GoldenIdeColor::GOLDENIDE_REMOTE_DISCONNECTED)
                    }
                    None => Color::TRANSPARENT,
                }
            };
            s.height_pct(100.0)
                .padding_horiz(10.0)
                .items_center()
                .background(color)
                .hover(|s| {
                    s.cursor(CursorStyle::Pointer).background(
                        config.color(GoldenIdeColor::PANEL_HOVERED_BACKGROUND),
                    )
                })
                .active(|s| {
                    s.cursor(CursorStyle::Pointer).background(
                        config
                            .color(GoldenIdeColor::PANEL_HOVERED_ACTIVE_BACKGROUND),
                    )
                })
        }),
        drag_window_area(empty())
            .style(|s| s.height_pct(100.0).flex_basis(0.0).flex_grow(1.0f32)),
    ))
    .style(move |s| {
        s.height_pct(100.0)
            .flex_basis(0.0)
            .flex_grow(1.0f32)
            .items_center()
    })
    .debug_name("Left Side of Top Bar")
}

fn middle(
    workspace: Arc<GoldenIdeWorkspace>,
    main_split: MainSplitData,
    workbench_command: Listener<GoldenIdeWorkbenchCommand>,
    config: ReadSignal<Arc<GoldenIdeConfig>>,
) -> impl View {
    let local_workspace = workspace.clone();
    let can_jump_backward = {
        let main_split = main_split.clone();
        create_memo(move |_| main_split.can_jump_location_backward(true))
    };
    let can_jump_forward =
        create_memo(move |_| main_split.can_jump_location_forward(true));

    let jump_backward = move || {
        clickable_icon(
            || GoldenIdeIcons::LOCATION_BACKWARD,
            move || {
                workbench_command
                    .send(GoldenIdeWorkbenchCommand::JumpLocationBackward);
            },
            || false,
            move || !can_jump_backward.get(),
            || "Jump Backward",
            config,
        )
        .style(move |s| s.margin_horiz(6.0))
    };
    let jump_forward = move || {
        clickable_icon(
            || GoldenIdeIcons::LOCATION_FORWARD,
            move || {
                workbench_command
                    .send(GoldenIdeWorkbenchCommand::JumpLocationForward);
            },
            || false,
            move || !can_jump_forward.get(),
            || "Jump Forward",
            config,
        )
        .style(move |s| s.margin_right(6.0))
    };

    let open_folder = move || {
        not_clickable_icon(
            || GoldenIdeIcons::PALETTE_MENU,
            || false,
            || false,
            || "Open Folder / Recent Workspace",
            config,
        )
        .popout_menu(move || {
            Menu::new("")
                .entry(MenuItem::new("Open Folder").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::OpenFolder);
                }))
                .entry(MenuItem::new("Open Recent Workspace").action(move || {
                    workbench_command
                        .send(GoldenIdeWorkbenchCommand::PaletteWorkspace);
                }))
        })
    };

    stack((
        stack((
            drag_window_area(empty())
                .style(|s| s.height_pct(100.0).flex_basis(0.0).flex_grow(1.0f32)),
            jump_backward(),
            jump_forward(),
        ))
        .style(|s| {
            s.flex_basis(0)
                .flex_grow(1.0f32)
                .justify_content(Some(JustifyContent::FlexEnd))
        }),
        container(
            stack((
                svg(move || config.get().ui_svg(GoldenIdeIcons::SEARCH)).style(
                    move |s| {
                        let config = config.get();
                        let icon_size = config.ui.icon_size() as f32;
                        s.size(icon_size, icon_size).color(
                            config.color(GoldenIdeColor::GOLDENIDE_ICON_ACTIVE),
                        )
                    },
                ),
                label(move || {
                    if let Some(s) = local_workspace.display() {
                        s
                    } else {
                        "Open Folder".to_string()
                    }
                })
                .style(|s| s.padding_left(10).padding_right(5).selectable(false)),
                open_folder(),
            ))
            .style(|s| s.align_items(Some(AlignItems::Center))),
        )
        .on_event_stop(EventListener::PointerDown, |_| {})
        .on_click_stop(move |_| {
            if workspace.clone().path.is_some() {
                workbench_command
                    .send(GoldenIdeWorkbenchCommand::PaletteHelpAndFile);
            } else {
                workbench_command.send(GoldenIdeWorkbenchCommand::PaletteWorkspace);
            }
        })
        .style(move |s| {
            let config = config.get();
            s.flex_basis(0)
                .flex_grow(10.0f32)
                .min_width(200.0)
                .max_width(500.0)
                .height(26.0)
                .justify_content(Some(JustifyContent::Center))
                .align_items(Some(AlignItems::Center))
                .border(1.0)
                .border_color(config.color(GoldenIdeColor::GOLDENIDE_BORDER))
                .border_radius(6.0)
                .background(config.color(GoldenIdeColor::EDITOR_BACKGROUND))
        }),
        stack((
            clickable_icon(
                || GoldenIdeIcons::START,
                move || {
                    workbench_command
                        .send(GoldenIdeWorkbenchCommand::PaletteRunAndDebug)
                },
                || false,
                || false,
                || "Run and Debug",
                config,
            )
            .style(move |s| s.margin_horiz(6.0)),
            drag_window_area(empty())
                .style(|s| s.height_pct(100.0).flex_basis(0.0).flex_grow(1.0f32)),
        ))
        .style(move |s| {
            s.flex_basis(0)
                .flex_grow(1.0f32)
                .justify_content(Some(JustifyContent::FlexStart))
        }),
    ))
    .style(|s| {
        s.flex_basis(0)
            .flex_grow(2.0f32)
            .align_items(Some(AlignItems::Center))
            .justify_content(Some(JustifyContent::Center))
    })
    .debug_name("Middle of Top Bar")
}

fn right(
    window_command: Listener<WindowCommand>,
    workbench_command: Listener<GoldenIdeWorkbenchCommand>,
    num_window_tabs: Memo<usize>,
    window_maximized: RwSignal<bool>,
    config: ReadSignal<Arc<GoldenIdeConfig>>,
) -> impl View {
    stack((
        drag_window_area(empty())
            .style(|s| s.height_pct(100.0).flex_basis(0.0).flex_grow(1.0f32)),
        stack((not_clickable_icon(
            || GoldenIdeIcons::SETTINGS,
            || false,
            || false,
            || "Settings",
            config,
        )
        .popout_menu(move || {
            let menu = Menu::new("")
                .entry(MenuItem::new("Command Palette").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::PaletteCommand)
                }))
                .separator()
                .entry(MenuItem::new("Open Settings").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::OpenSettings)
                }))
                .entry(MenuItem::new("Open Keyboard Shortcuts").action(move || {
                    workbench_command
                        .send(GoldenIdeWorkbenchCommand::OpenKeyboardShortcuts)
                }))
                .entry(MenuItem::new("Open Theme Color Settings").action(
                    move || {
                        workbench_command
                            .send(GoldenIdeWorkbenchCommand::OpenThemeColorSettings)
                    },
                ))
                .separator();
            menu.entry(MenuItem::new("About GoldenIDE").action(move || {
                workbench_command.send(GoldenIdeWorkbenchCommand::ShowAbout)
            }))
        }),))
        .style(move |s| s.margin_horiz(6.0)),
        window_controls_view(
            window_command,
            true,
            num_window_tabs,
            window_maximized,
            config,
        ),
    ))
    .style(|s| {
        s.flex_basis(0)
            .flex_grow(1.0f32)
            .justify_content(Some(JustifyContent::FlexEnd))
    })
    .debug_name("Right of top bar")
}

pub fn title(window_tab_data: Rc<WindowTabData>) -> impl View {
    let workspace = window_tab_data.workspace.clone();
    let goldenide_command = window_tab_data.common.goldenide_command;
    let workbench_command = window_tab_data.common.workbench_command;
    let window_command = window_tab_data.common.window_common.window_command;
    let proxy_status = window_tab_data.common.proxy_status;
    let num_window_tabs = window_tab_data.common.window_common.num_window_tabs;
    let window_maximized = window_tab_data.common.window_common.window_maximized;
    let title_height = window_tab_data.title_height;
    let config = window_tab_data.common.config;
    stack((
        left(
            workspace.clone(),
            goldenide_command,
            workbench_command,
            config,
            proxy_status,
            num_window_tabs,
        ),
        middle(
            workspace,
            window_tab_data.main_split.clone(),
            workbench_command,
            config,
        ),
        right(
            window_command,
            workbench_command,
            num_window_tabs,
            window_maximized,
            config,
        ),
    ))
    .on_resize(move |rect| {
        let height = rect.height();
        if height != title_height.get_untracked() {
            title_height.set(height);
        }
    })
    .style(move |s| {
        let config = config.get();
        s.width_pct(100.0)
            .height(37.0)
            .items_center()
            .background(config.color(GoldenIdeColor::PANEL_BACKGROUND))
            .border_bottom(1.0)
            .border_color(config.color(GoldenIdeColor::GOLDENIDE_BORDER))
    })
    .debug_name("Title / Top Bar")
}

pub fn window_controls_view(
    window_command: Listener<WindowCommand>,
    is_title: bool,
    num_window_tabs: Memo<usize>,
    window_maximized: RwSignal<bool>,
    config: ReadSignal<Arc<GoldenIdeConfig>>,
) -> impl View {
    stack((
        clickable_icon(
            || GoldenIdeIcons::WINDOW_MINIMIZE,
            || {
                floem::action::minimize_window();
            },
            || false,
            || false,
            || "Minimize",
            config,
        )
        .style(|s| s.margin_right(16.0).margin_left(10.0)),
        clickable_icon(
            move || {
                if window_maximized.get() {
                    GoldenIdeIcons::WINDOW_RESTORE
                } else {
                    GoldenIdeIcons::WINDOW_MAXIMIZE
                }
            },
            move || {
                floem::action::set_window_maximized(
                    !window_maximized.get_untracked(),
                );
            },
            || false,
            || false,
            || "Maximize",
            config,
        )
        .style(|s| s.margin_right(16.0)),
        clickable_icon(
            || GoldenIdeIcons::WINDOW_CLOSE,
            move || {
                window_command.send(WindowCommand::CloseWindow);
            },
            || false,
            || false,
            || "Close Window",
            config,
        )
        .style(|s| s.margin_right(6.0)),
    ))
    .style(move |s| {
        s.apply_if(
            cfg!(target_os = "macos")
                || !config.get_untracked().core.custom_titlebar
                || (is_title && num_window_tabs.get() > 1),
            |s| s.hide(),
        )
    })
}
