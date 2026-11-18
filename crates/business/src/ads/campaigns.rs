//! Messaging-ad campaigns (`messaging-ads/service.ts`
//! `messagingAdCampaignService`): the durable operation record, its Graph
//! create chain (campaign -> ad set -> creative -> ad, all PAUSED), publish /
//! pause / delete, the list with Meta's `effective_status` and the insights;
//! plus the wizard reads and uploads (`getAdAccountDetails`, ad videos,
//! `createAdsCreativeImageUpload`, `media-preflight.ts`,
//! `resolve-channel-assets.ts`).
//!
//! Auth is the channel integration's `MessagingAdsConnection`
//! ([`graph::messaging_auth`]); a Graph 190 marks it invalid. Graph errors
//! are `BAD_REQUEST` with Meta's message, tokens redacted
//! ([`crate::marketing_messages::graph`]). ponytail: the cached reads use the
//! in-process TTL cache of [`graph::cached`] (no stale-while-revalidate).

use std::time::Duration;

use chatbotx_core::utils::channel_safe_image::sniff_format;
use chatbotx_core::utils::storage::get_public_file_url;
use chatbotx_db::Db;
use chatbotx_db::repo::messaging_ad_operation::{
    self as ops, CreateMessagingAdOperationInput, ListOperationsInput, UpdateCreateProgressInput,
};
use chatbotx_db::repo::messaging_ads_connection::IntegrationRef;
use chatbotx_db::repo::models::MessagingAdOperation;
use chatbotx_db::repo::{Json, iso, new_id};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{OptionalExtension, params};
use chatbotx_files::Uploader;
use serde_json::{Map, Value, json};

use super::graph::{self, AdsAuth};
use crate::connection::credentials::send_bytes;
use crate::errors::{BusinessError, Result, is_query_dump, sanitize_public_text};
use crate::marketing_messages::graph::{self as g, GraphError, seg};
use crate::tx;

const GENERIC_ERROR: &str = "Could not create the ad. Please try again.";
/// `MESSAGING_AD_CREATIVE_UPLOAD_KIND`.
pub const CREATIVE_UPLOAD_KIND: &str = "adsCampaignCreative";
/// `MAX_MESSAGING_AD_IMAGE_BYTES`.
pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
/// `ADS_STATUS_STALE_AFTER_SECONDS`.
const STATUS_TTL: Duration = Duration::from_secs(20);
/// `AD_ACCOUNT_DETAILS_STALE_AFTER_SECONDS`.
const DETAILS_TTL: Duration = Duration::from_secs(5 * 60);
/// `ADS_INSIGHTS_STALE_AFTER_SECONDS`.
const INSIGHTS_TTL: Duration = Duration::from_secs(60);
/// `MESSAGING_CONVERSATION_STARTED_ACTION_TYPE_BY_CHANNEL` (same for all).
const CONVERSATION_STARTED: &str = "onsite_conversion.messaging_conversation_started_7d";
/// `RESTRICTED_SPECIAL_AD_CATEGORIES`.
const RESTRICTED: &[&str] = &[
    "HOUSING",
    "EMPLOYMENT",
    "CREDIT",
    "FINANCIAL_PRODUCTS_SERVICES",
];
/// `MAX_WELCOME_MESSAGE_TEMPLATES`.
const MAX_TEMPLATES: usize = 5;

/// `buildMessagingAdCreativeStoragePrefix`.
pub fn creative_prefix(workspace_id: i64) -> String {
    format!("public/space/{workspace_id}/ads-campaign/creatives")
}

/// One channel integration of a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    pub workspace_id: i64,
    pub channel: String,
    pub integration_id: i64,
}

impl Ident {
    fn reference(&self) -> IntegrationRef {
        graph::integration_ref(self.workspace_id, &self.channel, self.integration_id)
    }
    /// `scopeOf`; every cache key ends with it so [`graph::invalidate`]
    /// drops it.
    fn scope(&self) -> String {
        format!(
            "{}:{}:{}",
            self.workspace_id, self.channel, self.integration_id
        )
    }
    fn invalidate(&self) {
        graph::invalidate(self.workspace_id, &self.channel, self.integration_id);
    }
    fn of(record: &MessagingAdOperation) -> Result<Ident> {
        let integration_id = record
            .integration_whatsapp_id
            .or(record.integration_messenger_id)
            .or(record.integration_instagram_id)
            .ok_or_else(|| graph::internal("Messaging ad operation has no integration id"))?;
        Ok(Ident {
            workspace_id: record.workspace_id,
            channel: record.channel.clone(),
            integration_id,
        })
    }
}

/// A JS number (`10`, not `10.0`).
fn js_number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// `toPublicErrorMessage(error, fallback)`.
pub fn public_message(error: &BusinessError, fallback: &str) -> String {
    match error {
        BusinessError::Exception { message, code, .. }
            if code != "INTERNAL_SERVER_ERROR" && !is_query_dump(message) =>
        {
            let text = sanitize_public_text(message);
            if text.is_empty() {
                fallback.to_owned()
            } else {
                text
            }
        }
        _ => fallback.to_owned(),
    }
}

/// A failed Graph call: a 190 marks the connection invalid
/// (`markInvalidOnTokenError` / `withTokenInvalidation`).
async fn graph_error(db: &Db, ident: &Ident, error: GraphError) -> BusinessError {
    if error.code == Some(g::INVALID_TOKEN) {
        graph::mark_invalid(db, ident.reference()).await;
    }
    BusinessError::from(error)
}

fn invalid_response() -> GraphError {
    GraphError::invalid("Invalid Graph response")
}

