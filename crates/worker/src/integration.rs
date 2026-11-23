//! `apps/worker/src/integration/{worker,flow-execution-key,channel-origin,
//! routing}.ts`: the generic part of the `integration` queue. The channel
//! and step specific actions belong to later units.

use chatbotx_core::utils::id::create_id;
use chatbotx_jobs::{JobCtx, JobError};
use serde_json::Value;

use crate::{Deps, str_of};

/// The random prefixes of `mintRandomFlowExecutionKey`.
const RANDOM_KEY_PREFIXES: &[&str] = &["flow-inline-", "integration-job-"];

/// `mintRandomFlowExecutionKey`.
pub fn mint_random_flow_execution_key(prefix: &str) -> String {
    format!("{prefix}{}", create_id())
}

/// `isDurableFlowExecutionKey`: a retry of the run reproduces the key.
pub fn is_durable_flow_execution_key(key: Option<&str>) -> bool {
    key.is_some_and(|key| {
        !key.is_empty() && !RANDOM_KEY_PREFIXES.iter().any(|p| key.starts_with(p))
    })
}

/// `getFlowExecutionKey(job)`: the job id plus a value fixed across its
/// retries. The row id stands in for BullMQ's `job.timestamp` (it differs
/// when a job id is reused).
fn flow_execution_key(job: &JobCtx) -> String {
    format!("{}:{}", job.job_id, job.id)
}

const CHANNEL_ORIGINATED_JOB_TYPES: &[&str] = &[
    "incomingMessage",
    "incomingMessageEcho",
    "incomingComment",
    "runFlowPostback",
    "runFlowQuickReply",
    "runChallenge",
    "runRef",
    "processAutomatedResonse",
    "channelLabelChange",
    "processCommentAutomation",
    "threadControlEvent",
];

const INBOUND_WEBHOOK_JOB_TYPES: &[&str] = &[
    "contactMarkAsRead",
    "channelAudienceEvent",
    "messageReaction",
    "messageStatus",
    "deleteIncomingMessage",
    "updateIncomingComment",
    "deleteIncomingComment",
    "processLeadgen",
    "processMarketingMessageWebhook",
    "tiktokHighIntentComment",
    "adsAutomaticEvent",
    "providerAccountEvent",
    "whatsappIdentityChange",
    "whatsappCallEvent",
    "coexistWhatsappBuffer",
];

/// A bigint id sent as a string (or a number).
fn id_field(data: &Value, key: &str) -> Option<i64> {
    match &data[key] {
        Value::String(s) => s.parse::<i64>().ok(),
        other => other.as_i64(),
    }
}

/// `isChannelOriginatedJob(jobData)`.
pub fn is_channel_originated_job(job_data: &Value) -> bool {
    let kind = str_of(job_data, "type").unwrap_or("");
    CHANNEL_ORIGINATED_JOB_TYPES.contains(&kind)
        || (kind == "sendFlow" && str_of(&job_data["data"], "origin") == Some("channel"))
}

/// `isInboundWebhookJob(jobData)`.
pub fn is_inbound_webhook_job(job_data: &Value) -> bool {
    INBOUND_WEBHOOK_JOB_TYPES.contains(&str_of(job_data, "type").unwrap_or(""))
        || is_channel_originated_job(job_data)
}

/// `IncomingRoutingDecision` (the caller keeps the conversation).
#[derive(Debug, Clone, PartialEq)]
pub enum IncomingRouting {
    None,
    Challenge(Value),
    AutomatedResponse,
    HandoffReentry,
}

fn pending_challenge(additional_attributes: &Value) -> Option<&Value> {
    additional_attributes
        .get("challenge")
        .filter(|c| !c.is_null())
}

/// `shouldRunStoryMentionAutomation`.
pub async fn should_run_story_mention_automation(
    additional_attributes: &Value,
    is_conversation_active: impl AsyncFnOnce() -> bool,
) -> bool {
    if pending_challenge(additional_attributes).is_some() {
        return false;
    }
    is_conversation_active().await
}

