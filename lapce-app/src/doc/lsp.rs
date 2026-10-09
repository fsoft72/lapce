//! Language server data of a document: semantic styles, code lens, symbols, inlay hints, diagnostics and folding ranges.

use std::sync::atomic::{self};

use floem::{
    ext_event::create_ext_action,
    reactive::{RwSignal, SignalGet, SignalUpdate, SignalWith},
};
use lapce_core::{buffer::rope_text::RopeText, rope_text_pos::RopeTextPosition};
use lapce_rpc::proxy::ProxyResponse;
use lapce_xi_rope::{Interval, RopeDelta, spans::SpansBuilder};
use lsp_types::DocumentSymbolResponse;

use super::{DiagnosticData, Doc, DocContent};
use crate::panel::document_symbol::{SymbolData, SymbolInformationItemData};

impl Doc {
    /// Request semantic styles for the buffer from the LSP through the proxy.
    pub fn get_semantic_styles(&self) {
        if !self.loaded() {
            return;
        }

        let path =
            if let DocContent::File { path, .. } = self.content.get_untracked() {
                path
            } else {
                return;
            };

        let (atomic_rev, rev, len) = self
            .buffer
            .with_untracked(|b| (b.atomic_rev(), b.rev(), b.len()));

        let doc = self.clone();
        let send = create_ext_action(self.scope, move |styles| {
            if let Some(styles) = styles {
                if doc.buffer.with_untracked(|b| b.rev()) == rev {
                    doc.semantic_styles.set(Some(styles));
                    doc.clear_style_cache();
                }
            }
        });

        self.common.proxy.get_semantic_tokens(path, move |result| {
            if let Ok(ProxyResponse::GetSemanticTokens { styles }) = result {
                if styles.styles.is_empty() {
                    send(None);
                    return;
                }
                if atomic_rev.load(atomic::Ordering::Acquire) != rev {
                    send(None);
                    return;
                }
                std::thread::spawn(move || {
                    let mut styles_span = SpansBuilder::new(len);
                    for style in styles.styles {
                        if atomic_rev.load(atomic::Ordering::Acquire) != rev {
                            send(None);
                            return;
                        }
                        styles_span.add_span(
                            Interval::new(style.start, style.end),
                            style.style,
                        );
                    }

                    let styles = styles_span.build();
                    send(Some(styles));
                });
            } else {
                send(None);
            }
        });
    }

    pub fn get_code_lens(&self) {
        let cx = self.scope;
        let doc = self.clone();
        self.code_lens.update(|code_lens| {
            code_lens.clear();
        });
        let rev = self.rev();
        if let DocContent::File { path, .. } = doc.content.get_untracked() {
            let send = create_ext_action(cx, move |result| {
                if rev != doc.rev() {
                    return;
                }
                if let Ok(ProxyResponse::GetCodeLensResponse { plugin_id, resp }) =
                    result
                {
                    let Some(codelens) = resp else {
                        return;
                    };
                    doc.code_lens.update(|code_lens| {
                        for codelens in codelens {
                            if codelens.command.is_none() {
                                continue;
                            }
                            let entry = code_lens
                                .entry(codelens.range.start.line as usize)
                                .or_insert_with(|| {
                                    (
                                        plugin_id,
                                        doc.buffer.with_untracked(|b| {
                                            b.offset_of_line(
                                                codelens.range.start.line as usize,
                                            )
                                        }),
                                        im::Vector::new(),
                                    )
                                });
                            entry.2.push_back(codelens);
                        }
                    });
                }
            });
            self.common.proxy.get_code_lens(path, move |result| {
                send(result);
            });
        }
    }

    pub fn get_document_symbol(&self) {
        let cx = self.scope;
        let doc = self.clone();
        let rev = self.rev();
        if let DocContent::File { path, .. } = doc.content.get_untracked() {
            let send = create_ext_action(cx, {
                let path = path.clone();
                move |result| {
                    if rev != doc.rev() {
                        return;
                    }
                    if let Ok(ProxyResponse::GetDocumentSymbols { resp }) = result {
                        let items: Vec<RwSignal<SymbolInformationItemData>> =
                            match resp {
                                DocumentSymbolResponse::Flat(_symbols) => {
                                    Vec::with_capacity(0)
                                }
                                DocumentSymbolResponse::Nested(symbols) => symbols
                                    .into_iter()
                                    .map(|x| {
                                        cx.create_rw_signal(
                                            SymbolInformationItemData::from((x, cx)),
                                        )
                                    })
                                    .collect(),
                            };
                        let symbol_new = Some(SymbolData::new(items, path, cx));
                        doc.document_symbol_data.update(|symbol| {
                            *symbol = symbol_new;
                        });
                    }
                }
            });

            self.common.proxy.get_document_symbols(path, move |result| {
                send(result);
            });
        }
    }

