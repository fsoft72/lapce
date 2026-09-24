//! UI state of the AI agent panel.

use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    path::Path,
    rc::Rc,
    time::Duration,
};

use floem::{
    keyboard::Modifiers,
    reactive::{RwSignal, Scope, SignalGet, SignalUpdate, SignalWith},
};
use lapce_core::{
    buffer::rope_text::RopeText, command::EditCommand, editor::EditType, mode::Mode,
    selection::Selection,
};
use lapce_rpc::agent::{
    AgentContext, AgentEvent, AgentPermissionOption, AgentRequestId, AgentStatus,
    AgentToolStatus,
};

use crate::{
    command::{CommandExecuted, CommandKind, LapceCommand},
    editor::EditorData,
    keypress::{KeyPressFocus, condition::Condition},
    main_split::MainSplitData,
    window_tab::CommonData,
};

/// How long closing a window or the app waits for a proxy to confirm that
/// its agent process was killed.
const AGENT_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// One entry in the chat transcript.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentItem {
    User(String),
    Assistant(String),
    Thought(String),
    Tool {
        id: String,
        title: String,
        status: AgentToolStatus,
    },
    Error(String),
    /// Information from Lapce itself, such as an agent edit left unsaved.
    Note(String),
}

impl AgentItem {
    /// A key that changes whenever the visible content changes, so the list
    /// view re-renders items that grow while streaming.
    pub fn render_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        match self {
            AgentItem::User(text) => (0u8, text).hash(&mut hasher),
            AgentItem::Assistant(text) => (1u8, text).hash(&mut hasher),
            AgentItem::Thought(text) => (2u8, text).hash(&mut hasher),
            AgentItem::Tool { id, title, status } => {
                (3u8, id, title, *status as u8).hash(&mut hasher)
            }
            AgentItem::Error(text) => (4u8, text).hash(&mut hasher),
            AgentItem::Note(text) => (5u8, text).hash(&mut hasher),
        }
        hasher.finish()
    }
}

/// What to do with an agent write to an open document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEditPlan {
    /// The document already has the content.
    Skip,
    /// The document is read-only: nothing changes and the user is told.
    Refuse,
    /// Apply the edit, then save it: the document had no unsaved changes,
    /// so the disk (which the agent's own tools read) matches the buffer.
    ApplyAndSave,
    /// Apply the edit but leave it unsaved, so the user's own unsaved
    /// changes are not written without their consent.
    ApplyUnsaved,
}

/// Decides how to apply an agent write to an open document.
pub fn plan_agent_edit(
    read_only: bool,
    unchanged: bool,
    pristine: bool,
) -> AgentEditPlan {
    if read_only {
        return AgentEditPlan::Refuse;
    }
    if unchanged {
        return AgentEditPlan::Skip;
    }
    if pristine {
        AgentEditPlan::ApplyAndSave
    } else {
        AgentEditPlan::ApplyUnsaved
    }
}

/// A permission request waiting for the user's decision.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingPermission {
    pub request_id: AgentRequestId,
    pub title: String,
    pub options: Vec<AgentPermissionOption>,
}

/// Everything the panel displays. Plain data so it can be tested without a UI.
#[derive(Debug, Clone)]
pub struct AgentState {
    pub items: im::Vector<AgentItem>,
    pub status: AgentStatus,
    pub busy: bool,
    /// Permission requests waiting for a decision, oldest first. Only the
    /// first one is shown; answering it reveals the next.
    pub pending: VecDeque<PendingPermission>,
}

impl Default for AgentState {
    /// An empty transcript with no agent running.
    fn default() -> Self {
        Self {
            items: im::Vector::new(),
            status: AgentStatus::Disconnected {
                reason: "not started".to_string(),
            },
            busy: false,
            pending: VecDeque::new(),
        }
    }
}

impl AgentState {
    /// Records the user's message and marks the agent as working.
    pub fn push_user(&mut self, text: &str) {
        self.items.push_back(AgentItem::User(text.to_string()));
        self.busy = true;
    }

    /// Removes and returns the permission request currently shown, if any.
    pub fn take_pending(&mut self) -> Option<PendingPermission> {
        self.pending.pop_front()
    }

    /// The permission request currently shown, if any.
    pub fn current_permission(&self) -> Option<&PendingPermission> {
        self.pending.front()
    }

    /// Adds an error to the transcript without ending the turn.
    pub fn push_error(&mut self, message: String) {
        self.items.push_back(AgentItem::Error(message));
    }

