//! A Mini App as a WhatsApp Flow (Q.D1m): `features/mini-apps/lib/
//! whatsapp-publication.ts` (publish, draft, preview, draft delete) and the
//! Flow removal of `lib/delete-mini-apps.ts`, with the Graph calls they use
//! from `integrations/whatsapp/src/api/flow.ts` (`publishFlowJson`,
//! `publishFlowDraft`, `getFlowPreview`, `deleteFlow`, `deleteFlowDraft`),
//! `whatsappFlowService.upsertFromMeta` / `markRemovedOnMeta` and
//! `miniAppPublicationService.findForIntegration` / `record` / `remove`.

use std::time::Duration;

use chatbotx_core::mini_app as app;
use chatbotx_db::Db;
use chatbotx_db::repo::models::{IntegrationWhatsapp, MiniAppPublication};
use chatbotx_db::repo::{Model as _, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use futures::StreamExt as _;
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result, sanitize_public_text};
use crate::tx;
use crate::whatsapp::DEFAULT_API_VERSION;

/// `messages/en.json` `miniApps.publish.*`.
pub const NOT_ON_NUMBER: &str = "The Mini App is not on this WhatsApp number.";
pub const CONTACT_VARIABLES: &str = "This Mini App shows custom fields in its texts. They only work on the web link for now — remove them before publishing to WhatsApp.";
pub const FLOW_IS_LIVE: &str = "The Mini App is live on this WhatsApp number. Saving a draft would take it offline — publish instead to update it.";
pub const ONLY_DRAFT_DELETABLE: &str =
    "The Flow is live on Meta, so it can't be deleted. Only a draft can be deleted.";
const NUMBER_NOT_FOUND: &str = "WhatsApp number not found";
const NUMBER_NOT_CONNECTED: &str = "WhatsApp number is no longer in this workspace";

const FLOW_FIELDS: &str = "id,name,status,categories,validation_errors";
/// Flows that can still take a new Flow JSON in place.
const EDITABLE: &[&str] = &["DRAFT", "PUBLISHED"];
/// Statuses of a published Flow (Meta deprecates these, it never deletes them).
const LIVE: &[&str] = &["PUBLISHED", "THROTTLED", "BLOCKED"];
const PUBLISH_REFUSED: &str = "Meta did not publish the Flow";
/// "Can't deprecate unpublished flow" / "Flow is already deprecated".
const CANNOT_DEPRECATE: i64 = 139_003;
/// "Can't delete published Flow".
const CANNOT_DELETE: i64 = 139_004;
/// `FLOW_REMOVAL_CONCURRENCY`.
const REMOVAL_CONCURRENCY: usize = 5;

// ---- Graph -------------------------------------------------------------------

/// A failed Graph call (`parseOriginError`).
#[derive(Debug, Clone, Default)]
pub struct MetaError {
    pub status: u16,
    pub code: Option<i64>,
    pub message: Option<String>,
    pub user_message: Option<String>,
}

impl MetaError {
    fn of(message: impl Into<String>) -> MetaError {
        MetaError {
            status: 400,
            message: Some(message.into()),
            ..MetaError::default()
        }
    }

    /// `isFlowMissing`: code 100 with "does not exist".
    fn missing(&self) -> bool {
        self.code == Some(100)
            && self
                .message
                .as_deref()
                .is_some_and(|m| m.to_lowercase().contains("does not exist"))
    }

    fn text(&self) -> String {
        sanitize_public_text(
            self.message
                .as_deref()
                .unwrap_or("WhatsApp API call failed"),
        )
    }

    /// `rescue` → `WhatsappException`: `BAD_REQUEST` with Meta's status.
    fn into_business(self) -> BusinessError {
        BusinessError::exception(self.text(), "BAD_REQUEST", self.status.clamp(400, 599))
    }
}

/// The number's Graph access (`buildWhatsappContext`).
struct Graph<'a> {
    base: &'a str,
    version: String,
    token: String,
    waba_id: String,
}

