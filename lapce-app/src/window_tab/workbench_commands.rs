//! Workbench command handling of a window tab, including the font zoom commands.

use std::{collections::HashSet, env, time::Instant};

use floem::{
    action::open_file,
    ext_event::create_ext_action,
    file::FileDialogOptions,
    reactive::{Scope, SignalGet, SignalUpdate, SignalWith},
    views::editor::core::buffer::rope_text::RopeText,
};
use lapce_core::{directory::Directory, meta};
use lapce_rpc::{
    dap_types::{ConfigSource, RunDebugConfig},
    file::PathObject,
    proxy::ProxyResponse,
    source_control::FileDiff,
};
use serde_json::Value;
use tracing::{debug, error};

use super::{Focus, WindowTabData, open_uri};
use crate::{
    command::{InternalCommand, LapceWorkbenchCommand, WindowCommand},
    config::{LapceConfig, editor::DEFAULT_FONT_SIZE},
    debug::{DapData, RunDebugMode, RunDebugProcess},
    doc::DocContent,
    editor::location::{EditorLocation, EditorPosition},
    keypress::KeyPressFocus,
    palette::{DEFAULT_RUN_TOML, kind::PaletteKind},
    panel::{data::PanelSection, kind::PanelKind, position::PanelContainerPosition},
    tracing::*,
    workspace::{LapceWorkspace, LapceWorkspaceType},
};

impl WindowTabData {
    /// Moves the editor font size by `delta` points and persists it. The
    /// terminal size is pinned first so that it does not follow the editor.
    fn change_editor_font_size(&self, delta: i32) {
        let config = self.common.config.get_untracked();
        Self::pin_terminal_font_size(&config);
        Self::save_font_size("editor", config.editor.stepped_font_size(delta));
    }

    /// Moves the terminal font size by `delta` points and persists it.
    fn change_terminal_font_size(&self, delta: i32) {
        let config = self.common.config.get_untracked();
        Self::save_font_size("terminal", config.stepped_terminal_font_size(delta));
    }

    /// Stores the current terminal font size when it is inherited from the
    /// editor (`terminal.font-size = 0`), making the two independent.
    fn pin_terminal_font_size(config: &LapceConfig) {
        if config.terminal.font_size == 0 {
            Self::save_font_size("terminal", config.terminal_font_size());
        }
    }

    /// Writes `font-size` in the given settings section (`editor` or `terminal`).
    fn save_font_size(section: &str, size: usize) {
        LapceConfig::update_file(
            section,
            "font-size",
            toml_edit::Value::from(size as i64),
        );
    }

    /// Reads the font step from the command data (`1` when absent) and applies
    /// the given sign.
    fn font_step(data: &Option<Value>, sign: i32) -> i32 {
        let step = data.as_ref().and_then(Value::as_i64).unwrap_or(1);
        step as i32 * sign
    }

