//! The public Mini App runtime (Q.D1m): `app/mini-apps/page.tsx` (the page
//! data as `POST /rpc/pages/miniApp`), `submitMiniAppAction`,
//! `saveMiniAppScreenAction` (`POST /rpc/actions/<name>`, no session) and
//! `app/api/mini-apps/[id]/uploads/route.ts`, with `lib/public-mini-app.ts`,
//! `resolve-visitor-contact.ts`, `resolve-contact-variables.ts` and
//! `resume-flow.ts`. Lives here (not in `chatbotx-api`) because the
//! `{{variable}}` sources are the worker's.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use chatbotx_api::handlers::ApiState;
use chatbotx_api::handlers::input::{Input, Str};
use chatbotx_api::orpc::OrpcError;
use chatbotx_api::reflink_widget::{
    GuestRateLimit, UNKNOWN_CLIENT_IP, check_guest_ip_limit, guest_client_ip,
};
use chatbotx_business::errors::BusinessError;
use chatbotx_business::mini_app as service;
use chatbotx_core::mini_app::{
    MAX_UPLOAD_BYTES, escape_contact_value, has_contact_variable, map_display_text,
    parse_definition, strip_contact_variables,
};
use chatbotx_db::repo::models::MiniApp;
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::options::JobOptions;
use chatbotx_jobs::queue::QueueName;
use serde_json::{Value, json};

const SUBMITS_PER_WINDOW: i64 = 10;
const SCREEN_SAVES_PER_WINDOW: i64 = 60;
const UPLOADS_PER_WINDOW: i64 = 20;
/// Room for the multipart envelope around the largest allowed file.
const MAX_BODY_BYTES: usize = MAX_UPLOAD_BYTES + 64 * 1024;
/// Outlives the 7-day resume token, so a token never continues a flow twice.
const RESUME_ONCE_TTL_SECONDS: u64 = 8 * 24 * 60 * 60;
const NOT_FOUND: &str = "This link is invalid or the Mini App has been turned off.";
const RATE_LIMITED: &str = "Too many submissions. Please wait a moment and try again.";

#[derive(Clone)]
struct MiniAppState {
    api: Arc<ApiState>,
    deps: chatbotx_worker::Deps,
}

pub fn router(api: Arc<ApiState>, deps: chatbotx_worker::Deps) -> Router {
    Router::new()
        .route("/rpc/pages/miniApp", post(page))
        .route("/rpc/actions/submitMiniAppAction", post(submit))
        .route("/rpc/actions/saveMiniAppScreenAction", post(save_screen))
        .route(
            "/api/mini-apps/{id}/uploads",
            post(upload).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .with_state(MiniAppState { api, deps })
}

// ---- loading -----------------------------------------------------------------

/// `ID_PATTERN` (`/^\d{1,20}$/`): only bigint ids reach the database.
fn is_id(text: &str) -> bool {
    (1..=20).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

/// `loadServableWorkspace(..).servable`: the workspace exists and is not
/// scheduled for deletion (the owner entitlement is not consulted).
async fn servable_workspace(state: &ApiState, workspace_id: i64) -> bool {
    let row = state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "scheduledDeletionAt" FROM "Workspace" WHERE "id" = ?1"#,
                    params![workspace_id],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .optional()?)
        })
        .await;
    let workspace = row.unwrap_or_else(|err| {
        tracing::error!(err = %err, "Unable to load the Mini App workspace");
        None
    });
    chatbotx_business::workspace::lifecycle::resolve_freeze_reason(workspace, false).is_none()
}

/// `loadServableMiniApp`: `None` when it does not exist, is disabled, or its
/// workspace may not serve public pages.
async fn load_servable(state: &ApiState, id: i64) -> Option<MiniApp> {
    let mini_app = service::find_unscoped(&state.db, id).await.ok().flatten()?;
    (mini_app.enabled && servable_workspace(state, mini_app.workspace_id).await).then_some(mini_app)
}

fn exception(message: &str, code: &str, status: u16) -> BusinessError {
    BusinessError::Exception {
        message: message.into(),
        code: code.into(),
        status,
        field: None,
        data: None,
    }
}

