//! `app/api/reflink-widget/[reflinkId]/route.ts`: the public chat-widget
//! read of a ref link (CORS, guest rate limit, visitor language), and
//! `lib/rate-limit/guest-rate-limit.ts`, `features/reflinks/lib/widget-brand.ts`,
//! `widget-embed.ts`, and the `integration-webchat` helpers they use
//! (`readableForeground`, `isEmbedOriginAllowed`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get as route_get;
use chatbotx_business::inbox::links::LinkInbox;
use chatbotx_business::reflink;
use serde_json::{Value, json};

use crate::handlers::ApiState;

/// `UNKNOWN_CLIENT_IP`.
pub const UNKNOWN_CLIENT_IP: &str = "unknown";
const WINDOW_SECONDS: i64 = 10;
const IP_LIMIT: i64 = 60;
const SESSION_LIMIT: i64 = 20;
/// `DEFAULT_WIDGET_LOGO_BACKGROUND_COLOR`.
pub const DEFAULT_WIDGET_LOGO_BACKGROUND_COLOR: &str = "#111827";

/// `getGuestClientIp`: the first `x-forwarded-for` entry, else `x-real-ip`,
/// else [`UNKNOWN_CLIENT_IP`].
pub fn guest_client_ip(headers: &HeaderMap) -> String {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(forwarded) = text("x-forwarded-for") {
        let first = forwarded.split(',').next().unwrap_or("").trim();
        return if first.is_empty() {
            UNKNOWN_CLIENT_IP.to_string()
        } else {
            first.to_string()
        };
    }
    match text("x-real-ip").map(str::trim) {
        Some(ip) if !ip.is_empty() => ip.to_string(),
        _ => UNKNOWN_CLIENT_IP.to_string(),
    }
}

/// `checkGuestRateLimit` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRateLimit {
    pub limited: bool,
    /// Seconds until the next 10 s window.
    pub retry_after: i64,
}

/// `memoryCounters`: the local fallback (`count`, `expiresAt` ms).
fn memory_counters() -> &'static Mutex<HashMap<String, (i64, i64)>> {
    static COUNTERS: OnceLock<Mutex<HashMap<String, (i64, i64)>>> = OnceLock::new();
    COUNTERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn increment(state: &ApiState, key: &str, now_ms: i64) -> i64 {
    if let Some(kv) = &state.kv {
        return kv.mem.incr_with_window(key, WINDOW_SECONDS.unsigned_abs());
    }
    let mut counters = memory_counters()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    counters.retain(|_, (_, expires)| *expires > now_ms);
    let entry = counters
        .entry(key.to_string())
        .or_insert((0, now_ms + WINDOW_SECONDS * 1000));
    entry.0 += 1;
    entry.0
}

/// `checkGuestRateLimit`: 60 requests per IP (20 per guest conversation) in
/// a fixed 10 s window, on the kv store (a local counter when it is off).
pub fn check_guest_rate_limit(
    state: &ApiState,
    webchat_id: &str,
    client_ip: &str,
    guest_conversation_id: Option<&str>,
    now_ms: i64,
) -> GuestRateLimit {
    check_guest_rate_limit_with(state, webchat_id, client_ip, guest_conversation_id, IP_LIMIT, now_ms)
}

/// `checkGuestRateLimit({ webchatId, clientIp, ipLimit })`: no session key.
pub fn check_guest_ip_limit(state: &ApiState, webchat_id: &str, client_ip: &str, ip_limit: i64, now_ms: i64) -> GuestRateLimit {
    check_guest_rate_limit_with(state, webchat_id, client_ip, None, ip_limit, now_ms)
}

fn check_guest_rate_limit_with(
    state: &ApiState,
    webchat_id: &str,
    client_ip: &str,
    guest_conversation_id: Option<&str>,
    ip_limit: i64,
    now_ms: i64,
) -> GuestRateLimit {
    let window_ms = WINDOW_SECONDS * 1000;
    let suffix = now_ms.div_euclid(window_ms);
    let retry_after = (window_ms - now_ms.rem_euclid(window_ms) + 999) / 1000;
    let ip_key = format!("guest-rate-limit:ip:{webchat_id}:{client_ip}:{suffix}");
    if increment(state, &ip_key, now_ms) > ip_limit {
        return GuestRateLimit {
            limited: true,
            retry_after,
        };
    }
    if let Some(conversation) = guest_conversation_id {
        let key = format!("guest-rate-limit:session:{webchat_id}:{conversation}:{suffix}");
        if increment(state, &key, now_ms) > SESSION_LIMIT {
            return GuestRateLimit {
                limited: true,
                retry_after,
            };
        }
    }
    GuestRateLimit {
        limited: false,
        retry_after,
    }
}

// --- brand -------------------------------------------------------------------------

fn linear_channel(channel: u8) -> f64 {
    let normalized = f64::from(channel) / 255.0;
    if normalized <= 0.039_28 {
        normalized / 12.92
    } else {
        ((normalized + 0.055) / 1.055).powf(2.4)
    }
}

