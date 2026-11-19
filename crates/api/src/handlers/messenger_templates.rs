//! `features/integration-messenger/message-templates/api/public.ts`
//! (`messengerTemplatesPublicRouter`, scope `broadcasts`; Q.I1b slice 5),
//! with `lib/message-template-operations.ts`, `lib/build-template-components.ts`,
//! `schema/mutation.ts` and `messengerMessageTemplateService`.
//!
//! ponytail: no template-list cache in Rust, so no `invalidateCacheByTags`;
//! clone targets run one after another (the original runs batches of 5).

use std::time::Duration;

use chatbotx_business::{BusinessError, tx};
use chatbotx_core::flow_config::steps::{
    describe_messenger_template_parameters, find_messenger_token_variable,
    normalize_messenger_token_url,
};
use chatbotx_db::Db;
use chatbotx_db::repo::models::{IntegrationMessenger, MessengerMessageTemplate};
use chatbotx_db::repo::{find_many, find_one, new_id, now_ms};
use chatbotx_db::rusqlite::params;
use serde_json::{Map, Value, json};

use super::input::{Input, Seg, Str, child, key};
use super::tags::{array, digits_at};
use super::{ApiState, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content};
use crate::orpc::OrpcError;

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "messengerTemplates.list",
    "messengerTemplates.get",
    "messengerTemplates.delete",
    "messengerTemplates.create",
    "messengerTemplates.clone",
    "messengerTemplates.sync",
];

pub(super) const GRAPH_URL: &str = "https://graph.facebook.com";
const DEFAULT_API_VERSION: &str = "v23.0";
const TEMPLATE_FIELDS: &str = "name,status,language,category,parameter_format,components";
const MAX_PAGES: usize = 50;
const MAX_HEADER_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
/// `TOKEN_URL_SUFFIX_EXAMPLE`.
const TOKEN_URL_SUFFIX_EXAMPLE: &str = "example-token";
const STATUSES: &[&str] = &["APPROVED", "PENDING", "REJECTED"];

// --- errors ---------------------------------------------------------------------------

/// A failed call: an `SdkException` (Meta, 4xx / its status) or a business error.
pub(crate) enum Fail {
    Sdk(String, u16),
    Business(BusinessError),
}

impl From<BusinessError> for Fail {
    fn from(error: BusinessError) -> Self {
        Fail::Business(error)
    }
}

impl From<chatbotx_db::repo::Error> for Fail {
    fn from(error: chatbotx_db::repo::Error) -> Self {
        Fail::Business(error.into())
    }
}

pub(crate) fn reply_error(fail: Fail, errors: super::ErrorMap) -> OrpcError {
    match fail {
        // `SdkException` = `BAD_REQUEST` with Meta's status (`orpc.ts`).
        Fail::Sdk(message, status) => OrpcError::new("BAD_REQUEST", Some(status), Some(&message)),
        Fail::Business(error) => business_error(error, errors),
    }
}

/// `toPublicErrorMessage`: Meta's and our own messages; never a DB error.
pub(super) fn public_message(fail: &Fail, fallback: &str) -> String {
    match fail {
        Fail::Sdk(message, _) => message.clone(),
        Fail::Business(BusinessError::Exception { message, .. }) => message.clone(),
        Fail::Business(BusinessError::Repo(_)) => fallback.to_owned(),
    }
}

// --- Graph ------------------------------------------------------------------------------

/// `MessengerAuthValue`: page token, page id, version, app id.
struct PageAuth {
    token: String,
    page_id: String,
    version: String,
    client_id: String,
}

fn page_auth(page: &IntegrationMessenger) -> PageAuth {
    let auth = &page.auth.0;
    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or_default().to_owned();
    let version = text(auth.pointer("/metadata/version"));
    PageAuth {
        token: text(auth.pointer("/tokens/accessToken")),
        page_id: text(auth.pointer("/metadata/pageId")),
        version: if version.is_empty() {
            DEFAULT_API_VERSION.to_owned()
        } else {
            version
        },
        client_id: text(auth.get("clientId")),
    }
}

fn http() -> Result<&'static reqwest::Client, Fail> {
    static CLIENT: std::sync::OnceLock<Option<reqwest::Client>> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .ok()
        })
        .as_ref()
        .ok_or_else(|| Fail::Sdk("HTTP client unavailable".into(), 500))
}

/// `rescue(endpoint, ...)` over `facebookGraphClient`: bearer, JSON.
async fn graph(
    method: reqwest::Method,
    auth: &PageAuth,
    path: &str,
    query: &[(&str, &str)],
    body: Option<&Value>,
) -> Result<Value, Fail> {
    let url = format!("{GRAPH_URL}/{}/{path}", auth.version);
    graph_raw(method, Some(&auth.token), &url, query, body).await
}

/// One Meta Graph call at the full `url`; `bearer` = the token sent as
/// `Authorization`, if any.
pub(super) async fn graph_raw(
    method: reqwest::Method,
    bearer: Option<&str>,
    url: &str,
    query: &[(&str, &str)],
    body: Option<&Value>,
) -> Result<Value, Fail> {
    graph_call(method, bearer, url, query, body)
        .await
        .map_err(|failure| failure.0)
}