    /// Request inlay hints for the buffer from the LSP through the proxy.
    pub fn get_inlay_hints(&self) {
        if !self.loaded() {
            return;
        }

        let path =
            if let DocContent::File { path, .. } = self.content.get_untracked() {
                path
            } else {
                return;
            };

        let (buffer, rev, len) = self
            .buffer
            .with_untracked(|b| (b.clone(), b.rev(), b.len()));

        let doc = self.clone();
        let send = create_ext_action(self.scope, move |hints| {
            if doc.buffer.with_untracked(|b| b.rev()) == rev {
                doc.inlay_hints.set(Some(hints));
                doc.clear_text_cache();
            }
        });

        self.common.proxy.get_inlay_hints(path, move |result| {
            if let Ok(ProxyResponse::GetInlayHints { mut hints }) = result {
                // Sort the inlay hints by their position, as the LSP does not guarantee that it will
                // provide them in the order that they are in within the file
                // as well, Spans does not iterate in the order that they appear
                hints.sort_by(|left, right| left.position.cmp(&right.position));

                let mut hints_span = SpansBuilder::new(len);
                for hint in hints {
                    let offset = buffer.offset_of_position(&hint.position).min(len);
                    hints_span.add_span(
                        Interval::new(offset, (offset + 1).min(len)),
                        hint,
                    );
                }
                let hints = hints_span.build();
                send(hints);
            }
        });
    }

    pub fn diagnostics(&self) -> &DiagnosticData {
        &self.diagnostics
    }

    /// Update the diagnostics' positions after an edit so that they appear in the correct place.
    pub(super) fn update_diagnostics(&self, delta: &RopeDelta) {
        if self
            .diagnostics
            .diagnostics
            .with_untracked(|d| d.is_empty())
        {
            return;
        }

        self.diagnostics.diagnostics_span.update(|diagnostics| {
            diagnostics.apply_shape(delta);
        });
    }

    /// init diagnostics offset ranges from lsp positions
    pub fn init_diagnostics(&self) {
        let len = self.buffer.with_untracked(|b| b.len());
        let diagnostics = self.diagnostics.diagnostics.get_untracked();

        let span = self.buffer.with_untracked(|buffer| {
            let mut span = SpansBuilder::new(len);
            for diag in diagnostics.iter() {
                let start = buffer.offset_of_position(&diag.range.start);
                let end = buffer.offset_of_position(&diag.range.end);
                span.add_span(Interval::new(start, end), diag.to_owned());
            }
            span.build()
        });
        self.diagnostics.diagnostics_span.set(span);

        self.clear_text_cache();
        self.clear_code_actions();
    }

    pub fn get_folding_range(&self) {
        // let cx = self.scope;
        // let doc = self.clone();
        // let rev = self.rev();
        // if let DocContent::File { path, .. } = doc.content.get_untracked() {
        //     let send = create_ext_action(cx, {
        //         move |result| {
        //             if rev != doc.rev() {
        //                 return;
        //             }
        //             if let Ok(ProxyResponse::LspFoldingRangeResponse {
        //                 resp, ..
        //             }) = result
        //             {
        //                 let folding = resp
        //                     .unwrap_or_default()
        //                     .into_iter()
        //                     .map(|x| {
        //                         crate::editor::gutter::FoldingRange::from_lsp(x)
        //                     })
        //                     .sorted_by(|x, y| x.start.line.cmp(&y.start.line))
        //                     .collect();
        //                 doc.folding_ranges.update(|symbol| {
        //                     symbol.0 = folding;
        //                 });
        //             }
        //         }
        //     });

        //     self.common
        //         .proxy
        //         .get_lsp_folding_range(path, move |result| {
        //             send(result);
        //         });
        // }
    }
}
