//! UI state of the AI agent panel.

use std::{
    hash::{Hash, Hasher},
    path::Path,
    rc::Rc,
};

use floem::reactive::{RwSignal, Scope, SignalUpdate, SignalWith};
use lapce_core::{
    buffer::rope_text::RopeText, editor::EditType, selection::Selection,
};
use lapce_rpc::agent::{
    AgentEvent, AgentPermissionOption, AgentRequestId, AgentStatus, AgentToolStatus,
};

use crate::{main_split::MainSplitData, window_tab::CommonData};

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
        }
        hasher.finish()
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
    pub pending: Option<PendingPermission>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            items: im::Vector::new(),
            status: AgentStatus::Disconnected {
                reason: "not started".to_string(),
            },
            busy: false,
            pending: None,
        }
    }
}

impl AgentState {
    /// Records the user's message and marks the agent as working.
    pub fn push_user(&mut self, text: &str) {
        self.items.push_back(AgentItem::User(text.to_string()));
        self.busy = true;
    }

    /// Removes and returns the pending permission request, if any.
    pub fn take_pending(&mut self) -> Option<PendingPermission> {
        self.pending.take()
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
                self.pending = Some(PendingPermission {
                    request_id: *request_id,
                    title: title.clone(),
                    options: options.clone(),
                });
            }
            AgentEvent::Status { status } => {
                self.status = status.clone();
                if let AgentStatus::Disconnected { reason } = status {
                    self.busy = false;
                    self.pending = None;
                    self.items.push_back(AgentItem::Error(format!(
                        "Agent disconnected: {reason}"
                    )));
                }
            }
            AgentEvent::TurnEnded { .. } => {
                self.busy = false;
                self.pending = None;
            }
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
}

impl AgentData {
    /// Creates the agent data with an empty transcript.
    pub fn new(
        cx: Scope,
        main_split: MainSplitData,
        common: Rc<CommonData>,
    ) -> Self {
        Self {
            state: cx.create_rw_signal(AgentState::default()),
            main_split,
            common,
        }
    }

    /// Applies an event coming from the proxy.
    pub fn handle_event(&self, event: AgentEvent) {
        self.state.update(|state| state.apply(&event));
    }

    /// Replaces the whole content of an open document as one undoable edit.
    /// Does nothing if the document is not open or the text is unchanged.
    pub fn apply_edit(&self, path: &Path, content: &str) {
        let Some(doc) = self
            .main_split
            .docs
            .with_untracked(|docs| docs.get(path).cloned())
        else {
            return;
        };
        let (len, same) = doc
            .buffer
            .with_untracked(|buffer| (buffer.len(), buffer.to_string() == content));
        if same {
            return;
        }
        doc.do_raw_edit(&[(Selection::region(0, len), content)], EditType::Other);
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
        assert!(state.pending.is_some());
        state.apply(&AgentEvent::Status {
            status: AgentStatus::Disconnected {
                reason: "exit 1".into(),
            },
        });
        assert!(!state.busy);
        assert!(state.pending.is_none());
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
}