/// [`graph_raw`], with Meta's `error` object (or null) on a failure.
pub(super) async fn graph_call(
    method: reqwest::Method,
    bearer: Option<&str>,
    url: &str,
    query: &[(&str, &str)],
    body: Option<&Value>,
) -> Result<Value, Box<(Fail, Value)>> {
    let bare = |fail: Fail| Box::new((fail, Value::Null));
    let mut url = url::Url::parse(url).map_err(|e| bare(Fail::Sdk(e.to_string(), 400)))?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    let mut request = http().map_err(bare)?.request(method, url.as_str());
    if let Some(token) = bearer {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    let response = request
        .send()
        .await
        .map_err(|e| bare(Fail::Sdk(e.without_url().to_string(), 400)))?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| bare(Fail::Sdk(e.to_string(), 400)))?;
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let message = chatbotx_core::utils::graph_error::format_graph_error(value.get("error"))
            .unwrap_or_else(|| format!("Request failed with status code {status}"));
        let status = if (400..500).contains(&status) {
            status
        } else {
            400
        };
        let error = value.get("error").cloned().unwrap_or(Value::Null);
        return Err(Box::new((Fail::Sdk(message, status), error)));
    }
    Ok(value)
}

/// `listPageMessageTemplates`: every page (50 max) via `cursors.after`.
async fn list_meta_templates(auth: &PageAuth, name: Option<&str>) -> Result<Vec<Value>, Fail> {
    let path = format!("{}/message_templates", auth.page_id);
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut query = vec![("fields", TEMPLATE_FIELDS)];
        if let Some(name) = name {
            query.push(("name", name));
        }
        if let Some(after) = &after {
            query.push(("after", after.as_str()));
        }
        let page = graph(reqwest::Method::GET, auth, &path, &query, None).await?;
        out.extend(page["data"].as_array().cloned().unwrap_or_default());
        after = match (
            page.pointer("/paging/next"),
            page.pointer("/paging/cursors/after")
                .and_then(Value::as_str),
        ) {
            (Some(next), Some(cursor)) if !next.is_null() => Some(cursor.to_owned()),
            _ => None,
        };
        if after.is_none() {
            break;
        }
    }
    Ok(out)
}

fn is_http_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

fn is_meta_image_url(value: &str) -> bool {
    url::Url::parse(value)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .is_some_and(|h| {
            h == "facebook.com"
                || h.ends_with(".facebook.com")
                || h.ends_with(".fbcdn.net")
                || h.ends_with(".fbsbx.com")
        })
}

