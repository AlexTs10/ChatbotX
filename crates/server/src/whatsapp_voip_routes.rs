//! The browser call routes of the builder: `POST /api/whatsapp-voip-call-hangup`
//! (the `sendBeacon` hangup on tab close) and `POST /api/whatsapp-call-recording`
//! (the browser-recorded audio, then the `whatsappCallRecordingReady` job).
//! Same-site + session first, then the workspace membership.
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use chatbotx_api::handlers::ApiState;
use chatbotx_api::session::AuthState;
use chatbotx_business::whatsapp_voip_actions::{self as calling, Calling};
use chatbotx_business::{BusinessError, tx};
use chatbotx_db::repo::whatsapp_call as calls;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobOptions, QueueName};
use serde_json::{Value, json};

use crate::mini_apps::is_cross_site;

/// `MAX_RECORDING_UPLOAD_BYTES`.
const MAX_RECORDING_UPLOAD_BYTES: usize = 100 * 1024 * 1024;
/// Room for the multipart envelope around the largest recording.
const MAX_RECORDING_BODY_BYTES: usize = MAX_RECORDING_UPLOAD_BYTES + 64 * 1024;

#[derive(Clone)]
pub struct VoipRouteState {
    pub auth: Arc<AuthState>,
    pub api: Arc<ApiState>,
}

pub fn router(state: VoipRouteState) -> Router {
    Router::new()
        .route("/api/whatsapp-voip-call-hangup", post(hangup))
        .route("/api/whatsapp-call-recording", post(recording))
        .with_state(Arc::new(state))
}

fn reply(status: StatusCode, body: Value) -> Response {
    chatbotx_api::handlers::json(status.as_u16(), body)
}

fn refuse(status: StatusCode, error: &str) -> Response {
    reply(status, json!({ "error": error }))
}

/// `serverErrorHandler`: a `ChatbotXException` keeps its status, anything else is 500.
fn server_error(error: BusinessError) -> Response {
    match error {
        BusinessError::Exception {
            message, status, ..
        } => reply(
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            json!({ "message": message, "errors": [] }),
        ),
        other => {
            tracing::error!(err = %other.message(), "WhatsApp VoIP call route failed");
            reply(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "message": "Unknown error", "errors": [] }),
            )
        }
    }
}

/// `authorizeWorkspaceBeaconSession`: 403 cross-site, 401 signed out.
async fn session_user(state: &VoipRouteState, headers: &HeaderMap) -> Result<i64, Box<Response>> {
    if is_cross_site(headers) {
        return Err(Box::new(refuse(StatusCode::FORBIDDEN, "Forbidden")));
    }
    match chatbotx_api::rpc::find_rpc_session(&state.auth, headers).await {
        Ok(Some(session)) => Ok(session.user_id),
        Ok(None) => Err(Box::new(refuse(StatusCode::UNAUTHORIZED, "Unauthorized"))),
        Err(error) => Err(Box::new(server_error(BusinessError::Repo(
            chatbotx_db::repo::Error::Db(error),
        )))),
    }
}

/// `assertCurrentUserCanAccessChatbot` + the `scheduledDeletionAt` gate:
/// `Some(true)` = scheduled for deletion.
async fn membership(
    api: &ApiState,
    workspace_id: i64,
    user_id: i64,
) -> Result<bool, BusinessError> {
    let found = tx::read(&api.db, move |c| {
        Ok(c.query_row(
            r#"SELECT w."scheduledDeletionAt" IS NOT NULL FROM "WorkspaceMember" m
               JOIN "Workspace" w ON w."id" = m."workspaceId"
               WHERE m."workspaceId" = ?1 AND m."userId" = ?2"#,
            params![workspace_id, user_id],
            |r| r.get::<_, bool>(0),
        )
        .optional()?)
    })
    .await?;
    found.ok_or_else(|| calling::system_error("User is not associated with this workspace"))
}

fn digits(value: &Value) -> Option<i64> {
    let text = value.as_str()?;
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().unwrap_or(-1))
}

async fn hangup(
    State(state): State<Arc<VoipRouteState>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let user_id = match session_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let body = axum::body::to_bytes(body, 64 * 1024)
        .await
        .unwrap_or_default();
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (Some(workspace_id), Some(call_id)) = (
        digits(&body["workspaceId"]),
        digits(&body["whatsappCallId"]),
    ) else {
        return refuse(StatusCode::BAD_REQUEST, "Invalid request");
    };
    let api = &state.api;
    let run = async {
        membership(api, workspace_id, user_id).await?;
        let services = Calling {
            db: &api.db,
            graph_url: &api.graph_url,
            jobs: api.jobs.as_ref(),
            realtime: api.realtime.as_ref(),
        };
        calling::end_call_as_agent(
            services,
            workspace_id,
            user_id,
            call_id,
            "WhatsApp VoIP call hangup beacon: Graph action failed (call still finalized locally)",
        )
        .await
    };
    match run.await {
        Ok(_) => reply(StatusCode::OK, json!({ "ok": true })),
        Err(error) => server_error(error),
    }
}

