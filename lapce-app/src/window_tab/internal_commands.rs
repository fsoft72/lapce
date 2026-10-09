//! Internal command handling of a window tab.

use std::sync::Arc;

use alacritty_terminal::vte::ansi::Handler;
use floem::{
    ext_event::create_ext_action,
    reactive::{SignalGet, SignalUpdate, SignalWith},
};
use lapce_rpc::{RpcError, file::Naming, proxy::ProxyResponse};
use tracing::{Level, debug, error, event};

use super::WindowTabData;
use crate::{
    command::InternalCommand, config::LapceConfig, doc::DocContent,
    editor::location::EditorLocation, keypress::KeyPressFocus,
    palette::kind::PaletteKind, panel::kind::PanelKind, tracing::*,
};

impl WindowTabData {
    pub fn run_internal_command(&self, cmd: InternalCommand) {
        let cx = self.scope;
        match cmd {
            InternalCommand::ReloadConfig => {
                self.reload_config();
            }
            InternalCommand::UpdateLogLevel { level } => {
                // TODO: implement logging panel, runtime log level change
                debug!("{level}");
            }
            InternalCommand::MakeConfirmed => {
                if let Some(editor) = self.main_split.active_editor.get_untracked() {
                    editor.confirmed.set(true);
                }
            }
            InternalCommand::OpenFile { path } => {
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
            InternalCommand::OpenAndConfirmedFile { path } => {
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
                if let Some(editor) = self.main_split.active_editor.get_untracked() {
                    editor.confirmed.set(true);
                }
            }
            InternalCommand::OpenFileInNewTab { path } => {
                self.main_split.jump_to_location(
                    EditorLocation {
                        path,
                        position: None,
                        scroll_offset: None,
                        ignore_unconfirmed: true,
                        same_editor_tab: false,
                    },
                    None,
                );
            }
            InternalCommand::OpenFileChanges { path } => {
                self.main_split.open_file_changes(path);
            }
            InternalCommand::ReloadFileExplorer => {
                self.file_explorer.reload();
            }
            InternalCommand::TestPathCreation { new_path } => {
                let naming = self.file_explorer.naming;

                let send = create_ext_action(
                    self.scope,
                    move |response: Result<ProxyResponse, RpcError>| match response {
                        Ok(_) => {
                            naming.update(Naming::set_ok);
                        }
                        Err(err) => {
                            naming.update(|naming| naming.set_err(err.message));
                        }
                    },
                );

                self.common.proxy.test_create_at_path(new_path, send);
            }
            InternalCommand::FinishRenamePath {
                current_path,
                new_path,
            } => {
                let send_current_path = current_path.clone();
                let send_new_path = new_path.clone();
                let file_explorer = self.file_explorer.clone();
                let editors = self.main_split.editors;

                let send = create_ext_action(
                    self.scope,
                    move |response: Result<ProxyResponse, RpcError>| match response {
                        Ok(response) => {
                            // Get the canonicalized new path from the proxy.
                            let new_path =
                                if let ProxyResponse::CreatePathResponse { path } =
                                    response
                                {
                                    path
                                } else {
                                    send_new_path
                                };

                            // If the renamed item is a file, update any editors the file is open
                            // in to use the new path.
                            // If the renamed item is a directory, update any editors in which a
                            // file the renamed directory is an ancestor of is open to use the
                            // file's new path.
                            let renamed_editors_content: Vec<_> = editors
                                .with_editors_untracked(|editors| {
                                    editors
                                        .values()
                                        .map(|editor| editor.doc().content)
                                        .filter(|content| {
                                            content.with_untracked(|content| {
                                                match content {
                                                    DocContent::File {
                                                        path,
                                                        ..
                                                    } => path.starts_with(
                                                        &send_current_path,
                                                    ),
                                                    _ => false,
                                                }
                                            })
                                        })
                                        .collect()
                                });

                            for content in renamed_editors_content {
                                content.update(|content| {
                                    if let DocContent::File { path, .. } = content {
                                        if let Ok(suffix) =
                                            path.strip_prefix(&send_current_path)
                                        {
                                            *path = new_path.join(suffix);
                                        }
                                    }
                                });
                            }

                            file_explorer.reload();
                            file_explorer.naming.set(Naming::None);
                        }
                        Err(err) => {
                            file_explorer
                                .naming
                                .update(|naming| naming.set_err(err.message));
                        }
                    },
                );

                self.file_explorer.naming.update(Naming::set_pending);
                self.common
                    .proxy
                    .rename_path(current_path.clone(), new_path, send);
            }
            InternalCommand::FinishNewNode { is_dir, path } => {
                let file_explorer = self.file_explorer.clone();
                let internal_command = self.common.internal_command;

                let send = create_ext_action(
                    self.scope,
                    move |response: Result<ProxyResponse, RpcError>| {
                        match response {
                            Ok(response) => {
                                file_explorer.reload();
                                file_explorer.naming.set(Naming::None);

                                // Open a new file in the editor
                                if let ProxyResponse::CreatePathResponse { path } =
                                    response
                                {
                                    if !is_dir {
                                        internal_command.send(
                                            InternalCommand::OpenFile { path },
                                        );
                                    }
                                }
                            }
                            Err(err) => {
                                file_explorer
                                    .naming
                                    .update(|naming| naming.set_err(err.message));
                            }
                        }
                    },
                );

                self.file_explorer.naming.update(Naming::set_pending);
                if is_dir {
                    self.common.proxy.create_directory(path, send);
                } else {
                    self.common.proxy.create_file(path, send);
                }
            }
            InternalCommand::FinishDuplicate { source, path } => {
                let file_explorer = self.file_explorer.clone();

                let send = create_ext_action(
                    self.scope,
                    move |response: Result<_, RpcError>| {
                        if let Err(err) = response {
                            file_explorer
                                .naming
                                .update(|naming| naming.set_err(err.message));
                        } else {
                            file_explorer.reload();
                            file_explorer.naming.set(Naming::None);
                        }
                    },
                );

                self.file_explorer.naming.update(Naming::set_pending);
                self.common.proxy.duplicate_path(source, path, send);
            }
            InternalCommand::GoToLocation { location } => {
                self.main_split.go_to_location(location, None);
            }
            InternalCommand::JumpToLocation { location } => {
                self.main_split.jump_to_location(location, None);
            }
            InternalCommand::PaletteReferences { references } => {
                self.palette.references.set(references);
                self.palette.run(PaletteKind::Reference);
            }
            InternalCommand::Split {
                direction,
                editor_tab_id,
            } => {
                self.main_split.split(direction, editor_tab_id);
            }
            InternalCommand::SplitMove {
                direction,
                editor_tab_id,
            } => {
                self.main_split.split_move(direction, editor_tab_id);
            }
            InternalCommand::SplitExchange { editor_tab_id } => {
                self.main_split.split_exchange(editor_tab_id);
            }
            InternalCommand::EditorTabClose { editor_tab_id } => {
                self.main_split.editor_tab_close(editor_tab_id);
            }
            InternalCommand::EditorTabChildClose {
                editor_tab_id,
                child,
            } => {
                self.main_split
                    .editor_tab_child_close(editor_tab_id, child, false);
            }
            InternalCommand::EditorTabCloseByKind {
                editor_tab_id,
                child,
                kind,
            } => {
                self.main_split.editor_tab_child_close_by_kind(
                    editor_tab_id,
                    child,
                    kind,
                );
            }
            InternalCommand::ShowCodeActions {
                offset,
                mouse_click,
                plugin_id,
                code_actions,
            } => {
                let mut code_action = self.code_action.get_untracked();
                code_action.show(plugin_id, code_actions, offset, mouse_click);
                self.code_action.set(code_action);
            }
            InternalCommand::RunCodeAction { plugin_id, action } => {
                self.main_split.run_code_action(plugin_id, action);
            }
            InternalCommand::ApplyWorkspaceEdit { edit } => {
                self.main_split.apply_workspace_edit(&edit);
            }
            InternalCommand::SaveJumpLocation {
                path,
                offset,
                scroll_offset,
            } => {
                self.main_split
                    .save_jump_location(path, offset, scroll_offset);
            }
            InternalCommand::NewTerminal { profile } => {
                self.terminal.new_tab(profile);
            }
            InternalCommand::SplitTerminal { term_id } => {
                self.terminal.split(term_id);
            }
            InternalCommand::SplitTerminalNext { term_id } => {
                self.terminal.split_next(term_id);
            }
            InternalCommand::SplitTerminalPrevious { term_id } => {
                self.terminal.split_previous(term_id);
            }
            InternalCommand::SplitTerminalExchange { term_id } => {
                self.terminal.split_exchange(term_id);
            }
            InternalCommand::RunAndDebug { mode, config } => {
                self.run_and_debug(cx, &mode, &config);
            }
            InternalCommand::StartRename {
                path,
                placeholder,
                position,
                start,
            } => {
                self.rename.start(path, placeholder, start, position);
            }
            InternalCommand::Search { pattern } => {
                self.main_split.set_find_pattern(pattern);
            }
            InternalCommand::FindEditorReceiveChar { s } => {
                self.main_split.find_editor.receive_char(&s);
            }
            InternalCommand::ReplaceEditorReceiveChar { s } => {
                self.main_split.replace_editor.receive_char(&s);
            }
            InternalCommand::FindEditorCommand {
                command,
                count,
                mods,
            } => {
                self.main_split
                    .find_editor
                    .run_command(&command, count, mods);
            }
            InternalCommand::ReplaceEditorCommand {
                command,
                count,
                mods,
            } => {
                self.main_split
                    .replace_editor
                    .run_command(&command, count, mods);
            }
            InternalCommand::FocusEditorTab { editor_tab_id } => {
                self.main_split.active_editor_tab.set(Some(editor_tab_id));
            }
            InternalCommand::SetColorTheme { name, save } => {
                if save {
                    // The config file is watched
                    LapceConfig::update_file(
                        "core",
                        "color-theme",
                        toml_edit::Value::from(name),
                    );
                } else {
                    let mut new_config = self.common.config.get_untracked();
                    Arc::make_mut(&mut new_config)
                        .set_color_theme(&self.workspace, &name);
                    self.set_config.set(new_config);
                }
            }
            InternalCommand::SetIconTheme { name, save } => {
                if save {
                    // The config file is watched
                    LapceConfig::update_file(
                        "core",
                        "icon-theme",
                        toml_edit::Value::from(name),
                    );
                } else {
                    let mut new_config = self.common.config.get_untracked();
                    Arc::make_mut(&mut new_config)
                        .set_icon_theme(&self.workspace, &name);
                    self.set_config.set(new_config);
                }
            }
            InternalCommand::SetModal { modal } => {
                LapceConfig::update_file(
                    "core",
                    "modal",
                    toml_edit::Value::from(modal),
                );
            }
            InternalCommand::OpenWebUri { uri } => {
                if !uri.is_empty() {
                    match open::that(&uri) {
                        Ok(_) => {
                            trace!(TraceLevel::TRACE, "opened web uri: {uri:?}");
                        }
                        Err(e) => {
                            trace!(
                                TraceLevel::ERROR,
                                "failed to open web uri: {uri:?}, error: {e}"
                            );
                        }
                    }
                }
            }
            InternalCommand::ShowAlert {
                title,
                msg,
                buttons,
            } => {
                self.show_alert(title, msg, buttons);
            }
            InternalCommand::HideAlert => {
                self.alert_data.active.set(false);
            }
            InternalCommand::SaveScratchDoc { doc } => {
                self.main_split.save_scratch_doc(doc);
            }
            InternalCommand::SaveScratchDoc2 { doc } => {
                self.main_split.save_scratch_doc2(doc);
            }
            InternalCommand::UpdateProxyStatus { status } => {
                self.common.proxy_status.set(Some(status));
            }
            InternalCommand::DapFrameScopes { dap_id, frame_id } => {
                self.terminal.dap_frame_scopes(dap_id, frame_id);
            }
            InternalCommand::OpenVoltView { volt_id } => {
                self.main_split.open_volt_view(volt_id);
            }
            InternalCommand::ResetBlinkCursor => {
                // All the editors share the blinking information and logic, so we can just reset
                // one of them.
                if let Some(e_data) = self.main_split.active_editor.get_untracked() {
                    e_data.editor.cursor_info.reset();
                }
            }
            InternalCommand::OpenDiffFiles {
                left_path,
                right_path,
            } => self.main_split.open_diff_files(left_path, right_path),
            InternalCommand::ExecuteProcess { program, arguments } => {
                let mut cmd = match std::process::Command::new(program)
                    .args(arguments)
                    .spawn()
                {
                    Ok(v) => v,
                    Err(e) => {
                        return event!(Level::ERROR, "Failed to spawn process: {e}");
                    }
                };

                match cmd.wait() {
                    Ok(v) => event!(Level::TRACE, "Process exited with status {v}"),
                    Err(e) => {
                        event!(Level::ERROR, "Process exited with an error: {e}")
                    }
                };
            }
            InternalCommand::ClearTerminalBuffer {
                view_id,
                tab_index,
                terminal_index,
            } => {
                let Some(tab) = self.terminal.tab_info.with_untracked(|x| {
                    x.tabs.iter().find_map(|(index, data)| {
                        if index.get_untracked() == tab_index {
                            Some(data.clone())
                        } else {
                            None
                        }
                    })
                }) else {
                    error!("could not find terminal tab data: index={tab_index}");
                    return;
                };
                let Some(raw) = tab.terminals.with_untracked(|x| {
                    x.iter().find_map(|(index, data)| {
                        if index.get_untracked() == terminal_index {
                            Some(data.raw.get_untracked())
                        } else {
                            None
                        }
                    })
                }) else {
                    error!("could not find terminal data: index={terminal_index}");
                    return;
                };
                raw.write().term.reset_state();
                view_id.request_paint();
            }
            InternalCommand::StopTerminal { term_id } => {
                self.terminal.stop_run_debug(term_id);
            }
            InternalCommand::RestartTerminal { term_id } => {
                if let Some(is_debug) = self.terminal.restart_run_debug(term_id) {
                    self.panel.show_panel(&PanelKind::Terminal);
                    if is_debug {
                        self.panel.show_panel(&PanelKind::Debug);
                    }
                } else {
                    self.palette.run(PaletteKind::RunAndDebug);
                }
            }
            InternalCommand::CallHierarchyIncoming { item_id } => {
                self.call_hierarchy_incoming(item_id);
            }
        }
    }
}