/// `resolveIncomingTextRouting`.
pub async fn resolve_incoming_text_routing(
    additional_attributes: &Value,
    has_actionable_input: bool,
    has_text: bool,
    has_keyword_media: bool,
    is_conversation_active: impl AsyncFnOnce() -> bool,
) -> IncomingRouting {
    if !has_actionable_input {
        return IncomingRouting::None;
    }
    let challenge = pending_challenge(additional_attributes);
    if !is_conversation_active().await {
        return if challenge.is_none() && has_text {
            IncomingRouting::HandoffReentry
        } else {
            IncomingRouting::None
        };
    }
    if let Some(challenge) = challenge {
        return IncomingRouting::Challenge(challenge.clone());
    }
    if has_text || has_keyword_media {
        IncomingRouting::AutomatedResponse
    } else {
        IncomingRouting::None
    }
}

/// The `integration` queue processor (`runWorker` switch): `sendFlow` and
/// `sendSequenceFlow`.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("").to_owned();
    match kind.as_str() {
        "sendFlow" => {
            let data = &job.data["data"];
            let key = str_of(data, "flowExecutionKey")
                .map(str::to_owned)
                .unwrap_or_else(|| flow_execution_key(&job));
            crate::flow::run_flow_node(&deps, data, key).await?;
        }
        chatbotx_jobs::sequence::consumer::SEND_SEQUENCE_FLOW_ACTION => {
            crate::sequence::handle_send_sequence_flow(&deps, &job).await?;
        }
        crate::steps::delay::RESUME_WAIT => {
            let key = flow_execution_key(&job);
            crate::steps::delay::run_wait_resume(&deps, &job.data["data"], key).await?;
        }
        crate::steps::delay::RESUME_QUICK_REPLY_FOLLOW_UP => {
            crate::steps::delay::run_quick_reply_follow_up_resume(&deps, &job.data["data"]).await?;
        }
        crate::steps::delay::RESUME_FOLLOW_UP => {
            crate::steps::delay::run_follow_up_resume(&deps, &job.data["data"]).await?;
        }
        crate::flow_action::RUN_FLOW_POSTBACK | crate::flow_action::RUN_FLOW_QUICK_REPLY => {
            let key = flow_execution_key(&job);
            crate::flow_action::run_flow_action(&deps, &kind, &job.data["data"], key).await?;
        }
        crate::ref_link::RUN_REF => crate::ref_link::run_ref(&deps, &job.data["data"]).await?,
        crate::challenge::RUN_CHALLENGE => {
            crate::challenge::run_challenge(&deps, &job.data["data"]).await?
        }
        crate::challenge::RESOLVE_STEP_INPUT => {
            crate::challenge::resolve_step_input(&deps, &job.data["data"]).await?
        }
        crate::challenge::RESUME_STEP_INPUT_TIMEOUT => {
            crate::challenge::resume_step_input_timeout(&deps, &job.data["data"]).await?
        }
        crate::heavy::RESUME_HEAVY_STEP => {
            crate::heavy::resume_heavy_step(&deps, &job.data["data"]).await?;
        }
        "aiHandoverBulkToggle" => {
            crate::ai_handover::run_bulk_toggle(&deps, &job.data["data"]).await?;
        }
        // `incomingMessageEcho` (a message the account itself sent) takes the
        // same path; an echo never matches the contact-input rules.
        crate::inbound::INCOMING_MESSAGE | "incomingMessageEcho" => {
            crate::inbound::incoming_message(&deps, &job.data["data"]).await?
        }
        crate::inbound_events::CONTACT_MARK_AS_READ => {
            crate::inbound_events::contact_mark_as_read(&deps, &job.data["data"]).await?
        }
        crate::inbound_events::MESSAGE_REACTION => {
            crate::inbound_events::message_reaction(&deps, &job.data["data"]).await?
        }
        crate::inbound_events::DELETE_INCOMING_MESSAGE => {
            crate::inbound_events::delete_incoming(&deps, &job.data["data"], "messageId").await?
        }
        crate::inbound_events::DELETE_INCOMING_COMMENT => {
            crate::inbound_events::delete_incoming(&deps, &job.data["data"], "commentId").await?
        }
        "incomingComment" => crate::low::incoming_comment(&deps, &job.data["data"]).await?,
        crate::leaderboard::CHANNEL_AUDIENCE_EVENT => {
            crate::leaderboard::channel_audience_event(&deps, &job.data["data"]).await?
        }
        "tiktokHighIntentComment" => {
            crate::low::tiktok_high_intent_comment(&deps, &job.data["data"]).await?
        }
        crate::low::PROCESS_COMMENT_AUTOMATION | crate::low::DEFERRED_COMMENT_PRIVATE_REPLY => {
            crate::low::process_comment_automation_job(&deps, &job.data["data"]).await?
        }
        crate::inbound_events::UPDATE_INCOMING_COMMENT => {
            crate::inbound_events::update_incoming_comment(&deps, &job.data["data"]).await?
        }
        crate::inbox_labels::CHANNEL_LABEL_CHANGE => {
            crate::inbox_labels::handle(&deps, &job.data["data"]).await?
        }
        crate::inbound::MESSAGE_STATUS => {
            crate::inbound::message_status(&deps, &job.data["data"]).await?
        }
        crate::thread_control::EVENT_JOB => {
            let run = crate::thread_control::EventJob::of(&job);
            crate::thread_control::receive_event(&deps, &job.data["data"], &run).await?
        }
        crate::thread_control::TAKE_BACK_JOB => {
            crate::thread_control::run_take_back(&deps, &job.data["data"]).await?
        }
        chatbotx_business::thread_control::RELEASE_JOB => {
            crate::thread_control::release_owned_thread(&deps, &job.data["data"]).await?
        }
        crate::lead_ads::PROCESS_LEADGEN => crate::lead_ads::process_leadgen(&deps, &job).await?,
        crate::marketing_message::PROCESS_MARKETING_MESSAGE_WEBHOOK => {
            crate::marketing_message::process_webhook(&deps, &job.data["data"]).await?
        }
        crate::template_flow_response::CAPTURE_TEMPLATE_FLOW_RESPONSE => {
            crate::template_flow_response::capture(&deps, &job.data["data"]).await?
        }
        crate::inbound_cart::REPLY_TO_INBOUND_CART => {
            crate::inbound_cart::reply_to_inbound_cart(&deps, &job.data["data"]).await?
        }
        crate::order_payment::CONFIRM_ORDER_PAYMENT => {
            crate::order_payment::confirm_order_payment(&deps, &job.data["data"]).await?
        }
        crate::coexist_whatsapp::BUFFER_JOB => {
            crate::coexist_whatsapp::buffer(&deps, &job.data["data"]).await?
        }
        crate::coexist_whatsapp::FLUSH_JOB => {
            crate::coexist_whatsapp::flush(&deps, &job.data["data"]).await?
        }
        // `handleSyncRetargetAudience` (a failure is retried).
        "syncRetargetAudience" => {
            chatbotx_business::ads::retarget::sync(&deps.db, &job.data["data"])
                .await
                .map_err(|e| JobError::retryable(e.to_string()))?
        }
        // `handleSendGoogleAdsConversion` (`withBlockedOwnerGuard`).
        "evaluateConversionTrigger" => {
            let data = &job.data["data"];
            let (
                Some(workspace_id),
                Some(integration_id),
                Some(contact_inbox_id),
                Some(occurrence),
            ) = (
                id_field(data, "workspaceId"),
                id_field(data, "integrationId"),
                id_field(data, "contactInboxId"),
                chatbotx_business::ads::conversion::Occurrence::from_json(&data["occurrence"]),
            )
            else {
                return Err(JobError::unrecoverable(
                    "evaluateConversionTrigger: bad job data",
                ));
            };
            if crate::is_blocked_workspace(&deps.db, &workspace_id.to_string()).await? {
                return Ok(Value::Null);
            }
            let input = chatbotx_business::ads::conversion::EvaluateInput {
                workspace_id,
                channel: str_of(data, "channel").unwrap_or_default().to_owned(),
                integration_id,
                contact_inbox_id,
                occurrence,
            };
            chatbotx_business::ads::conversion::evaluate_conversion_trigger(
                &deps.db,
                deps.jobs.as_ref(),
                input,
            )
            .await
            .map_err(|e| JobError::retryable(e.message()))?;
        }
        "sendConversionEvent" | "sendMetaCapiEvent" => {
            let data = &job.data["data"];
            let key = if kind == "sendConversionEvent" {
                "adsConversionEventId"
            } else {
                "metaCapiEventId"
            };
            let (Some(event_id), Some(workspace_id)) =
                (id_field(data, key), id_field(data, "workspaceId"))
            else {
                return Err(JobError::unrecoverable(format!("{kind}: bad job data")));
            };
            if crate::is_blocked_workspace(&deps.db, &workspace_id.to_string()).await? {
                return Ok(Value::Null);
            }
            let up = chatbotx_business::connection::credentials::HttpUpstream;
            let send = chatbotx_business::meta_capi_send::SendDeps {
                db: &deps.db,
                up: &up,
                events: deps.events.as_ref(),
            };
            if kind == "sendConversionEvent" {
                chatbotx_business::meta_capi_send::send_conversion_event(
                    &send,
                    event_id,
                    workspace_id,
                )
                .await
            } else {
                chatbotx_business::meta_capi_send::send_meta_capi_event(
                    &send,
                    event_id,
                    workspace_id,
                )
                .await
            }
            .map_err(JobError::retryable)?;
        }
        "adsAutomaticEvent" => {
            use chatbotx_business::ads::conversion::{
                AutomaticPayload, enqueue_send_conversion_event, ingest_automatic_event,
            };
            let data = &job.data["data"];
            let (Some(phone), Some(waba)) = (str_of(data, "phoneNumberId"), str_of(data, "wabaId"))
            else {
                return Err(JobError::unrecoverable("adsAutomaticEvent: bad job data"));
            };
            let payload =
                AutomaticPayload::parse(&data["payload"]).map_err(JobError::unrecoverable)?;
            let (p, w) = (phone.to_owned(), waba.to_owned());
            let integration = deps
                .db
                .read(move |c| {
                    chatbotx_db::repo::integration_whatsapp::find_by_phone_number_id(
                        c,
                        &p,
                        Some(&w),
                    )
                    .map_err(crate::repo_err)
                })
                .await
                .map_err(crate::db_fail)?;
            let Some(integration) = integration else {
                tracing::warn!(
                    phone_number_id = phone,
                    waba_id = waba,
                    source_event_id = payload.id,
                    "Whatsapp automatic event skipped: integration not found"
                );
                return Ok(Value::Null);
            };
            if crate::is_blocked_workspace(&deps.db, &integration.workspace_id.to_string()).await? {
                return Ok(Value::Null);
            }
            let event = ingest_automatic_event(
                &deps.db,
                integration.workspace_id,
                integration.id,
                waba.to_owned(),
                payload,
            )
            .await
            .map_err(|e| JobError::retryable(e.message()))?;
            if let Some(event) = event {
                enqueue_send_conversion_event(deps.jobs.as_ref(), &event)
                    .await
                    .map_err(|e| JobError::retryable(e.message()))?;
            }
        }
        "sendGoogleAdsConversion" => {
            let data = &job.data["data"];
            let id = |key: &str| match &data[key] {
                Value::String(s) => s.parse::<i64>().ok(),
                other => other.as_i64(),
            };
            let (Some(event_id), Some(workspace_id)) =
                (id("googleAdsConversionEventId"), id("workspaceId"))
            else {
                return Err(JobError::unrecoverable(
                    "sendGoogleAdsConversion: bad job data",
                ));
            };
            let input = chatbotx_business::google_ads_conversion::DeliverInput {
                event_id,
                workspace_id,
                attempt: data["attempt"].as_i64().unwrap_or_default(),
                is_last_in_job_attempt: job.is_final_attempt(),
            };
            if crate::is_blocked_workspace(&deps.db, &workspace_id.to_string()).await? {
                return Ok(Value::Null);
            }
            let up = chatbotx_business::connection::credentials::HttpUpstream;
            let matching = MatchingTemplates(&deps);
            let delivery = chatbotx_business::google_ads_conversion::DeliverDeps {
                db: &deps.db,
                up: &up,
                jobs: deps.jobs.as_ref(),
                events: deps.events.as_ref(),
                app_url: &deps.app_url,
                matching: Some(&matching),
            };
            chatbotx_business::google_ads_conversion::deliver(&delivery, input)
                .await
                .map_err(JobError::retryable)?
        }
        crate::whatsapp_call_webhook::NATIVE_RECORDING_FETCH_JOB => {
            crate::whatsapp_call_media::native_recording_fetch(&deps, &job.data["data"]).await?
        }
        crate::whatsapp_call_webhook::NATIVE_TRANSCRIPT_FETCH_JOB => {
            crate::whatsapp_call_media::native_transcript_fetch(&deps, &job.data["data"]).await?
        }
        crate::whatsapp_call_media::RECORDING_READY_JOB => {
            crate::whatsapp_call_media::recording_ready(&deps, &job.data["data"]).await?
        }
        crate::whatsapp_call_webhook::EVENT_JOB => {
            crate::whatsapp_call::handle_event(&deps, &job.data["data"]).await?
        }
        crate::coexist_messenger::JOB => {
            crate::coexist_messenger::run(&deps, &job.data["data"]).await?
        }
        crate::coexist_instagram::JOB => {
            crate::coexist_instagram::run(&deps, &job.data["data"]).await?
        }
        crate::channels::whatsapp_webhook::WHATSAPP_IDENTITY_CHANGE => {
            crate::provider_account::whatsapp_identity_change(&deps, &job.data["data"]).await?
        }
        crate::channels::whatsapp_webhook::PROVIDER_ACCOUNT_EVENT => {
            crate::provider_account::provider_account_event(&deps, &job.data["data"]).await?
        }
        crate::contact_profile::UPDATE_CONTACT_AVATAR => {
            crate::contact_profile::update_contact_avatar(&deps, &job.data["data"]).await?
        }
        "recordEmailTrackingTransition" => {
            crate::steps::email_marketing::record_email_tracking_transition(
                &deps,
                &job.data["data"],
            )
            .await?
        }
        crate::contact_scan::CONTACT_SCAN => {
            crate::contact_scan::run(&deps, &job.data["data"]).await?
        }
        crate::store_sync::SHOPIFY_SYNC | crate::store_sync::SHOPIFY_WEBHOOK => {
            crate::store_sync::shopify(&deps, &kind, &job.data["data"]).await?
        }
        crate::store_sync::WOOCOMMERCE_SYNC | crate::store_sync::WOOCOMMERCE_WEBHOOK => {
            crate::store_sync::woocommerce(&deps, &kind, &job.data["data"]).await?
        }
        other => {
            // ponytail: every other integration action is a later unit
            // (inbound channels Q.I*, steps, sequences, ...).
            return Err(JobError::unrecoverable(format!(
                "IntegrationJobAction action is not defined: {other}"
            )));
        }
    }
    Ok(Value::Null)
}

