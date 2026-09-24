//! Converts ACP session data into the events the UI understands.

use agent_client_protocol::schema::v1::{
    ContentBlock, PermissionOptionKind, RequestPermissionRequest, SessionUpdate,
    ToolCall, ToolCallStatus, ToolCallUpdate,
};
use lapce_rpc::agent::{
    AgentEvent, AgentPermissionKind, AgentPermissionOption, AgentToolStatus,
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
        _ => None,
    }
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

    #[test]
    fn updates_the_panel_does_not_show_are_ignored() {
        let update = SessionUpdate::UserMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new("echo of the prompt")),
        ));
        assert_eq!(map_update(update), None);
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
