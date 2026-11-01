//! Converts ACP session data into the events the UI understands.

use agent_client_protocol::schema::v1::{
    ContentBlock, PermissionOptionKind, RequestPermissionRequest, SessionConfigKind,
    SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    SessionConfigSelectOptions, SessionUpdate, ToolCall, ToolCallContent,
    ToolCallStatus, ToolCallUpdate,
};
use lapce_rpc::agent::{
    AgentConfigChoice, AgentConfigKind, AgentConfigOption, AgentEvent,
    AgentPermissionKind, AgentPermissionOption, AgentToolStatus,
};

/// Title shown when the agent does not name the action it asks permission for.
const DEFAULT_PERMISSION_TITLE: &str = "Agent action";

/// Maps an ACP tool status to the UI status. Unknown states count as failed.
fn map_status(status: ToolCallStatus) -> AgentToolStatus {
    match status {
        ToolCallStatus::Pending => AgentToolStatus::Pending,
        ToolCallStatus::InProgress => AgentToolStatus::InProgress,
        ToolCallStatus::Completed => AgentToolStatus::Completed,
        _ => AgentToolStatus::Failed,
    }
}

/// Returns the text of a content block, or `None` for non-text content.
fn block_text(block: ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(text) => Some(text.text),
        _ => None,
    }
}

/// Maps a session update to a UI event. Updates the panel does not show return `None`.
pub fn map_update(update: SessionUpdate) -> Option<AgentEvent> {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            block_text(chunk.content).map(|text| AgentEvent::MessageChunk { text })
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            block_text(chunk.content).map(|text| AgentEvent::ThoughtChunk { text })
        }
        SessionUpdate::ToolCall(ToolCall {
            tool_call_id,
            title,
            status,
            ..
        }) => Some(AgentEvent::ToolCall {
            id: tool_call_id.to_string(),
            title,
            status: map_status(status),
        }),
        SessionUpdate::ToolCallUpdate(ToolCallUpdate {
            tool_call_id,
            fields,
            ..
        }) => Some(AgentEvent::ToolCallUpdate {
            id: tool_call_id.to_string(),
            title: fields.title,
            status: fields.status.map(map_status),
        }),
        SessionUpdate::ConfigOptionUpdate(update) => {
            Some(AgentEvent::ConfigOptions {
                options: map_config_options(&update.config_options),
            })
        }
        _ => None,
    }
}