impl<'a> Graph<'a> {
    fn new(base: &'a str, integration: &IntegrationWhatsapp) -> Graph<'a> {
        let auth = &integration.auth.0;
        let text = |p: &str| {
            auth.pointer(p)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let version = text("/version");
        Graph {
            base,
            version: if version.is_empty() {
                DEFAULT_API_VERSION.to_owned()
            } else {
                version
            },
            token: text("/tokens/accessToken"),
            waba_id: text("/metadata/wabaId"),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}/{path}", self.base, self.version)
    }
}

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    })
}

async fn send(
    request: reqwest::RequestBuilder,
    token: &str,
) -> std::result::Result<Value, MetaError> {
    let response = request
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| MetaError::of(e.without_url().to_string()))?;
    let status = response.status();
    let body: Value = response
        .bytes()
        .await
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    let error = body.get("error");
    let text = |k: &str| {
        error
            .and_then(|e| e.get(k))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Err(MetaError {
        status: error
            .and_then(|e| e.get("status"))
            .and_then(Value::as_u64)
            .and_then(|s| u16::try_from(s).ok())
            .unwrap_or(status.as_u16()),
        code: error.and_then(|e| e.get("code")).and_then(|c| {
            c.as_i64()
                .or_else(|| c.as_str().and_then(|s| s.parse().ok()))
        }),
        message: text("message").or_else(|| Some(format!("HTTP {status}"))),
        user_message: text("error_user_msg"),
    })
}

type Meta<T> = std::result::Result<T, MetaError>;

async fn get_flow(g: &Graph<'_>, flow_id: &str) -> Meta<Value> {
    send(
        http().get(g.url(flow_id)).query(&[("fields", FLOW_FIELDS)]),
        &g.token,
    )
    .await
}

fn status_of(flow: &Value) -> &str {
    flow.get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn has_errors(write: &Value) -> bool {
    write
        .get("validation_errors")
        .and_then(Value::as_array)
        .is_some_and(|e| !e.is_empty())
}

/// `updateFlowJson`: the Flow JSON as a `FLOW_JSON` asset (multipart).
async fn update_flow_json(g: &Graph<'_>, flow_id: &str, flow_json: &str) -> Meta<Value> {
    // ponytail: hand-built multipart (reqwest's `multipart` feature is off here).
    let boundary = format!("chatbotx-{}", new_id());
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"flow.json\"\r\nContent-Type: application/json\r\n\r\n{flow_json}\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nflow.json\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"asset_type\"\r\n\r\nFLOW_JSON\r\n--{boundary}--\r\n"
    );
    let request = http()
        .post(g.url(&format!("{flow_id}/assets")))
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body);
    send(request, &g.token).await
}

/// `PublishFlowJsonResult`.
#[derive(Debug, Clone)]
pub struct FlowResult {
    pub flow: Value,
    pub published: bool,
    pub publish_error: Option<String>,
    pub refused_live: bool,
}

impl FlowResult {
    fn of(flow: Value, published: bool) -> FlowResult {
        FlowResult {
            flow,
            published,
            publish_error: None,
            refused_live: false,
        }
    }
}

/// `tryPublish`: Meta's reason when it refuses, else `None`.
async fn try_publish(g: &Graph<'_>, flow_id: &str) -> Option<String> {
    match send(http().post(g.url(&format!("{flow_id}/publish"))), &g.token).await {
        Ok(body) => {
            (body.get("success") == Some(&Value::Bool(false))).then(|| PUBLISH_REFUSED.to_owned())
        }
        Err(e) => {
            tracing::warn!(flow_id, error = ?e.message, "Meta refused to publish the Flow");
            Some(sanitize_public_text(
                e.user_message
                    .or(e.message)
                    .as_deref()
                    .unwrap_or(PUBLISH_REFUSED),
            ))
        }
    }
}

/// `publishDraft`: publishes a Flow read as DRAFT.
async fn publish_draft_flow(g: &Graph<'_>, flow_id: &str) -> Meta<FlowResult> {
    let current = get_flow(g, flow_id).await?;
    if status_of(&current) != "DRAFT" {
        let published = status_of(&current) == "PUBLISHED";
        return Ok(FlowResult::of(current, published));
    }
    let publish_error = try_publish(g, flow_id).await;
    let mut flow = get_flow(g, flow_id).await?;
    if publish_error.is_some() {
        return Ok(FlowResult {
            publish_error,
            ..FlowResult::of(flow, false)
        });
    }
    // Meta accepted the publish; a read right after it can still say DRAFT.
    flow["status"] = json!("PUBLISHED");
    Ok(FlowResult::of(flow, true))
}

