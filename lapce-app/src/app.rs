use std::{path::PathBuf, rc::Rc, sync::Arc};

use floem::{
    View,
    event::{Event, EventListener, EventPropagation},
    menu::Menu,
    peniko::kurbo::{Point, Size},
    prelude::SignalTrack,
    reactive::{
        RwSignal, SignalGet, SignalUpdate, SignalWith, create_memo,
        create_rw_signal, use_context,
    },
    style::Position,
    views::{
        Decorators, container, drag_resize_window_area, drag_window_area, dyn_stack,
        empty, stack, tab, text,
    },
    window::{ResizeDirection, WindowConfig, WindowId},
};
use lapce_rpc::file::PathObject;
use serde::{Deserialize, Serialize};
use tracing_subscriber::{filter::Targets, reload::Handle};

use crate::{
    about, alert,
    command::{InternalCommand, WindowCommand},
    config::{LapceConfig, color::LapceColor, icon::LapceIcons},
    db::LapceDb,
    editor::location::{EditorLocation, EditorPosition},
    listener::Listener,
    panel::{position::PanelContainerPosition, view::panel_container_view},
    status::status,
    title::{title, window_controls_view},
    tracing::*,
    update::ReleaseInfo,
    window::{TabsInfo, WindowData, WindowInfo},
    window_tab::WindowTabData,
    workspace::{LapceWorkspace, LapceWorkspaceType},
};

mod editor_tab;
mod grammars;
mod launch;
mod logging;
mod menu;
mod palette;
mod popups;
mod split;
mod widgets;

pub use self::{
    launch::{get_socket, launch, load_shell_env, try_open_in_existing_process},
    menu::window_menu,
    widgets::{
        clickable_icon, clickable_icon_base, not_clickable_icon, tooltip_label,
    },
};
use self::{
    palette::palette,
    popups::{code_action, completion, hover, rename, window_message_view},
    split::main_split,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub windows: Vec<WindowInfo>,
}

#[derive(Clone)]
pub enum AppCommand {
    SaveApp,
    NewWindow { folder: Option<PathBuf> },
    CloseWindow(WindowId),
    WindowGotFocus(WindowId),
    WindowClosed(WindowId),
}

#[derive(Clone)]
pub struct AppData {
    pub windows: RwSignal<im::HashMap<WindowId, WindowData>>,
    pub active_window: RwSignal<WindowId>,
    pub window_scale: RwSignal<f64>,
    pub app_command: Listener<AppCommand>,
    pub app_terminated: RwSignal<bool>,
    /// The latest release information
    pub latest_release: RwSignal<Arc<Option<ReleaseInfo>>>,
    pub watcher: Arc<notify::RecommendedWatcher>,
    pub tracing_handle: Handle<Targets>,
    pub config: RwSignal<Arc<LapceConfig>>,
    /// Paths to extra plugins to load
    pub plugin_paths: Arc<Vec<PathBuf>>,
}

impl AppData {
    pub fn reload_config(&self) {
        let config =
            LapceConfig::load(&LapceWorkspace::default(), &[], &self.plugin_paths);

        self.config.set(Arc::new(config));
        self.window_scale.set(self.config.get().ui.scale());

        let windows = self.windows.get_untracked();
        for (_, window) in windows {
            window.reload_config();
        }
    }

    pub fn active_window_tab(&self) -> Option<Rc<WindowTabData>> {
        if let Some(window) = self.active_window() {
            return window.active_window_tab();
        }
        None
    }

    fn active_window(&self) -> Option<WindowData> {
        let windows = self.windows.get_untracked();
        let active_window = self.active_window.get_untracked();
        windows
            .get(&active_window)
            .cloned()
            .or_else(|| windows.iter().next().map(|(_, window)| window.clone()))
    }

    fn default_window_config(&self) -> WindowConfig {
        WindowConfig::default()
            .apply_default_theme(false)
            .title("Lapce")
    }

