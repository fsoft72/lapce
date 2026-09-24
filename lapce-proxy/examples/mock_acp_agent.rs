//! Mock ACP agent used by the integration tests. On every prompt it streams a
//! message, asks for permission, writes `/mock/out.txt` if allowed, then ends the turn.

use agent_client_protocol::{
    Agent, Result, Stdio,
    schema::v1::{
        AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest,
        InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
        PermissionOptionKind, PromptRequest, PromptResponse,
        RequestPermissionOutcome, RequestPermissionRequest, SessionNotification,
        SessionUpdate, StopReason, TextContent, ToolCallUpdate,
        ToolCallUpdateFields, WriteTextFileRequest,
    },
};

/// Serves the mock agent over stdio until the client disconnects.
fn main() -> Result<()> {
    futures::executor::block_on(
        Agent
            .builder()
            .name("mock-acp-agent")
            .on_receive_request(
                async move |init: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(init.protocol_version)
                            .agent_capabilities(AgentCapabilities::new()),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_req: NewSessionRequest, responder, _cx| {
                    responder.respond(NewSessionResponse::new("mock-session"))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |req: PromptRequest, responder, cx| {
                    let connection = cx.clone();
                    cx.spawn(async move {
                        let session_id = req.session_id.clone();
                        connection.send_notification(SessionNotification::new(
                            session_id.clone(),
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new("hello")),
                            )),
                        ))?;
                        let permission = connection
                            .send_request(RequestPermissionRequest::new(
                                session_id.clone(),
                                ToolCallUpdate::new(
                                    "tool-1",
                                    ToolCallUpdateFields::new(),
                                ),
                                vec![
                                    PermissionOption::new(
                                        "allow",
                                        "Allow",
                                        PermissionOptionKind::AllowOnce,
                                    ),
                                    PermissionOption::new(
                                        "reject",
                                        "Reject",
                                        PermissionOptionKind::RejectOnce,
                                    ),
                                ],
                            ))
                            .block_task()
                            .await?;
                        let allowed = matches!(
                            permission.outcome,
                            RequestPermissionOutcome::Selected(ref selected)
                                if selected.option_id.to_string() == "allow"
                        );
                        if allowed {
                            connection
                                .send_request(WriteTextFileRequest::new(
                                    session_id,
                                    "/mock/out.txt",
                                    "written by mock",
                                ))
                                .block_task()
                                .await?;
                        }
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    })
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(Stdio::new()),
    )
}