/// `resolveMatchingTemplates`: the customer-matching `{{variable}}`s against
/// the current contact. A value that is blank or still holds `{{` is no value.
/// Nothing here logs the values.
struct MatchingTemplates<'a>(&'a Deps);

impl chatbotx_business::google_ads_conversion::MatchingResolver for MatchingTemplates<'_> {
    fn resolve<'a>(
        &'a self,
        workspace_id: i64,
        contact_inbox_id: i64,
        email: Option<String>,
        phone: Option<String>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = chatbotx_business::google_ads_conversion::MatchingValues,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            use chatbotx_db::repo::Model as _;
            use chatbotx_db::repo::models::ContactInbox;
            use chatbotx_db::rusqlite::OptionalExtension as _;
            let inbox = self
                .0
                .db
                .read(move |c| {
                    Ok(c.query_row(
                        &format!(
                            r#"{} WHERE "id" = ?1 AND "inboxId" IN (SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?2)"#,
                            ContactInbox::select_sql()
                        ),
                        [contact_inbox_id, workspace_id],
                        ContactInbox::from_row,
                    )
                    .optional()?)
                })
                .await
                .map_err(|e| e.to_string())?;
            let Some(inbox) = inbox else {
                return Ok(None);
            };
            let templates = serde_json::json!({ "email": email, "phone": phone });
            let resolved = match &self.0.variables {
                Some(variables) => variables.resolve_contact(&inbox, &templates).await?,
                None => templates,
            };
            let usable = |key: &str| {
                resolved[key]
                    .as_str()
                    .filter(|v| !v.contains("{{") && !v.trim().is_empty())
                    .map(str::to_owned)
            };
            Ok(Some((usable("email"), usable("phone"))))
        })
    }
}