    pub fn new_window(&self, folder: Option<PathBuf>) {
        let config = self
            .active_window()
            .map(|window| {
                self.default_window_config()
                    .size(window.common.size.get_untracked())
                    .position(window.position.get_untracked() + (50.0, 50.0))
            })
            .or_else(|| {
                let db: Arc<LapceDb> = use_context().unwrap();
                db.get_window().ok().map(|info| {
                    self.default_window_config()
                        .size(info.size)
                        .position(info.pos)
                })
            })
            .unwrap_or_else(|| {
                self.default_window_config().size(Size::new(800.0, 600.0))
            });
        let config = if cfg!(target_os = "macos")
            || self.config.get_untracked().core.custom_titlebar
        {
            config.show_titlebar(false)
        } else {
            config
        };
        let workspace = LapceWorkspace {
            path: folder,
            ..Default::default()
        };
        let app_data = self.clone();
        floem::new_window(
            move |window_id| {
                app_data.app_view(
                    window_id,
                    WindowInfo {
                        size: Size::ZERO,
                        pos: Point::ZERO,
                        maximised: false,
                        tabs: TabsInfo {
                            active_tab: 0,
                            workspaces: vec![workspace],
                        },
                    },
                    vec![],
                )
            },
            Some(config),
        );
    }

    pub fn run_app_command(&self, cmd: AppCommand) {
        match cmd {
            AppCommand::SaveApp => {
                let db: Arc<LapceDb> = use_context().unwrap();
                if let Err(err) = db.save_app(self) {
                    tracing::error!("{:?}", err);
                }
            }
            AppCommand::WindowClosed(window_id) => {
                if self.app_terminated.get_untracked() {
                    return;
                }
                if let Some(window_data) = self
                    .windows
                    .with_untracked(|windows| windows.get(&window_id).cloned())
                {
                    window_data.stop_agents();
                }
                let db: Arc<LapceDb> = use_context().unwrap();
                if self.windows.with_untracked(|w| w.len()) == 1 {
                    if let Err(err) = db.insert_app(self.clone()) {
                        tracing::error!("{:?}", err);
                    }
                }
                let window_data = self
                    .windows
                    .try_update(|windows| windows.remove(&window_id))
                    .unwrap();
                if let Some(window_data) = window_data {
                    window_data.scope.dispose();
                }
                if let Err(err) = db.save_app(self) {
                    tracing::error!("{:?}", err);
                }
            }
            AppCommand::CloseWindow(window_id) => {
                floem::close_window(window_id);
            }
            AppCommand::NewWindow { folder } => {
                self.new_window(folder);
            }
            AppCommand::WindowGotFocus(window_id) => {
                self.active_window.set(window_id);
            }
        }
    }