fn id_field(value: &Value) -> std::result::Result<String, GraphError> {
    value["id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(invalid_response)
}

/// `postJsonFields`: `access_token` + `fields` as the JSON body.
async fn post_fields(
    auth: &AdsAuth,
    path: &str,
    fields: Value,
) -> std::result::Result<Value, GraphError> {
    let mut body = Map::new();
    body.insert("access_token".into(), json!(auth.access_token));
    if let Value::Object(fields) = fields {
        body.extend(fields);
    }
    g::call(
        "POST",
        g::url(&format!("{}/{path}", auth.version), &[]),
        Some(&Value::Object(body)),
        None,
    )
    .await
}

async fn get(
    auth: &AdsAuth,
    path: &str,
    query: &[(&str, &str)],
) -> std::result::Result<Value, GraphError> {
    let mut q = query.to_vec();
    q.push(("access_token", auth.access_token.as_str()));
    let path = if path.is_empty() {
        auth.version.clone()
    } else {
        format!("{}/{path}", auth.version)
    };
    g::call("GET", g::url(&path, &q), None, None).await
}

/// A `Content-Disposition` value (no quotes or line breaks).
fn disposition_text(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\r' | '\n'))
        .collect::<String>()
        .replace('"', "%22")
}

/// `postForm`: one file part as `multipart/form-data`, the token in the query.
async fn post_file(
    auth: &AdsAuth,
    path: &str,
    field: &str,
    file_name: &str,
    mime_type: &str,
    bytes: &[u8],
) -> std::result::Result<Value, GraphError> {
    let boundary = format!("----chatbotx{}", new_id());
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
        disposition_text(field),
        disposition_text(file_name),
        disposition_text(mime_type),
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let url = g::url(
        &format!("{}/{path}", auth.version),
        &[("access_token", &auth.access_token)],
    );
    let content_type = format!("multipart/form-data; boundary={boundary}");
    g::parse_reply(send_bytes(&url, &content_type, body).await)
}

// ---- assets ---------------------------------------------------------------

/// `MessagingAdChannelAssets`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assets {
    pub page_id: String,
    pub instagram_actor_id: Option<String>,
    pub whatsapp_phone_number: Option<String>,
}

fn page_missing(message: &str) -> BusinessError {
    BusinessError::exception(message, "messagingAdPageMissing", 400)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

/// `resolveMessagingAdChannelAssets`.
pub async fn resolve_assets(db: &Db, ident: &Ident, whatsapp_page: Option<i64>) -> Result<Assets> {
    let ws = ident.workspace_id;
    let id = ident.integration_id;
    let messenger_page = move |conn: &chatbotx_db::rusqlite::Connection, id: i64| {
        conn.query_row(
            "SELECT \"pageId\" FROM \"IntegrationMessenger\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
            params![id, ws],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
    };
    match ident.channel.as_str() {
        "messenger" => {
            let row = tx::read(db, move |conn| Ok(messenger_page(conn, id)?)).await?;
            let page =
                row.ok_or_else(|| BusinessError::not_found("Messenger integration not found"))?;
            let page_id = non_empty(page).ok_or_else(|| {
                page_missing(
                    "This Messenger channel has no linked Facebook Page. Reconnect it before creating an ad.",
                )
            })?;
            Ok(Assets {
                page_id,
                ..Assets::default()
            })
        }
        "instagram" => {
            let row = tx::read(db, move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT \"pageId\", \"igId\" FROM \"IntegrationInstagram\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
                        params![id, ws],
                        |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
                    )
                    .optional()?)
            })
            .await?;
            let (page, ig) =
                row.ok_or_else(|| BusinessError::not_found("Instagram integration not found"))?;
            let page_id = non_empty(page).ok_or_else(|| {
                page_missing(
                    "This Instagram channel has no linked Facebook Page. Reconnect it before creating an ad.",
                )
            })?;
            let ig = non_empty(ig).ok_or_else(|| {
                BusinessError::exception(
                    "This Instagram channel is missing its Instagram professional account id. Reconnect it before creating an ad.",
                    "messagingAdInstagramActorMissing",
                    400,
                )
            })?;
            Ok(Assets {
                page_id,
                instagram_actor_id: Some(ig),
                whatsapp_phone_number: None,
            })
        }
        _ => {
            let page_integration = whatsapp_page.ok_or_else(|| {
                BusinessError::exception(
                    "A connected Messenger Page must be selected to supply page_id for a WhatsApp messaging ad",
                    "messagingAdWhatsappPageRequired",
                    400,
                )
            })?;
            let (phone, page) = tx::read(db, move |conn| {
                let phone = conn
                    .query_row(
                        "SELECT \"displayPhoneNumber\" FROM \"IntegrationWhatsapp\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
                        params![id, ws],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .optional()?;
                Ok((phone, messenger_page(conn, page_integration)?))
            })
            .await?;
            let phone =
                phone.ok_or_else(|| BusinessError::not_found("WhatsApp integration not found"))?;
            let page = page.ok_or_else(|| {
                BusinessError::not_found("Messenger (Page) integration not found")
            })?;
            let page_id = non_empty(page).ok_or_else(|| {
                page_missing(
                    "The selected Facebook Page is not fully connected. Reconnect it before creating an ad.",
                )
            })?;
            let phone = phone.unwrap_or_default();
            let phone = phone.strip_prefix('+').unwrap_or(&phone).to_owned();
            if phone.is_empty() {
                return Err(BusinessError::exception(
                    "This WhatsApp channel has no phone number configured. Reconnect it before creating an ad.",
                    "messagingAdWhatsappPhoneMissing",
                    400,
                ));
            }
            Ok(Assets {
                page_id,
                instagram_actor_id: None,
                whatsapp_phone_number: Some(phone),
            })
        }
    }
}

/// `listMessengerPages` (the WhatsApp wizard's Page selector).
pub async fn messenger_pages(db: &Db, workspace_id: i64) -> Result<Value> {
    let rows = tx::read(db, move |conn| {
        Ok(chatbotx_db::repo::integration_messenger::list_by_workspace_id(conn, workspace_id)?)
    })
    .await?;
    let data: Vec<Value> = rows
        .iter()
        .map(|r| json!({ "id": r.id.to_string(), "name": r.name, "pageId": r.page_id }))
        .collect();
    Ok(json!({ "data": data }))
}

// ---- payload mapping ------------------------------------------------------

