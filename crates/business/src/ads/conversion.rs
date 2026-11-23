//! `adsConversionService.evaluateConversionTrigger` / `ingestAutomaticEvent` /
//! `enqueueKeywordMatchedEvaluation` and `ads-conversion/shared.ts`
//! (`insertConversionEventOrRecover`, `withMetaAttributedTouch`,
//! `enqueueSendConversionEvent`).

use chatbotx_db::Db;
use chatbotx_db::repo::ads_conversion_event::{
    self as events, AdReferralInput, AdsConversionEventCreate, FindBySourceEventInput,
};
use chatbotx_db::repo::contact_referral_event::{self, FindLatestAdTouchInput};
use chatbotx_db::repo::models::{AdsConversionEvent, AdsConversionRule};
use chatbotx_db::repo::{find_many, iso, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_jobs::Enqueuer;
use chatbotx_jobs::queues::integration::{
    IntegrationJob, IntegrationJobAction, enqueue_integration_job,
};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `EvaluateConversionTriggerOccurrence`.
#[derive(Debug, Clone, PartialEq)]
pub enum Occurrence {
    TagApplied { tag_id: String },
    KeywordMatched { automated_response_id: String },
    ContactReplied { is_first_reply: bool },
}

fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.trim().to_owned()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

impl Occurrence {
    /// `evaluateConversionTriggerOccurrenceSchema.parse`.
    pub fn from_json(v: &Value) -> Option<Self> {
        match v["type"].as_str()? {
            "tagApplied" => Some(Occurrence::TagApplied {
                tag_id: id_text(&v["tagId"])?,
            }),
            "keywordMatched" => Some(Occurrence::KeywordMatched {
                automated_response_id: id_text(&v["automatedResponseId"])?,
            }),
            "contactReplied" => Some(Occurrence::ContactReplied {
                is_first_reply: v["isFirstReply"].as_bool()?,
            }),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Occurrence::TagApplied { tag_id } => json!({ "type": "tagApplied", "tagId": tag_id }),
            Occurrence::KeywordMatched {
                automated_response_id,
            } => json!({ "type": "keywordMatched", "automatedResponseId": automated_response_id }),
            Occurrence::ContactReplied { is_first_reply } => {
                json!({ "type": "contactReplied", "isFirstReply": is_first_reply })
            }
        }
    }
}

/// `triggerHandlers[trigger.type]` (all but `templateSent`).
pub fn trigger_matches(trigger: &Value, occurrence: &Occurrence) -> bool {
    let listed = |key: &str, id: &str| {
        trigger[key]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|v| id_text(v).as_deref() == Some(id)))
    };
    match (trigger["type"].as_str(), occurrence) {
        (Some("tagApplied"), Occurrence::TagApplied { tag_id }) => listed("tagIds", tag_id),
        (
            Some("keywordMatched"),
            Occurrence::KeywordMatched {
                automated_response_id,
            },
        ) => listed("automatedResponseIds", automated_response_id),
        (Some("contactReplied"), Occurrence::ContactReplied { is_first_reply }) => {
            trigger["firstReplyOnly"] != true || *is_first_reply
        }
        _ => false,
    }
}

/// `EvaluateConversionTriggerInput` (parsed).
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluateInput {
    pub workspace_id: i64,
    pub channel: String,
    pub integration_id: i64,
    pub contact_inbox_id: i64,
    pub occurrence: Occurrence,
}

fn rule_fk(channel: &str) -> Option<&'static str> {
    match channel {
        "whatsapp" => Some("integrationWhatsappId"),
        "messenger" => Some("integrationMessengerId"),
        "instagram" => Some("integrationInstagramId"),
        _ => None,
    }
}

/// `adsConversionRuleRepository.listByWorkspace({ channel, enabled: true, <fk> })`.
fn enabled_rules(
    conn: &Connection,
    workspace_id: i64,
    channel: &str,
    fk: &str,
    integration_id: i64,
) -> chatbotx_db::repo::Result<Vec<AdsConversionRule>> {
    find_many(
        conn,
        &format!(
            "WHERE \"workspaceId\" = ?1 AND \"channel\" = ?2 AND \"enabled\" = 1 AND \"{fk}\" = ?3 \
             ORDER BY \"createdAt\" ASC, \"id\" ASC"
        ),
        &[&workspace_id, &channel, &integration_id],
    )
}

