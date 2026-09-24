use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use structdesc::FieldNames;

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
    #[field_names(skip)]
    pub servers: HashMap<String, AgentServerSetting>,
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
}