    fn create_windows(
        &self,
        db: Arc<LapceDb>,
        paths: Vec<PathObject>,
    ) -> floem::Application {
        let mut app = floem::Application::new();

        let mut initial_windows = 0;

        // Split user input into known existing directors and
        // file paths that exist or not
        let (dirs, files): (Vec<&PathObject>, Vec<&PathObject>) =
            paths.iter().partition(|p| p.is_dir);

        let files: Vec<PathObject> = files.into_iter().cloned().collect();
        let mut files = if files.is_empty() { None } else { Some(files) };

        if !dirs.is_empty() {
            // There were directories specified, so we'll load those as windows

            // Use the last opened window's size and position as the default
            let (size, mut pos) = db
                .get_window()
                .map(|i| (i.size, i.pos))
                .unwrap_or_else(|_| (Size::new(800.0, 600.0), Point::new(0.0, 0.0)));

            for dir in dirs {
                #[cfg(windows)]
                let workspace_type = if !std::env::var("WSL_DISTRO_NAME")
                    .unwrap_or_default()
                    .is_empty()
                    || !std::env::var("WSL_INTEROP").unwrap_or_default().is_empty()
                {
                    LapceWorkspaceType::RemoteWSL(crate::workspace::WslHost {
                        host: String::new(),
                    })
                } else {
                    LapceWorkspaceType::Local
                };
                #[cfg(not(windows))]
                let workspace_type = LapceWorkspaceType::Local;

                let info = WindowInfo {
                    size,
                    pos,
                    maximised: false,
                    tabs: TabsInfo {
                        active_tab: 0,
                        workspaces: vec![LapceWorkspace {
                            kind: workspace_type,
                            path: Some(dir.path.to_owned()),
                            last_open: 0,
                        }],
                    },
                };

                pos += (50.0, 50.0);

                let config = self
                    .default_window_config()
                    .size(info.size)
                    .position(info.pos);
                let config = if cfg!(target_os = "macos")
                    || self.config.get_untracked().core.custom_titlebar
                {
                    config.show_titlebar(false)
                } else {
                    config
                };
                let app_data = self.clone();
                let files = files.take().unwrap_or_default();
                app = app.window(
                    move |window_id| app_data.app_view(window_id, info, files),
                    Some(config),
                );
                initial_windows += 1;
            }
        } else if files.is_none() {
            // There were no dirs and no files specified, so we'll load the last windows
            match db.get_app() {
                Ok(app_info) => {
                    for info in app_info.windows {
                        let config = self
                            .default_window_config()
                            .size(info.size)
                            .position(info.pos);
                        let config = if cfg!(target_os = "macos")
                            || self.config.get_untracked().core.custom_titlebar
                        {
                            config.show_titlebar(false)
                        } else {
                            config
                        };
                        let app_data = self.clone();
                        app = app.window(
                            move |window_id| {
                                app_data.app_view(window_id, info, vec![])
                            },
                            Some(config),
                        );
                        initial_windows += 1;
                    }
                }
                Err(err) => {
                    tracing::error!("{:?}", err);
                }
            }
        }

        if initial_windows == 0 {
            let mut info = db.get_window().unwrap_or_else(|_| WindowInfo {
                size: Size::new(800.0, 600.0),
                pos: Point::ZERO,
                maximised: false,
                tabs: TabsInfo {
                    active_tab: 0,
                    workspaces: vec![LapceWorkspace::default()],
                },
            });
            info.tabs = TabsInfo {
                active_tab: 0,
                workspaces: vec![LapceWorkspace::default()],
            };
            let config = self
                .default_window_config()
                .size(info.size)
                .position(info.pos);
            let config = if cfg!(target_os = "macos")
                || self.config.get_untracked().core.custom_titlebar
            {
                config.show_titlebar(false)
            } else {
                config
            };
            let app_data = self.clone();
            app = app.window(
                move |window_id| {
                    app_data.app_view(
                        window_id,
                        info,
                        files.take().unwrap_or_default(),
                    )
                },
                Some(config),
            );
        }

        app
    }