/// `withMetaAttributedTouch`.
fn with_meta_attributed_touch(
    conn: &Connection,
    mut values: AdsConversionEventCreate,
) -> chatbotx_db::repo::Result<AdsConversionEventCreate> {
    values.referral_event_id = match values.contact_inbox_id {
        Some(contact_inbox_id) => contact_referral_event::find_latest_ad_touch_id(
            conn,
            &FindLatestAdTouchInput {
                workspace_id: values.workspace_id,
                contact_inbox_id,
                provider: "meta".into(),
                click_id: values.ctwa_clid.clone(),
                ad_id: values.ad_id.clone(),
            },
        )?,
        None => None,
    };
    Ok(values)
}

/// `enqueueSendConversionEvent` (`ads-conversion-send-{id}`).
pub async fn enqueue_send_conversion_event(
    jobs: &dyn Enqueuer,
    event: &AdsConversionEvent,
) -> Result<()> {
    enqueue_integration_job(
        jobs,
        IntegrationJob {
            action: IntegrationJobAction::SendConversionEvent,
            data: json!({
                "adsConversionEventId": event.id.to_string(),
                "workspaceId": event.workspace_id.to_string(),
            }),
        },
        Some(chatbotx_jobs::JobOptions {
            job_id: Some(format!("ads-conversion-send-{}", event.id)),
            ..chatbotx_jobs::JobOptions::default()
        }),
    )
    .await
    .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?;
    Ok(())
}

/// `insertConversionEventOrRecover`: insert (deduped by `sourceEventId`) and
/// enqueue the send; a deduped row still pending is re-enqueued.
async fn insert_or_recover(
    db: &Db,
    jobs: &dyn Enqueuer,
    values: AdsConversionEventCreate,
    lookup: FindBySourceEventInput,
) -> Result<Option<AdsConversionEvent>> {
    let (inserted, existing) = tx::write(db, move |tx| {
        let values = with_meta_attributed_touch(tx, values)?;
        if let Some(row) = events::insert_ignore_duplicate(tx, &values)? {
            return Ok((Some(row), None));
        }
        Ok((None, events::find_by_source_event_id(tx, &lookup)?))
    })
    .await?;
    if let Some(row) = &inserted {
        enqueue_send_conversion_event(jobs, row).await?;
    } else if let Some(row) = existing.filter(|e| e.capi_status == "pending") {
        enqueue_send_conversion_event(jobs, &row).await?;
    }
    Ok(inserted)
}

fn per_channel_ids(channel: &str, id: i64) -> (Option<i64>, Option<i64>, Option<i64>) {
    match channel {
        "whatsapp" => (Some(id), None, None),
        "messenger" => (None, Some(id), None),
        _ => (None, None, Some(id)),
    }
}

