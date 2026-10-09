//! Computes the lines shown on screen from the visual lines of the editor.

use std::{collections::HashMap, rc::Rc, sync::Arc};

use floem::{
    prelude::SignalTrack,
    reactive::{ReadSignal, RwSignal, SignalGet, SignalWith},
    views::editor::{
        view::{
            DiffSection, DiffSectionKind, LineInfo, ScreenLines, ScreenLinesBase,
        },
        visual_line::{ConfigId, Lines, TextLayoutProvider, VLine, VLineInfo},
    },
};
use lapce_core::buffer::diff::DiffLines;

use super::EditorViewKind;
use crate::{config::LapceConfig, doc::Doc};

// TODO(minor): Should we just put this on view, since it only requires those values?
pub(crate) fn compute_screen_lines(
    config: ReadSignal<Arc<LapceConfig>>,
    base: RwSignal<ScreenLinesBase>,
    view_kind: ReadSignal<EditorViewKind>,
    doc: &Doc,
    lines: &Lines,
    text_prov: impl TextLayoutProvider + Clone,
    config_id: ConfigId,
) -> ScreenLines {
    // TODO: this should probably be a get since we need to depend on line-height
    let config = config.get();
    let line_height = config.editor.line_height();

    let (y0, y1) = base
        .with_untracked(|base| (base.active_viewport.y0, base.active_viewport.y1));
    // Get the start and end (visual) lines that are visible in the viewport
    let min_vline = VLine((y0 / line_height as f64).floor() as usize);
    let max_vline = VLine((y1 / line_height as f64).ceil() as usize);

    let cache_rev = doc.cache_rev.get();
    lines.check_cache_rev(cache_rev);
    // TODO(minor): we don't really need to depend on various subdetails that aren't affecting how
    // the screen lines are set up, like the title of a scratch document.
    doc.content.track();
    doc.loaded.track();

    let min_info = once_cell::sync::Lazy::new(|| {
        lines
            .iter_vlines(text_prov.clone(), false, min_vline)
            .next()
    });

    match view_kind.get() {
        EditorViewKind::Normal => {
            let mut rvlines = Vec::new();
            let mut info = HashMap::new();

            let Some(min_info) = *min_info else {
                return ScreenLines {
                    lines: Rc::new(rvlines),
                    info: Rc::new(info),
                    diff_sections: None,
                    base,
                };
            };

            // TODO: the original was min_line..max_line + 1, are we iterating too little now?
            // the iterator is from min_vline..max_vline
            let count = max_vline.get() - min_vline.get();
            let iter = lines.iter_rvlines_init(
                text_prov,
                cache_rev,
                config_id,
                min_info.rvline,
                false,
            );

            // let range = doc.folding_ranges.get().get_folded_range();
            // let mut init_index = 0;

            for (i, vline_info) in iter.enumerate() {
                if rvlines.len() >= count {
                    break;
                }

                // let (folded, next_index) =
                //     range.contain_line(init_index, vline_info.rvline.line as u32);
                // init_index = next_index;
                // if folded {
                //     continue;
                // }
                rvlines.push(vline_info.rvline);

                let y_idx = min_vline.get() + i;
                let vline_y = y_idx * line_height;
                let line_y = vline_y - vline_info.rvline.line_index * line_height;

                // Add the information to make it cheap to get in the future.
                // This y positions are shifted by the baseline y0
                info.insert(
                    vline_info.rvline,
                    LineInfo {
                        y: line_y as f64 - y0,
                        vline_y: vline_y as f64 - y0,
                        vline_info,
                    },
                );
            }

            ScreenLines {
                lines: Rc::new(rvlines),
                info: Rc::new(info),
                diff_sections: None,
                base,
            }
        }
        EditorViewKind::Diff(diff_info) => {
            // TODO: let lines in diff view be wrapped, possibly screen_lines should be impl'd
            // on DiffEditorData

            let mut y_idx = 0;
            let mut rvlines = Vec::new();
            let mut info = HashMap::new();
            let mut diff_sections = Vec::new();
            let mut last_change: Option<&DiffLines> = None;
            let mut changes = diff_info.changes.iter().peekable();
            let is_right = diff_info.is_right;

            let line_y = |info: VLineInfo<()>, vline_y: usize| -> usize {
                vline_y.saturating_sub(info.rvline.line_index * line_height)
            };

            while let Some(change) = changes.next() {
                match (is_right, change) {
                    (true, DiffLines::Left(range)) => {
                        if let Some(DiffLines::Right(_)) = changes.peek() {
                        } else {
                            let len = range.len();
                            diff_sections.push(DiffSection {
                                y_idx,
                                height: len,
                                kind: DiffSectionKind::NoCode,
                            });
                            y_idx += len;
                        }
                    }
                    (false, DiffLines::Right(range)) => {
                        let len = if let Some(DiffLines::Left(r)) = last_change {
                            range.len() - r.len().min(range.len())
                        } else {
                            range.len()
                        };
                        if len > 0 {
                            diff_sections.push(DiffSection {
                                y_idx,
                                height: len,
                                kind: DiffSectionKind::NoCode,
                            });
                            y_idx += len;
                        }
                    }
                    (true, DiffLines::Right(range))
                    | (false, DiffLines::Left(range)) => {
                        // TODO: count vline count in the range instead
                        let height = range.len();

                        diff_sections.push(DiffSection {
                            y_idx,
                            height,
                            kind: if is_right {
                                DiffSectionKind::Added
                            } else {
                                DiffSectionKind::Removed
                            },
                        });

                        let initial_y_idx = y_idx;
                        // Mopve forward by the count given.
                        y_idx += height;

                        if y_idx < min_vline.get() {
                            if is_right {
                                if let Some(DiffLines::Left(r)) = last_change {
                                    // TODO: count vline count in the other editor since this is skipping an amount dependent on those vlines
                                    let len = r.len() - r.len().min(range.len());
                                    if len > 0 {
                                        diff_sections.push(DiffSection {
                                            y_idx,
                                            height: len,
                                            kind: DiffSectionKind::NoCode,
                                        });
                                        y_idx += len;
                                    }
                                };
                            }
                            last_change = Some(change);
                            continue;
                        }

                        let start_rvline =
                            lines.rvline_of_line(&text_prov, range.start);

                        // TODO: this wouldn't need to produce vlines if screen lines didn't
                        // require them.
                        let iter = lines
                            .iter_rvlines_init(
                                &text_prov,
                                cache_rev,
                                config_id,
                                start_rvline,
                                false,
                            )
                            .take_while(|vline_info| {
                                vline_info.rvline.line < range.end
                            })
                            .enumerate();
                        for (i, rvline_info) in iter {
                            let rvline = rvline_info.rvline;
                            if initial_y_idx + i < min_vline.0 {
                                continue;
                            }

                            rvlines.push(rvline);
                            let vline_y = (initial_y_idx + i) * line_height;
                            info.insert(
                                rvline,
                                LineInfo {
                                    y: line_y(rvline_info, vline_y) as f64 - y0,
                                    vline_y: vline_y as f64 - y0,
                                    vline_info: rvline_info,
                                },
                            );

                            if initial_y_idx + i > max_vline.0 {
                                break;
                            }
                        }

                        if is_right {
                            if let Some(DiffLines::Left(r)) = last_change {
                                // TODO: count vline count in the other editor since this is skipping an amount dependent on those vlines
                                let len = r.len() - r.len().min(range.len());
                                if len > 0 {
                                    diff_sections.push(DiffSection {
                                        y_idx,
                                        height: len,
                                        kind: DiffSectionKind::NoCode,
                                    });
                                    y_idx += len;
                                }
                            };
                        }
                    }
                    (_, DiffLines::Both(bothinfo)) => {
                        let start = if is_right {
                            bothinfo.right.start
                        } else {
                            bothinfo.left.start
                        };
                        let len = bothinfo.right.len();
                        let diff_height = len
                            - bothinfo
                                .skip
                                .as_ref()
                                .map(|skip| skip.len().saturating_sub(1))
                                .unwrap_or(0);
                        if y_idx + diff_height < min_vline.get() {
                            y_idx += diff_height;
                            last_change = Some(change);
                            continue;
                        }

                        let start_rvline = lines.rvline_of_line(&text_prov, start);

                        let mut iter = lines
                            .iter_rvlines_init(
                                &text_prov,
                                cache_rev,
                                config_id,
                                start_rvline,
                                false,
                            )
                            .peekable();
                        while let Some(rvline_info) = iter.next() {
                            let line = rvline_info.rvline.line;

                            if line >= start + len {
                                break;
                            }

                            // Skip over the lines
                            if let Some(skip) = bothinfo.skip.as_ref() {
                                if Some(skip.start) == line.checked_sub(start) {
                                    y_idx += 1;

                                    // restart iterator after the skip
                                    let start_rvline = lines.rvline_of_line(
                                        &text_prov,
                                        start + skip.end,
                                    );

                                    iter = lines
                                        .iter_rvlines_init(
                                            &text_prov,
                                            cache_rev,
                                            config_id,
                                            start_rvline,
                                            false,
                                        )
                                        .peekable();

                                    continue;
                                }
                            }

                            // Add the vline if it is within view
                            if y_idx >= min_vline.get() {
                                rvlines.push(rvline_info.rvline);
                                let vline_y = y_idx * line_height;
                                info.insert(
                                    rvline_info.rvline,
                                    LineInfo {
                                        y: line_y(rvline_info, vline_y) as f64 - y0,
                                        vline_y: vline_y as f64 - y0,
                                        vline_info: rvline_info,
                                    },
                                );
                            }

                            y_idx += 1;

                            if y_idx - 1 > max_vline.get() {
                                break;
                            }
                        }
                    }
                }
                last_change = Some(change);
            }
            ScreenLines {
                lines: Rc::new(rvlines),
                info: Rc::new(info),
                diff_sections: Some(Rc::new(diff_sections)),
                base,
            }
        }
    }
}
