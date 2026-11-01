use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use structdesc::FieldNames;

/// Top-level settings table of the agent panel. It names commands that Lapce
/// runs, so `LapceConfig` takes it only from the defaults and the user
/// settings file, never from themes or a workspace `.lapce/settings.toml`.
pub const AGENT_SETTINGS_KEY: &str = "agent";

/// How to launch one ACP agent server.
#[derive(FieldNames, Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct AgentServerSetting {
    #[field_names(desc = "Command that starts the ACP agent")]
    pub command: String,
    #[serde(default)]
    #[field_names(desc = "Arguments passed to the command")]
    pub arguments: Vec<String>,
    #[serde(default)]
    #[field_names(desc = "Extra environment variables for the agent process")]
    pub environment: HashMap<String, String>,
}

/// Settings of the AI agent panel.
#[derive(FieldNames, Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct AgentConfig {
    #[field_names(desc = "Name of the agent server started from the agent panel")]
    pub default_server: String,
    #[serde(default = "default_true")]
    #[field_names(desc = "Show the agent's thinking in the chat")]
    pub show_thinking: bool,
    #[serde(default = "default_true")]
    #[field_names(desc = "Show the agent's tool calls in the chat")]
    pub show_tools: bool,
    #[field_names(skip)]
    pub servers: HashMap<String, AgentServerSetting>,
}

/// Serde default of the options that are on unless the user turns them off.
fn default_true() -> bool {
    true
}

impl AgentConfig {
    /// Resolves the default server into the launch config sent to the proxy.
    /// Returns a user-facing message when the setting is unusable.
    pub fn resolve(&self) -> Result<lapce_rpc::agent::AgentServerConfig, String> {
        let Some(server) = self.servers.get(&self.default_server) else {
            let mut names: Vec<_> = self.servers.keys().cloned().collect();
            names.sort();
            return Err(format!(
                "agent server '{}' is not configured (available: {})",
                self.default_server,
                names.join(", ")
            ));
        };
        if server.command.trim().is_empty() {
            return Err(format!(
                "agent server '{}' has an empty command",
                self.default_server
            ));
        }
        Ok(lapce_rpc::agent::AgentServerConfig {
            command: server.command.clone(),
            args: server.arguments.clone(),
            env: server.environment.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an `AgentConfig` with one `claude-code` server for the tests below.
    fn config(default: &str, command: &str) -> AgentConfig {
        AgentConfig {
            default_server: default.to_string(),
            show_thinking: true,
            show_tools: true,
            servers: HashMap::from([(
                "claude-code".to_string(),
                AgentServerSetting {
                    command: command.to_string(),
                    arguments: vec!["-y".to_string()],
                    environment: HashMap::new(),
                },
            )]),
        }
    }

    #[test]
    fn resolves_the_default_server() {
        let resolved = config("claude-code", "npx").resolve().unwrap();
        assert_eq!(resolved.command, "npx");
        assert_eq!(resolved.args, vec!["-y".to_string()]);
    }

    #[test]
    fn unknown_default_server_lists_the_available_names() {
        let err = config("nope", "npx").resolve().unwrap_err();
        assert!(err.contains("nope"));
        assert!(err.contains("claude-code"));
    }

    #[test]
    fn empty_command_is_rejected() {
        assert!(config("claude-code", "  ").resolve().is_err());
    }

    #[test]
    fn thinking_and_tools_are_shown_unless_turned_off() {
        let missing: AgentConfig =
            toml::from_str("default-server = \"a\"\n[servers]").unwrap();
        assert!(missing.show_thinking && missing.show_tools);
        let off: AgentConfig = toml::from_str(
            "default-server = \"a\"\nshow-thinking = false\nshow-tools = false\n[servers]",
        )
        .unwrap();
        assert!(!off.show_thinking && !off.show_tools);
    }
}
