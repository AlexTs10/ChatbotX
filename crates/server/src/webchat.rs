//! The webchat guest routes (P1.26): `app/api/guest/messages/route.ts`
//! (`OPTIONS` / `GET` / `POST`) and `app/api/guest/realtime-token/route.ts`.
//! The POST body goes to `chatbotx_worker::webchat_guest::create_message`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{options, post};
use chatbotx_api::handlers::ApiState;
use chatbotx_api::reflink_widget::{check_guest_rate_limit, guest_client_ip};
use chatbotx_business::integration_webchat as webchat;
use chatbotx_business::message::{ListForConversation, list_for_conversation};
use chatbotx_db::repo::now_ms;
use chatbotx_worker::webchat_guest::{
    self as guest, GuestError, GuestInput, GuestMessage, find_guest_contact_inbox,
    find_guest_conversation,
};
use serde_json::{Map, Value, json};

const UNAUTHORIZED_DOMAIN: &str = "This website is not authorized to load this chat widget.";
const RATE_LIMIT_EXCEEDED: &str = "Too many requests. Please try again in a moment.";
const WORKSPACE_UNAVAILABLE: &str = "This workspace is no longer available.";
/// `GUEST_REALTIME_MINT_WORKSPACE_LIMIT`.
const MINT_WORKSPACE_LIMIT: u64 = 100;

#[derive(Clone)]
struct GuestState {
    api: Arc<ApiState>,
    deps: chatbotx_worker::Deps,
    /// `BETTER_AUTH_SECRET` (the access-token key).
    auth_secret: String,
    /// `resolveBroadcastSecret()`; `None` = unset (503).
    broadcast_secret: Option<String>,
}

pub fn router(
    api: Arc<ApiState>,
    deps: chatbotx_worker::Deps,
    auth_secret: String,
    broadcast_secret: Option<String>,
) -> Router {
    Router::new()
        .route(
            "/api/guest/messages",
            options(preflight).get(list).post(create),
        )
        .route("/api/guest/realtime-token", post(realtime_token))
        .with_state(GuestState {
            api,
            deps,
            auth_secret,
            broadcast_secret,
        })
}

fn cors(origin: Option<&str>, authorized: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Authorization"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    if authorized && let Some(origin) = origin.and_then(|o| HeaderValue::from_str(o).ok()) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    headers
}

fn json_reply(status: StatusCode, headers: HeaderMap, body: Value) -> Response {
    let mut headers = headers;
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    (status, headers, body.to_string()).into_response()
}

fn error_reply(status: u16, message: &str, headers: HeaderMap) -> Response {
    json_reply(
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
        headers,
        json!({ "message": message, "errors": [] }),
    )
}

fn validation_error(field: &str, headers: HeaderMap) -> Response {
    json_reply(
        StatusCode::UNPROCESSABLE_ENTITY,
        headers,
        json!({ "message": "Validation error", "errors": [{ "field": field, "message": "Invalid input" }] }),
    )
}

fn origin_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// `getBearerToken`: `Bearer <token>` (case-insensitive scheme).
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let mut parts = value.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.to_owned())
}

/// `getDomainFromHeader()`: the request host (the middleware's `x-domain`),
/// without the port.
fn app_host(headers: &HeaderMap) -> String {
    let host = headers
        .get("x-domain")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    host.rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(host, |(h, _)| h)
        .to_lowercase()
}

/// `zodBigintAsString`: digits as a string or a safe integer.
fn bigint(object: &Map<String, Value>, key: &str) -> Option<i64> {
    match object.get(key)? {
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            text.parse().ok()
        }
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// An optional string field (`Err` = present but not a string).
fn opt_str(object: &Map<String, Value>, key: &str) -> Result<Option<String>, ()> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(()),
    }
}

/// The common fields of the guest requests.
struct GuestRequest {
    workspace_id: i64,
    webchat_id: i64,
    guest_conversation_id: String,
    access_token: Option<String>,
    parent_origin: Option<String>,
}

fn parse_common(object: &Map<String, Value>) -> Result<GuestRequest, &'static str> {
    let workspace_id = bigint(object, "workspaceId").ok_or("workspaceId")?;
    let webchat_id = bigint(object, "webchatId").ok_or("webchatId")?;
    let guest_conversation_id = opt_str(object, "guestConversationId")
        .ok()
        .flatten()
        .filter(|id| webchat::is_valid_guest_conversation_id(id))
        .ok_or("guestConversationId")?;
    Ok(GuestRequest {
        workspace_id,
        webchat_id,
        guest_conversation_id,
        access_token: opt_str(object, "accessToken").map_err(|()| "accessToken")?,
        parent_origin: opt_str(object, "parentOrigin").map_err(|()| "parentOrigin")?,
    })
}

