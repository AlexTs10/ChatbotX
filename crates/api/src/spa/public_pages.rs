//! The public pages of Q.F2-admin goal 3 (no builder login): their
//! `/rpc/pages/<name>` loaders and the `/go/{workspaceId}` document.
//!
//! - `unsubscribe` (`app/unsubscribe/page.tsx`): verifies the token and
//!   clears `emailOptIn`.
//! - `connectSession` (`app/connect/[sessionId]/page.tsx`): the coarse
//!   status kind only (`resolveConnectSessionMessageKind`).
//! - `landing` (`app/l/[workspaceId]/[id]` and `.../p/[id]`): the inbox links
//!   of a QR code / reflink or a poll.
//! - `botSimulator` (`app/bs/[workspaceId]/[webchatId]/page.tsx`).
//! - `datetimePicker` / `submitDateTime` (`app/extensions/datetime-picker`).
//! - `webchat` (`app/(no-sidebar)/webchat/page.tsx`): the embed checks and
//!   the session-scoped access token.
//! - `GET /go/{workspaceId}` (`app/go/[workspaceId]/page.tsx`): App Links
//!   meta tags for crawlers, so it is server-rendered HTML, not the SPA.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use chatbotx_business::integration_webchat as webchat;
use chatbotx_business::tx;
use chatbotx_db::rusqlite::OptionalExtension;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

use super::{error, internal, ok};
use crate::handlers::ApiState;
use crate::session::AuthState;

fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "NOT_FOUND", "Not found")
}

fn str_of<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key)?.as_str().filter(|s| !s.is_empty())
}

/// `getIdFromParams` / `zodBigintAsString`: a positive decimal id.
fn id_of(input: &Value, key: &str) -> Option<i64> {
    let text = str_of(input, key)?;
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|id| *id > 0)
}

/// `loadServableWorkspace`: the workspace, or `None` when it is missing or
/// scheduled for deletion.
async fn servable(
    api: &ApiState,
    workspace_id: i64,
) -> Option<chatbotx_db::repo::models::Workspace> {
    tx::read(&api.db, move |conn| {
        Ok(chatbotx_business::workspace::find(conn, workspace_id)?)
    })
    .await
    .ok()
    .flatten()
    .filter(|w| w.scheduled_deletion_at.is_none())
}

/// The loaders of this module; `None` = not one of them.
pub(super) async fn page(
    state: &AuthState,
    api: Option<&ApiState>,
    name: &str,
    headers: &HeaderMap,
    input: &Value,
) -> Option<Response> {
    if !matches!(
        name,
        "unsubscribe"
            | "connectSession"
            | "landing"
            | "botSimulator"
            | "datetimePicker"
            | "submitDateTime"
            | "webchat"
    ) {
        return None;
    }
    let Some(api) = api else {
        return Some(internal("no app state"));
    };
    Some(match name {
        "unsubscribe" => unsubscribe(api, input).await,
        "connectSession" => connect_session(api, input).await,
        "landing" => landing(api, input).await,
        "botSimulator" => bot_simulator(api, input).await,
        "datetimePicker" => datetime_picker(api, input).await,
        "submitDateTime" => submit_date_time(api, input).await,
        _ => webchat_page(state, api, headers, input).await,
    })
}

// ---- /unsubscribe ------------------------------------------------------------

async fn unsubscribe(api: &ApiState, input: &Value) -> Response {
    let state = |s: &str| ok(json!({ "state": s }));
    let Some(token) = str_of(input, "token") else {
        return state("invalid");
    };
    let Ok((contact_id, workspace_id)) =
        chatbotx_business::email_marketing::smtp::verify_unsubscribe_token(token)
    else {
        return state("invalid");
    };
    if servable(api, workspace_id).await.is_none() {
        return state("unavailable");
    }
    // `contactService.unsubscribeEmail(cid)`.
    let cleared = tx::write(&api.db, move |tx| {
        tx.execute(
            r#"UPDATE "Contact" SET "emailOptIn" = 0 WHERE "id" = ?1"#,
            [contact_id],
        )
        .map_err(chatbotx_db::repo::Error::from)?;
        Ok(())
    })
    .await;
    match cleared {
        Ok(()) => state("done"),
        Err(_) => state("invalid"),
    }
}

