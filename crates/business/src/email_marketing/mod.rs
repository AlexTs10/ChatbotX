//! Q.I3: the e-mail marketing integrations' provider actions
//! (`integrations/<name>/src/{client,integration}.ts`) and the input
//! builders of their flow steps (`apps/worker/.../<name>-handler.ts`).
//!
//! The worker runs the steps (`chatbotx_worker::steps::email_marketing`),
//! the API the builder's list procedures (`integration<Name>API.*`). Both
//! read the satellite auth with [`load_auth`] and call the provider through
//! an [`Http`] (production: [`SafeHttp`], SSRF-checked on every hop).

pub mod active_campaign;
pub mod drip;
pub mod get_response;
pub mod klaviyo;
pub mod mailchimp;
pub mod mailer_lite;
pub mod moosend;
pub mod sendgrid;
pub mod smtp;

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use chatbotx_core::encryption::EncryptedData;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_kv::{LockMap, LockOpts};
use futures::future::BoxFuture;
use serde_json::Value;

use crate::connection::api_key::api_key_provider;
use crate::meta_catalog::process_encryption;
use crate::net::ssrf_guard::assert_public_url;
use crate::{BusinessError, tx};

/// `getContactFieldMap`: system field key or custom field id → value.
pub type Fields = HashMap<String, String>;

/// One provider request (`ky` call). `body` is sent as JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<Value>,
}

impl Request {
    pub fn new(method: &'static str, url: impl Into<String>) -> Request {
        Request {
            method,
            url: url.into(),
            headers: vec![("Accept", "application/json".into())],
            body: None,
        }
    }

    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Request {
        self.headers.push((name, value.into()));
        self
    }

    pub fn json(mut self, body: Value) -> Request {
        self.body = Some(body);
        self
    }
}

/// A provider answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reply {
    pub status: u16,
    /// Lower-case header names.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// `response.status === 204 ? undefined : response.json().catch(() =>
    /// undefined)`.
    pub fn payload(&self) -> Option<Value> {
        if self.status == 204 {
            return None;
        }
        serde_json::from_str(&self.body).ok()
    }
}

/// The provider transport. `Err` = no answer (network, timeout, SSRF
/// refusal).
pub trait Http: Send + Sync {
    fn send(&self, request: Request) -> BoxFuture<'_, Result<Reply, String>>;
}

const MAX_REDIRECT_HOPS: usize = 5;

/// [`Http`] on reqwest: 15 s timeout (`*_HTTP_TIMEOUT_MS`), no retries,
/// redirects followed by hand with `assertPublicUrl` before every hop; the
/// headers (credentials) go only to the first URL's origin.
pub struct SafeHttp {
    client: reqwest::Client,
}

impl Default for SafeHttp {
    fn default() -> SafeHttp {
        SafeHttp {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        }
    }
}

fn origin(url: &str) -> Option<url::Origin> {
    url::Url::parse(url).ok().map(|url| url.origin())
}

impl Http for SafeHttp {
    fn send(&self, request: Request) -> BoxFuture<'_, Result<Reply, String>> {
        Box::pin(async move {
            let first_origin = origin(&request.url);
            let mut url = request.url.clone();
            let mut method = request.method;
            let mut body = request.body.clone();
            for _ in 0..=MAX_REDIRECT_HOPS {
                assert_public_url(&url, "Integration URL").await?;
                let method_value =
                    reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
                let mut builder = self.client.request(method_value, &url);
                if origin(&url) == first_origin {
                    for (name, value) in &request.headers {
                        builder = builder.header(*name, value);
                    }
                }
                if let Some(body) = &body {
                    builder = builder
                        .header("Content-Type", "application/json")
                        .body(body.to_string());
                }
                let response = builder.send().await.map_err(|e| e.to_string())?;
                let status = response.status().as_u16();
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|l| l.to_str().ok())
                    .map(str::to_owned);
                if let (true, Some(location)) = ((300..400).contains(&status), location) {
                    url = url::Url::parse(&url)
                        .and_then(|base| base.join(&location))
                        .map_err(|e| e.to_string())?
                        .to_string();
                    if matches!(status, 301..=303) {
                        method = "GET";
                        body = None;
                    }
                    continue;
                }
                let headers = response
                    .headers()
                    .iter()
                    .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
                    .collect();
                let body = response.text().await.unwrap_or_default();
                return Ok(Reply {
                    status,
                    headers,
                    body,
                });
            }
            Err("Too many redirects".into())
        })
    }
}