/// `adsConversionService.evaluateConversionTrigger`: WhatsApp gates on the
/// contact inbox's CTWA click id, Messenger / Instagram on ad-referral
/// attribution; each matching enabled rule makes one event per contact inbox
/// per UTC day.
pub async fn evaluate_conversion_trigger(
    db: &Db,
    jobs: &dyn Enqueuer,
    input: EvaluateInput,
) -> Result<Vec<AdsConversionEvent>> {
    let Some(fk) = rule_fk(&input.channel) else {
        // "facebook" is a dead channel value.
        return Ok(Vec::new());
    };
    let probe = input.clone();
    let found = tx::read(db, move |conn| {
        let ws = probe.workspace_id;
        let attribution = if probe.channel == "whatsapp" {
            events::find_attribution_by_contact_inbox(
                conn,
                ws,
                probe.integration_id,
                probe.contact_inbox_id,
            )?
            .and_then(|a| {
                let clid = a
                    .referral
                    .as_ref()
                    .and_then(|r| r.0["ctwaClid"].as_str().map(str::to_owned))
                    .filter(|c| !c.is_empty())?;
                Some((a.id, a.referral.map(|r| r.0), Some(a.waba_id), Some(clid)))
            })
        } else {
            let (_, messenger, instagram) = per_channel_ids(&probe.channel, probe.integration_id);
            events::find_attribution_by_ad_referral(
                conn,
                &AdReferralInput {
                    workspace_id: ws,
                    channel: probe.channel.clone(),
                    integration_messenger_id: messenger,
                    integration_instagram_id: instagram,
                    contact_inbox_id: probe.contact_inbox_id,
                },
            )?
            .map(|a| (a.id, a.referral.map(|r| r.0), None, None))
        };
        let Some(attribution) = attribution else {
            return Ok(None);
        };
        let rules = enabled_rules(conn, ws, &probe.channel, fk, probe.integration_id)?;
        Ok(Some((attribution, rules)))
    })
    .await?;
    let Some(((cib, referral, waba_id, ctwa_clid), rules)) = found else {
        return Ok(Vec::new());
    };
    let now = now_ms();
    let day = iso(now)[..10].to_owned();
    let ad_id = referral
        .as_ref()
        .and_then(|r| r["adId"].as_str().map(str::to_owned));
    let (whatsapp, messenger, instagram) = per_channel_ids(&input.channel, input.integration_id);
    let mut out = Vec::new();
    for rule in rules {
        if !trigger_matches(&rule.trigger.0, &input.occurrence) {
            continue;
        }
        let source_event_id = format!("rule-{}-inbox-{cib}-{day}", rule.id);
        let values = AdsConversionEventCreate {
            workspace_id: input.workspace_id,
            channel: Some(input.channel.clone()),
            integration_whatsapp_id: whatsapp,
            integration_messenger_id: messenger,
            integration_instagram_id: instagram,
            waba_id: waba_id.clone(),
            source: "rule".into(),
            event_type: rule.event_type.clone(),
            ctwa_clid: ctwa_clid.clone(),
            ad_id: ad_id.clone(),
            contact_inbox_id: Some(cib),
            occurred_at: now,
            source_event_id: source_event_id.clone(),
            capi_status: Some("pending".into()),
            ..AdsConversionEventCreate::default()
        };
        let lookup = FindBySourceEventInput {
            workspace_id: input.workspace_id,
            integration_whatsapp_id: whatsapp,
            channel: (input.channel != "whatsapp").then(|| input.channel.clone()),
            integration_messenger_id: messenger,
            integration_instagram_id: instagram,
            source: "rule".into(),
            source_event_id,
        };
        if let Some(event) = insert_or_recover(db, jobs, values, lookup).await? {
            out.push(event);
        }
    }
    Ok(out)
}

/// `IngestAutomaticAdsConversionEventInput.payload` (`automaticAdsConversionEventPayloadSchema`).
#[derive(Debug, Clone, PartialEq)]
pub struct AutomaticPayload {
    pub event_type: &'static str,
    pub id: String,
    pub occurred_at: i64,
    pub ctwa_clid: String,
    pub currency: Option<String>,
    pub value: Option<String>,
}

fn trimmed(v: &Value) -> Option<String> {
    v.as_str()
        .map(|s| chatbotx_core::js::trim(s).to_owned())
        .filter(|s| !s.is_empty())
}

impl AutomaticPayload {
    /// The schema parse plus `parseUnixTimestamp` (seconds → ms).
    pub fn parse(v: &Value) -> std::result::Result<Self, String> {
        let event_type = match v["event_name"].as_str() {
            Some("LeadSubmitted") => "lead",
            Some("Purchase") => "purchase",
            _ => return Err("Invalid automatic conversion event name".into()),
        };
        let id = trimmed(&v["id"]).ok_or("Invalid automatic conversion event id")?;
        let ctwa_clid = trimmed(&v["ctwa_clid"]).ok_or("Invalid automatic conversion ctwa_clid")?;
        let seconds = match &v["timestamp"] {
            Value::Number(n) => n.as_f64(),
            // ponytail: a plain f64 parse, not `parseFloat`'s numeric-prefix rule.
            Value::String(s) => chatbotx_core::js::trim(s).parse::<f64>().ok(),
            _ => None,
        }
        .filter(|s| s.is_finite())
        .ok_or("Invalid automatic conversion event timestamp")?;
        let custom = &v["custom_data"];
        let (currency, value) = if custom.is_null() {
            (None, None)
        } else {
            let currency = trimmed(&custom["currency"]).ok_or("Invalid custom_data.currency")?;
            let value = match &custom["value"] {
                Value::Number(n) => chatbotx_core::js::number_to_string(n.as_f64().unwrap_or(0.0)),
                other => trimmed(other).ok_or("Invalid custom_data.value")?,
            };
            (Some(currency), Some(value))
        };
        Ok(AutomaticPayload {
            event_type,
            id,
            occurred_at: (seconds * 1000.0) as i64,
            ctwa_clid,
            currency,
            value,
        })
    }
}

