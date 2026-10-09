//! Font zoom with Ctrl and the mouse wheel.

use floem::pointer::PointerWheelEvent;

use crate::editor::EditorData;
use crate::{
    command::{CommandKind, LapceCommand, LapceWorkbenchCommand},
    listener::Listener,
};

/// Font points changed by each wheel event while the zoom modifier is held.
const WHEEL_FONT_STEP: i64 = 2;

/// Sends `increase` (wheel down) or `decrease` (wheel up) when the wheel is
/// used with the zoom modifier (`Cmd` on macOS, `Ctrl` elsewhere). Returns
/// `true` if the event was consumed and must not scroll the view.
pub fn zoom_font_on_wheel(
    lapce_command: Listener<LapceCommand>,
    event: &PointerWheelEvent,
    increase: LapceWorkbenchCommand,
    decrease: LapceWorkbenchCommand,
) -> bool {
    let zoom = if cfg!(target_os = "macos") {
        event.modifiers.meta()
    } else {
        event.modifiers.control()
    };
    if !zoom || event.delta.y == 0.0 {
        return false;
    }
    let command = if event.delta.y < 0.0 {
        increase
    } else {
        decrease
    };
    lapce_command.send(LapceCommand {
        kind: CommandKind::Workbench(command),
        data: Some(serde_json::json!(WHEEL_FONT_STEP)),
    });
    true
}

/// Wheel zoom of the editor font.
pub(super) fn zoom_editor_font_on_wheel(
    e_data: &EditorData,
    event: &PointerWheelEvent,
) -> bool {
    zoom_font_on_wheel(
        e_data.common.lapce_command,
        event,
        LapceWorkbenchCommand::EditorFontIncrease,
        LapceWorkbenchCommand::EditorFontDecrease,
    )
}