// ---- /connect/[sessionId] ----------------------------------------------------

/// `resolveConnectSessionMessageKind`.
pub fn connect_session_kind(status: Option<&str>) -> &'static str {
    match status {
        None => "invalid",
        Some("pending" | "authorized" | "awaiting_selection") => "processing",
        Some("completed") => "completed",
        Some("cancelled") => "cancelled",
        Some("expired") => "expired",
        Some(_) => "failed",
    }
}

async fn connect_session(api: &ApiState, input: &Value) -> Response {
    let status = match id_of(input, "sessionId") {
        Some(id) => tx::read(&api.db, move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "status" FROM "ConnectSession" WHERE "id" = ?1"#,
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(chatbotx_db::repo::Error::from)?)
        })
        .await
        .unwrap_or(None),
        None => None,
    };
    ok(json!({ "kind": connect_session_kind(status.as_deref()) }))
}

// ---- /l/[workspaceId]/[id] and /l/[workspaceId]/p/[id] -----------------------

async fn landing(api: &ApiState, input: &Value) -> Response {
    use chatbotx_business::inbox::links::{get_inbox_links, list_connected_link_inboxes};
    let (Some(ws), Some(id)) = (id_of(input, "workspaceId"), id_of(input, "id")) else {
        return not_found();
    };
    if servable(api, ws).await.is_none() {
        return not_found();
    }
    let inboxes = match tx::read(&api.db, move |conn| {
        Ok(list_connected_link_inboxes(conn, ws)?)
    })
    .await
    {
        Ok(inboxes) => inboxes,
        Err(e) => return internal(e),
    };
    let app_url = api.app_url.trim_end_matches('/');
    let links: Vec<(i64, String, String, String)> = if input.get("poll") == Some(&Value::Bool(true))
    {
        // `loadPollLinks`: a 404 for a poll of another workspace.
        if chatbotx_business::poll::service::find(&api.db, ws, id)
            .await
            .is_err()
        {
            return not_found();
        }
        chatbotx_business::poll::build_poll_links(app_url, ws, &inboxes, id)
            .links
            .into_iter()
            .map(|l| (l.inbox_id, l.channel, l.name, l.url))
            .collect()
    } else {
        let Ok(Some(qr)) = chatbotx_business::qr_code::find(&api.db, ws, id).await else {
            return not_found();
        };
        let config = chatbotx_business::referral::RefConfig::Reflink { name: qr.name };
        get_inbox_links(app_url, &inboxes, Some(&config))
            .into_iter()
            .map(|(inbox, url)| (inbox.id, inbox.channel, inbox.name, url))
            .collect()
    };
    if links.is_empty() {
        return not_found();
    }
    ok(json!({
        "links": links.into_iter().map(|(inbox_id, channel, name, url)| json!({
            "inbox": { "id": inbox_id.to_string(), "channel": channel, "name": name },
            "url": url,
        })).collect::<Vec<_>>(),
    }))
}

// ---- /bs/[workspaceId]/[webchatId] -------------------------------------------

/// `parseSimulatorWebsiteUrl`: an http(s) URL, else `None`.
pub fn parse_simulator_website_url(value: Option<&str>) -> Option<url::Url> {
    let parsed = url::Url::parse(value?.trim()).ok()?;
    matches!(parsed.scheme(), "http" | "https").then_some(parsed)
}

