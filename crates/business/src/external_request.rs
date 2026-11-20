//! `packages/business/src/external-request/service.ts`: the HTTP request of
//! the `callApi` flow step and of `testExternalRequestAction`. Every hop is
//! checked with the SSRF guard first; redirects are followed by hand (at most
//! 5) with the Fetch redirect rules (`nextHopInit`).

use std::time::{Duration, Instant};

use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::rusqlite::params;
pub use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::contact_custom_field::{self, SetValuesInput};
use crate::errors::{BusinessError, Result};
use crate::net::ssrf_guard::{DohFetch, HttpDoh, SsrfCheck, check_ssrf_safety_with};
use crate::tx;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REDIRECTS: usize = 5;
const REQUEST_BODY_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-encoding",
    "content-language",
    "content-location",
];

/// One hop of the request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HopRequest {
    pub method: String,
    pub url: String,
    /// `Headers` order of insertion; names compare case-insensitively.
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

impl HopRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `Headers.set`.
    fn set(&mut self, name: &str, value: &str) {
        self.remove(name);
        self.headers.push((name.to_owned(), value.to_owned()));
    }

    fn remove(&mut self, name: &str) {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }
}

/// A hop's answer (lower-case header names).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HopReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// One HTTP exchange without redirects. `Err` = no answer.
pub trait Hop: Send + Sync {
    fn send(&self, request: &HopRequest) -> BoxFuture<'_, Result<HopReply, String>>;
}

/// [`Hop`] on reqwest: no redirects, 15 s (`REQUEST_TIMEOUT_MS`).
pub struct ReqwestHop(reqwest::Client);

impl Default for ReqwestHop {
    fn default() -> Self {
        ReqwestHop(
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
        )
    }
}

impl Hop for ReqwestHop {
    fn send(&self, request: &HopRequest) -> BoxFuture<'_, Result<HopReply, String>> {
        let request = request.clone();
        Box::pin(async move {
            let method = reqwest::Method::from_bytes(request.method.as_bytes())
                .map_err(|e| e.to_string())?;
            let mut builder = self.0.request(method, &request.url);
            for (name, value) in &request.headers {
                builder = builder.header(name, value);
            }
            if let Some(body) = request.body {
                builder = builder.body(body);
            }
            let response = builder.send().await.map_err(|e| e.to_string())?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
                .collect();
            // A redirect body is dropped unread (`response.body.cancel()`).
            let body = if (300..400).contains(&status) {
                String::new()
            } else {
                response.text().await.map_err(|e| e.to_string())?
            };
            Ok(HopReply {
                status,
                headers,
                body,
            })
        })
    }
}

/// `ExternalRequestResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalRequestResult {
    pub status_code: u16,
    pub duration_ms: u64,
    pub response_body: String,
    pub response_headers: Vec<(String, String)>,
}

fn ssrf_blocked(message: &str) -> BusinessError {
    BusinessError::exception(message, "ssrfBlocked", 400)
}

fn not_allowed() -> BusinessError {
    ssrf_blocked("This URL is not allowed for external requests")
}

/// A transport failure: no code, so `testExternalRequestAction` answers its
/// generic `externalRequestFailed`.
fn transport(message: String) -> BusinessError {
    BusinessError::Repo(repo::Error::msg(message))
}