/// `uploadHeaderImage` + `resumableUploadImage`: SSRF check, bounded
/// download (no redirects), upload session, bytes; the handle `h`.
async fn upload_header_image(
    auth: &PageAuth,
    image_url: &str,
    authenticated_download: bool,
) -> Result<String, Fail> {
    let plain =
        |message: String| Fail::Business(BusinessError::exception(message, "systemError", 400));
    chatbotx_business::net::ssrf_guard::assert_public_url(image_url, "Template header image URL")
        .await
        .map_err(plain)?;
    let mut request = http()?.get(image_url).timeout(Duration::from_secs(20));
    if authenticated_download {
        request = request.header("authorization", format!("Bearer {}", auth.token));
    }
    let response = request
        .send()
        .await
        .map_err(|e| plain(e.without_url().to_string()))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(plain(format!(
            "Template image download failed: HTTP {status}"
        )));
    }
    let mime = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_owned();
    if !mime.starts_with("image/") {
        return Err(plain(format!(
            "Header image must be an image file, got \"{mime}\" instead"
        )));
    }
    let too_large = || {
        plain(format!(
            "Header image is larger than {MAX_HEADER_IMAGE_BYTES} bytes"
        ))
    };
    if response
        .content_length()
        .is_some_and(|len| len > MAX_HEADER_IMAGE_BYTES)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| plain(e.without_url().to_string()))?
    {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > MAX_HEADER_IMAGE_BYTES {
            return Err(too_large());
        }
    }
    let file_name = url::Url::parse(image_url)
        .ok()
        .and_then(|u| u.path_segments()?.next_back().map(str::to_owned))
        .unwrap_or_else(|| "header.jpg".into());
    let mut session_url = url::Url::parse(&format!(
        "{GRAPH_URL}/{}/{}/uploads",
        auth.version, auth.client_id
    ))
    .map_err(|e| plain(e.to_string()))?;
    session_url
        .query_pairs_mut()
        .append_pair("file_name", &file_name)
        .append_pair("file_length", &bytes.len().to_string())
        .append_pair("file_type", &mime)
        .append_pair("access_token", &auth.token);
    let session = http()?
        .post(session_url.as_str())
        .send()
        .await
        .map_err(|e| plain(e.without_url().to_string()))?;
    let session_status = session.status().as_u16();
    let session_body = session.text().await.unwrap_or_default();
    if !(200..300).contains(&session_status) {
        return Err(plain(format!(
            "Upload session create failed: HTTP {session_status} {session_body}"
        )));
    }
    let session_id = serde_json::from_str::<Value>(&session_body).unwrap_or_default()["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let upload = http()?
        .post(format!("{GRAPH_URL}/{}/{session_id}", auth.version))
        .header("authorization", format!("OAuth {}", auth.token))
        .header("file_offset", "0")
        .body(bytes)
        .send()
        .await
        .map_err(|e| plain(e.without_url().to_string()))?;
    let upload_status = upload.status().as_u16();
    let upload_body = upload.text().await.unwrap_or_default();
    if !(200..300).contains(&upload_status) {
        return Err(plain(format!(
            "Image upload failed: HTTP {upload_status} {upload_body}"
        )));
    }
    Ok(
        serde_json::from_str::<Value>(&upload_body).unwrap_or_default()["h"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    )
}

// --- rows -------------------------------------------------------------------------------

/// `messengerMessageTemplateResource`.
fn template_json(row: &MessengerMessageTemplate) -> Value {
    json!({
        "id": row.id.to_string(),
        "name": row.name,
        "integrationMessengerId": row.integration_messenger_id.to_string(),
        "language": row.language,
        "category": row.category,
        "status": row.status,
        "parameterFormat": row.parameter_format,
        "components": row.components.0,
    })
}

fn template_where(alias_filter: &str) -> String {
    format!(
        r#"WHERE {alias_filter} AND "integrationMessengerId" IN (SELECT "id" FROM "IntegrationMessenger" WHERE "workspaceId" = ?1)"#
    )
}

/// `findByIdForWorkspace` with its page.
async fn find_template(
    db: &Db,
    workspace_id: i64,
    id: i64,
) -> Result<Option<(MessengerMessageTemplate, IntegrationMessenger)>, BusinessError> {
    tx::read(db, move |conn| {
        let Some(template) = find_one::<MessengerMessageTemplate>(
            conn,
            r#""id" = ?2 AND "integrationMessengerId" IN (SELECT "id" FROM "IntegrationMessenger" WHERE "workspaceId" = ?1)"#,
            &[&workspace_id, &id],
        )?
        else {
            return Ok(None);
        };
        let page = find_one::<IntegrationMessenger>(
            conn,
            r#""id" = ?1"#,
            &[&template.integration_messenger_id],
        )?;
        Ok(page.map(|page| (template, page)))
    })
    .await
}

/// `messengerIntegrationService.findByIdForWorkspace`.
async fn find_page(
    db: &Db,
    workspace_id: i64,
    id: i64,
) -> Result<IntegrationMessenger, BusinessError> {
    tx::read(db, move |conn| {
        find_one::<IntegrationMessenger>(
            conn,
            r#""id" = ?1 AND "workspaceId" = ?2"#,
            &[&id, &workspace_id],
        )?
        .ok_or_else(|| BusinessError::not_found("Messenger channel not found"))
    })
    .await
}

/// `syncFromMeta`: upsert by `(integrationMessengerId, sourceId)`; a full
/// sync also deletes the rows Meta no longer lists.
async fn sync_from_meta(
    db: &Db,
    integration_id: i64,
    templates: Vec<Value>,
    partial: bool,
) -> Result<(), BusinessError> {
    tx::write(db, move |conn| {
        if !partial {
            let incoming: Vec<String> = templates
                .iter()
                .filter_map(|t| t["id"].as_str().map(str::to_owned))
                .collect();
            conn.execute(
                r#"DELETE FROM "MessengerMessageTemplate" WHERE "integrationMessengerId" = ?1
                   AND "sourceId" NOT IN (SELECT value FROM json_each(?2))"#,
                params![integration_id, serde_json::to_string(&incoming).unwrap_or_default()],
            )?;
        }
        let text = |t: &Value, k: &str| t[k].as_str().unwrap_or_default().to_owned();
        for t in &templates {
            let format = t["parameter_format"].as_str().unwrap_or("POSITIONAL");
            conn.execute(
                r#"INSERT INTO "MessengerMessageTemplate" ("id", "name", "integrationMessengerId", "language",
                     "category", "status", "parameterFormat", "sourceId", "components")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                   ON CONFLICT ("integrationMessengerId", "sourceId") DO UPDATE SET
                     "name" = excluded."name", "language" = excluded."language",
                     "category" = excluded."category", "status" = excluded."status",
                     "parameterFormat" = excluded."parameterFormat",
                     "components" = excluded."components", "updatedAt" = ?10"#,
                params![
                    new_id(),
                    text(t, "name"),
                    integration_id,
                    text(t, "language"),
                    text(t, "category"),
                    text(t, "status"),
                    format,
                    text(t, "id"),
                    t.get("components").cloned().unwrap_or(json!([])).to_string(),
                    now_ms(),
                ],
            )?;
        }
        Ok(())
    })
    .await
}

/// `syncMessengerMessageTemplatesForIntegration`.
async fn sync_page(
    db: &Db,
    page: &IntegrationMessenger,
    filter: Option<(&str, &str, &str)>,
) -> Result<(), Fail> {
    let auth = page_auth(page);
    let templates = list_meta_templates(&auth, filter.map(|(_, name, _)| name))
        .await
        .map_err(|fail| {
            let message = match &fail {
                Fail::Sdk(message, _) => message.clone(),
                other => public_message(other, "Unknown error"),
            };
            Fail::Sdk(
                format!("Failed to fetch Messenger templates from Facebook API: {message}"),
                400,
            )
        })?;
    let templates: Vec<Value> = templates
        .into_iter()
        .filter(|t| {
            filter.is_none_or(|(id, name, language)| {
                t["id"].as_str() == Some(id)
                    && t["name"].as_str() == Some(name)
                    && t["language"].as_str() == Some(language)
            })
        })
        .collect();
    sync_from_meta(db, page.id, templates, filter.is_some()).await?;
    Ok(())
}

