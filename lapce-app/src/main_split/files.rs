//! Save as and scratch document saving of the main split.

use std::{path::PathBuf, rc::Rc};

use floem::{
    action::save_as,
    ext_event::create_ext_action,
    file::{FileDialogOptions, FileInfo},
    reactive::{SignalGet, SignalUpdate, SignalWith},
};
use lapce_core::syntax::Syntax;
use tracing::{Level, event};

use super::MainSplitData;
use crate::doc::{Doc, DocContent};

impl MainSplitData {
    pub fn save_as(&self, doc: Rc<Doc>, path: PathBuf, action: impl Fn() + 'static) {
        let (buffer_id, doc_content, rev, content) = (
            doc.buffer_id,
            doc.content.get_untracked(),
            doc.rev(),
            doc.buffer.with_untracked(|b| b.to_string()),
        );
        match doc_content {
            DocContent::Scratch { .. } => {
                let send = {
                    let path = path.clone();
                    create_ext_action(self.scope, move |result| {
                        if let Err(err) = result {
                            event!(
                                Level::WARN,
                                "Failed to save as a file: {:?}",
                                err
                            );
                        } else {
                            let syntax = Syntax::init(&path);
                            doc.content.set(DocContent::File {
                                path: path.clone(),
                                read_only: false,
                            });
                            doc.buffer.update(|buffer| {
                                buffer.set_pristine();
                            });
                            doc.set_syntax(syntax);
                            doc.trigger_syntax_change(None);
                            action();
                        }
                    })
                };
                self.common.proxy.save_buffer_as(
                    buffer_id,
                    path,
                    rev,
                    content,
                    true,
                    Box::new(move |result| {
                        send(result);
                    }),
                );
            }
            DocContent::Local => {}
            DocContent::File { .. } => {}
            DocContent::History(_) => {}
        }
    }

    pub fn save_as2(
        &self,
        doc: Rc<Doc>,
        path: PathBuf,
        action: impl Fn() + 'static,
    ) {
        let (buffer_id, doc_content, rev, content) = (
            doc.buffer_id,
            doc.content.get_untracked(),
            doc.rev(),
            doc.buffer.with_untracked(|b| b.to_string()),
        );
        match doc_content {
            DocContent::Scratch { .. } => {
                let send = {
                    let path = path.clone();
                    create_ext_action(self.scope, move |result| {
                        if let Err(err) = result {
                            event!(
                                Level::WARN,
                                "Failed to save as a file: {:?}",
                                err
                            );
                        } else {
                            let syntax = Syntax::init(&path);
                            doc.content.set(DocContent::File {
                                path: path.clone(),
                                read_only: false,
                            });
                            doc.buffer.update(|buffer| {
                                buffer.set_pristine();
                            });
                            doc.set_syntax(syntax);
                            doc.trigger_syntax_change(None);
                            action();
                        }
                    })
                };
                self.common.proxy.save_buffer_as(
                    buffer_id,
                    path,
                    rev,
                    content,
                    true,
                    Box::new(move |result| {
                        send(result);
                    }),
                );
            }
            DocContent::Local => {}
            DocContent::File { .. } => {}
            DocContent::History(_) => {}
        }
    }

    pub(super) fn get_name_for_new_file(&self) -> String {
        const PREFIX: &str = "Untitled-";

        // Checking just the current scratch_docs rather than all the different document
        // collections seems to be the right thing to do. The user may have genuine 'new N'
        // files tucked away somewhere in their workspace.
        let new_num = self.scratch_docs.with_untracked(|scratch_docs| {
            scratch_docs
                .values()
                .filter_map(|doc| {
                    doc.content.with_untracked(|content| match content {
                        DocContent::Scratch { name, .. } => {
                            // The unwraps are safe because scratch docs are always
                            // titled the same format and the user cannot change the name.
                            let num_part = name.strip_prefix(PREFIX).unwrap();
                            let num = num_part.parse::<i32>().unwrap();
                            Some(num)
                        }
                        _ => None,
                    })
                })
                .max()
                .unwrap_or(0)
                + 1
        });

        format!("{PREFIX}{new_num}")
    }
}

impl MainSplitData {
    pub fn save_scratch_doc(&self, doc: Rc<Doc>) {
        let main_split = self.clone();
        save_as(
            FileDialogOptions::new().title("Save File"),
            move |file: Option<FileInfo>| {
                if let Some(mut file) = file {
                    main_split.save_as(
                        doc.clone(),
                        if let Some(path) = file.path.pop() {
                            path
                        } else {
                            tracing::error!("No path");
                            return;
                        },
                        move || {},
                    );
                }
            },
        );
    }

    pub fn save_scratch_doc2(&self, doc: Rc<Doc>) {
        let main_split = self.clone();
        save_as(
            FileDialogOptions::new().title("Save File"),
            move |file: Option<FileInfo>| {
                if let Some(mut file) = file {
                    main_split.save_as2(
                        doc.clone(),
                        if let Some(path) = file.path.pop() {
                            path
                        } else {
                            tracing::error!("No path");
                            return;
                        },
                        move || {},
                    );
                }
            },
        );
    }
}