/// `requireServableMiniApp`: `notFound` (404) instead of `None`.
async fn require_servable(state: &ApiState, id: i64) -> Result<MiniApp, OrpcError> {
    load_servable(state, id).await.ok_or_else(|| {
        chatbotx_api::rpc_procedures::action_fail(exception(NOT_FOUND, "notFound", 404))
    })
}

/// `resolveVisitorContactId`: the contact of a valid `{{mini_app_token}}` of
/// this workspace, or of a resume token of this Mini App; else anonymous.
fn visitor_contact_id(state: &ApiState, token: Option<&str>, mini_app: &MiniApp) -> Option<i64> {
    let token = token.filter(|t| !t.is_empty())?;
    let encryption = state.encryption.as_ref()?;
    let workspace_id = mini_app.workspace_id.to_string();
    if let Ok(payload) = encryption.verify_mini_app_token(token) {
        return (payload.claims.workspace_id == workspace_id)
            .then(|| payload.claims.contact_id.parse().ok())
            .flatten();
    }
    let payload = encryption.verify_mini_app_resume_token(token).ok()?;
    let claims = payload.claims;
    (claims.workspace_id == workspace_id && claims.mini_app_id == mini_app.id.to_string())
        .then(|| claims.contact_id.parse().ok())
        .flatten()
}

/// `resolveGuestRateLimitKey`: the client IP, else `fallback`.
fn guest_rate_limit_key(headers: &HeaderMap, fallback: &str) -> String {
    let ip = guest_client_ip(headers);
    if ip == UNKNOWN_CLIENT_IP {
        fallback.to_owned()
    } else {
        ip
    }
}

/// `assertVisitorNotRateLimited`: `rateLimited` (429) past `limit` requests
/// of one kind per visitor (contact, else IP) in the window.
fn assert_not_rate_limited(
    state: &ApiState,
    headers: &HeaderMap,
    bucket: &str,
    mini_app: &MiniApp,
    contact_id: Option<i64>,
    limit: i64,
) -> Result<(), OrpcError> {
    let fallback = contact_id.map_or_else(|| "anonymous".to_owned(), |id| format!("contact:{id}"));
    let key = guest_rate_limit_key(headers, &fallback);
    let GuestRateLimit { limited, .. } = check_guest_ip_limit(
        state,
        &format!("{bucket}:{}", mini_app.id),
        &key,
        limit,
        now_ms(),
    );
    if limited {
        return Err(chatbotx_api::rpc_procedures::action_fail(exception(
            RATE_LIMITED,
            "rateLimited",
            429,
        )));
    }
    Ok(())
}

// ---- contact variables ---------------------------------------------------------

/// `resolveContactVariables`: every `{{variable}}` of the visitor-facing
/// texts becomes the contact's value (escaped for where it lands); anonymous
/// visitors, a failed lookup or an unknown variable leave it out.
async fn resolve_contact_variables(
    deps: &chatbotx_worker::Deps,
    mini_app: &MiniApp,
    contact_id: Option<i64>,
) -> Value {
    let Some(mut definition) = parse_definition(&mini_app.definition.0) else {
        return mini_app.definition.0.clone();
    };
    let mut texts = BTreeMap::new();
    map_display_text(&mut definition, &mut |text, markdown| {
        if has_contact_variable(text) {
            texts.insert((markdown, text.clone()), String::new());
        }
    });
    if texts.is_empty() {
        return mini_app.definition.0.clone();
    }
    let resolved = match (contact_id, &deps.variables) {
        (Some(contact_id), Some(variables)) => resolve_texts(
            variables,
            mini_app.workspace_id,
            contact_id,
            texts,
        )
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(err = %err, contact_id, "Failed to resolve Mini App contact variables");
            BTreeMap::new()
        }),
        _ => BTreeMap::new(),
    };
    map_display_text(&mut definition, &mut |text, markdown| {
        if has_contact_variable(text) {
            let next = strip_contact_variables(
                resolved
                    .get(&(markdown, text.clone()))
                    .map_or(text.as_str(), String::as_str),
            );
            *text = next;
        }
    });
    definition.to_value()
}