/// `(destinationType, ctaType, needsInstagramActor)` of
/// `messagingAdConfigByChannel` (`ctaAppDestination` = `destinationType`).
fn channel_config(channel: &str) -> (&'static str, &'static str, bool) {
    match channel {
        "messenger" => ("MESSENGER", "MESSAGE_PAGE", false),
        "instagram" => ("INSTAGRAM_DIRECT", "INSTAGRAM_MESSAGE", true),
        _ => ("WHATSAPP", "WHATSAPP_MESSAGE", false),
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn present_array(value: &Value) -> bool {
    value.as_array().is_some_and(|a| !a.is_empty())
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.is_empty())
}

/// `buildSpecialAdCategoriesParam`.
fn special_categories_param(categories: &[String]) -> Vec<String> {
    let real: Vec<String> = categories
        .iter()
        .filter(|c| *c != "NONE")
        .cloned()
        .collect();
    if real.is_empty() {
        vec!["NONE".to_owned()]
    } else {
        real
    }
}

/// `mapTargeting` + `enforceSpecialAdCategoryTargeting`.
pub fn map_targeting(targeting: &Value, categories: &[String]) -> Value {
    let custom = !targeting["ageMin"].is_null()
        || !targeting["ageMax"].is_null()
        || present_array(&targeting["genders"])
        || present_array(&targeting["locales"]);
    let restricted = categories.iter().any(|c| RESTRICTED.contains(&c.as_str()));
    let mut out = json!({
        "geo_locations": { "countries": targeting["countries"] },
        "targeting_automation": { "advantage_audience": if custom || restricted { 0 } else { 1 } },
    });
    if !restricted {
        for (from, to) in [("ageMin", "age_min"), ("ageMax", "age_max")] {
            if !targeting[from].is_null() {
                out[to] = targeting[from].clone();
            }
        }
        if present_array(&targeting["genders"]) {
            out["genders"] = targeting["genders"].clone();
        }
    }
    if present_array(&targeting["locales"]) {
        out["locales"] = targeting["locales"].clone();
    }
    out
}

/// `mapCreativeMedia`: `link_data` or `video_data` (without the CTA).
pub fn map_media(media: &Value, image_hash: Option<&str>) -> (&'static str, Map<String, Value>) {
    let mut out = Map::new();
    if media["kind"] == "image" {
        out.insert("link".into(), media["link"].clone());
        let hash = text(&media["imageHash"]).or(image_hash).unwrap_or_default();
        out.insert("image_hash".into(), json!(hash));
        for (from, to) in [
            ("message", "message"),
            ("headline", "name"),
            ("description", "description"),
            ("caption", "caption"),
        ] {
            if let Some(value) = text(&media[from]) {
                out.insert(to.into(), json!(value));
            }
        }
        return ("link_data", out);
    }
    out.insert("video_id".into(), media["videoId"].clone());
    for (from, to) in [
        ("thumbnailImageHash", "image_hash"),
        ("title", "title"),
        ("message", "message"),
        ("linkDescription", "link_description"),
    ] {
        if let Some(value) = text(&media[from]) {
            out.insert(to.into(), json!(value));
        }
    }
    ("video_data", out)
}

fn quick_replies(value: &Value) -> Option<Value> {
    let replies = value.as_array().filter(|a| !a.is_empty())?;
    Some(Value::Array(
        replies
            .iter()
            .map(|r| json!({ "title": r["title"] }))
            .collect(),
    ))
}

fn welcome_payload(content: Map<String, Value>) -> Value {
    json!({
        "type": "VISUAL_EDITOR",
        "version": 1,
        "landing_screen_type": "welcome_message",
        "media_type": "text",
        "text_format": {
            "customer_action_type": "VISUAL_EDITOR_CUSTOMER_ACTIONS",
            "content": content,
        },
    })
}

/// `buildPageWelcomeMessage`: the `page_welcome_message` JSON string, `None`
/// for Meta's default.
pub fn welcome_message(input: &Value) -> Option<String> {
    match input["type"].as_str() {
        Some("single") => {
            let mut content = Map::new();
            content.insert("title".into(), input["message"].clone());
            if let Some(replies) = quick_replies(&input["quickReplies"]) {
                content.insert("quick_replies".into(), replies);
            }
            Some(welcome_payload(content).to_string())
        }
        Some("templates") => {
            let templates: Vec<Value> = input["templates"]
                .as_array()
                .map(|a| a.iter().take(MAX_TEMPLATES).cloned().collect())
                .unwrap_or_default();
            let payloads: Vec<Value> = templates
                .iter()
                .map(|t| {
                    let mut content = Map::new();
                    if let Some(heading) = text(&t["heading"]) {
                        content.insert("title".into(), json!(heading));
                    }
                    content.insert("message".into(), t["message"].clone());
                    if let Some(replies) = quick_replies(&t["quickReplies"]) {
                        content.insert("quick_replies".into(), replies);
                    }
                    welcome_payload(content)
                })
                .collect();
            match payloads.as_slice() {
                [one] => Some(one.to_string()),
                _ => Some(Value::Array(payloads).to_string()),
            }
        }
        _ => None,
    }
}

// ---- the operation record -------------------------------------------------

/// `toMessagingAdOperationResource` without `workspaceId`.
pub fn resource(record: &MessagingAdOperation, effective_status: Option<&str>) -> Value {
    json!({
        "id": record.id.to_string(),
        "channel": record.channel,
        "adAccountId": record.ad_account_id,
        "name": record.name,
        "createState": record.create_state,
        "publishState": record.publish_state,
        "metaCampaignId": record.meta_campaign_id,
        "metaAdSetId": record.meta_ad_set_id,
        "metaAdCreativeId": record.meta_ad_creative_id,
        "metaAdId": record.meta_ad_id,
        "lastError": record.last_error,
        "cleanupError": record.cleanup_error,
        "effectiveStatus": effective_status,
        "createdAt": iso(record.created_at),
    })
}

/// `CreateMessagingAdDraftInput`: the operation `input` is the validated
/// request (`MessagingAdOperationInput`: `adAccountId`,
/// `whatsappPageIntegrationId?`, `campaign`, `adSet`, `creative`).
#[derive(Debug, Clone)]
pub struct CreateDraft {
    pub ident: Ident,
    pub whatsapp_page_integration_id: Option<i64>,
    pub ad_account_id: String,
    pub name: String,
    pub input: Value,
}

async fn get_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<MessagingAdOperation> {
    tx::read(db, move |conn| {
        Ok(ops::find_by_id_for_workspace(conn, id, workspace_id)?)
    })
    .await?
    .ok_or_else(|| BusinessError::not_found("Messaging ad operation not found"))
}

async fn progress(
    db: &Db,
    record: &MessagingAdOperation,
    state: &str,
    meta: [Option<String>; 4],
    last_error: Option<String>,
) -> Result<MessagingAdOperation> {
    let [campaign, ad_set, creative, ad] = meta;
    let input = UpdateCreateProgressInput {
        id: record.id,
        workspace_id: record.workspace_id,
        create_state: state.to_owned(),
        meta_campaign_id: campaign,
        meta_ad_set_id: ad_set,
        meta_ad_creative_id: creative,
        meta_ad_id: ad,
        last_error,
    };
    let updated = tx::write(db, move |tx| Ok(ops::update_create_progress(tx, &input)?)).await?;
    Ok(updated.unwrap_or_else(|| record.clone()))
}

/// `createDraft`: the record is written before the connection check (a 409
/// leaves it `pending`).
pub async fn create_draft(
    db: &Db,
    uploader: Option<&Uploader>,
    draft: CreateDraft,
) -> Result<MessagingAdOperation> {
    let ident = draft.ident.clone();
    let assets = resolve_assets(db, &ident, draft.whatsapp_page_integration_id).await?;
    let reference = ident.reference();
    let row = CreateMessagingAdOperationInput {
        id: new_id(),
        workspace_id: ident.workspace_id,
        channel: ident.channel.clone(),
        integration_whatsapp_id: reference.integration_whatsapp_id,
        integration_messenger_id: reference.integration_messenger_id,
        integration_instagram_id: reference.integration_instagram_id,
        ad_account_id: draft.ad_account_id,
        name: draft.name,
        input: Json(draft.input),
        created_by: None,
    };
    let record = tx::write(db, move |tx| Ok(ops::create(tx, &row)?)).await?;
    let auth = graph::messaging_auth(db, reference).await?;
    let result = run_create_steps(db, uploader, &ident, &assets, record, &auth).await;
    ident.invalidate();
    result
}

/// `retryDraft`: resumes from the persisted Meta ids after a CAS
/// `failed -> pending`.
pub async fn retry_draft(
    db: &Db,
    uploader: Option<&Uploader>,
    workspace_id: i64,
    id: i64,
) -> Result<MessagingAdOperation> {
    let record = get_or_fail(db, workspace_id, id).await?;
    let ident = Ident::of(&record)?;
    let whatsapp_page = record.input.0["whatsappPageIntegrationId"]
        .as_str()
        .and_then(|s| s.parse().ok());
    let assets = resolve_assets(db, &ident, whatsapp_page).await?;
    let auth = graph::messaging_auth(db, ident.reference()).await?;
    let claimed = tx::write(db, move |tx| {
        Ok(ops::claim_for_retry(tx, id, workspace_id)?)
    })
    .await?
    .ok_or_else(|| {
        BusinessError::conflict(
            "This ad is already being retried or is not in a retryable state.",
            "messagingAdNotRetryable",
        )
    })?;
    let result = run_create_steps(db, uploader, &ident, &assets, claimed, &auth).await;
    ident.invalidate();
    result
}

/// `runCreateSteps`: any failure marks the record `failed` with the public
/// message (progress is kept in the Meta ids).
async fn run_create_steps(
    db: &Db,
    uploader: Option<&Uploader>,
    ident: &Ident,
    assets: &Assets,
    record: MessagingAdOperation,
    auth: &AdsAuth,
) -> Result<MessagingAdOperation> {
    let fallback = record.clone();
    match create_steps(db, uploader, ident, assets, record, auth).await {
        Ok(record) => Ok(record),
        Err(error) => {
            let message = public_message(&error, GENERIC_ERROR);
            progress(db, &fallback, "failed", Default::default(), Some(message)).await?;
            Err(error)
        }
    }
}

async fn create_steps(
    db: &Db,
    uploader: Option<&Uploader>,
    ident: &Ident,
    assets: &Assets,
    mut record: MessagingAdOperation,
    auth: &AdsAuth,
) -> Result<MessagingAdOperation> {
    let input = record.input.0.clone();
    let media = &input["creative"]["media"];
    let image = if media["kind"] == "image" && media["imageKey"].is_string() {
        Some(resolve_stored_image(db, uploader, record.workspace_id, media).await?)
    } else {
        None
    };
    let account = seg(&record.ad_account_id);
    let categories = strings(&input["campaign"]["specialAdCategories"]);
    let (destination, cta, needs_actor) = channel_config(&ident.channel);

    if record.meta_campaign_id.is_none() {
        let mut fields = json!({
            "name": record.name,
            "objective": "OUTCOME_ENGAGEMENT",
            "buying_type": "AUCTION",
            "special_ad_categories": special_categories_param(&categories),
        });
        if present_array(&input["campaign"]["specialAdCategoryCountry"]) {
            fields["special_ad_category_country"] =
                input["campaign"]["specialAdCategoryCountry"].clone();
        }
        fields["is_adset_budget_sharing_enabled"] = json!(false);
        fields["status"] = json!("PAUSED");
        let id = match post_fields(auth, &format!("{account}/campaigns"), fields).await {
            Ok(answer) => id_field(&answer),
            Err(e) => Err(e),
        };
        let id = match id {
            Ok(id) => id,
            Err(e) => return Err(graph_error(db, ident, e).await),
        };
        record = progress(
            db,
            &record,
            "campaignCreated",
            [Some(id), None, None, None],
            None,
        )
        .await?;
    }

    if record.meta_ad_set_id.is_none() {
        let ad_set = &input["adSet"];
        let mut promoted = json!({ "page_id": assets.page_id });
        if ident.channel == "whatsapp" {
            promoted["whatsapp_phone_number"] = json!(assets.whatsapp_phone_number);
        }
        let mut fields = json!({
            "name": record.name,
            "campaign_id": record.meta_campaign_id,
            "daily_budget": ad_set["dailyBudgetMinorUnits"],
            "billing_event": "IMPRESSIONS",
            "optimization_goal": "CONVERSATIONS",
            "bid_strategy": "LOWEST_COST_WITHOUT_CAP",
            "destination_type": destination,
            "promoted_object": promoted,
            "targeting": map_targeting(&ad_set["targeting"], &categories),
            "status": "PAUSED",
        });
        for (from, to) in [("startTime", "start_time"), ("endTime", "end_time")] {
            if let Some(time) = text(&ad_set[from]) {
                fields[to] = json!(time);
            }
        }
        let id = match post_fields(auth, &format!("{account}/adsets"), fields).await {
            Ok(answer) => id_field(&answer),
            Err(e) => Err(e),
        };
        let id = match id {
            Ok(id) => id,
            Err(e) => return Err(graph_error(db, ident, e).await),
        };
        record = progress(
            db,
            &record,
            "adSetCreated",
            [None, Some(id), None, None],
            None,
        )
        .await?;
    }

    if record.meta_ad_creative_id.is_none() {
        let image_hash = match &image {
            Some(image) => {
                match upload_image(
                    auth,
                    &record.ad_account_id,
                    &image.file_name,
                    image.mime_type,
                    &image.bytes,
                )
                .await
                {
                    Ok(hash) => Some(hash),
                    Err(e) => return Err(graph_error(db, ident, e).await),
                }
            }
            None => None,
        };
        let (media_key, mut media_fields) = map_media(media, image_hash.as_deref());
        media_fields.insert(
            "call_to_action".into(),
            json!({ "type": cta, "value": { "app_destination": destination } }),
        );
        if let Some(welcome) = welcome_message(&input["creative"]["welcomeMessage"]) {
            media_fields.insert("page_welcome_message".into(), json!(welcome));
        }
        let mut spec = Map::new();
        spec.insert("page_id".into(), json!(assets.page_id));
        if needs_actor && let Some(actor) = &assets.instagram_actor_id {
            spec.insert("instagram_actor_id".into(), json!(actor));
        }
        spec.insert(media_key.into(), Value::Object(media_fields));
        let fields = json!({ "name": record.name, "object_story_spec": spec });
        let id = match post_fields(auth, &format!("{account}/adcreatives"), fields).await {
            Ok(answer) => id_field(&answer),
            Err(e) => Err(e),
        };
        let id = match id {
            Ok(id) => id,
            Err(e) => return Err(graph_error(db, ident, e).await),
        };
        record = progress(
            db,
            &record,
            "creativeCreated",
            [None, None, Some(id), None],
            None,
        )
        .await?;
    }

    if record.meta_ad_id.is_none() {
        let fields = json!({
            "name": record.name,
            "adset_id": record.meta_ad_set_id,
            "creative": { "creative_id": record.meta_ad_creative_id },
            "status": "PAUSED",
        });
        let id = match post_fields(auth, &format!("{account}/ads"), fields).await {
            Ok(answer) => id_field(&answer),
            Err(e) => Err(e),
        };
        let id = match id {
            Ok(id) => id,
            Err(e) => return Err(graph_error(db, ident, e).await),
        };
        record = progress(db, &record, "adCreated", [None, None, None, Some(id)], None).await?;
    }
    Ok(record)
}

/// `uploadAdImage`: the `image_hash` of the uploaded file.
async fn upload_image(
    auth: &AdsAuth,
    ad_account_id: &str,
    file_name: &str,
    mime_type: &str,
    bytes: &[u8],
) -> std::result::Result<String, GraphError> {
    let answer = post_file(
        auth,
        &format!("{}/adimages", seg(ad_account_id)),
        file_name,
        file_name,
        mime_type,
        bytes,
    )
    .await?;
    answer["images"][file_name]["hash"]
        .as_str()
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            GraphError::invalid(&format!(
                "Facebook adimages response did not include a hash for filename \"{file_name}\""
            ))
        })
}

