//! The channel provider calls shared by the token refresh (`refreshAuth` of
//! the channel integrations) and the contact profile fetches
//! (`contact.getProfile`, `getProfileSnapshot`, `getContactProfilePicUrl`):
//! Meta Graph (Messenger, Instagram, WhatsApp), Zalo, TikTok, Threads and
//! Telegram. One injectable transport ([`ChannelHttp`]) so the API, the
//! worker and the tests share the same code.

use std::sync::OnceLock;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::connection::credentials::{HttpUpstream, Upstream, UpstreamReply, UpstreamRequest};
use crate::net::ssrf_guard::assert_public_url;

pub const GRAPH_API_URL: &str = "https://graph.facebook.com";
pub const INSTAGRAM_API_URL: &str = "https://graph.instagram.com";
pub const THREADS_API_URL: &str = "https://graph.threads.com";
pub const ZALO_OAUTH_URL: &str = "https://oauth.zaloapp.com";
pub const ZALO_API_URL: &str = "https://openapi.zalo.me";
pub const TIKTOK_API_URL: &str = "https://business-api.tiktok.com/open_api/v1.3/";
pub const TELEGRAM_API_URL: &str = "https://api.telegram.org";
/// `DEFAULT_API_VERSION` of the Messenger / WhatsApp / Facebook-login
/// Instagram integrations.
pub const META_DEFAULT_VERSION: &str = "v23.0";
/// `DEFAULT_API_VERSION` of the Instagram Business Login integration.
pub const INSTAGRAM_DEFAULT_VERSION: &str = "v22.0";
/// `DEFAULT_MEDIA_MAX_BYTES` of `fetchMediaWithLimits`.
const MEDIA_MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

/// A downloaded file: the bytes and the declared `Content-Type`.
pub type Download = (Bytes, Option<String>);

/// The provider transport. `send` = one JSON API call; `download` = a media
/// GET (`fetchMediaWithLimits`): `Ok(None)` for a non-2xx answer or a body
/// over 10 MB.
pub trait ChannelHttp: Send + Sync {
    fn send(&self, request: UpstreamRequest) -> BoxFuture<'_, UpstreamReply>;
    fn download(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> BoxFuture<'_, Result<Option<Download>, String>>;
}

/// [`ChannelHttp`] on reqwest: the API calls through [`HttpUpstream`] (fixed
/// provider hosts), the downloads SSRF-checked at every redirect hop, the
/// token sent only to the first URL's origin.
#[derive(Debug, Clone, Copy, Default)]
pub struct LiveChannelHttp;

fn media_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default()
    })
}

fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}

async fn download_checked(url: &str, bearer: Option<&str>) -> Result<Option<Download>, String> {
    let mut current = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        assert_public_url(&current, "Media URL").await?;
        let mut request = media_client().get(&current).header("User-Agent", "node");
        if let Some(token) = bearer.filter(|_| same_origin(url, &current)) {
            request = request.bearer_auth(token);
        }
        let mut response = request.send().await.map_err(|e| e.to_string())?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("Redirect response has no Location header")?;
            current = url::Url::parse(&current)
                .and_then(|base| base.join(location))
                .map_err(|e| e.to_string())?
                .to_string();
            continue;
        }
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|len| len > MEDIA_MAX_BYTES as u64)
        {
            return Ok(None);
        }
        let declared = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut out = BytesMut::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            if out.len() + chunk.len() > MEDIA_MAX_BYTES {
                return Ok(None);
            }
            out.extend_from_slice(&chunk);
        }
        return Ok(Some((out.freeze(), declared)));
    }
    Err("Too many redirects while fetching media".into())
}

impl ChannelHttp for LiveChannelHttp {
    fn send(&self, request: UpstreamRequest) -> BoxFuture<'_, UpstreamReply> {
        Box::pin(HttpUpstream.send(request))
    }

    fn download(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> BoxFuture<'_, Result<Option<Download>, String>> {
        Box::pin(async move { download_checked(&url, bearer.as_deref()).await })
    }
}

