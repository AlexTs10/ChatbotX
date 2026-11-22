//! The channel Meta Conversions API operations behind the public
//! `<channel>Channels.*Capi*` routes (`meta-conversions/lib/capi-operations.ts`
//! and `metaConversionsService` save / provision / test-code / test-event /
//! disconnect / reconnect, `integrations/meta-conversions` dataset + events).
//!
//! One table-generic path for Messenger, Instagram (Facebook Login only) and
//! WhatsApp: the three integration tables share the CAPI columns.

use std::sync::OnceLock;
use std::time::Duration;

use chatbotx_core::encryption::EncryptedData;
use chatbotx_core::utils::graph_error::{GraphErrorSource, format_graph_error_message};
use chatbotx_core::utils::meta_capi::parse_capi_test_messaging_id;
use chatbotx_db::Db;
use chatbotx_db::repo::{new_id, now_ms, whatsapp_business_account};
use chatbotx_db::rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::email_marketing::open_auth;
use crate::errors::{BusinessError, Result};
use crate::meta_catalog::process_encryption;
use crate::tx;

/// `API_URL` / `DEFAULT_API_VERSION` / `META_CONVERSIONS_PARTNER_AGENT`.
const API_URL: &str = "https://graph.facebook.com";
const API_VERSION: &str = "v24.0";
const PARTNER_AGENT: &str = "ChatConnectX";
/// `DATASET_NAME_SUFFIX`.
const DATASET_NAME_SUFFIX: &str = "Event Data";

/// `MetaConversionsChannel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapiChannel {
    Messenger,
    Instagram,
    Whatsapp,
}

impl CapiChannel {
    fn table(self) -> &'static str {
        match self {
            CapiChannel::Messenger => "IntegrationMessenger",
            CapiChannel::Instagram => "IntegrationInstagram",
            CapiChannel::Whatsapp => "IntegrationWhatsapp",
        }
    }
    /// The Meta resource the dataset hangs off (page / IG user / WABA).
    fn resource_column(self) -> &'static str {
        match self {
            CapiChannel::Messenger => "pageId",
            CapiChannel::Instagram => "igId",
            CapiChannel::Whatsapp => "wabaId",
        }
    }
    fn messaging_channel(self) -> &'static str {
        match self {
            CapiChannel::Messenger => "messenger",
            CapiChannel::Instagram => "instagram",
            CapiChannel::Whatsapp => "whatsapp",
        }
    }
}

/// The CAPI columns of one channel integration row, with the token resolved.
#[derive(Debug, Clone)]
struct CapiIntegration {
    id: i64,
    workspace_id: i64,
    name: String,
    resource_id: String,
    dataset_id: Option<String>,
    test_event_code: Option<String>,
    disconnected: bool,
    /// `resolveCapiAccessToken`: manual token, else (WhatsApp) the WABA
    /// credential, else `auth.tokens.accessToken`. `None` = unreadable.
    access_token: Option<String>,
}

fn not_found() -> BusinessError {
    BusinessError::exception("Channel not found", "notFound", 404)
}

fn request_failed(message: impl Into<String>) -> BusinessError {
    BusinessError::exception(message, "capiRequestFailed", 400)
}

fn refused(message: &str) -> BusinessError {
    BusinessError::exception(message, "capiTestEventRefused", 422)
}

fn token_error() -> BusinessError {
    BusinessError::exception(
        "Meta CAPI access token is missing",
        "internalServerError",
        500,
    )
}