/// `ResolvedStoredImage`.
struct StoredImage {
    bytes: Vec<u8>,
    mime_type: &'static str,
    file_name: String,
}

fn preflight(message: &str) -> BusinessError {
    BusinessError::exception(message, "invalidRequest", 400)
}

/// `resolveStoredImageBytes`: the key is in the workspace's prefix, its
/// pending `File` row matches, the object is at most 10 MB and is a JPEG /
/// PNG / GIF / WebP by its bytes.
async fn resolve_stored_image(
    db: &Db,
    uploader: Option<&Uploader>,
    workspace_id: i64,
    media: &Value,
) -> Result<StoredImage> {
    let key = media["imageKey"].as_str().unwrap_or_default().to_owned();
    if !key.starts_with(&format!("{}/", creative_prefix(workspace_id))) {
        return Err(preflight("This image is not owned by this workspace."));
    }
    let file_id: Option<i64> = media["fileId"].as_str().and_then(|s| s.parse().ok());
    let file = match file_id {
        Some(id) => {
            tx::read(db, move |conn| {
                Ok(chatbotx_db::repo::file::find_by_id_for_workspace(
                    conn,
                    id,
                    workspace_id,
                    None,
                )?)
            })
            .await?
        }
        None => None,
    };
    let verified = file
        .as_ref()
        .is_some_and(|f| f.path == key && f.sub_type.as_deref() == Some(CREATIVE_UPLOAD_KIND));
    if !verified {
        return Err(preflight("This image upload could not be verified."));
    }
    let uploader =
        uploader.ok_or_else(|| preflight("This image could not be found in storage."))?;
    let head = uploader
        .head_object(&key)
        .await
        .map_err(|_| preflight("This image could not be found in storage."))?;
    if head.content_length == 0 || head.content_length > MAX_IMAGE_BYTES {
        return Err(preflight("This image exceeds the maximum allowed size."));
    }
    let bytes = uploader
        .get_object(&key)
        .await
        .map_err(|_| preflight("This image could not be found in storage."))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(preflight("This image exceeds the maximum allowed size."));
    }
    let (mime_type, extension) = match sniff_format(&bytes) {
        Some("jpeg") => ("image/jpeg", "jpg"),
        Some("png") => ("image/png", "png"),
        Some("gif") => ("image/gif", "gif"),
        Some("webp") => ("image/webp", "webp"),
        _ => return Err(preflight("This file is not a supported image.")),
    };
    Ok(StoredImage {
        bytes: bytes.to_vec(),
        mime_type,
        file_name: format!("{}.{extension}", file_id.unwrap_or_default()),
    })
}