/// Sends `request`; a transport failure is a [`ProviderError`].
pub(crate) async fn call(http: &dyn Http, request: Request) -> Outcome<Reply> {
    http.send(request).await.map_err(ProviderError::new)
}

/// Sends `request` and parses the JSON answer; a non-2xx answer is an
/// error with `message(payload, status)`.
pub(crate) async fn json_call(
    http: &dyn Http,
    request: Request,
    message: impl FnOnce(Option<&Value>, u16) -> String,
) -> Outcome<Option<Value>> {
    let reply = call(http, request).await?;
    let payload = reply.payload();
    if !reply.ok() {
        return Err(ProviderError::http(
            message(payload.as_ref(), reply.status),
            reply.status,
            &reply,
        ));
    }
    Ok(payload)
}

/// `payload[key]` as an array (`.default([])`); not an object = invalid.
pub(crate) fn items(payload: Option<Value>, key: &str, provider: &str) -> Outcome<Vec<Value>> {
    match payload {
        Some(Value::Object(mut map)) => match map.remove(key) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(items)) => Ok(items),
            Some(_) => Err(invalid_response(provider)),
        },
        _ => Err(invalid_response(provider)),
    }
}

/// `payload.<path>` as a string.
pub(crate) fn text_at<'a>(payload: Option<&'a Value>, path: &[&str]) -> Option<&'a str> {
    let mut value = payload?;
    for key in path {
        value = match key.parse::<usize>() {
            Ok(index) => value.get(index)?,
            Err(_) => value.get(*key)?,
        };
    }
    value.as_str().filter(|s| !s.is_empty())
}

/// The process-wide [`SafeHttp`].
pub fn safe_http() -> &'static SafeHttp {
    static HTTP: OnceLock<SafeHttp> = OnceLock::new();
    HTTP.get_or_init(SafeHttp::default)
}

/// A provider failure (`<Name>ApiError`, a zod parse error, a transport
/// error): the message the step logs and returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub message: String,
    /// The HTTP status (`statusCode`).
    pub status: Option<u16>,
    /// Moosend's `kind`.
    pub kind: Option<&'static str>,
    pub retry_after_seconds: Option<u64>,
}

impl ProviderError {
    pub fn new(message: impl Into<String>) -> ProviderError {
        ProviderError {
            message: message.into(),
            status: None,
            kind: None,
            retry_after_seconds: None,
        }
    }

    /// A step's own `{ status: "error" }` return (no provider error log).
    pub fn local(message: impl Into<String>) -> ProviderError {
        ProviderError {
            kind: Some(LOCAL),
            ..ProviderError::new(message)
        }
    }

    /// `false` for [`ProviderError::local`]: the TS step returned without
    /// `logStepProviderError`.
    pub fn logged(&self) -> bool {
        self.kind != Some(LOCAL)
    }

    pub fn http(message: impl Into<String>, status: u16, reply: &Reply) -> ProviderError {
        ProviderError {
            status: Some(status),
            retry_after_seconds: retry_after(reply.header("retry-after")),
            ..ProviderError::new(message)
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<BusinessError> for ProviderError {
    fn from(error: BusinessError) -> ProviderError {
        ProviderError::new(error.message())
    }
}

pub type Outcome<T> = Result<T, ProviderError>;

const LOCAL: &str = "local";

/// A zod parse failure of a provider answer.
pub(crate) fn invalid_response(provider: &str) -> ProviderError {
    ProviderError::new(format!("{provider} returned an invalid response"))
}

/// `parseRetryAfter`: integer seconds, or an HTTP date from now.
pub(crate) fn retry_after(value: Option<&str>) -> Option<u64> {
    let value = value?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds);
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let millis = date.timestamp_millis() - chrono::Utc::now().timestamp_millis();
    Some(u64::try_from(millis.max(0)).unwrap_or(0).div_ceil(1000))
}

/// `[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}` → `[redacted]` (provider messages).
pub(crate) fn redact_emails(text: &str) -> String {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            regex::Regex::new(r"[A-Za-z0-9_.+-]+@[A-Za-z0-9_.-]+\.[A-Za-z]{2,}").expect("pattern")
        })
        .replace_all(text, "[redacted]")
        .into_owned()
}

