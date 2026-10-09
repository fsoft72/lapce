//! Styling of a document: fonts, indentation, syntax colors and phantom text styles.

use std::{borrow::Cow, rc::Rc, sync::Arc};

use floem::{
    peniko::Color,
    reactive::{ReadSignal, SignalGet, SignalWith},
    text::{Attrs, AttrsList, FamilyOwned, TextLayout},
    views::editor::{
        EditorStyle,
        id::EditorId,
        layout::{LineExtraStyle, TextLayoutLine},
        text::{Document, DocumentPhantom, Styling},
    },
};
use lapce_core::buffer::rope_text::RopeText;
use lsp_types::DiagnosticSeverity;

use super::Doc;
use crate::config::{LapceConfig, color::LapceColor};

#[derive(Clone)]
pub struct DocStyling {
    pub(super) config: ReadSignal<Arc<LapceConfig>>,
    pub(super) doc: Rc<Doc>,
}
impl DocStyling {
    fn apply_colorization(
        &self,
        edid: EditorId,
        style: &EditorStyle,
        line: usize,
        attrs: &Attrs,
        attrs_list: &mut AttrsList,
    ) {
        let config = self.config.get_untracked();
        let phantom_text = self.doc.phantom_text(edid, style, line);
        if let Some(bracket_styles) = self.doc.parser.borrow().bracket_pos.get(&line)
        {
            for bracket_style in bracket_styles.iter() {
                if let Some(fg_color) = bracket_style.style.fg_color.as_ref() {
                    if let Some(fg_color) = config.style_color(fg_color) {
                        let start = phantom_text.col_at(bracket_style.start);
                        let end = phantom_text.col_at(bracket_style.end);
                        attrs_list
                            .add_span(start..end, attrs.clone().color(fg_color));
                    }
                }
            }
        }
    }
}
impl Styling for DocStyling {
    fn id(&self) -> u64 {
        self.config.with_untracked(|config| config.id)
    }

    fn font_size(&self, _: EditorId, _line: usize) -> usize {
        self.config
            .with_untracked(|config| config.editor.font_size())
    }

    fn line_height(&self, _: EditorId, _line: usize) -> f32 {
        self.config
            .with_untracked(|config| config.editor.line_height()) as f32
    }

