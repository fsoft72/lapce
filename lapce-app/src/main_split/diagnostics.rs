//! Code actions, code lenses, workspace edits and diagnostics navigation of the main split.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use floem::{
    ext_event::create_ext_action,
    reactive::{SignalGet, SignalUpdate, SignalWith},
};
use itertools::Itertools;
use lapce_core::rope_text_pos::RopeTextPosition;
use lapce_rpc::{plugin::PluginId, proxy::ProxyResponse};
use lapce_xi_rope::spans::SpansBuilder;
use lsp_types::{
    CodeAction, CodeActionOrCommand, DiagnosticSeverity, DocumentChangeOperation,
    DocumentChanges, OneOf, Position, TextEdit, Url, WorkspaceEdit,
};
use serde_json::Value;

use super::MainSplitData;
use crate::{
    doc::{DiagnosticData, EditorDiagnostic},
    editor::location::{EditorLocation, EditorPosition},
};

impl MainSplitData {
    pub fn run_code_action(&self, plugin_id: PluginId, action: CodeActionOrCommand) {
        match action {
            CodeActionOrCommand::Command(command) => {
                self.run_code_lens(
                    &command.command,
                    command.arguments.unwrap_or_default(),
                );
            }
            CodeActionOrCommand::CodeAction(action) => {
                if let Some(edit) = action.edit.as_ref() {
                    self.apply_workspace_edit(edit);
                } else {
                    self.resolve_code_action(plugin_id, action);
                }
            }
        }
    }

    pub fn run_code_lens(&self, command: &str, args: Vec<Value>) {
        self.code_lens.get_untracked().run(command, args);
    }

    /// Resolve a code action and apply its held workspace edit
    fn resolve_code_action(&self, plugin_id: PluginId, action: CodeAction) {
        let main_split = self.clone();
        let send = create_ext_action(self.scope, move |edit| {
            main_split.apply_workspace_edit(&edit);
        });
        self.common
            .proxy
            .code_action_resolve(action, plugin_id, move |result| {
                if let Ok(ProxyResponse::CodeActionResolveResponse { item }) = result
                {
                    if let Some(edit) = item.edit {
                        send(edit);
                    }
                }
            });
    }

    /// Perform a workspace edit, which are from the LSP (such as code actions, or symbol renaming)
    pub fn apply_workspace_edit(&self, edit: &WorkspaceEdit) {
        if let Some(DocumentChanges::Operations(_op)) =
            edit.document_changes.as_ref()
        {
            // TODO: apply the create, rename and delete file operations of the edit.
        }

        if let Some(edits) = workspace_edits(edit) {
            for (url, edits) in edits {
                if let Ok(path) = url.to_file_path() {
                    let active_path = self
                        .active_editor
                        .get_untracked()
                        .map(|editor| editor.doc())
                        .map(|doc| doc.content.get_untracked())
                        .and_then(|content| content.path().cloned());
                    let position = if active_path.as_ref() == Some(&path) {
                        None
                    } else {
                        edits
                            .first()
                            .map(|edit| EditorPosition::Position(edit.range.start))
                    };
                    let location = EditorLocation {
                        path,
                        position,
                        scroll_offset: None,
                        ignore_unconfirmed: true,
                        same_editor_tab: false,
                    };
                    self.jump_to_location(location, Some(edits));
                }
            }
        }
    }

    pub fn next_error(&self) {
        let file_diagnostics =
            self.file_diagnostics_items(DiagnosticSeverity::ERROR);
        if file_diagnostics.is_empty() {
            return;
        }
        let active_editor = self.active_editor.get_untracked();
        let active_path = active_editor
            .map(|editor| (editor.doc(), editor.cursor()))
            .and_then(|(doc, cursor)| {
                let offset = cursor.with_untracked(|c| c.offset());
                let (path, position) = (
                    doc.content.get_untracked().path().cloned(),
                    doc.buffer.with_untracked(|b| b.offset_to_position(offset)),
                );
                path.map(|path| (path, offset, position))
            });
        let (path, position) =
            next_in_file_errors_offset(active_path, &file_diagnostics);
        let location = EditorLocation {
            path,
            position: Some(position),
            scroll_offset: None,
            ignore_unconfirmed: false,
            same_editor_tab: false,
        };
        self.jump_to_location(location, None);
    }

