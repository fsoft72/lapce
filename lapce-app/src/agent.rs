//! UI state of the AI agent panel.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use floem::{
    action::exec_after,
    ext_event::create_ext_action,
    keyboard::Modifiers,
    reactive::{Memo, RwSignal, Scope, SignalGet, SignalUpdate, SignalWith},
};
use lapce_core::{
    buffer::rope_text::RopeText,
    command::{EditCommand, FocusCommand},
    editor::EditType,
    mode::Mode,
    selection::Selection,
};
use lapce_rpc::agent::{
    AgentContext, AgentEvent, AgentPermissionOption, AgentRequestId, AgentStatus,
    AgentToolStatus,
};

use lapce_rpc::proxy::ProxyResponse;

use crate::{
    agent_mention::{
        MAX_MENTION_ITEMS, Mention, filter_files, is_excluded, mention_at_end,
        mentioned_paths,
    },
    command::{CommandExecuted, CommandKind, LapceCommand},
    editor::EditorData,
    keypress::{KeyPressFocus, condition::Condition},
    main_split::MainSplitData,
    window_tab::CommonData,
};

/// How long closing a window or the app waits for a proxy to confirm that
/// its agent process was killed.
const AGENT_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a toast stays visible.
const TOAST_DURATION: Duration = Duration::from_millis(1500);

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
    /// The transcript, oldest first. Items are only ever appended or changed
    /// in place, so an index identifies an item for the whole session.
    items: im::Vector<AgentItem>,
    /// Revision of each item, bumped whenever the item changes, so the panel
    /// re-renders an item only when its own revision moves.
    revisions: im::Vector<u64>,
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
            revisions: im::Vector::new(),
            status: AgentStatus::Disconnected {
                reason: "not started".to_string(),
            },
            busy: false,
            pending: VecDeque::new(),
        }
    }
}

impl AgentState {
    /// The transcript, oldest first.
    pub fn items(&self) -> &im::Vector<AgentItem> {
        &self.items
    }

    /// The transcript entry at `index`, if any.
    pub fn item(&self, index: usize) -> Option<&AgentItem> {
        self.items.get(index)
    }

    /// The revision of the transcript entry at `index`, if any.
    pub fn revision(&self, index: usize) -> Option<u64> {
        self.revisions.get(index).copied()
    }

    /// Appends an item to the transcript.
    fn push_item(&mut self, item: AgentItem) {
        self.items.push_back(item);
        self.revisions.push_back(0);
    }

    /// Records that the item at `index` changed.
    fn touch(&mut self, index: usize) {
        if let Some(revision) = self.revisions.get_mut(index) {
            *revision += 1;
        }
    }

    /// Records the user's message and marks the agent as working.
    pub fn push_user(&mut self, text: &str) {
        self.push_item(AgentItem::User(text.to_string()));
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
        self.push_item(AgentItem::Error(message));
    }

    /// Tells the user an agent edit to `path` was left unsaved because the
    /// document already had unsaved changes.
    pub fn note_unsaved_edit(&mut self, path: &Path) {
        self.push_item(AgentItem::Note(format!(
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
            self.touch(self.items.len() - 1);
            return;
        }
        let item = if thought {
            AgentItem::Thought(text.to_string())
        } else {
            AgentItem::Assistant(text.to_string())
        };
        self.push_item(item);
    }

    /// Changes the title and status of the tool calls with `id`.
    fn update_tool(
        &mut self,
        id: &str,
        title: Option<&String>,
        status: Option<AgentToolStatus>,
    ) {
        let mut changed = Vec::new();
        for (index, item) in self.items.iter_mut().enumerate() {
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
                *tool_status = status;
            }
            changed.push(index);
        }
        for index in changed {
            self.touch(index);
        }
    }

    /// Applies one event from the proxy to the state.
    pub fn apply(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::MessageChunk { text } => self.append_text(text, false),
            AgentEvent::ThoughtChunk { text } => self.append_text(text, true),
            AgentEvent::ToolCall { id, title, status } => {
                self.push_item(AgentItem::Tool {
                    id: id.clone(),
                    title: title.clone(),
                    status: *status,
                });
            }
            AgentEvent::ToolCallUpdate { id, title, status } => {
                self.update_tool(id, title.as_ref(), *status);
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
                    self.push_item(AgentItem::Error(format!(
                        "Agent disconnected: {reason}"
                    )));
                }
            }
            AgentEvent::TurnEnded { .. } => self.reset_turn(),
            AgentEvent::Error { message } => {
                self.busy = false;
                self.push_item(AgentItem::Error(message.clone()));
            }
        }
    }
}