// ---- status changes -------------------------------------------------------

/// `setStatus`: `POST /<id>` with `{ status }`.
async fn set_status(db: &Db, ident: &Ident, auth: &AdsAuth, id: &str, status: &str) -> Result<()> {
    match post_fields(auth, &seg(id), json!({ "status": status })).await {
        Ok(_) => Ok(()),
        Err(e) => Err(graph_error(db, ident, e).await),
    }
}

/// `safeSetStatus`: a failure is pushed as `level(id): message`.
async fn safe_set_status(
    db: &Db,
    ident: &Ident,
    auth: &AdsAuth,
    level: &str,
    id: &str,
    status: &str,
    errors: &mut Vec<String>,
) {
    if let Err(error) = set_status(db, ident, auth, id, status).await {
        errors.push(format!(
            "{level}({id}): {}",
            public_message(&error, GENERIC_ERROR)
        ));
    }
}

async fn set_publish_state(
    db: &Db,
    record: &MessagingAdOperation,
    state: &'static str,
    last_error: Option<String>,
) -> Result<MessagingAdOperation> {
    let (id, ws) = (record.id, record.workspace_id);
    let updated = tx::write(db, move |tx| {
        Ok(ops::update_publish_state(
            tx,
            id,
            ws,
            state,
            last_error.as_deref(),
        )?)
    })
    .await?;
    Ok(updated.unwrap_or_else(|| record.clone()))
}

async fn set_cleanup_error(
    db: &Db,
    record: &MessagingAdOperation,
    errors: &[String],
) -> Result<()> {
    let (id, ws) = (record.id, record.workspace_id);
    let text = (!errors.is_empty()).then(|| errors.join("; "));
    tx::write(db, move |tx| {
        Ok(ops::set_cleanup_error(tx, id, ws, text.as_deref())?)
    })
    .await
}

