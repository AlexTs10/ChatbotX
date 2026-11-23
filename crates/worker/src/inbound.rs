//! The shared inbound pipeline every channel reuses
//! (`apps/worker/src/integration/handlers/{incoming-message,received-message,
//! message-status}.ts`): a channel webhook queues `incomingMessage` /
//! `messageStatus` on the `integration` queue with `{ integrationType,
//! integrationIdentifier, payload }`; this module resolves the integration,
//! lets the channel parse the payload ([`Channel::receive_message`] /
//! [`Channel::message_status`]), then finds or creates the contact, contact
//! inbox and conversation, stores the message (deduplicated by `sourceId`),
//! publishes `messageCreated` and routes the message (keywords → AI through
//! the `ai-agent` queue, flow postbacks / quick replies, `ref`).

use std::sync::Arc;

use chatbotx_business::message::{contact_inbox_json, conversation_json, message_json};
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::message::{self as repo_message, CreateAttachmentInput, CreateMessageInput};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_jobs::options::Dedup;
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use chatbotx_kv::SystemClock;
use chatbotx_kv::durable::DurableKv;
use chatbotx_rt::EventType;
use serde_json::{Value, json};

use crate::channel::{InboundInbox, IncomingContact, IncomingMessage, Received};
use crate::thread_control::record_inbound;
use crate::{Deps, db_fail, fail, str_of};
use chatbotx_business::thread_control as tc;

/// `IntegrationJobAction.incomingMessage`.
pub const INCOMING_MESSAGE: &str = "incomingMessage";
/// `IntegrationJobAction.messageStatus`.
pub const MESSAGE_STATUS: &str = "messageStatus";
/// `AIJobAction.processAutomatedResponse`.
pub const PROCESS_AUTOMATED_RESPONSE: &str = "processAutomatedResponse";

/// `AUTOMATED_RESPONSE_DELAY_SECONDS` / `AUTOMATED_RESPONSE_TTL_SECONDS`
/// (default 2 s each).
fn automated_response_timing() -> (u64, u64) {
    let read = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(2)
    };
    (
        read("AUTOMATED_RESPONSE_DELAY_SECONDS"),
        read("AUTOMATED_RESPONSE_TTL_SECONDS"),
    )
}

/// `SMART_RESPONSE_DELAY_OPTIONS`.
const SMART_RESPONSE_DELAY_OPTIONS: [i64; 10] = [3, 5, 10, 15, 20, 30, 45, 60, 120, 180];