    /// Tells the user an agent edit to `path` was left unsaved because the
    /// document already had unsaved changes.
    pub fn note_unsaved_edit(&mut self, path: &Path) {
        self.items.push_back(AgentItem::Note(format!(
            "The agent edited {}, which has unsaved changes: its edit is not saved \
             either, so the agent's own tools still see the file on disk.",
            path.display()
        )));
    }

    /// Forgets the running turn: the agent is no longer busy and every
    /// pending permission request is dropped. Used when the turn ends and
    /// when the session is replaced, whose events will never arrive.
    pub fn reset_turn(&mut self) {
        self.busy = false;
        self.pending.clear();
    }

    /// Appends streamed text to the last item when it has the same kind,
    /// otherwise starts a new item.
    fn append_text(&mut self, text: &str, thought: bool) {
        let merged = match self.items.back_mut() {
            Some(AgentItem::Assistant(existing)) if !thought => {
                existing.push_str(text);
                true
            }
            Some(AgentItem::Thought(existing)) if thought => {
                existing.push_str(text);
                true
            }
            _ => false,
        };
        if merged {
            return;
        }
        let item = if thought {
            AgentItem::Thought(text.to_string())
        } else {
            AgentItem::Assistant(text.to_string())
        };
        self.items.push_back(item);
    }

    /// Applies one event from the proxy to the state.
    pub fn apply(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::MessageChunk { text } => self.append_text(text, false),
            AgentEvent::ThoughtChunk { text } => self.append_text(text, true),
            AgentEvent::ToolCall { id, title, status } => {
                self.items.push_back(AgentItem::Tool {
                    id: id.clone(),
                    title: title.clone(),
                    status: *status,
                });
            }
            AgentEvent::ToolCallUpdate { id, title, status } => {
                for item in self.items.iter_mut() {
                    let AgentItem::Tool {
                        id: tool_id,
                        title: tool_title,
                        status: tool_status,
                    } = item
                    else {
                        continue;
                    };
                    if tool_id != id {
                        continue;
                    }
                    if let Some(title) = title {
                        *tool_title = title.clone();
                    }
                    if let Some(status) = status {
                        *tool_status = *status;
                    }
                }
            }
            AgentEvent::PermissionRequest {
                request_id,
                title,
                options,
            } => {
                self.pending.push_back(PendingPermission {
                    request_id: *request_id,
                    title: title.clone(),
                    options: options.clone(),
                });
            }
            AgentEvent::Status { status } => {
                self.status = status.clone();
                if let AgentStatus::Disconnected { reason } = status {
                    self.reset_turn();
                    self.items.push_back(AgentItem::Error(format!(
                        "Agent disconnected: {reason}"
                    )));
                }
            }
            AgentEvent::TurnEnded { .. } => self.reset_turn(),
            AgentEvent::Error { message } => {
                self.busy = false;
                self.items.push_back(AgentItem::Error(message.clone()));
            }
        }
    }
}

/// Reactive wrapper around [`AgentState`] plus the pieces needed to edit buffers.
#[derive(Clone)]
pub struct AgentData {
    /// Transcript, status and pending permission of the agent panel.
    pub state: RwSignal<AgentState>,
    /// Open documents, used to apply agent edits to buffers.
    pub main_split: MainSplitData,
    /// Shared window tab data (proxy handle, config, focus).
    pub common: Rc<CommonData>,
    /// The prompt input box.
    pub input: EditorData,
}

impl AgentData {
    /// Creates the agent data with an empty transcript.
    pub fn new(
        cx: Scope,
        main_split: MainSplitData,
        common: Rc<CommonData>,
    ) -> Self {
        let input = main_split.editors.make_local(cx, common.clone());
        Self {
            state: cx.create_rw_signal(AgentState::default()),
            main_split,
            common,
            input,
        }
    }

    /// Applies an event coming from the proxy.
    pub fn handle_event(&self, event: AgentEvent) {
        self.state.update(|state| state.apply(&event));
    }