    fn app_view(
        &self,
        window_id: WindowId,
        info: WindowInfo,
        files: Vec<PathObject>,
    ) -> impl View + use<> {
        let app_view_id = create_rw_signal(floem::ViewId::new());
        let window_data = WindowData::new(
            window_id,
            app_view_id,
            info,
            self.window_scale,
            self.latest_release.read_only(),
            self.plugin_paths.clone(),
            self.app_command,
        );

        {
            let cur_window_tab = window_data.active.get_untracked();
            let (_, window_tab) =
                &window_data.window_tabs.get_untracked()[cur_window_tab];
            for file in files {
                let position = file.linecol.map(|pos| {
                    EditorPosition::Position(lsp_types::Position {
                        line: pos.line.saturating_sub(1) as u32,
                        character: pos.column.saturating_sub(1) as u32,
                    })
                });

                window_tab.run_internal_command(InternalCommand::GoToLocation {
                    location: EditorLocation {
                        path: file.path.clone(),
                        position,
                        scroll_offset: None,
                        // Create a new editor for the file, so we don't change any current unconfirmed
                        // editor
                        ignore_unconfirmed: true,
                        same_editor_tab: false,
                    },
                });
            }
        }

        self.windows.update(|windows| {
            windows.insert(window_id, window_data.clone());
        });
        let window_size = window_data.common.size;
        let position = window_data.position;
        let window_scale = window_data.window_scale;
        let app_command = window_data.app_command;
        let config = window_data.config;
        // The KeyDown and PointerDown event handlers both need ownership of a WindowData object.
        let key_down_window_data = window_data.clone();
        let view = stack((
            workspace_tab_header(window_data.clone()),
            window(window_data.clone()),
            stack((
                drag_resize_window_area(ResizeDirection::West, empty()).style(|s| {
                    s.absolute().width(4.0).height_full().pointer_events_auto()
                }),
                drag_resize_window_area(ResizeDirection::North, empty()).style(
                    |s| s.absolute().width_full().height(4.0).pointer_events_auto(),
                ),
                drag_resize_window_area(ResizeDirection::East, empty()).style(
                    move |s| {
                        s.absolute()
                            .margin_left(window_size.get().width as f32 - 4.0)
                            .width(4.0)
                            .height_full()
                            .pointer_events_auto()
                    },
                ),
                drag_resize_window_area(ResizeDirection::South, empty()).style(
                    move |s| {
                        s.absolute()
                            .margin_top(window_size.get().height as f32 - 4.0)
                            .width_full()
                            .height(4.0)
                            .pointer_events_auto()
                    },
                ),
                drag_resize_window_area(ResizeDirection::NorthWest, empty()).style(
                    |s| s.absolute().width(20.0).height(4.0).pointer_events_auto(),
                ),
                drag_resize_window_area(ResizeDirection::NorthWest, empty()).style(
                    |s| s.absolute().width(4.0).height(20.0).pointer_events_auto(),
                ),
                drag_resize_window_area(ResizeDirection::NorthEast, empty()).style(
                    move |s| {
                        s.absolute()
                            .margin_left(window_size.get().width as f32 - 20.0)
                            .width(20.0)
                            .height(4.0)
                            .pointer_events_auto()
                    },
                ),
                drag_resize_window_area(ResizeDirection::NorthEast, empty()).style(
                    move |s| {
                        s.absolute()
                            .margin_left(window_size.get().width as f32 - 4.0)
                            .width(4.0)
                            .height(20.0)
                            .pointer_events_auto()
                    },
                ),
                drag_resize_window_area(ResizeDirection::SouthWest, empty()).style(
                    move |s| {
                        s.absolute()
                            .margin_top(window_size.get().height as f32 - 4.0)
                            .width(20.0)
                            .height(4.0)
                            .pointer_events_auto()
                    },
                ),
                drag_resize_window_area(ResizeDirection::SouthWest, empty()).style(
                    move |s| {
                        s.absolute()
                            .margin_top(window_size.get().height as f32 - 20.0)
                            .width(4.0)
                            .height(20.0)
                            .pointer_events_auto()
                    },
                ),
                drag_resize_window_area(ResizeDirection::SouthEast, empty()).style(
                    move |s| {
                        s.absolute()
                            .margin_left(window_size.get().width as f32 - 20.0)
                            .margin_top(window_size.get().height as f32 - 4.0)
                            .width(20.0)
                            .height(4.0)
                            .pointer_events_auto()
                    },
                ),
                drag_resize_window_area(ResizeDirection::SouthEast, empty()).style(
                    move |s| {
                        s.absolute()
                            .margin_left(window_size.get().width as f32 - 4.0)
                            .margin_top(window_size.get().height as f32 - 20.0)
                            .width(4.0)
                            .height(20.0)
                            .pointer_events_auto()
                    },
                ),
            ))
            .debug_name("Drag Resize Areas")
            .style(move |s| {
                s.absolute()
                    .size_full()
                    .apply_if(
                        cfg!(target_os = "macos")
                            || !config.get_untracked().core.custom_titlebar,
                        |s| s.hide(),
                    )
                    .pointer_events_none()
            }),
        ))
        .style(|s| s.flex_col().size_full());
        let view_id = view.id();
        app_view_id.set(view_id);

        view_id.request_focus();

        view.window_scale(move || window_scale.get())
            .keyboard_navigable()
            .on_event(EventListener::KeyDown, move |event| {
                if let Event::KeyDown(key_event) = event {
                    if key_down_window_data.key_down(key_event) {
                        view_id.request_focus();
                    }
                    EventPropagation::Stop
                } else {
                    EventPropagation::Continue
                }
            })
            .on_event(EventListener::PointerDown, {
                let window_data = window_data.clone();
                move |event| {
                    if let Event::PointerDown(pointer_event) = event {
                        window_data.key_down(pointer_event);
                        EventPropagation::Stop
                    } else {
                        EventPropagation::Continue
                    }
                }
            })
            .on_event_stop(EventListener::WindowResized, move |event| {
                if let Event::WindowResized(size) = event {
                    window_size.set(*size);
                }
            })
            .on_event_stop(EventListener::WindowMoved, move |event| {
                if let Event::WindowMoved(point) = event {
                    position.set(*point);
                }
            })
            .on_event_stop(EventListener::WindowGotFocus, move |_| {
                app_command.send(AppCommand::WindowGotFocus(window_id));
            })
            .on_event_stop(EventListener::WindowClosed, move |_| {
                app_command.send(AppCommand::WindowClosed(window_id));
            })
            .on_event_stop(EventListener::DroppedFile, move |event: &Event| {
                if let Event::DroppedFile(file) = event {
                    if file.path.is_dir() {
                        app_command.send(AppCommand::NewWindow {
                            folder: Some(file.path.clone()),
                        });
                    } else if let Some(win_tab_data) =
                        window_data.active_window_tab()
                    {
                        win_tab_data.common.internal_command.send(
                            InternalCommand::GoToLocation {
                                location: EditorLocation {
                                    path: file.path.clone(),
                                    position: None,
                                    scroll_offset: None,
                                    ignore_unconfirmed: false,
                                    same_editor_tab: false,
                                },
                            },
                        )
                    }
                }
            })
            .debug_name("App View")
    }
}

