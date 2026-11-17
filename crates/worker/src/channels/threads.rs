//! `integrations/threads` (Q.I1b): the Threads comment channel on
//! `graph.threads.com`. `lib/error-sanitizer.ts`, `exception.ts`
//! (`rescue`), `lib/error-mapper.ts`, `apis/comment.ts` (reply container
//! create / poll / publish, `manage_reply`), the comment handlers and the
//! webhook (`handlers/webhook.ts`, `lib/webhook.ts`).

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use chatbotx_core::utils::crypto::hmac_sha256_hex;
use chatbotx_jobs::BoxFuture;
use regex::Regex;
use serde_json::{Value, json};

use super::messenger_webhook::{Outcome, handshake};
use super::whatsapp_webhook::WebhookJob;
use crate::channel::{
    Category, Channel, ChannelCtx, ChannelError, ChannelResult, SendFailure, SendMessage,
    SendResult,
};

/// `API_URL`.
pub const API_URL: &str = "https://graph.threads.com";
/// `DEFAULT_API_VERSION`.
pub const DEFAULT_API_VERSION: &str = "v1.0";
/// `THREADS_REPLY_PUBLISH_TIMEOUT_MS`.
const REPLY_PUBLISH_TIMEOUT: Duration = Duration::from_millis(30_000);
/// `THREADS_REPLY_PUBLISH_POLL_INTERVAL_MS`.
const REPLY_PUBLISH_POLL_INTERVAL: Duration = Duration::from_millis(5_000);
/// Status codes `ky` retries.
const RETRY_STATUS: [u16; 7] = [408, 413, 429, 500, 502, 503, 504];

// ---- error sanitizer ----------------------------------------------------------

const SENSITIVE_PARAM_NAMES: [&str; 4] = [
    "access_token",
    "client_secret",
    "refresh_token",
    "authorization",
];

static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)https?://[^\s"'`]+"#).expect("url regex"));

static PATTERNS: LazyLock<Vec<(Regex, Regex)>> = LazyLock::new(|| {
    SENSITIVE_PARAM_NAMES
        .iter()
        .map(|key| {
            (
                Regex::new(&format!(r"(?i)([?&]{key}=)([^&#\s]+)")).expect("query regex"),
                Regex::new(&format!(
                    r#"(?i)(["']?{key}["']?\s*[:=]\s*["']?)([^"',\s}}]+)(["']?)"#
                ))
                .expect("assignment regex"),
            )
        })
        .collect()
});

/// `sanitizeUrl`: every query value re-set (sensitive ones masked), so the
/// URL is re-serialized; an unparsable URL is kept.
fn sanitize_url(raw: &str) -> String {
    let Ok(mut url) = url::Url::parse(raw) else {
        return raw.to_owned();
    };
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| {
            let masked = if SENSITIVE_PARAM_NAMES.contains(&k.to_lowercase().as_str()) {
                "[REDACTED]".to_owned()
            } else {
                v.into_owned()
            };
            (k.into_owned(), masked)
        })
        .collect();
    if !pairs.is_empty() {
        url.query_pairs_mut().clear().extend_pairs(&pairs);
    }
    url.to_string()
}

/// `sanitizeSensitiveText`.
pub fn sanitize_sensitive_text(value: &str) -> String {
    let mut sanitized = URL_REGEX
        .replace_all(value, |c: &regex::Captures<'_>| sanitize_url(&c[0]))
        .into_owned();
    for (query, assignment) in PATTERNS.iter() {
        sanitized = query.replace_all(&sanitized, "${1}[REDACTED]").into_owned();
        sanitized = assignment
            .replace_all(&sanitized, "${1}[REDACTED]${3}")
            .into_owned();
    }
    sanitized
}

// ---- exception / error mapper -----------------------------------------------------

/// `ThreadsException`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThreadsError {
    pub message: String,
    /// `code` as sent (a number or a string).
    pub code: Option<Value>,
    pub sub_code: Option<Value>,
    pub kind: Option<String>,
    pub http_status: Option<u16>,
    /// `originError instanceof HTTPError`.
    pub http_origin: bool,
}

impl ThreadsError {
    pub fn new(message: impl Into<String>) -> ThreadsError {
        ThreadsError {
            message: message.into(),
            ..ThreadsError::default()
        }
    }
}

