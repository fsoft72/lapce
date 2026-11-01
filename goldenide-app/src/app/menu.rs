//! The window context menu.

use floem::menu::{Menu, MenuItem};
use goldenide_core::command::{EditCommand, FocusCommand};

use crate::{
    command::{CommandKind, GoldenIdeCommand, GoldenIdeWorkbenchCommand},
    listener::Listener,
};

pub fn window_menu(
    goldenide_command: Listener<GoldenIdeCommand>,
    workbench_command: Listener<GoldenIdeWorkbenchCommand>,
) -> Menu {
    Menu::new("GoldenIDE")
        .entry({
            let mut menu = Menu::new("GoldenIDE")
                .entry(MenuItem::new("About GoldenIDE").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::ShowAbout)
                }))
                .separator()
                .entry(
                    Menu::new("Settings...")
                        .entry(MenuItem::new("Open Settings").action(move || {
                            workbench_command
                                .send(GoldenIdeWorkbenchCommand::OpenSettings);
                        }))
                        .entry(MenuItem::new("Open Keyboard Shortcuts").action(
                            move || {
                                workbench_command.send(
                                    GoldenIdeWorkbenchCommand::OpenKeyboardShortcuts,
                                );
                            },
                        )),
                )
                .separator()
                .entry(MenuItem::new("Quit GoldenIDE").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::Quit);
                }));
            if cfg!(target_os = "macos") {
                menu = menu
                    .separator()
                    .entry(MenuItem::new("Hide GoldenIDE"))
                    .entry(MenuItem::new("Hide Others"))
                    .entry(MenuItem::new("Show All"))
            }
            menu
        })
        .separator()
        .entry(
            Menu::new("File")
                .entry(MenuItem::new("New File").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::NewFile);
                }))
                .separator()
                .entry(MenuItem::new("Open").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::OpenFile);
                }))
                .entry(MenuItem::new("Open Folder").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::OpenFolder);
                }))
                .separator()
                .entry(MenuItem::new("Save").action(move || {
                    goldenide_command.send(GoldenIdeCommand {
                        kind: CommandKind::Focus(FocusCommand::Save),
                        data: None,
                    });
                }))
                .entry(MenuItem::new("Save All").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::SaveAll);
                }))
                .separator()
                .entry(MenuItem::new("Close Folder").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::CloseFolder);
                }))
                .entry(MenuItem::new("Close Window").action(move || {
                    workbench_command.send(GoldenIdeWorkbenchCommand::CloseWindow);
                })),
        )
        .entry(
            Menu::new("Edit")
                .entry(MenuItem::new("Cut").action(move || {
                    goldenide_command.send(GoldenIdeCommand {
                        kind: CommandKind::Edit(EditCommand::ClipboardCut),
                        data: None,
                    });
                }))
                .entry(MenuItem::new("Copy").action(move || {
                    goldenide_command.send(GoldenIdeCommand {
                        kind: CommandKind::Edit(EditCommand::ClipboardCopy),
                        data: None,
                    });
                }))
                .entry(MenuItem::new("Paste").action(move || {
                    goldenide_command.send(GoldenIdeCommand {
                        kind: CommandKind::Edit(EditCommand::ClipboardPaste),
                        data: None,
                    });
                }))
                .separator()
                .entry(MenuItem::new("Undo").action(move || {
                    goldenide_command.send(GoldenIdeCommand {
                        kind: CommandKind::Edit(EditCommand::Undo),
                        data: None,
                    });
                }))
                .entry(MenuItem::new("Redo").action(move || {
                    goldenide_command.send(GoldenIdeCommand {
                        kind: CommandKind::Edit(EditCommand::Redo),
                        data: None,
                    });
                }))
                .separator()
                .entry(MenuItem::new("Find").action(move || {
                    goldenide_command.send(GoldenIdeCommand {
                        kind: CommandKind::Focus(FocusCommand::Search),
                        data: None,
                    });
                })),
        )
}
