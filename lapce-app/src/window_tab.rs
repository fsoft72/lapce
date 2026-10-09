use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        mpsc::{Sender, channel},
    },
};

mod internal_commands;
mod notifications;
mod panels;
mod popup_origins;
mod workbench_commands;

use floem::{
    ViewId,
    action::{TimerToken, remove_overlay},
    ext_event::{create_ext_action, create_signal_from_channel},
    keyboard::Modifiers,
    kurbo::Size,
    peniko::kurbo::{Point, Rect},
    prelude::SignalTrack,
    reactive::{
        Memo, ReadSignal, RwSignal, Scope, SignalGet, SignalUpdate, SignalWith,
        WriteSignal, use_context,
    },
    text::{Attrs, AttrsList, FamilyOwned, LineHeightValue, TextLayout},
};
use im::HashMap;
use indexmap::IndexMap;
use lapce_core::{command::FocusCommand, mode::Mode, register::Register};
use lapce_rpc::{
    RpcError,
    file::PathObject,
    plugin::PluginId,
    proxy::{ProxyResponse, ProxyRpcHandler, ProxyStatus},
    terminal::TermId,
};
use lsp_types::{CodeActionOrCommand, CodeLens, ProgressToken, ShowMessageParams};
use tracing::{debug, error};

use crate::{
    about::AboutData,
    agent::AgentData,
    alert::AlertBoxData,
    code_action::CodeActionData,
    command::{
        CommandExecuted, CommandKind, InternalCommand, LapceCommand,
        LapceWorkbenchCommand, WindowCommand,
    },
    completion::CompletionData,
    config::LapceConfig,
    db::LapceDb,
    debug::LapceBreakpoint,
    editor::location::{EditorLocation, EditorPosition},
    editor_tab::EditorTabChild,
    file_explorer::data::FileExplorerData,
    find::Find,
    global_search::GlobalSearchData,
    hover::HoverData,
    id::WindowTabId,
    inline_completion::InlineCompletionData,
    keypress::{EventRef, KeyPressData, KeyPressFocus, condition::Condition},
    listener::Listener,
    main_split::{MainSplitData, SplitData, SplitDirection, SplitMoveDirection},
    palette::{PaletteData, PaletteStatus},
    panel::{
        call_hierarchy_view::{CallHierarchyData, CallHierarchyItemData},
        data::{PanelData, default_panel_order},
        kind::PanelKind,
    },
    plugin::PluginData,
    proxy::{ProxyData, new_proxy},
    rename::RenameData,
    source_control::SourceControlData,
    terminal::{
        event::{TermEvent, TermNotification, terminal_update_process},
        panel::TerminalPanelData,
    },
    tracing::*,
    window::WindowCommonData,
    workspace::{LapceWorkspace, WorkspaceInfo},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Focus {
    Workbench,
    Palette,
    CodeAction,
    Rename,
    AboutPopup,
    Panel(PanelKind),
}

#[derive(Clone)]
pub enum DragContent {
    Panel(PanelKind),
    EditorTab(EditorTabChild),
}

impl DragContent {
    pub fn is_panel(&self) -> bool {
        matches!(self, DragContent::Panel(_))
    }
}

#[derive(Clone)]
pub struct WorkProgress {
    pub token: ProgressToken,
    pub title: String,
    pub message: Option<String>,
    pub percentage: Option<u32>,
}

#[derive(Clone)]
pub struct CommonData {
    pub workspace: Arc<LapceWorkspace>,
    pub scope: Scope,
    pub focus: RwSignal<Focus>,
    pub keypress: RwSignal<KeyPressData>,
    pub completion: RwSignal<CompletionData>,
    pub inline_completion: RwSignal<InlineCompletionData>,
    pub hover: HoverData,
    pub register: RwSignal<Register>,
    pub find: Find,
    pub workbench_size: RwSignal<Size>,
    pub window_origin: RwSignal<Point>,
    pub internal_command: Listener<InternalCommand>,
    pub lapce_command: Listener<LapceCommand>,
    pub workbench_command: Listener<LapceWorkbenchCommand>,
    pub term_tx: Sender<(TermId, TermEvent)>,
    pub term_notification_tx: Sender<TermNotification>,
    pub proxy: ProxyRpcHandler,
    pub view_id: RwSignal<ViewId>,
    pub ui_line_height: Memo<f64>,
    pub dragging: RwSignal<Option<DragContent>>,
    pub config: ReadSignal<Arc<LapceConfig>>,
    pub proxy_status: RwSignal<Option<ProxyStatus>>,
    pub mouse_hover_timer: RwSignal<TimerToken>,
    pub breakpoints: RwSignal<BTreeMap<PathBuf, BTreeMap<usize, LapceBreakpoint>>>,
    // the current focused view which will receive keyboard events
    pub keyboard_focus: RwSignal<Option<ViewId>>,
    pub window_common: Rc<WindowCommonData>,
}

impl std::fmt::Debug for CommonData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommonData")
            .field("workspace", &self.workspace)
            .finish()
    }
}

