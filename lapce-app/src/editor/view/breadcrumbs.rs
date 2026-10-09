//! The breadcrumbs bar shown above the editor.

use std::sync::Arc;

use floem::{
    View,
    peniko::kurbo::Point,
    reactive::{ReadSignal, SignalGet, SignalWith, create_memo},
    views::{Decorators, container, dyn_stack, label, scroll::scroll, stack, svg},
};

use crate::editor::EditorData;
use crate::{
    config::{LapceConfig, color::LapceColor, icon::LapceIcons},
    doc::DocContent,
    workspace::LapceWorkspace,
};

pub(super) fn editor_breadcrumbs(
    workspace: Arc<LapceWorkspace>,
    e_data: EditorData,
    config: ReadSignal<Arc<LapceConfig>>,
) -> impl View {
    let doc = e_data.doc_signal();
    let doc_path = create_memo(move |_| {
        let doc = doc.get();
        let content = doc.content.get();
        if let DocContent::History(history) = &content {
            Some(history.path.clone())
        } else {
            content.path().cloned()
        }
    });
    container(
        scroll(
            stack((
                {
                    let workspace = workspace.clone();
                    dyn_stack(
                        move || {
                            let full_path = doc_path.get().unwrap_or_default();
                            let mut path = full_path;
                            if let Some(workspace_path) =
                                workspace.clone().path.as_ref()
                            {
                                path = path
                                    .strip_prefix(workspace_path)
                                    .unwrap_or(&path)
                                    .to_path_buf();
                            }
                            path.ancestors()
                                .filter_map(|path| {
                                    Some(
                                        path.file_name()?
                                            .to_string_lossy()
                                            .into_owned(),
                                    )
                                })
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .enumerate()
                        },
                        |(i, section)| (*i, section.to_string()),
                        move |(i, section)| {
                            stack((
                                svg(move || {
                                    config
                                        .get()
                                        .ui_svg(LapceIcons::BREADCRUMB_SEPARATOR)
                                })
                                .style(move |s| {
                                    let config = config.get();
                                    let size = config.ui.icon_size() as f32;
                                    s.apply_if(i == 0, |s| s.hide())
                                        .size(size, size)
                                        .color(
                                            config.color(
                                                LapceColor::LAPCE_ICON_ACTIVE,
                                            ),
                                        )
                                }),
                                label(move || section.clone())
                                    .style(move |s| s.selectable(false)),
                            ))
                            .style(|s| s.items_center())
                        },
                    )
                    .style(|s| s.padding_horiz(10.0))
                },
                label(move || {
                    let doc = doc.get();
                    if let DocContent::History(history) = doc.content.get() {
                        format!("({})", history.version)
                    } else {
                        "".to_string()
                    }
                })
                .style(move |s| {
                    let doc = doc.get();
                    let is_history = doc.content.with_untracked(|content| {
                        matches!(content, DocContent::History(_))
                    });

                    s.padding_right(10.0).apply_if(!is_history, |s| s.hide())
                }),
            ))
            .style(|s| s.items_center()),
        )
        .scroll_to(move || {
            doc.track();
            Some(Point::new(3000.0, 0.0))
        })
        .scroll_style(|s| s.hide_bars(true))
        .style(move |s| {
            s.absolute()
                .size_pct(100.0, 100.0)
                .border_bottom(1.0)
                .border_color(config.get().color(LapceColor::LAPCE_BORDER))
                .items_center()
        }),
    )
    .style(move |s| {
        let config = config.get_untracked();
        let line_height = config.editor.line_height();
        s.items_center()
            .width_pct(100.0)
            .height(line_height as f32)
            .apply_if(doc_path.get().is_none(), |s| s.hide())
            .apply_if(!config.editor.show_bread_crumbs, |s| s.hide())
    })
    .debug_name("Editor BreadCrumbs")
}