async fn resolve_texts(
    variables: &chatbotx_worker::variables::VariableDeps,
    workspace_id: i64,
    contact_id: i64,
    texts: BTreeMap<(bool, String), String>,
) -> Result<BTreeMap<(bool, String), String>, String> {
    use chatbotx_core::variables::contact_variable::{ContactInboxArg, GetAllInput};
    use chatbotx_core::variables::{VariableSources as _, Variables};
    // Custom fields belong to the contact, so a contact without an inbox
    // still resolves them; only inbox-based variables come out empty.
    let inbox = variables
        .sources
        .find_recent_contact_inbox(&workspace_id.to_string(), &contact_id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    let clock = chatbotx_kv::SystemClock;
    let v = Variables::new(
        &variables.sources,
        &variables.cache,
        &variables.encryption,
        &clock,
    );
    let context = v
        .get_all(GetAllInput {
            contact_id: contact_id.to_string(),
            contact_inbox: inbox.map_or(ContactInboxArg::None, ContactInboxArg::Model),
            ..GetAllInput::default()
        })
        .await
        .map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    for (markdown, text) in texts.into_keys() {
        let escape = |value: &str| escape_contact_value(value, &text, markdown);
        let value = v
            .replace_all(&text, &context, Some(&escape))
            .await
            .map_err(|e| e.to_string())?;
        out.insert((markdown, text), value);
    }
    Ok(out)
}

// ---- page ----------------------------------------------------------------------

fn rpc_json(value: Value) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "json": value }).to_string(),
    )
        .into_response()
}

fn rpc_body(body: &Bytes) -> Value {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("json").cloned())
        .unwrap_or(Value::Null)
}

/// `MiniAppPublicPage` data: `{ miniApp: { id, name, definition, token } |
/// null }`. An unresolved `{{mini_app_token}}` is dropped.
async fn page(State(s): State<MiniAppState>, body: Bytes) -> Response {
    let input = rpc_body(&body);
    let text = |key: &str| input.get(key).and_then(Value::as_str);
    let mini_app = match text("id")
        .filter(|id| is_id(id))
        .and_then(|id| id.parse().ok())
    {
        Some(id) => load_servable(&s.api, id).await,
        None => None,
    };
    let Some(mini_app) = mini_app else {
        return rpc_json(json!({ "miniApp": null }));
    };
    let token = text("token").filter(|t| !t.starts_with("{{"));
    let contact_id = visitor_contact_id(&s.api, token, &mini_app);
    let definition = resolve_contact_variables(&s.deps, &mini_app, contact_id).await;
    rpc_json(json!({ "miniApp": {
        "id": mini_app.id.to_string(),
        "name": mini_app.name,
        "definition": definition,
        "token": token,
    } }))
}

// ---- actions -------------------------------------------------------------------

struct VisitorRequest {
    mini_app_id: i64,
    token: Option<String>,
    answers: Value,
    timezone: Option<String>,
}

/// `miniAppVisitorRequest` (+ `extra` for the action's own fields).
fn visitor_request(input: &mut Input) -> Option<VisitorRequest> {
    let mini_app_id = input.id("miniAppId");
    let token = input.opt_string("token", Str::max(4096));
    let answers = match input.get("answers") {
        Some(Value::Object(map)) => Some(Value::Object(map.clone())),
        _ => {
            input.issue(
                &chatbotx_api::handlers::input::key("answers"),
                "invalid_type",
                "Invalid input: expected record, received undefined",
            );
            None
        }
    };
    let timezone = input.opt_string("timezone", Str::max(64));
    Some(VisitorRequest {
        mini_app_id: mini_app_id?,
        token,
        answers: answers?,
        timezone,
    })
}

fn action_args(body: &Bytes) -> Input {
    match rpc_body(body).get("args").and_then(|a| a.get(0)) {
        Some(Value::Object(map)) => Input::from_object(map.clone()),
        _ => Input::from_object(serde_json::Map::new()),
    }
}