#[cfg(test)]
mod tests {
    //! `flow-execution-key.test.ts`, `integration-channel-origin.test.ts`
    //! and `integration-routing.test.ts`.
    use std::cell::Cell;

    use serde_json::json;

    use super::*;

    #[test]
    fn a_job_id_with_its_creation_time_is_durable() {
        assert!(is_durable_flow_execution_key(Some("42:1760000000000")));
    }

    #[test]
    fn a_minted_key_is_not_durable() {
        for prefix in ["flow-inline-", "integration-job-"] {
            assert!(!is_durable_flow_execution_key(Some(
                &mint_random_flow_execution_key(prefix)
            )));
        }
    }

    #[test]
    fn a_missing_or_empty_key_is_not_durable() {
        assert!(!is_durable_flow_execution_key(None));
        assert!(!is_durable_flow_execution_key(Some("")));
    }

    #[test]
    fn allows_known_channel_originated_job_types() {
        assert!(is_channel_originated_job(
            &json!({ "type": "runRef", "data": {} })
        ));
        assert!(is_channel_originated_job(
            &json!({ "type": "runChallenge", "data": {} })
        ));
    }

    #[test]
    fn treats_a_channel_echo_as_channel_originated() {
        let job = json!({ "type": "incomingMessageEcho", "data": {} });
        assert!(is_channel_originated_job(&job));
        assert!(is_inbound_webhook_job(&job));
    }