fn pairs(value: &Value) -> Vec<(String, String)> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let text =
                        |k: &str| item.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
                    (text("key"), text("value"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `buildRequest`. `input` is the step / action shape (`method`, `url`,
/// `headers`, `body`).
async fn build_request(
    db: &Db,
    input: &Value,
    workspace_id: i64,
    contact_id: Option<i64>,
    doh: &impl DohFetch,
) -> Result<HopRequest> {
    let url = input.get("url").and_then(Value::as_str).unwrap_or("");
    if check_ssrf_safety_with(url, doh).await == SsrfCheck::Unsafe {
        return Err(not_allowed());
    }
    let mut request = HopRequest {
        method: input
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_owned(),
        url: url.to_owned(),
        ..HopRequest::default()
    };
    for (key, value) in pairs(&input["headers"]) {
        if !key.trim().is_empty() {
            request.set(&key, &value);
        }
    }
    let body = &input["body"];
    match body.get("bodyType").and_then(Value::as_str) {
        Some("json") => {
            request.body = Some(body["jsonBody"].as_str().unwrap_or("").to_owned());
            if request.header("Content-Type").is_none() {
                request.set("Content-Type", "application/json");
            }
        }
        Some("formEncoded") => {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            let mut fields: Vec<(String, String)> = Vec::new();
            for (key, value) in pairs(&body["formFields"]) {
                if key.trim().is_empty() {
                    continue;
                }
                // `URLSearchParams.set`: the last value wins, in first position.
                match fields.iter_mut().find(|(k, _)| *k == key) {
                    Some(field) => field.1 = value,
                    None => fields.push((key, value)),
                }
            }
            for (key, value) in &fields {
                form.append_pair(key, value);
            }
            request.body = Some(form.finish());
            request.set("Content-Type", "application/x-www-form-urlencoded");
        }
        Some("allContactData") => {
            let Some(contact_id) = contact_id else {
                return Err(BusinessError::exception(
                    "This request requires a contact and cannot be tested without one",
                    "contactRequired",
                    400,
                ));
            };
            let payload = tx::read(db, move |conn| {
                let contact = crate::contact::service::find_or_fail(conn, workspace_id, contact_id)?;
                let mut stmt = conn
                    .prepare(
                        r#"SELECT "customFieldId", "value" FROM "ContactCustomField" WHERE "contactId" = ?1"#,
                    )
                    .map_err(|e| BusinessError::Repo(e.into()))?;
                let fields = stmt
                    .query_map(params![contact_id], |r| {
                        Ok(json!({
                            "customFieldId": r.get::<_, i64>(0)?.to_string(),
                            "value": r.get::<_, Option<String>>(1)?,
                        }))
                    })
                    .and_then(|rows| rows.collect::<std::result::Result<Vec<_>, _>>())
                    .map_err(|e| BusinessError::Repo(e.into()))?;
                Ok(json!({ "contact": contact.to_json(), "customFields": fields }))
            })
            .await?;
            request.body = Some(payload.to_string());
            request.set("Content-Type", "application/json");
        }
        _ => {}
    }
    Ok(request)
}

fn origin(url: &str) -> Option<url::Origin> {
    url::Url::parse(url).ok().map(|u| u.origin())
}

/// `nextHopInit`: Fetch's HTTP-redirect-fetch step.
fn next_hop(status: u16, prior: &HopRequest, to_url: String) -> HopRequest {
    let method = prior.method.to_uppercase();
    let drop_body = (status == 303 && method != "GET" && method != "HEAD")
        || (matches!(status, 301 | 302) && method == "POST");
    let mut next = HopRequest {
        url: to_url,
        ..prior.clone()
    };
    if drop_body {
        for header in REQUEST_BODY_HEADERS {
            next.remove(header);
        }
        next.method = "GET".into();
        next.body = None;
    }
    if origin(&prior.url) != origin(&next.url) {
        next.remove("Authorization");
    }
    next
}

/// `fetchFollowingSafeRedirects`.
async fn fetch_following_safe_redirects(
    hop: &dyn Hop,
    doh: &impl DohFetch,
    mut request: HopRequest,
) -> Result<HopReply> {
    let mut redirects_left = MAX_REDIRECTS;
    loop {
        let reply = hop.send(&request).await.map_err(transport)?;
        if !(300..400).contains(&reply.status) {
            return Ok(reply);
        }
        if redirects_left == 0 {
            return Err(ssrf_blocked("Too many redirects for this external request"));
        }
        let location = reply
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("location"))
            .map(|(_, v)| v.clone())
            .ok_or_else(not_allowed)?;
        let next_url = url::Url::parse(&request.url)
            .and_then(|base| base.join(&location))
            .map_err(|_| not_allowed())?
            .to_string();
        if check_ssrf_safety_with(&next_url, doh).await == SsrfCheck::Unsafe {
            return Err(not_allowed());
        }
        request = next_hop(reply.status, &request, next_url);
        redirects_left -= 1;
    }
}

/// `execute` over `hop` / `doh`.
pub async fn execute_with(
    db: &Db,
    hop: &dyn Hop,
    doh: &impl DohFetch,
    input: &Value,
    workspace_id: i64,
    contact_id: Option<i64>,
) -> Result<ExternalRequestResult> {
    let request = build_request(db, input, workspace_id, contact_id, doh).await?;
    let started = Instant::now();
    let reply = fetch_following_safe_redirects(hop, doh, request).await?;
    Ok(ExternalRequestResult {
        status_code: reply.status,
        duration_ms: started.elapsed().as_millis() as u64,
        response_body: reply.body,
        response_headers: reply.headers,
    })
}

/// `execute` with the real transport and DoH client.
pub async fn execute(
    db: &Db,
    input: &Value,
    workspace_id: i64,
    contact_id: Option<i64>,
) -> Result<ExternalRequestResult> {
    static HOP: std::sync::OnceLock<ReqwestHop> = std::sync::OnceLock::new();
    execute_with(
        db,
        HOP.get_or_init(ReqwestHop::default),
        &HttpDoh,
        input,
        workspace_id,
        contact_id,
    )
    .await
}

/// `executeAndMap`: a 2xx JSON answer writes each `mapping[]` path to its
/// custom field (`getProperty`; a string as is, anything else as JSON).
pub async fn execute_and_map_with(
    db: &Db,
    hop: &dyn Hop,
    doh: &impl DohFetch,
    workspace_id: i64,
    contact_id: i64,
    input: &Value,
    mapping: &Value,
) -> Result<ExternalRequestResult> {
    let result = execute_with(db, hop, doh, input, workspace_id, Some(contact_id)).await?;
    if result.status_code >= 400 {
        return Ok(result);
    }
    let Ok(payload) = serde_json::from_str::<Value>(&result.response_body) else {
        return Ok(result);
    };
    let fields: Vec<(i64, String)> = mapping
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let path = entry.get("jsonPath")?.as_str()?;
            let value = chatbotx_core::utils::json_path::read_json_path(&payload, path)
                .filter(|v| !v.is_null())?;
            // A non-numeric field id matches no custom field.
            let field = entry.get("outputFieldId")?.as_str()?.parse().ok()?;
            let text = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            Some((field, text))
        })
        .collect();
    if !fields.is_empty() {
        contact_custom_field::set_values(
            db,
            SetValuesInput {
                workspace_id,
                contact_id,
                fields,
                ..SetValuesInput::default()
            },
        )
        .await?;
    }
    Ok(result)
}