fn workbench(window_tab_data: Rc<WindowTabData>) -> impl View {
    let workbench_size = window_tab_data.common.workbench_size;
    let main_split_width = window_tab_data.main_split.width;
    stack((
        panel_container_view(window_tab_data.clone(), PanelContainerPosition::Left),
        {
            let window_tab_data = window_tab_data.clone();
            stack((
                main_split(window_tab_data.clone()),
                panel_container_view(
                    window_tab_data,
                    PanelContainerPosition::Bottom,
                ),
            ))
            .on_resize(move |rect| {
                let width = rect.size().width;
                if main_split_width.get_untracked() != width {
                    main_split_width.set(width);
                }
            })
            .style(|s| s.flex_col().flex_grow(1.0f32))
        },
        panel_container_view(window_tab_data.clone(), PanelContainerPosition::Right),
        window_message_view(window_tab_data.messages, window_tab_data.common.config),
    ))
    .on_resize(move |rect| {
        let size = rect.size();
        if size != workbench_size.get_untracked() {
            workbench_size.set(size);
        }
    })
    .style(move |s| s.size_full())
    .debug_name("Workbench")
}

fn window_tab(window_tab_data: Rc<WindowTabData>) -> impl View {
    let source_control = window_tab_data.source_control.clone();
    let window_origin = window_tab_data.common.window_origin;
    let layout_rect = window_tab_data.layout_rect;
    let config = window_tab_data.common.config;
    let workbench_command = window_tab_data.common.workbench_command;
    let window_tab_scope = window_tab_data.scope;
    let hover_active = window_tab_data.common.hover.active;
    let status_height = window_tab_data.status_height;

    let view = stack((
        stack((
            title(window_tab_data.clone()),
            workbench(window_tab_data.clone()),
            status(
                window_tab_data.clone(),
                source_control,
                workbench_command,
                status_height,
                config,
            ),
        ))
        .on_resize(move |rect| {
            layout_rect.set(rect);
        })
        .on_move(move |point| {
            window_origin.set(point);
        })
        .style(|s| s.size_full().flex_col())
        .debug_name("Base Layer"),
        completion(window_tab_data.clone()),
        hover(window_tab_data.clone()),
        code_action(window_tab_data.clone()),
        rename(window_tab_data.clone()),
        palette(window_tab_data.clone()),
        about::about_popup(window_tab_data.clone()),
        alert::alert_box(window_tab_data.alert_data.clone()),
    ))
    .on_cleanup(move || {
        window_tab_scope.dispose();
    })
    .on_event_cont(EventListener::PointerMove, move |_| {
        if hover_active.get_untracked() {
            hover_active.set(false);
        }
    })
    .style(move |s| {
        let config = config.get();
        s.size_full()
            .color(config.color(LapceColor::EDITOR_FOREGROUND))
            .background(config.color(LapceColor::EDITOR_BACKGROUND))
            .font_size(config.ui.font_size() as f32)
            .apply_if(!config.ui.font_family.is_empty(), |s| {
                s.font_family(config.ui.font_family.clone())
            })
            .class(floem::views::scroll::Handle, |s| {
                s.background(config.color(LapceColor::LAPCE_SCROLL_BAR))
            })
    })
    .debug_name("Window Tab");

    let view_id = view.id();
    window_tab_data.common.view_id.set(view_id);
    view
}

