//! Core notifications, alerts, progress and messages of a window tab.

use std::collections::HashSet;

use floem::reactive::{SignalGet, SignalUpdate, SignalWith};
use itertools::Itertools;
use lapce_rpc::core::CoreNotification;
use lsp_types::{Diagnostic, ProgressParams, ShowMessageParams};
use tracing::debug;

use super::{Focus, WindowTabData, WorkProgress};
use crate::{
    alert::AlertButton, debug::RunDebugMode, lsp::path_from_url,
    panel::kind::PanelKind, terminal::event::TermEvent, tracing::*,
};

impl WindowTabData {
    pub(super) fn handle_core_notification(&self, rpc: &CoreNotification) {
        let cx = self.scope;
        match rpc {
            CoreNotification::ProxyStatus { status } => {
                self.common.proxy_status.set(Some(status.to_owned()));
            }
            CoreNotification::DiffInfo { diff } => {
                self.source_control.branch.set(diff.head.clone());
                self.source_control
                    .branches
                    .set(diff.branches.iter().cloned().collect());
                self.source_control
                    .tags
                    .set(diff.tags.iter().cloned().collect());
                self.source_control.file_diffs.update(|file_diffs| {
                    *file_diffs = diff
                        .diffs
                        .iter()
                        .cloned()
                        .map(|diff| {
                            let checked =
                                file_diffs.get(diff.path()).is_none_or(|(_, c)| *c);
                            (diff.path().clone(), (diff, checked))
                        })
                        .collect();
                });

                let docs = self.main_split.docs.get_untracked();
                for (_, doc) in docs {
                    doc.retrieve_head();
                }
            }
            CoreNotification::CompletionResponse {
                request_id,
                input,
                resp,
                plugin_id,
            } => {
                self.common.completion.update(|completion| {
                    completion.receive(*request_id, input, resp, *plugin_id);
                });

                let completion = self.common.completion.get_untracked();
                let editor_data = completion
                    .latest_editor_id
                    .and_then(|id| self.main_split.editors.editor_untracked(id));
                if let Some(editor_data) = editor_data {
                    let cursor_offset =
                        editor_data.cursor().with_untracked(|c| c.offset());
                    completion
                        .update_document_completion(&editor_data, cursor_offset);
                }
            }
            CoreNotification::PublishDiagnostics { diagnostics } => {
                let path = path_from_url(&diagnostics.uri);
                let diagnostics: im::Vector<Diagnostic> = diagnostics
                    .diagnostics
                    .clone()
                    .into_iter()
                    .sorted_by_key(|d| d.range.start)
                    .collect();

                self.main_split
                    .get_diagnostic_data(&path)
                    .diagnostics
                    .set(diagnostics);

                // inform the document about the diagnostics
                if let Some(doc) = self
                    .main_split
                    .docs
                    .with_untracked(|docs| docs.get(&path).cloned())
                {
                    doc.init_diagnostics();
                }
            }
            CoreNotification::ServerStatus { params } => {
                if params.is_ok() {
                    // todo filter by language
                    self.main_split.docs.with_untracked(|x| {
                        for doc in x.values() {
                            doc.get_code_lens();
                            doc.get_document_symbol();
                            doc.get_semantic_styles();
                            doc.get_folding_range();
                            doc.get_inlay_hints();
                        }
                    });
                }
            }
            CoreNotification::TerminalProcessStopped { term_id, exit_code } => {
                debug!("TerminalProcessStopped {:?}, {:?}", term_id, exit_code);
                if let Err(err) = self
                    .common
                    .term_tx
                    .send((*term_id, TermEvent::CloseTerminal))
                {
                    tracing::error!("{:?}", err);
                }
                self.terminal.terminal_stopped(term_id, *exit_code);
                if self
                    .terminal
                    .tab_info
                    .with_untracked(|info| info.tabs.is_empty())
                {
                    if self.panel.is_panel_visible(&PanelKind::Terminal) {
                        self.panel.hide_panel(&PanelKind::Terminal);
                    }
                    self.common.focus.set(Focus::Workbench);
                }
            }
            CoreNotification::TerminalLaunchFailed { term_id, error } => {
                self.terminal.launch_failed(term_id, error);
            }
            CoreNotification::RunInTerminal { config } => {
                self.run_in_terminal(cx, &RunDebugMode::Debug, config, true);
            }
            CoreNotification::TerminalProcessId {
                term_id,
                process_id,
            } => {
                self.terminal.set_process_id(term_id, *process_id);
            }
            CoreNotification::DapStopped {
                dap_id,
                stopped,
                stack_frames,
                variables,
            } => {
                self.show_panel(PanelKind::Debug);
                self.terminal
                    .dap_stopped(dap_id, stopped, stack_frames, variables);
            }
            CoreNotification::OpenPaths { paths } => {
                self.open_paths(paths);
            }
            CoreNotification::DapContinued { dap_id } => {
                self.terminal.dap_continued(dap_id);
            }
            CoreNotification::DapBreakpointsResp {
                path, breakpoints, ..
            } => {
                self.terminal.debug.breakpoints.update(|all_breakpoints| {
                    if let Some(current_breakpoints) = all_breakpoints.get_mut(path)
                    {
                        let mut line_changed = HashSet::new();
                        let mut i = 0;
                        for (_, current_breakpoint) in current_breakpoints.iter_mut()
                        {
                            if !current_breakpoint.active {
                                continue;
                            }
                            if let Some(breakpoint) = breakpoints.get(i) {
                                current_breakpoint.id = breakpoint.id;
                                current_breakpoint.verified = breakpoint.verified;
                                current_breakpoint
                                    .message
                                    .clone_from(&breakpoint.message);
                                if let Some(new_line) = breakpoint.line {
                                    if current_breakpoint.line + 1 != new_line {
                                        line_changed.insert(current_breakpoint.line);
                                        current_breakpoint.line =
                                            new_line.saturating_sub(1);
                                    }
                                }
                            }
                            i += 1;
                        }
                        for line in line_changed {
                            if let Some(changed) = current_breakpoints.remove(&line)
                            {
                                current_breakpoints.insert(changed.line, changed);
                            }
                        }
                    }
                });
            }
            CoreNotification::OpenFileChanged { path, content } => {
                self.main_split.open_file_changed(path, content);
            }
            CoreNotification::VoltInstalled { volt, icon } => {
                self.plugin.volt_installed(volt, icon);
            }
            CoreNotification::VoltRemoved { volt, .. } => {
                self.plugin.volt_removed(volt);
            }
            CoreNotification::WorkDoneProgress { progress } => {
                self.update_progress(progress);
            }
            CoreNotification::ShowMessage { title, message } => {
                self.show_message(title, message);
            }
            CoreNotification::Log {
                level,
                message,
                target,
            } => {
                use lapce_rpc::core::LogLevel;
                use tracing_log::log::{Level, log};

                let target = target.clone().unwrap_or(String::from("unknown"));

                match level {
                    LogLevel::Trace => {
                        log!(target: &target, Level::Trace, "{}", message);
                    }
                    LogLevel::Debug => {
                        log!(target: &target, Level::Debug, "{}", message);
                    }
                    LogLevel::Info => {
                        log!(target: &target, Level::Info, "{}", message);
                    }
                    LogLevel::Warn => {
                        log!(target: &target, Level::Warn, "{}", message);
                    }
                    LogLevel::Error => {
                        log!(target: &target, Level::Error, "{}", message);
                    }
                }
            }
            CoreNotification::LogMessage { message, target } => {
                use lsp_types::MessageType;
                use tracing_log::log::{Level, log};
                match message.typ {
                    MessageType::ERROR => {
                        log!(target: target, Level::Error, "{}", message.message)
                    }
                    MessageType::WARNING => {
                        log!(target: target, Level::Warn, "{}", message.message)
                    }
                    MessageType::INFO => {
                        log!(target: target, Level::Info, "{}", message.message)
                    }
                    MessageType::DEBUG => {
                        log!(target: target, Level::Debug, "{}", message.message)
                    }
                    MessageType::LOG => {
                        log!(target: target, Level::Debug, "{}", message.message)
                    }
                    _ => {}
                }
            }
            CoreNotification::WorkspaceFileChange => {
                self.file_explorer.reload();
            }
            CoreNotification::AgentEvent { event } => {
                self.agent.handle_event(event.clone());
            }
            CoreNotification::AgentApplyEdit { path, content } => {
                self.agent.apply_edit(path, content);
            }
            _ => {}
        }
    }
}