/// `rescue`: an HTTP error with Meta's `{ error }` body (or the bare `ky`
/// message), or a transport failure, wrapped with the endpoint.
fn rescued(endpoint: &str, status: Option<u16>, body: &Value, fallback: &str) -> ThreadsError {
    let api = body.get("error").filter(|e| e.is_object());
    let field = |k: &str| api.and_then(|e| e.get(k)).filter(|v| !v.is_null());
    let message = field("message").and_then(Value::as_str).unwrap_or(fallback);
    ThreadsError {
        message: format!(
            "Threads API error at {endpoint}: {}",
            sanitize_sensitive_text(message)
        ),
        code: field("code").cloned(),
        sub_code: field("error_subcode").cloned(),
        kind: field("type").and_then(Value::as_str).map(str::to_owned),
        http_status: status,
        http_origin: status.is_some(),
    }
}

/// `categorize`.
pub fn categorize(error: &ThreadsError) -> Category {
    if let Some(code) = error.code.as_ref().and_then(Value::as_i64) {
        match code {
            102 | 190 | 458 | 459 | 460 | 463 | 464 | 467 => return Category::AuthFailed,
            4 | 17 | 32 | 613 => return Category::RateLimited,
            2_207_042 => return Category::QuotaExceeded,
            3 | 10 | 200..=299 | 341 | 368 => return Category::PermissionDenied,
            1 | 2 | 5 => return Category::NetworkError,
            33 | 100 | 506 => return Category::PayloadInvalid,
            _ => {}
        }
    }
    if error.kind.as_deref() == Some("OAuthException") || error.http_status == Some(401) {
        return Category::AuthFailed;
    }
    if error.http_status == Some(429) {
        return Category::RateLimited;
    }
    if error.http_status.is_some_and(|s| s >= 500) {
        return Category::NetworkError;
    }
    if error.message.contains("did not finish within")
        || error.message.contains("network")
        || error.http_origin
    {
        return Category::NetworkError;
    }
    Category::Unknown
}

fn default_http_status(category: Category) -> u16 {
    match category {
        Category::AuthFailed => 401,
        Category::PermissionDenied => 403,
        Category::RateLimited | Category::QuotaExceeded => 429,
        Category::NetworkError => 503,
        _ => 400,
    }
}

/// `mapToChannelError` of a `ThreadsException`.
pub fn map_error(error: &ThreadsError) -> ChannelError {
    let category = categorize(error);
    let mut mapped = ChannelError::new(error.message.clone(), category);
    mapped.code = Some(match &error.code {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => "channel_error".into(),
    });
    mapped.http_status_code = error
        .http_status
        .unwrap_or_else(|| default_http_status(category));
    mapped
}

/// `isRevokedTokenError`.
pub fn is_revoked_token_error(error: &ThreadsError) -> bool {
    let sub_code = match &error.sub_code {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    categorize(error) == Category::AuthFailed
        && error.code.as_ref().and_then(Value::as_i64) == Some(190)
        && sub_code.is_some_and(|s| [458.0, 460.0, 463.0, 467.0].contains(&s))
}

impl From<ThreadsError> for SendFailure {
    fn from(error: ThreadsError) -> SendFailure {
        map_error(&error).into()
    }
}

// ---- Graph calls --------------------------------------------------------------

/// The `ThreadsAuthValue` fields the handlers read.
#[derive(Debug, Clone, PartialEq)]
pub struct Auth {
    pub access_token: String,
    pub threads_user_id: String,
    /// `auth.metadata.version ?? DEFAULT_API_VERSION`.
    pub version: String,
}

pub fn auth_of(auth: &Value) -> ChannelResult<Auth> {
    let text = |p: &str| auth.pointer(p).and_then(Value::as_str);
    let (Some(token), Some(user)) = (text("/tokens/accessToken"), text("/metadata/threadsUserId"))
    else {
        return Err(ChannelError::new(
            "Threads integration auth is incomplete",
            Category::AuthFailed,
        )
        .with_code("integration_auth_missing")
        .into());
    };
    Ok(Auth {
        access_token: token.to_owned(),
        threads_user_id: user.to_owned(),
        version: text("/metadata/version")
            .unwrap_or(DEFAULT_API_VERSION)
            .to_owned(),
    })
}

/// `threadsGraphClient`: a 30 s `ky` instance; `ky` retries GET twice and
/// never retries POST.
#[derive(Clone)]
pub struct ThreadsHttp {
    client: reqwest::Client,
    base_url: String,
}

impl ThreadsHttp {
    pub fn new(base_url: &str) -> ThreadsHttp {
        ThreadsHttp {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    async fn once(
        &self,
        method: &reqwest::Method,
        endpoint: &str,
        params: &[(&str, &str)],
    ) -> Result<Value, ThreadsError> {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish();
        let url = format!("{}/{endpoint}", self.base_url);
        let builder = if *method == reqwest::Method::GET {
            self.client.get(format!("{url}?{encoded}"))
        } else {
            self.client
                .request(method.clone(), url.clone())
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded;charset=UTF-8",
                )
                .body(encoded)
        };
        let response = builder
            .send()
            .await
            .map_err(|e| rescued(endpoint, None, &Value::Null, &e.without_url().to_string()))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(parsed);
        }
        let fallback = format!(
            "Request failed with status code {} {}: {method} {url}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        );
        Err(rescued(endpoint, Some(status.as_u16()), &parsed, &fallback))
    }

    async fn call(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        params: &[(&str, &str)],
    ) -> Result<Value, ThreadsError> {
        let retries = if method == reqwest::Method::GET { 2 } else { 0 };
        let mut attempt = 0;
        loop {
            match self.once(&method, endpoint, params).await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    let retryable = error.http_status.is_none_or(|s| RETRY_STATUS.contains(&s));
                    if attempt >= retries || !retryable {
                        return Err(error);
                    }
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(300 * (1 << (attempt - 1)))).await;
                }
            }
        }
    }
}