/// `adsConversionService.ingestAutomaticEvent` (WhatsApp only): the row is
/// attributed by its click id; `None` when deduped.
pub async fn ingest_automatic_event(
    db: &Db,
    workspace_id: i64,
    integration_whatsapp_id: i64,
    waba_id: String,
    payload: AutomaticPayload,
) -> Result<Option<AdsConversionEvent>> {
    tx::write(db, move |tx| {
        let attribution = events::find_attribution_by_ctwa_clid(
            tx,
            workspace_id,
            integration_whatsapp_id,
            &payload.ctwa_clid,
        )?;
        let values = AdsConversionEventCreate {
            workspace_id,
            channel: Some("whatsapp".into()),
            integration_whatsapp_id: Some(integration_whatsapp_id),
            waba_id: Some(waba_id),
            source: "automatic".into(),
            event_type: payload.event_type.into(),
            ctwa_clid: Some(payload.ctwa_clid),
            ad_id: attribution
                .as_ref()
                .and_then(|a| a.referral.as_ref())
                .and_then(|r| r.0["adId"].as_str().map(str::to_owned)),
            contact_inbox_id: attribution.as_ref().map(|a| a.id),
            currency: payload.currency,
            value: payload.value,
            occurred_at: payload.occurred_at,
            source_event_id: payload.id,
            capi_status: Some("pending".into()),
            ..AdsConversionEventCreate::default()
        };
        let values = with_meta_attributed_touch(tx, values)?;
        Ok(events::insert_ignore_duplicate(tx, &values)?)
    })
    .await
}

/// `findWorkspaceIntegrationByInboxId` (the id only).
pub fn integration_id_by_inbox(
    conn: &Connection,
    channel: &str,
    workspace_id: i64,
    inbox_id: i64,
) -> chatbotx_db::rusqlite::Result<Option<i64>> {
    let table = match channel {
        "whatsapp" => "IntegrationWhatsapp",
        "messenger" => "IntegrationMessenger",
        "instagram" => "IntegrationInstagram",
        _ => return Ok(None),
    };
    conn.query_row(
        &format!(
            "SELECT \"id\" FROM \"{table}\" WHERE \"inboxId\" = ?1 AND \"workspaceId\" = ?2 LIMIT 1"
        ),
        params![inbox_id, workspace_id],
        |r| r.get(0),
    )
    .optional()
}

/// `enqueueConversionTriggerJob`.
async fn enqueue_evaluation(
    jobs: &dyn Enqueuer,
    workspace_id: i64,
    channel: &str,
    integration_id: i64,
    contact_inbox_id: i64,
    occurrence: &Occurrence,
    job_id: String,
) -> Result<()> {
    enqueue_integration_job(
        jobs,
        IntegrationJob {
            action: IntegrationJobAction::EvaluateConversionTrigger,
            data: json!({
                "workspaceId": workspace_id.to_string(),
                "channel": channel,
                "integrationId": integration_id.to_string(),
                "contactInboxId": contact_inbox_id.to_string(),
                "occurrence": occurrence.to_json(),
            }),
        },
        Some(chatbotx_jobs::JobOptions {
            job_id: Some(job_id),
            ..chatbotx_jobs::JobOptions::default()
        }),
    )
    .await
    .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?;
    Ok(())
}

/// `enqueueKeywordMatchedEvaluation` (inside `safeEnqueue`: a failure is
/// logged, never thrown into the reply).
#[allow(clippy::too_many_arguments)]
pub async fn enqueue_keyword_matched_evaluation(
    db: &Db,
    jobs: &dyn Enqueuer,
    workspace_id: i64,
    channel: &str,
    inbox_id: i64,
    contact_inbox_id: i64,
    automated_response_id: i64,
    message_id: &str,
) {
    let owned = channel.to_owned();
    let run = async {
        let integration = tx::read(db, move |conn| {
            Ok(integration_id_by_inbox(
                conn,
                &owned,
                workspace_id,
                inbox_id,
            )?)
        })
        .await?;
        let Some(integration_id) = integration else {
            return Ok(());
        };
        enqueue_evaluation(
            jobs,
            workspace_id,
            channel,
            integration_id,
            contact_inbox_id,
            &Occurrence::KeywordMatched {
                automated_response_id: automated_response_id.to_string(),
            },
            format!("ads-conversion-evaluate-keyword-{message_id}"),
        )
        .await
    };
    if let Err(err) = run.await {
        tracing::warn!(
            err = %err.message(),
            operation = "keywordMatched",
            "Ads conversion trigger enqueue failed; primary operation unaffected"
        );
    }
}

/// `isAdsEligibleChannel`.
pub fn is_ads_eligible_channel(channel: &str) -> bool {
    matches!(channel, "whatsapp" | "messenger" | "instagram")
}
