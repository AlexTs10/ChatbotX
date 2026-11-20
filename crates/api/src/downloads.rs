//! The file download routes of the builder (Q.P6-U5, COVERAGE C1-C4, C10):
//!
//! - `GET /space/{workspaceId}/flows/{id}/export`
//!   (`(no-sidebar)/space/[workspaceId]/flows/[id]/export/route.ts`): the
//!   flow export document; the `flows` permission, else a bare 404.
//! - `GET /space/{workspaceId}/dashboard/ads/export` and its old path
//!   `/space/{workspaceId}/ads/analytics/export`
//!   (`space/[workspaceId]/dashboard/ads/export/route.ts`): the ads
//!   conversion CSV; `superAdmin`, else 404.
//! - `GET /api/imports/products/template` (`api/imports/products/template/route.ts`):
//!   the product import template in the `NEXT_LOCALE` language (no login, as
//!   the original).
//! - `GET /extensions/me/download` (`extensions/me/download/route.ts`): a
//!   contact's own data, behind the signed "me" link.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_business::ads::funnel;
use chatbotx_business::flow;
use chatbotx_business::import::products::{
    build_product_import_template, product_import_template_file_name,
    resolve_product_import_template_locale,
};
use chatbotx_business::pipeline::analytics::parse_analytics_date_range;
use chatbotx_business::template::{Blocking, list_blocking_ids};
use chatbotx_core::utils::csv::{CsvCell, CsvDialect, UTF8_BOM, to_csv_row};
use chatbotx_db::repo::ads_conversion_event::{DateRangeInput, ExportSegmentInput};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::handlers::ApiState;
use crate::rpc::find_rpc_session;
use crate::session::{AuthState, request_cookie};

/// The state of the routes.
#[derive(Clone)]
pub struct DownloadState {
    pub auth: Arc<AuthState>,
    pub api: Arc<ApiState>,
}

/// The download routes.
pub fn router(state: DownloadState) -> Router {
    Router::new()
        .route("/space/{workspaceId}/flows/{id}/export", get(flow_export))
        .route("/space/{workspaceId}/dashboard/ads/export", get(ads_export))
        .route("/space/{workspaceId}/ads/analytics/export", get(ads_export))
        .route("/api/imports/products/template", get(product_template))
        .route("/extensions/me/download", get(me_download))
        .with_state(Arc::new(state))
}

fn bare(status: StatusCode) -> Response {
    status.into_response()
}

fn json_answer(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn digits(text: &str) -> Option<i64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(text.parse().unwrap_or(-1))
}

/// `getCurrentUserAndTargetWorkspace` + `hasWorkspacePermission(key)`
/// (`superAdmin` grants every key): whether the signed-in caller may use
/// `key` in the workspace. Support sessions are not ported.
async fn has_permission(state: &DownloadState, headers: &HeaderMap, ws: i64, key: &str) -> bool {
    let Ok(Some(session)) = find_rpc_session(&state.auth, headers).await else {
        return false;
    };
    let user_id = session.user_id;
    let permissions = state
        .api
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "permissions" FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                    params![ws, user_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?)
        })
        .await
        .ok()
        .flatten();
    let Some(permissions) = permissions.and_then(|p| serde_json::from_str::<Value>(&p).ok()) else {
        return false;
    };
    permissions.get("superAdmin") == Some(&Value::Bool(true))
        || permissions.get(key) == Some(&Value::Bool(true))
}