/// `normalizeReplyStatus`.
fn normalize_reply_status(status: Option<&str>) -> String {
    status.map_or_else(|| "UNKNOWN".to_owned(), |s| s.trim().to_uppercase())
}

/// `waitForReplyContainerReady`: polls until `FINISHED` / `PUBLISHED`;
/// only `ERROR` / `EXPIRED` abort early, the deadline is the one stop.
pub async fn wait_for_ready<F, Fut>(
    mut status_of: F,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), ThreadsError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Value, ThreadsError>>,
{
    let deadline = Instant::now() + timeout;
    loop {
        let response = status_of().await?;
        let status = normalize_reply_status(response.get("status").and_then(Value::as_str));
        if status == "FINISHED" || status == "PUBLISHED" {
            return Ok(());
        }
        if status == "ERROR" || status == "EXPIRED" {
            return Err(ThreadsError::new(format!(
                "Threads reply creation {}: {}",
                status.to_lowercase(),
                response
                    .get("error_message")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown error")
            )));
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(ThreadsError::new(format!(
                "Threads reply creation did not finish within {}ms. Retry later.",
                timeout.as_millis()
            )));
        }
        tokio::time::sleep(poll_interval.min(deadline - now)).await;
    }
}

pub struct Threads {
    http: ThreadsHttp,
    poll_timeout: Duration,
    poll_interval: Duration,
}

impl Threads {
    pub fn new() -> Threads {
        Threads::with_base_url(API_URL)
    }

    pub fn with_base_url(base_url: &str) -> Threads {
        Threads {
            http: ThreadsHttp::new(base_url),
            poll_timeout: REPLY_PUBLISH_TIMEOUT,
            poll_interval: REPLY_PUBLISH_POLL_INTERVAL,
        }
    }
}

impl Default for Threads {
    fn default() -> Threads {
        Threads::new()
    }
}