pub(crate) fn decrypt_token(raw: &Value, aad: Option<&str>) -> Option<String> {
    let data: EncryptedData = serde_json::from_value(raw.clone()).ok()?;
    let value: Value = process_encryption().decrypt_object(&data, aad).ok()?;
    value["accessToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// `findCapiIntegration` + `requireIntegration`: a workspace row, 404 when
/// missing or (Instagram) not a Facebook Login account.
async fn require(
    db: &Db,
    channel: CapiChannel,
    workspace_id: i64,
    id: i64,
) -> Result<CapiIntegration> {
    tx::read(db, move |conn| {
        let kind = if channel == CapiChannel::Instagram {
            "\"type\""
        } else {
            "'facebook'"
        };
        let sql = format!(
            "SELECT \"name\", \"{}\", \"datasetId\", \"capiTestEventCode\", \"capiDisconnectedAt\", \
             \"capiAccessToken\", \"auth\", {kind} FROM \"{}\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
            channel.resource_column(),
            channel.table(),
        );
        type Row = (
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            String,
            Option<String>,
        );
        let row: Option<Row> = conn
            .query_row(&sql, params![id, workspace_id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })
            .optional()?;
        let Some((name, resource_id, dataset_id, code, disconnected_at, manual, auth, kind)) = row
        else {
            return Err(not_found());
        };
        // Instagram Business Login channels have no Meta CAPI.
        if kind.as_deref() != Some("facebook") {
            return Err(not_found());
        }
        let manual = manual.and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        let access_token = match manual {
            Some(raw) if !raw.is_null() => decrypt_token(&raw, None),
            _ => {
                let waba = if channel == CapiChannel::Whatsapp {
                    whatsapp_business_account::find_by_waba(conn, workspace_id, &resource_id)?
                        .and_then(|w| {
                            decrypt_token(
                                &w.credential.0,
                                Some(&format!("whatsapp-waba:{workspace_id}:{resource_id}")),
                            )
                        })
                } else {
                    None
                };
                waba.or_else(|| {
                    open_auth(&auth)?["tokens"]["accessToken"]
                        .as_str()
                        .filter(|t| !t.is_empty())
                        .map(str::to_owned)
                })
            }
        };
        Ok(CapiIntegration {
            id,
            workspace_id,
            name: name.unwrap_or_default(),
            resource_id,
            dataset_id,
            test_event_code: code,
            disconnected: disconnected_at.is_some(),
            access_token,
        })
    })
    .await
}

/// One `UPDATE` of the CAPI columns (`updatedAt` too).
async fn update(
    db: &Db,
    channel: CapiChannel,
    integration: &CapiIntegration,
    set: &'static str,
    value: Option<String>,
) -> Result<()> {
    let (id, workspace_id) = (integration.id, integration.workspace_id);
    tx::write(db, move |tx| {
        tx.execute(
            &format!(
                "UPDATE \"{}\" SET {set}, \"updatedAt\" = ?2 WHERE \"id\" = ?3 AND \"workspaceId\" = ?4",
                channel.table()
            ),
            params![value, now_ms(), id, workspace_id],
        )?;
        Ok(())
    })
    .await
}

// ---- Graph -----------------------------------------------------------------

fn http_client() -> Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<std::result::Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            // ponytail: fixed Meta host, no redirects followed; the ids are path-encoded.
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| request_failed(e.clone()))
}

fn segment(id: &str) -> String {
    url::form_urlencoded::byte_serialize(id.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// `MetaConversionsException` message of a Graph error body.
pub(crate) fn graph_message(body: &Value) -> String {
    let error = &body["error"];
    let text = |v: &Value| v.as_str().map(str::to_owned);
    let details = match &error["error_data"] {
        Value::Object(o) => o.get("details").and_then(Value::as_str).map(str::to_owned),
        _ => None,
    };
    format_graph_error_message(&GraphErrorSource {
        code: error.get("code").cloned().filter(|v| !v.is_null()),
        sub_code: error.get("error_subcode").cloned().filter(|v| !v.is_null()),
        message: text(&error["message"]),
        user_message: text(&error["error_user_msg"])
            .or(details)
            .or_else(|| text(&error["error_user_title"])),
    })
    .unwrap_or_else(|| "Meta Conversions API call failed".to_owned())
}

/// `metaConversionsGraphClient` GET / POST: any failure is a `capiRequestFailed`.
async fn graph(
    method: reqwest::Method,
    path: &str,
    token: &str,
    query: &[(&str, &str)],
    body: Option<&Value>,
) -> Result<Value> {
    let mut url = url::Url::parse(&format!("{API_URL}/{API_VERSION}/{path}"))
        .map_err(|e| request_failed(e.to_string()))?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    let mut request = http_client()?
        .request(method, url.as_str())
        .header("authorization", format!("Bearer {token}"));
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    let response = request
        .send()
        .await
        .map_err(|e| request_failed(e.to_string()))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| request_failed(e.to_string()))?;
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(request_failed(graph_message(&body)));
    }
    Ok(body)
}