impl WindowTabData {
    pub fn show_alert(&self, title: String, msg: String, buttons: Vec<AlertButton>) {
        self.alert_data.title.set(title);
        self.alert_data.msg.set(msg);
        self.alert_data.buttons.set(buttons);
        self.alert_data.active.set(true);
    }

    fn update_progress(&self, progress: &ProgressParams) {
        let token = progress.token.clone();
        match &progress.value {
            lsp_types::ProgressParamsValue::WorkDone(progress) => match progress {
                lsp_types::WorkDoneProgress::Begin(progress) => {
                    let progress = WorkProgress {
                        token: token.clone(),
                        title: progress.title.clone(),
                        message: progress.message.clone(),
                        percentage: progress.percentage,
                    };
                    self.progresses.update(|p| {
                        p.insert(token, progress);
                    });
                }
                lsp_types::WorkDoneProgress::Report(report) => {
                    self.progresses.update(|p| {
                        if let Some(progress) = p.get_mut(&token) {
                            progress.message.clone_from(&report.message);
                            progress.percentage = report.percentage;
                        }
                    })
                }
                lsp_types::WorkDoneProgress::End(_) => {
                    self.progresses.update(|p| {
                        p.swap_remove(&token);
                    });
                }
            },
        }
    }

    fn show_message(&self, title: &str, message: &ShowMessageParams) {
        self.messages.update(|messages| {
            messages.push((title.to_string(), message.clone()));
        });
    }
}
