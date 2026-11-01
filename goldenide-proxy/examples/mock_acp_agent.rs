//! Mock ACP agent used by the integration tests. On every prompt it streams a
//! message, asks for permission, writes `/mock/out.txt` if allowed, then ends the turn.
//! If the permission request is cancelled it streams `permission cancelled` and
//! ends the turn with the `Cancelled` stop reason.

use agent_client_protocol::{
    Agent, Result, Stdio,
    schema::v1::{
        AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest,
        InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
        PermissionOptionKind, PromptRequest, PromptResponse,
        RequestPermissionOutcome, RequestPermissionRequest, SessionConfigOption,
        SessionConfigOptionCategory, SessionConfigOptionValue,
        SessionConfigSelectOption, SessionNotification, SessionUpdate,
        SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, StopReason,
        TextContent, ToolCallUpdate, ToolCallUpdateFields, WriteTextFileRequest,
    },
};

/// The model setting the mock offers, with `current` selected.
fn model_option(current: &str) -> SessionConfigOption {
    let mut option = SessionConfigOption::select(
        "model",
        "Model",
        current.to_string(),
        vec![
            SessionConfigSelectOption::new("fast", "Fast"),
            SessionConfigSelectOption::new("smart", "Smart"),
        ],
    );
    option.category = Some(SessionConfigOptionCategory::Model);
    option
}

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
                    responder.respond(
                        NewSessionResponse::new("mock-session")
                            .config_options(vec![model_option("fast")]),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |req: SetSessionConfigOptionRequest, responder, _cx| {
                    let SessionConfigOptionValue::ValueId { value } = req.value
                    else {
                        return responder.respond_with_error(
                            agent_client_protocol::Error::invalid_params(),
                        );
                    };
                    responder.respond(SetSessionConfigOptionResponse::new(vec![
                        model_option(&value.to_string()),
                    ]))
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
                        if matches!(
                            permission.outcome,
                            RequestPermissionOutcome::Cancelled
                        ) {
                            connection.send_notification(
                                SessionNotification::new(
                                    session_id.clone(),
                                    SessionUpdate::AgentMessageChunk(
                                        ContentChunk::new(ContentBlock::Text(
                                            TextContent::new("permission cancelled"),
                                        )),
                                    ),
                                ),
                            )?;
                            return responder.respond(PromptResponse::new(
                                StopReason::Cancelled,
                            ));
                        }
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