/// `publish`: campaign -> ad set -> ad to ACTIVE; a refused level pauses the
/// levels already activated (failures in `cleanupError`) and answers
/// `publishFailed`.
pub async fn publish(db: &Db, workspace_id: i64, id: i64) -> Result<MessagingAdOperation> {
    let record = get_or_fail(db, workspace_id, id).await?;
    let (Some(campaign), Some(ad_set), Some(ad)) = (
        record.meta_campaign_id.clone(),
        record.meta_ad_set_id.clone(),
        record.meta_ad_id.clone(),
    ) else {
        return Err(BusinessError::conflict(
            "Cannot publish before campaign/ad set/ad have all been created",
            "messagingAdNotPublishable",
        ));
    };
    let ident = Ident::of(&record)?;
    let auth = graph::messaging_auth(db, ident.reference()).await?;
    set_publish_state(db, &record, "publishing", record.last_error.clone()).await?;

    let mut compensation = Vec::new();
    let mut outcome = set_status(db, &ident, &auth, &campaign, "ACTIVE").await;
    if outcome.is_ok() {
        outcome = set_status(db, &ident, &auth, &ad_set, "ACTIVE").await;
        if outcome.is_err() {
            safe_set_status(
                db,
                &ident,
                &auth,
                "campaign",
                &campaign,
                "PAUSED",
                &mut compensation,
            )
            .await;
        } else {
            outcome = set_status(db, &ident, &auth, &ad, "ACTIVE").await;
            if outcome.is_err() {
                safe_set_status(
                    db,
                    &ident,
                    &auth,
                    "adSet",
                    &ad_set,
                    "PAUSED",
                    &mut compensation,
                )
                .await;
                safe_set_status(
                    db,
                    &ident,
                    &auth,
                    "campaign",
                    &campaign,
                    "PAUSED",
                    &mut compensation,
                )
                .await;
            }
        }
    }
    let updated = match outcome {
        Err(error) => {
            if !compensation.is_empty() {
                set_cleanup_error(db, &record, &compensation).await?;
            }
            let message = public_message(&error, GENERIC_ERROR);
            set_publish_state(db, &record, "publishFailed", Some(message)).await?
        }
        Ok(()) => {
            if record.cleanup_error.is_some() {
                set_cleanup_error(db, &record, &[]).await?;
            }
            set_publish_state(db, &record, "published", record.last_error.clone()).await?
        }
    };
    ident.invalidate();
    Ok(updated)
}

/// `pause` (best effort, ad -> ad set -> campaign).
pub async fn pause(db: &Db, workspace_id: i64, id: i64) -> Result<MessagingAdOperation> {
    let record = get_or_fail(db, workspace_id, id).await?;
    let ident = Ident::of(&record)?;
    let auth = graph::messaging_auth(db, ident.reference()).await?;
    set_publish_state(db, &record, "pausing", record.last_error.clone()).await?;
    let errors = set_all(db, &ident, &auth, &record, "PAUSED").await;
    if let Err(error) = set_cleanup_error(db, &record, &errors).await {
        tracing::warn!(?error, "failed to record the messaging-ad pause errors");
    }
    let last_error = (!errors.is_empty()).then(|| errors.join("; "));
    let updated = set_publish_state(db, &record, "paused", last_error).await?;
    ident.invalidate();
    Ok(updated)
}

async fn set_all(
    db: &Db,
    ident: &Ident,
    auth: &AdsAuth,
    record: &MessagingAdOperation,
    status: &str,
) -> Vec<String> {
    let mut errors = Vec::new();
    for (level, id) in [
        ("ad", &record.meta_ad_id),
        ("adSet", &record.meta_ad_set_id),
        ("campaign", &record.meta_campaign_id),
    ] {
        if let Some(id) = id {
            safe_set_status(db, ident, auth, level, id, status, &mut errors).await;
        }
    }
    errors
}

/// `deleteOperation`: every level to DELETED and the stored creative image
/// removed (best effort); the record stays (`deleting` while a level failed).
pub async fn delete(
    db: &Db,
    uploader: Option<&Uploader>,
    workspace_id: i64,
    id: i64,
) -> Result<MessagingAdOperation> {
    let record = get_or_fail(db, workspace_id, id).await?;
    let ident = Ident::of(&record)?;
    let auth = graph::messaging_auth(db, ident.reference()).await?;
    set_publish_state(db, &record, "deleting", record.last_error.clone()).await?;
    let mut errors = set_all(db, &ident, &auth, &record, "DELETED").await;
    let media = &record.input.0["creative"]["media"];
    if media["kind"] == "image"
        && let Some(key) = media["imageKey"].as_str()
    {
        let deleted = match uploader {
            Some(uploader) => uploader.delete_object(key).await.is_ok(),
            None => false,
        };
        if !deleted {
            errors.push(format!(
                "image({key}): Could not delete the stored creative image."
            ));
        }
    }
    set_cleanup_error(db, &record, &errors).await?;
    let state = if errors.is_empty() {
        "deleted"
    } else {
        "deleting"
    };
    let updated = set_publish_state(db, &record, state, record.last_error.clone()).await?;
    ident.invalidate();
    Ok(updated)
}

// ---- reads ------------------------------------------------------------------

async fn rows_of(db: &Db, ident: &Ident) -> Result<Vec<MessagingAdOperation>> {
    let reference = ident.reference();
    let input = ListOperationsInput {
        workspace_id: ident.workspace_id,
        channel: Some(ident.channel.clone()),
        integration_whatsapp_id: reference.integration_whatsapp_id,
        integration_messenger_id: reference.integration_messenger_id,
        integration_instagram_id: reference.integration_instagram_id,
    };
    tx::read(db, move |conn| Ok(ops::list_by_workspace_id(conn, &input)?)).await
}

/// A cached Graph read of one integration (`forceRefresh` drops the key).
async fn cached_read<F, Fut>(
    db: &Db,
    ident: &Ident,
    key: String,
    ttl: Duration,
    force: bool,
    fetch: F,
) -> Result<Value>
where
    F: FnOnce(AdsAuth) -> Fut,
    Fut: Future<Output = std::result::Result<Value, GraphError>>,
{
    if force {
        graph::forget(&key);
    }
    graph::cached(key, ttl, || async {
        let auth = graph::messaging_auth(db, ident.reference()).await?;
        match fetch(auth).await {
            Ok(value) => Ok(value),
            Err(e) => Err(graph_error(db, ident, e).await),
        }
    })
    .await
}