/// `isSmartDelayEligible` + `resolveAutomatedResponseTiming`: the
/// workspace's Smart Delay when it is a valid option, the message matches no
/// keyword rule and an async responder (default AI agent or Global Router)
/// exists. `None` = the default timing.
async fn smart_delay_seconds(
    deps: &Deps,
    workspace_id: i64,
    text: Option<&str>,
    attachment_file_types: &[String],
) -> Result<Option<u64>, String> {
    use chatbotx_business::keyword_match::{MatchInput, matches_any_keyword_rule};
    let (delay, has_router) = deps
        .db
        .read(move |conn| {
            let delay: Option<i64> = conn
                .query_row(
                    r#"SELECT "smartResponseDelaySeconds" FROM "Workspace" WHERE "id" = ?1"#,
                    [workspace_id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            let router = conn
                .query_row(
                    r#"SELECT 1 FROM "DecisionProfile" WHERE "workspaceId" = ?1 AND "globalRouterEnabled" = 1 LIMIT 1"#,
                    [workspace_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            Ok((delay, router))
        })
        .await
        .map_err(|e| e.to_string())?;
    let Some(delay) = delay.filter(|d| SMART_RESPONSE_DELAY_OPTIONS.contains(d)) else {
        return Ok(None);
    };
    let has_input = text.is_some() || !attachment_file_types.is_empty();
    if has_input {
        let rules = crate::ai_agent::inbound_rules(deps, workspace_id)
            .await
            .map_err(|e| e.to_string())?;
        let input = MatchInput {
            text: text.map(str::to_owned),
            attachment_file_types: attachment_file_types.to_vec(),
        };
        if matches_any_keyword_rule(
            &input,
            &rules.iter().map(|r| r.1.clone()).collect::<Vec<_>>(),
        ) {
            return Ok(None);
        }
    }
    let has_agent = has_router
        || chatbotx_business::ai_agent::find_default(&deps.db, workspace_id)
            .await
            .map_err(|e| e.message())?
            .is_some();
    Ok(has_agent.then_some(delay as u64))
}

/// `getKey` of `@chatbotx.io/automated-response`.
pub fn automated_response_key(conversation_id: i64, contact_inbox_id: i64) -> String {
    format!("automated-response:{conversation_id}-{contact_inbox_id}:messages")
}

pub(crate) fn kv(deps: &Deps) -> DurableKv {
    DurableKv::new(Arc::clone(&deps.db), Arc::new(SystemClock))
}

/// What `receiveMessage` stored and the routing inputs (`processIncomingMessage`).
#[derive(Debug, Clone)]
pub struct Stored {
    pub inbox: InboundInbox,
    pub conversation: Conversation,
    pub contact_inbox: ContactInbox,
    /// The new message row JSON (`None` = a redelivery or nothing stored).
    pub message: Option<Value>,
    pub message_id: Option<i64>,
    pub received: Received,
    pub is_new_contact: bool,
    /// A step's Skip tap: resolved by its own `resolveStepInput` job, never
    /// routed as a typed reply.
    pub step_input_skip: bool,
    /// `standbyCopy`: the stored standby copy (also on a retry of an unpromoted one).
    pub standby_copy: Option<chatbotx_db::repo::models::Message>,
}

/// `IntegrationJobAction.incomingMessage`: [`receive_message`] then the
/// routing of `processIncomingMessage`.
pub async fn incoming_message(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(stored) = receive_message(deps, data).await? else {
        return Ok(());
    };
    route_incoming_message(deps, &stored).await
}

/// `receiveMessage` (received-message.ts): parse, find or create the
/// contact and conversation, store and broadcast the message. `None` = the
/// channel had nothing to store.
pub async fn receive_message(deps: &Deps, data: &Value) -> Result<Option<Stored>, JobError> {
    let integration_type = str_of(data, "integrationType").unwrap_or("").to_owned();
    let identifier = data
        .get("integrationIdentifier")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let resolved = deps
        .channels
        .resolve_by_identifier(
            &deps.db,
            deps.realtime.as_ref(),
            &integration_type,
            &identifier,
        )
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?;
    let Some((channel, ctx, inbox)) = resolved else {
        // `IntegrationNotFoundError`: the integration was disconnected.
        return Err(JobError::unrecoverable(format!(
            "Integration not found: {integration_type} {identifier}"
        )));
    };
    let payload = data.get("payload").cloned().unwrap_or(Value::Null);
    let received = channel
        .receive_message(&ctx, payload)
        .await
        .map_err(|e| fail(e.to_string()))?;
    let Some(mut received) = received else {
        tracing::warn!(
            integration_type,
            inbox_id = inbox.inbox_id,
            workspace_id = inbox.workspace_id,
            "Received message has no storable content, skipping"
        );
        return Ok(None);
    };
    let suppress = received
        .thread_control
        .as_ref()
        .and_then(|t| str_of(t, "delivery"))
        == Some("standby");
    // Q.D9 hook: `parseAppointmentCancelPostback(rawPostbackAction)` (skipped
    // on a standby delivery) and its `cancelAppointmentByToken` run go here.
    let skip_challenge_id = step_input_skip_challenge_id(&received);
    sanitize_actions(
        &mut received,
        &integration_type,
        skip_challenge_id.is_some(),
    );
    // `markStandbyDelivery`: a standby copy of an inbound message carries a marker
    // so its later owner delivery can promote it once.
    if suppress
        && let Some(incoming) = received
            .message
            .as_mut()
            .filter(|m| m.message_type != "outgoing")
    {
        let attrs = incoming.content_attributes.get_or_insert_with(|| json!({}));
        attrs[tc::DELIVERY_KEY] = json!(tc::STANDBY_DELIVERY);
    }
    if is_third_party_echo(&received) {
        let (probe_inbox, contact) = (inbox.clone(), received.contact.clone());
        let known = deps
            .db
            .read(move |conn| Ok(find_existing_contact_inbox(conn, &probe_inbox, &contact)?))
            .await
            .map_err(db_fail)?;
        if known.is_none() {
            tracing::debug!(
                inbox_id = inbox.inbox_id,
                source_id = received.contact.source_id,
                "Skipping third-party echo for an unknown contact"
            );
            return Ok(None);
        }
    }
    // A cart or product question is named before the save, so the stored
    // message already carries it.
    crate::inbound_cart::enrich(deps, &inbox, &mut received).await;
    let source = contact_source(
        received.referral.as_ref(),
        received.referral_source.as_deref(),
    );
    // Orchestrator decision (DECISIONS 2026-11-17): the contact lookup /
    // creation, the contact updates and the message insert are one write
    // transaction, so no reader sees a new contact without its message.
    let profile = profile_if_new(deps, &inbox, &received.contact).await?;
    let (tx_inbox, tx_received, tx_skip) =
        (inbox.clone(), received.clone(), skip_challenge_id.clone());
    let detected = deps
        .db
        .write(move |tx| {
            let Some(detected) = detect_contact_and_conversation_tx(
                tx,
                &tx_inbox,
                &tx_received.contact,
                source,
                profile.as_ref(),
            )?
            else {
                return Ok(None);
            };
            let (contact_inbox, conversation) = (
                detected.contact_inbox.clone(),
                detected.conversation.clone(),
            );
            // Zalo / Telegram echo a tap's raw payload as the text: show the
            // label the contact pressed instead.
            let tap_label =
                postback_button_label(tx, tx_inbox.workspace_id, &tx_received).or_else(|| {
                    tx_skip.as_deref().and_then(|id| {
                        step_input_skip_label(
                            tx,
                            tx_inbox.workspace_id,
                            &conversation,
                            &tx_received,
                            id,
                        )
                    })
                });
            let mut tx_received = tx_received.clone();
            if let Some(label) = &tap_label {
                apply_tap_label(&mut tx_received, label);
            }
            let mut saved = None;
            if let Some(incoming) = &tx_received.message {
                if incoming.message_type != "outgoing" {
                    if let Some(text) = incoming.text.as_deref() {
                        update_contact_from_message_tx(
                            tx,
                            &contact_inbox,
                            tx_inbox.workspace_id,
                            text,
                        );
                    }
                    unblock_if_blocked_tx(tx, contact_inbox.contact_id, tx_inbox.workspace_id);
                }
                saved = Some(save_message_tx(
                    tx,
                    &tx_inbox,
                    &contact_inbox,
                    &conversation,
                    incoming,
                    &tx_received,
                )?);
            }
            Ok(Some((detected, saved, tap_label)))
        })
        .await
        .map_err(db_fail)?;
    let Some((mut detected, saved, tap_label)) = detected else {
        return Err(mac_limit_reached());
    };
    after_detect(deps, inbox.workspace_id, &mut detected).await;
    let Detected {
        contact_inbox,
        conversation,
        is_new_contact,
        ..
    } = detected;
    if let Some(label) = &tap_label {
        apply_tap_label(&mut received, label);
    }
    // `lookupAndPersistGoogleClick` (gated, best effort, saved by itself).
    // A new contact's click is looked up here too
    // (`persistNewContactGoogleClick`).
    if received
        .message
        .as_ref()
        .is_some_and(is_inbound_conversation_message)
    {
        crate::google_click_lookup::lookup_and_persist(
            deps,
            inbox.workspace_id,
            &contact_inbox,
            is_new_contact,
        )
        .await;
    }

    let mut message = None;
    let mut message_id = None;
    let mut standby_copy = None;
    let info = received.thread_control.clone();
    let delivery = info
        .as_ref()
        .and_then(|t| str_of(t, "delivery"))
        .map(str::to_owned);
    // A routing delivery with no storable message still records its ownership
    // observation (the guarded write is idempotent on the delivery's own time).
    if saved.is_none()
        && let Some(info) = &info
    {
        record_inbound(
            deps,
            &inbox,
            &contact_inbox,
            conversation.id,
            info,
            now_ms(),
            false,
        )
        .await?;
    }
    if let Some((stored, is_new)) = saved {
        let mut owns_automation = is_new;
        if let Some(info) = info
            .as_ref()
            .filter(|_| delivery.as_deref() == Some("owner"))
        {
            // An owner delivery unlocks the thread for our automation. It is
            // recorded after the save (one transaction above), so a retry after a
            // failed record sees a stored message: a record that still changes the
            // state means the earlier attempt never got to the automation.
            let recorded = record_inbound(
                deps,
                &inbox,
                &contact_inbox,
                conversation.id,
                info,
                now_ms(),
                false,
            )
            .await?;
            owns_automation |= recorded.is_some_and(|r| r.state_changed);
            // The owner delivery of a message first stored from its standby copy:
            // recorded at the copy's own time, then the one-shot promotion claim.
            let attrs = stored.message.content_attributes.as_ref().map(|j| &j.0);
            if !is_new && tc::is_unpromoted_standby_copy(attrs) {
                record_inbound(
                    deps,
                    &inbox,
                    &contact_inbox,
                    conversation.id,
                    info,
                    stored.message.created_at,
                    true,
                )
                .await?;
                owns_automation |=
                    tc::promote_standby_delivery(&deps.db, inbox.workspace_id, &stored.message)
                        .await
                        .map_err(|e| fail(e.message()))?;
            }
        }
        // A standby copy (new, or a retry of one still unpromoted) records the
        // standby transition; a delivery that says nothing about ownership (a
        // partner's template echo) is stored only.
        if let Some(info) = info.as_ref().filter(|i| {
            delivery.as_deref() == Some(tc::STANDBY_DELIVERY)
                && i.get("signalsOwnership") != Some(&json!(false))
        }) && (is_new
            || tc::is_unpromoted_standby_copy(
                stored.message.content_attributes.as_ref().map(|j| &j.0),
            ))
        {
            standby_copy = Some(stored.message.clone());
            record_inbound(
                deps,
                &inbox,
                &contact_inbox,
                conversation.id,
                info,
                stored.message.created_at,
                false,
            )
            .await?;
        }
        let row = broadcast_saved(
            deps,
            &inbox,
            &contact_inbox,
            &conversation,
            (&stored, received.message.as_ref()),
            is_new,
            None,
        )
        .await;
        // Also on a redelivery: the job is deduplicated and the challenge
        // claim settles it once.
        if let Some(challenge_id) = &skip_challenge_id
            && !suppress
            && workspace_is_active(deps, inbox.workspace_id).await?
        {
            let job = chatbotx_jobs::queues::resolve_step_input::build_resolve_step_input_job(
                &chatbotx_jobs::queues::resolve_step_input::ResolveStepInput {
                    conversation_id: conversation.id.to_string(),
                    contact_inbox_id: contact_inbox.id.to_string(),
                    challenge_id: challenge_id.clone(),
                    reason: chatbotx_jobs::queues::resolve_step_input::ResolveStepInputReason::UserSkipped,
                    message_id: Some(stored.message.id.to_string()),
                    message_created_at: Some(chatbotx_db::repo::iso(stored.message.created_at)),
                },
            );
            add_job(deps, QueueName::Integration, &job.name, job.data, job.opts).await?;
        }
        if is_new {
            crate::leaderboard::award_inbound_message(
                deps,
                &inbox,
                contact_inbox.contact_id,
                &stored.message,
                received.referral.as_ref(),
            )
            .await;
        }
        if owns_automation {
            message_id = row["id"].as_str().and_then(|s| s.parse().ok());
            message = Some(row);
        }
    }
    Ok(Some(Stored {
        step_input_skip: skip_challenge_id.is_some(),
        inbox,
        conversation,
        contact_inbox,
        message,
        message_id,
        received,
        is_new_contact,
        standby_copy,
    }))
}

/// The Skip tap's challenge id (`decodeStepInputSkipPostback` of the raw
/// postback, then of the raw quick reply).
/// The saved text and `buttonTitle` of a tap whose label was resolved.
fn apply_tap_label(received: &mut Received, label: &str) {
    if let Some(message) = received.message.as_mut() {
        message.text = Some(label.to_owned());
    }
    if received.button_title.as_deref().is_none_or(str::is_empty) {
        received.button_title = Some(label.to_owned());
    }
}

/// Whether `text` is the bare (Telegram) or `postback_`-prefixed (Zalo) echo
/// of `payload`.
fn is_payload_echo(text: Option<&str>, payload: &str) -> bool {
    use chatbotx_core::flow_config::util::ZALO_POSTBACK_TEXT_PREFIX;
    text.is_some_and(|t| t == payload || t.strip_prefix(ZALO_POSTBACK_TEXT_PREFIX) == Some(payload))
}

/// `detectFlowVersion`'s nodes (the given version, else the active flow's
/// current one); `None` when gone.
fn flow_nodes(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
    version_id: Option<i64>,
) -> Option<Vec<Value>> {
    let version_id = match version_id {
        Some(id) => Some(id),
        None => conn
            .query_row(
                r#"SELECT "currentVersionId" FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1"#,
                params![flow_id, workspace_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .ok()??,
    }?;
    let nodes: String = conn
        .query_row(
            r#"SELECT "nodes" FROM "FlowVersion" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![version_id, workspace_id],
            |row| row.get(0),
        )
        .optional()
        .ok()??;
    serde_json::from_str::<Value>(&nodes)
        .ok()?
        .as_array()
        .cloned()
}

/// `resolvePostbackButtonLabel`: the tapped button's label when the channel
/// sent only the payload echo; `None` leaves the text (a stale tap never
/// fails the save).
fn postback_button_label(
    conn: &Connection,
    workspace_id: i64,
    received: &Received,
) -> Option<String> {
    let action = received.postback_action.as_deref()?;
    if received
        .button_title
        .as_deref()
        .is_some_and(|t| !t.is_empty())
    {
        return None;
    }
    let text = received.message.as_ref()?.text.as_deref();
    if !is_payload_echo(text, action) {
        return None;
    }
    let payload = chatbotx_core::flow_config::util::decode_button_payload(action, false)?;
    let button_id = payload.button_id.as_deref()?;
    let version = match payload.flow_version_id.as_deref() {
        Some(id) => Some(id.parse().ok()?),
        None => None,
    };
    let nodes = flow_nodes(conn, workspace_id, payload.flow_id.parse().ok()?, version)?;
    let target = chatbotx_core::flow_config::util::resolve_flow_action_target(&nodes, button_id)?;
    target.details["label"].as_str().map(str::to_owned)
}

/// `resolveStepInputSkipLabel`: the Skip label of the step the pending
/// challenge belongs to, when the text is the raw Skip payload echo.
fn step_input_skip_label(
    conn: &Connection,
    workspace_id: i64,
    conversation: &Conversation,
    received: &Received,
    challenge_id: &str,
) -> Option<String> {
    let payload =
        chatbotx_core::flow_config::step_input_resolution::encode_step_input_skip_postback(
            challenge_id,
        );
    let text = received.message.as_ref()?.text.as_deref();
    if received
        .button_title
        .as_deref()
        .is_some_and(|t| !t.is_empty())
        || !is_payload_echo(text, &payload)
    {
        return None;
    }
    let challenge = conversation
        .additional_attributes
        .as_ref()?
        .0
        .get("challenge")?;
    let data = &challenge["data"];
    if challenge["type"] != "step" || data["challengeId"] != challenge_id {
        return None;
    }
    let nodes = flow_nodes(
        conn,
        workspace_id,
        crate::id_of(&data["flowId"])?,
        crate::id_of(&data["flowVersionId"]),
    )?;
    let node = nodes.iter().find(|n| n["id"] == data["nodeId"])?;
    let step = node["data"]["details"]["steps"]
        .as_array()?
        .iter()
        .find(|s| s["id"] == data["stepId"])?;
    let label = step["skipButtonLabel"].as_str()?.trim();
    (!label.is_empty()).then(|| label.to_owned())
}

fn step_input_skip_challenge_id(received: &Received) -> Option<String> {
    use chatbotx_core::flow_config::step_input_resolution::decode_step_input_skip_postback;
    decode_step_input_skip_postback(received.postback_action.as_deref())
        .or_else(|| decode_step_input_skip_postback(received.quick_reply_action.as_deref()))
}

/// `sanitizeFlowAction` of both taps, the contact-hint quick reply (its full
/// value becomes the text) and the value quick replies (a payload equal to
/// the text is a typed reply). Only Messenger accepts a bare flow id.
fn sanitize_actions(received: &mut Received, integration_type: &str, skip_tap: bool) {
    use chatbotx_core::flow_config::steps::decode_contact_hint_quick_reply_payload;
    let allow_bare = integration_type == "messenger";
    let keep = |raw: Option<String>, kind: &str| {
        let raw = raw.filter(|r| !r.is_empty())?;
        if chatbotx_core::flow_config::util::decode_button_payload(&raw, allow_bare).is_some() {
            return Some(raw);
        }
        tracing::warn!(
            kind,
            integration_type,
            action = raw,
            "Dropping undecodable flow action from channel webhook"
        );
        None
    };
    let hint = decode_contact_hint_quick_reply_payload(received.quick_reply_action.as_deref());
    let text = received.message.as_ref().and_then(|m| m.text.clone());
    if let (Some(hint), Some(message)) = (&hint, received.message.as_mut()) {
        message.text = Some(hint.clone());
    }
    let value_reply = hint.is_some()
        || (received.quick_reply_action.is_some() && received.quick_reply_action == text);
    let postback = received.postback_action.take().filter(|_| !skip_tap);
    received.postback_action = keep(postback, "postback");
    let quick_reply = received
        .quick_reply_action
        .take()
        .filter(|_| !skip_tap && !value_reply);
    received.quick_reply_action = keep(quick_reply, "quickReply");
}

/// `isThirdPartyEcho`: another app's send mirrored back (not a story reply).
fn is_third_party_echo(received: &Received) -> bool {
    received.echo_origin.as_deref() == Some("thirdParty")
        && received.message.as_ref().is_some_and(|m| {
            m.message_type == "outgoing"
                && !m.content_attributes.as_ref().is_some_and(|a| {
                    let story = if str_of(a, "type") == Some("story_reply") {
                        a.get("story")
                    } else {
                        a.get("storyReply")
                    };
                    story.is_some_and(|s| !s.is_null())
                })
        })
}

/// `resolveContactSource`: a Google click makes an ad contact, else
/// `metaReferralToContactSource(referralSource) ?? inboundMessage`.
fn contact_source(referral: Option<&Value>, referral_source: Option<&str>) -> &'static str {
    if chatbotx_core::utils::google_click::has_google_click(referral) {
        return "ads";
    }
    match referral_source {
        Some("ADS") => "ads",
        Some("SHORTLINK") => "botLink",
        Some("CUSTOMER_CHAT_PLUGIN") => "chatPlugin",
        _ => "inboundMessage",
    }
}

/// `resolveSourceScopedIdentityMatch` over `contactInboxRepository.findWithContact`:
/// `sourceId`, then `sourceUserId`, then `sourceParentUserId`.
pub(crate) fn find_existing_contact_inbox(
    conn: &Connection,
    inbox: &InboundInbox,
    contact: &IncomingContact,
) -> chatbotx_db::rusqlite::Result<Option<(ContactInbox, &'static str)>> {
    let probes: [(&'static str, Option<&str>); 3] = [
        ("sourceId", Some(contact.source_id.as_str())),
        ("sourceUserId", contact.source_user_id.as_deref()),
        (
            "sourceParentUserId",
            contact.source_parent_user_id.as_deref(),
        ),
    ];
    for (column, value) in probes {
        let Some(value) = value else { continue };
        let found = conn
            .query_row(
                &format!(
                    r#"{} WHERE "inboxId" = ?1 AND "channel" = ?2 AND "{column}" = ?3
                       AND EXISTS (SELECT 1 FROM "Contact" c WHERE c."id" = "ContactInbox"."contactId")
                       LIMIT 1"#,
                    ContactInbox::select_sql()
                ),
                params![inbox.inbox_id, inbox.channel, value],
                ContactInbox::from_row,
            )
            .optional()?;
        if let Some(found) = found {
            return Ok(Some((found, column)));
        }
    }
    Ok(None)
}

/// `conversationService.findOrCreate` (inside the caller's transaction).
pub(crate) fn find_or_create_conversation(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    source_id: Option<&str>,
    channel_conversation_id: Option<&str>,
) -> chatbotx_db::rusqlite::Result<Conversation> {
    let find = |conn: &Connection| {
        conn.query_row(
            &format!(
                r#"{} WHERE "workspaceId" = ?1 AND "contactId" = ?2 AND "sourceId" IS ?3 LIMIT 1"#,
                Conversation::select_sql()
            ),
            params![workspace_id, contact_id, source_id],
            Conversation::from_row,
        )
        .optional()
    };
    let channel_conversation_id = channel_conversation_id.filter(|id| !id.is_empty());
    if let Some(existing) = find(conn)? {
        // `syncChannelConversationId`.
        let Some(channel_id) = channel_conversation_id else {
            return Ok(existing);
        };
        let mut attributes = existing
            .additional_attributes
            .as_ref()
            .map(|a| a.0.clone())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if attributes["channelConversationId"] == channel_id {
            return Ok(existing);
        }
        attributes["channelConversationId"] = json!(channel_id);
        conn.execute(
            r#"UPDATE "Conversation" SET "additionalAttributes" = ?2 WHERE "id" = ?1"#,
            params![existing.id, attributes.to_string()],
        )?;
        return find(conn)?.ok_or(chatbotx_db::rusqlite::Error::QueryReturnedNoRows);
    }
    let attributes =
        channel_conversation_id.map(|id| json!({ "channelConversationId": id }).to_string());
    conn.execute(
        r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "sourceId", "additionalAttributes")
           VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING"#,
        params![new_id(), workspace_id, contact_id, source_id, attributes],
    )?;
    find(conn)?.ok_or(chatbotx_db::rusqlite::Error::QueryReturnedNoRows)
}

/// What a new contact is created with: the channel's `contact.getProfile`
/// (when the channel has one) and `finalizeContactProfile`.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewContactProfile {
    first_name: Option<String>,
    last_name: Option<String>,
    avatar: Option<String>,
    gender: Option<String>,
    locale: Option<String>,
    timezone: Option<String>,
    language: Option<String>,
    source_username: Option<String>,
    snapshot: Option<chatbotx_business::contact::channel_profile::ProfileSnapshot>,
}

impl NewContactProfile {
    /// `finalizeContactProfile` over the webhook fields and a looked-up profile.
    fn finalize(
        inbox: &InboundInbox,
        contact: &IncomingContact,
        mut profile: NewContactProfile,
    ) -> NewContactProfile {
        use chatbotx_business::contact::locale_profile::{LocaleProfile, finalize_contact_profile};
        let user_id_keyed = contact
            .source_user_id
            .as_deref()
            .is_some_and(|u| !u.is_empty() && u == contact.source_id);
        let phone_hint = contact.phone_number.clone().or_else(|| {
            (inbox.channel == "whatsapp" && !user_id_keyed).then(|| contact.source_id.clone())
        });
        let finalized = finalize_contact_profile(
            &LocaleProfile {
                locale: profile.locale.clone().or_else(|| contact.locale.clone()),
                language: contact.language.clone(),
                timezone: profile.timezone.clone(),
            },
            phone_hint.as_deref(),
            (inbox.channel == "zalo").then_some("vi_VN"),
        );
        profile.locale = finalized.locale;
        profile.timezone = finalized.timezone;
        profile.language = finalized.language;
        profile
    }
}

/// The profile of a new contact: the channel lookup (a failure is logged
/// and the contact is created from the webhook fields), then the locale.
pub(crate) async fn new_contact_profile(
    deps: &Deps,
    inbox: &InboundInbox,
    contact: &IncomingContact,
) -> NewContactProfile {
    use chatbotx_business::contact::channel_profile::{
        ProfileDeps, fetch_profile, fetch_profile_snapshot,
    };
    let mut profile = NewContactProfile::default();
    if let Some(http) = deps.channel_http.as_deref()
        && chatbotx_business::contact::profile_refresh::has_on_demand_profile_api(&inbox.channel)
    {
        let include_avatar = !chatbotx_core::utils::live_media::is_lazy_avatar_channel(
            &chatbotx_core::env::ProcessEnv,
            &inbox.channel,
        );
        let profile_deps = ProfileDeps {
            db: &deps.db,
            http,
            uploader: deps.files.as_deref(),
        };
        match fetch_profile(
            profile_deps,
            inbox.workspace_id,
            &inbox.channel,
            inbox.inbox_id,
            &contact.source_id,
            include_avatar,
        )
        .await
        {
            Ok(found) => {
                profile.first_name = found.first_name;
                profile.last_name = found.last_name;
                profile.avatar = found.avatar;
                profile.gender = found.gender;
                profile.locale = found.locale;
                profile.timezone = found.timezone;
            }
            Err(error) => {
                tracing::warn!(
                    error,
                    source_id = contact.source_id,
                    channel = inbox.channel,
                    "detectContactAndConversation: getProfile failed, creating contact without profile data"
                );
                // `recordProfileRefreshFailure`: no contact yet, the source id only.
                crate::events::log_failure(
                    deps,
                    &inbox.channel,
                    inbox.workspace_id,
                    None,
                    Some(&contact.source_id),
                    serde_json::Value::String(error),
                )
                .await;
            }
        }
        if chatbotx_business::contact::profile_snapshot::supports_profile_snapshot(&inbox.channel) {
            match fetch_profile_snapshot(
                &deps.db,
                http,
                inbox.workspace_id,
                &inbox.channel,
                inbox.inbox_id,
                &contact.source_id,
            )
            .await
            {
                Ok(snapshot) => {
                    profile.source_username = snapshot.as_ref().and_then(|s| s.username.clone());
                    profile.snapshot = snapshot;
                }
                Err(error) => {
                    tracing::warn!(%error, source_id = contact.source_id, "getProfileSnapshot failed")
                }
            }
        }
    }
    NewContactProfile::finalize(inbox, contact, profile)
}

/// The result of [`detect_contact_and_conversation_tx`]: the rows, and the
/// trigger events to publish once the transaction commits.
pub(crate) struct Detected {
    pub contact_inbox: ContactInbox,
    pub conversation: Conversation,
    pub is_new_contact: bool,
    pub events: Vec<chatbotx_business::events::TriggerEvent>,
    /// A new contact's stored avatar (the background re-encode).
    pub new_avatar: Option<(i64, String)>,
}

/// `UnrecoverableError("contact_mac_limit_reached")`.
fn mac_limit_reached() -> JobError {
    JobError::unrecoverable("contact_mac_limit_reached")
}

/// Publishes the trigger events of a detection and queues a new avatar's
/// re-encode.
pub(crate) async fn after_detect(deps: &Deps, workspace_id: i64, detected: &mut Detected) {
    if !detected.events.is_empty() {
        let bus = crate::order_payment::bus(deps);
        for event in std::mem::take(&mut detected.events) {
            bus.publish(chatbotx_business::events::Event::Trigger(event))
                .await;
        }
    }
    if let Some((contact_id, key)) = detected.new_avatar.take() {
        use chatbotx_jobs::queues::stored_image::{
            StoredImageJob, StoredImageOptimizationConfig, enqueue_stored_image_optimizations,
        };
        let config = StoredImageOptimizationConfig {
            enabled:
                chatbotx_core::utils::image_optimize_settings::is_stored_image_optimization_enabled(
                    &chatbotx_core::env::ProcessEnv,
                ),
        };
        let job = StoredImageJob::Avatar {
            workspace_id: workspace_id.to_string(),
            object_key: key,
            contact_id: contact_id.to_string(),
        };
        enqueue_stored_image_optimizations(deps.jobs.as_ref(), &config, vec![job]).await;
    }
}

/// The profile a detection needs: a lookup only when the identity is new.
async fn profile_if_new(
    deps: &Deps,
    inbox: &InboundInbox,
    contact: &IncomingContact,
) -> Result<Option<NewContactProfile>, JobError> {
    let (probe_inbox, probe) = (inbox.clone(), contact.clone());
    let known = deps
        .db
        .read(move |conn| Ok(find_existing_contact_inbox(conn, &probe_inbox, &probe)?.is_some()))
        .await
        .map_err(db_fail)?;
    Ok(if known {
        None
    } else {
        Some(new_contact_profile(deps, inbox, contact).await)
    })
}

/// [`detect_contact_and_conversation_tx`] in its own write transaction
/// (for callers outside [`receive_message`]).
pub(crate) async fn detect_contact_and_conversation(
    deps: &Deps,
    inbox: &InboundInbox,
    contact: &IncomingContact,
    source: &'static str,
) -> Result<(ContactInbox, Conversation, bool), JobError> {
    let profile = profile_if_new(deps, inbox, contact).await?;
    let inbox_owned = inbox.clone();
    let contact = contact.clone();
    let detected = deps
        .db
        .write(move |tx| {
            detect_contact_and_conversation_tx(tx, &inbox_owned, &contact, source, profile.as_ref())
        })
        .await
        .map_err(db_fail)?;
    let Some(mut detected) = detected else {
        return Err(mac_limit_reached());
    };
    after_detect(deps, inbox.workspace_id, &mut detected).await;
    Ok((
        detected.contact_inbox,
        detected.conversation,
        detected.is_new_contact,
    ))
}

/// `detectContactAndConversation` in the caller's transaction: the existing
/// contact inbox (its newly learned identity synced), or a new contact +
/// contact inbox + conversation behind the MAC gate. `None` = the MAC limit
/// refused the new contact.
fn detect_contact_and_conversation_tx(
    tx: &Connection,
    inbox: &InboundInbox,
    contact: &IncomingContact,
    source: &'static str,
    profile: Option<&NewContactProfile>,
) -> Result<Option<Detected>, chatbotx_db::DbError> {
    use chatbotx_business::contact_identity::{IncomingIdentity, sync_existing_contact_identity};
    use chatbotx_business::events::trigger;
    let source_conversation_id = contact.source_conversation_id.as_deref();
    let business = |e: chatbotx_business::BusinessError| chatbotx_db::DbError::other(e.to_string());
    if let Some((existing, matched_by)) = find_existing_contact_inbox(tx, inbox, contact)? {
        let identity = IncomingIdentity {
            source_id: contact.source_id.clone(),
            source_user_id: contact.source_user_id.clone(),
            source_parent_user_id: contact.source_parent_user_id.clone(),
            source_username: contact.source_username.clone(),
        };
        let (synced, phone) = sync_existing_contact_identity(
            tx,
            inbox.workspace_id,
            &existing,
            &identity,
            matched_by,
        )
        .map_err(business)?;
        let conversation = find_or_create_conversation(
            tx,
            inbox.workspace_id,
            synced.contact_id,
            source_conversation_id,
            contact.channel_conversation_id.as_deref(),
        )?;
        let events = phone
            .map(|(before, after)| {
                trigger::contact_info_changes(
                    &inbox.workspace_id.to_string(),
                    &synced.contact_id.to_string(),
                    [before.as_deref(), None],
                    [Some(&after), None],
                )
            })
            .unwrap_or_default();
        return Ok(Some(Detected {
            contact_inbox: synced,
            conversation,
            is_new_contact: false,
            events,
            new_avatar: None,
        }));
    }
    let profile = match profile {
        Some(profile) => profile.clone(),
        None => NewContactProfile::finalize(inbox, contact, NewContactProfile::default()),
    };
    let owner_id: i64 = tx
        .query_row(
            r#"SELECT "ownerId" FROM "Workspace" WHERE "id" = ?1"#,
            [inbox.workspace_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| chatbotx_db::DbError::other("Workspace not found"))?;
    let phone_number = contact.phone_number.clone().or_else(|| {
        (inbox.channel == "whatsapp"
            && contact.source_user_id.as_deref() != Some(contact.source_id.as_str())
            && !contact.source_id.is_empty())
        .then(|| format!("+{}", contact.source_id.trim_start_matches('+')))
    });
    let first_name = profile
        .first_name
        .clone()
        .or_else(|| contact.first_name.clone());
    let last_name = profile
        .last_name
        .clone()
        .or_else(|| contact.last_name.clone());
    let avatar = profile.avatar.clone().or_else(|| contact.avatar.clone());
    let snapshot = profile.snapshot.clone().unwrap_or_default();
    let username = contact
        .source_username
        .clone()
        .or_else(|| profile.source_username.clone());
    let created = chatbotx_business::quota_enforcement::create_new_contact_with_mac(
        tx,
        owner_id,
        inbox.workspace_id,
        |tx, period_start| {
            let contact_id = new_id();
            let now = now_ms();
            let db = |e: chatbotx_db::rusqlite::Error| {
                chatbotx_business::BusinessError::Repo(e.into())
            };
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "firstName",
                             "lastName", "email", "phoneNumber", "avatar", "locale", "timezone", "gender")
                           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
                params![
                    contact_id,
                    now,
                    inbox.workspace_id,
                    first_name,
                    last_name,
                    contact.email,
                    phone_number,
                    avatar,
                    profile.locale,
                    profile.timezone,
                    profile.gender,
                ],
            )
            .map_err(db)?;
            let contact_inbox_id = new_id();
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "createdAt", "updatedAt", "inboxId", "contactId",
                     "originalContactId", "source", "sourceId", "sourceUserId", "sourceParentUserId",
                     "sourceUsername", "channel", "language", "followsBusiness",
                     "businessFollowsContact", "accountVerified", "followerCount")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?4, ?11, ?5, ?6, ?7, ?8, ?9, ?10, ?12, ?13, ?14, ?15)"#,
                params![
                    contact_inbox_id,
                    now,
                    inbox.inbox_id,
                    contact_id,
                    contact.source_id,
                    contact.source_user_id,
                    contact.source_parent_user_id,
                    username,
                    inbox.channel,
                    profile.language,
                    source,
                    snapshot.follows_business,
                    snapshot.business_follows_contact,
                    snapshot.account_verified,
                    snapshot.follower_count,
                ],
            )
            .map_err(db)?;
            let counted = match period_start {
                Some(period_start) => chatbotx_business::analytics::mac::claim_new_active_contact(
                    tx,
                    inbox.workspace_id,
                    (contact_id, contact_inbox_id),
                    inbox.inbox_id,
                    period_start,
                    now,
                )
                .map_err(chatbotx_business::BusinessError::Repo)?,
                None => false,
            };
            Ok(((contact_id, contact_inbox_id, now), counted))
        },
    )
    .map_err(business)?;
    let Ok((contact_id, contact_inbox_id, created_at)) = created else {
        tracing::error!(
            workspace_id = inbox.workspace_id,
            owner_id,
            channel = inbox.channel,
            source_id = contact.source_id,
            "Inbound new-contact rejected: MAC limit reached"
        );
        return Ok(None);
    };
    let contact_inbox = tx.query_row(
        &format!(r#"{} WHERE "id" = ?1"#, ContactInbox::select_sql()),
        params![contact_inbox_id],
        ContactInbox::from_row,
    )?;
    let conversation = find_or_create_conversation(
        tx,
        inbox.workspace_id,
        contact_id,
        source_conversation_id,
        contact.channel_conversation_id.as_deref(),
    )?;
    // The `analytics:dashboard` `contact:created` row (`contactId` carries
    // the contact inbox id, as the original).
    if !contact_inbox.source_id.is_empty() {
        tx.execute(
            r#"INSERT INTO "AnalyticsContactEvent" ("eventId", "workspaceId", "contactId", "eventType",
                   "occurredAt", "source", "sourceId", "channel", "inboxId", "language", "metadata")
               VALUES (?1, ?2, ?3, 'contact_created', ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
            params![
                new_id().to_string(),
                inbox.workspace_id,
                contact_inbox_id,
                created_at,
                contact_inbox.source,
                contact_inbox.source_id,
                contact_inbox.channel,
                inbox.inbox_id,
                profile.language,
                r#"{"triggerContext":{"triggerSource":"worker","triggerHandler":"receiveMessage","triggerType":"contact_created"}}"#,
            ],
        )?;
    }
    let ws = inbox.workspace_id.to_string();
    let events = vec![trigger::contact_created(
        &ws,
        &contact_id.to_string(),
        first_name.as_deref().filter(|n| !n.is_empty()),
        phone_number.as_deref().filter(|p| !p.is_empty()),
        contact.email.as_deref().filter(|e| !e.is_empty()),
        Some(&contact_inbox_id.to_string()),
    )];
    // Only a stored key is re-encoded (a remote URL is not ours).
    let new_avatar = profile
        .avatar
        .clone()
        .filter(|a| !a.starts_with("http"))
        .map(|key| (contact_id, key));
    Ok(Some(Detected {
        contact_inbox,
        conversation,
        is_new_contact: true,
        events,
        new_avatar,
    }))
}

/// `updateContactFromMessage`: a phone / email the contact typed overwrites
/// the contact's columns. Failures are logged.
fn update_contact_from_message_tx(
    tx: &Connection,
    contact_inbox: &ContactInbox,
    workspace_id: i64,
    text: &str,
) {
    use chatbotx_business::contact::extract::{ExtractOptions, extract_contact_info};
    let info = extract_contact_info(Some(text), None, ExtractOptions::default());
    if info.phone_number.is_none() && info.email.is_none() {
        return;
    }
    let contact_id = contact_inbox.contact_id;
    let result = tx.execute(
        r#"UPDATE "Contact" SET "phoneNumber" = COALESCE(?1, "phoneNumber"),
             "email" = COALESCE(?2, "email"), "updatedAt" = ?3
           WHERE "id" = ?4 AND "workspaceId" = ?5"#,
        params![
            info.phone_number,
            info.email,
            now_ms(),
            contact_id,
            workspace_id
        ],
    );
    if let Err(err) = result {
        tracing::warn!(err = %err, contact_id, "Contact update from message text failed");
    }
}

/// `contactService.unblockIfBlocked`.
pub(crate) async fn unblock_if_blocked(deps: &Deps, contact_id: i64, workspace_id: i64) {
    let _ = deps
        .db
        .write(move |tx| {
            unblock_if_blocked_tx(tx, contact_id, workspace_id);
            Ok(())
        })
        .await
        .inspect_err(
            |err| tracing::warn!(err = %err, contact_id, "Auto-unblock on inbound message failed"),
        );
}

fn unblock_if_blocked_tx(tx: &Connection, contact_id: i64, workspace_id: i64) {
    let result = tx.execute(
        r#"UPDATE "Contact" SET "blockedAt" = NULL, "updatedAt" = ?1
           WHERE "id" = ?2 AND "workspaceId" = ?3 AND "blockedAt" IS NOT NULL"#,
        params![now_ms(), contact_id, workspace_id],
    );
    if let Err(err) = result {
        tracing::warn!(err = %err, contact_id, "Auto-unblock on inbound message failed");
    }
}

/// `isInboundConversationMessage`.
fn is_inbound_conversation_message(message: &IncomingMessage) -> bool {
    message.message_type == "incoming" && message.kind.as_deref().unwrap_or("message") == "message"
}

/// `saveAndBroadcastMessage` / `persistMessage`: the deduplicated insert,
/// the activity tracking of a new row, the realtime event. Returns the
/// stored row JSON and whether it is new.
pub(crate) async fn save_message(
    deps: &Deps,
    inbox: &InboundInbox,
    contact_inbox: &ContactInbox,
    conversation: &Conversation,
    incoming: &IncomingMessage,
    received: &Received,
    client_id: Option<&str>,
) -> Result<(Value, bool), JobError> {
    let (i, ci, c, m, r) = (
        inbox.clone(),
        contact_inbox.clone(),
        conversation.clone(),
        incoming.clone(),
        received.clone(),
    );
    let (stored, is_new) = deps
        .db
        .write(move |tx| save_message_tx(tx, &i, &ci, &c, &m, &r))
        .await
        .map_err(db_fail)?;
    Ok((
        broadcast_saved(
            deps,
            inbox,
            contact_inbox,
            conversation,
            (&stored, Some(incoming)),
            is_new,
            client_id,
        )
        .await,
        is_new,
    ))
}

/// The attachment rows of an incoming message (the message id is set by the
/// insert).
pub(crate) fn attachment_inputs(
    attachments: &[crate::channel::IncomingAttachment],
    workspace_id: i64,
    conversation_id: i64,
) -> Vec<CreateAttachmentInput> {
    attachments
        .iter()
        .map(|a| CreateAttachmentInput {
            id: None,
            workspace_id,
            conversation_id,
            message_id: 0,
            message_created_at: 0,
            file_type: a.file_type.clone(),
            mime_type: a.mime_type.clone(),
            origin_path: a.origin_path.clone(),
            created_at: None,
            height: a.height,
            width: a.width,
            keep_original: Some(
                !(a.is_photo_message && matches!(a.file_type.as_str(), "image" | "gif")),
            ),
            name: a.name.clone(),
            size: a.size,
            source_id: a.source_id.clone(),
            thumbnail_path: None,
        })
        .collect()
}

/// The insert + activity tracking of [`save_message`] on the caller's
/// transaction.
fn save_message_tx(
    tx: &Connection,
    inbox: &InboundInbox,
    contact_inbox: &ContactInbox,
    conversation: &Conversation,
    incoming: &IncomingMessage,
    received: &Received,
) -> Result<(repo_message::MessageWithAttachments, bool), chatbotx_db::DbError> {
    let inbound = incoming.message_type != "outgoing";
    let created_at = now_ms();
    let input = CreateMessageInput {
        id: Some(new_id()),
        contact_inbox_id: contact_inbox.id,
        conversation_id: conversation.id,
        workspace_id: inbox.workspace_id,
        content_attributes: incoming.content_attributes.clone(),
        content_type: incoming.content_type.clone(),
        message_type: incoming.message_type.clone(),
        sender_type: if inbound { "contact" } else { "user" }.into(),
        sender_id: inbound.then_some(contact_inbox.contact_id),
        source_id: incoming.source_id.clone(),
        parent_id: incoming.parent_id.clone(),
        text: incoming.text.clone(),
        r#type: Some(incoming.kind.clone().unwrap_or_else(|| "message".into())),
        created_at: Some(created_at),
        updated_at: None,
    };
    let attachments = attachment_inputs(&incoming.attachments, inbox.workspace_id, conversation.id);
    let tracking_inbound = is_inbound_conversation_message(incoming);
    let last_btn_title = received.button_title.clone();
    let user_input = incoming.text.clone();
    let user_input_type = (incoming.content_type == "location")
        .then(|| "location".to_owned())
        .or_else(|| incoming.text.is_some().then(|| "text".to_owned()));
    let referral = received.referral.as_ref().map(Value::to_string);
    let ci = contact_inbox;
    let conversation_id = conversation.id;
    let workspace_id = inbox.workspace_id;
    let (stored, is_new) = if attachments.is_empty() {
        let r = repo_message::create_or_update(tx, &input).map_err(crate::repo_err)?;
        (
            repo_message::MessageWithAttachments {
                message: r.message,
                attachment_count: None,
                attachments: Vec::new(),
            },
            r.is_new,
        )
    } else {
        let r = repo_message::create_or_update_with_attachments(tx, &input, &attachments)
            .map_err(crate::repo_err)?;
        (r.result, r.is_new)
    };
    if !is_new {
        return Ok((stored, false));
    }
    let at = stored.message.created_at;
    // `recordInboundActivity`: contact-inbox tracking + the
    // conversation's activity and waiting window.
    tx.execute(
                r#"UPDATE "ContactInbox" SET
                     "firstInteractionAt" = CASE WHEN "firstInteractionAt" IS NULL OR "firstInteractionAt" > ?1 THEN ?1 ELSE "firstInteractionAt" END,
                     "lastMessageAt" = ?1,
                     "lastIncomingMessageAt" = CASE WHEN ?2 THEN ?1 ELSE "lastIncomingMessageAt" END,
                     "lastUserInput" = CASE WHEN ?2 AND ?3 IS NOT NULL THEN ?3 ELSE "lastUserInput" END,
                     "lastUserInputType" = CASE WHEN ?2 AND ?4 IS NOT NULL THEN ?4 ELSE "lastUserInputType" END,
                     "lastBtnTitle" = COALESCE(?5, "lastBtnTitle"),
                     "referral" = CASE WHEN ?6 IS NULL THEN "referral"
                                       ELSE json_patch(COALESCE("referral", '{}'), ?6) END,
                     "updatedAt" = ?7
                   WHERE "id" = ?8 AND "contactId" = ?9"#,
                params![
                    at,
                    tracking_inbound,
                    user_input,
                    user_input_type,
                    last_btn_title,
                    referral,
                    now_ms(),
                    ci.id,
                    ci.contact_id
                ],
            )?;
    if let Some(raw) = received.referral.as_ref() {
        // `referralEventKey: message.sourceId ?? message.id`.
        let key = stored
            .message
            .source_id
            .clone()
            .unwrap_or_else(|| stored.message.id.to_string());
        chatbotx_business::contact_referral_event::record_ad_touches_contained(
            tx,
            chatbotx_business::contact_referral_event::TouchOwner {
                workspace_id,
                contact_id: ci.contact_id,
                contact_inbox_id: ci.id,
                channel: &ci.channel,
            },
            raw,
            chatbotx_business::contact_referral_event::ReferralOccurrence {
                occurred_at: at,
                event_key: Some(&key),
            },
        );
    }
    // A contact message opening the window starts the response SLA.
    let activity = |c: &chatbotx_db::rusqlite::Connection| {
        c.execute(
            r#"UPDATE "Conversation" SET
                 "lastActivityAt" = ?1,
                 "contactRepliedAt" = CASE WHEN ?2 THEN ?1 ELSE "contactRepliedAt" END,
                 "awaitingHumanReplySince" = CASE WHEN ?2 THEN COALESCE("awaitingHumanReplySince", ?1)
                                                  ELSE "awaitingHumanReplySince" END,
                 "updatedAt" = ?3
               WHERE "id" = ?4 AND "workspaceId" = ?5"#,
            params![at, inbound, now_ms(), conversation_id, workspace_id],
        )
    };
    if inbound {
        chatbotx_business::response_sla::track_window(
            tx,
            workspace_id,
            &[conversation_id],
            false,
            activity,
        )?;
    } else {
        activity(tx)?;
    }
    Ok((stored, true))
}

/// The realtime `messageCreated` event of a new row; returns the row JSON.
async fn broadcast_saved(
    deps: &Deps,
    inbox: &InboundInbox,
    contact_inbox: &ContactInbox,
    conversation: &Conversation,
    (stored, incoming): (
        &repo_message::MessageWithAttachments,
        Option<&IncomingMessage>,
    ),
    is_new: bool,
    client_id: Option<&str>,
) -> Value {
    let row = message_json(stored);
    if !is_new {
        return row;
    }
    let message = &stored.message;
    let inbound = message.message_type != "outgoing";
    if let Some(incoming) = incoming {
        enqueue_photo_optimizations(deps, inbox.workspace_id, incoming, stored).await;
    }
    let mut event = row.clone();
    if let Some(client_id) = client_id {
        event["clientId"] = json!(client_id);
    }
    crate::chat::queue_event(
        deps,
        conversation,
        inbox.inbox_id,
        EventType::MessageCreated,
        event,
    )
    .await;
    if inbound {
        // `enqueueIncomingNotification` (best effort).
        let added = deps
            .jobs
            .add(
                QueueName::Notification,
                "notifyIncomingMessage",
                json!({ "type": "notifyIncomingMessage", "data": {
                    "workspaceId": inbox.workspace_id.to_string(),
                    "conversationId": conversation.id.to_string(),
                    "messageId": message.id.to_string(),
                    "messageText": message.text.as_deref().map(|t| t.chars().take(140).collect::<String>()),
                    "contentType": message.content_type,
                    "attachmentCount": stored.attachments.len(),
                }}),
                JobOptions {
                    job_id: Some(format!("notify-incoming-{}", message.id)),
                    ..JobOptions::default()
                },
            )
            .await;
        if let Err(err) = added {
            tracing::warn!(err = %err, "Unable to enqueue incoming message notification");
        }
    }
    // `emitMessageReceived`: `isFirstIncomingMessage` reads the snapshot taken
    // before this message's tracking update.
    let mut payload = json!({
        "workspaceId": inbox.workspace_id.to_string(),
        "contactId": contact_inbox.contact_id.to_string(),
        "contactInboxId": contact_inbox.id.to_string(),
        "channel": inbox.channel,
        "inboxId": inbox.inbox_id.to_string(),
        "occurredAt": chatbotx_db::repo::iso(message.created_at),
        "messageId": message.id.to_string(),
        "isFirstIncomingMessage": inbound && contact_inbox.last_incoming_message_at.is_none(),
    });
    if let Some(source_id) = &message.source_id {
        payload["sourceId"] = json!(source_id);
    }
    if inbound {
        payload["origin"] = json!("inbound");
    }
    emit_received(deps, payload).await;
    row
}

/// Emits `message:received`; a bus failure is logged, never raised.
pub(crate) async fn emit_received(deps: &Deps, payload: Value) {
    let Some(buses) = deps.events.as_ref() else {
        return;
    };
    if let Some(future) = chatbotx_jobs::events::emit(buses, "message:received", payload)
        && let Err(err) = future.await
    {
        tracing::warn!(err = %err, "message:received emit failed");
    }
}

/// `enqueuePhotoOptimizations`: the background re-encode of a new message's
/// photos that the channel did not encode before the first write.
async fn enqueue_photo_optimizations(
    deps: &Deps,
    workspace_id: i64,
    incoming: &IncomingMessage,
    stored: &repo_message::MessageWithAttachments,
) {
    use chatbotx_jobs::queues::stored_image::{
        StoredImageJob, StoredImageOptimizationConfig, enqueue_stored_image_optimizations,
    };
    let photos: std::collections::HashSet<&str> = incoming
        .attachments
        .iter()
        .filter(|a| a.is_photo_message && !a.optimized)
        .map(|a| a.origin_path.as_str())
        .collect();
    if photos.is_empty() {
        return;
    }
    let jobs = stored
        .attachments
        .iter()
        .filter(|a| photos.contains(a.origin_path.as_str()))
        .map(|a| StoredImageJob::Attachment {
            workspace_id: workspace_id.to_string(),
            object_key: a.origin_path.clone(),
            attachment_id: a.id.to_string(),
            attachment_created_at: chatbotx_db::repo::iso(a.created_at),
            message_created_at: chatbotx_db::repo::iso(a.message_created_at),
        })
        .collect();
    let config = StoredImageOptimizationConfig {
        enabled:
            chatbotx_core::utils::image_optimize_settings::is_stored_image_optimization_enabled(
                &chatbotx_core::env::ProcessEnv,
            ),
    };
    enqueue_stored_image_optimizations(deps.jobs.as_ref(), &config, jobs).await;
}

/// `processIncomingMessage`: route a new contact message to its automation.
pub async fn route_incoming_message(deps: &Deps, stored: &Stored) -> Result<(), JobError> {
    let Some(message) = &stored.message else {
        return Ok(());
    };
    let Some(message_id) = stored.message_id else {
        return Ok(());
    };
    let received = &stored.received;
    let conversation = &stored.conversation;
    let workspace_active = workspace_is_active(deps, conversation.workspace_id).await?;
    let suppress = received
        .thread_control
        .as_ref()
        .and_then(|t| str_of(t, "delivery"))
        == Some("standby");
    let can_automate = workspace_active && !suppress;
    if stored.step_input_skip {
        return Ok(());
    }
    if can_automate {
        crate::inbound_cart::announce(deps, stored, message, message_id).await?;
    }

    // The appointment cancel postback replaces the flow action.
    // ponytail: routing ends here (the original falls through with no
    // action left); the button title is not matched as a typed reply.
    if !suppress
        && let Some(payload) = received.postback_action.as_deref().filter(|p| {
            chatbotx_core::encryption::parse_appointment_cancel_postback(Some(p)).is_some()
        })
    {
        crate::appointment::cancel_postback(deps, stored, payload, workspace_active).await;
        return Ok(());
    }

    // A template FLOW button's response (`captureTemplateFlowResponse`).
    if let Some(token) = received
        .template_flow_token
        .as_deref()
        .filter(|_| can_automate)
        && let Some(flow_response) = chatbotx_business::whatsapp::whatsapp_flow_response(
            received
                .message
                .as_ref()
                .and_then(|m| m.content_attributes.as_ref()),
        )
        .and_then(|entity| entity.get("flowResponse"))
    {
        let flow_files = flow_files(deps, received);
        add_job(
            deps,
            QueueName::Integration,
            "captureTemplateFlowResponse",
            json!({ "type": "captureTemplateFlowResponse", "data": {
                "workspaceId": conversation.workspace_id.to_string(),
                "conversationId": conversation_json(conversation),
                "contactInboxId": contact_inbox_json(&stored.contact_inbox),
                "messageId": message_id.to_string(),
                "templateFlowToken": token,
                "flowResponse": flow_response,
                "flowFiles": flow_files,
            }}),
            JobOptions {
                job_id: Some(format!("template-flow-response-{message_id}")),
                ..JobOptions::default()
            },
        )
        .await?;
    }

    // Flow postbacks / quick replies (`automatedResponseService.enqueueFlowAction`).
    let action = received
        .postback_action
        .as_ref()
        .map(|a| ("postback", a))
        .or_else(|| {
            received
                .quick_reply_action
                .as_ref()
                .map(|a| ("quickReply", a))
        });
    if let Some((kind, action)) = action {
        if can_automate {
            enqueue_flow_action(deps, stored, kind, action, message_id).await?;
        }
        return Ok(());
    }
    if let Some(r#ref) = received.r#ref.as_ref().filter(|_| can_automate) {
        add_job(
            deps,
            QueueName::Integration,
            "runRef",
            json!({ "type": "runRef", "data": {
                "conversationId": conversation_json(conversation),
                "contactInboxId": contact_inbox_json(&stored.contact_inbox),
                "ref": r#ref,
                "messageId": message_id.to_string(),
                "isNewContact": stored.is_new_contact,
            }}),
            JobOptions::default(),
        )
        .await?;
    }
    if !can_automate {
        return Ok(());
    }
    let from_contact = str_of(message, "senderType") == Some("contact");
    if from_contact && crate::story_automation::dispatch(deps, stored, message, message_id).await? {
        return Ok(());
    }
    let text = str_of(message, "text")
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    let attachment_types: Vec<String> = message["attachments"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| str_of(x, "fileType").map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let is_location = str_of(message, "contentType") == Some("location");
    let has_keyword_media = attachment_types
        .iter()
        .any(|t| chatbotx_business::keyword_match::is_keyword_media_file_type(t));
    let additional = conversation
        .additional_attributes
        .as_ref()
        .map(|j| j.0.clone())
        .unwrap_or(Value::Null);
    let routing = crate::integration::resolve_incoming_text_routing(
        &additional,
        from_contact && (text.is_some() || !attachment_types.is_empty() || is_location),
        from_contact && text.is_some(),
        from_contact && has_keyword_media,
        async || ensure_active(deps, conversation).await,
    )
    .await;
    use crate::integration::IncomingRouting;
    match routing {
        IncomingRouting::AutomatedResponse => {
            enqueue_automated_response(deps, stored, message_id, text, attachment_types).await?;
        }
        IncomingRouting::Challenge(challenge) => {
            add_job(
                deps,
                QueueName::Integration,
                "runChallenge",
                json!({ "type": "runChallenge", "data": {
                    "conversationId": conversation.id.to_string(),
                    "contactInboxId": stored.contact_inbox.id.to_string(),
                    "messageId": message_id.to_string(),
                    "messageCreatedAt": message["createdAt"],
                    "challenge": challenge,
                }}),
                JobOptions {
                    job_id: Some(format!(
                        "questionnaire-challenge-{}-{message_id}",
                        conversation.id
                    )),
                    ..JobOptions::default()
                },
            )
            .await?;
        }
        IncomingRouting::HandoffReentry => {
            crate::handoff_reentry::enqueue(
                deps,
                conversation.id,
                stored.contact_inbox.id,
                message_id,
                conversation.workspace_id,
            )
            .await;
        }
        IncomingRouting::None => {}
    }
    Ok(())
}

pub(crate) async fn add_job(
    deps: &Deps,
    queue: QueueName,
    name: &str,
    data: Value,
    opts: JobOptions,
) -> Result<(), JobError> {
    deps.jobs
        .add(queue, name, data, opts)
        .await
        .map(|_| ())
        .map_err(|e| fail(e.to_string()))
}

/// `workspaceService.isActiveNow(workspace)`.
pub(crate) async fn workspace_is_active(deps: &Deps, workspace_id: i64) -> Result<bool, JobError> {
    let row = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "isActive", "startTime", "endTime", "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                    params![workspace_id],
                    |r| {
                        Ok((
                            r.get::<_, bool>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    Ok(row.is_some_and(|(active, start, end, tz)| {
        chatbotx_business::workspace::is_active_now(
            active,
            start.as_deref(),
            end.as_deref(),
            &tz,
            now_ms(),
        )
    }))
}

/// `conversationService.ensureActive`: bot on, or a passed `botResumeAt`
/// turns it back on.
pub(crate) async fn ensure_active(deps: &Deps, conversation: &Conversation) -> bool {
    if conversation.bot_enabled {
        return true;
    }
    let Some(resume_at) = conversation.bot_resume_at.filter(|at| *at <= now_ms()) else {
        return false;
    };
    let _ = resume_at;
    let (id, workspace_id) = (conversation.id, conversation.workspace_id);
    let result = deps
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "Conversation" SET "botEnabled" = 1, "botResumeAt" = NULL, "updatedAt" = ?1
                   WHERE "id" = ?2 AND "workspaceId" = ?3"#,
                params![now_ms(), id, workspace_id],
            )?;
            Ok(())
        })
        .await;
    if let Err(err) = result {
        tracing::warn!(err = %err, conversation_id = id, "Bot auto-resume failed");
    }
    true
}

/// `automatedResponseService.enqueue` (`enqueueMessage`): the delayed,
/// debounced `processAutomatedResponse` job plus the pending-message list.
/// A media-only message is queued only when a keyword rule matches it.
pub(crate) async fn enqueue_automated_response(
    deps: &Deps,
    stored: &Stored,
    message_id: i64,
    text: Option<String>,
    attachment_file_types: Vec<String>,
) -> Result<(), JobError> {
    enqueue_automated_response_for(
        deps,
        (
            stored.conversation.id,
            stored.contact_inbox.id,
            stored.conversation.workspace_id,
        ),
        message_id,
        text,
        attachment_file_types,
    )
    .await
}

/// [`enqueue_automated_response`] by `(conversationId, contactInboxId,
/// workspaceId)`.
pub(crate) async fn enqueue_automated_response_for(
    deps: &Deps,
    (conversation_id, contact_inbox_id, workspace_id): (i64, i64, i64),
    message_id: i64,
    text: Option<String>,
    attachment_file_types: Vec<String>,
) -> Result<(), JobError> {
    use chatbotx_business::keyword_match::{MatchInput, matches_any_keyword_rule};
    let has_input = text.is_some() || !attachment_file_types.is_empty();
    if text.is_none() && has_input {
        let rules = crate::ai_agent::inbound_rules(deps, workspace_id).await?;
        let input = MatchInput {
            text: None,
            attachment_file_types: attachment_file_types.clone(),
        };
        if !matches_any_keyword_rule(
            &input,
            &rules.iter().map(|r| r.1.clone()).collect::<Vec<_>>(),
        ) {
            return Ok(());
        }
    }
    let (delay_secs, ttl_secs) = match smart_delay_seconds(
        deps,
        workspace_id,
        text.as_deref(),
        &attachment_file_types,
    )
    .await
    {
        Ok(Some(seconds)) => (seconds, seconds),
        Ok(None) => automated_response_timing(),
        Err(err) => {
            tracing::warn!(%err, "Smart delay lookup failed; using default timing");
            automated_response_timing()
        }
    };
    let key = automated_response_key(conversation_id, contact_inbox_id);
    let opts = JobOptions {
        job_id: Some(format!("automated-response-{message_id}")),
        delay_ms: Some(delay_secs * 1000),
        dedup: Some(Dedup {
            id: key.clone(),
            ttl_ms: Some(ttl_secs * 1000),
            extend: true,
            replace: true,
            keep_last_if_active: false,
        }),
        ..JobOptions::default()
    };
    let data = json!({ "type": PROCESS_AUTOMATED_RESPONSE, "data": {
        "conversationId": conversation_id.to_string(),
        "contactInboxId": contact_inbox_id.to_string(),
        "messageId": message_id.to_string(),
    }});
    // The list is written first so the job always finds its message.
    if let Err(err) = kv(deps)
        .push(&key, &message_id.to_string(), ttl_secs * 5)
        .await
    {
        tracing::error!(err = %err, "Unable to trigger automated response");
        return Ok(());
    }
    if let Err(err) = deps
        .jobs
        .add(QueueName::AiAgent, PROCESS_AUTOMATED_RESPONSE, data, opts)
        .await
    {
        tracing::error!(err = %err, "Unable to trigger automated response");
    }
    Ok(())
}

/// `automatedResponseService.enqueueFlowAction`: a button tap starts the
/// flow node it names (`runFlowPostback` / `runFlowQuickReply`).
/// `resolveFlowFiles`: the stored files of a Flow response's file answers
/// with their public URLs, by answer key (a key whose files all failed stays,
/// empty). `None` when the response has no files.
fn flow_files(deps: &Deps, received: &Received) -> Option<Value> {
    let message = received.message.as_ref()?;
    let answers = message
        .content_attributes
        .as_ref()?
        .get("files")?
        .as_object()?;
    if answers.is_empty() {
        return None;
    }
    let storage_url = crate::heavy::storage_url(deps);
    let resolved = answers
        .iter()
        .map(|(key, files)| {
            let kept: Vec<Value> = files
                .as_array()
                .into_iter()
                .flatten()
                .filter(|file| file["status"] == "stored")
                .filter_map(|file| {
                    let media_id = file["mediaId"].as_str()?;
                    let path = message
                        .attachments
                        .iter()
                        .find(|a| a.source_id.as_deref() == Some(media_id))?
                        .origin_path
                        .as_str();
                    let url =
                        chatbotx_core::utils::storage::get_public_file_url(path, &storage_url)
                            .ok()?;
                    Some(json!({
                        "mediaId": media_id,
                        "fileName": file["fileName"],
                        "mimeType": file["mimeType"],
                        "size": file["size"],
                        "url": url,
                    }))
                })
                .collect();
            (key.clone(), Value::Array(kept))
        })
        .collect();
    Some(Value::Object(resolved))
}

pub(crate) async fn enqueue_flow_action(
    deps: &Deps,
    stored: &Stored,
    kind: &str,
    action: &str,
    message_id: i64,
) -> Result<(), JobError> {
    let name = if kind == "postback" {
        "runFlowPostback"
    } else {
        "runFlowQuickReply"
    };
    let mut payload = json!({});
    if let Some(flow_response) = stored
        .received
        .message
        .as_ref()
        .and_then(|m| m.content_attributes.as_ref())
        .and_then(|a| a.get("flowResponse"))
    {
        payload["waFlowResponse"] = flow_response.clone();
    }
    if let Some(files) = flow_files(deps, &stored.received) {
        payload["flowFiles"] = files;
    }
    // `isDebounceEligible`: a double tap within the TTL runs once (the last
    // one); a Flow form answer is never debounced.
    let opts = if kind == "postback" && payload.get("waFlowResponse").is_some() {
        JobOptions::default()
    } else {
        let (delay_secs, ttl_secs) = automated_response_timing();
        JobOptions {
            delay_ms: Some(delay_secs * 1000),
            dedup: Some(Dedup {
                id: flow_action_key(stored.conversation.id, stored.contact_inbox.id, action),
                ttl_ms: Some(ttl_secs * 1000),
                extend: true,
                replace: true,
                keep_last_if_active: false,
            }),
            ..JobOptions::default()
        }
    };
    add_job(
        deps,
        QueueName::Integration,
        name,
        json!({ "type": name, "data": {
            "conversationId": conversation_json(&stored.conversation),
            "contactInboxId": contact_inbox_json(&stored.contact_inbox),
            "action": action,
            "ref": stored.received.r#ref,
            "messageId": message_id.to_string(),
            "payload": payload,
        }}),
        opts,
    )
    .await
}

/// `getFlowActionKey`: the action hashed (sha256, 16 hex chars).
fn flow_action_key(conversation_id: i64, contact_inbox_id: i64, action: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, action.as_bytes());
    let hash: String = digest.as_ref()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("automated-response:flow-action:{conversation_id}-{contact_inbox_id}:{hash}")
}

/// `IntegrationJobAction.messageStatus` (`message-status.ts`): the channel
/// reads the status; a `failed` status stores the send error, `delivered` /
/// `read` stamp the contact inbox.
pub async fn message_status(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let integration_type = str_of(data, "integrationType").unwrap_or("").to_owned();
    let identifier = str_of(data, "integrationIdentifier")
        .unwrap_or("")
        .to_owned();
    let Some((channel, ctx, inbox)) = deps
        .channels
        .resolve_by_identifier(
            &deps.db,
            deps.realtime.as_ref(),
            &integration_type,
            &identifier,
        )
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?
    else {
        return Ok(());
    };
    let payload = data.get("payload").cloned().unwrap_or(Value::Null);
    let Some(status) = channel
        .message_status(&ctx, payload)
        .await
        .map_err(|e| fail(e.to_string()))?
    else {
        return Ok(());
    };
    let workspace_id = inbox.workspace_id;
    let source_id = status.message_id.clone();
    let found = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT m."id", m."createdAt", m."conversationId", m."contactInboxId", m."contentAttributes"
                       FROM "Message" m WHERE m."workspaceId" = ?1 AND m."sourceId" = ?2
                       ORDER BY m."createdAt" DESC LIMIT 1"#,
                    params![workspace_id, source_id],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    let Some((message_id, created_at, conversation_id, contact_inbox_id, attributes)) = found
    else {
        return Ok(());
    };
    let Some(conversation) = crate::load_conversation(&deps.db, conversation_id).await? else {
        return Ok(());
    };
    // `message:delivered` / `message:failed` with the send's metadata (the
    // broadcast-ops listener). ponytail: `message:seen` and
    // `message:received` feed analytics only (Q.D13).
    if matches!(status.status.as_str(), "delivered" | "failed" | "read")
        && let Some(contact_inbox) =
            crate::load_contact_inbox(&deps.db, contact_inbox_id, conversation.contact_id).await?
    {
        let attributes: Value = attributes
            .and_then(|a| serde_json::from_str(&a).ok())
            .unwrap_or(Value::Null);
        let metadata = attributes
            .get("metadata")
            .filter(|m| m.is_object())
            .cloned()
            .unwrap_or_else(|| json!({ "type": "updateStatus" }));
        let mut action = json!({ "messageId": message_id.to_string() });
        for key in ["flowId", "flowVersionId"] {
            if let Some(value) = attributes.get(key).filter(|v| v.is_string()) {
                action[key] = value.clone();
            }
        }
        let at = status.timestamp.unwrap_or_else(now_ms);
        let ctx = crate::events::MessageContext::of(&conversation, &contact_inbox);
        let mut extra = json!({ "nodeId": attributes.get("nodeId").cloned().unwrap_or(json!("")) });
        let event_type = if status.status == "delivered" {
            extra["providerOccurredAt"] = json!(chatbotx_db::repo::iso(at));
            "message:delivered"
        } else if status.status == "read" {
            // `markReadByContact` first, then `message:seen` at the read time.
            mark_read_by_contact(deps, &conversation, &contact_inbox, at).await?;
            extra["occurredAt"] = json!(chatbotx_db::repo::iso(at));
            "message:seen"
        } else {
            extra["errorData"] = status
                .error
                .as_ref()
                .map_or(json!({}), |e| json!({ "message": e }));
            extra["willRetry"] = json!(false);
            "message:failed"
        };
        crate::events::emit_message_event(
            deps.events.as_ref(),
            event_type,
            &ctx,
            action,
            Some(&metadata),
            extra,
        )
        .await;
        if status.status == "delivered" {
            emit_received(
                deps,
                json!({
                    "workspaceId": workspace_id.to_string(),
                    "contactId": contact_inbox.contact_id.to_string(),
                    "contactInboxId": contact_inbox.id.to_string(),
                    "channel": inbox.channel,
                    "inboxId": inbox.inbox_id.to_string(),
                    "occurredAt": chatbotx_db::repo::iso(created_at),
                    "sourceId": status.message_id,
                }),
            )
            .await;
        }
    }
    match status.status.as_str() {
        "failed" => {
            crate::chat::persist_send_error(
                deps,
                &conversation,
                inbox.inbox_id,
                (message_id, created_at, None),
                Some(status.error.unwrap_or_else(|| "Message failed".into())),
                false,
            )
            .await;
        }
        "delivered" | "read" => {
            let at = status.timestamp.unwrap_or_else(now_ms);
            let contact_id = conversation.contact_id;
            let result = deps
                .db
                .write(move |tx| {
                    chatbotx_db::repo::contact_activity::upsert_last_delivered(
                        tx,
                        workspace_id,
                        &[(contact_id, at)],
                    )
                    .map_err(crate::repo_err)?;
                    Ok(())
                })
                .await;
            if let Err(err) = result {
                tracing::warn!(err = %err, contact_inbox_id, "Recording the delivery failed");
            }
        }
        _ => {}
    }
    Ok(())
}

/// `conversationService.markReadByContact`: the advance-only read mark of
/// the conversation and the contact inbox.
async fn mark_read_by_contact(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    seen_at: i64,
) -> Result<(), JobError> {
    let (workspace_id, conversation_id) = (conversation.workspace_id, conversation.id);
    let (contact_inbox_id, contact_id) = (contact_inbox.id, contact_inbox.contact_id);
    deps.db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "Conversation" SET "contactLastReadAt" = MAX(COALESCE("contactLastReadAt", ?1), ?1)
                   WHERE "id" = ?2 AND "workspaceId" = ?3"#,
                params![seen_at, conversation_id, workspace_id],
            )?;
            tx.execute(
                r#"UPDATE "ContactInbox" SET "contactLastReadAt" = MAX(COALESCE("contactLastReadAt", ?1), ?1), "updatedAt" = ?2
                   WHERE "id" = ?3 AND "contactId" = ?4"#,
                params![seen_at, now_ms(), contact_inbox_id, contact_id],
            )?;
            Ok(())
        })
        .await
        .map_err(db_fail)
}