/// `readableForeground`: `#0a0a0a` on light colors, else `#ffffff`.
pub fn readable_foreground(hex_color: &str) -> &'static str {
    let Some(hex) = hex_color.strip_prefix('#') else {
        return "#ffffff";
    };
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return "#ffffff";
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or(0);
    let luminance = 0.2126 * linear_channel(channel(0))
        + 0.7152 * linear_channel(channel(2))
        + 0.0722 * linear_channel(channel(4));
    if luminance > 0.5 {
        "#0a0a0a"
    } else {
        "#ffffff"
    }
}

/// `isEmbedOriginAllowed` (shared with the webchat guest routes).
pub use chatbotx_business::integration_webchat::is_embed_origin_allowed;

/// `resolveWidgetBrand` (without the labels).
pub fn widget_brand(row: &reflink::ReflinkWithLogo, storage_url: &str) -> Value {
    let r = &row.reflink;
    let background = r
        .widget_logo_background_color
        .clone()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| DEFAULT_WIDGET_LOGO_BACKGROUND_COLOR.to_string());
    let has_powered_by = r
        .widget_brand_name
        .as_deref()
        .is_some_and(|v| !v.is_empty())
        && r.widget_brand_url.as_deref().is_some_and(|v| !v.is_empty());
    let logo_url = row
        .widget_logo_file_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .and_then(|path| {
            chatbotx_core::utils::storage::get_public_file_url(path, storage_url).ok()
        });
    let foreground = readable_foreground(background.get(..7).unwrap_or(&background));
    json!({
        "name": if has_powered_by { json!(r.widget_brand_name) } else { Value::Null },
        "url": if has_powered_by { json!(r.widget_brand_url) } else { Value::Null },
        "logoUrl": logo_url,
        "logoBackgroundColor": background,
        "logoForegroundColor": foreground,
    })
}

/// `buildReflinkWidgetScriptUrl`.
pub fn widget_script_url(app_url: &str) -> String {
    format!("{app_url}/chat-widget/ref-widget.js")
}

/// `buildReflinkWidgetEmbedCode`.
pub fn widget_embed_code(app_url: &str, reflink_id: &str) -> String {
    format!(
        r#"<script async src="{}" data-reflink-id="{reflink_id}"></script>"#,
        widget_script_url(app_url)
    )
}

/// The ids in a stored JSON string array.
pub fn json_strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

// --- visitor language --------------------------------------------------------------

/// `locales` of the builder (`i18n/config.ts`).
const LOCALES: &[&str] = &[
    "en", "ar", "az", "da", "de", "es", "fi", "fr", "he", "id", "it", "ja", "nl", "pt-BR", "pt-PT",
    "ro", "sv", "tr", "vi", "zh-CN", "zh-TW",
];

/// `resolveLocale`.
pub fn resolve_locale(value: Option<&str>) -> &'static str {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return "en";
    };
    if let Some(found) = LOCALES.iter().find(|l| **l == value) {
        return found;
    }
    let lower = value.to_lowercase();
    let language = lower.split('-').next().unwrap_or("");
    if language.is_empty() {
        return "en";
    }
    if language == "pt" {
        return "pt-BR";
    }
    let matches = |tag: &str| lower == tag || lower.starts_with(&format!("{tag}-"));
    if ["zh-hant", "zh-hk", "zh-mo", "zh-tw"]
        .iter()
        .any(|t| matches(t))
    {
        return "zh-TW";
    }
    if ["zh-cn", "zh-hans"].iter().any(|t| matches(t)) || language == "zh" {
        return "zh-CN";
    }
    LOCALES
        .iter()
        .find(|l| l.to_lowercase().split('-').next() == Some(language))
        .copied()
        .unwrap_or("en")
}

/// `reflinks.chatWidget.poweredBy` / `.toggleLabel` of `messages/<locale>.json`.
fn widget_labels(locale: &str) -> (&'static str, &'static str) {
    match locale {
        "ar" => ("بواسطة", "تحدث معنا"),
        "az" => ("tərəfindən", "Bizimlə söhbət edin"),
        "da" => ("af", "Chat med os"),
        "de" => ("von", "Chatte mit uns"),
        "es" => ("por", "Chatea con nosotros"),
        "fi" => ("tekijä", "Keskustele kanssamme"),
        "fr" => ("par", "Discutez avec nous"),
        "he" => ("מאת", "שוחחו איתנו"),
        "id" => ("oleh", "Chat dengan kami"),
        "it" => ("da", "Chatta con noi"),
        "ja" => ("提供", "チャットで相談"),
        "nl" => ("door", "Chat met ons"),
        "pt-BR" => ("por", "Converse com a gente"),
        "pt-PT" => ("por", "Fale connosco"),
        "ro" => ("de", "Discută cu noi"),
        "sv" => ("av", "Chatta med oss"),
        "tr" => ("sağlayan", "Bizimle sohbet edin"),
        "vi" => ("bởi", "Trò chuyện với chúng tôi"),
        "zh-CN" => ("由", "与我们聊天"),
        "zh-TW" => ("由", "與我們聊天"),
        _ => ("by", "Chat with us"),
    }
}