/// `readDraft`.
async fn read_draft(g: &Graph<'_>, flow_id: &str) -> Meta<FlowResult> {
    let flow = get_flow(g, flow_id).await?;
    let published = status_of(&flow) == "PUBLISHED";
    Ok(FlowResult::of(flow, published))
}

/// `tryUpdateExisting`: `None` when the Flow cannot be reused.
async fn try_update_existing(
    g: &Graph<'_>,
    flow_id: &str,
    flow_json: &str,
    publish: bool,
) -> Meta<Option<FlowResult>> {
    let current = match get_flow(g, flow_id).await {
        Ok(flow) => Some(flow),
        // A draft save that cannot tell whether the Flow is live must not
        // create a new one in its place; only a Flow gone from Meta is replaced.
        Err(e) if publish || e.missing() => None,
        Err(e) => return Err(e),
    };
    if let Some(flow) = &current
        && !publish
        && LIVE.contains(&status_of(flow))
    {
        return Ok(Some(FlowResult {
            refused_live: true,
            ..FlowResult::of(flow.clone(), true)
        }));
    }
    if !current
        .as_ref()
        .is_some_and(|f| EDITABLE.contains(&status_of(f)))
    {
        return Ok(None);
    }
    match update_flow_json(g, flow_id, flow_json).await {
        Ok(update) if has_errors(&update) => {
            return Ok(Some(FlowResult::of(get_flow(g, flow_id).await?, false)));
        }
        Ok(_) => {}
        Err(e) => {
            // Older published Flows are immutable: fall back to a fresh Flow.
            tracing::warn!(flow_id, error = ?e.message, "Flow JSON update rejected, creating a new Flow");
            return Ok(None);
        }
    }
    let result = if publish {
        publish_draft_flow(g, flow_id).await?
    } else {
        read_draft(g, flow_id).await?
    };
    Ok(Some(result))
}

/// `publishFlowJson`.
async fn publish_flow_json(
    g: &Graph<'_>,
    name: &str,
    flow_json: &str,
    existing_flow_id: Option<&str>,
    publish: bool,
) -> Meta<FlowResult> {
    if let Some(flow_id) = existing_flow_id
        && let Some(updated) = try_update_existing(g, flow_id, flow_json, publish).await?
    {
        return Ok(updated);
    }
    let body = json!({ "name": name, "categories": ["OTHER"], "flow_json": flow_json });
    let created = send(
        http()
            .post(g.url(&format!("{}/flows", g.waba_id)))
            .json(&body),
        &g.token,
    )
    .await?;
    let Some(flow_id) = created.get("id").and_then(Value::as_str) else {
        return Err(MetaError::of("Meta did not return a Flow id"));
    };
    if has_errors(&created) {
        return Ok(FlowResult::of(get_flow(g, flow_id).await?, false));
    }
    if publish {
        publish_draft_flow(g, flow_id).await
    } else {
        read_draft(g, flow_id).await
    }
}

/// `readFlowStatus`: `None` once the Flow no longer exists.
async fn read_flow_status(g: &Graph<'_>, flow_id: &str) -> Meta<Option<String>> {
    match get_flow(g, flow_id).await {
        Ok(flow) => Ok(Some(status_of(&flow).to_owned())),
        Err(e) if e.missing() => Ok(None),
        Err(e) => Err(e),
    }
}

async fn delete_draft_flow(g: &Graph<'_>, flow_id: &str) -> Meta<Value> {
    send(http().delete(g.url(flow_id)), &g.token).await
}