async fn bot_simulator(api: &ApiState, input: &Value) -> Response {
    let website = parse_simulator_website_url(str_of(input, "url"));
    let (Some(ws), Some(webchat_id), Some(website)) = (
        id_of(input, "workspaceId"),
        id_of(input, "webchatId"),
        website,
    ) else {
        return not_found();
    };
    if servable(api, ws).await.is_none() {
        return not_found();
    }
    let Ok(Some(row)) = webchat::find_by_id_for_workspace_or_null(&api.db, webchat_id, ws).await
    else {
        return not_found();
    };
    // `isSimulatorWebsiteAllowed`: an empty list allows any website.
    let origin = website.origin().ascii_serialization();
    if !(row.enable
        && (row.authorized_domains.is_empty()
            || webchat::is_origin_authorized(Some(&origin), &row.authorized_domains)))
    {
        return not_found();
    }
    ok(json!({
        "brandColor": row.brand_color,
        "webchatId": row.id.to_string(),
        "websiteUrl": website.to_string(),
    }))
}

// ---- /extensions/datetime-picker ---------------------------------------------

fn webview_claims(token: &str) -> Option<chatbotx_core::encryption::UserDataWebviewClaims> {
    chatbotx_business::meta_catalog::process_encryption()
        .verify_user_data_webview_token(token)
        .ok()
        .map(|payload| payload.claims)
}

async fn datetime_picker(api: &ApiState, input: &Value) -> Response {
    let Some(claims) = str_of(input, "token").and_then(webview_claims) else {
        return ok(json!({ "state": "invalid" }));
    };
    let Some(workspace) = claims
        .workspace_id
        .parse::<i64>()
        .ok()
        .map(|ws| servable(api, ws))
    else {
        return not_found();
    };
    let Some(workspace) = workspace.await else {
        return not_found();
    };
    ok(json!({
        "state": "ok",
        "mode": claims.reply_format,
        "language": workspace.language,
    }))
}

/// `submitDateTimeAction`: every field but `selectedValue` comes from the
/// verified token; the worker persists the field and consumes the challenge.
async fn submit_date_time(api: &ApiState, input: &Value) -> Response {
    let invalid = || {
        error(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "Input validation failed",
        )
    };
    let (Some(token), Some(selected)) = (str_of(input, "token"), str_of(input, "selectedValue"))
    else {
        return invalid();
    };
    // `z.iso.datetime()`.
    if !chatbotx_core::flow_config::z::string()
        .iso_datetime(false, false, None)
        .safe_parse(&json!(selected))
        .success
    {
        return invalid();
    }
    let Some(claims) = webview_claims(token) else {
        return error(StatusCode::BAD_REQUEST, "invalidToken", "Invalid token");
    };
    let Some(jobs) = api.jobs.as_deref() else {
        return internal("no job queue");
    };
    let data = json!({
        "type": "sendFlow",
        "data": {
            "conversationId": claims.conversation_id,
            "contactInboxId": claims.contact_inbox_id,
            "flowId": claims.flow_id,
            "flowVersionId": claims.flow_version_id,
            "nodeId": claims.node_id,
            "startFromStepId": claims.step_id,
            "metadata": {
                "type": "getUserDataWebviewSelection" /* GET_USER_DATA_WEBVIEW_SELECTION_PAYLOAD_TYPE */,
                "stepId": claims.step_id,
                "challengeId": claims.challenge_id,
                "contactInboxId": claims.contact_inbox_id,
                "selectedValue": selected,
            },
            "origin": "channel",
        },
    });
    match jobs
        .add(
            chatbotx_jobs::QueueName::Integration,
            "sendFlow",
            data,
            chatbotx_jobs::JobOptions::default(),
        )
        .await
    {
        Ok(_) => ok(json!({ "completed": true })),
        Err(e) => internal(e),
    }
}

// ---- /webchat ----------------------------------------------------------------