    /// Replaces the whole content of an open document as one undoable edit.
    /// The edit is saved when the document had no unsaved changes, so the
    /// disk matches what the agent was told it wrote; otherwise it is left
    /// unsaved and a note says so. A document that is missing or read-only
    /// is reported as an error.
    pub fn apply_edit(&self, path: &Path, content: &str) {
        let Some(doc) = self
            .main_split
            .docs
            .with_untracked(|docs| docs.get(path).cloned())
        else {
            return self.report_edit_error(path, "the file is not open");
        };
        let read_only = doc.content.with_untracked(|content| content.read_only());
        let (len, unchanged) = doc
            .buffer
            .with_untracked(|buffer| (buffer.len(), buffer.to_string() == content));
        match plan_agent_edit(read_only, unchanged, doc.is_pristine()) {
            AgentEditPlan::Skip => {}
            AgentEditPlan::Refuse => {
                self.report_edit_error(path, "the file is read-only");
            }
            AgentEditPlan::ApplyAndSave => {
                doc.do_raw_edit(
                    &[(Selection::region(0, len), content)],
                    EditType::Other,
                );
                doc.save(|| {});
            }
            AgentEditPlan::ApplyUnsaved => {
                doc.do_raw_edit(
                    &[(Selection::region(0, len), content)],
                    EditType::Other,
                );
                self.state.update(|state| state.note_unsaved_edit(path));
            }
        }
    }

    /// Logs and shows an agent edit that could not be applied to `path`.
    fn report_edit_error(&self, path: &Path, reason: &str) {
        let message = format!(
            "The agent's edit to {} was not applied: {reason}",
            path.display()
        );
        tracing::error!("{message}");
        self.state.update(|state| state.push_error(message));
    }

    /// Reads and clears the input box. Returns `None` when it only has whitespace.
    fn take_input_text(&self) -> Option<String> {
        let doc = self.input.doc();
        let text = doc.buffer.with_untracked(|buffer| buffer.to_string());
        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let len = doc.buffer.with_untracked(|buffer| buffer.len());
        doc.do_raw_edit(&[(Selection::region(0, len), "")], EditType::Other);
        Some(text)
    }

    /// Builds the context attached to a prompt: the active file, and its selection if any.
    fn active_context(&self) -> Vec<AgentContext> {
        let Some(editor) = self.main_split.active_editor.get_untracked() else {
            return Vec::new();
        };
        let doc = editor.doc();
        let Some(path) = doc
            .content
            .with_untracked(|content| content.path().cloned())
        else {
            return Vec::new();
        };
        let selection = editor
            .cursor()
            .with_untracked(|cursor| cursor.get_selection())
            .filter(|(start, end)| start != end)
            .map(|(start, end)| {
                doc.buffer.with_untracked(|buffer| {
                    buffer.slice_to_cow(start..end).to_string()
                })
            });
        vec![AgentContext { path, selection }]
    }

    /// Starts the configured agent server. Reports configuration problems in the transcript.
    /// The running turn is forgotten first: the replaced session is silenced,
    /// so its `TurnEnded` would never arrive and the panel would stay busy.
    pub fn restart(&self) {
        self.state.update(|state| state.reset_turn());
        match self.common.config.get_untracked().agent.resolve() {
            Ok(config) => self.common.proxy.agent_start(config),
            Err(message) => self.handle_event(AgentEvent::Error { message }),
        }
    }

    /// Sends the input box content as a prompt, starting the agent first if needed.
    pub fn send_input(&self) {
        if self.state.with_untracked(|state| state.busy) {
            return;
        }
        let Some(text) = self.take_input_text() else {
            return;
        };
        let connected = self.state.with_untracked(|state| {
            matches!(state.status, AgentStatus::Ready | AgentStatus::Starting)
        });
        if !connected {
            self.restart();
        }
        self.state.update(|state| state.push_user(&text));
        self.common.proxy.agent_prompt(text, self.active_context());
    }

    /// Stops the agent before its window or the app closes, blocking up to
    /// [`AGENT_STOP_TIMEOUT`] until the proxy confirms the process was
    /// killed: without the wait the app could exit first and orphan it.
    /// Does nothing when no agent is running.
    pub fn stop_before_exit(&self) {
        let running = self.state.with_untracked(|state| {
            !matches!(state.status, AgentStatus::Disconnected { .. })
        });
        if !running {
            return;
        }
        if !self.common.proxy.agent_stop(AGENT_STOP_TIMEOUT) {
            tracing::warn!("the proxy did not confirm that the agent was stopped");
        }
    }

    /// Cancels the running turn.
    pub fn cancel(&self) {
        self.common.proxy.agent_cancel();
    }