fn workspace_title(workspace: &LapceWorkspace) -> Option<String> {
    let p = workspace.path.as_ref()?;
    let dir = p.file_name().unwrap_or(p.as_os_str()).to_string_lossy();
    Some(match &workspace.kind {
        LapceWorkspaceType::Local => format!("{dir}"),
        LapceWorkspaceType::RemoteSSH(remote) => format!("{dir} [{remote}]"),
        #[cfg(windows)]
        LapceWorkspaceType::RemoteWSL(remote) => format!("{dir} [{remote}]"),
    })
}

fn workspace_tab_header(window_data: WindowData) -> impl View {
    let tabs = window_data.window_tabs;
    let active = window_data.active;
    let config = window_data.config;
    let window_tab_header_height = window_data.common.window_tab_header_height;
    let available_width = create_rw_signal(0.0);
    let add_icon_width = create_rw_signal(0.0);
    let window_control_width = create_rw_signal(0.0);
    let window_maximized = window_data.common.window_maximized;
    let num_window_tabs = window_data.num_window_tabs;
    let window_command = window_data.common.window_command;

    let tab_width = create_memo(move |_| {
        let window_control_width = if !cfg!(target_os = "macos")
            && config.get_untracked().core.custom_titlebar
        {
            window_control_width.get()
        } else {
            0.0
        };
        let available_width = available_width.get()
            - add_icon_width.get()
            - if cfg!(target_os = "macos") { 75.0 } else { 0.0 }
            - window_control_width
            - 30.0;
        let tabs_len = tabs.with(|tabs| tabs.len());
        if tabs_len > 0 {
            (available_width / tabs_len as f64).min(200.0)
        } else {
            available_width
        }
    });

    let local_window_data = window_data.clone();
    let dragging_index: RwSignal<Option<RwSignal<usize>>> = create_rw_signal(None);
    let view_fn = move |(index, tab): (RwSignal<usize>, Rc<WindowTabData>)| {
        let drag_over_left = create_rw_signal(None);
        let window_data = local_window_data.clone();
        stack((
            container({
                stack((
                    stack((
                        text(
                            workspace_title(&tab.workspace)
                                .unwrap_or_else(|| String::from("New Tab")),
                        )
                        .style(|s| {
                            s.margin_left(10.0)
                                .min_width(0.0)
                                .flex_basis(0.0)
                                .flex_grow(1.0f32)
                                .selectable(false)
                                .text_ellipsis()
                        }),
                        {
                            let window_data = local_window_data.clone();
                            clickable_icon(
                                || LapceIcons::WINDOW_CLOSE,
                                move || {
                                    window_data.run_window_command(
                                        WindowCommand::CloseWorkspaceTab {
                                            index: Some(index.get_untracked()),
                                        },
                                    );
                                },
                                || false,
                                || false,
                                || "Close",
                                config.read_only(),
                            )
                            .style(|s| s.margin_horiz(6.0))
                        },
                    ))
                    .on_event_stop(EventListener::DragOver, move |event| {
                        if dragging_index.get_untracked().is_some() {
                            if let Event::PointerMove(pointer_event) = event {
                                let left = pointer_event.pos.x
                                    < tab_width.get_untracked() / 2.0;
                                if drag_over_left.get_untracked() != Some(left) {
                                    drag_over_left.set(Some(left));
                                }
                            }
                        }
                    })
                    .on_event(EventListener::Drop, move |event| {
                        if dragging_index.get_untracked().is_some() {
                            drag_over_left.set(None);
                            if let Event::PointerUp(pointer_event) = event {
                                let left = pointer_event.pos.x
                                    < tab_width.get_untracked() / 2.0;
                                let index = index.get_untracked();
                                let new_index = if left { index } else { index + 1 };
                                if let Some(from_index) =
                                    dragging_index.get_untracked()
                                {
                                    window_data.move_tab(
                                        from_index.get_untracked(),
                                        new_index,
                                    );
                                }
                                dragging_index.set(None);
                            }
                            EventPropagation::Stop
                        } else {
                            EventPropagation::Continue
                        }
                    })
                    .on_event_stop(EventListener::DragLeave, move |_| {
                        drag_over_left.set(None);
                    })
                    .style(move |s| {
                        let config = config.get();
                        s.width_full()
                            .min_width(0.0)
                            .items_center()
                            .border_right(1.0)
                            .border_color(config.color(LapceColor::LAPCE_BORDER))
                            .apply_if(
                                cfg!(target_os = "macos") && index.get() == 0,
                                |s| s.border_left(1.0),
                            )
                    }),
                    container(empty().style(move |s| {
                        s.size_full()
                            .apply_if(active.get() == index.get(), |s| {
                                s.border_bottom(2.0)
                            })
                            .border_color(
                                config
                                    .get()
                                    .color(LapceColor::LAPCE_TAB_ACTIVE_UNDERLINE),
                            )
                    }))
                    .style(move |s| {
                        s.position(Position::Absolute)
                            .padding_horiz(3.0)
                            .size_full()
                            .pointer_events_none()
                    }),
                ))
                .style(move |s| s.size_full().items_center())
            })
            .draggable()
            .on_event_stop(EventListener::DragStart, move |_| {
                dragging_index.set(Some(index));
            })
            .on_event_stop(EventListener::DragEnd, move |_| {
                dragging_index.set(None);
            })
            .dragging_style(move |s| {
                let config = config.get();
                s.border(1.0)
                    .border_radius(6.0)
                    .border_color(config.color(LapceColor::LAPCE_BORDER))
                    .color(
                        config
                            .color(LapceColor::EDITOR_FOREGROUND)
                            .multiply_alpha(0.7),
                    )
                    .background(
                        config
                            .color(LapceColor::PANEL_BACKGROUND)
                            .multiply_alpha(0.7),
                    )
            })
            .on_click_stop(move |_| {
                active.set(index.get_untracked());
            })
            .style(move |s| s.size_full()),
            empty().style(move |s| {
                let index = index.get();
                s.absolute()
                    .margin_left(if index == 0 { 0.0 } else { -2.0 })
                    .width(
                        tab_width.get() as f32 + if index == 0 { 1.0 } else { 3.0 },
                    )
                    .height_full()
                    .border_color(
                        config.get().color(LapceColor::LAPCE_TAB_ACTIVE_UNDERLINE),
                    )
                    .apply_if(drag_over_left.get().is_some(), move |s| {
                        let drag_over_left = drag_over_left.get_untracked().unwrap();
                        if drag_over_left {
                            s.border_left(3.0)
                        } else {
                            s.border_right(3.0)
                        }
                    })
                    .apply_if(drag_over_left.get().is_none(), move |s| s.hide())
            }),
        ))
        .style(move |s| s.height_full().width(tab_width.get() as f32))
    };

    stack((
        empty().style(move |s| {
            let is_macos = cfg!(target_os = "macos");
            s.min_width(75.0)
                .width(75.0)
                .apply_if(!is_macos, |s| s.hide())
        }),
        dyn_stack(
            move || {
                let tabs = tabs.get();
                for (i, (index, _)) in tabs.iter().enumerate() {
                    if index.get_untracked() != i {
                        index.set(i);
                    }
                }
                tabs
            },
            |(_, tab)| tab.window_tab_id,
            view_fn,
        )
        .style(|s| s.height_full()),
        container(clickable_icon(
            || LapceIcons::ADD,
            move || {
                window_data.run_window_command(WindowCommand::NewWorkspaceTab {
                    workspace: LapceWorkspace::default(),
                    end: true,
                });
            },
            || false,
            || false,
            || "New Workspace Tab",
            config.read_only(),
        ))
        .on_resize(move |rect| {
            let current = add_icon_width.get_untracked();
            if rect.width() != current {
                add_icon_width.set(rect.width());
            }
        })
        .style(|s| {
            s.height_full()
                .padding_left(10.0)
                .padding_right(10.0)
                .items_center()
        }),
        drag_window_area(empty())
            .style(|s| s.height_full().flex_basis(0.0).flex_grow(1.0f32)),
        window_controls_view(
            window_command,
            false,
            num_window_tabs,
            window_maximized,
            config.read_only(),
        )
        .on_resize(move |rect| {
            let width = rect.width();
            if window_control_width.get_untracked() != width {
                window_control_width.set(width);
            }
        }),
    ))
    .on_resize(move |rect| {
        let current = available_width.get_untracked();
        if rect.width() != current {
            available_width.set(rect.width());
        }
        window_tab_header_height.set(rect.height());
    })
    .style(move |s| {
        let config = config.get();
        s.border_bottom(1.0)
            .width_full()
            .height(37.0)
            .font_size(config.ui.font_size() as f32)
            .apply_if(!config.ui.font_family.is_empty(), |s| {
                s.font_family(config.ui.font_family.clone())
            })
            .apply_if(tabs.with(|tabs| tabs.len() < 2), |s| s.hide())
            .color(config.color(LapceColor::EDITOR_FOREGROUND))
            .border_color(config.color(LapceColor::LAPCE_BORDER))
            .background(config.color(LapceColor::PANEL_BACKGROUND))
            .items_center()
    })
    .debug_name("Workspace Tab Header")
}