#[derive(Clone)]
pub struct WindowTabData {
    pub scope: Scope,
    pub window_tab_id: WindowTabId,
    pub workspace: Arc<LapceWorkspace>,
    pub palette: PaletteData,
    pub main_split: MainSplitData,
    pub file_explorer: FileExplorerData,
    pub panel: PanelData,
    pub terminal: TerminalPanelData,
    pub plugin: PluginData,
    pub agent: AgentData,
    pub code_action: RwSignal<CodeActionData>,
    pub code_lens: RwSignal<Option<ViewId>>,
    pub source_control: SourceControlData,
    pub rename: RenameData,
    pub global_search: GlobalSearchData,
    pub call_hierarchy_data: CallHierarchyData,
    pub about_data: AboutData,
    pub alert_data: AlertBoxData,
    pub layout_rect: RwSignal<Rect>,
    pub title_height: RwSignal<f64>,
    pub status_height: RwSignal<f64>,
    pub proxy: ProxyData,
    pub set_config: WriteSignal<Arc<LapceConfig>>,
    pub update_in_progress: RwSignal<bool>,
    pub progresses: RwSignal<IndexMap<ProgressToken, WorkProgress>>,
    pub messages: RwSignal<Vec<(String, ShowMessageParams)>>,
    pub common: Rc<CommonData>,
}

impl std::fmt::Debug for WindowTabData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowTabData")
            .field("window_tab_id", &self.window_tab_id)
            .finish()
    }
}

impl KeyPressFocus for WindowTabData {
    fn get_mode(&self) -> Mode {
        Mode::Normal
    }

    fn check_condition(&self, condition: Condition) -> bool {
        match condition {
            Condition::PanelFocus => {
                matches!(self.common.focus.get_untracked(), Focus::Panel(_))
            }
            Condition::SourceControlFocus => {
                self.common.focus.get_untracked()
                    == Focus::Panel(PanelKind::SourceControl)
            }
            _ => false,
        }
    }

    fn run_command(
        &self,
        command: &LapceCommand,
        _count: Option<usize>,
        _mods: Modifiers,
    ) -> CommandExecuted {
        match &command.kind {
            CommandKind::Workbench(cmd) => {
                self.run_workbench_command(cmd.clone(), None);
            }
            CommandKind::Focus(cmd) => {
                if self.common.focus.get_untracked() == Focus::Workbench {
                    match cmd {
                        FocusCommand::SplitClose => {
                            self.main_split.editor_tab_child_close_active();
                        }
                        FocusCommand::SplitVertical => {
                            self.main_split.split_active(SplitDirection::Vertical);
                        }
                        FocusCommand::SplitHorizontal => {
                            self.main_split.split_active(SplitDirection::Horizontal);
                        }
                        FocusCommand::SplitRight => {
                            self.main_split
                                .split_move_active(SplitMoveDirection::Right);
                        }
                        FocusCommand::SplitLeft => {
                            self.main_split
                                .split_move_active(SplitMoveDirection::Left);
                        }
                        FocusCommand::SplitUp => {
                            self.main_split
                                .split_move_active(SplitMoveDirection::Up);
                        }
                        FocusCommand::SplitDown => {
                            self.main_split
                                .split_move_active(SplitMoveDirection::Down);
                        }
                        FocusCommand::SplitExchange => {
                            self.main_split.split_exchange_active();
                        }
                        _ => {
                            return CommandExecuted::No;
                        }
                    }
                }
            }
            _ => {
                return CommandExecuted::No;
            }
        }

        CommandExecuted::Yes
    }

