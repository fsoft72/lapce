//! Types shared between the UI and the proxy for the AI agent (ACP) panel.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Identifier of a permission request raised by the agent.
pub type AgentRequestId = u64;

/// How to launch an ACP agent subprocess.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// A piece of editor context attached to a prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentContext {
    pub path: PathBuf,
    /// Selected text. When `None` the whole file is attached.
    pub selection: Option<String>,
}

/// The kind of a permission option offered by the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

/// One choice the user can pick when the agent asks for permission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentPermissionOption {
    pub id: String,
    pub name: String,
    pub kind: AgentPermissionKind,
}

/// Execution status of a tool call shown in the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// Connection status of the agent process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentStatus {
    Starting,
    Ready,
    Disconnected { reason: String },
}

impl std::fmt::Display for AgentStatus {
    /// A short human readable form for the panel's status label.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Starting => write!(f, "Starting..."),
            AgentStatus::Ready => write!(f, "Ready"),
            AgentStatus::Disconnected { reason } => {
                write!(f, "Disconnected: {reason}")
            }
        }
    }
}

/// Events streamed from the proxy to the UI while an agent session runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    MessageChunk {
        text: String,
    },
    ThoughtChunk {
        text: String,
    },
    ToolCall {
        id: String,
        title: String,
        status: AgentToolStatus,
    },
    ToolCallUpdate {
        id: String,
        title: Option<String>,
        status: Option<AgentToolStatus>,
    },
    PermissionRequest {
        request_id: AgentRequestId,
        title: String,
        options: Vec<AgentPermissionOption>,
    },
    Status {
        status: AgentStatus,
    },
    TurnEnded {
        stop_reason: String,
    },
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes then deserializes an event and checks the JSON is stable.
    fn roundtrip(event: AgentEvent) {
        let json = serde_json::to_string(&event).unwrap();
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn permission_request_roundtrips() {
        roundtrip(AgentEvent::PermissionRequest {
            request_id: 7,
            title: "Edit main.rs".to_string(),
            options: vec![AgentPermissionOption {
                id: "allow".to_string(),
                name: "Allow".to_string(),
                kind: AgentPermissionKind::AllowOnce,
            }],
        });
    }

    #[test]
    fn status_and_tool_events_roundtrip() {
        roundtrip(AgentEvent::Status {
            status: AgentStatus::Disconnected {
                reason: "exit 1".to_string(),
            },
        });
        roundtrip(AgentEvent::ToolCallUpdate {
            id: "t1".to_string(),
            title: None,
            status: Some(AgentToolStatus::Completed),
        });
    }

    #[test]
    fn status_displays_in_a_readable_form() {
        assert_eq!(AgentStatus::Starting.to_string(), "Starting...");
        assert_eq!(AgentStatus::Ready.to_string(), "Ready");
        let status = AgentStatus::Disconnected {
            reason: "exit 1".to_string(),
        };
        assert_eq!(status.to_string(), "Disconnected: exit 1");
    }
}