/// Focused views of [`AgentState`]. Each memo only notifies its readers when
/// its own value changes, so a streamed chunk does not re-run the toolbar or
/// the permission bar, and a status change does not re-run the transcript.
#[derive(Clone, Copy)]
pub struct AgentStateMemos {
    /// Connection status of the agent.
    pub status: Memo<AgentStatus>,
    /// Whether a turn is running.
    pub busy: Memo<bool>,
    /// The permission request currently shown, if any.
    pub permission: Memo<Option<PendingPermission>>,
    /// Number of transcript entries, which only changes when one is added.
    pub item_count: Memo<usize>,
}

impl AgentStateMemos {
    /// Creates the memos reading `state` in the scope `cx`.
    pub fn new(cx: Scope, state: RwSignal<AgentState>) -> Self {
        Self {
            status: cx.create_memo(move |_| state.with(|s| s.status.clone())),
            busy: cx.create_memo(move |_| state.with(|s| s.busy)),
            permission: cx.create_memo(move |_| {
                state.with(|s| s.current_permission().cloned())
            }),
            item_count: cx.create_memo(move |_| state.with(|s| s.items.len())),
        }
    }
}

/// Reactive wrapper around [`AgentState`] plus the pieces needed to edit buffers.
#[derive(Clone)]
pub struct AgentData {
    /// Transcript, status and pending permission of the agent panel.
    pub state: RwSignal<AgentState>,
    /// Focused views of `state` for the panel to read.
    pub memos: AgentStateMemos,
    /// Open documents, used to apply agent edits to buffers.
    pub main_split: MainSplitData,
    /// Shared window tab data (proxy handle, config, focus).
    pub common: Rc<CommonData>,
    /// The prompt input box.
    pub input: EditorData,
    /// The `@file` mention being typed in the input box, if any.
    pub mention: RwSignal<Option<Mention>>,
    /// Index of the highlighted entry of the mention list.
    pub mention_selected: RwSignal<usize>,
    /// The files matching the mention being typed, best match first.
    pub mention_items: Memo<Vec<String>>,
    /// Workspace files (relative paths) that can be mentioned.
    files: RwSignal<Vec<String>>,
    /// The short message currently shown over the panel, if any.
    pub toast: RwSignal<Option<String>>,
    /// Counts the toasts shown, so an old timer never hides a newer toast.
    toast_seq: RwSignal<u64>,
}

impl AgentData {
    /// Creates the agent data with an empty transcript.
    pub fn new(
        cx: Scope,
        main_split: MainSplitData,
        common: Rc<CommonData>,
    ) -> Self {
        let input = main_split.editors.make_local(cx, common.clone());
        let mention = cx.create_rw_signal(None::<Mention>);
        let files = cx.create_rw_signal(Vec::<String>::new());
        let mention_items = cx.create_memo(move |_| {
            let Some(mention) = mention.get() else {
                return Vec::new();
            };
            files
                .with(|files| filter_files(files, &mention.query, MAX_MENTION_ITEMS))
        });
        let state = cx.create_rw_signal(AgentState::default());
        Self {
            state,
            memos: AgentStateMemos::new(cx, state),
            main_split,
            common,
            input,
            mention,
            mention_selected: cx.create_rw_signal(0),
            mention_items,
            files,
            toast: cx.create_rw_signal(None),
            toast_seq: cx.create_rw_signal(0),
        }
    }