/// `replyToComment`: create the reply container, wait until it is ready,
/// publish it.
async fn reply_to_comment(
    http: &ThreadsHttp,
    auth: &Auth,
    reply_to: &str,
    text: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<Value, ThreadsError> {
    let token = auth.access_token.as_str();
    let created = http
        .call(
            reqwest::Method::POST,
            &format!("{}/{}/threads", auth.version, auth.threads_user_id),
            &[
                ("media_type", "TEXT"),
                ("text", text),
                ("reply_to_id", reply_to),
                ("access_token", token),
            ],
        )
        .await?;
    let container = created
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let status_endpoint = format!("{}/{container}", auth.version);
    let status_endpoint = status_endpoint.as_str();
    wait_for_ready(
        move || async move {
            http.call(
                reqwest::Method::GET,
                status_endpoint,
                &[("fields", "status,error_message"), ("access_token", token)],
            )
            .await
        },
        timeout,
        poll_interval,
    )
    .await?;
    http.call(
        reqwest::Method::POST,
        &format!("{}/{}/threads_publish", auth.version, auth.threads_user_id),
        &[("creation_id", &container), ("access_token", token)],
    )
    .await
}

impl Channel for Threads {
    fn send_comment(
        &self,
        ctx: &ChannelCtx,
        data: SendMessage,
    ) -> BoxFuture<'static, ChannelResult<SendResult>> {
        let auth = auth_of(&ctx.auth);
        let http = self.http.clone();
        let (timeout, interval) = (self.poll_timeout, self.poll_interval);
        Box::pin(async move {
            let Some(reply_to) = data
                .message
                .pointer("/contentAttributes/replyToCommentId")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
            else {
                return Err(ChannelError::new(
                    "Cannot send comment reply: replyToCommentId is missing. The outgoing message must be linked to a parent comment.",
                    Category::PayloadInvalid,
                )
                .into());
            };
            let Some(text) = data
                .message
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
            else {
                return Err(ChannelError::new(
                    "Cannot send comment reply: text is required.",
                    Category::PayloadInvalid,
                )
                .into());
            };
            let auth = auth?;
            let published = reply_to_comment(&http, &auth, reply_to, text, timeout, interval)
                .await
                .inspect_err(|e| {
                    let mapped = map_error(e);
                    tracing::error!(reply_to, category = ?mapped.category, code = ?e.code,
                        status = e.http_status, sub_code = ?e.sub_code, kind = ?e.kind,
                        "Failed to send Threads comment reply");
                })?;
            Ok(SendResult {
                message_ids: published
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .into_iter()
                    .collect(),
                sent_count: 1,
            })
        })
    }

    fn hide_comment(
        &self,
        ctx: &ChannelCtx,
        comment_id: String,
        hidden: bool,
        _post_id: Option<String>,
    ) -> BoxFuture<'static, ChannelResult<()>> {
        let auth = auth_of(&ctx.auth);
        let http = self.http.clone();
        Box::pin(async move {
            let auth = auth?;
            http.call(
                reqwest::Method::POST,
                &format!("{}/{comment_id}/manage_reply", auth.version),
                &[
                    ("hide", if hidden { "true" } else { "false" }),
                    ("access_token", &auth.access_token),
                ],
            )
            .await
            .inspect_err(|e| {
                tracing::error!(comment_id, hidden, category = ?categorize(e), code = ?e.code,
                    status = e.http_status, "Failed to change Threads reply visibility");
            })?;
            Ok(())
        })
    }
}

// ---- webhook ------------------------------------------------------------------