/// `list`: the integration's operations, newest first, with the ad's live
/// `effective_status` (`null` when the read fails).
pub async fn list(db: &Db, ident: &Ident, force: bool) -> Result<Value> {
    let rows = rows_of(db, ident).await?;
    let mut ad_ids: Vec<String> = rows.iter().filter_map(|r| r.meta_ad_id.clone()).collect();
    let statuses = if ad_ids.is_empty() {
        Value::Null
    } else {
        ad_ids.sort();
        let ids = ad_ids.join(",");
        let key = format!("msgads:ads-status:{ids}:{}", ident.scope());
        cached_read(db, ident, key, STATUS_TTL, force, |auth| async move {
            get(
                &auth,
                "",
                &[
                    ("ids", ids.as_str()),
                    ("fields", "id,name,status,effective_status"),
                ],
            )
            .await
        })
        .await
        .unwrap_or(Value::Null)
    };
    let data: Vec<Value> = rows
        .iter()
        .map(|r| {
            let status = r
                .meta_ad_id
                .as_ref()
                .and_then(|id| statuses[id]["effective_status"].as_str());
            resource(r, status)
        })
        .collect();
    Ok(json!({ "data": data }))
}

/// `toFiniteNumber`.
fn finite(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().filter(|v| v.is_finite()).unwrap_or_default(),
        Value::String(s) if !s.is_empty() => {
            let n = chatbotx_core::js::to_number(s);
            if n.is_finite() { n } else { 0.0 }
        }
        _ => 0.0,
    }
}

fn action_value(actions: &Value) -> Option<f64> {
    actions
        .as_array()?
        .iter()
        .find(|a| a["action_type"] == CONVERSATION_STARTED)
        .map(|a| finite(&a["value"]))
}

/// `listInsights`: only the workspace's own ads of that ad account; `[]`
/// without a Graph call when none of `ad_ids` is owned.
pub async fn insights(
    db: &Db,
    ident: &Ident,
    ad_account_id: &str,
    ad_ids: &[String],
    date_preset: Option<&str>,
    force: bool,
) -> Result<Value> {
    let rows = rows_of(db, ident).await?;
    let owned: Vec<&str> = rows
        .iter()
        .filter(|r| r.ad_account_id == ad_account_id)
        .filter_map(|r| r.meta_ad_id.as_deref())
        .collect();
    let mut scoped: Vec<String> = ad_ids
        .iter()
        .filter(|id| owned.contains(&id.as_str()))
        .cloned()
        .collect();
    if scoped.is_empty() {
        return Ok(json!({ "data": [] }));
    }
    let preset = date_preset.unwrap_or("maximum").to_owned();
    let filtering = json!([{ "field": "ad.id", "operator": "IN", "value": scoped }]).to_string();
    scoped.sort();
    let key = format!(
        "msgads:ads-insights:{ad_account_id}:{preset}:{}:{}",
        scoped.join(","),
        ident.scope()
    );
    let path = format!("{}/insights", seg(ad_account_id));
    let rows = cached_read(db, ident, key, INSIGHTS_TTL, force, |auth| async move {
        let path = format!("{}/{path}", auth.version);
        g::all_pages(
            &path,
            &[
                ("level", "ad"),
                (
                    "fields",
                    "ad_id,account_currency,impressions,reach,spend,clicks,actions,cost_per_action_type",
                ),
                ("date_preset", preset.as_str()),
                ("filtering", filtering.as_str()),
                ("limit", g::PAGE_LIMIT),
                ("access_token", auth.access_token.as_str()),
            ],
        )
        .await
        .map(Value::Array)
    })
    .await?;
    let data: Vec<Value> = rows
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|row| row["ad_id"].is_string())
        .map(|row| {
            json!({
                "adId": row["ad_id"],
                "currency": row["account_currency"].as_str(),
                "impressions": js_number(finite(&row["impressions"])),
                "reach": js_number(finite(&row["reach"])),
                "spend": js_number(finite(&row["spend"])),
                "clicks": js_number(finite(&row["clicks"])),
                "conversations": js_number(action_value(&row["actions"]).unwrap_or_default()),
                "costPerConversation": action_value(&row["cost_per_action_type"]).map(js_number),
            })
        })
        .collect();
    Ok(json!({ "data": data }))
}

/// `listCachedMessagingAdAccounts` (`{ id, name? }`).
pub async fn ad_accounts(db: &Db, ident: &Ident, force: bool) -> Result<Value> {
    let key = format!("msgads:ad-accounts:{}", ident.scope());
    if force {
        graph::forget(&key);
    }
    graph::cached(key, graph::LIST_TTL, || async {
        let auth = graph::messaging_auth(db, ident.reference()).await?;
        match graph::ad_accounts(&auth).await {
            Ok(accounts) => Ok(accounts),
            Err(failure) => {
                if failure.code == Some(g::INVALID_TOKEN) {
                    graph::mark_invalid(db, ident.reference()).await;
                }
                let message = sanitize_public_text(&g::redact_tokens(&failure.message));
                Err(BusinessError::exception(message, "BAD_REQUEST", 400))
            }
        }
    })
    .await
}

/// `getCachedMessagingAdAccountDetails`.
pub async fn ad_account_details(
    db: &Db,
    ident: &Ident,
    ad_account_id: &str,
    force: bool,
) -> Result<Value> {
    let key = format!(
        "msgads:ad-account-details:{ad_account_id}:{}",
        ident.scope()
    );
    let path = seg(ad_account_id);
    cached_read(db, ident, key, DETAILS_TTL, force, |auth| async move {
        let row = get(
            &auth,
            &path,
            &[(
                "fields",
                "id,name,currency,timezone_name,account_status,min_daily_budget",
            )],
        )
        .await?;
        let mut out = json!({ "id": row["id"] });
        if row["name"].is_string() {
            out["name"] = row["name"].clone();
        }
        out["currency"] = row["currency"].clone();
        out["timezoneName"] = row["timezone_name"].clone();
        out["accountStatus"] = row["account_status"].clone();
        if row["min_daily_budget"].is_number() {
            out["minDailyBudgetMinorUnits"] = row["min_daily_budget"].clone();
        }
        Ok(out)
    })
    .await
}