// --- create request ---------------------------------------------------------------------

/// `{{...}}` runs of a text (`/{{[^{}]*}}/g`).
fn any_placeholders(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        match after.find(['{', '}']) {
            Some(end) if after[end..].starts_with("}}") => {
                out.push(&rest[start..start + 2 + end + 2]);
                rest = &after[end + 2..];
            }
            Some(end) => rest = &after[end..],
            None => break,
        }
    }
    out
}

fn is_numeric_placeholder(p: &str) -> bool {
    let inner = &p[2..p.len() - 2];
    !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit())
}

/// `SUPPORTED_PLACEHOLDER_PATTERN`: `{{1}}` to `{{15}}`.
fn is_supported_placeholder(p: &str) -> bool {
    is_numeric_placeholder(p)
        && !p[2..].starts_with('0')
        && p[2..p.len() - 2]
            .parse::<u32>()
            .is_ok_and(|n| (1..=15).contains(&n))
}

/// `extractPlaceholders`: unique numeric placeholders, by number.
fn extract_placeholders(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in any_placeholders(text) {
        if is_numeric_placeholder(p) && !out.iter().any(|o| o == p) {
            out.push(p.to_owned());
        }
    }
    out.sort_by_key(|p| p[2..p.len() - 2].parse::<f64>().unwrap_or(0.0) as u64);
    out
}

struct Variable {
    key: String,
    example: String,
}

enum Button {
    Postback(String),
    Phone(String, String),
    Url(String, String, Vec<String>),
    MiniApp(String, i64),
}

struct CreateRequest {
    name: String,
    language: String,
    header_type: String,
    header_text: String,
    header_variables: Vec<Variable>,
    header_image_url: Option<String>,
    body: String,
    body_variables: Vec<Variable>,
    buttons: Vec<Button>,
}

fn variables(input: &mut Input, name: &str, max: usize) -> Option<Vec<Variable>> {
    if !input.has(name) {
        return Some(Vec::new());
    }
    array(input, name, 0, max, |input, path, value| {
        let path = path.to_vec();
        let key = input.string_at(
            &child(&path, "key"),
            value.get("key"),
            Str {
                pattern: Some(is_supported_placeholder),
                ..Str::default()
            },
            true,
        );
        let example = input.string_at(
            &child(&path, "example"),
            value.get("example"),
            Str {
                min: Some(1),
                ..Str::default()
            },
            true,
        );
        Some(Variable {
            key: key?,
            example: example?,
        })
    })
}

fn button(input: &mut Input, path: &[Seg], value: &Value) -> Option<Button> {
    let path = path.to_vec();
    if !value.is_object() {
        input.issue(
            &path,
            "invalid_type",
            &format!(
                "Invalid input: expected object, received {}",
                super::input::type_name(Some(value))
            ),
        );
        return None;
    }
    let title_spec = Str {
        min: Some(1),
        max: Some(20),
        ..Str::default()
    };
    let non_empty = Str {
        min: Some(1),
        ..Str::default()
    };
    let kind = value.get("type").and_then(Value::as_str);
    if !matches!(kind, Some("POSTBACK" | "PHONE_NUMBER" | "URL" | "MINI_APP")) {
        input.issue(&child(&path, "type"), "invalid_union", "Invalid input");
        return None;
    }
    let title = input.string_at(&child(&path, "title"), value.get("title"), title_spec, true);
    match kind {
        Some("POSTBACK") => Some(Button::Postback(title?)),
        Some("PHONE_NUMBER") => {
            let phone = input.string_at(
                &child(&path, "phoneNumber"),
                value.get("phoneNumber"),
                non_empty,
                true,
            );
            Some(Button::Phone(title?, phone?))
        }
        Some("URL") => {
            let url = input.string_at(
                &child(&path, "url"),
                value.get("url"),
                Str::trimmed(1, usize::MAX),
                true,
            );
            let vars_path = child(&path, "variables");
            let vars = match value.get("variables") {
                None => Some(Vec::new()),
                Some(Value::Array(items)) => {
                    let mut out = Vec::new();
                    let mut ok = true;
                    for (index, item) in items.iter().enumerate() {
                        let mut at = vars_path.clone();
                        at.push(Seg::Index(index));
                        match input.string_at(&at, Some(item), non_empty, true) {
                            Some(text) => out.push(text),
                            None => ok = false,
                        }
                    }
                    if items.len() > 1 {
                        input.issue(
                            &vars_path,
                            "too_big",
                            "Too big: expected array to have <=1 items",
                        );
                        ok = false;
                    }
                    ok.then_some(out)
                }
                Some(other) => {
                    let other = other.clone();
                    input.issue(
                        &vars_path,
                        "invalid_type",
                        &format!(
                            "Invalid input: expected array, received {}",
                            super::input::type_name(Some(&other))
                        ),
                    );
                    None
                }
            };
            Some(Button::Url(title?, url?, vars?))
        }
        _ => {
            let mini_app = digits_at(
                input,
                &child(&path, "miniAppId"),
                value.get("miniAppId"),
                true,
            );
            Some(Button::MiniApp(title?, mini_app?))
        }
    }
}