    /// Answers the pending permission request with `option_id`, or rejects it with `None`.
    pub fn reply_permission(&self, option_id: Option<String>) {
        let Some(pending) = self
            .state
            .try_update(|state| state.take_pending())
            .flatten()
        else {
            return;
        };
        self.common
            .proxy
            .agent_permission_reply(pending.request_id, option_id);
    }
}

impl std::fmt::Debug for AgentData {
    /// Prints the state only; `MainSplitData` does not implement `Debug`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentData")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl KeyPressFocus for AgentData {
    /// The input box always behaves like an insert-mode text field.
    fn get_mode(&self) -> Mode {
        Mode::Insert
    }

    /// The agent panel only satisfies the panel focus condition.
    fn check_condition(&self, condition: Condition) -> bool {
        matches!(condition, Condition::PanelFocus)
    }

    /// Forwards editing commands to the input box; Enter sends the prompt.
    fn run_command(
        &self,
        command: &LapceCommand,
        count: Option<usize>,
        mods: Modifiers,
    ) -> CommandExecuted {
        match &command.kind {
            CommandKind::Workbench(_) => {}
            CommandKind::Scroll(_) => {}
            CommandKind::Focus(_) => {}
            CommandKind::Edit(_)
            | CommandKind::Move(_)
            | CommandKind::MultiSelection(_) => {
                #[allow(clippy::single_match)]
                match command.kind {
                    CommandKind::Edit(EditCommand::InsertNewLine) => {
                        self.send_input();
                        return CommandExecuted::Yes;
                    }
                    _ => {}
                }

                return self.input.run_command(command, count, mods);
            }
            CommandKind::MotionMode(_) => {}
        }
        CommandExecuted::No
    }

    /// Types a character into the input box.
    fn receive_char(&self, c: &str) {
        self.input.receive_char(c);
    }
}

#[cfg(test)]
mod tests {
    use lapce_rpc::agent::{AgentStatus, AgentToolStatus};

    use super::*;