/// The embed checks of the webchat page. The SPA sends the frame's
/// `document.referrer` (the builder read the document request's `Referer`)
/// and whether it runs in a frame (`Sec-Fetch-Dest: iframe`).
async fn webchat_page(
    state: &AuthState,
    api: &ApiState,
    headers: &HeaderMap,
    input: &Value,
) -> Response {
    let (Some(ws), Some(webchat_id)) = (id_of(input, "workspaceId"), id_of(input, "webchatId"))
    else {
        return not_found();
    };
    let Ok(Some(row)) = webchat::find_by_id_for_workspace_or_null(&api.db, webchat_id, ws).await
    else {
        return not_found();
    };
    let workspace = tx::read(&api.db, move |conn| {
        Ok(chatbotx_business::workspace::find(conn, ws)?)
    })
    .await
    .ok()
    .flatten();
    if workspace
        .as_ref()
        .is_some_and(|w| w.scheduled_deletion_at.is_some())
    {
        return ok(json!({ "state": "unavailable" }));
    }
    let referer = str_of(input, "referrer");
    let app_host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let first_party_referer =
        referer.is_some() && webchat::is_first_party_origin(referer, app_host);
    let refererless_iframe = referer.is_none()
        && input.get("inIframe") == Some(&Value::Bool(true))
        && !row.authorized_domains.is_empty();
    if refererless_iframe
        || !(webchat::is_first_party_origin(referer, app_host)
            || webchat::is_origin_authorized(referer, &row.authorized_domains))
    {
        return ok(json!({ "state": "unauthorized" }));
    }
    let guest_conversation_id = str_of(input, "guestConversationId")
        .map(str::to_owned)
        .unwrap_or_else(|| webchat::create_guest_conversation_id(ws));
    // Only a first-party referer makes the URL origin parameters trustworthy.
    let token_origin = if first_party_referer {
        str_of(input, "parentOrigin")
            .or_else(|| str_of(input, "domain"))
            .or(referer)
    } else {
        referer
    };
    let now_secs = chatbotx_db::repo::now_ms() / 1000;
    let access_token = webchat::create_access_token(
        &state.settings.secret,
        &ws.to_string(),
        &row.id.to_string(),
        token_origin,
        now_secs,
    );
    let urls = crate::handlers::contacts::tenant_urls(api);
    let logo_url = match (&workspace, row.show_logo) {
        (Some(w), true) => w.logo.as_deref().and_then(|logo| {
            chatbotx_core::utils::storage::get_public_file_url(logo, &urls.storage_url).ok()
        }),
        _ => None,
    };
    // Community: the "Built with" branding menu entry is always there.
    let menus = if api.edition == chatbotx_business::keys::Edition::Community {
        webchat::brand_webchat_menus(row.persistent_menus.clone(), &api.app_url)
    } else {
        row.persistent_menus.clone()
    };
    ok(json!({
        "state": "ok",
        "accessToken": access_token,
        "guestConversationId": guest_conversation_id,
        "workspaceLogoUrl": logo_url,
        "customCss": row.custom_css,
        // `toWebchatClientConfig`: the client-safe fields only.
        "config": {
            "id": row.id.to_string(),
            "workspaceId": ws.to_string(),
            "name": row.name,
            "brandColor": row.brand_color,
            "hideHeader": row.hide_header,
            "showLogo": row.show_logo,
            "hideMessageInput": row.hide_message_input,
            "welcomeFlowId": row.welcome_flow_id.map(|id| id.to_string()),
            "persistentMenus": menus,
        },
    }))
}

// ---- GET /go/{workspaceId} ---------------------------------------------------

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// `resolveOpenLinkDestination`: the HMAC-verified http(s) destination.
pub fn open_link_destination(
    encryption: &chatbotx_core::encryption::Encryption,
    workspace_id: &str,
    u: Option<&str>,
    s: Option<&str>,
) -> Option<String> {
    use base64::Engine as _;
    let (u, s) = (u.filter(|v| !v.is_empty())?, s.filter(|v| !v.is_empty())?);
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(u.trim_end_matches('='))
        .ok()?;
    let url = String::from_utf8_lossy(&bytes).into_owned();
    let parsed = url::Url::parse(&url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let params = chatbotx_core::encryption::OpenLinkParams {
        workspace_id: workspace_id.to_string(),
        url: url.clone(),
    };
    encryption
        .verify_open_link_url(&params, Some(s))
        .then_some(url)
}

const PAGE_STYLE: &str = "body{margin:0;font-family:system-ui,sans-serif}main{display:flex;min-height:100vh;align-items:center;justify-content:center;padding:24px;text-align:center}a.btn{display:block;width:100%;max-width:20rem;padding:12px 16px;border-radius:8px;background:#111;color:#fff;text-decoration:none;font-weight:600}p{color:#666}";

fn document(head: &str, body: &str) -> Response {
    let html = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex,nofollow\"><title>Open link</title><style>{PAGE_STYLE}</style>{head}</head><body>{body}</body></html>"
    );
    ([(header::CACHE_CONTROL, "no-store")], Html(html)).into_response()
}