/// A failed provider call: the provider message, the HTTP status (`None` =
/// no answer) and the provider error code / sub code / type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiFailure {
    pub message: String,
    pub status: Option<u16>,
    pub code: Option<i64>,
    pub sub_code: Option<i64>,
    pub kind: Option<String>,
}

impl ApiFailure {
    pub fn new(message: impl Into<String>) -> ApiFailure {
        ApiFailure {
            message: message.into(),
            ..ApiFailure::default()
        }
    }

    /// The Meta `REVOKED_TOKEN_SUBCODES` check (`isRevokedTokenError` of
    /// Messenger, Instagram, Threads): code 190 with sub code 458 / 460 /
    /// 463 / 467.
    pub fn is_graph_revoked(&self) -> bool {
        self.code == Some(190)
            && self
                .sub_code
                .is_some_and(|s| [458, 460, 463, 467].contains(&s))
    }

    /// WhatsApp `isRevokedTokenError`: also a 190 without a sub code, and
    /// `GraphMethodException` 100 / 33.
    pub fn is_whatsapp_revoked(&self) -> bool {
        let method = self.kind.as_deref() == Some("GraphMethodException")
            && self.code == Some(100)
            && self.sub_code == Some(33);
        method || (self.code == Some(190) && (self.sub_code.is_none() || self.is_graph_revoked()))
    }
}