/// `ALLOWED_RECORDING_CONTENT_TYPES`: content type → extension.
fn recording_extension(content_type: &str) -> Option<&'static str> {
    Some(match content_type {
        "audio/ogg" => "ogg",
        "audio/webm" => "webm",
        "audio/mp4" => "m4a",
        "audio/mpeg" => "mp3",
        _ => return None,
    })
}

async fn recording(
    State(state): State<Arc<VoipRouteState>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let user_id = match session_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let Ok(body) = axum::body::to_bytes(body, MAX_RECORDING_BODY_BYTES).await else {
        return refuse(StatusCode::BAD_REQUEST, "Recording too large");
    };
    let parts = chatbotx_api::handlers::ai_files::parse_multipart(&body).unwrap_or_default();
    let text = |name: &str| {
        parts
            .iter()
            .find(|p| p.name == name && p.filename.is_none())
            .and_then(|p| String::from_utf8(p.data.clone()).ok())
    };
    let audio = parts
        .iter()
        .find(|p| p.name == "audio" && p.filename.is_some());
    let (Some(call_id), Some(content_type), Some(audio)) = (
        text("whatsappCallId").filter(|t| !t.is_empty()),
        text("contentType"),
        audio,
    ) else {
        return refuse(StatusCode::BAD_REQUEST, "Invalid request");
    };
    let Some(extension) = recording_extension(&content_type) else {
        return refuse(StatusCode::BAD_REQUEST, "Unsupported audio format");
    };
    if audio.data.is_empty() || audio.data.len() > MAX_RECORDING_UPLOAD_BYTES {
        return refuse(StatusCode::BAD_REQUEST, "Recording too large");
    }
    let call_id = digits(&Value::String(call_id)).unwrap_or(-1);
    let api = &state.api;
    let call = match tx::read(&api.db, move |c| Ok(calls::find_by_id(c, call_id)?)).await {
        Ok(Some(call)) => call,
        Ok(None) => return refuse(StatusCode::NOT_FOUND, "Call not found"),
        Err(error) => return server_error(error),
    };
    match membership(api, call.workspace_id, user_id).await {
        Ok(false) => {}
        Ok(true) => {
            return server_error(BusinessError::exception(
                "Workspace deletion scheduled",
                "workspaceScheduledDeletion",
                403,
            ));
        }
        Err(error) => return server_error(error),
    }
    // ponytail: `checkWorkspaceOwnerAccess` is a no-op here (no owner quota
    // outside cloud, `LiveAuthDeps`); wire it when the quota store lands.
    let (ws, inbox) = (call.workspace_id, call.inbox_id);
    let integration = match tx::read(&api.db, move |c| {
        Ok(chatbotx_db::repo::integration_whatsapp::find_by_inbox_id_for_workspace(c, ws, inbox)?)
    })
    .await
    {
        Ok(Some(integration)) => integration,
        Ok(None) => return refuse(StatusCode::NOT_FOUND, "Integration not found"),
        Err(error) => return server_error(error),
    };
    if call.answered_by_user_id != Some(user_id) {
        return refuse(StatusCode::FORBIDDEN, "Forbidden");
    }
    if !integration.call_recording_enabled {
        return refuse(StatusCode::FORBIDDEN, "Call recording is disabled");
    }
    if integration.call_recording_mode != "browserWhisper" {
        return refuse(
            StatusCode::FORBIDDEN,
            "Call recording is not in browser-capture mode",
        );
    }
    let Some(files) = api.files.as_ref() else {
        return server_error(calling::system_error("File storage is not configured"));
    };
    let recording_path = format!("space/{ws}/calls/{}.{extension}", call.id);
    let size = audio.data.len();
    if let Err(error) = files
        .put_object(
            &recording_path,
            audio.data.clone(),
            chatbotx_files::PutOptions {
                content_type: Some(content_type.clone()),
                content_length: Some(size as u64),
                ..chatbotx_files::PutOptions::default()
            },
            None,
        )
        .await
    {
        tracing::error!(%error, whatsapp_call_id = call.id, "WhatsApp VoIP call recording upload failed");
        return server_error(calling::system_error("Unknown error"));
    }
    let Some(jobs) = api.jobs.as_ref() else {
        return server_error(calling::system_error("The job queue is off"));
    };
    let data = json!({
        "callId": call.id.to_string(),
        "workspaceId": ws.to_string(),
        "recordingPath": recording_path,
        "mimeType": content_type,
        "sizeBytes": size,
        "correlationId": call.wacid.clone().or_else(|| call.attempt_id.clone()),
    });
    let added = jobs
        .add(
            QueueName::Integration,
            "whatsappCallRecordingReady",
            json!({ "type": "whatsappCallRecordingReady", "data": data }),
            JobOptions {
                job_id: Some(
                    chatbotx_jobs::queues::whatsapp_call::whatsapp_call_recording_ready_job_id(
                        &call.id.to_string(),
                    ),
                ),
                ..JobOptions::default()
            },
        )
        .await;
    if let Err(error) = added {
        tracing::error!(%error, whatsapp_call_id = call.id, "WhatsApp VoIP call recording job enqueue failed");
        return server_error(calling::system_error("Unknown error"));
    }
    reply(StatusCode::OK, json!({ "ok": true }))
}