/// `labelKeyByAppId` with the `messages/en.json` text.
fn open_label(app_id: &str) -> &'static str {
    match app_id {
        "zaloGroup" => "Open Zalo group",
        "zalo" => "Open Zalo",
        _ => "Open Messenger",
    }
}

pub(super) async fn open_link(
    State(api): State<Option<Arc<ApiState>>>,
    Path(workspace): Path<String>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let query: HashMap<String, String> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    use chatbotx_core::flow_config::deep_link::{
        build_app_link_targets, match_deep_link_app, resolve_deep_link_navigation,
    };
    let Some(api) = api else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(ws) = id_of(&json!({ "w": workspace }), "w") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if servable(&api, ws).await.is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let encryption = chatbotx_business::meta_catalog::process_encryption();
    let Some(destination) = open_link_destination(
        encryption,
        &ws.to_string(),
        query.get("u").map(String::as_str),
        query.get("s").map(String::as_str),
    ) else {
        // A forged destination is an error, never a redirect (no open redirect).
        return document(
            "",
            "<main><div><h1>Invalid link.</h1><p>This link is invalid or has expired.</p></div></main>",
        );
    };
    let Some(app) = match_deep_link_app(&destination) else {
        return (
            StatusCode::TEMPORARY_REDIRECT,
            [(header::LOCATION, destination)],
        )
            .into_response();
    };
    let targets = build_app_link_targets(&destination, app);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let (href, auto) = resolve_deep_link_navigation(&targets, user_agent);
    // `AppLinksMeta`: `og:url` is this page, never the destination.
    let page_url = format!(
        "{}/go/{ws}?{}",
        api.app_url.trim_end_matches('/'),
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(query.get("u").map(|u| ("u", u.as_str())))
            .extend_pairs(query.get("s").map(|s| ("s", s.as_str())))
            .finish()
    );
    let mut head = format!(
        "<meta property=\"al:android:package\" content=\"{}\"><meta property=\"al:android:app_name\" content=\"{}\">",
        escape(&targets.android_package),
        escape(app.app_name)
    );
    if let Some(ios_url) = &targets.ios_url {
        head.push_str(&format!(
            "<meta property=\"al:ios:url\" content=\"{}\">",
            escape(ios_url)
        ));
        if let Some(store_id) = &targets.ios_app_store_id {
            head.push_str(&format!(
                "<meta property=\"al:ios:app_store_id\" content=\"{}\">",
                escape(store_id)
            ));
        }
        head.push_str(&format!(
            "<meta property=\"al:ios:app_name\" content=\"{}\">",
            escape(app.app_name)
        ));
    }
    head.push_str(&format!(
        "<meta property=\"al:web:should_fallback\" content=\"true\"><meta property=\"og:url\" content=\"{}\"><meta property=\"og:title\" content=\"{}\">",
        escape(&page_url),
        escape(app.app_name)
    ));
    // `OpenLinkRedirect`: before paint; `<` escaped so it cannot close the script.
    let redirect = if auto {
        let encoded = serde_json::to_string(&href)
            .unwrap_or_default()
            .replace('<', "\\u003c");
        format!(
            "<script>(function(){{var d={encoded};try{{var k=\"chatbotx.openLink.autoNavigated:\"+d;if(sessionStorage.getItem(k))return;sessionStorage.setItem(k,\"1\")}}catch(e){{}}location.replace(d)}})()</script>"
        )
    } else {
        String::new()
    };
    document(
        &head,
        &format!(
            "{redirect}<main><a class=\"btn\" href=\"{}\" rel=\"noopener noreferrer\">{}</a></main>",
            escape(&href),
            open_label(app.id)
        ),
    )
}