/// `deleteFlow`: a draft is deleted, a live Flow deprecated. Meta's refusal
/// codes pick the other call once (the status can change between the read
/// and the call).
async fn delete_flow(g: &Graph<'_>, flow_id: &str) -> Meta<(&'static str, Option<String>)> {
    let mut status = read_flow_status(g, flow_id).await?;
    let mut can_retry = true;
    loop {
        let Some(current) = status.clone() else {
            return Ok(("missing", None));
        };
        if current == "DRAFT" {
            match delete_draft_flow(g, flow_id).await {
                Ok(_) => return Ok(("deleted", None)),
                Err(e) if e.missing() => return Ok(("missing", None)),
                Err(e) if can_retry && e.code == Some(CANNOT_DELETE) => {
                    status = Some("PUBLISHED".to_owned());
                }
                Err(e) => return Err(e),
            }
        } else if LIVE.contains(&current.as_str()) {
            let url = g.url(&format!("{flow_id}/deprecate"));
            match send(http().post(url), &g.token).await {
                Ok(_) => return Ok(("deprecated", Some("DEPRECATED".to_owned()))),
                Err(e) if e.missing() => return Ok(("missing", None)),
                Err(e) if can_retry && e.code == Some(CANNOT_DEPRECATE) => {
                    status = read_flow_status(g, flow_id).await?;
                }
                Err(e) => return Err(e),
            }
        } else {
            return Ok(("skipped", Some(current)));
        }
        can_retry = false;
    }
}

/// `deleteFlowDraft`: deletes only a draft; any other status is `notDraft`.
async fn delete_flow_draft(g: &Graph<'_>, flow_id: &str) -> Meta<(&'static str, Option<String>)> {
    let Some(status) = read_flow_status(g, flow_id).await? else {
        return Ok(("missing", None));
    };
    if status != "DRAFT" {
        return Ok(("notDraft", Some(status)));
    }
    match delete_draft_flow(g, flow_id).await {
        Ok(_) => Ok(("deleted", None)),
        Err(e) if e.missing() => Ok(("missing", None)),
        Err(e) if e.code == Some(CANNOT_DELETE) => Ok(match read_flow_status(g, flow_id).await? {
            None => ("missing", None),
            Some(current) => ("notDraft", Some(current)),
        }),
        Err(e) => Err(e),
    }
}

// ---- rows --------------------------------------------------------------------