    pub fn run_workbench_command(
        &self,
        cmd: LapceWorkbenchCommand,
        data: Option<Value>,
    ) {
        use LapceWorkbenchCommand::*;
        match cmd {
            // ==== Modal ====
            EnableModal => {
                let internal_command = self.common.internal_command;
                internal_command.send(InternalCommand::SetModal { modal: true });
            }
            DisableModal => {
                let internal_command = self.common.internal_command;
                internal_command.send(InternalCommand::SetModal { modal: false });
            }

            // ==== Files / Folders ====
            OpenFolder => {
                if !self.workspace.kind.is_remote() {
                    let window_command = self.common.window_common.window_command;
                    let mut options = FileDialogOptions::new().title("Choose a folder").select_directories();
                    options = if let Some(parent) = self.workspace.path.as_ref().and_then(|x| x.parent()) {
                        options.force_starting_directory(parent)
                    } else {
                        options
                    };
                    open_file(options, move |file| {
                        if let Some(mut file) = file {
                            let workspace = LapceWorkspace {
                                kind: LapceWorkspaceType::Local,
                                path: Some(if let Some(path) = file.path.pop() {
                                    path
                                } else {
                                    tracing::error!("No path");
                                    return;
                                }),
                                last_open: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs(),
                            };
                            window_command
                                .send(WindowCommand::SetWorkspace { workspace });
                        }
                    });
                }
            }
            CloseFolder => {
                if !self.workspace.kind.is_remote() {
                    let window_command = self.common.window_common.window_command;
                    let workspace = LapceWorkspace {
                        kind: LapceWorkspaceType::Local,
                        path: None,
                        last_open: 0,
                    };
                    window_command.send(WindowCommand::SetWorkspace { workspace });
                }
            }
            OpenFile => {
                if !self.workspace.kind.is_remote() {
                    let internal_command = self.common.internal_command;
                    let options = FileDialogOptions::new().title("Choose a file");
                    open_file(options, move |file| {
                        if let Some(mut file) = file {
                            internal_command.send(InternalCommand::OpenFile {
                                path: if let Some(path) = file.path.pop() {
                                    path
                                } else {
                                    tracing::error!("No path");
                                    return;
                                },
                            })
                        }
                    });
                }
            }
            NewFile => {
                self.main_split.new_file();
            }
            RevealActiveFileInFileExplorer => {
                if let Some(editor_data) = self.main_split.active_editor.get() {
                    let doc = editor_data.doc();
                    let path = if let DocContent::File { path, .. } =
                        doc.content.get_untracked()
                    {
                        Some(path)
                    } else {
                        None
                    };
                    let Some(path) = path else { return };
                    let path = path.parent().unwrap_or(&path);

                    open_uri(path);
                }
            }

            SaveAll => {
                self.main_split.editors.with_editors_untracked(|editors| {
                    let mut paths = HashSet::new();
                    for (_, editor_data) in editors.iter() {
                        let doc = editor_data.doc();
                        let should_save = if let DocContent::File { path, .. } =
                            doc.content.get_untracked()
                        {
                            if paths.contains(&path) {
                                false
                            } else {
                                paths.insert(path.clone());

                                true
                            }
                        } else {
                            false
                        };

                        if should_save {
                            editor_data.save(true, || {});
                        }
                    }
                });
            }

            // ==== Configuration / Info Files and Folders ====
            OpenSettings => {
                self.main_split.open_settings();
            }
            OpenSettingsFile => {
                if let Some(path) = LapceConfig::settings_file() {
                    self.main_split.jump_to_location(
                        EditorLocation {
                            path,
                            position: None,
                            scroll_offset: None,
                            ignore_unconfirmed: false,
                            same_editor_tab: false,
                        },
                        None,
                    );
                }
            }
            OpenSettingsDirectory => {
                if let Some(dir) = Directory::config_directory() {
                    open_uri(&dir);
                }
            }
            OpenThemeColorSettings => {
                self.main_split.open_theme_color_settings();
            }
            OpenKeyboardShortcuts => {
                self.main_split.open_keymap();
            }
            OpenKeyboardShortcutsFile => {
                if let Some(path) = LapceConfig::keymaps_file() {
                    self.main_split.jump_to_location(
                        EditorLocation {
                            path,
                            position: None,
                            scroll_offset: None,
                            ignore_unconfirmed: false,
                            same_editor_tab: false,
                        },
                        None,
                    );
                }
            }
            OpenLogFile => {
                if let Some(dir) = Directory::logs_directory() {
                    self.open_paths(&[PathObject::from_path(
                        dir.join(format!(
                            "lapce.{}.log",
                            chrono::prelude::Local::now().format("%Y-%m-%d")
                        )),
                        false,
                    )])
                }
            }
            OpenLogsDirectory => {
                if let Some(dir) = Directory::logs_directory() {
                    open_uri(&dir);
                }
            }
            OpenProxyDirectory => {
                if let Some(dir) = Directory::proxy_directory() {
                    open_uri(&dir);
                }
            }
            OpenThemesDirectory => {
                if let Some(dir) = Directory::themes_directory() {
                    open_uri(&dir);
                }
            }
            OpenPluginsDirectory => {
                if let Some(dir) = Directory::plugins_directory() {
                    open_uri(&dir);
                }
            }
            OpenGrammarsDirectory => {
                if let Some(dir) = Directory::grammars_directory() {
                    open_uri(&dir);
                }
            }
            OpenQueriesDirectory => {
                if let Some(dir) = Directory::queries_directory() {
                    open_uri(&dir);
                }
            }

            InstallTheme => {}
            ExportCurrentThemeSettings => {
                self.main_split.export_theme();
            }
            ToggleInlayHints => {}

            // ==== Window ====
            ReloadWindow => {
                self.common.window_common.window_command.send(
                    WindowCommand::SetWorkspace {
                        workspace: (*self.workspace).clone(),
                    },
                );
            }
            NewWindow => {
                self.common
                    .window_common
                    .window_command
                    .send(WindowCommand::NewWindow);
            }
            CloseWindow => {
                self.common
                    .window_common
                    .window_command
                    .send(WindowCommand::CloseWindow);
            }
            // ==== Window Tabs ====
            NewWindowTab => {
                self.common.window_common.window_command.send(
                    WindowCommand::NewWorkspaceTab {
                        workspace: LapceWorkspace::default(),
                        end: false,
                    },
                );
            }
            CloseWindowTab => {
                self.common
                    .window_common
                    .window_command
                    .send(WindowCommand::CloseWorkspaceTab { index: None });
            }
            NextWindowTab => {
                self.common
                    .window_common
                    .window_command
                    .send(WindowCommand::NextWorkspaceTab);
            }
            PreviousWindowTab => {
                self.common
                    .window_common
                    .window_command
                    .send(WindowCommand::PreviousWorkspaceTab);
            }

            // ==== Editor Tabs ====
            NextEditorTab => {
                if let Some(editor_tab_id) =
                    self.main_split.active_editor_tab.get_untracked()
                {
                    self.main_split.editor_tabs.with_untracked(|editor_tabs| {
                        let Some(editor_tab) = editor_tabs.get(&editor_tab_id)
                        else {
                            return;
                        };

                        let new_index = editor_tab.with_untracked(|editor_tab| {
                            if editor_tab.children.is_empty() {
                                None
                            } else if editor_tab.active
                                == editor_tab.children.len() - 1
                            {
                                Some(0)
                            } else {
                                Some(editor_tab.active + 1)
                            }
                        });

                        if let Some(new_index) = new_index {
                            editor_tab.update(|editor_tab| {
                                editor_tab.active = new_index;
                            });
                        }
                    });
                }
            }
            PreviousEditorTab => {
                if let Some(editor_tab_id) =
                    self.main_split.active_editor_tab.get_untracked()
                {
                    self.main_split.editor_tabs.with_untracked(|editor_tabs| {
                        let Some(editor_tab) = editor_tabs.get(&editor_tab_id)
                        else {
                            return;
                        };

                        let new_index = editor_tab.with_untracked(|editor_tab| {
                            if editor_tab.children.is_empty() {
                                None
                            } else if editor_tab.active == 0 {
                                Some(editor_tab.children.len() - 1)
                            } else {
                                Some(editor_tab.active - 1)
                            }
                        });

                        if let Some(new_index) = new_index {
                            editor_tab.update(|editor_tab| {
                                editor_tab.active = new_index;
                            });
                        }
                    });
                }
            }

            // ==== Terminal ====
            NewTerminalTab => {
                self.terminal.new_tab(
                    self.common
                        .config
                        .get_untracked()
                        .terminal
                        .get_default_profile(),
                );
                if !self.panel.is_panel_visible(&PanelKind::Terminal) {
                    self.panel.show_panel(&PanelKind::Terminal);
                }
                self.common.focus.set(Focus::Panel(PanelKind::Terminal));
            }
            CloseTerminalTab => {
                self.terminal.close_tab(None);
                if self
                    .terminal
                    .tab_info
                    .with_untracked(|info| info.tabs.is_empty())
                {
                    if self.panel.is_panel_visible(&PanelKind::Terminal) {
                        self.panel.hide_panel(&PanelKind::Terminal);
                    }
                    self.common.focus.set(Focus::Workbench);
                } else {
                    if !self.panel.is_panel_visible(&PanelKind::Terminal) {
                        self.panel.show_panel(&PanelKind::Terminal);
                    }
                    self.common.focus.set(Focus::Panel(PanelKind::Terminal));
                }
            }
            NextTerminalTab => {
                self.terminal.next_tab();
                if !self.panel.is_panel_visible(&PanelKind::Terminal) {
                    self.panel.show_panel(&PanelKind::Terminal);
                }
                self.common.focus.set(Focus::Panel(PanelKind::Terminal));
            }
            PreviousTerminalTab => {
                self.terminal.previous_tab();
                if !self.panel.is_panel_visible(&PanelKind::Terminal) {
                    self.panel.show_panel(&PanelKind::Terminal);
                }
                self.common.focus.set(Focus::Panel(PanelKind::Terminal));
            }

            // ==== Remote ====
            ConnectSshHost => {
                self.palette.run(PaletteKind::SshHost);
            }
            #[cfg(windows)]
            ConnectWslHost => {
                self.palette.run(PaletteKind::WslHost);
            }
            DisconnectRemote => {
                self.common.window_common.window_command.send(
                    WindowCommand::SetWorkspace {
                        workspace: LapceWorkspace {
                            kind: LapceWorkspaceType::Local,
                            path: None,
                            last_open: 0,
                        },
                    },
                );
            }

            // ==== Palette Commands ====
            PaletteHelp => self.palette.run(PaletteKind::PaletteHelp),
            PaletteHelpAndFile => self.palette.run(PaletteKind::HelpAndFile),
            PaletteLine => {
                self.palette.run(PaletteKind::Line);
            }
            Palette => {
                self.palette.run(PaletteKind::File);
            }
            PaletteSymbol => {
                self.palette.run(PaletteKind::DocumentSymbol);
            }
            PaletteWorkspaceSymbol => {}
            PaletteCommand => {
                self.palette.run(PaletteKind::Command);
            }
            PaletteWorkspace => {
                self.palette.run(PaletteKind::Workspace);
            }
            PaletteRunAndDebug => {
                self.palette.run(PaletteKind::RunAndDebug);
            }
            PaletteSCMReferences => {
                self.palette.run(PaletteKind::SCMReferences);
            }
            ChangeColorTheme => {
                self.palette.run(PaletteKind::ColorTheme);
            }
            ChangeIconTheme => {
                self.palette.run(PaletteKind::IconTheme);
            }
            ChangeFileLanguage => {
                self.palette.run(PaletteKind::Language);
            }
            ChangeFileLineEnding => {
                self.palette.run(PaletteKind::LineEnding);
            }
            DiffFiles => self.palette.run(PaletteKind::DiffFiles),

            // ==== Running / Debugging ====
            RunAndDebugRestart => {
                let active_term = self.terminal.debug.active_term.get_untracked();
                if let Some(is_debug) = active_term
                    .and_then(|term_id| self.terminal.restart_run_debug(term_id))
                {
                    self.panel.show_panel(&PanelKind::Terminal);
                    if is_debug {
                        self.panel.show_panel(&PanelKind::Debug);
                    }
                } else {
                    self.palette.run(PaletteKind::RunAndDebug);
                }
            }
            RunAndDebugStop => {
                let active_term = self.terminal.debug.active_term.get_untracked();
                if let Some(term_id) = active_term {
                    self.terminal.stop_run_debug(term_id);
                }
            }

            // ==== UI ====
            ZoomIn => {
                let mut scale =
                    self.common.window_common.window_scale.get_untracked();
                scale += 0.1;
                if scale > 4.0 {
                    scale = 4.0
                }
                self.common.window_common.window_scale.set(scale);

                LapceConfig::update_file(
                    "ui",
                    "scale",
                    toml_edit::Value::from(scale),
                );
            }
            ZoomOut => {
                let mut scale =
                    self.common.window_common.window_scale.get_untracked();
                scale -= 0.1;
                if scale < 0.1 {
                    scale = 0.1
                }
                self.common.window_common.window_scale.set(scale);

                LapceConfig::update_file(
                    "ui",
                    "scale",
                    toml_edit::Value::from(scale),
                );
            }
            ZoomReset => {
                self.common.window_common.window_scale.set(1.0);

                LapceConfig::update_file(
                    "ui",
                    "scale",
                    toml_edit::Value::from(1.0),
                );
            }
            EditorFontIncrease => {
                self.change_editor_font_size(Self::font_step(&data, 1))
            }
            EditorFontDecrease => {
                self.change_editor_font_size(Self::font_step(&data, -1))
            }
            EditorFontReset => {
                Self::pin_terminal_font_size(&self.common.config.get_untracked());
                Self::save_font_size("editor", DEFAULT_FONT_SIZE);
            }
            TerminalFontIncrease => {
                self.change_terminal_font_size(Self::font_step(&data, 1))
            }
            TerminalFontDecrease => {
                self.change_terminal_font_size(Self::font_step(&data, -1))
            }
            TerminalFontReset => Self::save_font_size("terminal", 0),

            ToggleMaximizedPanel => {
                if let Some(data) = data {
                    if let Ok(kind) = serde_json::from_value::<PanelKind>(data) {
                        self.panel.toggle_maximize(&kind);
                    }
                } else {
                    self.panel.toggle_active_maximize();
                }
            }
            HidePanel => {
                if let Some(data) = data {
                    if let Ok(kind) = serde_json::from_value::<PanelKind>(data) {
                        self.hide_panel(kind);
                    }
                }
            }
            ShowPanel => {
                if let Some(data) = data {
                    if let Ok(kind) = serde_json::from_value::<PanelKind>(data) {
                        self.show_panel(kind);
                    }
                }
            }
            TogglePanelFocus => {
                if let Some(data) = data {
                    if let Ok(kind) = serde_json::from_value::<PanelKind>(data) {
                        self.toggle_panel_focus(kind);
                    }
                }
            }
            TogglePanelVisual => {
                if let Some(data) = data {
                    if let Ok(kind) = serde_json::from_value::<PanelKind>(data) {
                        self.toggle_panel_visual(kind);
                    }
                }
            }
            TogglePanelLeftVisual => {
                self.toggle_container_visual(&PanelContainerPosition::Left);
            }
            TogglePanelRightVisual => {
                self.toggle_container_visual(&PanelContainerPosition::Right);
            }
            TogglePanelBottomVisual => {
                self.toggle_container_visual(&PanelContainerPosition::Bottom);
            }
            ToggleTerminalFocus => {
                self.toggle_panel_focus(PanelKind::Terminal);
            }
            ToggleSourceControlFocus => {
                self.toggle_panel_focus(PanelKind::SourceControl);
            }
            TogglePluginFocus => {
                self.toggle_panel_focus(PanelKind::Plugin);
            }
            ToggleFileExplorerFocus => {
                self.toggle_panel_focus(PanelKind::FileExplorer);
            }
            ToggleProblemFocus => {
                self.toggle_panel_focus(PanelKind::Problem);
            }
            ToggleAgentFocus => {
                self.toggle_panel_focus(PanelKind::Agent);
            }
            ToggleSearchFocus => {
                self.toggle_panel_focus(PanelKind::Search);
            }
            ToggleTerminalVisual => {
                self.toggle_panel_visual(PanelKind::Terminal);
            }
            ToggleSourceControlVisual => {
                self.toggle_panel_visual(PanelKind::SourceControl);
            }
            TogglePluginVisual => {
                self.toggle_panel_visual(PanelKind::Plugin);
            }
            ToggleFileExplorerVisual => {
                self.toggle_panel_visual(PanelKind::FileExplorer);
            }
            ToggleProblemVisual => {
                self.toggle_panel_visual(PanelKind::Problem);
            }
            ToggleAgentVisual => {
                self.toggle_panel_visual(PanelKind::Agent);
            }
            ToggleDebugVisual => {
                self.toggle_panel_visual(PanelKind::Debug);
            }
            ToggleSearchVisual => {
                self.toggle_panel_visual(PanelKind::Search);
            }
            FocusEditor => {
                self.common.focus.set(Focus::Workbench);
            }
            FocusTerminal => {
                self.common.focus.set(Focus::Panel(PanelKind::Terminal));
            }
            OpenUIInspector => {
                self.common.view_id.get_untracked().inspect();
            }
            ShowEnvironment => {
                self.main_split.show_env();
            }

            // ==== Source Control ====
            SourceControlInit => {
                self.proxy.proxy_rpc.git_init();
            }
            CheckoutReference => match data {
                Some(reference) => {
                    if let Some(reference) = reference.as_str() {
                        self.proxy.proxy_rpc.git_checkout(reference.to_string());
                    }
                }
                None => error!("No ref provided"),
            },
            SourceControlCommit => {
                self.source_control.commit();
            }
            SourceControlCopyActiveFileRemoteUrl => {}
            SourceControlDiscardActiveFileChanges => {}
            SourceControlDiscardTargetFileChanges => {
                if let Some(diff) = data
                    .and_then(|data| serde_json::from_value::<FileDiff>(data).ok())
                {
                    match diff {
                        FileDiff::Added(path) => {
                            self.common.proxy.trash_path(path, Box::new(|_| {}));
                        }
                        FileDiff::Modified(path) | FileDiff::Deleted(path) => {
                            self.common.proxy.git_discard_files_changes(vec![path]);
                        }
                        FileDiff::Renamed(old_path, new_path) => {
                            self.common
                                .proxy
                                .git_discard_files_changes(vec![old_path]);
                            self.common.proxy.trash_path(new_path, Box::new(|_| {}));
                        }
                    }
                }
            }
            SourceControlDiscardWorkspaceChanges => {}

            // ==== UI ====
            ShowAbout => {
                self.about_data.open();
            }

            // ==== Updating ====
            RestartToUpdate => {
                if let Some(release) = self
                    .common
                    .window_common
                    .latest_release
                    .get_untracked()
                    .as_ref()
                {
                    let release = release.clone();
                    let update_in_progress = self.update_in_progress;
                    if release.version != *meta::VERSION {
                        if let Ok(process_path) = env::current_exe() {
                            update_in_progress.set(true);
                            let send = create_ext_action(
                                self.common.scope,
                                move |_started| {
                                    update_in_progress.set(false);
                                },
                            );
                            std::thread::Builder::new().name("RestartToUpdate".to_owned()).spawn(move || {
                                let do_update = || -> anyhow::Result<()> {
                                    let src =
                                        crate::update::download_release(&release)?;

                                    let path =
                                        crate::update::extract(&src, &process_path)?;

                                    crate::update::restart(&path)?;

                                    Ok(())
                                };

                                if let Err(err) = do_update() {
                                    error!("Failed to update: {err}");
                                }

                                send(false);
                            }).unwrap();
                        }
                    }
                }
            }

            // ==== Movement ====
            #[cfg(target_os = "macos")]
            InstallToPATH => {
                self.common.internal_command.send(
                    InternalCommand::ExecuteProcess {
                        program: String::from("osascript"),
                        arguments: vec![String::from("-e"), format!(r#"do shell script "ln -sf '{}' /usr/local/bin/lapce" with administrator privileges"#, std::env::args().next().unwrap())],
                    }
                )
            }
            #[cfg(target_os = "macos")]
            UninstallFromPATH => {
                self.common.internal_command.send(
                    InternalCommand::ExecuteProcess {
                        program: String::from("osascript"),
                        arguments: vec![String::from("-e"), String::from(r#"do shell script "rm /usr/local/bin/lapce" with administrator privileges"#)],
                    }
                )
            }
            JumpLocationForward => {
                self.main_split.jump_location_forward(false);
            }
            JumpLocationBackward => {
                self.main_split.jump_location_backward(false);
            }
            JumpLocationForwardLocal => {
                self.main_split.jump_location_forward(true);
            }
            JumpLocationBackwardLocal => {
                self.main_split.jump_location_backward(true);
            }
            NextError => {
                self.main_split.next_error();
            }
            PreviousError => {}
            Quit => {
                floem::quit_app();
            }
            RevealInPanel => {
                if let Some(editor_data) =
                    self.main_split.active_editor.get_untracked()
                {
                    self.show_panel(PanelKind::FileExplorer);
                    self.panel
                        .section_open(PanelSection::FileExplorer).update(|x| {
                        *x = true;
                    });
                    if let DocContent::File {path, ..} = editor_data.doc().content.get_untracked() {
                        self.file_explorer.reveal_in_file_tree(path);
                    }
                }
            }
            SourceControlOpenActiveFileRemoteUrl => {
                if let Some(editor_data) =
                    self.main_split.active_editor.get_untracked()
                {
                    if let DocContent::File {path, ..} = editor_data.doc().content.get_untracked() {
                        let offset = editor_data.cursor().with_untracked(|c| c.offset());
                        let line = editor_data.doc()
                            .buffer
                            .with_untracked(|buffer| buffer.line_of_offset(offset));
                        self.common.proxy.git_get_remote_file_url(
                            path,
                            create_ext_action(self.scope, move |result| {
                                if let Ok(ProxyResponse::GitGetRemoteFileUrl {
                                              file_url
                                          }) = result
                                {
                                    if let Err(err) = open::that(format!("{}#L{}", file_url, line)) {
                                        error!("Failed to open file in github: {}",  err);
                                    }
                                }
                            }),
                        );

                    }
                }
            }
            RevealInFileExplorer => {
                if let Some(editor_data) =
                    self.main_split.active_editor.get_untracked()
                {
                    if let DocContent::File {path, ..} = editor_data.doc().content.get_untracked() {
                        let path = path.parent().unwrap_or(&path);
                        if !path.exists() {
                            return;
                        }
                        if let Err(err) = open::that(path) {
                            error!(
                            "Failed to reveal file in system file explorer: {}",
                            err
                        );
                        }
                    }
                }
            }
            ShowCallHierarchy => {
                if let Some(editor_data) =
                    self.main_split.active_editor.get_untracked()
                {
                    editor_data.call_hierarchy(self.clone());
                }
            }
            FindReferences => {
                if let Some(editor_data) =
                    self.main_split.active_editor.get_untracked()
                {
                    editor_data.find_reference(self.clone());
                }
            }
            GoToImplementation => {
                if let Some(editor_data) =
                    self.main_split.active_editor.get_untracked()
                {
                    editor_data.go_to_implementation(self.clone());
                }
            }
            RunInTerminal => {
                if let Some(editor_data) =
                    self.main_split.active_editor.get_untracked()
                {
                    let name = editor_data.word_at_cursor();
                    if !name.is_empty() {
                        let mut args_str = name.split(" ");
                        let program = args_str.next().map(|x| x.to_string()).unwrap();
                        let args: Vec<String> = args_str.map(|x| x.to_string()).collect();
                        let args = if args.is_empty() {
                            None
                        } else {
                            Some(args)
                        };

                        let config = RunDebugConfig {
                            ty: None,
                            name,
                            program,
                            args,
                            cwd: None,
                            env: None,
                            prelaunch: None,
                            debug_command: None,
                            dap_id: Default::default(),
                            tracing_output: false,
                            config_source: ConfigSource::RunInTerminal,
                        };
                        self.common
                            .internal_command
                            .send(InternalCommand::RunAndDebug { mode: RunDebugMode::Run, config });
                    }
                }
            }
            GoToLocation => {
                if let Some(editor_data) =
                    self.main_split.active_editor.get_untracked()
                {
                    let doc = editor_data.doc();
                    let path = match if doc.loaded() {
                        doc.content.with_untracked(|c| c.path().cloned())
                    } else {
                        None
                    } {
                        Some(path) => path,
                        None => return,
                    };
                    let offset = editor_data.cursor().with_untracked(|c| c.offset());
                    let internal_command = self.common.internal_command;

                    internal_command.send(InternalCommand::MakeConfirmed);
                    internal_command.send(InternalCommand::GoToLocation { location: EditorLocation {
                        path,
                        position: Some(EditorPosition::Offset(offset)),
                        scroll_offset: None,
                        ignore_unconfirmed: false,
                        same_editor_tab: false,
                    } });
                }
            }
            AddRunDebugConfig => {
                if let Some(editor_data) =
                    self.main_split.active_editor.get_untracked()
                {
                    editor_data.receive_char(DEFAULT_RUN_TOML);
                }
            }

        }
    }
}

impl WindowTabData {
    pub(super) fn run_and_debug(
        &self,
        cx: Scope,
        mode: &RunDebugMode,
        config: &RunDebugConfig,
    ) {
        debug!("{:?}", config);
        match mode {
            RunDebugMode::Run => {
                self.run_in_terminal(cx, mode, config, false);
            }
            RunDebugMode::Debug => {
                if config.prelaunch.is_some() {
                    self.run_in_terminal(cx, mode, config, false);
                } else {
                    self.common.proxy.dap_start(
                        config.clone(),
                        self.terminal.debug.source_breakpoints(),
                    )
                };
                if !self.panel.is_panel_visible(&PanelKind::Debug) {
                    self.panel.show_panel(&PanelKind::Debug);
                }
            }
        }
    }

    pub(super) fn run_in_terminal(
        &self,
        cx: Scope,
        mode: &RunDebugMode,
        config: &RunDebugConfig,
        from_dap: bool,
    ) {
        // if not from dap, then run prelaunch first
        let is_prelaunch = !from_dap;
        let term_id = if let Some(terminal) =
            self.terminal.get_stopped_run_debug_terminal(mode, config)
        {
            terminal.new_process(Some(RunDebugProcess {
                mode: *mode,
                config: config.clone(),
                stopped: false,
                created: Instant::now(),
                is_prelaunch,
            }));

            terminal.term_id
        } else {
            let new_terminal_tab = self.terminal.new_tab_run_debug(
                Some(RunDebugProcess {
                    mode: *mode,
                    config: config.clone(),
                    stopped: false,
                    created: Instant::now(),
                    is_prelaunch,
                }),
                None,
            );
            new_terminal_tab.active_terminal(false).unwrap().term_id
        };
        self.common.focus.set(Focus::Panel(PanelKind::Terminal));
        self.terminal.focus_terminal(term_id);

        self.terminal.debug.active_term.set(Some(term_id));
        self.terminal.debug.daps.update(|daps| {
            daps.insert(
                config.dap_id,
                DapData::new(cx, config.dap_id, term_id, self.common.clone()),
            );
        });

        if !self.panel.is_panel_visible(&PanelKind::Terminal) {
            self.panel.show_panel(&PanelKind::Terminal);
        }
    }
}