    #[test]
    fn allows_send_flow_only_when_origin_is_channel() {
        assert!(is_channel_originated_job(
            &json!({ "type": "sendFlow", "data": { "origin": "channel" } })
        ));
        assert!(!is_channel_originated_job(
            &json!({ "type": "sendFlow", "data": {} })
        ));
    }

    #[test]
    fn rejects_non_channel_job_types() {
        assert!(!is_channel_originated_job(
            &json!({ "type": "sendSequenceFlow", "data": {} })
        ));
    }

    #[test]
    fn recognizes_webhook_jobs() {
        let job = |kind: &str| json!({ "type": kind, "data": {} });
        assert!(is_inbound_webhook_job(&job("contactMarkAsRead")));
        assert!(is_inbound_webhook_job(&job("messageReaction")));
        assert!(!is_inbound_webhook_job(&job("sendSequenceFlow")));
        assert!(is_inbound_webhook_job(
            &json!({ "type": "sendFlow", "data": { "origin": "channel" } })
        ));
        assert!(is_inbound_webhook_job(&job("channelAudienceEvent")));
        assert!(!is_channel_originated_job(&job("contactMarkAsRead")));
    }

    fn challenge() -> Value {
        json!({ "challenge": { "type": "step", "data": { "stepId": "step-1" } } })
    }