fn window(window_data: WindowData) -> impl View {
    let window_tabs = window_data.window_tabs.read_only();
    let active = window_data.active.read_only();
    let items = move || window_tabs.get();
    let key = |(_, window_tab): &(RwSignal<usize>, Rc<WindowTabData>)| {
        window_tab.window_tab_id
    };
    let active = move || active.get();
    let window_focus = create_rw_signal(false);
    let ime_enabled = window_data.ime_enabled;
    let window_maximized = window_data.common.window_maximized;

    tab(active, items, key, |(_, window_tab_data)| {
        window_tab(window_tab_data)
    })
    .window_title(move || {
        let active = active();
        let window_tabs = window_tabs.get();
        let workspace = window_tabs
            .get(active)
            .or_else(|| window_tabs.last())
            .and_then(|(_, window_tab)| window_tab.workspace.display());
        match workspace {
            Some(workspace) => format!("{workspace} - Lapce"),
            None => "Lapce".to_string(),
        }
    })
    .on_event_stop(EventListener::ImeEnabled, move |_| {
        ime_enabled.set(true);
    })
    .on_event_stop(EventListener::ImeDisabled, move |_| {
        ime_enabled.set(false);
    })
    .on_event_cont(EventListener::WindowGotFocus, move |_| {
        window_focus.set(true);
    })
    .on_event_cont(EventListener::WindowMaximizeChanged, move |event| {
        if let Event::WindowMaximizeChanged(maximized) = event {
            window_maximized.set(*maximized);
        }
    })
    .window_menu(move || {
        window_focus.track();
        let active = active();
        let window_tabs = window_tabs.get();
        let window_tab = window_tabs.get(active).or_else(|| window_tabs.last());
        if let Some((_, window_tab)) = window_tab {
            window_tab.common.keypress.track();
            let workbench_command = window_tab.common.workbench_command;
            let lapce_command = window_tab.common.lapce_command;
            window_menu(lapce_command, workbench_command)
        } else {
            Menu::new("Lapce")
        }
    })
    .style(|s| s.size_full())
    .debug_name("Window")
}