#[cfg(test)]
pub(crate) mod tests {
    //! The API channel through the shared pipeline: contact, conversation
    //! and message rows, the redelivery dedup, the debounced keyword job and
    //! its text reply (`41-keywords-trigger` in small).
    use std::sync::Mutex;

    use chatbotx_db::Db;
    use chatbotx_jobs::{AddResult, Enqueuer, JobsError, NewJob};

    use super::*;

    #[test]
    fn flow_action_key_hashes_the_action() {
        assert_eq!(
            flow_action_key(1, 2, "a"),
            "automated-response:flow-action:1-2:ca978112ca1bbdca"
        );
    }

    #[derive(Default)]
    pub(crate) struct Recording(pub(crate) Mutex<Vec<(QueueName, String, Value, JobOptions)>>);

    impl Enqueuer for Recording {
        fn add(
            &self,
            queue: QueueName,
            name: &str,
            data: Value,
            opts: JobOptions,
        ) -> chatbotx_jobs::BoxFuture<'_, Result<AddResult, JobsError>> {
            self.0
                .lock()
                .unwrap()
                .push((queue, name.into(), data, opts));
            Box::pin(async {
                Ok(AddResult {
                    job_id: "1".into(),
                    created: true,
                })
            })
        }

        fn add_bulk(
            &self,
            _: QueueName,
            _: Vec<NewJob>,
        ) -> chatbotx_jobs::BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    pub(crate) async fn setup() -> (Deps, Arc<Recording>) {
        let dir = std::env::temp_dir().join(format!(
            "chatbotx-worker-inbound-{}-{}",
            std::process::id(),
            new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Arc::new(Db::open(dir.join("t.db")).unwrap());
        db.migrate().await.unwrap();
        db.write(|tx| {
            for sql in [
                r#"INSERT INTO "User" ("id", "email") VALUES (1, 'u@example.test')"#,
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (2, 'WS', 1)"#,
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (4, 2, 'Api', 'api', '4')"#,
                r#"INSERT INTO "IntegrationApi" ("id", "auth", "name", "tokenHash", "tokenPrefix", "workspaceId", "inboxId") VALUES (5, '{}', 'Api', 'h', 'p', 2, 4)"#,
                r#"INSERT INTO "AutomatedResponse" ("id", "workspaceId", "keywords", "status", "text", "type") VALUES (6, 2, '["hello"]', 1, 'Hi there', 'inbound')"#,
            ] {
                tx.execute(sql, [])?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let jobs = Arc::new(Recording::default());
        let deps = Deps {
            db,
            jobs: jobs.clone(),
            waiter: chatbotx_jobs::wait::CompletionRegistry::new(),
            realtime: None,
            channels: crate::Channels::builtin(),
            app_url: "http://app.test".into(),
            variables: None,
            ai: Arc::new(crate::ai::Ai::live()),
            files: None,
            webhook_http: Arc::new(crate::webhook::RecordingWebhookHttp::default()),
            channel_http: None,
            events: None,
            push: None,
        };
        (deps, jobs)
    }

    #[derive(Default)]
    pub(crate) struct Emitted(pub(crate) Mutex<Vec<(String, Value)>>);

    impl chatbotx_jobs::events::EventEmitter for Emitted {
        fn emit<'a>(
            &'a self,
            event_type: &'a str,
            payload: Value,
        ) -> Result<
            chatbotx_jobs::BoxFuture<'a, Result<String, chatbotx_jobs::events::EventBusError>>,
            chatbotx_jobs::events::EventBusError,
        > {
            self.0
                .lock()
                .unwrap()
                .push((event_type.to_owned(), payload));
            Ok(Box::pin(async { Ok("1-0".to_owned()) }))
        }
    }

    /// `emits message:received with origin: 'inbound' and
    /// isFirstIncomingMessage` (`received-message.test.ts`): true for the
    /// contact's first inbound message, false after; none on a redelivery.
    #[tokio::test]
    async fn a_new_inbound_message_emits_message_received_once() {
        let (mut deps, _) = setup().await;
        let emitted = Arc::new(Emitted::default());
        deps.events = Some(chatbotx_jobs::events::EventBuses {
            message: emitted.clone(),
            flow: emitted.clone(),
            dashboard: emitted.clone(),
            error_log: emitted.clone(),
        });
        let mut second = payload();
        second["payload"]["message"]["sourceId"] = json!("m-2");
        for data in [payload(), payload(), second] {
            incoming_message(&deps, &data).await.unwrap();
        }
        let received: Vec<Value> = emitted
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t == "message:received")
            .map(|(_, p)| p.clone())
            .collect();
        assert_eq!(received.len(), 2, "{received:?}");
        assert_eq!(received[0]["origin"], "inbound");
        assert_eq!(received[0]["channel"], "api");
        assert_eq!(received[0]["inboxId"], "4");
        assert_eq!(received[0]["sourceId"], "m-1");
        assert_eq!(received[0]["isFirstIncomingMessage"], true);
        assert_eq!(received[1]["isFirstIncomingMessage"], false);
        assert!(received[0]["messageId"].is_string());
    }

    fn payload() -> Value {
        json!({ "integrationType": "api", "integrationIdentifier": "4", "payload": {
            "contact": { "sourceId": "c-1", "firstName": "Ada" },
            "message": { "sourceId": "m-1", "text": "hello", "contentType": "text" },
        }})
    }

    async fn count(deps: &Deps, sql: &'static str) -> i64 {
        deps.db
            .read(move |c| Ok(c.query_row(sql, [], |r| r.get(0))?))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn api_message_creates_rows_once_and_the_keyword_rule_replies() {
        let (deps, jobs) = setup().await;
        incoming_message(&deps, &payload()).await.unwrap();
        incoming_message(&deps, &payload()).await.unwrap();

        assert_eq!(
            count(
                &deps,
                r#"SELECT count(*) FROM "Contact" WHERE "firstName" = 'Ada'"#
            )
            .await,
            1
        );
        assert_eq!(
            count(&deps, r#"SELECT count(*) FROM "ContactInbox" WHERE "sourceId" = 'c-1' AND "channel" = 'api' AND "lastIncomingMessageAt" IS NOT NULL"#).await,
            1
        );
        assert_eq!(
            count(&deps, r#"SELECT count(*) FROM "Conversation""#).await,
            1
        );
        assert_eq!(
            count(
                &deps,
                r#"SELECT count(*) FROM "Message" WHERE "sourceId" = 'm-1' AND "senderType" = 'contact' AND "messageType" = 'incoming'"#
            )
            .await,
            1
        );
        let (notify, queued): (Vec<_>, Vec<_>) = jobs
            .0
            .lock()
            .unwrap()
            .drain(..)
            .partition(|(queue, ..)| *queue == QueueName::Notification);
        // `enqueueIncomingNotification` of the new inbound row only.
        assert_eq!(notify.len(), 1, "{notify:?}");
        assert_eq!(notify[0].1, "notifyIncomingMessage");
        assert_eq!(notify[0].2["data"]["messageText"], "hello");
        assert_eq!(notify[0].2["data"]["attachmentCount"], 0);
        assert_eq!(
            notify[0].3.job_id,
            Some(format!(
                "notify-incoming-{}",
                notify[0].2["data"]["messageId"].as_str().unwrap()
            ))
        );
        assert_eq!(queued.len(), 1, "a redelivery queues nothing");
        let (queue, name, data, opts) = &queued[0];
        assert_eq!(
            (*queue, name.as_str()),
            (QueueName::AiAgent, PROCESS_AUTOMATED_RESPONSE)
        );
        assert_eq!(opts.delay_ms, Some(2000));
        let dedup = opts.dedup.as_ref().unwrap();
        assert!(dedup.id.starts_with("automated-response:") && dedup.extend && dedup.replace);
        assert_eq!(
            opts.job_id.as_deref(),
            Some(
                format!(
                    "automated-response-{}",
                    data["data"]["messageId"].as_str().unwrap()
                )
                .as_str()
            )
        );

        crate::ai_agent::process_automated_response(&deps, &data["data"])
            .await
            .unwrap();
        let replies: Vec<_> = jobs.0.lock().unwrap().drain(..).collect();
        assert_eq!(replies.len(), 1);
        let (queue, name, data, _) = &replies[0];
        assert_eq!(
            (*queue, name.as_str()),
            (QueueName::Chat, "sendChatMessage")
        );
        assert_eq!(data["data"]["text"], "Hi there");
        assert_eq!(
            count(&deps, r#"SELECT count(*) FROM "ContactActivity" WHERE "activityType" = 'automatedResponseMatched' AND "referenceId" = 6"#).await,
            1
        );
        // The pending list is cleared: a second run sends nothing.
        crate::ai_agent::process_automated_response(
            &deps,
            &json!({
                "conversationId": queued[0].2["data"]["conversationId"],
                "contactInboxId": queued[0].2["data"]["contactInboxId"],
            }),
        )
        .await
        .unwrap();
        assert!(jobs.0.lock().unwrap().is_empty());
    }

    /// A keyword miss with no AI agent starts the default reply flow, once
    /// per day per contact (`triggerDefaultReplyFlow`).
    #[tokio::test]
    async fn a_keyword_miss_starts_the_throttled_default_reply_flow() {
        let (deps, jobs) = setup().await;
        deps.db
            .write(|tx| {
                tx.execute(r#"INSERT INTO "Flow" ("id", "name", "workspaceId", "currentVersionId") VALUES (9, 'Default', 2, 19)"#, [])?;
                tx.execute(r#"UPDATE "Workspace" SET "defaultReply" = '9', "defaultReplyFrequency" = 'oncePerDay' WHERE "id" = 2"#, [])?;
                Ok(())
            })
            .await
            .unwrap();
        let run = |source: &'static str| {
            let deps = deps.clone();
            let jobs = Arc::clone(&jobs);
            async move {
                let mut data = payload();
                data["payload"]["message"] =
                    json!({ "sourceId": source, "text": "what is up", "contentType": "text" });
                incoming_message(&deps, &data).await.unwrap();
                let queued: Vec<_> = jobs.0.lock().unwrap().drain(..).collect();
                let data = queued
                    .iter()
                    .find(|(_, name, ..)| name == PROCESS_AUTOMATED_RESPONSE)
                    .map(|(_, _, data, _)| data.clone())
                    .unwrap();
                crate::ai_agent::process_automated_response(&deps, &data["data"])
                    .await
                    .unwrap();
                let sent: Vec<_> = jobs.0.lock().unwrap().drain(..).collect();
                (data, sent)
            }
        };
        let (trigger, sent) = run("m-1").await;
        assert_eq!(sent.len(), 1, "{sent:?}");
        let (queue, name, job, _) = &sent[0];
        assert_eq!(
            (*queue, name.as_str()),
            (QueueName::Integration, "sendFlow")
        );
        assert_eq!(job["data"]["flowId"], "9");
        assert_eq!(job["data"]["origin"], "channel");
        assert_eq!(
            job["data"]["triggerMessageId"],
            trigger["data"]["messageId"]
        );
        assert_eq!(
            job["data"]["trackingContext"]["triggerType"],
            "bot_response_default_reply_flow_no_ai_agent"
        );
        let (_, sent) = run("m-2").await;
        assert!(sent.is_empty(), "throttled for the day: {sent:?}");
    }

    #[tokio::test]
    async fn an_unknown_identifier_is_an_unrecoverable_failure() {
        let (deps, _) = setup().await;
        let mut data = payload();
        data["integrationIdentifier"] = json!("999");
        assert!(incoming_message(&deps, &data).await.is_err());
    }

    /// `sanitizeFlowAction`, the Skip tap, the contact hint, the value quick
    /// reply and `isThirdPartyEcho`.
    #[test]
    fn taps_and_echoes() {
        use chatbotx_core::flow_config::step_input_resolution::encode_step_input_skip_postback;
        let tap = |postback: Option<&str>, quick: Option<&str>, text: &str| Received {
            postback_action: postback.map(str::to_owned),
            quick_reply_action: quick.map(str::to_owned),
            message: Some(IncomingMessage {
                text: Some(text.to_owned()),
                ..IncomingMessage::incoming(None)
            }),
            ..Received::default()
        };
        let mut r = tap(Some("1161247330962636"), Some("junk"), "hi");
        sanitize_actions(&mut r, "messenger", false);
        assert_eq!(
            (r.postback_action.as_deref(), r.quick_reply_action),
            (Some("1161247330962636"), None)
        );
        let mut r = tap(Some("1161247330962636"), None, "hi");
        sanitize_actions(&mut r, "telegram", false);
        assert_eq!(r.postback_action, None);

        let skip = encode_step_input_skip_postback("11612473309626368");
        let mut r = tap(Some(&skip), None, "Skip");
        assert_eq!(
            step_input_skip_challenge_id(&r).as_deref(),
            Some("11612473309626368")
        );
        sanitize_actions(&mut r, "messenger", true);
        assert_eq!(r.postback_action, None);

        let mut r = tap(
            None,
            Some("GET_USER_DATA_CONTACT_HINT|+84901234567"),
            "+849012…",
        );
        sanitize_actions(&mut r, "messenger", false);
        assert_eq!(r.quick_reply_action, None);
        assert_eq!(r.message.unwrap().text.as_deref(), Some("+84901234567"));
        let mut r = tap(None, Some("84901234567"), "84901234567");
        sanitize_actions(&mut r, "messenger", false);
        assert_eq!(r.quick_reply_action, None);

        let mut echo = tap(None, None, "hello");
        echo.echo_origin = Some("thirdParty".into());
        assert!(!is_third_party_echo(&echo));
        echo.message.as_mut().unwrap().message_type = "outgoing".into();
        assert!(is_third_party_echo(&echo));
        echo.message.as_mut().unwrap().content_attributes =
            Some(json!({ "type": "story_reply", "story": { "id": "s1" } }));
        assert!(!is_third_party_echo(&echo));
    }
}