/// `readDatasetId`: `id` | `dataset_id` | `data.id` | `dataset.id`.
fn read_dataset_id(body: &Value) -> Result<String> {
    [
        &body["id"],
        &body["dataset_id"],
        &body["data"]["id"],
        &body["dataset"]["id"],
    ]
    .into_iter()
    .find_map(|v| {
        v.as_str()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    })
    .ok_or_else(|| request_failed("Meta Conversions API call failed"))
}

/// `ensureDataset` with `buildDatasetName(resourceName)`.
async fn create_dataset(integration: &CapiIntegration, token: &str) -> Result<String> {
    let name = integration.name.trim();
    let dataset_name = if name.is_empty() {
        DATASET_NAME_SUFFIX.to_owned()
    } else {
        format!("{name} {DATASET_NAME_SUFFIX}")
    };
    let body = graph(
        reqwest::Method::POST,
        &format!("{}/dataset", segment(&integration.resource_id)),
        token,
        &[],
        Some(&json!({ "dataset_name": dataset_name })),
    )
    .await?;
    read_dataset_id(&body)
}

/// `provisionDatasetViaMeta`: WhatsApp creates with the agency System User
/// token first and retries once with the connect token on an authorization
/// error; Messenger / Instagram create with the connect token.
async fn provision_dataset(
    db: &Db,
    channel: CapiChannel,
    integration: &CapiIntegration,
    token: &str,
) -> Result<String> {
    if channel != CapiChannel::Whatsapp {
        return create_dataset(integration, token).await;
    }
    let (ws, id) = (integration.workspace_id, integration.id);
    let row = tx::read(db, move |conn| {
        Ok(crate::meta_capi_send::find_integration(
            conn, channel, ws, id,
        )?)
    })
    .await?
    .ok_or_else(not_found)?;
    crate::meta_capi_send::provision_dataset(
        db,
        &crate::connection::credentials::HttpUpstream,
        &row,
        token,
    )
    .await
    .map_err(|e| request_failed(e.message))
}

// ---- operations --------------------------------------------------------------

/// `setCapiDataset`: validate + store the given dataset, or create (reuse) the
/// resource's dataset at Meta; then clear a user disconnect.
pub async fn set_dataset(
    db: &Db,
    channel: CapiChannel,
    workspace_id: i64,
    id: i64,
    dataset_id: Option<String>,
) -> Result<()> {
    let integration = require(db, channel, workspace_id, id).await?;
    let token = integration.access_token.clone().ok_or_else(token_error)?;
    match dataset_id {
        Some(dataset_id) => {
            let body = graph(
                reqwest::Method::GET,
                &segment(&dataset_id),
                &token,
                &[("fields", "id")],
                None,
            )
            .await?;
            read_dataset_id(&body)?;
            update(
                db,
                channel,
                &integration,
                "\"datasetId\" = ?1",
                Some(dataset_id),
            )
            .await?;
        }
        None => {
            let created = provision_dataset(db, channel, &integration, &token).await?;
            if integration.dataset_id.as_deref() != Some(created.as_str()) {
                update(
                    db,
                    channel,
                    &integration,
                    "\"datasetId\" = ?1",
                    Some(created),
                )
                .await?;
            }
        }
    }
    update(
        db,
        channel,
        &integration,
        "\"capiDisconnectedAt\" = ?1",
        None,
    )
    .await
}

/// `disconnectCapi`: a user-intent disconnect; dataset and code are kept.
pub async fn disconnect(db: &Db, channel: CapiChannel, workspace_id: i64, id: i64) -> Result<()> {
    let integration = require(db, channel, workspace_id, id).await?;
    let (id, ws) = (integration.id, integration.workspace_id);
    tx::write(db, move |tx| {
        let now = now_ms();
        tx.execute(
            &format!(
                "UPDATE \"{}\" SET \"capiDisconnectedAt\" = ?1, \"updatedAt\" = ?1 WHERE \"id\" = ?2 AND \"workspaceId\" = ?3",
                channel.table()
            ),
            params![now, id, ws],
        )?;
        Ok(())
    })
    .await
}