pub(crate) fn sha256(value: &str) -> String {
    chatbotx_core::utils::crypto::sha256_hex(value)
}

/// `value?.trim() || undefined`.
pub fn non_empty(value: Option<&String>) -> Option<String> {
    value
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

pub(crate) fn field(fields: &Fields, key: Option<&str>) -> Option<String> {
    non_empty(fields.get(key?))
}

pub(crate) fn step_str<'a>(step: &'a Value, key: &str) -> Option<&'a str> {
    step.get(key).and_then(Value::as_str)
}

pub(crate) fn step_strings(step: &Value, key: &str) -> Vec<String> {
    step.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// `step[key]` mappings as `(contactFieldId, provider key)` pairs.
pub(crate) fn mappings<'a>(step: &'a Value, from: &str, to: &str) -> Vec<(&'a str, &'a str)> {
    step.get("mergeFields")
        .or_else(|| step.get("fieldValues"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|m| Some((m.get(from)?.as_str()?, m.get(to)?.as_str()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// `fullName.trim().split(/\s+/)`: the first word and the rest.
pub(crate) fn split_full_name(full_name: &str) -> (String, String) {
    let mut words = full_name.split_whitespace();
    let first = words.next().unwrap_or_default().to_owned();
    (first, words.collect::<Vec<_>>().join(" "))
}

/// `z.string().email()` (zod's pattern, ASCII).
pub fn is_email(value: &str) -> bool {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            regex::Regex::new(
                r"^[A-Za-z0-9_'+\-\.]*[A-Za-z0-9_+\-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$",
            )
            .expect("pattern")
        })
        .is_match(value)
        && !value.starts_with('.')
        && !value.contains("..")
}

// ---- auth ------------------------------------------------------------------

/// `integration<Name>Service.findByWorkspaceIdOrFail` + `decryptObject`:
/// the workspace's satellite auth. A plain JSON auth (a row written before
/// the API-key connect encrypted it) is read as is.
pub async fn load_auth(db: &Db, provider: &str, workspace_id: i64) -> Outcome<Value> {
    let target = api_key_provider(provider)
        .ok_or_else(|| ProviderError::new(format!("Unknown integration: {provider}")))?;
    let table = target.table;
    let raw: Option<String> = tx::read(db, move |conn| {
        conn.query_row(
            &format!(r#"SELECT "auth" FROM "{table}" WHERE "workspaceId" = ?1"#),
            params![workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await?;
    let raw = raw.ok_or_else(|| {
        ProviderError::new(format!("{} integration not found", target.display_name))
    })?;
    open_auth(&raw).ok_or_else(|| ProviderError::new("Failed to decrypt data"))
}

/// The stored auth text as JSON (decrypted when it is an `EncryptedData`).
pub fn open_auth(raw: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(raw).ok()?;
    match serde_json::from_value::<EncryptedData>(value.clone()) {
        Ok(data) => process_encryption().decrypt_object(&data, None).ok(),
        Err(_) => Some(value),
    }
}

/// `encryptUtils.encryptObject(auth)` as the stored text.
pub fn seal_auth(auth: &Value) -> crate::Result<String> {
    let sealed = process_encryption()
        .encrypt_object(auth, None)
        .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?;
    serde_json::to_string(&sealed)
        .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))
}

pub(crate) fn auth_str<'a>(auth: &'a Value, key: &str) -> &'a str {
    auth.get(key).and_then(Value::as_str).unwrap_or_default()
}

// ---- contact fields --------------------------------------------------------

/// `getContactFieldMap`.
pub async fn contact_field_map(db: &Db, workspace_id: i64, contact_id: i64) -> Outcome<Fields> {
    let fields = tx::read(db, move |conn| {
        let repo = |e: chatbotx_db::rusqlite::Error| BusinessError::Repo(e.into());
        let row = conn
            .query_row(
                r#"SELECT "firstName", "lastName", "fullName", "email", "phoneNumber", "avatar",
                       "locale", "gender", "timezone"
                   FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![contact_id, workspace_id],
                |r| {
                    (0..9)
                        .map(|i| r.get::<_, Option<String>>(i))
                        .collect::<Result<Vec<_>, _>>()
                },
            )
            .optional()
            .map_err(repo)?
            .ok_or_else(|| BusinessError::not_found("Contact not found"))?;
        let keys = [
            "first_name",
            "last_name",
            "full_name",
            "email",
            "phone",
            "avatar",
            "locale",
            "gender",
            "timezone",
        ];
        let mut fields: Fields = keys
            .iter()
            .zip(row)
            .map(|(k, v)| ((*k).to_owned(), v.unwrap_or_default()))
            .collect();
        fields.insert("user_id".into(), contact_id.to_string());
        fields.insert("workspace_id".into(), workspace_id.to_string());
        let mut statement = conn
            .prepare(r#"SELECT "customFieldId", "value" FROM "ContactCustomField" WHERE "contactId" = ?1"#)
            .map_err(repo)?;
        let custom = statement
            .query_map(params![contact_id], |r| {
                Ok((r.get::<_, i64>(0)?.to_string(), r.get::<_, String>(1)?))
            })
            .map_err(repo)?;
        for entry in custom {
            let (key, value) = entry.map_err(repo)?;
            fields.insert(key, value);
        }
        Ok(fields)
    })
    .await?;
    Ok(fields)
}

// ---- lock ------------------------------------------------------------------

/// `distributedLock.runExclusive({ key, timeoutInSeconds, fn })` on the
/// process lock map (one process; no Redis).
pub async fn run_exclusive<T>(
    key: &str,
    ttl_secs: u64,
    f: impl AsyncFnOnce() -> Outcome<T>,
) -> Outcome<T> {
    static LOCKS: OnceLock<LockMap> = OnceLock::new();
    let _guard = LOCKS
        .get_or_init(LockMap::new)
        .acquire(
            key,
            LockOpts {
                ttl_secs,
                retry_secs: None,
            },
        )
        .await
        .map_err(|e| ProviderError::new(e.to_string()))?;
    f().await
}

#[cfg(test)]
pub(crate) mod test_http {
    //! A recording fake [`Http`].
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct FakeHttp {
        pub replies: Mutex<Vec<Reply>>,
        pub requests: Mutex<Vec<Request>>,
    }

    impl FakeHttp {
        pub fn with(replies: Vec<(u16, &str)>) -> FakeHttp {
            FakeHttp {
                replies: Mutex::new(
                    replies
                        .into_iter()
                        .rev()
                        .map(|(status, body)| Reply {
                            status,
                            headers: Vec::new(),
                            body: body.to_owned(),
                        })
                        .collect(),
                ),
                requests: Mutex::default(),
            }
        }

        pub fn requests(&self) -> Vec<Request> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Http for FakeHttp {
        fn send(&self, request: Request) -> BoxFuture<'_, Result<Reply, String>> {
            self.requests.lock().unwrap().push(request);
            let reply = self.replies.lock().unwrap().pop();
            Box::pin(async move { reply.ok_or_else(|| "no reply".to_owned()) })
        }
    }

    pub fn fields(pairs: &[(&str, &str)]) -> Fields {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_and_parses_helpers() {
        assert_eq!(
            redact_emails("bad a.b+c@x-y.example.com here"),
            "bad [redacted] here"
        );
        assert_eq!(retry_after(Some("7")), Some(7));
        assert_eq!(retry_after(Some("Thu, 01 Jan 1970 00:00:00 GMT")), Some(0));
        assert_eq!(retry_after(Some("soon")), None);
        assert_eq!(
            split_full_name("  Ada  King Lovelace "),
            ("Ada".into(), "King Lovelace".into())
        );
        assert!(is_email("ada@example.com"));
        assert!(!is_email("ada@example"));
        assert!(!is_email("not an email"));
    }

    #[test]
    fn auth_round_trips_and_reads_plain_rows() {
        let auth = serde_json::json!({ "authType": "custom", "apiKey": "k" });
        let sealed = seal_auth(&auth).expect("seal");
        assert!(!sealed.contains("\"k\""));
        assert_eq!(open_auth(&sealed), Some(auth.clone()));
        assert_eq!(open_auth(&auth.to_string()), Some(auth));
    }
}