    fn receive_char(&self, _c: &str) {}
}

impl WindowTabData {
    pub fn new(
        cx: Scope,
        workspace: Arc<LapceWorkspace>,
        window_common: Rc<WindowCommonData>,
    ) -> Self {
        let cx = cx.create_child();
        let db: Arc<LapceDb> = use_context().unwrap();

        let disabled_volts = db.get_disabled_volts().unwrap_or_default();
        let workspace_disabled_volts = db
            .get_workspace_disabled_volts(&workspace)
            .unwrap_or_default();
        let mut all_disabled_volts = disabled_volts.clone();
        all_disabled_volts.extend(workspace_disabled_volts.clone());

        let workspace_info = if workspace.path.is_some() {
            db.get_workspace_info(&workspace).ok()
        } else {
            let mut info = db.get_workspace_info(&workspace).ok();
            if let Some(info) = info.as_mut() {
                info.split.children.clear();
            }
            info
        };

        let config = LapceConfig::load(
            &workspace,
            &all_disabled_volts,
            &window_common.extra_plugin_paths,
        );
        let lapce_command = Listener::new_empty(cx);
        let workbench_command = Listener::new_empty(cx);
        let internal_command = Listener::new_empty(cx);
        let keypress = cx.create_rw_signal(KeyPressData::new(cx, &config));
        let proxy_status = cx.create_rw_signal(None);

        let (term_tx, term_rx) = channel();
        let (term_notification_tx, term_notification_rx) = channel();
        {
            let term_notification_tx = term_notification_tx.clone();
            std::thread::Builder::new()
                .name("terminal update process".to_owned())
                .spawn(move || {
                    terminal_update_process(term_rx, term_notification_tx);
                })
                .unwrap();
        }

        let proxy = new_proxy(
            workspace.clone(),
            all_disabled_volts,
            window_common.extra_plugin_paths.as_ref().clone(),
            config.plugins.clone(),
            term_tx.clone(),
        );
        let (config, set_config) = cx.create_signal(Arc::new(config));

        let focus = cx.create_rw_signal(Focus::Workbench);
        let completion = cx.create_rw_signal(CompletionData::new(cx, config));
        let inline_completion = cx.create_rw_signal(InlineCompletionData::new(cx));
        let hover = HoverData::new(cx);

        let register = cx.create_rw_signal(Register::default());
        let view_id = cx.create_rw_signal(ViewId::new());
        let find = Find::new(cx);

        let ui_line_height = cx.create_memo(move |_| {
            let config = config.get();
            let mut text_layout = TextLayout::new();

            let family: Vec<FamilyOwned> =
                FamilyOwned::parse_list(&config.ui.font_family).collect();
            let attrs = Attrs::new()
                .family(&family)
                .font_size(config.ui.font_size() as f32)
                .line_height(LineHeightValue::Normal(1.8));
            let attrs_list = AttrsList::new(attrs);
            text_layout.set_text("W", attrs_list, None);
            text_layout.size().height
        });

        let common = Rc::new(CommonData {
            workspace: workspace.clone(),
            scope: cx,
            keypress,
            focus,
            completion,
            inline_completion,
            hover,
            register,
            find,
            internal_command,
            lapce_command,
            workbench_command,
            term_tx,
            term_notification_tx,
            proxy: proxy.proxy_rpc.clone(),
            view_id,
            ui_line_height,
            dragging: cx.create_rw_signal(None),
            workbench_size: cx.create_rw_signal(Size::ZERO),
            config,
            proxy_status,
            mouse_hover_timer: cx.create_rw_signal(TimerToken::INVALID),
            window_origin: cx.create_rw_signal(Point::ZERO),
            breakpoints: cx.create_rw_signal(BTreeMap::new()),
            keyboard_focus: cx.create_rw_signal(None),
            window_common: window_common.clone(),
        });

        let main_split = MainSplitData::new(cx, common.clone());
        let code_action =
            cx.create_rw_signal(CodeActionData::new(cx, common.clone()));
        let source_control =
            SourceControlData::new(cx, main_split.editors, common.clone());
        let file_explorer =
            FileExplorerData::new(cx, main_split.editors, common.clone());

        if let Some(info) = workspace_info.as_ref() {
            let root_split = main_split.root_split;
            info.split.to_data(main_split.clone(), None, root_split);
        } else {
            let root_split = main_split.root_split;
            let root_split_data = {
                let cx = cx.create_child();
                let root_split_data = SplitData {
                    scope: cx,
                    parent_split: None,
                    split_id: root_split,
                    children: Vec::new(),
                    direction: SplitDirection::Horizontal,
                    window_origin: Point::ZERO,
                    layout_rect: Rect::ZERO,
                };
                cx.create_rw_signal(root_split_data)
            };
            main_split.splits.update(|splits| {
                splits.insert(root_split, root_split_data);
            });
        }

        let palette = PaletteData::new(
            cx,
            workspace.clone(),
            main_split.clone(),
            keypress.read_only(),
            source_control.clone(),
            common.clone(),
        );

        let title_height = cx.create_rw_signal(0.0);
        let status_height = cx.create_rw_signal(0.0);
        let panel_available_size = cx.create_memo(move |_| {
            let title_height = title_height.get();
            let status_height = status_height.get();
            let num_window_tabs = window_common.num_window_tabs.get();
            let window_size = window_common.size.get();
            Size::new(
                window_size.width,
                window_size.height
                    - title_height
                    - status_height
                    - if num_window_tabs > 1 {
                        window_common.window_tab_header_height.get()
                    } else {
                        0.0
                    },
            )
        });
        let panel = workspace_info
            .as_ref()
            .map(|i| {
                let panel_order = db
                    .get_panel_orders()
                    .unwrap_or_else(|_| default_panel_order());
                PanelData {
                    panels: cx.create_rw_signal(panel_order),
                    styles: cx.create_rw_signal(i.panel.styles.clone()),
                    size: cx.create_rw_signal(i.panel.size.clone()),
                    available_size: panel_available_size,
                    sections: cx.create_rw_signal(
                        i.panel
                            .sections
                            .iter()
                            .map(|(key, value)| (*key, cx.create_rw_signal(*value)))
                            .collect(),
                    ),
                    common: common.clone(),
                }
            })
            .unwrap_or_else(|| {
                let panel_order = db
                    .get_panel_orders()
                    .unwrap_or_else(|_| default_panel_order());
                PanelData::new(
                    cx,
                    panel_order,
                    panel_available_size,
                    im::HashMap::new(),
                    common.clone(),
                )
            });

        let terminal = TerminalPanelData::new(
            workspace.clone(),
            common.config.get_untracked().terminal.get_default_profile(),
            common.clone(),
            main_split.clone(),
        );
        if let Some(workspace_info) = workspace_info.as_ref() {
            terminal.debug.breakpoints.set(
                workspace_info
                    .breakpoints
                    .clone()
                    .into_iter()
                    .map(|(path, breakpoints)| {
                        (
                            path,
                            breakpoints
                                .into_iter()
                                .map(|b| (b.line, b))
                                .collect::<BTreeMap<usize, LapceBreakpoint>>(),
                        )
                    })
                    .collect(),
            );
        }

        let rename = RenameData::new(cx, main_split.editors, common.clone());
        let global_search = GlobalSearchData::new(cx, main_split.clone());

        let plugin = PluginData::new(
            cx,
            HashSet::from_iter(disabled_volts),
            HashSet::from_iter(workspace_disabled_volts),
            main_split.editors,
            common.clone(),
            proxy.core_rpc.clone(),
        );
        let agent = AgentData::new(cx, main_split.clone(), common.clone());

        {
            let notification = create_signal_from_channel(term_notification_rx);
            let terminal = terminal.clone();
            cx.create_effect(move |_| {
                notification.with(|notification| {
                    if let Some(notification) = notification.as_ref() {
                        match notification {
                            TermNotification::SetTitle { term_id, title } => {
                                terminal.set_title(term_id, title);
                            }
                            TermNotification::RequestPaint => {
                                view_id.get_untracked().request_paint();
                            }
                        }
                    }
                });
            });
        }

        let about_data = AboutData::new(cx, common.focus);
        let alert_data = AlertBoxData::new(cx, common.clone());

        let window_tab_data = Self {
            scope: cx,
            window_tab_id: WindowTabId::next(),
            workspace,
            palette,
            main_split,
            terminal,
            panel,
            file_explorer,
            code_action,
            code_lens: cx.create_rw_signal(None),
            source_control,
            plugin,
            agent,
            rename,
            global_search,
            call_hierarchy_data: CallHierarchyData {
                root: cx.create_rw_signal(None),
                common: common.clone(),
                scroll_to_line: cx.create_rw_signal(None),
            },
            about_data,
            alert_data,
            layout_rect: cx.create_rw_signal(Rect::ZERO),
            title_height,
            status_height,
            proxy,
            set_config,
            update_in_progress: cx.create_rw_signal(false),
            progresses: cx.create_rw_signal(IndexMap::new()),
            messages: cx.create_rw_signal(Vec::new()),
            common,
        };

        {
            let focus = window_tab_data.common.focus;
            let active_editor = window_tab_data.main_split.active_editor;
            let rename_active = window_tab_data.rename.active;
            let internal_command = window_tab_data.common.internal_command;
            cx.create_effect(move |_| {
                let focus = focus.get();
                active_editor.track();
                internal_command.send(InternalCommand::ResetBlinkCursor);

                if focus != Focus::Rename && rename_active.get_untracked() {
                    rename_active.set(false);
                }
            });
        }

        {
            let window_tab_data = window_tab_data.clone();
            window_tab_data.common.lapce_command.listen(move |cmd| {
                window_tab_data.run_lapce_command(cmd);
            });
        }

        {
            let window_tab_data = window_tab_data.clone();
            window_tab_data.common.workbench_command.listen(move |cmd| {
                window_tab_data.run_workbench_command(cmd, None);
            });
        }

        {
            let window_tab_data = window_tab_data.clone();
            let internal_command = window_tab_data.common.internal_command;
            internal_command.listen(move |cmd| {
                window_tab_data.run_internal_command(cmd);
            });
        }

        {
            let window_tab_data = window_tab_data.clone();
            let notification = window_tab_data.proxy.notification;
            cx.create_effect(move |_| {
                notification.with(|rpc| {
                    if let Some(rpc) = rpc.as_ref() {
                        window_tab_data.handle_core_notification(rpc);
                    }
                });
            });
        }

        window_tab_data
    }