/// `createWebchatMessageRequest`: the union branches in zod order.
fn parse_input(object: &Map<String, Value>) -> Result<GuestInput, &'static str> {
    if let Some(Value::String(text)) = object.get("text") {
        let text = text.trim();
        let postback = match object.get("postback") {
            None => Some(None),
            Some(Value::String(p)) => Some(Some(p.trim().to_owned())),
            Some(_) => None,
        };
        if !text.is_empty()
            && text.chars().count() <= 1000
            && let Some(postback) = postback
        {
            return Ok(GuestInput::Text {
                text: text.to_owned(),
                postback,
            });
        }
    }
    if let Some(flow_id) = bigint(object, "flowId") {
        return Ok(GuestInput::Flow(flow_id));
    }
    if let Some(Value::String(r#ref)) = object.get("initRef") {
        return Ok(GuestInput::InitRef(r#ref.clone()));
    }
    if object.get("init") == Some(&Value::Bool(true)) {
        return Ok(GuestInput::Init);
    }
    Err("text")
}

/// `clientId`, `parentUrl` and the two other optional strings.
type Extras = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The optional tail of `createWebchatMessageRequest`.
fn parse_extras(object: &Map<String, Value>) -> Result<Extras, &'static str> {
    let client_id = opt_str(object, "clientId").map_err(|()| "clientId")?;
    opt_str(object, "ref").map_err(|()| "ref")?;
    let parent_url = opt_str(object, "parentUrl").map_err(|()| "parentUrl")?;
    if parent_url
        .as_deref()
        .is_some_and(|u| u.chars().count() > 2048 || url::Url::parse(u).is_err())
    {
        return Err("parentUrl");
    }
    let locale = opt_str(object, "locale").map_err(|()| "locale")?;
    if locale.as_deref().is_some_and(|l| l.chars().count() > 35) {
        return Err("locale");
    }
    let timezone = opt_str(object, "timezone").map_err(|()| "timezone")?;
    if timezone.as_deref().is_some_and(|t| t.chars().count() > 64) {
        return Err("timezone");
    }
    Ok((client_id, parent_url, locale, timezone))
}

async fn preflight(headers: HeaderMap) -> Response {
    (
        StatusCode::NO_CONTENT,
        cors(origin_of(&headers).as_deref(), true),
    )
        .into_response()
}

fn now_secs() -> i64 {
    now_ms() / 1000
}

/// Token bound to the presented origin, then the optional allowlist.
fn is_authorized(
    state: &GuestState,
    request: &GuestRequest,
    token: Option<&str>,
    authorized_domains: &[String],
    headers: &HeaderMap,
) -> bool {
    webchat::verify_access_token(
        &state.auth_secret,
        token,
        &request.workspace_id.to_string(),
        &request.webchat_id.to_string(),
        request.parent_origin.as_deref(),
        now_secs(),
    ) && webchat::is_allowed_guest_origin(
        request.parent_origin.as_deref(),
        authorized_domains,
        &app_host(headers),
    )
}

async fn workspace(
    state: &GuestState,
    id: i64,
) -> Result<Option<chatbotx_db::repo::models::Workspace>, String> {
    state
        .api
        .db
        .read(move |conn| {
            chatbotx_business::workspace::find(conn, id)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .map_err(|e| e.to_string())
}

fn rate_limited(state: &GuestState, headers: &HeaderMap, request: &GuestRequest) -> Option<i64> {
    let limit = check_guest_rate_limit(
        &state.api,
        &request.webchat_id.to_string(),
        &guest_client_ip(headers),
        Some(&request.guest_conversation_id),
        now_ms(),
    );
    limit.limited.then_some(limit.retry_after)
}

fn too_many(mut out: HeaderMap, retry_after: i64) -> Response {
    if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
        out.insert(header::RETRY_AFTER, value);
    }
    error_reply(429, RATE_LIMIT_EXCEEDED, out)
}

/// `GET /api/guest/messages`.
async fn list(State(state): State<GuestState>, headers: HeaderMap, uri: Uri) -> Response {
    let query = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()).into_owned();
    let origin = origin_of(&headers);
    let origin = origin.as_deref();
    // `Object.fromEntries(searchParams)`: the last value of a key wins.
    let object: Map<String, Value> = query
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    let request = match parse_common(&object) {
        Ok(request) => request,
        Err(field) => return validation_error(field, cors(origin, false)),
    };
    let per_page = match object.get("perPage").and_then(Value::as_str) {
        None => 20,
        Some(text) => match text.trim().parse::<f64>() {
            Ok(n) if n.is_finite() => n as i64,
            _ => return validation_error("perPage", cors(origin, false)),
        },
    };
    let cursor = object
        .get("cursor")
        .and_then(Value::as_str)
        .map(str::to_owned);
    match workspace(&state, request.workspace_id).await {
        Ok(Some(ws)) if chatbotx_business::workspace::is_scheduled_for_deletion(&ws) => {
            return error_reply(403, UNAUTHORIZED_DOMAIN, cors(origin, false));
        }
        Ok(_) => {}
        Err(message) => return error_reply(400, &message, cors(origin, false)),
    }
    if let Some(retry_after) = rate_limited(&state, &headers, &request) {
        return too_many(cors(origin, false), retry_after);
    }
    let webchat_row = match webchat::find_by_id_for_workspace(
        &state.api.db,
        request.webchat_id,
        request.workspace_id,
    )
    .await
    {
        Ok(row) => row,
        Err(e) => return error_reply(e.status().unwrap_or(400), &e.message(), cors(origin, false)),
    };
    let token = request
        .access_token
        .clone()
        .or_else(|| bearer_token(&headers));
    let authorized = is_authorized(
        &state,
        &request,
        token.as_deref(),
        &webchat_row.authorized_domains,
        &headers,
    );
    let out = cors(origin, authorized);
    if !authorized {
        return error_reply(403, UNAUTHORIZED_DOMAIN, out);
    }
    let empty = || {
        json_reply(
            StatusCode::OK,
            out.clone(),
            json!({ "data": [], "nextCursor": null, "prevCursor": null }),
        )
    };
    let ws = request.workspace_id;
    let contact_inbox = match find_guest_contact_inbox(
        &state.deps,
        ws,
        webchat_row.inbox_id,
        &request.guest_conversation_id,
    )
    .await
    {
        Ok(Some(ci)) => ci,
        Ok(None) => return empty(),
        Err(e) => return error_reply(e.status, &e.message, cors(origin, false)),
    };
    let conversation =
        match find_guest_conversation(&state.deps, ws, contact_inbox.contact_id).await {
            Ok(Some(c)) => c,
            Ok(None) => return empty(),
            Err(e) => return error_reply(e.status, &e.message, cors(origin, false)),
        };
    let page = list_for_conversation(
        &state.api.db,
        ListForConversation {
            workspace_id: conversation.workspace_id,
            conversation_id: Some(conversation.id),
            contact_inbox_id: Some(contact_inbox.id),
            cursor: cursor
                .as_deref()
                .and_then(chatbotx_api::handlers::contact_messages::decode_cursor),
            limit: per_page,
        },
    )
    .await;
    let page = match page {
        Ok(page) => chatbotx_api::handlers::contact_messages::messages_json(
            &state.api,
            conversation.workspace_id,
            &page.data,
        )
        .await
        .map(|data| (page, data)),
        Err(e) => Err(e),
    };
    match page {
        Ok((page, data)) => json_reply(
            StatusCode::OK,
            out,
            json!({
                "data": data,
                "nextCursor": page.next_cursor.as_ref().map(chatbotx_api::handlers::contact_messages::encode_cursor),
                "prevCursor": null,
            }),
        ),
        Err(e) => error_reply(e.status().unwrap_or(400), &e.message(), cors(origin, false)),
    }
}

/// `POST /api/guest/messages` (+ `handleCreateWebchatMessage`'s checks).
async fn create(State(state): State<GuestState>, headers: HeaderMap, body: Bytes) -> Response {
    let origin = origin_of(&headers);
    let origin = origin.as_deref();
    let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(&body) else {
        return error_reply(400, "Invalid JSON body", cors(origin, false));
    };
    let parsed = parse_input(&object).and_then(|input| {
        let request = parse_common(&object)?;
        Ok((input, request, parse_extras(&object)?))
    });
    let (input, request, (client_id, parent_url, locale, timezone)) = match parsed {
        Ok(parsed) => parsed,
        Err(field) => return validation_error(field, cors(origin, false)),
    };
    let ws = request.workspace_id;
    let token = request
        .access_token
        .clone()
        .or_else(|| bearer_token(&headers));
    let webchat_row = match webchat::find_by_id_for_workspace(&state.api.db, request.webchat_id, ws)
        .await
    {
        Ok(row) => row,
        Err(e) => return error_reply(e.status().unwrap_or(400), &e.message(), cors(origin, false)),
    };
    let authorized = is_authorized(
        &state,
        &request,
        token.as_deref(),
        &webchat_row.authorized_domains,
        &headers,
    );
    let out = cors(origin, authorized);
    if !authorized {
        return error_reply(403, UNAUTHORIZED_DOMAIN, out);
    }
    // `handleCreateWebchatMessage`.
    let owner_id = match workspace(&state, ws).await {
        Ok(Some(w)) if chatbotx_business::workspace::is_scheduled_for_deletion(&w) => {
            return error_reply(403, WORKSPACE_UNAVAILABLE, cors(origin, false));
        }
        Ok(Some(w)) => w.owner_id,
        Ok(None) => return error_reply(400, "Workspace not found", cors(origin, false)),
        Err(message) => return error_reply(400, &message, cors(origin, false)),
    };
    if let Some(retry_after) = rate_limited(&state, &headers, &request) {
        return too_many(cors(origin, false), retry_after);
    }
    let message = GuestMessage {
        workspace_id: ws,
        guest_conversation_id: request.guest_conversation_id,
        input,
        client_id,
        parent_url,
        locale,
        timezone,
    };
    match guest::create_message(&state.deps, &webchat_row, owner_id, message).await {
        Ok(message) => json_reply(StatusCode::OK, out, json!({ "data": message })),
        Err(GuestError { message, status }) => error_reply(status, &message, cors(origin, false)),
    }
}

/// `POST /api/guest/realtime-token`.
async fn realtime_token(
    State(state): State<GuestState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let status = |code: StatusCode| code.into_response();
    let Some(object) = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v.as_object().cloned())
    else {
        return status(StatusCode::BAD_REQUEST);
    };
    let request = match parse_common(&object) {
        Ok(request) => GuestRequest {
            access_token: None,
            ..request
        },
        Err(_) => return status(StatusCode::BAD_REQUEST),
    };
    if let Some(retry_after) = rate_limited(&state, &headers, &request) {
        return retry(StatusCode::TOO_MANY_REQUESTS, retry_after);
    }
    let ws = request.workspace_id.to_string();
    if request.guest_conversation_id.contains(':')
        && !request.guest_conversation_id.starts_with(&format!("{ws}:"))
    {
        return status(StatusCode::BAD_REQUEST);
    }
    if !webchat::verify_access_token(
        &state.auth_secret,
        bearer_token(&headers).as_deref(),
        &ws,
        &request.webchat_id.to_string(),
        request.parent_origin.as_deref(),
        now_secs(),
    ) {
        return status(StatusCode::FORBIDDEN);
    }
    match workspace(&state, request.workspace_id).await {
        Ok(Some(w)) if chatbotx_business::workspace::is_scheduled_for_deletion(&w) => {
            return status(StatusCode::FORBIDDEN);
        }
        Ok(_) => {}
        Err(err) => {
            tracing::error!(
                err,
                workspace_id = ws,
                "Failed to issue guest realtime token"
            );
            return status(StatusCode::SERVICE_UNAVAILABLE);
        }
    }
    let webchat_row = match webchat::find_by_id_for_workspace_or_null(
        &state.api.db,
        request.webchat_id,
        request.workspace_id,
    )
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return status(StatusCode::NOT_FOUND),
        Err(e) => return error_reply(e.status().unwrap_or(400), &e.message(), HeaderMap::new()),
    };
    if !webchat::is_allowed_guest_origin(
        request.parent_origin.as_deref(),
        &webchat_row.authorized_domains,
        &app_host(&headers),
    ) {
        return status(StatusCode::FORBIDDEN);
    }
    if let Some(kv) = &state.api.kv {
        let limit = chatbotx_kv::rate_limit::check_api_rate_limit(
            chatbotx_kv::rate_limit::ApiRateLimitInput {
                scope: "guest-realtime-mint-workspace-rate-limit",
                key: &ws,
                store: &kv.mem,
                now_ms: now_ms(),
                limit: Some(MINT_WORKSPACE_LIMIT),
            },
        );
        if limit.limited {
            return retry(
                StatusCode::TOO_MANY_REQUESTS,
                i64::try_from(limit.retry_after).unwrap_or(10),
            );
        }
    }
    let Some(secret) = state.broadcast_secret.as_deref() else {
        tracing::error!(workspace_id = ws, "Failed to issue guest realtime token");
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    let token = chatbotx_rt::token::sign_guest_connect_token(
        &request.guest_conversation_id,
        &ws,
        secret,
        &chatbotx_rt::SystemClock,
    );
    json_reply(StatusCode::OK, HeaderMap::new(), json!({ "token": token }))
}

fn retry(code: StatusCode, retry_after: i64) -> Response {
    let mut response = code.into_response();
    if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}