    /// Shows `message` over the panel for [`TOAST_DURATION`].
    pub fn show_toast(&self, message: &str) {
        let id = self.toast_seq.get_untracked() + 1;
        self.toast_seq.set(id);
        self.toast.set(Some(message.to_string()));
        let (toast, seq) = (self.toast, self.toast_seq);
        exec_after(TOAST_DURATION, move |_| {
            if seq.get_untracked() == id {
                toast.set(None);
            }
        });
    }

    /// Whether the mention list is open and has something to pick.
    pub fn mention_visible(&self) -> bool {
        self.mention.with_untracked(|mention| mention.is_some())
            && self.mention_items.with_untracked(|items| !items.is_empty())
    }

    /// Re-reads the text before the cursor to open, update or close the mention list.
    fn update_mention(&self) {
        let offset = self.input.cursor().with_untracked(|cursor| cursor.offset());
        let before = self
            .input
            .doc()
            .buffer
            .with_untracked(|buffer| buffer.slice_to_cow(0..offset).to_string());
        let mention = mention_at_end(&before);
        if mention == self.mention.get_untracked() {
            return;
        }
        if mention.is_some() && self.mention.get_untracked().is_none() {
            self.load_files();
        }
        self.mention_selected.set(0);
        self.mention.set(mention);
    }

    /// Fetches the workspace files for the mention list, leaving out `.git` and `node_modules`.
    fn load_files(&self) {
        let Some(root) = self.common.workspace.path.clone() else {
            return;
        };
        let files = self.files;
        let send =
            create_ext_action(self.common.scope, move |paths: Vec<PathBuf>| {
                let mut relative: Vec<String> = paths
                    .iter()
                    .map(|path| path.strip_prefix(&root).unwrap_or(path))
                    .filter(|path| !is_excluded(path))
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
                relative.sort();
                files.set(relative);
            });
        self.common.proxy.get_files(move |result| {
            if let Ok(ProxyResponse::GetFilesResponse { items }) = result {
                send(items);
            }
        });
    }

    /// Moves the highlight of the mention list by `step` entries, wrapping around.
    fn move_mention_selection(&self, step: isize) {
        let len = self.mention_items.with_untracked(|items| items.len()) as isize;
        if len == 0 {
            return;
        }
        let selected = self.mention_selected.get_untracked() as isize;
        self.mention_selected
            .set((selected + step).rem_euclid(len) as usize);
    }

    /// Replaces the `@query` being typed with the highlighted file.
    pub fn accept_mention(&self) {
        let Some(mention) = self.mention.get_untracked() else {
            return;
        };
        let file = self.mention_items.with_untracked(|items| {
            items.get(self.mention_selected.get_untracked()).cloned()
        });
        let Some(file) = file else {
            return;
        };
        let offset = self.input.cursor().with_untracked(|cursor| cursor.offset());
        let replacement = format!("@{file} ");
        self.input.doc().do_raw_edit(
            &[(
                Selection::region(mention.start, offset),
                replacement.as_str(),
            )],
            EditType::InsertChars,
        );
        let caret = mention.start + replacement.len();
        self.input
            .cursor()
            .update(|cursor| cursor.set_insert(Selection::caret(caret)));
        self.mention.set(None);
    }

    /// The whole files mentioned as `@path` in `text`. Only files of the
    /// workspace list are attached, and the ones already in `contexts` are skipped.
    fn mention_contexts(
        &self,
        text: &str,
        contexts: &[AgentContext],
    ) -> Vec<AgentContext> {
        let Some(root) = self.common.workspace.path.clone() else {
            return Vec::new();
        };
        let files = self.files.get_untracked();
        mentioned_paths(text)
            .into_iter()
            .filter(|path| files.iter().any(|file| file == path))
            .map(|path| root.join(path))
            .filter(|path| contexts.iter().all(|context| &context.path != path))
            .map(|path| AgentContext {
                path,
                selection: None,
            })
            .collect()
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
        // `reset` also moves the cursor to 0: clearing the buffer alone would
        // leave it past the end, and the next keystroke would panic in xi-rope.
        self.input.reset();
        self.mention.set(None);
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
        self.send_text(text);
    }

