//! The window context menu.

use floem::menu::{Menu, MenuItem};
use lapce_core::command::{EditCommand, FocusCommand};

use crate::{
    command::{CommandKind, LapceCommand, LapceWorkbenchCommand},
    listener::Listener,
};

pub fn window_menu(
    lapce_command: Listener<LapceCommand>,
    workbench_command: Listener<LapceWorkbenchCommand>,
) -> Menu {
    Menu::new("Lapce")
        .entry({
            let mut menu = Menu::new("Lapce")
                .entry(MenuItem::new("About Lapce").action(move || {
                    workbench_command.send(LapceWorkbenchCommand::ShowAbout)
                }))
                .separator()
                .entry(
                    Menu::new("Settings...")
                        .entry(MenuItem::new("Open Settings").action(move || {
                            workbench_command
                                .send(LapceWorkbenchCommand::OpenSettings);
                        }))
                        .entry(MenuItem::new("Open Keyboard Shortcuts").action(
                            move || {
                                workbench_command.send(
                                    LapceWorkbenchCommand::OpenKeyboardShortcuts,
                                );
                            },
                        )),
                )
                .separator()
                .entry(MenuItem::new("Quit Lapce").action(move || {
                    workbench_command.send(LapceWorkbenchCommand::Quit);
                }));
            if cfg!(target_os = "macos") {
                menu = menu
                    .separator()
                    .entry(MenuItem::new("Hide Lapce"))
                    .entry(MenuItem::new("Hide Others"))
                    .entry(MenuItem::new("Show All"))
            }
            menu
        })
        .separator()
        .entry(
            Menu::new("File")
                .entry(MenuItem::new("New File").action(move || {
                    workbench_command.send(LapceWorkbenchCommand::NewFile);
                }))
                .separator()
                .entry(MenuItem::new("Open").action(move || {
                    workbench_command.send(LapceWorkbenchCommand::OpenFile);
                }))
                .entry(MenuItem::new("Open Folder").action(move || {
                    workbench_command.send(LapceWorkbenchCommand::OpenFolder);
                }))
                .separator()
                .entry(MenuItem::new("Save").action(move || {
                    lapce_command.send(LapceCommand {
                        kind: CommandKind::Focus(FocusCommand::Save),
                        data: None,
                    });
                }))
                .entry(MenuItem::new("Save All").action(move || {
                    workbench_command.send(LapceWorkbenchCommand::SaveAll);
                }))
                .separator()
                .entry(MenuItem::new("Close Folder").action(move || {
                    workbench_command.send(LapceWorkbenchCommand::CloseFolder);
                }))
                .entry(MenuItem::new("Close Window").action(move || {
                    workbench_command.send(LapceWorkbenchCommand::CloseWindow);
                })),
        )
        .entry(
            Menu::new("Edit")
                .entry(MenuItem::new("Cut").action(move || {
                    lapce_command.send(LapceCommand {
                        kind: CommandKind::Edit(EditCommand::ClipboardCut),
                        data: None,
                    });
                }))
                .entry(MenuItem::new("Copy").action(move || {
                    lapce_command.send(LapceCommand {
                        kind: CommandKind::Edit(EditCommand::ClipboardCopy),
                        data: None,
                    });
                }))
                .entry(MenuItem::new("Paste").action(move || {
                    lapce_command.send(LapceCommand {
                        kind: CommandKind::Edit(EditCommand::ClipboardPaste),
                        data: None,
                    });
                }))
                .separator()
                .entry(MenuItem::new("Undo").action(move || {
                    lapce_command.send(LapceCommand {
                        kind: CommandKind::Edit(EditCommand::Undo),
                        data: None,
                    });
                }))
                .entry(MenuItem::new("Redo").action(move || {
                    lapce_command.send(LapceCommand {
                        kind: CommandKind::Edit(EditCommand::Redo),
                        data: None,
                    });
                }))
                .separator()
                .entry(MenuItem::new("Find").action(move || {
                    lapce_command.send(LapceCommand {
                        kind: CommandKind::Focus(FocusCommand::Search),
                        data: None,
                    });
                })),
        )
}