/// `setCapiTestEventCode` (`None` clears).
pub async fn set_test_event_code(
    db: &Db,
    channel: CapiChannel,
    workspace_id: i64,
    id: i64,
    code: Option<String>,
) -> Result<()> {
    let integration = require(db, channel, workspace_id, id).await?;
    update(
        db,
        channel,
        &integration,
        "\"capiTestEventCode\" = ?1",
        code,
    )
    .await
}

/// `sendTestEvent`: one sample Purchase to the dataset's Test Events.
pub async fn send_test_event(
    db: &Db,
    channel: CapiChannel,
    workspace_id: i64,
    id: i64,
    messaging_id: &str,
) -> Result<()> {
    let integration = require(db, channel, workspace_id, id).await?;
    let Some(code) = integration
        .test_event_code
        .clone()
        .filter(|c| !c.is_empty())
    else {
        return Err(refused(
            "Save a test event code first (`setCapiTestEventCode` of the same channel group), then send a test event.",
        ));
    };
    if integration.disconnected {
        return Err(refused(
            "Conversions API is disconnected for this channel. Save a dataset to reconnect it.",
        ));
    }
    let messaging_id = parse_capi_test_messaging_id(messaging_id)
        .ok_or_else(|| refused("`messagingId` is not a valid messaging id."))?;
    let token = integration.access_token.clone().ok_or_else(token_error)?;
    // `ensureDatasetId`: the stored id first, else create and store it if still null.
    let dataset_id = match integration.dataset_id.clone() {
        Some(dataset_id) => dataset_id,
        None => {
            let created = provision_dataset(db, channel, &integration, &token).await?;
            let (table, row_id, ws, value) = (
                channel.table(),
                integration.id,
                integration.workspace_id,
                created,
            );
            tx::write(db, move |tx| {
                tx.execute(
                    &format!(
                        "UPDATE \"{table}\" SET \"datasetId\" = ?1, \"updatedAt\" = ?2 \
                         WHERE \"id\" = ?3 AND \"workspaceId\" = ?4 AND \"datasetId\" IS NULL"
                    ),
                    params![value, now_ms(), row_id, ws],
                )?;
                let stored: Option<String> = tx.query_row(
                    &format!("SELECT \"datasetId\" FROM \"{table}\" WHERE \"id\" = ?1"),
                    params![row_id],
                    |r| r.get(0),
                )?;
                stored.ok_or_else(|| {
                    BusinessError::exception(
                        "Meta CAPI dataset id was not stored",
                        "internalServerError",
                        500,
                    )
                })
            })
            .await?
        }
    };
    let user_data = match channel {
        CapiChannel::Messenger => json!({
            "page_id": integration.resource_id,
            "page_scoped_user_id": messaging_id,
        }),
        CapiChannel::Instagram => json!({
            "ig_account_id": integration.resource_id,
            "instagram_business_account_id": integration.resource_id,
            "ig_sid": messaging_id,
        }),
        CapiChannel::Whatsapp => json!({
            "whatsapp_business_account_id": integration.resource_id,
            "ctwa_clid": messaging_id,
        }),
    };
    let event = json!({
        "event_name": "Purchase",
        "event_time": now_ms() / 1000,
        "event_id": format!("test:{}", new_id()),
        "action_source": "business_messaging",
        "messaging_channel": channel.messaging_channel(),
        "user_data": user_data,
        "custom_data": { "currency": "USD", "value": 100 },
    });
    graph(
        reqwest::Method::POST,
        &format!("{}/events", segment(&dataset_id)),
        &token,
        &[],
        Some(&json!({
            "data": [event],
            "partner_agent": PARTNER_AGENT,
            "test_event_code": code,
        })),
    )
    .await?;
    Ok(())
}

// ---- dedicated token connect (OAuth) -------------------------------------------

/// A Graph call of the dedicated connect through `upstream` (bearer token).
async fn graph_via(
    upstream: &impl crate::connection::credentials::Upstream,
    method: &'static str,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let reply = upstream
        .send(crate::connection::credentials::UpstreamRequest {
            method,
            url: format!("{API_URL}/{API_VERSION}/{path}"),
            headers: {
                let mut headers = vec![("authorization".to_string(), format!("Bearer {token}"))];
                if body.is_some() {
                    headers.push(("content-type".into(), "application/json".into()));
                }
                headers
            },
            body: body.map(Value::to_string),
        })
        .await
        .map_err(request_failed)?;
    let (status, text) = reply;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        return Err(request_failed(graph_message(&body)));
    }
    Ok(body)
}