    /// Sends `text` again as a new prompt, leaving the input box untouched.
    pub fn rerun(&self, text: String) {
        if self.state.with_untracked(|state| state.busy) {
            return;
        }
        self.send_text(text);
    }

    /// Replaces the content of the input box with `text` and puts the caret at its end.
    pub fn edit_in_input(&self, text: &str) {
        let doc = self.input.doc();
        let len = doc.buffer.with_untracked(|buffer| buffer.len());
        doc.do_raw_edit(&[(Selection::region(0, len), text)], EditType::InsertChars);
        self.input
            .cursor()
            .update(|cursor| cursor.set_insert(Selection::caret(text.len())));
        self.mention.set(None);
    }

    /// Pushes `text` to the transcript and sends it to the agent, starting the agent first if needed.
    fn send_text(&self, text: String) {
        let connected = self.state.with_untracked(|state| {
            matches!(state.status, AgentStatus::Ready | AgentStatus::Starting)
        });
        if !connected {
            self.restart();
        }
        self.state.update(|state| state.push_user(&text));
        let mut contexts = self.active_context();
        let mentioned = self.mention_contexts(&text, &contexts);
        contexts.extend(mentioned);
        self.common.proxy.agent_prompt(text, contexts);
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

    /// The agent panel satisfies the panel focus condition; while the mention
    /// list is open it also acts as a list, so Up, Down, Enter, Tab and Esc drive it.
    fn check_condition(&self, condition: Condition) -> bool {
        match condition {
            Condition::PanelFocus => true,
            Condition::ListFocus | Condition::CompletionFocus => {
                self.mention_visible()
            }
            _ => false,
        }
    }

    /// Forwards editing commands to the input box; Enter sends the prompt and
    /// Shift+Enter inserts a new line.
    fn run_command(
        &self,
        command: &LapceCommand,
        count: Option<usize>,
        mods: Modifiers,
    ) -> CommandExecuted {
        match &command.kind {
            CommandKind::Workbench(_) => {}
            CommandKind::Scroll(_) => {}
            CommandKind::Focus(cmd) => {
                if !self.mention_visible() {
                    return CommandExecuted::No;
                }
                match cmd {
                    FocusCommand::ListNext => self.move_mention_selection(1),
                    FocusCommand::ListPrevious => self.move_mention_selection(-1),
                    FocusCommand::ListSelect => self.accept_mention(),
                    FocusCommand::ModalClose => self.mention.set(None),
                    _ => return CommandExecuted::No,
                }
                return CommandExecuted::Yes;
            }
            CommandKind::Edit(_)
            | CommandKind::Move(_)
            | CommandKind::MultiSelection(_) => {
                #[allow(clippy::single_match)]
                match command.kind {
                    CommandKind::Edit(EditCommand::InsertNewLine)
                        if !mods.shift() =>
                    {
                        self.send_input();
                        return CommandExecuted::Yes;
                    }
                    _ => {}
                }

                let executed = self.input.run_command(command, count, mods);
                self.update_mention();
                return executed;
            }
            CommandKind::MotionMode(_) => {}
        }
        CommandExecuted::No
    }

    /// Types a character into the input box.
    fn receive_char(&self, c: &str) {
        self.input.receive_char(c);
        self.update_mention();
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

    /// Counts how many times an effect reading `read` runs.
    fn count_runs(cx: Scope, read: impl Fn() + 'static) -> Rc<std::cell::Cell<u32>> {
        let runs = Rc::new(std::cell::Cell::new(0));
        let counter = runs.clone();
        cx.create_effect(move |_| {
            read();
            counter.set(counter.get() + 1);
        });
        runs
    }

    #[test]
    fn memos_only_notify_their_readers_when_their_value_changes() {
        let cx = Scope::new();
        let state = cx.create_rw_signal(AgentState::default());
        let memos = AgentStateMemos::new(cx, state);
        let status_runs = count_runs(cx, move || memos.status.with(|_| ()));
        let permission_runs = count_runs(cx, move || memos.permission.with(|_| ()));
        assert_eq!((status_runs.get(), permission_runs.get()), (1, 1));

        state.update(|s| s.apply(&AgentEvent::MessageChunk { text: "a".into() }));
        assert_eq!((status_runs.get(), permission_runs.get()), (1, 1));

        state.update(|s| {
            s.apply(&AgentEvent::Status {
                status: AgentStatus::Ready,
            })
        });
        assert_eq!((status_runs.get(), permission_runs.get()), (2, 1));
        assert_eq!(memos.status.get_untracked(), AgentStatus::Ready);

        request_permission_in(state, 3);
        assert_eq!((status_runs.get(), permission_runs.get()), (2, 2));
        assert_eq!(memos.permission.get_untracked().unwrap().request_id, 3);
        cx.dispose();
    }

    #[test]
    fn a_streamed_chunk_bumps_only_the_revision_of_the_growing_reply() {
        let mut state = AgentState::default();
        state.push_user("hi");
        state.apply(&AgentEvent::MessageChunk { text: "a".into() });
        assert_eq!((state.revision(0), state.revision(1)), (Some(0), Some(0)));
        state.apply(&AgentEvent::MessageChunk { text: "b".into() });
        state.apply(&AgentEvent::MessageChunk { text: "c".into() });
        assert_eq!((state.revision(0), state.revision(1)), (Some(0), Some(2)));
        assert_eq!(
            state.item(1),
            Some(&AgentItem::Assistant("abc".to_string()))
        );
        assert_eq!(state.revision(2), None);
    }

    #[test]
    fn a_tool_update_bumps_only_the_revision_of_that_tool() {
        let mut state = AgentState::default();
        for id in ["t1", "t2"] {
            state.apply(&AgentEvent::ToolCall {
                id: id.into(),
                title: "Edit".into(),
                status: AgentToolStatus::Pending,
            });
        }
        state.apply(&AgentEvent::ToolCallUpdate {
            id: "t2".into(),
            title: None,
            status: Some(AgentToolStatus::Completed),
        });
        assert_eq!((state.revision(0), state.revision(1)), (Some(0), Some(1)));
    }

    #[test]
    fn item_count_memo_ignores_chunks_that_extend_the_last_item() {
        let cx = Scope::new();
        let state = cx.create_rw_signal(AgentState::default());
        let memos = AgentStateMemos::new(cx, state);
        let count_runs = count_runs(cx, move || memos.item_count.with(|_| ()));
        state.update(|s| s.apply(&AgentEvent::MessageChunk { text: "a".into() }));
        assert_eq!((memos.item_count.get_untracked(), count_runs.get()), (1, 2));
        state.update(|s| s.apply(&AgentEvent::MessageChunk { text: "b".into() }));
        assert_eq!((memos.item_count.get_untracked(), count_runs.get()), (1, 2));
        cx.dispose();
    }

    #[test]
    fn busy_memo_follows_the_turn() {
        let cx = Scope::new();
        let state = cx.create_rw_signal(AgentState::default());
        let memos = AgentStateMemos::new(cx, state);
        assert!(!memos.busy.get_untracked());
        state.update(|s| s.push_user("hi"));
        assert!(memos.busy.get_untracked());
        state.update(|s| s.reset_turn());
        assert!(!memos.busy.get_untracked());
        cx.dispose();
    }

    /// Queues a permission request with the given id in a reactive state.
    fn request_permission_in(
        state: RwSignal<AgentState>,
        request_id: AgentRequestId,
    ) {
        state.update(|s| request_permission(s, request_id));
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