fn attachment(file_name: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("attachment; filename=\"{file_name}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

// ---- C1: flow export -------------------------------------------------------

async fn flow_export(
    State(state): State<Arc<DownloadState>>,
    Path((workspace_id, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let (Some(ws), Some(id)) = (digits(&workspace_id), digits(&id)) else {
        return bare(StatusCode::NOT_FOUND);
    };
    // A bare 404 on the deny path (a page would be saved as the file).
    if !has_permission(&state, &headers, ws, "flows").await {
        return bare(StatusCode::NOT_FOUND);
    }
    let db = &state.api.db;
    let built = flow::build_export(db, ws, id, false).await;
    if let Err(error) = &built
        && error.code() == Some("notFound")
    {
        return bare(StatusCode::NOT_FOUND);
    }
    // A view-only template copy can not leave as an importable file.
    match list_blocking_ids(db, ws, "flow", &[id], Blocking::Edit).await {
        Ok(locked) if !locked.is_empty() => {
            return json_answer(StatusCode::FORBIDDEN, &json!({ "code": "viewOnly" }));
        }
        Ok(_) => {}
        Err(error) => {
            tracing::error!(%error, "flow export lock check failed");
            return bare(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
    let out = match built {
        Ok(out) => out,
        Err(error) if error.code() == Some("flowNotPublished") => {
            return json_answer(StatusCode::CONFLICT, &json!({ "code": "notPublished" }));
        }
        Err(error) => {
            tracing::error!(%error, "flow export failed");
            return bare(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let envelope = crate::handlers::flows::export_envelope(&out);
    let body = serde_json::to_string_pretty(&envelope).unwrap_or_default();
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CONTENT_DISPOSITION, attachment(&out.file_name)),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body,
    )
        .into_response()
}

// ---- C2 / C3: ads conversion CSV --------------------------------------------

/// `EXPORT_PAGE_SIZE`.
const EXPORT_PAGE_SIZE: i64 = 500;

/// `adsConversionChannelSchema`.
const ADS_CHANNELS: &[&str] = &["whatsapp", "facebook", "messenger", "instagram"];

/// The query as `Object.fromEntries(searchParams)` (the last value wins).
fn query_map(query: Option<&str>) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .into_owned()
        .collect()
}

/// The parsed export request (`exportRequestSchema` /
/// `allChannelExportRequestSchema`); `None` = `invalidRequest`.
struct AdsExportRequest {
    segment: String,
    ad_id: Option<String>,
    channel: Option<String>,
    integration_whatsapp_id: Option<i64>,
    integration_messenger_id: Option<i64>,
    integration_instagram_id: Option<i64>,
    from: String,
    to: String,
    tz: String,
}

fn ads_export_request(raw: &HashMap<String, String>, all: bool) -> Option<AdsExportRequest> {
    let segment = raw
        .get("segment")
        .filter(|s| ["conversations", "leads", "purchases"].contains(&s.as_str()))?
        .clone();
    // `z.string().trim().min(1)`.
    let trimmed = |key: &str| -> Option<Option<String>> {
        match raw.get(key) {
            None => Some(None),
            Some(value) => {
                let value = value.trim();
                (!value.is_empty()).then(|| Some(value.to_string()))
            }
        }
    };
    // `zodBigintAsString().optional()`.
    let id = |key: &str| -> Option<Option<i64>> {
        match raw.get(key) {
            None => Some(None),
            Some(value) => digits(value).map(Some),
        }
    };
    let ad_id = trimmed("adId")?;
    let from = trimmed("from")??;
    let to = trimmed("to")??;
    let tz = match raw.get("tz") {
        None => String::new(),
        Some(tz) if tz.trim().chars().count() <= 64 => tz.trim().to_string(),
        Some(_) => return None,
    };
    if all {
        return Some(AdsExportRequest {
            segment,
            ad_id,
            channel: None,
            integration_whatsapp_id: None,
            integration_messenger_id: None,
            integration_instagram_id: None,
            from,
            to,
            tz,
        });
    }
    let channel = match raw.get("channel") {
        None => None,
        Some(channel) if ADS_CHANNELS.contains(&channel.as_str()) => Some(channel.clone()),
        Some(_) => return None,
    };
    Some(AdsExportRequest {
        segment,
        ad_id,
        channel,
        integration_whatsapp_id: id("integrationWhatsappId")?,
        integration_messenger_id: id("integrationMessengerId")?,
        integration_instagram_id: id("integrationInstagramId")?,
        from,
        to,
        tz,
    })
}

/// `resolveLocale(NEXT_LOCALE)` of the builder (`i18n/config.ts`).
fn request_locale(headers: &HeaderMap) -> &'static str {
    const LOCALES: &[&str] = &[
        "ar", "az", "da", "de", "en", "es", "fi", "fr", "he", "id", "it", "ja", "nl", "pt-BR",
        "pt-PT", "ro", "sv", "tr", "vi", "zh-CN", "zh-TW",
    ];
    let Some(value) = request_cookie(headers, "NEXT_LOCALE") else {
        return "en";
    };
    let value = value.trim();
    if let Some(locale) = LOCALES.iter().find(|l| **l == value) {
        return locale;
    }
    let lower = value.to_lowercase();
    let language = lower.split('-').next().unwrap_or_default();
    match language {
        "" => "en",
        "pt" => {
            if lower.ends_with("-pt") {
                "pt-PT"
            } else {
                "pt-BR"
            }
        }
        "zh" => {
            if ["-tw", "-hk", "-mo", "-hant"]
                .iter()
                .any(|s| lower.contains(s))
            {
                "zh-TW"
            } else {
                "zh-CN"
            }
        }
        _ => LOCALES
            .iter()
            .find(|l| l.eq_ignore_ascii_case(language))
            .copied()
            .unwrap_or("en"),
    }
}

/// `ads.analytics.csv.{contactName,phone,adId,occurredAt,channel}` of
/// `messages/<locale>.json`.
fn csv_labels(locale: &str) -> [&'static str; 5] {
    match locale {
        "ar" => [
            "اسم جهة الاتصال",
            "الهاتف",
            "معرّف الإعلان",
            "وقت الحدوث",
            "القناة",
        ],
        "az" => [
            "Kontakt adı",
            "Telefon",
            "Reklam ID-si",
            "Başvermə vaxtı",
            "Kanal",
        ],
        "da" => [
            "Kontaktnavn",
            "Telefon",
            "Annonce-ID",
            "Fandt sted",
            "Kanal",
        ],
        "de" => [
            "Kontaktname",
            "Telefon",
            "Anzeigen-ID",
            "Aufgetreten am",
            "Kanal",
        ],
        "es" => [
            "Nombre del contacto",
            "Teléfono",
            "ID del anuncio",
            "Ocurrió el",
            "Canal",
        ],
        "fi" => [
            "Kontaktin nimi",
            "Puhelin",
            "Mainostunnus",
            "Tapahtumisajankohta",
            "Kanava",
        ],
        "fr" => [
            "Nom du contact",
            "Téléphone",
            "ID de l’annonce",
            "Survenu le",
            "Canal",
        ],
        "he" => ["שם איש הקשר", "טלפון", "מזהה מודעה", "התרחש בתאריך", "ערוץ"],
        "id" => [
            "Nama kontak",
            "Telepon",
            "ID Iklan",
            "Terjadi pada",
            "Saluran",
        ],
        "it" => [
            "Nome del contatto",
            "Telefono",
            "ID inserzione",
            "Avvenuto il",
            "Canale",
        ],
        "ja" => ["連絡先名", "電話番号", "広告ID", "発生日時", "チャネル"],
        "nl" => [
            "Naam contact",
            "Telefoon",
            "Advertentie-ID",
            "Opgetreden op",
            "Kanaal",
        ],
        "pt-BR" => [
            "Nome do contato",
            "Telefone",
            "ID do anúncio",
            "Ocorreu em",
            "Canal",
        ],
        "pt-PT" => [
            "Nome do contacto",
            "Telefone",
            "ID do anúncio",
            "Ocorrido em",
            "Canal",
        ],
        "ro" => [
            "Numele contactului",
            "Telefon",
            "ID anunț",
            "A avut loc la",
            "Canal",
        ],
        "sv" => [
            "Kontaktens namn",
            "Telefon",
            "Annons-ID",
            "Skedde vid",
            "Kanal",
        ],
        "tr" => [
            "Kişi adı",
            "Telefon",
            "Reklam Kimliği",
            "Gerçekleşme zamanı",
            "Kanal",
        ],
        "vi" => [
            "Tên liên hệ",
            "Số điện thoại",
            "ID quảng cáo",
            "Thời điểm xảy ra",
            "Kênh",
        ],
        "zh-CN" => ["联系人名称", "电话", "广告 ID", "发生时间", "渠道"],
        "zh-TW" => ["聯絡人姓名", "電話", "廣告 ID", "發生時間", "頻道"],
        _ => ["Contact name", "Phone", "Ad ID", "Occurred at", "Channel"],
    }
}

/// `ads.conversionEvents.tabs.<channel>` (the same in every locale but `ar`).
fn channel_label(locale: &str, channel: &str) -> String {
    match channel {
        "whatsapp" => "WhatsApp".to_string(),
        "instagram" if locale == "ar" => "إنستغرام".to_string(),
        "instagram" => "Instagram".to_string(),
        "messenger" => "Messenger".to_string(),
        "facebook" => "Facebook".to_string(),
        other => format!("ads.conversionEvents.tabs.{other}"),
    }
}

async fn ads_export(
    State(state): State<Arc<DownloadState>>,
    Path(workspace_id): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let Some(ws) = digits(&workspace_id) else {
        return bare(StatusCode::NOT_FOUND);
    };
    if !has_permission(&state, &headers, ws, "superAdmin").await {
        return bare(StatusCode::NOT_FOUND);
    }
    let raw = query_map(query.as_deref());
    let all = raw.get("channel").map(String::as_str) == Some("all");
    let Some(request) = ads_export_request(&raw, all) else {
        return json_answer(
            StatusCode::BAD_REQUEST,
            &json!({ "code": "invalidRequest" }),
        );
    };
    let range = parse_analytics_date_range(
        &request.from,
        &request.to,
        &request.tz,
        chatbotx_db::repo::now_ms(),
    );
    let locale = request_locale(&headers);
    let labels = csv_labels(locale);
    // An omitted `channel` is a legacy consumer of the 4-column `ctwa-*.csv`.
    let legacy = !all && request.channel.is_none();
    let header_cells: Vec<CsvCell> = labels[..if legacy { 4 } else { 5 }]
        .iter()
        .map(|label| CsvCell::from(*label))
        .collect();
    let file_name = if all {
        format!(
            "ads-all-{}-{}-{}.csv",
            request.segment, range.from, range.to
        )
    } else if legacy {
        format!("ctwa-{}-{}-{}.csv", request.segment, range.from, range.to)
    } else {
        format!(
            "ads-{}-{}-{}-{}.csv",
            request.channel.as_deref().unwrap_or_default(),
            request.segment,
            range.from,
            range.to
        )
    };
    let request_label = channel_label(
        locale,
        request
            .channel
            .as_deref()
            .unwrap_or(chatbotx_db::repo::ads_conversion_event::DEFAULT_ADS_CONVERSION_CHANNEL),
    );
    // ponytail: the CSV is built in memory (the original streams page by
    // page); at most 366 days of one workspace's conversions. Stream with
    // `Body::from_stream` if exports grow past a few MB.
    let mut body = format!(
        "{UTF8_BOM}{}",
        to_csv_row(&header_cells, CsvDialect::Minimal)
    );
    let mut after_id = None;
    loop {
        let page = funnel::export_rows(
            &state.api.db,
            ExportSegmentInput {
                range: DateRangeInput {
                    workspace_id: ws,
                    since: range.since,
                    until: range.until,
                    integration_whatsapp_id: request.integration_whatsapp_id,
                    channel: request.channel.clone(),
                    integration_messenger_id: request.integration_messenger_id,
                    integration_instagram_id: request.integration_instagram_id,
                    all_channels: all,
                    timezone: None,
                },
                segment: request.segment.clone(),
                ad_id: request.ad_id.clone(),
                after_id,
                limit: EXPORT_PAGE_SIZE,
            },
        )
        .await;
        let page = match page {
            Ok(page) => page,
            Err(error) => {
                tracing::error!(%error, "ads export failed");
                return bare(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };
        for row in page["data"].as_array().into_iter().flatten() {
            let text = |key: &str| CsvCell::from(row[key].as_str().map(str::to_string));
            let mut cells = vec![
                text("contactName"),
                text("phoneNumber"),
                text("adId"),
                text("occurredAt"),
            ];
            if !legacy {
                cells.push(CsvCell::from(if all {
                    row["channel"]
                        .as_str()
                        .map_or_else(String::new, |c| channel_label(locale, c))
                } else {
                    request_label.clone()
                }));
            }
            body.push_str(&to_csv_row(&cells, CsvDialect::Minimal));
        }
        after_id = page["nextAfterId"].as_str().and_then(|id| id.parse().ok());
        if after_id.is_none() {
            break;
        }
    }
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/csv; charset=utf-8"),
            ),
            (header::CONTENT_DISPOSITION, attachment(&file_name)),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=60"),
            ),
        ],
        body,
    )
        .into_response()
}

// ---- C4: product import template ------------------------------------------

/// `PRODUCT_IMPORT_TEMPLATE_MIME_TYPE`.
const XLSX_MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

async fn product_template(headers: HeaderMap) -> Response {
    let locale = resolve_product_import_template_locale(request_locale(&headers));
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(XLSX_MIME)),
            (
                header::CONTENT_DISPOSITION,
                attachment(&product_import_template_file_name(locale)),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=300"),
            ),
        ],
        build_product_import_template(locale),
    )
        .into_response()
}

// ---- C10: the "me" data download -------------------------------------------

/// `buildMeExport` (`systemFieldService`, after `loadVerifiedMeContext`).
async fn me_export(
    api: &ApiState,
    link: [String; 5],
) -> Result<Option<Value>, chatbotx_db::DbError> {
    let [w, u, ib, id, hash] = link;
    let Some(encryption) = api.encryption.as_deref() else {
        return Ok(None);
    };
    let signed = chatbotx_core::encryption::MeLinkParams {
        workspace_id: w.clone(),
        source_id: u.clone(),
        integration_id: ib.clone(),
        form_id: id.clone(),
    };
    if !encryption.verify_me_link(&signed, Some(&hash)) {
        return Ok(None);
    }
    let Some(form_id) = digits(&id) else {
        return Ok(None);
    };
    let urls = crate::handlers::contacts::tenant_urls(api);
    api.db
        .read(move |conn| {
            let payload: Option<String> = conn
                .query_row(
                    r#"SELECT "payload" FROM "SystemField" WHERE "id" = ?1 AND "type" = 'me'"#,
                    [form_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(payload) = payload.and_then(|p| serde_json::from_str::<Value>(&p).ok()) else {
                return Ok(None);
            };
            let field = |key: &str| match &payload[key] {
                Value::String(text) => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            };
            let (Some(workspace), Some(channel), Some(source), Some(integration)) = (
                field("workspaceId"),
                field("channel"),
                field("sourceId"),
                field("integrationId"),
            ) else {
                return Ok(None);
            };
            let (Some(contact_inbox_id), Some(contact_id)) = (
                field("contactInboxId").as_deref().and_then(digits),
                field("contactId").as_deref().and_then(digits),
            ) else {
                return Ok(None);
            };
            if workspace != w || source != u || integration != ib {
                return Ok(None);
            }
            let Some(ws) = digits(&workspace) else {
                return Ok(None);
            };
            let contact = conn
                .query_row(
                    r#"SELECT "firstName", "lastName", "fullName", "email", "phoneNumber", "locale",
                              "timezone", "gender", "avatar"
                       FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![contact_id, ws],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, Option<String>>(7)?,
                            row.get::<_, Option<String>>(8)?,
                        ))
                    },
                )
                .optional()?;
            let inbox = conn
                .query_row(
                    r#"SELECT "contactId", "sourceId", "channel", "firstInteractionAt", "createdAt", "lastMessageAt"
                       FROM "ContactInbox" WHERE "id" = ?1"#,
                    [contact_inbox_id],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, Option<i64>>(5)?,
                        ))
                    },
                )
                .optional()?;
            let (Some(contact), Some(inbox)) = (contact, inbox) else {
                return Ok(None);
            };
            if inbox.0 != contact_id || inbox.1 != source || inbox.2 != channel {
                return Ok(None);
            }
            if let Some(conversation_id) = field("conversationId").filter(|c| !c.is_empty()) {
                let found: Option<i64> = conn
                    .query_row(
                        r#"SELECT "id" FROM "Conversation" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "contactId" = ?3"#,
                        params![digits(&conversation_id).unwrap_or(-1), ws, contact_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if found.is_none() {
                    return Ok(None);
                }
            }
            let messages = chatbotx_db::repo::message::list_incoming_texts_by_contact_inbox(
                conn,
                &chatbotx_db::repo::message::ListIncomingTextsByContactInboxParams {
                    contact_inbox_id,
                    limit: None,
                    since_time: inbox.3.unwrap_or(inbox.4),
                    workspace_id: ws,
                },
            )
            .map_err(|e| match e {
                chatbotx_db::repo::Error::Db(e) => e,
                other => chatbotx_db::DbError::other(other.message()),
            })?;
            let language: Option<String> = conn
                .query_row(
                    r#"SELECT "language" FROM "Workspace" WHERE "id" = ?1"#,
                    [ws],
                    |row| row.get(0),
                )
                .optional()?;
            let now = chatbotx_db::repo::now_ms();
            let avatar = chatbotx_business::contact::avatar::resolve_contact_avatar_url(
                ws,
                contact.8.as_deref(),
                &[chatbotx_business::contact::avatar::AvatarInbox {
                    id: contact_inbox_id,
                    channel: inbox.2.clone(),
                    last_message_at: inbox.5,
                }],
                &urls,
                now,
            );
            let generated = chatbotx_db::repo::iso(now);
            Ok(Some(json!({
                "data": {
                    "id": inbox.1,
                    "first_name": contact.0,
                    "last_name": contact.1,
                    "full_name": contact.2,
                    "email": contact.3.unwrap_or_default(),
                    "phone": contact.4.unwrap_or_default(),
                    "locale": contact.5,
                    "timezone": contact.6,
                    "gender": chatbotx_core::variables::system_field::resolve_gender_label(
                        language.as_deref(),
                        contact.7.as_deref(),
                    ),
                    "profile_pic": avatar,
                },
                "messages": messages,
                "generated": format!("{} (UTC)", generated[..19].replace('T', " ")),
            })))
        })
        .await
}