    pub fn reload_config(&self) {
        let db: Arc<LapceDb> = use_context().unwrap();

        let disabled_volts = db.get_disabled_volts().unwrap_or_default();
        let workspace_disabled_volts = db
            .get_workspace_disabled_volts(&self.workspace)
            .unwrap_or_default();
        let mut all_disabled_volts = disabled_volts;
        all_disabled_volts.extend(workspace_disabled_volts);

        let config = LapceConfig::load(
            &self.workspace,
            &all_disabled_volts,
            &self.common.window_common.extra_plugin_paths,
        );
        self.common.keypress.update(|keypress| {
            keypress.update_keymaps(&config);
        });

        let mut change_plugins = Vec::new();
        for (key, configs) in self.common.config.get_untracked().plugins.iter() {
            if config
                .plugins
                .get(key)
                .map(|x| x != configs)
                .unwrap_or_default()
            {
                change_plugins.push(key.clone());
            }
        }
        self.set_config.set(Arc::new(config.clone()));
        if !change_plugins.is_empty() {
            self.common
                .proxy
                .update_plugin_configs(config.plugins.clone());
            if config.core.auto_reload_plugin {
                let mut plugin_metas: HashMap<
                    String,
                    lapce_rpc::plugin::VoltMetadata,
                > = self
                    .plugin
                    .installed
                    .get_untracked()
                    .values()
                    .map(|x| {
                        let meta = x.meta.get_untracked();
                        (meta.name.clone(), meta)
                    })
                    .collect();
                for name in change_plugins {
                    if let Some(meta) = plugin_metas.remove(&name) {
                        self.common.proxy.reload_volt(meta);
                    } else {
                        tracing::error!("not found volt metadata of {}", name);
                    }
                }
            }
        }
    }

