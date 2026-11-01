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

/// What a session setting chosen in the chat box controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentConfigKind {
    /// The model that answers.
    Model,
    /// How much the model thinks (the effort).
    Effort,
}

/// One value a session setting can take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentConfigChoice {
    pub value: String,
    pub name: String,
}

/// A session setting the agent lets the user change, with its current value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentConfigOption {
    /// Identifier sent back when the user picks another value.
    pub id: String,
    pub name: String,
    pub kind: AgentConfigKind,
    /// The value in use.
    pub current: String,
    pub choices: Vec<AgentConfigChoice>,
}

impl AgentConfigOption {
    /// The name of the current value, or the raw value when it is not listed.
    pub fn current_name(&self) -> &str {
        self.choices
            .iter()
            .find(|choice| choice.value == self.current)
            .map_or(self.current.as_str(), |choice| choice.name.as_str())
    }

    /// The text of the chat box button: the setting name followed by the
    /// current value, unless the value already starts with the name (such as
    /// "Thinking: high" for the setting "Thinking").
    pub fn label(&self) -> String {
        let value = self.current_name();
        if value.to_lowercase().starts_with(&self.name.to_lowercase()) {
            return value.to_string();
        }
        format!("{}: {value}", self.name)
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
    /// The agent changed a file, reported by a completed edit tool through an
    /// ACP diff block. `old_text` is `None` when the file did not exist.
    FileChange {
        path: PathBuf,
        old_text: Option<String>,
        new_text: String,
    },
    PermissionRequest {
        request_id: AgentRequestId,
        title: String,
        options: Vec<AgentPermissionOption>,
    },
    Status {
        status: AgentStatus,
    },
    /// The model and effort settings the agent offers, with their values.
    /// Replaces any earlier list.
    ConfigOptions {
        options: Vec<AgentConfigOption>,
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
    fn config_options_roundtrip_and_name_the_current_value() {
        let option = AgentConfigOption {
            id: "model".to_string(),
            name: "Model".to_string(),
            kind: AgentConfigKind::Model,
            current: "opus".to_string(),
            choices: vec![AgentConfigChoice {
                value: "opus".to_string(),
                name: "Opus".to_string(),
            }],
        };
        assert_eq!(option.current_name(), "Opus");
        roundtrip(AgentEvent::ConfigOptions {
            options: vec![option],
        });
    }

    #[test]
    fn the_label_does_not_repeat_the_setting_name() {
        let mut option = AgentConfigOption {
            id: "t".to_string(),
            name: "Thinking".to_string(),
            kind: AgentConfigKind::Effort,
            current: "high".to_string(),
            choices: vec![AgentConfigChoice {
                value: "high".to_string(),
                name: "Thinking: high".to_string(),
            }],
        };
        assert_eq!(option.label(), "Thinking: high");
        option.choices[0].name = "High".to_string();
        assert_eq!(option.label(), "Thinking: High");
    }

    #[test]
    fn an_unlisted_current_value_is_shown_as_is() {
        let option = AgentConfigOption {
            id: "model".to_string(),
            name: "Model".to_string(),
            kind: AgentConfigKind::Model,
            current: "custom".to_string(),
            choices: vec![],
        };
        assert_eq!(option.current_name(), "custom");
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