// --- route -------------------------------------------------------------------------

fn cors_headers(origin: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, OPTIONS"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::VARY,
        HeaderValue::from_static("Origin, Accept-Language"),
    );
    if let Some(origin) = origin.and_then(|o| HeaderValue::from_str(o).ok()) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    headers
}

fn empty(status: StatusCode, origin: Option<&str>) -> Response {
    (status, cors_headers(origin), Body::empty()).into_response()
}

fn origin_of(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
}

/// `OPTIONS`: 204 with the CORS headers.
pub fn options(headers: &HeaderMap) -> Response {
    empty(StatusCode::NO_CONTENT, origin_of(headers))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The widget's channel list: the open-chat links minus the hidden inboxes.
pub fn widget_links(
    app_url: &str,
    inboxes: &[LinkInbox],
    row: &reflink::ReflinkWithLogo,
) -> Vec<Value> {
    let hidden = json_strings(&row.reflink.widget_hidden_inbox_ids.0);
    crate::handlers::reflinks::links(app_url, inboxes, &row.reflink.name)
        .into_iter()
        .filter(|link| {
            !hidden
                .iter()
                .any(|h| link["inboxId"].as_str() == Some(h.as_str()))
        })
        .collect()
}

/// `GET /api/reflink-widget/{reflinkId}`.
pub async fn get(state: &ApiState, reflink_id: &str, headers: &HeaderMap) -> Response {
    let origin = origin_of(headers);
    let valid_id = !reflink_id.is_empty()
        && reflink_id.len() <= 18
        && reflink_id.bytes().all(|b| b.is_ascii_digit());
    let Some(id) = valid_id.then(|| reflink_id.parse::<i64>().ok()).flatten() else {
        return empty(StatusCode::NOT_FOUND, origin);
    };
    let client_ip = guest_client_ip(headers);
    if client_ip != UNKNOWN_CLIENT_IP {
        let limit = check_guest_rate_limit(
            state,
            &format!("reflink-widget:{reflink_id}"),
            &client_ip,
            None,
            now_ms(),
        );
        if limit.limited {
            let mut out = cors_headers(origin);
            if let Ok(value) = HeaderValue::from_str(&limit.retry_after.to_string()) {
                out.insert(header::RETRY_AFTER, value);
            }
            return (StatusCode::TOO_MANY_REQUESTS, out, Body::empty()).into_response();
        }
    }
    let row = match reflink::find_for_widget(&state.db, id).await {
        Ok(Some(row)) => row,
        Ok(None) => return empty(StatusCode::NOT_FOUND, origin),
        Err(error) => {
            tracing::error!(reflink_id, error = %error.message(), "Failed to load reflink widget");
            return empty(StatusCode::INTERNAL_SERVER_ERROR, origin);
        }
    };
    let domains = json_strings(&row.reflink.widget_authorized_domains.0);
    if !is_embed_origin_allowed(origin, &domains) {
        return empty(StatusCode::FORBIDDEN, None);
    }
    let inboxes = match crate::handlers::reflinks::linkable_inboxes(
        &state.db,
        row.reflink.workspace_id,
    )
    .await
    {
        Ok(inboxes) => inboxes,
        Err(_) => return empty(StatusCode::INTERNAL_SERVER_ERROR, origin),
    };
    let tenant = crate::handlers::contacts::tenant_urls(state);
    let channels: Vec<Value> = widget_links(&tenant.app_url, &inboxes, &row)
        .into_iter()
        .map(|link| json!({ "channel": link["channel"], "name": link["inboxName"], "url": link["url"] }))
        .collect();
    let first_language = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.split(';').next());
    let (powered_by, toggle) = widget_labels(resolve_locale(first_language));
    let mut brand = widget_brand(&row, &tenant.storage_url);
    if let Some(obj) = brand.as_object_mut() {
        obj.insert("poweredByLabel".into(), json!(powered_by));
        obj.insert("toggleLabel".into(), json!(toggle));
    }
    let body = json!({ "channels": channels, "brand": brand });
    let mut out = cors_headers(origin);
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    (StatusCode::OK, out, body.to_string()).into_response()
}

/// The `/api/reflink-widget/{reflinkId}` routes.
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route(
            "/api/reflink-widget/{reflinkId}",
            route_get(
                |State(state): State<Arc<ApiState>>, Path(id): Path<String>, headers: HeaderMap| async move {
                    self::get(&state, &id, &headers).await
                },
            )
            .options(|headers: HeaderMap| async move { options(&headers) }),
        )
        .with_state(state)
}