    #[test]
    fn message_chunks_extend_the_last_assistant_item() {
        let mut state = AgentState::default();
        state.apply(&AgentEvent::MessageChunk { text: "Hel".into() });
        state.apply(&AgentEvent::MessageChunk { text: "lo".into() });
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0], AgentItem::Assistant("Hello".to_string()));
    }

    #[test]
    fn a_tool_call_between_chunks_starts_a_new_assistant_item() {
        let mut state = AgentState::default();
        state.apply(&AgentEvent::MessageChunk { text: "a".into() });
        state.apply(&AgentEvent::ToolCall {
            id: "t1".into(),
            title: "Edit".into(),
            status: AgentToolStatus::Pending,
        });
        state.apply(&AgentEvent::MessageChunk { text: "b".into() });
        assert_eq!(state.items.len(), 3);
    }

    #[test]
    fn tool_update_changes_the_matching_tool() {
        let mut state = AgentState::default();
        state.apply(&AgentEvent::ToolCall {
            id: "t1".into(),
            title: "Edit".into(),
            status: AgentToolStatus::Pending,
        });
        state.apply(&AgentEvent::ToolCallUpdate {
            id: "t1".into(),
            title: None,
            status: Some(AgentToolStatus::Completed),
        });
        assert_eq!(
            state.items[0],
            AgentItem::Tool {
                id: "t1".to_string(),
                title: "Edit".to_string(),
                status: AgentToolStatus::Completed,
            }
        );
    }

    #[test]
    fn push_user_marks_the_state_busy_and_turn_end_clears_it() {
        let mut state = AgentState::default();
        state.push_user("hi");
        assert!(state.busy);
        state.apply(&AgentEvent::TurnEnded {
            stop_reason: "EndTurn".into(),
        });
        assert!(!state.busy);
    }

    #[test]
    fn disconnect_resets_busy_clears_permission_and_reports_the_reason() {
        let mut state = AgentState::default();
        state.push_user("hi");
        state.apply(&AgentEvent::PermissionRequest {
            request_id: 1,
            title: "Edit".into(),
            options: vec![],
        });
        assert!(!state.pending.is_empty());
        state.apply(&AgentEvent::Status {
            status: AgentStatus::Disconnected {
                reason: "exit 1".into(),
            },
        });
        assert!(!state.busy);
        assert!(state.pending.is_empty());
        assert!(matches!(
            state.items.back(),
            Some(AgentItem::Error(message)) if message.contains("exit 1")
        ));
    }

    #[test]
    fn error_event_adds_an_error_item_and_stops_busy() {
        let mut state = AgentState::default();
        state.push_user("hi");
        state.apply(&AgentEvent::Error {
            message: "boom".into(),
        });
        assert!(!state.busy);
        assert_eq!(
            state.items.back(),
            Some(&AgentItem::Error("boom".to_string()))
        );
    }

    #[test]
    fn take_pending_removes_the_request() {
        let mut state = AgentState::default();
        state.apply(&AgentEvent::PermissionRequest {
            request_id: 4,
            title: "Run".into(),
            options: vec![],
        });
        assert_eq!(state.take_pending().unwrap().request_id, 4);
        assert!(state.take_pending().is_none());
    }

    /// Queues a permission request with the given id.
    fn request_permission(state: &mut AgentState, request_id: AgentRequestId) {
        state.apply(&AgentEvent::PermissionRequest {
            request_id,
            title: format!("Request {request_id}"),
            options: vec![],
        });
    }

    #[test]
    fn reset_turn_clears_busy_and_every_pending_permission() {
        let mut state = AgentState::default();
        state.push_user("hi");
        request_permission(&mut state, 1);
        request_permission(&mut state, 2);
        state.reset_turn();
        assert!(!state.busy);
        assert!(state.pending.is_empty());
    }

    #[test]
    fn a_new_session_status_after_reset_lets_the_user_send_again() {
        // Restart during a turn: the replaced session never sends TurnEnded,
        // so only the reset done by restart can clear `busy`.
        let mut state = AgentState::default();
        state.push_user("hi");
        request_permission(&mut state, 1);
        state.reset_turn();
        state.apply(&AgentEvent::Status {
            status: AgentStatus::Starting,
        });
        state.apply(&AgentEvent::Status {
            status: AgentStatus::Ready,
        });
        assert!(!state.busy);
        assert!(state.current_permission().is_none());
        state.push_user("again");
        assert!(state.busy);
    }

    #[test]
    fn cancelling_a_prompt_queued_while_starting_ends_the_turn() {
        // The proxy reports a dropped queued prompt as a cancelled turn.
        let mut state = AgentState::default();
        state.apply(&AgentEvent::Status {
            status: AgentStatus::Starting,
        });
        state.push_user("hi");
        state.apply(&AgentEvent::TurnEnded {
            stop_reason: "Cancelled".into(),
        });
        assert!(!state.busy);
    }

    #[test]
    fn permission_requests_queue_and_are_answered_in_order() {
        let mut state = AgentState::default();
        request_permission(&mut state, 1);
        request_permission(&mut state, 2);
        assert_eq!(state.current_permission().unwrap().request_id, 1);
        assert_eq!(state.take_pending().unwrap().request_id, 1);
        assert_eq!(state.current_permission().unwrap().request_id, 2);
        assert_eq!(state.take_pending().unwrap().request_id, 2);
        assert!(state.take_pending().is_none());
    }

    #[test]
    fn turn_end_clears_the_permission_queue() {
        let mut state = AgentState::default();
        state.push_user("hi");
        request_permission(&mut state, 1);
        request_permission(&mut state, 2);
        state.apply(&AgentEvent::TurnEnded {
            stop_reason: "EndTurn".into(),
        });
        assert!(state.pending.is_empty());
    }

    #[test]
    fn agent_edit_plan_saves_only_documents_without_unsaved_changes() {
        assert_eq!(
            plan_agent_edit(false, false, true),
            AgentEditPlan::ApplyAndSave
        );
        assert_eq!(
            plan_agent_edit(false, false, false),
            AgentEditPlan::ApplyUnsaved
        );
        assert_eq!(plan_agent_edit(false, true, false), AgentEditPlan::Skip);
    }

    #[test]
    fn agent_edit_plan_refuses_read_only_documents() {
        assert_eq!(plan_agent_edit(true, false, true), AgentEditPlan::Refuse);
        assert_eq!(plan_agent_edit(true, true, true), AgentEditPlan::Refuse);
    }

    #[test]
    fn an_unsaved_edit_note_names_the_file_and_keeps_the_turn_running() {
        let mut state = AgentState::default();
        state.push_user("hi");
        state.note_unsaved_edit(Path::new("/ws/src/main.rs"));
        state.push_error("boom".to_string());
        assert!(state.busy, "notes and edit errors do not end the turn");
        assert!(matches!(
            &state.items[1],
            AgentItem::Note(text) if text.contains("/ws/src/main.rs")
        ));
    }
}