fn placeholder_issues(input: &mut Input, path: &str, text: &str, max: usize, vars: &[Variable]) {
    let placeholders = extract_placeholders(text);
    if placeholders.iter().any(|p| !is_supported_placeholder(p)) {
        input.issue(
            &key(path),
            "custom",
            "Only variables from {{1}} to {{15}} are supported",
        );
        return;
    }
    if placeholders.len() > max {
        input.issue(
            &key(path),
            "custom",
            &format!("Maximum {max} variable(s) are supported"),
        );
        return;
    }
    let keys: Vec<&str> = vars.iter().map(|v| v.key.as_str()).collect();
    let missing = placeholders.iter().any(|p| !keys.contains(&p.as_str()));
    let stale = keys.iter().any(|k| !placeholders.iter().any(|p| p == k));
    if missing || stale || keys.len() != placeholders.len() {
        input.issue(
            &key(path),
            "custom",
            "Variable examples must match the template text",
        );
    }
}

/// `createMessengerMessageTemplateRequest` (shape, refinements, transform).
fn create_request(input: &mut Input) -> Option<CreateRequest> {
    let name = input.string(
        "name",
        Str {
            trim: true,
            min: Some(1),
            max: Some(512),
            pattern: Some(|t| {
                !t.is_empty()
                    && t.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            }),
        },
    );
    let language = input.string(
        "language",
        Str {
            min: Some(1),
            ..Str::default()
        },
    );
    let value = input.get("headerType").cloned();
    let header_type = input.enum_at(
        &key("headerType"),
        value.as_ref(),
        &["none", "text", "text_and_image"],
        true,
    );
    let header_text = if input.has("headerText") {
        input.string("headerText", Str::default())
    } else {
        Some(String::new())
    };
    let header_variables = variables(input, "headerVariables", 1);
    let header_image_url = if input.has("headerImageUrl") {
        let value = input.get("headerImageUrl").cloned();
        super::polls::url_at(input, &key("headerImageUrl"), value.as_ref(), true).map(Some)
    } else {
        Some(None)
    };
    let body = input.string(
        "body",
        Str {
            min: Some(1),
            ..Str::default()
        },
    );
    let body_variables = variables(input, "bodyVariables", 15);
    let buttons = if input.has("buttons") {
        array(input, "buttons", 0, 3, button)
    } else {
        Some(Vec::new())
    };
    if input.failed() {
        return None;
    }
    let request = CreateRequest {
        name: name?,
        language: language?,
        header_type: header_type?,
        header_text: header_text?,
        header_variables: header_variables?,
        header_image_url: header_image_url?,
        body: body?,
        body_variables: body_variables?,
        buttons: buttons?,
    };
    // `superRefine`.
    if request.header_type != "none" && request.header_text.trim().is_empty() {
        input.issue(&key("headerText"), "custom", "Header text is required");
    }
    if request.header_type == "text_and_image" && request.header_image_url.is_none() {
        input.issue(&key("headerImageUrl"), "custom", "Header image is required");
    }
    if request.header_type != "none" {
        placeholder_issues(
            input,
            "headerVariables",
            &request.header_text,
            1,
            &request.header_variables,
        );
    }
    placeholder_issues(
        input,
        "bodyVariables",
        &request.body,
        15,
        &request.body_variables,
    );
    for (index, button) in request.buttons.iter().enumerate() {
        let Button::Url(_, url, vars) = button else {
            continue;
        };
        let url_path = vec![
            Seg::Key("buttons".into()),
            Seg::Index(index),
            Seg::Key("url".into()),
        ];
        let all = any_placeholders(url);
        if normalize_messenger_token_url(url).is_some() {
            if all.len() > 1 {
                input.issue(
                    &url_path,
                    "custom",
                    "A Mini App or Minigame link cannot have other variables besides its token",
                );
            }
            continue;
        }
        if find_messenger_token_variable(Some(url)).is_some() {
            input.issue(
                &url_path,
                "custom",
                "The token variable must be the last part of a Mini App or Minigame link",
            );
            continue;
        }
        let placeholders = extract_placeholders(url);
        if placeholders.iter().any(|p| !is_supported_placeholder(p))
            || all.iter().any(|p| !is_numeric_placeholder(p))
        {
            input.issue(
                &url_path,
                "custom",
                "Only variables from {{1}} to {{15}} are supported",
            );
            continue;
        }
        if placeholders.len() > 1 {
            input.issue(&url_path, "custom", "URL buttons support only one variable");
            continue;
        }
        if vars.len() != placeholders.len() {
            input.issue(
                &[
                    Seg::Key("buttons".into()),
                    Seg::Index(index),
                    Seg::Key("variables".into()),
                ],
                "custom",
                "URL examples must match URL variables",
            );
        }
    }
    if input.failed() {
        return None;
    }
    // `.transform`: `normalizeTokenUrlButton`.
    let mut request = request;
    for button in &mut request.buttons {
        if let Button::Url(_, url, vars) = button
            && let Some((normalized, _)) = normalize_messenger_token_url(url)
        {
            *url = normalized;
            if vars.is_empty() {
                vars.push(TOKEN_URL_SUFFIX_EXAMPLE.to_owned());
            }
        }
    }
    Some(request)
}