/// Collects the file changes of the diff blocks in `content`.
fn diff_events(content: &[ToolCallContent]) -> Vec<AgentEvent> {
    content
        .iter()
        .filter_map(|block| match block {
            ToolCallContent::Diff(diff) => Some(AgentEvent::FileChange {
                path: diff.path.clone(),
                old_text: diff.old_text.clone(),
                new_text: diff.new_text.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// The file changes reported by a session update. Only a completed tool call
/// counts: a pending one may still be rejected, a failed one changed nothing.
pub fn map_file_changes(update: &SessionUpdate) -> Vec<AgentEvent> {
    match update {
        SessionUpdate::ToolCall(call) if call.status == ToolCallStatus::Completed => {
            diff_events(&call.content)
        }
        SessionUpdate::ToolCallUpdate(update)
            if update.fields.status == Some(ToolCallStatus::Completed) =>
        {
            update
                .fields
                .content
                .as_deref()
                .map(diff_events)
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Maps a select value to a choice.
fn map_choice(option: &SessionConfigSelectOption) -> AgentConfigChoice {
    AgentConfigChoice {
        value: option.value.to_string(),
        name: option.name.clone(),
    }
}

/// Keeps the settings the chat box can show: the model and the effort
/// (thought level) selectors. Other settings and toggles are left out, and
/// grouped choices are flattened.
pub fn map_config_options(
    options: &[SessionConfigOption],
) -> Vec<AgentConfigOption> {
    options
        .iter()
        .filter_map(|option| {
            let kind = match option.category {
                Some(SessionConfigOptionCategory::Model) => AgentConfigKind::Model,
                Some(SessionConfigOptionCategory::ThoughtLevel) => {
                    AgentConfigKind::Effort
                }
                _ => return None,
            };
            let SessionConfigKind::Select(select) = &option.kind else {
                return None;
            };
            let choices = match &select.options {
                SessionConfigSelectOptions::Ungrouped(options) => {
                    options.iter().map(map_choice).collect()
                }
                SessionConfigSelectOptions::Grouped(groups) => groups
                    .iter()
                    .flat_map(|group| group.options.iter().map(map_choice))
                    .collect(),
                _ => return None,
            };
            Some(AgentConfigOption {
                id: option.id.to_string(),
                name: option.name.clone(),
                kind,
                current: select.current_value.to_string(),
                choices,
            })
        })
        .collect()
}

/// Maps an ACP permission option kind. Unknown kinds count as reject-always (the safe side).
fn map_kind(kind: PermissionOptionKind) -> AgentPermissionKind {
    match kind {
        PermissionOptionKind::AllowOnce => AgentPermissionKind::AllowOnce,
        PermissionOptionKind::AllowAlways => AgentPermissionKind::AllowAlways,
        PermissionOptionKind::RejectOnce => AgentPermissionKind::RejectOnce,
        _ => AgentPermissionKind::RejectAlways,
    }
}

/// Converts the options of a permission request for display.
pub fn map_permission_options(
    req: &RequestPermissionRequest,
) -> Vec<AgentPermissionOption> {
    req.options
        .iter()
        .map(|option| AgentPermissionOption {
            id: option.option_id.to_string(),
            name: option.name.clone(),
            kind: map_kind(option.kind),
        })
        .collect()
}

/// Returns the title of the action the agent asks permission for.
pub fn permission_title(req: &RequestPermissionRequest) -> String {
    req.tool_call
        .fields
        .title
        .clone()
        .unwrap_or_else(|| DEFAULT_PERMISSION_TITLE.to_string())
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::v1::{
        ContentChunk, PermissionOption, TextContent, ToolCallUpdateFields,
    };

    use super::*;

    #[test]
    fn maps_message_chunk() {
        let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("hi")),
        ));
        assert_eq!(
            map_update(update),
            Some(AgentEvent::MessageChunk {
                text: "hi".to_string()
            })
        );
    }

    #[test]
    fn maps_tool_call_and_update() {
        let call = SessionUpdate::ToolCall(ToolCall::new("t1", "Edit foo.rs"));
        assert_eq!(
            map_update(call),
            Some(AgentEvent::ToolCall {
                id: "t1".to_string(),
                title: "Edit foo.rs".to_string(),
                status: AgentToolStatus::Pending,
            })
        );
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t1",
            ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
        ));
        assert_eq!(
            map_update(update),
            Some(AgentEvent::ToolCallUpdate {
                id: "t1".to_string(),
                title: None,
                status: Some(AgentToolStatus::Completed),
            })
        );
    }

    /// A tool call update with one diff block and the given status.
    fn diff_update(status: ToolCallStatus) -> SessionUpdate {
        let diff = agent_client_protocol::schema::v1::Diff::new("/w/a.rs", "new")
            .old_text("old");
        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t1",
            ToolCallUpdateFields::new()
                .status(status)
                .content(vec![ToolCallContent::Diff(diff)]),
        ))
    }

    #[test]
    fn a_completed_diff_is_a_file_change() {
        assert_eq!(
            map_file_changes(&diff_update(ToolCallStatus::Completed)),
            vec![AgentEvent::FileChange {
                path: "/w/a.rs".into(),
                old_text: Some("old".into()),
                new_text: "new".into(),
            }]
        );
    }

    #[test]
    fn a_diff_that_is_not_completed_is_not_a_file_change() {
        for status in [
            ToolCallStatus::Pending,
            ToolCallStatus::InProgress,
            ToolCallStatus::Failed,
        ] {
            assert!(map_file_changes(&diff_update(status)).is_empty());
        }
    }

    #[test]
    fn updates_the_panel_does_not_show_are_ignored() {
        let update = SessionUpdate::UserMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("echo of the prompt")),
        ));
        assert_eq!(map_update(update), None);
    }

    #[test]
    fn keeps_only_the_model_and_effort_selectors() {
        let choices = vec![
            SessionConfigSelectOption::new("a", "Alpha"),
            SessionConfigSelectOption::new("b", "Beta"),
        ];
        let mut model =
            SessionConfigOption::select("model", "Model", "a", choices.clone());
        model.category = Some(SessionConfigOptionCategory::Model);
        let mut effort =
            SessionConfigOption::select("effort", "Effort", "b", choices.clone());
        effort.category = Some(SessionConfigOptionCategory::ThoughtLevel);
        let mut mode = SessionConfigOption::select("mode", "Mode", "a", choices);
        mode.category = Some(SessionConfigOptionCategory::Mode);
        let mut toggle = SessionConfigOption::boolean("flag", "Flag", true);
        toggle.category = Some(SessionConfigOptionCategory::Model);

        let mapped = map_config_options(&[model, effort, mode, toggle]);
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0].id, "model");
        assert_eq!(mapped[0].kind, AgentConfigKind::Model);
        assert_eq!(mapped[0].current_name(), "Alpha");
        assert_eq!(mapped[1].kind, AgentConfigKind::Effort);
        assert_eq!(mapped[1].current_name(), "Beta");
        assert_eq!(mapped[1].choices.len(), 2);
    }

    #[test]
    fn maps_permission_options_and_title() {
        let req = RequestPermissionRequest::new(
            "s1",
            ToolCallUpdate::new(
                "t1",
                ToolCallUpdateFields::new().title("Run tests"),
            ),
            vec![
                PermissionOption::new(
                    "allow",
                    "Allow",
                    PermissionOptionKind::AllowOnce,
                ),
                PermissionOption::new(
                    "no",
                    "Reject",
                    PermissionOptionKind::RejectOnce,
                ),
            ],
        );
        assert_eq!(permission_title(&req), "Run tests");
        let options = map_permission_options(&req);
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].id, "allow");
        assert_eq!(options[1].kind, AgentPermissionKind::RejectOnce);
    }

    #[test]
    fn missing_title_gets_a_default() {
        let req = RequestPermissionRequest::new(
            "s1",
            ToolCallUpdate::new("t1", ToolCallUpdateFields::new()),
            vec![],
        );
        assert_eq!(permission_title(&req), "Agent action");
    }
}