/// `lib/webhook.ts` `timingSafeStringEqual`: compares the decoded hex
/// bytes (upper-case matches, non-hex fails).
fn hex_equal(left: &str, right: &str) -> bool {
    let decode = |hex: &str| -> Option<Vec<u8>> {
        if hex.len() % 2 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
            .collect()
    };
    let (Some(a), Some(b)) = (decode(left), decode(right)) else {
        return false;
    };
    a.len() == b.len() && a.iter().zip(&b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `verifyWebhookSignature`: exactly `sha256=<hex>`.
pub fn verify_signature(payload: &str, signature: &str, client_secret: &str) -> bool {
    let parts: Vec<&str> = signature.split('=').collect();
    let ["sha256", hash] = parts.as_slice() else {
        return false;
    };
    hex_equal(hash, &hmac_sha256_hex(client_secret, payload))
}

fn is_text(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

fn optional_text(v: Option<&Value>) -> bool {
    v.is_none_or(Value::is_string)
}

/// `threadsWebhookEntrySchema`.
fn is_valid_entry(entry: &Value) -> bool {
    let value = entry.get("value");
    let at = |p: &str| value.and_then(|v| v.pointer(p));
    is_text(entry.get("field"))
        && value.is_some_and(Value::is_object)
        && is_text(at("/id"))
        && is_text(at("/username"))
        && optional_text(at("/text"))
        && at("/replied_to").is_none_or(|r| is_text(r.get("id")))
        && [
            "/root_post/id",
            "/root_post/owner_id",
            "/root_post/username",
        ]
        .iter()
        .all(|p| is_text(at(p)))
        && optional_text(at("/timestamp"))
        && optional_text(at("/profile_picture_url"))
}

/// `threadsWebhookPayloadSchema`: the entries (`values` as an array or one
/// object), or `None` on a mismatch.
fn parse_payload(payload: &Value) -> Option<(f64, Vec<&Value>)> {
    let time = payload.get("time").and_then(Value::as_f64)?;
    let valid = ["app_id", "topic", "target_id", "subscription_id"]
        .iter()
        .all(|k| is_text(payload.get(*k)))
        && time >= 0.0
        && time.fract() == 0.0;
    let values: Vec<&Value> = match payload.get("values")? {
        Value::Array(values) => values.iter().collect(),
        single => vec![single],
    };
    (valid && values.iter().all(|v| is_valid_entry(v))).then_some((time, values))
}

/// `toEpochSeconds`.
fn to_epoch_seconds(timestamp: Option<&str>, fallback: f64) -> Value {
    match timestamp
        .filter(|t| !t.is_empty())
        .and_then(chatbotx_core::js::parse_date)
    {
        Some(ms) => json!(ms.div_euclid(1000)),
        // `parse_payload` accepts only a whole, non-negative `time`.
        None => json!(fallback as i64),
    }
}

/// `handleWebhookEvent` after the signature check.
pub fn webhook_jobs(body: &str, client_id: Option<&str>) -> Result<Vec<WebhookJob>, String> {
    let Ok(parsed) = serde_json::from_str::<Value>(body) else {
        tracing::warn!(
            reason = "invalid_json",
            raw_body = body,
            "threads webhook payload unrecognized — skipping"
        );
        return Ok(Vec::new());
    };
    let Some((time, values)) = parse_payload(&parsed) else {
        tracing::warn!(
            reason = "schema_mismatch",
            raw_body = body,
            "threads webhook payload unrecognized — skipping"
        );
        return Ok(Vec::new());
    };
    if parsed.get("app_id").and_then(Value::as_str) != client_id {
        return Err("Webhook app_id does not match configured clientId".into());
    }
    if parsed.get("topic").and_then(Value::as_str) != Some("moderate") {
        return Ok(Vec::new());
    }
    Ok(values
        .into_iter()
        .filter(|entry| {
            let at = |p: &str| entry.pointer(p).and_then(Value::as_str).unwrap_or("");
            at("/field") == "replies"
                && at("/value/username").to_lowercase()
                    != at("/value/root_post/username").to_lowercase()
        })
        .map(|entry| {
            let value = &entry["value"];
            let username = value["username"].as_str().unwrap_or("");
            let comment = super::meta::compact(json!({
                "commentId": value["id"],
                "postId": value["root_post"]["id"],
                "parentId": value.pointer("/replied_to/id"),
                "fromId": username.to_lowercase(),
                "fromName": username,
                "fromAvatarUrl": value.get("profile_picture_url"),
                "message": value.get("text"),
                "createdTime": to_epoch_seconds(value.get("timestamp").and_then(Value::as_str), time),
            }));
            WebhookJob {
                name: "incomingComment",
                data: json!({ "type": "incomingComment", "data": {
                    "integrationType": "threads",
                    "integrationIdentifier": value["root_post"]["owner_id"],
                    "commentData": comment,
                } }),
                job_id: String::new(),
            }
        })
        .collect())
}

/// `webhookHandler`: the handshake, or the verified body's jobs; `Err` is
/// the `ThreadsException` text.
pub fn handle_request(
    is_get: bool,
    query: Option<&str>,
    signature: Option<&str>,
    body: &[u8],
    credential: (Option<&str>, Option<&str>, &str),
) -> Result<Outcome, String> {
    let (verify_token, client_id, client_secret) = credential;
    if is_get {
        return handshake(query, verify_token)
            .map(Outcome::Challenge)
            .map_err(str::to_owned);
    }
    let body = String::from_utf8_lossy(body);
    if body.is_empty() {
        return Err("Empty webhook payload".into());
    }
    let signature = signature.unwrap_or("");
    if signature.is_empty() {
        return Err("Missing webhook signature".into());
    }
    if !verify_signature(&body, signature, client_secret) {
        return Err("Invalid webhook signature".into());
    }
    webhook_jobs(&body, client_id).map(Outcome::Jobs)
}

/// `THREADS_BAD_REQUEST_MESSAGES`.
const BAD_REQUEST_MESSAGES: [&str; 5] = [
    "Empty webhook payload",
    "Invalid webhook signature",
    "Invalid webhook verification parameters",
    "Missing webhook signature",
    "Webhook app_id does not match configured clientId",
];

/// `createThreadsErrorResponse`: the status and the public message.
pub fn error_response(message: &str) -> (u16, &'static str) {
    let status = if message.starts_with("Unsupported HTTP method:") {
        405
    } else if BAD_REQUEST_MESSAGES.contains(&message) {
        400
    } else {
        500
    };
    let public = if status < 500 {
        "Invalid Threads webhook request"
    } else {
        "Failed to process Threads webhook"
    };
    (status, public)
}

#[cfg(test)]
#[path = "threads_tests.rs"]
mod tests;