    fn font_family(
        &self,
        _: EditorId,
        _line: usize,
    ) -> std::borrow::Cow<'_, [floem::text::FamilyOwned]> {
        // TODO: cache this
        Cow::Owned(self.config.with_untracked(|config| {
            FamilyOwned::parse_list(&config.editor.font_family).collect()
        }))
    }

    fn weight(&self, _: EditorId, _line: usize) -> floem::text::Weight {
        floem::text::Weight::NORMAL
    }

    fn italic_style(&self, _: EditorId, _line: usize) -> floem::text::Style {
        floem::text::Style::Normal
    }

    fn stretch(&self, _: EditorId, _line: usize) -> floem::text::Stretch {
        floem::text::Stretch::Normal
    }

    fn indent_line(&self, _: EditorId, line: usize, line_content: &str) -> usize {
        if line_content.trim().is_empty() {
            let text = self.doc.rope_text();
            let offset = text.offset_of_line(line);
            if let Some(offset) =
                self.doc.syntax.with_untracked(|s| s.parent_offset(offset))
            {
                return text.line_of_offset(offset);
            }
        }

        line
    }

    fn tab_width(&self, _: EditorId, _line: usize) -> usize {
        self.config.with_untracked(|config| config.editor.tab_width)
    }

    fn atomic_soft_tabs(&self, _: EditorId, _line: usize) -> bool {
        self.config
            .with_untracked(|config| config.editor.atomic_soft_tabs)
    }

    fn apply_attr_styles(
        &self,
        edid: EditorId,
        style: &EditorStyle,
        line: usize,
        default: Attrs,
        attrs_list: &mut AttrsList,
    ) {
        let config = self.doc.common.config.get_untracked();

        self.apply_colorization(edid, style, line, &default, attrs_list);

        let phantom_text = self.doc.phantom_text(edid, style, line);
        for line_style in self.doc.line_style(line).iter() {
            if let Some(fg_color) = line_style.style.fg_color.as_ref() {
                if let Some(fg_color) = config.style_color(fg_color) {
                    let start = phantom_text.col_at(line_style.start);
                    let end = phantom_text.col_at(line_style.end);
                    attrs_list.add_span(start..end, default.clone().color(fg_color));
                }
            }
        }
    }

    fn apply_layout_styles(
        &self,
        edid: EditorId,
        style: &EditorStyle,
        line: usize,
        layout_line: &mut TextLayoutLine,
    ) {
        let doc = &self.doc;
        let config = doc.common.config.get_untracked();

        layout_line.extra_style.clear();
        let layout = &layout_line.text;

        let phantom_text = doc.phantom_text(edid, style, line);

        let phantom_styles = phantom_text
            .offset_size_iter()
            .filter(move |(_, _, _, p)| p.bg.is_some() || p.under_line.is_some())
            .flat_map(move |(col_shift, size, col, phantom)| {
                let start = col + col_shift;
                let end = start + size;

                extra_styles_for_range(
                    layout,
                    start,
                    end,
                    phantom.bg,
                    phantom.under_line,
                    None,
                )
            });
        layout_line.extra_style.extend(phantom_styles);

        let (start_offset, end_offset) = doc.buffer.with_untracked(|buffer| {
            (buffer.offset_of_line(line), buffer.offset_of_line(line + 1))
        });

        let mut max_severity: Option<DiagnosticSeverity> = None;
        doc.diagnostics.diagnostics_span.with_untracked(|diags| {
            diags
                .iter_chunks(start_offset..end_offset)
                .for_each(|(iv, diag)| {
                    let start = iv.start();
                    let end = iv.end();

                    if start <= end_offset
                        && end >= start_offset
                        && diag.severity < Some(DiagnosticSeverity::HINT)
                    {
                        let start = start.saturating_sub(start_offset);
                        let end = end - start_offset;
                        let start = phantom_text.col_after(start, true);
                        let end = phantom_text.col_after(end, false);

                        match (diag.severity, max_severity) {
                            (Some(severity), Some(max)) => {
                                if severity < max {
                                    max_severity = Some(severity);
                                }
                            }
                            (Some(severity), None) => {
                                max_severity = Some(severity);
                            }
                            _ => {}
                        }

                        let color_name = match diag.severity {
                            Some(DiagnosticSeverity::ERROR) => {
                                LapceColor::LAPCE_ERROR
                            }
                            _ => LapceColor::LAPCE_WARN,
                        };
                        let color = config.color(color_name);
                        let styles = extra_styles_for_range(
                            layout,
                            start,
                            end,
                            None,
                            None,
                            Some(color),
                        );
                        layout_line.extra_style.extend(styles);
                    }
                });
        });

        // Add the styling for the diagnostic severity, if applicable
        if let Some(max_severity) = max_severity {
            let theme_prop = if max_severity == DiagnosticSeverity::ERROR {
                LapceColor::ERROR_LENS_ERROR_BACKGROUND
            } else if max_severity == DiagnosticSeverity::WARNING {
                LapceColor::ERROR_LENS_WARNING_BACKGROUND
            } else {
                LapceColor::ERROR_LENS_OTHER_BACKGROUND
            };

            let size = layout.size();
            let x1 = if !config.editor.error_lens_end_of_line {
                let error_end_x = size.width;
                Some(error_end_x.max(size.width))
            } else {
                None
            };

            // TODO(minor): Should we show the background only on wrapped lines that have the
            // diagnostic actually on that line?
            // That would make it more obvious where it is from and matches other editors.
            layout_line.extra_style.push(LineExtraStyle {
                x: 0.0,
                y: 0.0,
                width: x1,
                height: size.height,
                bg_color: Some(config.color(theme_prop)),
                under_line: None,
                wave_line: None,
            });
        }
    }

    fn paint_caret(&self, edid: EditorId, _line: usize) -> bool {
        let Some(e_data) = self.doc.editor_data(edid) else {
            return true;
        };

        // If the find is active, then we don't want to paint the caret
        !e_data.find_focus.get_untracked()
    }
}

fn extra_styles_for_range(
    text_layout: &TextLayout,
    start: usize,
    end: usize,
    bg_color: Option<Color>,
    under_line: Option<Color>,
    wave_line: Option<Color>,
) -> impl Iterator<Item = LineExtraStyle> + '_ {
    let start_hit = text_layout.hit_position(start);
    let end_hit = text_layout.hit_position(end);

    text_layout
        .layout_runs()
        .enumerate()
        .filter_map(move |(current_line, run)| {
            if current_line < start_hit.line || current_line > end_hit.line {
                return None;
            }

            let x = if current_line == start_hit.line {
                start_hit.point.x
            } else {
                run.glyphs.first().map(|g| g.x).unwrap_or(0.0) as f64
            };
            let end_x = if current_line == end_hit.line {
                end_hit.point.x
            } else {
                run.glyphs.last().map(|g| g.x + g.w).unwrap_or(0.0) as f64
            };
            let width = end_x - x;

            if width == 0.0 {
                return None;
            }

            let height = (run.max_ascent + run.max_descent) as f64;
            let y = run.line_y as f64 - run.max_ascent as f64;

            Some(LineExtraStyle {
                x,
                y,
                width: Some(width),
                height,
                bg_color,
                under_line,
                wave_line,
            })
        })
}