/// `uploadMessagingAdVideo`.
pub async fn upload_video(
    db: &Db,
    ident: &Ident,
    ad_account_id: &str,
    file_name: &str,
    mime_type: &str,
    bytes: &[u8],
) -> Result<Value> {
    let auth = graph::messaging_auth(db, ident.reference()).await?;
    let path = format!("{}/advideos", seg(ad_account_id));
    let answer = post_file(&auth, &path, "source", file_name, mime_type, bytes).await;
    match answer.and_then(|a| id_field(&a)) {
        Ok(id) => Ok(json!({ "videoId": id })),
        Err(e) => Err(graph_error(db, ident, e).await),
    }
}

/// `getMessagingAdVideoStatus`.
pub async fn video_status(db: &Db, ident: &Ident, video_id: &str) -> Result<Value> {
    let auth = graph::messaging_auth(db, ident.reference()).await?;
    let row = match get(&auth, &seg(video_id), &[("fields", "status")]).await {
        Ok(row) => row,
        Err(e) => return Err(graph_error(db, ident, e).await),
    };
    let status = row["status"]["video_status"]
        .as_str()
        .unwrap_or("processing")
        .to_owned();
    Ok(json!({
        "videoId": row["id"].as_str().unwrap_or(video_id),
        "isReady": status == "ready",
        "isError": status == "error",
        "status": status,
    }))
}

// ---- image upload URL -------------------------------------------------------

fn extension_of(file_name: &str) -> String {
    match file_name.rfind('.') {
        Some(dot) if dot > 0 => file_name[dot..].to_lowercase(),
        _ => String::new(),
    }
}

/// `createAdsCreativeImageUpload`: checks the declared type / extension /
/// size, then a presigned PUT under the workspace's creative prefix and a
/// pending `File` row (`subType` `adsCampaignCreative`).
pub async fn create_image_upload(
    db: &Db,
    uploader: &Uploader,
    storage_url: &str,
    workspace_id: i64,
    file_name: &str,
    mime_type: &str,
    file_size: f64,
) -> Result<Value> {
    let allowed: &[&str] = match mime_type {
        "image/jpeg" => &[".jpg", ".jpeg"],
        "image/png" => &[".png"],
        "image/gif" => &[".gif"],
        "image/webp" => &[".webp"],
        _ => &[],
    };
    let extension = extension_of(file_name);
    if !allowed.contains(&extension.as_str()) {
        return Err(BusinessError::exception(
            format!("Unsupported ad image: {file_name} ({mime_type}). Use JPEG, PNG, GIF or WebP."),
            "adsCreativeUnsupportedImage",
            400,
        ));
    }
    if file_size > MAX_IMAGE_BYTES as f64 {
        return Err(BusinessError::exception(
            format!("Ad images are limited to {MAX_IMAGE_BYTES} bytes."),
            "adsCreativeImageTooLarge",
            400,
        ));
    }
    let key = format!("{}/{}{extension}", creative_prefix(workspace_id), new_id());
    let upload_url = crate::import::presigned_upload_url(uploader, &key).await?;
    let values = vec![
        ("workspaceId", Sql::Integer(workspace_id)),
        ("userId", Sql::Null),
        ("contextType", Sql::Text("generic".into())),
        ("subType", Sql::Text(CREATIVE_UPLOAD_KIND.into())),
        ("path", Sql::Text(key.clone())),
        ("fileName", Sql::Text(file_name.to_owned())),
        ("mimeType", Sql::Text(mime_type.to_owned())),
        ("status", Sql::Text("pending".into())),
    ];
    let file = tx::write(db, move |tx| {
        Ok(chatbotx_db::repo::file::create(tx, &values)?)
    })
    .await?;
    let public_url =
        get_public_file_url(&key, storage_url).unwrap_or_else(|_| format!("{storage_url}{key}"));
    Ok(json!({
        "fileId": file.id.to_string(),
        "imageKey": key,
        "presignedPostUrl": upload_url,
        "publicUrl": public_url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_targeting_for_restricted_categories() {
        let targeting =
            json!({ "countries": ["US"], "ageMin": 30, "genders": [1], "locales": [1033] });
        let out = map_targeting(&targeting, &["HOUSING".into(), "NONE".into()]);
        assert_eq!(
            out,
            json!({
                "geo_locations": { "countries": ["US"] },
                "targeting_automation": { "advantage_audience": 0 },
                "locales": [1033],
            })
        );
        let plain = map_targeting(&json!({ "countries": ["DE"] }), &["NONE".into()]);
        assert_eq!(plain["targeting_automation"]["advantage_audience"], 1);
        assert_eq!(
            special_categories_param(&["HOUSING".into(), "NONE".into()]),
            ["HOUSING"]
        );
        assert_eq!(special_categories_param(&["NONE".into()]), ["NONE"]);
    }

    #[test]
    fn builds_welcome_messages() {
        assert_eq!(welcome_message(&json!({ "type": "default" })), None);
        let single: Value = serde_json::from_str(
            &welcome_message(
                &json!({ "type": "single", "message": "Hi", "quickReplies": [{ "title": "A" }] }),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            single["text_format"]["content"],
            json!({ "title": "Hi", "quick_replies": [{ "title": "A" }] })
        );
        let many: Value = serde_json::from_str(
            &welcome_message(&json!({ "type": "templates", "templates": [
                { "heading": "Hey", "message": "One" }, { "message": "Two" } ] }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            many[0]["text_format"]["content"],
            json!({ "title": "Hey", "message": "One" })
        );
        assert_eq!(
            many[1]["text_format"]["content"],
            json!({ "message": "Two" })
        );
    }

    #[test]
    fn maps_media_and_extensions() {
        let (key, video) = map_media(
            &json!({ "kind": "video", "videoId": "1", "title": "" }),
            None,
        );
        assert_eq!(key, "video_data");
        assert_eq!(Value::Object(video), json!({ "video_id": "1" }));
        let (key, image) = map_media(
            &json!({ "kind": "image", "link": "https://x.test/", "headline": "H" }),
            Some("h1"),
        );
        assert_eq!(key, "link_data");
        assert_eq!(
            Value::Object(image),
            json!({ "link": "https://x.test/", "image_hash": "h1", "name": "H" })
        );
        assert_eq!(extension_of("Photo.JPEG"), ".jpeg");
        assert_eq!(extension_of("noext"), "");
        assert_eq!(extension_of(".png"), "");
        assert_eq!(disposition_text("a\"b\r\n.mp4"), "a%22b.mp4");
        assert_eq!(js_number(1200.0), json!(1200));
        assert_eq!(finite(&json!("12.34")), 12.34);
        assert_eq!(finite(&json!("x")), 0.0);
    }
}
