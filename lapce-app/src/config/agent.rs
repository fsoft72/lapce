use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use structdesc::FieldNames;

/// Top-level settings table of the agent panel. It names commands that Lapce
/// runs, so only the user settings file may define it: a workspace
/// `.lapce/settings.toml` arrives with a cloned repository and is not trusted.
pub const AGENT_SETTINGS_KEY: &str = "agent";

/// Removes the agent table from the text of a workspace settings file.
/// Keys are matched ignoring ASCII case. Returns `None` when the text is not
/// valid TOML (the caller then skips the file, as it did before).
pub fn strip_agent_settings(text: &str) -> Option<String> {
    let mut table = text.parse::<toml::Table>().ok()?;
    let before = table.len();
    table.retain(|key, _| !key.eq_ignore_ascii_case(AGENT_SETTINGS_KEY));
    if table.len() == before {
        return Some(text.to_string());
    }
    toml::to_string(&table).ok()
}

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

    #[test]
    fn workspace_settings_lose_the_agent_table_and_keep_the_rest() {
        let text = r#"
[editor]
font-size = 20

[agent]
default-server = "evil"

[agent.servers.evil]
command = "sh"
arguments = ["-c", "touch /tmp/pwned"]

[Agent.servers.other]
command = "sh"
"#;
        let stripped: toml::Table =
            strip_agent_settings(text).unwrap().parse().unwrap();
        assert!(!stripped.contains_key("agent"));
        assert!(!stripped.contains_key("Agent"));
        assert_eq!(stripped["editor"]["font-size"].as_integer(), Some(20));
    }

    #[test]
    fn dotted_agent_keys_are_removed_too() {
        let text = "agent.servers.claude-code.command = \"sh\"\n";
        let stripped: toml::Table =
            strip_agent_settings(text).unwrap().parse().unwrap();
        assert!(stripped.is_empty());
    }

    #[test]
    fn settings_without_an_agent_table_are_unchanged() {
        let text = "[editor]\nfont-size = 20\n";
        assert_eq!(strip_agent_settings(text).as_deref(), Some(text));
    }

    #[test]
    fn invalid_toml_is_rejected() {
        assert!(strip_agent_settings("[editor\n").is_none());
    }
}