/// `isDatasetAccessError`: Graph code 10, 100, 803 or 200-299.
fn is_dataset_access_error(body: &Value) -> bool {
    body["error"]["code"]
        .as_i64()
        .is_some_and(|c| matches!(c, 10 | 100 | 803 | 200..=299))
}

/// `metaConversionsService.connectCapiWithToken`: the stored dataset when the
/// new token can read it, else a dataset created with that token; then the
/// dataset id and the encrypted token are written together, the disconnect
/// flag cleared. `Ok(false)` = the row is gone. Messenger / Instagram only.
pub async fn connect_with_token(
    db: &Db,
    upstream: &impl crate::connection::credentials::Upstream,
    channel: CapiChannel,
    workspace_id: i64,
    id: i64,
    access_token: &str,
) -> Result<bool> {
    if access_token.is_empty() {
        return Err(request_failed("Meta CAPI access token is missing"));
    }
    let integration = require(db, channel, workspace_id, id).await?;
    // `findReadableStoredDataset`: only a definite "this token cannot read
    // it" swaps the dataset; a timeout or rate limit is surfaced.
    let readable = match &integration.dataset_id {
        Some(dataset) => {
            let (status, text) = upstream
                .send(crate::connection::credentials::UpstreamRequest {
                    method: "GET",
                    url: format!("{API_URL}/{API_VERSION}/{}?fields=id", segment(dataset)),
                    headers: vec![("authorization".into(), format!("Bearer {access_token}"))],
                    body: None,
                })
                .await
                .map_err(request_failed)?;
            let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if (200..300).contains(&status) {
                Some(read_dataset_id(&body)?)
            } else if is_dataset_access_error(&body) {
                tracing::info!(
                    integration_id = id,
                    "Stored CAPI dataset is not readable with the dedicated token; using the dataset linked at Meta"
                );
                None
            } else {
                return Err(request_failed(graph_message(&body)));
            }
        }
        None => None,
    };
    let dataset_id = match readable {
        Some(id) => id,
        None => {
            let name = integration.name.trim();
            let dataset_name = if name.is_empty() {
                DATASET_NAME_SUFFIX.to_owned()
            } else {
                format!("{name} {DATASET_NAME_SUFFIX}")
            };
            let body = graph_via(
                upstream,
                "POST",
                &format!("{}/dataset", segment(&integration.resource_id)),
                access_token,
                Some(&json!({ "dataset_name": dataset_name })),
            )
            .await?;
            read_dataset_id(&body)?
        }
    };
    let sealed = process_encryption()
        .encrypt_object(&json!({ "accessToken": access_token }), None)
        .map_err(|e| request_failed(e.to_string()))
        .and_then(|blob| serde_json::to_string(&blob).map_err(|e| request_failed(e.to_string())))?;
    tx::write(db, move |tx| {
        let updated = tx.execute(
            &format!(
                "UPDATE \"{}\" SET \"datasetId\" = ?1, \"capiAccessToken\" = ?2, \"capiDisconnectedAt\" = NULL, \
                 \"updatedAt\" = ?3 WHERE \"id\" = ?4 AND \"workspaceId\" = ?5",
                channel.table()
            ),
            params![dataset_id, sealed, now_ms(), id, workspace_id],
        )?;
        Ok(updated > 0)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_message_and_dataset_id() {
        let body = json!({ "error": {
            "message": "(#200) App does not have page_events permission",
            "code": 200, "error_subcode": 2804079,
        }});
        assert_eq!(
            graph_message(&body),
            "(#200 - 2804079) App does not have page_events permission"
        );
        assert_eq!(
            graph_message(&Value::Null),
            "Meta Conversions API call failed"
        );
        assert_eq!(
            read_dataset_id(&json!({ "data": { "id": " 7 " } })).ok(),
            Some("7".into())
        );
        assert!(read_dataset_id(&json!({ "id": "  " })).is_err());
        assert_eq!(segment("1/2 3"), "1%2F2%203");
    }
}
