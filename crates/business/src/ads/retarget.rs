//! `ads-retarget/service.ts` (`startAudienceSync`) and the worker's
//! `handleSyncRetargetAudience`: the segment's contacts, hashed, into a Meta
//! custom audience of the workspace's Facebook Ads connection
//! (`bulkSyncHashedAudienceUsers`).

use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_db::repo::ads_conversion_event::{self as events, DateRangeInput, ExportSegmentInput};
use chatbotx_db::repo::iso;
use chatbotx_db::rusqlite::OptionalExtension;
use chatbotx_jobs::queues::integration::{
    IntegrationJob, IntegrationJobAction, enqueue_integration_job,
};
use chatbotx_jobs::{Enqueuer, JobOptions};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::graph::{self, AdsAuth};
use crate::errors::{BusinessError, Result};
use crate::marketing_messages::graph::{self as g, GraphError, seg};
use crate::tx;

/// `RETARGET_CONTACT_PAGE_SIZE`.
const PAGE_SIZE: i64 = 500;
/// `META_AUDIENCE_BATCH_SIZE` / `AUDIENCE_USERS_BATCH_SIZE`.
const BATCH: usize = 5000;

/// `startRetargetAudienceSyncInput` (checked by the handler).
#[derive(Debug, Clone, Default)]
pub struct StartSync {
    pub workspace_id: i64,
    pub segment: String,
    pub ad_id: Option<String>,
    pub channel: Option<String>,
    pub integration_whatsapp_id: Option<String>,
    pub integration_messenger_id: Option<String>,
    pub integration_instagram_id: Option<String>,
    /// ms.
    pub since: i64,
    pub until: i64,
    pub ad_account_id: String,
    pub audience_name: Option<String>,
    pub custom_audience_id: Option<String>,
}