/// `whatsappFlowService.upsertFromMeta`: the local `WhatsappFlow` id.
fn upsert_whatsapp_flow(conn: &Connection, integration_id: i64, flow: &Value) -> Result<i64> {
    let text = |k: &str| {
        flow.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let list = |k: &str| {
        flow.get(k)
            .filter(|v| v.is_array())
            .cloned()
            .unwrap_or(json!([]))
    };
    Ok(conn.query_row(
        r#"INSERT INTO "WhatsappFlow" ("id", "createdAt", "updatedAt", "name", "integrationWhatsappId", "sourceId", "status", "categories", "validationErrors", "completedCount", "screens")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, '[]')
           ON CONFLICT ("integrationWhatsappId", "sourceId") DO UPDATE SET
             "updatedAt" = excluded."updatedAt", "name" = excluded."name", "status" = excluded."status",
             "categories" = excluded."categories", "validationErrors" = excluded."validationErrors"
           RETURNING "id""#,
        params![
            new_id(),
            now_ms(),
            text("name"),
            integration_id,
            text("id"),
            text("status"),
            list("categories").to_string(),
            list("validation_errors").to_string()
        ],
        |r| r.get(0),
    )?)
}

/// `whatsappFlowService.markRemovedOnMeta`: drops (status `None`) or updates
/// the workspace's copies of the Flow.
fn mark_removed_on_meta(
    conn: &Connection,
    ws: i64,
    source_id: &str,
    status: Option<&str>,
) -> Result<()> {
    let scope = r#""sourceId" = ?1 AND "integrationWhatsappId" IN (SELECT "id" FROM "IntegrationWhatsapp" WHERE "workspaceId" = ?2)"#;
    match status {
        None => conn.execute(
            &format!(r#"DELETE FROM "WhatsappFlow" WHERE {scope}"#),
            params![source_id, ws],
        )?,
        Some(status) => conn.execute(
            &format!(r#"UPDATE "WhatsappFlow" SET "status" = ?3, "updatedAt" = ?4 WHERE {scope}"#),
            params![source_id, ws, status, now_ms()],
        )?,
    };
    Ok(())
}

fn find_publication(
    conn: &Connection,
    mini_app_id: i64,
    integration_id: i64,
) -> Result<Option<MiniAppPublication>> {
    Ok(conn
        .query_row(
            &format!(
                r#"SELECT {} FROM "MiniAppPublication" WHERE "miniAppId" = ?1 AND "integrationWhatsappId" = ?2"#,
                MiniAppPublication::columns_sql(None)
            ),
            params![mini_app_id, integration_id],
            MiniAppPublication::from_row,
        )
        .optional()?)
}

struct Record<'a> {
    mini_app_id: i64,
    integration_id: i64,
    whatsapp_flow_id: Option<i64>,
    source_id: &'a str,
    status: &'a str,
    validation_errors: Value,
    published: bool,
}

/// `miniAppPublicationService.record` (one row per Mini App + number;
/// `publishedAt` moves only on a publish).
fn record(conn: &Connection, r: &Record<'_>) -> Result<MiniAppPublication> {
    let now = now_ms();
    conn.execute(
        r#"INSERT INTO "MiniAppPublication" ("id", "createdAt", "updatedAt", "miniAppId", "integrationWhatsappId", "whatsappFlowId", "sourceId", "status", "validationErrors", "publishedAt")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
           ON CONFLICT ("miniAppId", "integrationWhatsappId") DO UPDATE SET
             "updatedAt" = excluded."updatedAt", "whatsappFlowId" = excluded."whatsappFlowId",
             "sourceId" = excluded."sourceId", "status" = excluded."status",
             "validationErrors" = excluded."validationErrors",
             "publishedAt" = COALESCE(excluded."publishedAt", "MiniAppPublication"."publishedAt")"#,
        params![
            new_id(),
            now,
            r.mini_app_id,
            r.integration_id,
            r.whatsapp_flow_id,
            r.source_id,
            r.status,
            r.validation_errors.to_string(),
            r.published.then_some(now)
        ],
    )?;
    find_publication(conn, r.mini_app_id, r.integration_id)?
        .ok_or_else(|| BusinessError::not_found(NOT_ON_NUMBER))
}

// ---- service -----------------------------------------------------------------

/// One Mini App on one of the workspace's WhatsApp numbers.
#[derive(Debug, Clone, Copy)]
pub struct Target {
    pub workspace_id: i64,
    pub mini_app_id: i64,
    pub integration_whatsapp_id: i64,
}

/// A recorded publish attempt; `published` comes from the publish call itself.
#[derive(Debug, Clone)]
pub struct Published {
    pub publication: MiniAppPublication,
    pub published: bool,
}

async fn load_integration(db: &Db, t: Target) -> Result<IntegrationWhatsapp> {
    tx::read(db, move |conn| {
        Ok(
            chatbotx_db::repo::integration_whatsapp::find_by_id_for_workspace(
                conn,
                t.integration_whatsapp_id,
                t.workspace_id,
            )?,
        )
    })
    .await?
    .ok_or_else(|| BusinessError::not_found(NUMBER_NOT_FOUND))
}

/// `loadPublication`: throws when the Mini App was never pushed to the number.
async fn load_publication(db: &Db, t: Target) -> Result<MiniAppPublication> {
    let app = crate::mini_app::find_or_fail(db, t.workspace_id, t.mini_app_id).await?;
    let app_id = app.mini_app.id;
    tx::read(db, move |conn| {
        find_publication(conn, app_id, t.integration_whatsapp_id)
    })
    .await?
    .ok_or_else(|| BusinessError::not_found(NOT_ON_NUMBER))
}

/// `recordFlowResult`: mirrors a Meta result into `WhatsappFlow` and the publication.
async fn record_flow_result(
    db: &Db,
    mini_app_id: i64,
    integration_id: i64,
    result: FlowResult,
) -> Result<Published> {
    let published = result.published;
    let publication = tx::write(db, move |conn| {
        let flow_id = upsert_whatsapp_flow(conn, integration_id, &result.flow)?;
        let mut errors = result
            .flow
            .get("validation_errors")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // A refused publish has no validation error; surface Meta's reason the same way.
        if let Some(message) = &result.publish_error {
            errors.push(json!({ "message": message }));
        }
        record(
            conn,
            &Record {
                mini_app_id,
                integration_id,
                whatsapp_flow_id: Some(flow_id),
                source_id: result
                    .flow
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                status: status_of(&result.flow),
                validation_errors: Value::Array(errors),
                published: result.published,
            },
        )
    })
    .await?;
    Ok(Published {
        publication,
        published,
    })
}

/// `assertPublishable`: no validation errors and no `{{variables}}`.
fn assert_publishable(definition: &Value) -> Result<()> {
    let validation = app::validate(definition);
    if !validation.valid {
        let errors = validation
            .issues
            .iter()
            .filter(|i| i.severity == "error")
            .count();
        return Err(BusinessError::Exception {
            message: "Fix the Mini App errors before publishing".to_owned(),
            code: "validation".to_owned(),
            status: 422,
            field: Some("definition".to_owned()),
            data: Some(json!({ "errors": errors })),
        });
    }
    // ponytail: `hasContactVariables` = the validator's `contact_variable_whatsapp`
    // warning (it checks the same screen titles and node texts).
    if validation
        .issues
        .iter()
        .any(|i| i.code == "contact_variable_whatsapp")
    {
        return Err(BusinessError::validation("definition", CONTACT_VARIABLES));
    }
    Ok(())
}

/// `publishMiniAppToWhatsapp`: creates or updates the Flow; `publish: false`
/// keeps it a draft on Meta (refused while the Flow is live there).
pub async fn publish(db: &Db, graph_url: &str, t: Target, publish: bool) -> Result<Published> {
    let found = crate::mini_app::find_or_fail(db, t.workspace_id, t.mini_app_id).await?;
    let app = found.mini_app;
    assert_publishable(&app.definition.0)?;
    let integration = load_integration(db, t).await?;
    let (app_id, integration_id) = (app.id, integration.id);
    let existing = tx::read(db, move |conn| {
        find_publication(conn, app_id, integration_id)
    })
    .await?;
    let g = Graph::new(graph_url, &integration);
    let result = publish_flow_json(
        &g,
        &app.name,
        &app.flow_json.0.to_string(),
        existing.as_ref().map(|p| p.source_id.as_str()),
        publish,
    )
    .await
    .map_err(MetaError::into_business)?;
    if result.refused_live {
        // Nothing changed on Meta; bring the stored status in line (without
        // moving `publishedAt`) before refusing the draft save.
        let result = FlowResult {
            published: false,
            ..result
        };
        record_flow_result(db, app_id, integration_id, result).await?;
        return Err(BusinessError::validation("publish", FLOW_IS_LIVE));
    }
    record_flow_result(db, app_id, integration_id, result).await
}

/// `publishMiniAppWhatsappDraft`: publishes the draft already on Meta.
pub async fn publish_draft(db: &Db, graph_url: &str, t: Target) -> Result<Published> {
    let publication = load_publication(db, t).await?;
    let integration = load_integration(db, t).await?;
    let g = Graph::new(graph_url, &integration);
    let result = publish_draft_flow(&g, &publication.source_id)
        .await
        .map_err(MetaError::into_business)?;
    record_flow_result(
        db,
        publication.mini_app_id,
        publication.integration_whatsapp_id,
        result,
    )
    .await
}

/// `getMiniAppWhatsappPreview`: Meta's temporary web preview link.
pub async fn preview(db: &Db, graph_url: &str, t: Target) -> Result<Value> {
    let publication = load_publication(db, t).await?;
    let integration = load_integration(db, t).await?;
    let g = Graph::new(graph_url, &integration);
    let request = http()
        .get(g.url(&publication.source_id))
        .query(&[("fields", "preview.invalidate(false)")]);
    let body = send(request, &g.token)
        .await
        .map_err(MetaError::into_business)?;
    let field = |k: &str| {
        body.pointer(&format!("/preview/{k}"))
            .and_then(Value::as_str)
    };
    Ok(json!({ "previewUrl": field("preview_url"), "expiresAt": field("expires_at") }))
}

/// `deleteMiniAppWhatsappDraft`: deletes a draft Flow and forgets it; a Flow
/// live on Meta is refused and left serving.
pub async fn delete_draft(db: &Db, graph_url: &str, t: Target) -> Result<&'static str> {
    let publication = load_publication(db, t).await?;
    let integration = load_integration(db, t).await?;
    let g = Graph::new(graph_url, &integration);
    let (outcome, status) = delete_flow_draft(&g, &publication.source_id)
        .await
        .map_err(MetaError::into_business)?;
    let ws = t.workspace_id;
    tx::write(db, move |conn| {
        mark_removed_on_meta(conn, ws, &publication.source_id, status.as_deref())?;
        if outcome == "notDraft" {
            record(
                conn,
                &Record {
                    mini_app_id: publication.mini_app_id,
                    integration_id: publication.integration_whatsapp_id,
                    whatsapp_flow_id: publication.whatsapp_flow_id,
                    source_id: &publication.source_id,
                    status: status.as_deref().unwrap_or(&publication.status),
                    validation_errors: json!([]),
                    published: false,
                },
            )?;
        } else {
            conn.execute(
                r#"DELETE FROM "MiniAppPublication" WHERE "miniAppId" = ?1 AND "integrationWhatsappId" = ?2"#,
                params![publication.mini_app_id, publication.integration_whatsapp_id],
            )?;
        }
        Ok(())
    })
    .await?;
    if outcome == "notDraft" {
        return Err(BusinessError::validation(
            "integrationWhatsappId",
            ONLY_DRAFT_DELETABLE,
        ));
    }
    Ok(outcome)
}

/// `removeFlow` of `deleteMiniApps` for each publication (read before the
/// Mini Apps were deleted): a draft is deleted, a live Flow deprecated. A
/// refusal is reported (`failed`), never thrown.
pub async fn remove_flows(
    db: &Db,
    graph_url: &str,
    ws: i64,
    publications: Vec<MiniAppPublication>,
) -> Vec<Value> {
    let ids: std::collections::HashSet<i64> = publications
        .iter()
        .map(|p| p.integration_whatsapp_id)
        .collect();
    let mut numbers = std::collections::HashMap::new();
    for integration_whatsapp_id in ids {
        let t = Target {
            workspace_id: ws,
            mini_app_id: 0,
            integration_whatsapp_id,
        };
        numbers.insert(integration_whatsapp_id, load_integration(db, t).await.ok());
    }
    let numbers = &numbers;
    futures::stream::iter(publications)
        .map(|p| async move {
            let mut out = json!({
                "miniAppId": p.mini_app_id.to_string(),
                "integrationWhatsappId": p.integration_whatsapp_id.to_string(),
                "flowId": p.source_id,
            });
            let Some(Some(integration)) = numbers.get(&p.integration_whatsapp_id) else {
                out["outcome"] = json!("failed");
                out["error"] = json!(NUMBER_NOT_CONNECTED);
                return out;
            };
            let g = Graph::new(graph_url, integration);
            match delete_flow(&g, &p.source_id).await {
                Ok((outcome, status)) => {
                    let source = p.source_id.clone();
                    let marked = tx::write(db, move |conn| mark_removed_on_meta(conn, ws, &source, status.as_deref())).await;
                    if let Err(error) = marked {
                        // Meta already did it; the next Flows sync repairs the local copy.
                        tracing::warn!(%error, "Local WhatsApp Flow copy not updated");
                    }
                    out["outcome"] = json!(outcome);
                }
                Err(e) => {
                    tracing::warn!(error = ?e.message, "WhatsApp Flow not removed while deleting its Mini App");
                    out["outcome"] = json!("failed");
                    out["error"] = json!(e.text());
                }
            }
            out
        })
        .buffered(REMOVAL_CONCURRENCY)
        .collect()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_flow_is_code_100_with_does_not_exist() {
        let e = |code, m: &str| MetaError {
            code: Some(code),
            message: Some(m.to_owned()),
            ..MetaError::default()
        };
        assert!(e(100, "Object with ID '1' Does Not Exist").missing());
        assert!(!e(100, "Invalid parameter").missing());
        assert!(!e(139_004, "does not exist").missing());
    }

    #[test]
    fn meta_errors_are_bad_request_with_metas_status() {
        let error = MetaError {
            status: 403,
            message: Some("no permission".to_owned()),
            ..MetaError::default()
        }
        .into_business();
        assert_eq!(error.code(), Some("BAD_REQUEST"));
        assert_eq!(error.status(), Some(403));
    }
}