fn reply(result: Result<Value, OrpcError>) -> Response {
    match result {
        Ok(value) => rpc_json(value),
        Err(error) => chatbotx_api::rpc::procedures::error_response(&error),
    }
}

async fn emit_changes(
    state: &ApiState,
    workspace_id: i64,
    changes: Vec<chatbotx_business::contact_custom_field::FieldChange>,
) {
    chatbotx_api::handlers::contact_custom_fields::emit_field_changes(state, workspace_id, changes)
        .await;
}

/// `submitMiniAppAction`: the contact comes only from the signed token; a
/// resume token continues the flow the Mini App was opened from.
async fn submit(State(s): State<MiniAppState>, headers: HeaderMap, body: Bytes) -> Response {
    reply(submit_inner(&s, &headers, &body).await)
}

async fn submit_inner(
    s: &MiniAppState,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Value, OrpcError> {
    let mut input = action_args(body);
    let request = visitor_request(&mut input);
    let resume = input.opt_string("resume", Str::max(4096));
    chatbotx_api::rpc_ai::finish_action(input)?;
    let Some(request) = request else {
        return Err(chatbotx_api::rpc_ai::validation_failed());
    };
    let mini_app = require_servable(&s.api, request.mini_app_id).await?;
    let contact_id = visitor_contact_id(&s.api, request.token.as_deref(), &mini_app);
    assert_not_rate_limited(
        &s.api,
        headers,
        "mini-app",
        &mini_app,
        contact_id,
        SUBMITS_PER_WINDOW,
    )?;
    let storage_url = chatbotx_api::handlers::contacts::tenant_urls(&s.api).storage_url;
    let submitted = service::create_submission(
        &s.api.db,
        &storage_url,
        &mini_app,
        contact_id,
        request.answers,
        request.timezone,
    )
    .await
    .map_err(chatbotx_api::rpc_procedures::action_fail)?;
    emit_changes(&s.api, mini_app.workspace_id, submitted.changes).await;
    // A Messenger template's Mini App link carries its resume token as
    // `token`; a contact token there is simply not a resume token.
    if let Some(resume_token) = resume.or(request.token) {
        resume_flow(&s.api, &resume_token, &mini_app, contact_id).await;
    }
    Ok(json!({ "success": true }))
}

/// `saveMiniAppScreenAction`: the screen's mapped answers go to the
/// contact's custom fields; the definition comes back with its variables
/// resolved again. Anonymous visitors get no definition.
async fn save_screen(State(s): State<MiniAppState>, headers: HeaderMap, body: Bytes) -> Response {
    reply(save_screen_inner(&s, &headers, &body).await)
}

async fn save_screen_inner(
    s: &MiniAppState,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Value, OrpcError> {
    let mut input = action_args(body);
    let request = visitor_request(&mut input);
    let screen_key = input.string(
        "screenKey",
        Str {
            trim: false,
            min: Some(1),
            max: Some(64),
            pattern: None,
        },
    );
    chatbotx_api::rpc_ai::finish_action(input)?;
    let (Some(request), Some(screen_key)) = (request, screen_key) else {
        return Err(chatbotx_api::rpc_ai::validation_failed());
    };
    let mini_app = require_servable(&s.api, request.mini_app_id).await?;
    let Some(contact_id) = visitor_contact_id(&s.api, request.token.as_deref(), &mini_app) else {
        return Ok(json!({ "definition": null }));
    };
    assert_not_rate_limited(
        &s.api,
        headers,
        "mini-app-screen",
        &mini_app,
        Some(contact_id),
        SCREEN_SAVES_PER_WINDOW,
    )?;
    let saved = service::save_screen_answers(
        &s.api.db,
        &mini_app,
        Some(contact_id),
        screen_key,
        request.answers,
        request.timezone,
    )
    .await
    .map_err(chatbotx_api::rpc_procedures::action_fail)?;
    let Some(changes) = saved else {
        return Ok(json!({ "definition": null }));
    };
    emit_changes(&s.api, mini_app.workspace_id, changes).await;
    // The new values are read back by the variable sources.
    Ok(
        json!({ "definition": resolve_contact_variables(&s.deps, &mini_app, Some(contact_id)).await }),
    )
}

/// `workspaceService.isActiveNow`.
async fn workspace_active_now(state: &ApiState, workspace_id: i64) -> bool {
    let row = state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "isActive", "startTime", "endTime", "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                    params![workspace_id],
                    |r| {
                        Ok((
                            r.get::<_, bool>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()?)
        })
        .await
        .ok()
        .flatten();
    row.is_some_and(|(active, start, end, tz)| {
        chatbotx_business::workspace::is_active_now(
            active,
            start.as_deref(),
            end.as_deref(),
            &tz,
            now_ms(),
        )
    })
}

/// `resumeFlowAfterMiniAppSubmit`: replays the "Open Mini App" button as a
/// tap, once per token, while the bot is on. The token must name this Mini
/// App, its workspace and the submitting contact. Never fails the submit.
async fn resume_flow(
    state: &ApiState,
    resume_token: &str,
    mini_app: &MiniApp,
    contact_id: Option<i64>,
) {
    let Some(encryption) = &state.encryption else {
        return;
    };
    let Ok(payload) = encryption.verify_mini_app_resume_token(resume_token) else {
        return;
    };
    let claims = payload.claims;
    if !(contact_id.is_some_and(|id| claims.contact_id == id.to_string())
        && claims.workspace_id == mini_app.workspace_id.to_string()
        && claims.mini_app_id == mini_app.id.to_string())
    {
        return;
    }
    if !workspace_active_now(state, mini_app.workspace_id).await {
        tracing::info!(
            mini_app_id = mini_app.id,
            "Bot is not active; the flow does not continue after the Mini App submit"
        );
        return;
    }
    let once_key = format!(
        "mini-app-resume:{}",
        chatbotx_core::utils::crypto::sha256_hex(resume_token)
    );
    // ponytail: without a kv store (tests) there is no once guard.
    if let Some(kv) = &state.kv {
        match kv
            .durable
            .set_number_if_not_exists(&once_key, 1.0, RESUME_ONCE_TTL_SECONDS)
            .await
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(err) => {
                tracing::error!(err = %err, mini_app_id = mini_app.id, "Unable to continue the flow after a Mini App submit");
                return;
            }
        }
    }
    let Some(jobs) = &state.jobs else { return };
    let data = json!({ "type": "runFlowPostback", "data": {
        "conversationId": claims.conversation_id,
        "contactInboxId": claims.contact_inbox_id,
        "action": claims.button_payload,
    }});
    if let Err(err) = jobs
        .add(
            QueueName::Integration,
            "runFlowPostback",
            data,
            JobOptions::default(),
        )
        .await
    {
        tracing::error!(err = %err, mini_app_id = mini_app.id, "Unable to continue the flow after a Mini App submit");
        // Hand the token back so the next submit can try again.
        if let Some(kv) = &state.kv {
            let _ = kv.durable.delete(&[once_key.as_str()]).await;
        }
    }
}

// ---- uploads -------------------------------------------------------------------

fn reject(code: &str, status: StatusCode) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": code }).to_string(),
    )
        .into_response()
}