async fn me_download(
    State(state): State<Arc<DownloadState>>,
    RawQuery(query): RawQuery,
) -> Response {
    let raw = query_map(query.as_deref());
    let param = |key: &str| raw.get(key).filter(|v| !v.is_empty()).cloned();
    let (Some(w), Some(u), Some(ib), Some(id), Some(hash)) = (
        param("w"),
        param("u"),
        param("ib"),
        param("id"),
        param("hash"),
    ) else {
        return bare(StatusCode::NOT_FOUND);
    };
    // `loadServableWorkspace`: a missing or frozen workspace answers 410.
    let servable = match digits(&w) {
        Some(ws) => state
            .api
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT 1 FROM "Workspace" WHERE "id" = ?1 AND "scheduledDeletionAt" IS NULL"#,
                        [ws],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some())
            })
            .await
            .unwrap_or(false),
        None => false,
    };
    if !servable {
        return json_answer(
            StatusCode::GONE,
            &json!({ "code": "workspaceScheduledDeletion" }),
        );
    }
    let file_name: String = u
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    match me_export(&state.api, [w, u, ib, id, hash]).await {
        Ok(Some(data)) => (
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
                (
                    header::CONTENT_DISPOSITION,
                    attachment(&format!("{file_name}.json")),
                ),
            ],
            data.to_string(),
        )
            .into_response(),
        Ok(None) => bare(StatusCode::NOT_FOUND),
        Err(error) => {
            tracing::error!(%error, "me export failed");
            bare(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_request_follows_the_zod_schemas() {
        let map = |q: &str| query_map(Some(q));
        assert!(ads_export_request(&map("segment=&from=a&to=b"), false).is_none());
        assert!(
            ads_export_request(&map("segment=leads&channel=tiktok&from=a&to=b"), false).is_none()
        );
        assert!(ads_export_request(&map("segment=leads&adId=+&from=a&to=b"), false).is_none());
        assert!(ads_export_request(&map("segment=leads&from=a"), false).is_none());
        let all = ads_export_request(
            &map("segment=leads&channel=all&integrationWhatsappId=x&from=a&to=b"),
            true,
        )
        .expect("all channels ignores the integration ids");
        assert_eq!(all.integration_whatsapp_id, None);
        let legacy = ads_export_request(
            &map("segment=leads&integrationWhatsappId=1234567890123&from=2026-08-01&to=2026-08-10"),
            false,
        )
        .expect("legacy");
        assert_eq!(legacy.channel, None);
        assert_eq!(legacy.integration_whatsapp_id, Some(1_234_567_890_123));
    }

    #[test]
    fn the_locale_comes_from_the_next_locale_cookie() {
        let headers = |cookie: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
            h
        };
        assert_eq!(request_locale(&HeaderMap::new()), "en");
        assert_eq!(request_locale(&headers("NEXT_LOCALE=vi")), "vi");
        assert_eq!(request_locale(&headers("NEXT_LOCALE=de-AT")), "de");
        assert_eq!(request_locale(&headers("NEXT_LOCALE=xx")), "en");
    }
}