fn sanitize_job_id_part(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `buildRetargetJobId`.
fn job_id(input: &StartSync, audience: &str, since: &str, until: &str) -> String {
    let account = input
        .integration_messenger_id
        .as_deref()
        .or(input.integration_instagram_id.as_deref())
        .or(input.integration_whatsapp_id.as_deref())
        .unwrap_or("all-accounts");
    [
        "ads-retarget",
        &input.workspace_id.to_string(),
        audience,
        &input.segment,
        input.channel.as_deref().unwrap_or("whatsapp"),
        account,
        since,
        until,
        input.ad_id.as_deref().unwrap_or("all"),
    ]
    .iter()
    .map(|part| sanitize_job_id_part(part))
    .collect::<Vec<_>>()
    .join("-")
}

/// The workspace connection (a plain `Error`, 500, without one).
async fn auth_of(db: &Db, workspace_id: i64) -> Result<AdsAuth> {
    graph::workspace_auth(db, workspace_id)
        .await?
        .ok_or_else(|| graph::internal("Facebook Ads integration not found"))
}

/// `startAudienceSync`: creates the audience (unless one is given), enqueues
/// `syncRetargetAudience` and answers `{ customAudienceId, enqueued }`.
pub async fn start_sync(
    db: &Db,
    jobs: Option<&Arc<dyn Enqueuer>>,
    input: StartSync,
) -> Result<Value> {
    let auth = auth_of(db, input.workspace_id).await?;
    let audience = match input.custom_audience_id.clone() {
        Some(id) => id,
        None => {
            let body = json!({
                "name": input.audience_name.clone().unwrap_or_default(),
                "subtype": "CUSTOM",
                "customer_file_source": "USER_PROVIDED_ONLY",
            });
            let answer = g::call(
                "POST",
                g::url(
                    &format!(
                        "{}/{}/customaudiences",
                        auth.version,
                        seg(&input.ad_account_id)
                    ),
                    &[("access_token", auth.access_token.as_str())],
                ),
                Some(&body),
                None,
            )
            .await?;
            answer["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| BusinessError::from(GraphError::invalid("Invalid Graph response")))?
        }
    };
    let since = iso(input.since);
    let until = iso(input.until);
    let mut data = Map::new();
    data.insert("workspaceId".into(), json!(input.workspace_id.to_string()));
    data.insert("customAudienceId".into(), json!(audience));
    data.insert("segment".into(), json!(input.segment));
    for (key, value) in [
        ("adId", &input.ad_id),
        ("integrationWhatsappId", &input.integration_whatsapp_id),
        ("channel", &input.channel),
        ("integrationMessengerId", &input.integration_messenger_id),
        ("integrationInstagramId", &input.integration_instagram_id),
    ] {
        if let Some(value) = value {
            data.insert(key.into(), json!(value));
        }
    }
    data.insert("since".into(), json!(since));
    data.insert("until".into(), json!(until));
    let jobs = jobs.ok_or_else(|| graph::internal("the job queue is not configured"))?;
    let opts = JobOptions {
        job_id: Some(job_id(&input, &audience, &since, &until)),
        ..JobOptions::default()
    };
    enqueue_integration_job(
        jobs.as_ref(),
        IntegrationJob {
            action: IntegrationJobAction::SyncRetargetAudience,
            data: Value::Object(data),
        },
        Some(opts),
    )
    .await
    .map_err(|e| graph::internal(&e.to_string()))?;
    Ok(json!({ "customAudienceId": audience, "enqueued": true }))
}

fn sha256_hex(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `normalizePhoneNumber(phone, undefined, fallbackCountry)`.
fn normalize_phone(phone: &str, fallback_country: Option<&str>) -> Option<String> {
    let region = fallback_country
        .map(|c| c.trim().to_uppercase())
        .filter(|c| c.len() == 2 && c.chars().all(|ch| ch.is_ascii_uppercase()));
    let parse = |region: Option<&str>| {
        crate::contact::phone::parse(phone.trim(), region)
            .filter(|n| n.is_valid())
            .map(|n| {
                n.format()
                    .mode(phonenumber::Mode::E164)
                    .to_string()
                    .replacen('+', "", 1)
            })
    };
    if let Some(parsed) = parse(None).or_else(|| region.as_deref().and_then(|r| parse(Some(r)))) {
        return Some(parsed);
    }
    let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty() && !digits.starts_with('0')).then_some(digits)
}

/// `buildBulkHashedAudiencePayload`: `None` when no contact has a usable
/// phone or email.
pub fn bulk_payload(
    contacts: &[(Option<String>, Option<String>)],
    fallback_country: Option<&str>,
) -> Option<Value> {
    let identities: Vec<(Option<String>, Option<String>)> = contacts
        .iter()
        .filter(|(email, phone)| email.is_some() || phone.is_some())
        .filter_map(|(email, phone)| {
            let phone = phone
                .as_deref()
                .filter(|p| !p.is_empty())
                .and_then(|p| normalize_phone(p, fallback_country))
                .map(|p| sha256_hex(&p));
            let email = email
                .as_deref()
                .map(|e| e.trim().to_lowercase())
                .filter(|e| !e.is_empty())
                .map(|e| sha256_hex(&e));
            (phone.is_some() || email.is_some()).then_some((phone, email))
        })
        .collect();
    if identities.is_empty() {
        return None;
    }
    if identities.iter().all(|(_, email)| email.is_none()) {
        let data: Vec<Value> = identities
            .iter()
            .filter_map(|(phone, _)| phone.as_ref().map(|p| json!([p])))
            .collect();
        return Some(json!({ "schema": ["PHONE"], "data": data }));
    }
    let data: Vec<Value> = identities
        .iter()
        .map(|(phone, email)| {
            json!([
                phone.clone().unwrap_or_default(),
                email.clone().unwrap_or_default()
            ])
        })
        .collect();
    Some(json!({ "schema": ["PHONE", "EMAIL"], "data": data }))
}

/// `bulkSyncHashedAudienceUsers` (`add`): one `POST /<audience>/users` per
/// 5000 contacts.
async fn flush(
    auth: &AdsAuth,
    audience: &str,
    contacts: &[(Option<String>, Option<String>)],
    fallback_country: Option<&str>,
) -> std::result::Result<usize, GraphError> {
    let mut received = 0;
    for chunk in contacts.chunks(BATCH) {
        let Some(payload) = bulk_payload(chunk, fallback_country) else {
            continue;
        };
        let rows = payload["data"].as_array().map_or(0, Vec::len);
        if rows == 0 {
            continue;
        }
        g::call(
            "POST",
            g::url(
                &format!("{}/{}/users", auth.version, seg(audience)),
                &[("access_token", auth.access_token.as_str())],
            ),
            Some(&json!({ "payload": payload })),
            None,
        )
        .await?;
        received += rows;
    }
    Ok(received)
}

fn opt_id(data: &Value, key: &str) -> Option<i64> {
    data[key].as_str().and_then(|s| s.parse().ok())
}

/// `handleSyncRetargetAudience`: pages the segment (500 rows, keyset) and
/// uploads in batches of 5000. A Graph 190 marks the workspace connection
/// invalid and stops; a 429 / 5xx / network error is retried (`Err`);
/// another Graph error stops the job.
pub async fn sync(db: &Db, data: &Value) -> Result<()> {
    let workspace_id = opt_id(data, "workspaceId").unwrap_or_default();
    if crate::broadcast::send::is_blocked_workspace(db, workspace_id).await? {
        return Ok(());
    }
    let auth = auth_of(db, workspace_id).await?;
    let country: Option<String> = tx::read(db, move |conn| {
        Ok(conn
            .query_row(
                "SELECT \"targetCountry\" FROM \"Workspace\" WHERE \"id\" = ?1",
                [workspace_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    })
    .await?;
    let audience = data["customAudienceId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let date = |key: &str| data[key].as_str().and_then(chatbotx_core::js::parse_date);
    let range = DateRangeInput {
        workspace_id,
        since: date("since").unwrap_or_default(),
        until: date("until").unwrap_or_default(),
        integration_whatsapp_id: opt_id(data, "integrationWhatsappId"),
        channel: data["channel"].as_str().map(str::to_owned),
        integration_messenger_id: opt_id(data, "integrationMessengerId"),
        integration_instagram_id: opt_id(data, "integrationInstagramId"),
        all_channels: false,
        timezone: None,
    };
    let mut pending: Vec<(Option<String>, Option<String>)> = Vec::new();
    let mut after_id = None;
    let mut received = 0;
    let outcome: std::result::Result<(), GraphError> = async {
        loop {
            let input = ExportSegmentInput {
                range: range.clone(),
                segment: data["segment"].as_str().unwrap_or_default().to_owned(),
                ad_id: data["adId"].as_str().map(str::to_owned),
                after_id,
                limit: PAGE_SIZE,
            };
            let page = tx::read(db, move |conn| {
                Ok(events::list_export_segment_rows(conn, &input)?)
            })
            .await
            .map_err(|e| GraphError {
                status: 500,
                code: None,
                subcode: None,
                message: e.message(),
                network: true,
            })?;
            pending.extend(
                page.rows
                    .iter()
                    .map(|r| (r.email.clone(), r.phone_number.clone())),
            );
            while pending.len() >= BATCH {
                let batch: Vec<_> = pending.drain(..BATCH).collect();
                received += flush(&auth, &audience, &batch, country.as_deref()).await?;
            }
            after_id = page.rows.last().map(|r| r.id);
            if !(page.has_more && after_id.is_some()) {
                break;
            }
        }
        received += flush(&auth, &audience, &pending, country.as_deref()).await?;
        Ok(())
    }
    .await;
    match outcome {
        Ok(()) => {
            tracing::info!(workspace_id, %audience, received, "Completed CTWA retarget audience sync");
            Ok(())
        }
        Err(error) if error.code == Some(g::INVALID_TOKEN) => {
            tx::write(db, move |tx| {
                tx.execute(
                    "UPDATE \"IntegrationFacebookAds\" SET \"status\" = 'invalid' WHERE \"workspaceId\" = ?1",
                    [workspace_id],
                )?;
                Ok(())
            })
            .await?;
            tracing::error!(workspace_id, %audience, "Stopped CTWA retarget audience sync because Facebook Ads auth is invalid");
            Ok(())
        }
        Err(error) if error.network || error.status == 429 || error.status >= 500 => {
            Err(BusinessError::from(error))
        }
        Err(error) => {
            tracing::error!(workspace_id, %audience, message = %error.message, "Stopped CTWA retarget audience sync after terminal Facebook Ads error");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_hashed_payloads_and_job_ids() {
        let email = Some(" Lead@Parity.test ".to_owned());
        let payload = bulk_payload(&[(email, None)], None).unwrap();
        assert_eq!(payload["schema"], json!(["PHONE", "EMAIL"]));
        assert_eq!(payload["data"][0][0], "");
        assert_eq!(payload["data"][0][1], json!(sha256_hex("lead@parity.test")));
        let phones = bulk_payload(&[(None, Some("+1 415 555 2671".into()))], None).unwrap();
        assert_eq!(phones["data"], json!([[sha256_hex("14155552671")]]));
        assert_eq!(bulk_payload(&[(None, None)], None), None);
        assert_eq!(normalize_phone("0123", None), None);
        let input = StartSync {
            workspace_id: 7,
            segment: "leads".into(),
            ..StartSync::default()
        };
        assert_eq!(
            job_id(&input, "99", "2026-10-01T00:00:00.000Z", "x"),
            "ads-retarget-7-99-leads-whatsapp-all-accounts-2026-10-01T00_00_00_000Z-x-all"
        );
    }
}