fn url_host(text: &str) -> Option<String> {
    let url = url::Url::parse(text).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

/// `isCrossSiteRequest`: `Sec-Fetch-Site`, then `Origin` / `Referer` against
/// the public host; fails closed.
pub fn is_cross_site(headers: &HeaderMap) -> bool {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(site) = text("sec-fetch-site") {
        return site == "cross-site";
    }
    let parts = chatbotx_core::utils::request::RequestParts { url: "", headers };
    let Some(host) = chatbotx_core::utils::request::get_raw_public_host_from_request(&parts) else {
        return true;
    };
    match text("origin").or_else(|| text("referer")) {
        Some(url) => url_host(url).as_deref() != Some(host.as_str()),
        None => true,
    }
}

/// `POST /api/mini-apps/{id}/uploads`: a PhotoPicker / DocumentPicker file
/// checked against its input's rules; the contact only from the token.
async fn upload(
    State(s): State<MiniAppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if is_cross_site(&headers) {
        return reject("forbidden", StatusCode::FORBIDDEN);
    }
    if !is_id(&id) {
        return reject("not_found", StatusCode::NOT_FOUND);
    }
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if declared == 0 {
        return reject("invalid_request", StatusCode::LENGTH_REQUIRED);
    }
    if declared > MAX_BODY_BYTES as u64 {
        return reject("too_large", StatusCode::PAYLOAD_TOO_LARGE);
    }
    let bucket = format!("mini-app-upload:{id}");
    let key = guest_rate_limit_key(&headers, &bucket);
    if check_guest_ip_limit(&s.api, &bucket, &key, UPLOADS_PER_WINDOW, now_ms()).limited {
        return reject("rate_limited", StatusCode::TOO_MANY_REQUESTS);
    }
    let Some(mini_app) = (match id.parse() {
        Ok(id) => load_servable(&s.api, id).await,
        Err(_) => None,
    }) else {
        return reject("not_found", StatusCode::NOT_FOUND);
    };
    let Ok(body) = axum::body::to_bytes(body, MAX_BODY_BYTES).await else {
        return reject("too_large", StatusCode::PAYLOAD_TOO_LARGE);
    };
    let parts = chatbotx_api::handlers::ai_files::parse_multipart(&body).unwrap_or_default();
    let field = |name: &str| parts.iter().find(|p| p.name == name);
    let text = |name: &str| {
        field(name)
            .filter(|p| p.filename.is_none())
            .and_then(|p| String::from_utf8(p.data.clone()).ok())
    };
    let (Some(file), Some(input_name)) = (
        field("file").filter(|p| p.filename.is_some()),
        text("inputName"),
    ) else {
        return reject("invalid_request", StatusCode::BAD_REQUEST);
    };
    if file.data.len() > MAX_UPLOAD_BYTES {
        return reject("too_large", StatusCode::PAYLOAD_TOO_LARGE);
    }
    let Some(uploader) = &s.api.files else {
        tracing::error!("Mini App upload failed: no file store");
        return reject("upload_failed", StatusCode::INTERNAL_SERVER_ERROR);
    };
    let contact_id = visitor_contact_id(&s.api, text("token").as_deref(), &mini_app);
    let storage_url = chatbotx_api::handlers::contacts::tenant_urls(&s.api).storage_url;
    let file = service::UploadedFile {
        name: file.filename.clone().unwrap_or_default(),
        mime_type: file.content_type.clone().unwrap_or_default(),
        bytes: file.data.clone(),
    };
    match service::create_upload(
        &s.api.db,
        uploader,
        &storage_url,
        &mini_app,
        contact_id,
        input_name,
        file,
    )
    .await
    {
        Ok(result) => (
            StatusCode::CREATED,
            [(header::CONTENT_TYPE, "application/json")],
            result.to_string(),
        )
            .into_response(),
        Err(BusinessError::Exception {
            code, status, data, ..
        }) if code == "miniAppUploadRejected" => {
            let reason = data
                .as_ref()
                .and_then(|d| d.get("reason"))
                .and_then(Value::as_str)
                .unwrap_or("rejected");
            reject(
                reason,
                StatusCode::from_u16(status).unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
            )
        }
        Err(err) => {
            tracing::error!(err = %err, "Mini App upload failed");
            reject("upload_failed", StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn cross_site_checks_fetch_site_then_origin_then_referer() {
        assert!(is_cross_site(&headers(&[("sec-fetch-site", "cross-site")])));
        assert!(!is_cross_site(&headers(&[(
            "sec-fetch-site",
            "same-origin"
        )])));
        assert!(!is_cross_site(&headers(&[
            ("host", "app.test"),
            ("origin", "https://app.test")
        ])));
        assert!(is_cross_site(&headers(&[
            ("host", "app.test"),
            ("origin", "https://evil.test")
        ])));
        assert!(!is_cross_site(&headers(&[
            ("host", "app.test:8080"),
            ("referer", "http://app.test:8080/x")
        ])));
        assert!(is_cross_site(&headers(&[("host", "app.test")])));
        assert!(is_cross_site(&HeaderMap::new()));
    }
}