    async fn route(
        attrs: &Value,
        actionable: bool,
        text: bool,
        media: bool,
        active: bool,
    ) -> IncomingRouting {
        resolve_incoming_text_routing(attrs, actionable, text, media, async || active).await
    }

    #[tokio::test]
    async fn resolves_incoming_text_routing() {
        let none = json!({});
        let pending = IncomingRouting::Challenge(challenge()["challenge"].clone());
        // does not route pending challenges when bot automation is inactive
        assert_eq!(
            route(&challenge(), true, true, false, false).await,
            IncomingRouting::None
        );
        // routes pending challenges after bot automation is confirmed active
        assert_eq!(route(&challenge(), true, true, false, true).await, pending);
        // routes an attachment-only reply to a pending challenge
        assert_eq!(route(&challenge(), true, false, false, true).await, pending);
        assert_eq!(
            route(&none, true, true, false, true).await,
            IncomingRouting::AutomatedResponse
        );
        assert_eq!(
            route(&none, true, false, false, true).await,
            IncomingRouting::None
        );
        assert_eq!(
            route(&none, true, true, false, false).await,
            IncomingRouting::HandoffReentry
        );
        assert_eq!(
            route(&none, true, false, false, false).await,
            IncomingRouting::None
        );
        assert_eq!(
            route(&none, true, false, true, true).await,
            IncomingRouting::AutomatedResponse
        );
        assert_eq!(
            route(&none, true, false, true, false).await,
            IncomingRouting::None
        );
    }

    #[tokio::test]
    async fn skips_messages_with_no_actionable_input_without_checking_bot_automation() {
        let called = Cell::new(false);
        let result = resolve_incoming_text_routing(&json!({}), false, false, false, async || {
            called.set(true);
            true
        })
        .await;
        assert_eq!(result, IncomingRouting::None);
        assert!(!called.get());
    }

    #[tokio::test]
    async fn story_mention_automation() {
        assert!(should_run_story_mention_automation(&json!({}), async || true).await);
        let called = Cell::new(false);
        assert!(
            !should_run_story_mention_automation(&challenge(), async || {
                called.set(true);
                true
            })
            .await
        );
        assert!(!called.get());
        assert!(!should_run_story_mention_automation(&json!({}), async || false).await);
    }
}