impl std::fmt::Display for ApiFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn number(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

/// The Graph `{ error: { message, type, code, error_subcode } }` of an answer.
fn graph_failure(status: Option<u16>, body: &Value, fallback: &str) -> ApiFailure {
    let error = &body["error"];
    ApiFailure {
        message: error["message"]
            .as_str()
            .map_or_else(|| fallback.to_owned(), str::to_owned),
        status,
        code: number(&error["code"]),
        sub_code: number(&error["error_subcode"]),
        kind: error["type"].as_str().map(str::to_owned),
    }
}

fn get(url: String, bearer: Option<&str>) -> UpstreamRequest {
    UpstreamRequest {
        method: "GET",
        url,
        headers: bearer
            .map(|token| vec![("Authorization".to_owned(), format!("Bearer {token}"))])
            .unwrap_or_default(),
        body: None,
    }
}

fn with_query(base: &str, pairs: &[(&str, &str)]) -> String {
    match url::Url::parse(base) {
        Ok(mut url) => {
            if !pairs.is_empty() {
                url.query_pairs_mut().extend_pairs(pairs.iter().copied());
            }
            url.to_string()
        }
        Err(_) => base.to_owned(),
    }
}

/// One Graph-style call (Graph, Instagram, Threads): the JSON answer, or the
/// provider error of a non-2xx answer.
pub async fn graph_call(
    http: &dyn ChannelHttp,
    request: UpstreamRequest,
    endpoint: &str,
) -> Result<Value, ApiFailure> {
    match http.send(request).await {
        Err(message) => Err(ApiFailure::new(message)),
        Ok((status, text)) => {
            let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if (200..300).contains(&status) {
                Ok(body)
            } else {
                Err(graph_failure(Some(status), &body, endpoint))
            }
        }
    }
}

/// `GET {base}/{path}?{query}` with the bearer token.
pub async fn graph_get(
    http: &dyn ChannelHttp,
    base: &str,
    path: &str,
    query: &[(&str, &str)],
    bearer: Option<&str>,
) -> Result<Value, ApiFailure> {
    let url = with_query(&format!("{base}/{path}"), query);
    graph_call(http, get(url, bearer), path).await
}

/// `exchangeLongLivedToken` (Messenger, Facebook-login Instagram, WhatsApp):
/// `GET {version}/oauth/access_token?grant_type=fb_exchange_token`.
pub async fn exchange_long_lived_token(
    http: &dyn ChannelHttp,
    version: &str,
    client_id: &str,
    client_secret: &str,
    access_token: &str,
) -> Result<String, ApiFailure> {
    let answer = graph_get(
        http,
        GRAPH_API_URL,
        &format!("{version}/oauth/access_token"),
        &[
            ("grant_type", "fb_exchange_token"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("fb_exchange_token", access_token),
        ],
        None,
    )
    .await?;
    Ok(answer["access_token"]
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

/// `refreshLongLivedToken` (Instagram Business Login): `(token, expires_in)`.
pub async fn refresh_instagram_token(
    http: &dyn ChannelHttp,
    access_token: &str,
) -> Result<(String, i64), ApiFailure> {
    let answer = graph_get(
        http,
        INSTAGRAM_API_URL,
        "refresh_access_token",
        &[
            ("grant_type", "ig_refresh_token"),
            ("access_token", access_token),
        ],
        None,
    )
    .await?;
    Ok((
        answer["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        number(&answer["expires_in"]).unwrap_or_default(),
    ))
}

/// Threads `refreshAccessToken`: `(token, expires_in)`.
pub async fn refresh_threads_token(
    http: &dyn ChannelHttp,
    access_token: &str,
) -> Result<(String, Option<i64>), ApiFailure> {
    let answer = graph_get(
        http,
        THREADS_API_URL,
        "refresh_access_token",
        &[
            ("grant_type", "th_refresh_token"),
            ("access_token", access_token),
        ],
        None,
    )
    .await?;
    Ok((
        answer["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        number(&answer["expires_in"]).filter(|s| *s != 0),
    ))
}

/// Zalo `refreshAccessToken`: the token answer (`access_token`,
/// `refresh_token`, `expires_in`). A Zalo error is `error` (its code) +
/// a message; an answer without both tokens is an error too.
pub async fn refresh_zalo_token(
    http: &dyn ChannelHttp,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<Value, ApiFailure> {
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("app_id", client_id)
        .append_pair("app_secret", client_secret)
        .append_pair("refresh_token", refresh_token)
        .append_pair("grant_type", "refresh_token")
        .finish();
    let reply = http
        .send(UpstreamRequest {
            method: "POST",
            url: format!("{ZALO_OAUTH_URL}/v4/oa/access_token"),
            headers: vec![
                (
                    "Content-Type".into(),
                    "application/x-www-form-urlencoded".into(),
                ),
                ("secret_key".into(), client_secret.to_owned()),
            ],
            body: Some(form),
        })
        .await;
    let (status, text) = reply.map_err(ApiFailure::new)?;
    let tokens: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    zalo_failure(status, &tokens)?;
    let has = |key: &str| tokens[key].as_str().is_some_and(|t| !t.is_empty());
    if !(has("access_token") && has("refresh_token")) {
        return Err(ApiFailure::new(format!(
            "Refresh access token failed: {tokens}"
        )));
    }
    Ok(tokens)
}

/// `ZaloHttpClient.handleResponse`: a non-2xx answer, a non-zero `error` or
/// an `error_name` is a `ZaloException` (code = `error`).
fn zalo_failure(status: u16, body: &Value) -> Result<(), ApiFailure> {
    let code = number(&body["error"]).filter(|c| *c != 0);
    if (200..300).contains(&status) && code.is_none() && !body["error_name"].is_string() {
        return Ok(());
    }
    let message = ["message", "error_description", "error_reason", "error_name"]
        .iter()
        .find_map(|key| body[*key].as_str())
        .unwrap_or("Zalo OA API error");
    Err(ApiFailure {
        message: message.to_owned(),
        status: Some(status),
        code,
        ..ApiFailure::default()
    })
}

/// `POST {ZALO_API}/{endpoint}` with a JSON body (the OA tag calls).
pub async fn zalo_post(
    http: &dyn ChannelHttp,
    access_token: &str,
    endpoint: &str,
    body: &Value,
) -> Result<Value, ApiFailure> {
    let reply = http
        .send(UpstreamRequest {
            method: "POST",
            url: format!("{ZALO_API_URL}/{endpoint}"),
            headers: vec![
                ("access_token".into(), access_token.to_owned()),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: Some(body.to_string()),
        })
        .await;
    let (status, text) = reply.map_err(ApiFailure::new)?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    zalo_failure(status, &body)?;
    Ok(body)
}

/// Zalo `isRevokedTokenError`: code `-124`.
pub fn is_zalo_revoked(failure: &ApiFailure) -> bool {
    failure.code == Some(-124)
}

/// `GET {ZALO_API}/v3.0/oa/user/detail` (`fetchZaloUserDetail`): `data`.
pub async fn zalo_user_detail(
    http: &dyn ChannelHttp,
    access_token: &str,
    user_id: &str,
) -> Result<Value, ApiFailure> {
    let data = json!({ "user_id": user_id }).to_string();
    let url = with_query(
        &format!("{ZALO_API_URL}/v3.0/oa/user/detail"),
        &[("data", &data)],
    );
    let reply = http
        .send(UpstreamRequest {
            method: "GET",
            url,
            headers: vec![("access_token".into(), access_token.to_owned())],
            body: None,
        })
        .await;
    let (status, text) = reply.map_err(ApiFailure::new)?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    zalo_failure(status, &body)?;
    Ok(body["data"].clone())
}

/// `AUTH_FAILED_PATTERNS` of the TikTok error mapper.
const TIKTOK_AUTH_FAILED: &[&str] = &[
    "access_token_invalid",
    "access_token_expired",
    "token_not_valid",
    "unauthorized",
    "invalid_token",
    "token revoked",
    "auth_failed",
];

/// TikTok `isRevokedTokenError`: an auth-failed message or HTTP 401.
pub fn is_tiktok_revoked(failure: &ApiFailure) -> bool {
    let lower = failure.message.to_lowercase();
    failure.status == Some(401) || TIKTOK_AUTH_FAILED.iter().any(|p| lower.contains(p))
}

/// TikTok `refreshAccessToken` (`tt_user/oauth2/refresh_token/`): `data`.
pub async fn refresh_tiktok_token(
    http: &dyn ChannelHttp,
    client_id: &Value,
    client_secret: &Value,
    refresh_token: &str,
) -> Result<Value, ApiFailure> {
    let body = json!({
        "client_id": client_id,
        "client_secret": client_secret,
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
    });
    let reply = http
        .send(UpstreamRequest {
            method: "POST",
            url: format!("{TIKTOK_API_URL}tt_user/oauth2/refresh_token/"),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(body.to_string()),
        })
        .await;
    let (status, text) = reply.map_err(ApiFailure::new)?;
    let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        return Err(ApiFailure {
            message: if status == 401 {
                "Unauthorized".into()
            } else {
                format!("tt_user/oauth2/refresh_token: HTTP {status}")
            },
            status: Some(status),
            ..ApiFailure::default()
        });
    }
    if parsed["code"].as_i64() != Some(0) {
        return Err(ApiFailure {
            message: parsed["message"]
                .as_str()
                .unwrap_or("Token refresh failed")
                .to_owned(),
            status: Some(status),
            ..ApiFailure::default()
        });
    }
    Ok(parsed["data"].clone())
}

/// `parseTiktokScopes`.
pub fn parse_tiktok_scopes(scope: &Value) -> Vec<String> {
    scope
        .as_str()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Telegram `GET bot<token>/<method>?..`: `result`.
pub async fn telegram_call(
    http: &dyn ChannelHttp,
    token: &str,
    method: &str,
    query: &[(&str, &str)],
) -> Result<Value, ApiFailure> {
    let url = with_query(&format!("{TELEGRAM_API_URL}/bot{token}/{method}"), query);
    let (status, text) = http.send(get(url, None)).await.map_err(ApiFailure::new)?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if (200..300).contains(&status) && body["ok"] != json!(false) {
        return Ok(body["result"].clone());
    }
    Err(ApiFailure {
        message: body["description"].as_str().unwrap_or(method).to_owned(),
        status: Some(status),
        code: number(&body["error_code"]),
        ..ApiFailure::default()
    })
}