    fn file_diagnostics_items(
        &self,
        severity: DiagnosticSeverity,
    ) -> Vec<(PathBuf, Vec<EditorDiagnostic>)> {
        let diagnostics = self.diagnostics.get_untracked();
        diagnostics
            .into_iter()
            .filter_map(|(path, diagnostic)| {
                let span = diagnostic.diagnostics_span.get_untracked();
                if !span.is_empty() {
                    let diags = span
                        .iter()
                        .filter_map(|(iv, diag)| {
                            if diag.severity == Some(severity) {
                                Some(EditorDiagnostic {
                                    range: Some((iv.start, iv.end)),
                                    diagnostic: diag.to_owned(),
                                })
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<EditorDiagnostic>>();
                    if !diags.is_empty() {
                        Some((path, diags))
                    } else {
                        None
                    }
                } else {
                    let diagnostics = diagnostic.diagnostics.get_untracked();
                    let diagnostics: Vec<EditorDiagnostic> = diagnostics
                        .into_iter()
                        .filter(|d| d.severity == Some(severity))
                        .map(|d| EditorDiagnostic {
                            range: None,
                            diagnostic: d,
                        })
                        .collect();
                    if !diagnostics.is_empty() {
                        Some((path, diagnostics))
                    } else {
                        None
                    }
                }
            })
            .sorted_by_key(|(path, _)| path.clone())
            .collect()
    }

    pub fn get_diagnostic_data(&self, path: &Path) -> DiagnosticData {
        if let Some(d) = self.diagnostics.with_untracked(|d| d.get(path).cloned()) {
            d
        } else {
            let diagnostic_data = DiagnosticData {
                expanded: self.scope.create_rw_signal(true),
                diagnostics: self.scope.create_rw_signal(im::Vector::new()),
                diagnostics_span: self
                    .scope
                    .create_rw_signal(SpansBuilder::new(0).build()),
            };
            self.diagnostics.update(|d| {
                d.insert(path.to_path_buf(), diagnostic_data.clone());
            });
            diagnostic_data
        }
    }
}

fn workspace_edits(edit: &WorkspaceEdit) -> Option<HashMap<Url, Vec<TextEdit>>> {
    if let Some(changes) = edit.changes.as_ref() {
        return Some(changes.clone());
    }

    let changes = edit.document_changes.as_ref()?;
    let edits = match changes {
        DocumentChanges::Edits(edits) => edits
            .iter()
            .map(|e| {
                (
                    e.text_document.uri.clone(),
                    e.edits
                        .iter()
                        .map(|e| match e {
                            OneOf::Left(e) => e.clone(),
                            OneOf::Right(e) => e.text_edit.clone(),
                        })
                        .collect(),
                )
            })
            .collect::<HashMap<Url, Vec<TextEdit>>>(),
        DocumentChanges::Operations(ops) => ops
            .iter()
            .filter_map(|o| match o {
                DocumentChangeOperation::Op(_op) => None,
                DocumentChangeOperation::Edit(e) => Some((
                    e.text_document.uri.clone(),
                    e.edits
                        .iter()
                        .map(|e| match e {
                            OneOf::Left(e) => e.clone(),
                            OneOf::Right(e) => e.text_edit.clone(),
                        })
                        .collect(),
                )),
            })
            .collect::<HashMap<Url, Vec<TextEdit>>>(),
    };
    Some(edits)
}

fn next_in_file_errors_offset(
    active_path: Option<(PathBuf, usize, Position)>,
    file_diagnostics: &[(PathBuf, Vec<EditorDiagnostic>)],
) -> (PathBuf, EditorPosition) {
    if let Some((active_path, offset, position)) = active_path {
        for (current_path, diagnostics) in file_diagnostics {
            if &active_path == current_path {
                for diagnostic in diagnostics {
                    if let Some((start, _)) = diagnostic.range {
                        if start > offset {
                            return (
                                (*current_path).clone(),
                                EditorPosition::Offset(start),
                            );
                        }
                    }

                    if diagnostic.diagnostic.range.start.line > position.line
                        || (diagnostic.diagnostic.range.start.line == position.line
                            && diagnostic.diagnostic.range.start.character
                                > position.character)
                    {
                        return (
                            (*current_path).clone(),
                            EditorPosition::Position(
                                diagnostic.diagnostic.range.start,
                            ),
                        );
                    }
                }
            }
            if current_path > &active_path {
                if let Some((start, _)) = diagnostics[0].range {
                    return ((*current_path).clone(), EditorPosition::Offset(start));
                }
                return (
                    (*current_path).clone(),
                    if let Some((start, _)) = diagnostics[0].range {
                        EditorPosition::Offset(start)
                    } else {
                        EditorPosition::Position(
                            diagnostics[0].diagnostic.range.start,
                        )
                    },
                );
            }
        }
    }

    (
        file_diagnostics[0].0.clone(),
        if let Some((start, _)) = file_diagnostics[0].1[0].range {
            EditorPosition::Offset(start)
        } else {
            EditorPosition::Position(file_diagnostics[0].1[0].diagnostic.range.start)
        },
    )
}