/// `buildUrlSuffixExample`.
fn url_suffix_example(url: &str, vars: &[String]) -> String {
    vars.iter()
        .enumerate()
        .fold(url.to_owned(), |out, (index, var)| {
            out.replace(&format!("{{{{{}}}}}", index + 1), var)
        })
}

/// `getBrokerOrigin()`.
pub(crate) fn broker_origin(app_url: &str) -> String {
    let raw = std::env::var("NEXT_PUBLIC_BROKER_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| app_url.to_owned());
    url::Url::parse(&raw)
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or(raw)
}

/// `buildMessengerMessageTemplateComponents`.
fn build_components(
    request: &CreateRequest,
    header_handle: Option<&str>,
    app_url: &str,
) -> Result<Vec<Value>, Fail> {
    let examples =
        |vars: &[Variable]| -> Vec<Value> { vars.iter().map(|v| json!(v.example)).collect() };
    let mut components = Vec::new();
    if request.header_type == "text" {
        let mut header = json!({ "type": "HEADER", "format": "TEXT", "text": request.header_text });
        let example = examples(&request.header_variables);
        if !example.is_empty() {
            header["example"] = json!({ "header_text": example });
        }
        components.push(header);
    }
    if request.header_type == "text_and_image" {
        let mut example = Map::new();
        let header_text = examples(&request.header_variables);
        if !header_text.is_empty() {
            example.insert("header_text".into(), json!(header_text));
        }
        example.insert(
            "header_handle".into(),
            json!(header_handle.map(|h| vec![h]).unwrap_or_default()),
        );
        components.push(json!({ "type": "HEADER", "format": "IMAGE", "text": request.header_text, "example": example }));
    }
    let mut body = json!({ "type": "BODY", "text": request.body });
    let body_example = examples(&request.body_variables);
    if !body_example.is_empty() {
        body["example"] = json!({ "body_text": [body_example] });
    }
    components.push(body);
    if !request.buttons.is_empty() {
        let mut buttons = Vec::new();
        for button in &request.buttons {
            buttons.push(match button {
                Button::Postback(title) => json!({ "type": "POSTBACK", "text": title, "payload": "{{1}}" }),
                Button::Phone(title, phone) => {
                    json!({ "type": "PHONE_NUMBER", "text": title, "phone_number": phone })
                }
                Button::Url(title, url, vars) => {
                    let mut out = json!({ "type": "URL", "text": title, "url": url });
                    if !vars.is_empty() {
                        out["example"] = json!({ "url_suffix_example": url_suffix_example(url, vars) });
                    }
                    out
                }
                Button::MiniApp(title, id) => {
                    let public = format!(
                        "{}/mini-apps?id={id}&token={{{{mini_app_token}}}}",
                        broker_origin(app_url)
                    );
                    let Some((url, _)) = normalize_messenger_token_url(&public) else {
                        return Err(Fail::Business(BusinessError::exception(
                            "A Mini App link must end in its token variable",
                            "systemError",
                            400,
                        )));
                    };
                    let example = url_suffix_example(&url, &[TOKEN_URL_SUFFIX_EXAMPLE.to_owned()]);
                    json!({ "type": "URL", "text": title, "url": url, "example": { "url_suffix_example": example } })
                }
            });
        }
        components.push(json!({ "type": "BUTTONS", "buttons": buttons }));
    }
    Ok(components)
}

/// `createMessengerMessageTemplate`.
async fn create_template(
    state: &ApiState,
    workspace_id: i64,
    page: &IntegrationMessenger,
    request: CreateRequest,
) -> Result<Value, Fail> {
    // `assertMiniAppButtonsInWorkspace`.
    for button in &request.buttons {
        if let Button::MiniApp(_, id) = button {
            let id = *id;
            tx::read(&state.db, move |conn| {
                find_one::<chatbotx_db::repo::models::MiniApp>(
                    conn,
                    r#""id" = ?1 AND "workspaceId" = ?2"#,
                    &[&id, &workspace_id],
                )?
                .ok_or_else(|| BusinessError::not_found("Mini App not found"))
            })
            .await?;
        }
    }
    let auth = page_auth(page);
    let header_handle = match (&request.header_type[..], &request.header_image_url) {
        ("text_and_image", Some(url)) => Some(upload_header_image(&auth, url, false).await?),
        _ => None,
    };
    let components = build_components(&request, header_handle.as_deref(), &state.app_url)?;
    let resp = graph(
        reqwest::Method::POST,
        &auth,
        &format!("{}/message_templates", auth.page_id),
        &[],
        Some(&json!({
            "name": request.name,
            "language": request.language,
            "category": "UTILITY",
            "components": components,
        })),
    )
    .await?;
    let meta_id = resp["id"].as_str().unwrap_or_default().to_owned();
    sync_page(
        &state.db,
        page,
        Some((&meta_id, &request.name, &request.language)),
    )
    .await?;
    let (integration_id, source) = (page.id, meta_id.clone());
    let local = tx::read(&state.db, move |conn| {
        Ok(find_one::<MessengerMessageTemplate>(
            conn,
            r#""integrationMessengerId" = ?1 AND "sourceId" = ?2"#,
            &[&integration_id, &source],
        )?
        .map(|t| t.id.to_string()))
    })
    .await?;
    let opt = |k: &str| {
        resp.get(k)
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or(Value::Null)
    };
    Ok(json!({
        "id": meta_id,
        "templateId": local,
        "status": resp["status"],
        "rejectionReason": opt("rejection_reason"),
        "specificRejectionReason": opt("specific_rejection_reason"),
    }))
}

/// `prepareComponentsForClone`: IMAGE headers re-uploaded to the target.
async fn prepare_components_for_clone(components: &Value, auth: &PageAuth) -> Result<Value, Fail> {
    let mut out = Vec::new();
    for c in components.as_array().cloned().unwrap_or_default() {
        let upper = |k: &str| c.get(k).and_then(Value::as_str).map(str::to_uppercase);
        if upper("type").as_deref() != Some("HEADER") || upper("format").as_deref() != Some("IMAGE")
        {
            out.push(c);
            continue;
        }
        let stored = c
            .pointer("/example/header_handle/0")
            .and_then(Value::as_str)
            .filter(|h| is_http_url(h))
            .or_else(|| {
                c.pointer("/example/header_image_url")
                    .and_then(Value::as_str)
                    .filter(|h| is_http_url(h))
            })
            .map(str::to_owned);
        let Some(stored) = stored else {
            return Err(Fail::Business(BusinessError::exception(
                "Image header cannot be cloned because Meta returned a page-owned file handle instead of a downloadable image URL. Recreate the template on the target channel with the original image.",
                "systemError",
                400,
            )));
        };
        let handle = upload_header_image(auth, &stored, is_meta_image_url(&stored)).await?;
        let mut c = c;
        let mut example = c
            .get("example")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        example.remove("header_image_url");
        example.insert("header_handle".into(), json!([handle]));
        c["example"] = Value::Object(example);
        out.push(c);
    }
    Ok(Value::Array(out))
}

/// `cloneMessengerMessageTemplate`.
async fn clone_template(
    db: &Db,
    source: &MessengerMessageTemplate,
    targets: Vec<IntegrationMessenger>,
) -> Value {
    let (mut succeeded, mut failed) = (Vec::new(), Vec::new());
    for target in targets {
        let auth = page_auth(&target);
        let outcome: Result<Option<String>, Fail> = async {
            let components = prepare_components_for_clone(&source.components.0, &auth).await?;
            let resp = graph(
                reqwest::Method::POST,
                &auth,
                &format!("{}/message_templates", auth.page_id),
                &[],
                Some(&json!({
                    "name": source.name,
                    "category": source.category,
                    "language": source.language,
                    "parameter_format": source.parameter_format,
                    "components": components,
                })),
            )
            .await?;
            let status = resp["status"].as_str().unwrap_or_default().to_owned();
            if status != "APPROVED" {
                return Ok(Some(format!("Template returned status: {status}")));
            }
            let id = resp["id"].as_str().unwrap_or_default().to_owned();
            sync_page(db, &target, Some((&id, &source.name, &source.language))).await?;
            Ok(None)
        }
        .await;
        match outcome {
            Ok(None) => succeeded.push(json!({ "channel": target.name })),
            Ok(Some(error)) => failed.push(json!({ "channel": target.name, "error": error })),
            Err(fail) => failed.push(json!({
                "channel": target.name,
                "error": public_message(&fail, "Could not clone the template"),
            })),
        }
    }
    json!({ "succeeded": succeeded, "failed": failed })
}

fn id_param(input: &mut Input) -> Option<i64> {
    let value = input.get("id").cloned();
    digits_at(input, &key("id"), value.as_ref(), true)
}

fn opt_digits(input: &mut Input, name: &str) -> Option<Option<i64>> {
    let value = input.get(name).cloned()?;
    digits_at(input, &key(name), Some(&value), true).map(Some)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "messengerTemplates.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let inbox = opt_digits(&mut input, "inboxId").flatten();
            let integration = opt_digits(&mut input, "integrationMessengerId").flatten();
            let value = input.get("status").cloned();
            let status = input.enum_at(&key("status"), value.as_ref(), STATUSES, false);
            input.finish()?;
            let rows = tx::read(db, move |conn| {
                // `resolveIntegrationMessengerId`.
                let integration = match (integration, inbox) {
                    (Some(id), _) => Some(id),
                    (None, Some(inbox)) => {
                        let page = find_one::<IntegrationMessenger>(
                            conn,
                            r#""workspaceId" = ?1 AND "inboxId" = ?2"#,
                            &[&ws, &inbox],
                        )?;
                        match page {
                            Some(page) => Some(page.id),
                            None => return Ok(Vec::new()),
                        }
                    }
                    (None, None) => None,
                };
                let templates: Vec<MessengerMessageTemplate> = find_many(
                    conn,
                    &format!(
                        "{} ORDER BY \"id\" DESC",
                        template_where(
                            r#"(?2 IS NULL OR "integrationMessengerId" = ?2) AND (?3 IS NULL OR "status" = ?3)"#
                        )
                    ),
                    &[&ws, &integration, &status],
                )?;
                let pages: Vec<IntegrationMessenger> =
                    find_many(conn, r#"WHERE "workspaceId" = ?1"#, &[&ws])?;
                Ok(templates
                    .iter()
                    .filter_map(|t| {
                        let page = pages.iter().find(|p| p.id == t.integration_messenger_id)?;
                        let mut out = template_json(t);
                        out["integrationMessenger"] = json!({
                            "id": page.id.to_string(),
                            "name": page.name,
                            "inboxId": page.inbox_id.to_string(),
                        });
                        Some(out)
                    })
                    .collect::<Vec<_>>())
            })
            .await
            .map_err(|e| business_error(e, LISTING))?;
            Ok(json(200, Value::Array(rows)))
        }
        "messengerTemplates.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input).unwrap_or_default();
            input.finish()?;
            let (template, _) = find_template(db, ws, id)
                .await
                .map_err(|e| business_error(e, FINDING))?
                .ok_or_else(|| {
                    business_error(BusinessError::not_found("Template not found"), FINDING)
                })?;
            let components = template
                .components
                .0
                .as_array()
                .cloned()
                .unwrap_or_default();
            let parameters: Vec<Value> = describe_messenger_template_parameters(&components)
                .into_iter()
                .map(|spec| {
                    let mut out = json!({
                        "key": spec.key,
                        "component": spec.component,
                        "kind": spec.kind,
                        "required": spec.required,
                    });
                    if let Some(placeholder) = spec.placeholder {
                        out["placeholder"] = json!(placeholder);
                    }
                    out
                })
                .collect();
            let mut out = template_json(&template);
            out["parameters"] = Value::Array(parameters);
            Ok(json(200, out))
        }
        "messengerTemplates.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input).unwrap_or_default();
            input.finish()?;
            let (template, _) = find_template(db, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?
                .ok_or_else(|| {
                    business_error(BusinessError::not_found("Template not found"), MUTATING)
                })?;
            tx::write(db, move |conn| {
                conn.execute(
                    r#"DELETE FROM "MessengerMessageTemplate" WHERE "id" = ?1 AND "integrationMessengerId" = ?2"#,
                    params![template.id, template.integration_messenger_id],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "messengerTemplates.sync" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_param(&mut input).unwrap_or_default();
            input.finish()?;
            let page = find_page(db, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            sync_page(db, &page, None)
                .await
                .map_err(|f| reply_error(f, MUTATING))?;
            Ok(no_content())
        }
        "messengerTemplates.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let request = create_request(&mut input);
            let id = id_param(&mut input);
            input.finish()?;
            let (Some(request), Some(id)) = (request, id) else {
                return Err(super::internal_error("invalid request"));
            };
            let page = find_page(db, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            let out = create_template(state, ws, &page, request)
                .await
                .map_err(|f| reply_error(f, MUTATING))?;
            Ok(json(201, out))
        }
        "messengerTemplates.clone" => {
            let mut input = Input::from_body(call.params, call.body);
            let targets = array(
                &mut input,
                "targetIntegrationMessengerIds",
                1,
                20,
                |input, path, value| digits_at(input, path, Some(value), true),
            );
            let id = id_param(&mut input).unwrap_or_default();
            input.finish()?;
            let requested = targets.unwrap_or_default();
            let (source, source_page) = find_template(db, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?
                .ok_or_else(|| {
                    business_error(BusinessError::not_found("Template not found"), MUTATING)
                })?;
            let pages = tx::read(db, move |conn| {
                Ok(find_many::<IntegrationMessenger>(
                    conn,
                    r#"WHERE "workspaceId" = ?1 ORDER BY "createdAt", "id""#,
                    &[&ws],
                )?)
            })
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            let targets: Vec<IntegrationMessenger> = pages
                .into_iter()
                .filter(|p| requested.contains(&p.id) && p.page_id != source_page.page_id)
                .collect();
            if targets.is_empty() {
                return Err(business_error(
                    BusinessError::not_found("No target channels found in this workspace"),
                    MUTATING,
                ));
            }
            Ok(json(200, clone_template(db, &source, targets).await))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_extracted_unique_and_by_number() {
        assert_eq!(
            extract_placeholders("{{2}} {{10}} {{1}} {{2}} {{x}}"),
            vec!["{{1}}", "{{2}}", "{{10}}"]
        );
        assert_eq!(
            any_placeholders("a {{x}} {{1}} {{ {{2}}"),
            vec!["{{x}}", "{{1}}", "{{2}}"]
        );
        assert!(is_supported_placeholder("{{15}}"));
        assert!(!is_supported_placeholder("{{16}}"));
        assert!(!is_supported_placeholder("{{01}}"));
        assert_eq!(
            url_suffix_example("https://x.io/{{1}}", &["a".into()]),
            "https://x.io/a"
        );
    }
}