    pub fn run_lapce_command(&self, cmd: LapceCommand) {
        match cmd.kind {
            CommandKind::Workbench(command) => {
                self.run_workbench_command(command, cmd.data);
            }
            CommandKind::Scroll(_)
            | CommandKind::Focus(_)
            | CommandKind::Edit(_)
            | CommandKind::Move(_) => {
                if self.palette.status.get_untracked() != PaletteStatus::Inactive {
                    self.palette.run_command(&cmd, None, Modifiers::empty());
                } else if let Some(editor_data) =
                    self.main_split.active_editor.get_untracked()
                {
                    editor_data.run_command(&cmd, None, Modifiers::empty());
                } else {
                    // TODO: dispatch to current focused view?
                }
            }
            CommandKind::MotionMode(_) => {}
            CommandKind::MultiSelection(_) => {}
        }
    }

    pub fn key_down<'a>(&self, event: impl Into<EventRef<'a>> + Copy) -> bool {
        if self.alert_data.active.get_untracked() {
            return false;
        }
        let focus = self.common.focus.get_untracked();
        let keypress = self.common.keypress.get_untracked();
        let handle = match focus {
            Focus::Workbench => self.main_split.key_down(event, &keypress),
            Focus::Palette => Some(keypress.key_down(event, &self.palette)),
            Focus::CodeAction => {
                let code_action = self.code_action.get_untracked();
                Some(keypress.key_down(event, &code_action))
            }
            Focus::Rename => Some(keypress.key_down(event, &self.rename)),
            Focus::AboutPopup => Some(keypress.key_down(event, &self.about_data)),
            Focus::Panel(PanelKind::Terminal) => {
                self.terminal.key_down(event, &keypress)
            }
            Focus::Panel(PanelKind::Search) => {
                Some(keypress.key_down(event, &self.global_search))
            }
            Focus::Panel(PanelKind::Plugin) => {
                Some(keypress.key_down(event, &self.plugin))
            }
            Focus::Panel(PanelKind::SourceControl) => {
                Some(keypress.key_down(event, &self.source_control))
            }
            Focus::Panel(PanelKind::Agent) => {
                Some(keypress.key_down(event, &self.agent))
            }
            _ => None,
        };

        if let Some(handle) = &handle {
            if handle.handled {
                true
            } else {
                keypress
                    .handle_keymatch(
                        self,
                        handle.keymatch.clone(),
                        handle.keypress.clone(),
                    )
                    .handled
            }
        } else {
            keypress.key_down(event, self).handled
        }
    }

    pub fn workspace_info(&self) -> WorkspaceInfo {
        let main_split_data = self
            .main_split
            .splits
            .get_untracked()
            .get(&self.main_split.root_split)
            .cloned()
            .unwrap();
        WorkspaceInfo {
            split: main_split_data.get_untracked().split_info(self),
            panel: self.panel.panel_info(),
            breakpoints: self
                .terminal
                .debug
                .breakpoints
                .get_untracked()
                .into_iter()
                .map(|(path, breakpoints)| {
                    (path, breakpoints.into_values().collect::<Vec<_>>())
                })
                .collect(),
        }
    }

    /// Get the mode for the current editor or terminal
    pub fn mode(&self) -> Mode {
        if self.common.config.get().core.modal {
            let mode = if self.common.focus.get() == Focus::Workbench {
                self.main_split
                    .active_editor
                    .get()
                    .map(|editor| editor.cursor().with(|c| c.get_mode()))
            } else {
                None
            };

            mode.unwrap_or(Mode::Normal)
        } else {
            Mode::Insert
        }
    }

    pub fn open_paths(&self, paths: &[PathObject]) {
        let (folders, files): (Vec<&PathObject>, Vec<&PathObject>) =
            paths.iter().partition(|p| p.is_dir);

        for folder in folders {
            self.common.window_common.window_command.send(
                WindowCommand::NewWorkspaceTab {
                    workspace: LapceWorkspace {
                        kind: self.workspace.kind.clone(),
                        path: Some(folder.path.clone()),
                        last_open: 0,
                    },
                    end: false,
                },
            );
        }

        for file in files {
            let position = file.linecol.map(|pos| {
                EditorPosition::Position(lsp_types::Position {
                    line: pos.line.saturating_sub(1) as u32,
                    character: pos.column.saturating_sub(1) as u32,
                })
            });

            self.common
                .internal_command
                .send(InternalCommand::GoToLocation {
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

    pub fn update_code_lens_id(&self, view_id: Option<ViewId>) {
        if let Some(Some(old_id)) = self.code_lens.try_update(|x| {
            let old = x.take();
            if let Some(id) = view_id {
                let _ = x.insert(id);
            }
            old
        }) {
            remove_overlay(old_id);
        }
    }

    pub fn show_code_lens(
        &self,
        mouse_click: bool,
        plugin_id: PluginId,
        offset: usize,
        lens: im::Vector<CodeLens>,
    ) {
        self.common
            .internal_command
            .send(InternalCommand::ShowCodeActions {
                offset,
                mouse_click,
                plugin_id,
                code_actions: lens
                    .into_iter()
                    .filter_map(|lens| {
                        Some(CodeActionOrCommand::Command(lens.command?))
                    })
                    .collect(),
            });
    }

    pub fn call_hierarchy_incoming(&self, item_id: ViewId) {
        let Some(root) = self.call_hierarchy_data.root.get_untracked() else {
            return;
        };
        let Some(item) = CallHierarchyItemData::find_by_id(root, item_id) else {
            return;
        };
        let root_item = item;
        let path: PathBuf = item.get_untracked().item.uri.to_file_path().unwrap();
        let scope = self.scope;
        let send =
            create_ext_action(scope, move |_rs: Result<ProxyResponse, RpcError>| {
                match _rs {
                    Ok(ProxyResponse::CallHierarchyIncomingResponse { items }) => {
                        if let Some(items) = items {
                            let mut item_children = Vec::new();
                            for x in items {
                                let item = Rc::new(x.from);
                                for range in x.from_ranges {
                                    item_children.push(scope.create_rw_signal(
                                        CallHierarchyItemData {
                                            view_id: floem::ViewId::new(),
                                            item: item.clone(),
                                            from_range: range,
                                            init: false,
                                            open: scope.create_rw_signal(false),
                                            children:
                                                scope.create_rw_signal(Vec::new()),
                                        },
                                    ))
                                }
                            }
                            root_item.update(|x| {
                                x.init = true;
                                x.children.update(|children| {
                                    *children = item_children;
                                })
                            });
                        }
                    }
                    Err(err) => {
                        tracing::error!("{:?}", err);
                    }
                    Ok(_) => {}
                }
            });
        self.common.proxy.call_hierarchy_incoming(
            path,
            item.get_untracked().item.as_ref().clone(),
            send,
        );
    }
}

/// Open path with the default application without blocking.
pub(super) fn open_uri(path: &Path) {
    match open::that(path) {
        Ok(_) => {
            debug!("opened active file: {path:?}");
        }
        Err(e) => {
            error!("failed to open active file: {path:?}, error: {e}");
        }
    }
}
