//! The rich-response button store of a tap (`flow.ts`
//! `tryRunRichButtonFallback`, `rich-response/button-payload.ts`,
//! `rich-response/action-executor.ts`) and the poll chat runtime
//! (`handlers/poll/send.ts`, `handlers/poll/handle-tap.ts`).

use std::collections::BTreeMap;

use chatbotx_business::conversation_actions::{self, ActionJob, Assigned};
use chatbotx_business::facebook_lead_ads::{rich_system_columns, set_contact_columns};
use chatbotx_business::poll::bot_strings::{PollBotStrings, get_poll_bot_strings};
use chatbotx_business::poll::chat_messages::{
    CandidateCard, PollCard, build_candidates_message, build_poll_cards_message,
    build_vote_registered_message,
};
use chatbotx_business::poll::vote::{CastVoteOutcome, cast_vote};
use chatbotx_business::poll::{PollState, get_poll_state};
use chatbotx_business::questionnaire::is_rich_system_contact_field;
use chatbotx_business::{contact_custom_field, tag};
use chatbotx_core::flow_config as fc;
use chatbotx_core::utils::id::create_id;
use chatbotx_db::repo::models::{ContactInbox, Conversation, Poll};
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::poll::{self as poll_repo, PollWithCandidates, VoteInput};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::{Deps, db_fail, fail, str_of};

/// `POLL_FLOW_CONTEXT_ID`: the flow id of a poll message (no flow has it).
pub const POLL_FLOW_CONTEXT_ID: &str = "0";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// `RICH_RESPONSE_FALLBACK_LOOKBACK_MS`.
const LOOKBACK_MS: i64 = 7 * DAY_MS;
/// `POLL_RICH_RESPONSE_LOOKBACK_MS`: poll cards stay votable longer.
const POLL_LOOKBACK_MS: i64 = 90 * DAY_MS;
/// `MESSAGE_ORDER_DELAY_MS`.
const MESSAGE_ORDER_DELAY_MS: i64 = 250;
const LIST_LIMIT: i64 = 10;

/// `richActionSchema` (parsed).
#[derive(Debug, Clone, PartialEq)]
pub enum RichAction {
    AddTag(String),
    RemoveTag(String),
    SetField(String, String),
    UnsetField(String),
    SendFlow(String),
    Transfer,
    Assign(String),
    Unassign,
}

impl RichAction {
    fn name(&self) -> &'static str {
        match self {
            RichAction::AddTag(_) => "add_tag",
            RichAction::RemoveTag(_) => "remove_tag",
            RichAction::SetField(..) => "set_field_value",
            RichAction::UnsetField(_) => "unset_field_value",
            RichAction::SendFlow(_) => "send_flow",
            RichAction::Transfer => "transfer_conversation_to",
            RichAction::Assign(_) => "assign_conversation",
            RichAction::Unassign => "unassign_conversation",
        }
    }
}

/// A trimmed string field of `min(1).max(max)` (UTF-16 length, as zod).
fn trimmed(value: &Value, key: &str, max: Option<usize>) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim();
    let len = text.encode_utf16().count();
    (len >= 1 && max.is_none_or(|max| len <= max)).then(|| text.to_owned())
}

/// `richActionSchema.parse`.
pub fn parse_rich_action(value: &Value) -> Option<RichAction> {
    Some(match str_of(value, "action")? {
        "add_tag" => RichAction::AddTag(trimmed(value, "tag_name", Some(100))?),
        "remove_tag" => RichAction::RemoveTag(trimmed(value, "tag_name", Some(100))?),
        "set_field_value" => {
            let text = value.get("value")?.as_str()?;
            if text.encode_utf16().count() > 5000 {
                return None;
            }
            RichAction::SetField(trimmed(value, "field_name", Some(100))?, text.to_owned())
        }
        "unset_field_value" => RichAction::UnsetField(trimmed(value, "field_name", Some(100))?),
        "send_flow" => RichAction::SendFlow(trimmed(value, "flow_id", None)?),
        "transfer_conversation_to" if str_of(value, "value") == Some("human") => {
            RichAction::Transfer
        }
        "assign_conversation" => RichAction::Assign(trimmed(value, "admin_id", None)?),
        "unassign_conversation" => RichAction::Unassign,
        _ => return None,
    })
}

/// `z.array(richActionSchema).min(1)`.
pub fn parse_rich_actions(value: &Value) -> Option<Vec<RichAction>> {
    let actions = value
        .as_array()?
        .iter()
        .map(parse_rich_action)
        .collect::<Option<Vec<_>>>()?;
    (!actions.is_empty()).then_some(actions)
}

/// `RichButtonPayload` (`richButtonPayloadSchema`).
#[derive(Debug, Clone, PartialEq)]
pub enum ButtonPayload {
    SendFlow(String),
    Actions(Vec<RichAction>),
    Text,
    PollOpen(i64),
    PollVote(i64, i64),
    PollList,
    Unsupported(String),
}

/// `richButtonPayloadSchema.safeParse` (`None` = invalid). A poll id that
/// is not a bigint matches no poll: it is a poll list.
pub fn parse_button_payload(value: &Value) -> Option<ButtonPayload> {
    let id = |key: &str| trimmed(value, key, None);
    Some(match str_of(value, "type")? {
        "send_flow" => ButtonPayload::SendFlow(id("flowId")?),
        "actions" => ButtonPayload::Actions(parse_rich_actions(value.get("actions")?)?),
        "text" => {
            let text = value.get("text")?.as_str()?;
            let len = text.encode_utf16().count();
            if !(1..=1000).contains(&len) {
                return None;
            }
            ButtonPayload::Text
        }
        "poll_open" => ButtonPayload::PollOpen(id("pollId")?.parse().unwrap_or(-1)),
        "poll_vote" => ButtonPayload::PollVote(
            id("pollId")?.parse().unwrap_or(-1),
            id("candidateId")?.parse().unwrap_or(-1),
        ),
        "poll_list" => ButtonPayload::PollList,
        "unsupported" => ButtonPayload::Unsupported(str_of(value, "reason")?.to_owned()),
        _ => return None,
    })
}

/// `RichResponseContext`.
pub struct RichContext<'a> {
    pub conversation: &'a Conversation,
    pub contact_inbox: &'a ContactInbox,
    pub execution_id: String,
}

/// `executeRichActions`: every action in order; a failed `send_flow` or
/// transfer stops the rest. Answers `(executed, failed)`.
pub async fn execute_rich_actions(
    deps: &Deps,
    actions: &[RichAction],
    ctx: &RichContext<'_>,
) -> (usize, Vec<(&'static str, String)>) {
    let (mut executed, mut failed) = (0, Vec::new());
    for action in actions {
        match execute_rich_action(deps, action, ctx).await {
            Ok(()) => executed += 1,
            Err(reason) => {
                tracing::warn!(
                    workspace_id = ctx.conversation.workspace_id,
                    conversation_id = ctx.conversation.id,
                    contact_id = ctx.conversation.contact_id,
                    execution_id = ctx.execution_id,
                    action = action.name(),
                    error = reason,
                    "[rich-response] action failed"
                );
                failed.push((action.name(), reason));
                if matches!(action, RichAction::SendFlow(_) | RichAction::Transfer) {
                    break;
                }
            }
        }
    }
    (executed, failed)
}

pub(crate) async fn publish_assigned(
    deps: &Deps,
    ws: i64,
    assigned: Assigned,
) -> Result<(), String> {
    if let (Some(realtime), Some(event)) = (&deps.realtime, assigned.event) {
        realtime.queue_workspace_event(&ws.to_string(), event).await;
    }
    if let Some(ActionJob {
        queue,
        name,
        data,
        job_id,
    }) = assigned.job
    {
        deps.jobs
            .add(
                queue,
                name,
                data,
                JobOptions {
                    job_id,
                    ..JobOptions::default()
                },
            )
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn flow_exists(deps: &Deps, ws: i64, flow_id: &str) -> Result<bool, String> {
    let Ok(flow_id) = flow_id.parse::<i64>() else {
        return Ok(false);
    };
    deps.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![flow_id, ws],
                    |_| Ok(()),
                )
                .optional()?
                .is_some())
        })
        .await
        .map_err(|e| e.to_string())
}

async fn execute_rich_action(
    deps: &Deps,
    action: &RichAction,
    ctx: &RichContext<'_>,
) -> Result<(), String> {
    let (db, conversation) = (&deps.db, ctx.conversation);
    let (ws, contact_id) = (conversation.workspace_id, conversation.contact_id);
    let err = |e: chatbotx_business::BusinessError| e.to_string();
    match action {
        RichAction::AddTag(name) => {
            let (_, pairs) = tag::attach_by_names_to_contacts_with_pairs(
                db,
                ws,
                &[contact_id],
                std::slice::from_ref(name),
            )
            .await
            .map_err(err)?;
            chatbotx_business::tag_sync::enqueue_attach_many(deps.jobs.as_ref(), ws, &pairs).await;
        }
        RichAction::RemoveTag(name) => {
            let (_, pairs) = tag::detach_by_names_from_contacts_with_pairs(
                db,
                ws,
                &[contact_id],
                std::slice::from_ref(name),
            )
            .await
            .map_err(err)?;
            chatbotx_business::tag_sync::enqueue_detach_many(deps.jobs.as_ref(), ws, &pairs).await;
        }
        RichAction::SetField(field, value) if is_rich_system_contact_field(field) => {
            set_contact_columns(db, ws, contact_id, rich_system_columns(field, value))
                .await
                .map_err(err)?;
        }
        RichAction::SetField(field, value) => {
            contact_custom_field::set_value_by_key(
                db,
                contact_custom_field::SetValueByKeyInput {
                    workspace_id: ws,
                    contact_id,
                    keyword: field.clone(),
                    value: value.clone(),
                    ..Default::default()
                },
            )
            .await
            .map_err(err)?;
        }
        RichAction::UnsetField(field) if is_rich_system_contact_field(field) => {
            let columns = rich_system_columns(field, "x")
                .into_iter()
                .map(|(column, _)| (column, None))
                .collect();
            set_contact_columns(db, ws, contact_id, columns)
                .await
                .map_err(err)?;
        }
        RichAction::UnsetField(field) => {
            contact_custom_field::delete_by_key(db, ws, contact_id, field, false)
                .await
                .map_err(err)?;
        }
        RichAction::SendFlow(flow_id) => {
            if !flow_exists(deps, ws, flow_id).await? {
                return Err("Flow not found".into());
            }
            let job_id = format!(
                "rich-response-{}-{}-send-flow-{flow_id}",
                conversation.id, ctx.execution_id
            );
            deps.jobs
                .add(
                    QueueName::Integration,
                    crate::flow::SEND_FLOW,
                    json!({ "type": crate::flow::SEND_FLOW, "data": {
                        "conversationId": conversation.id.to_string(),
                        "contactInboxId": ctx.contact_inbox.id.to_string(),
                        "flowId": flow_id,
                        "origin": "webhook",
                    }}),
                    JobOptions {
                        job_id: Some(job_id),
                        ..JobOptions::default()
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        RichAction::Transfer => {
            conversation_actions::hand_off_if_bot_enabled(
                db,
                ws,
                conversation.id,
                true,
                "user_requested_human",
                "ai_system_tool",
                json!({}),
            )
            .await
            .map_err(err)?;
        }
        RichAction::Assign(admin_id) => {
            let user = admin_id.parse::<i64>().unwrap_or(-1);
            let member = db
                .read(move |conn| {
                    Ok(conn
                        .query_row(
                            r#"SELECT 1 FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                            params![ws, user],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_some())
                })
                .await
                .map_err(|e| e.to_string())?;
            if !member {
                return Err("Workspace member not found".into());
            }
            let on_shift =
                crate::steps::working_hours::list_on_shift_user_ids(db, ws, vec![user], now_ms())
                    .await
                    .map_err(|e| e.to_string())?;
            if !on_shift.contains(&user) {
                tracing::debug!(
                    workspace_id = ws,
                    conversation_id = conversation.id,
                    "Assignment skipped: the target agent is off shift"
                );
                return Ok(());
            }
            let assigned = conversation_actions::assign_one(
                db,
                ws,
                conversation.id,
                Some(format!("u_{user}")),
            )
            .await
            .map_err(err)?;
            publish_assigned(deps, ws, assigned).await?;
        }
        RichAction::Unassign => {
            let assigned = conversation_actions::assign_one(db, ws, conversation.id, None)
                .await
                .map_err(err)?;
            publish_assigned(deps, ws, assigned).await?;
        }
    }
    Ok(())
}

/// `findRichResponseByButton`: the `richResponse` of the tapped message
/// (when it has one), else of the newest message of the conversation that
/// stores `button_id`, not older than `since`.
fn find_rich_response_by_button(
    conn: &chatbotx_db::rusqlite::Connection,
    (ws, conversation_id, contact_inbox_id): (i64, i64, i64),
    message_id: Option<i64>,
    button_id: &str,
    since: i64,
) -> chatbotx_db::rusqlite::Result<Option<Value>> {
    let parse = |text: Option<String>| -> Option<Value> {
        let attributes: Value = serde_json::from_str(&text?).ok()?;
        attributes
            .get("richResponse")
            .filter(|v| v.is_object())
            .cloned()
    };
    if let Some(message_id) = message_id {
        let found: Option<Option<String>> = conn
            .query_row(
                r#"SELECT "contentAttributes" FROM "Message" WHERE "id" = ?1 AND "workspaceId" = ?2
                   AND "conversationId" = ?3 AND "contactInboxId" = ?4 AND "createdAt" >= ?5"#,
                params![message_id, ws, conversation_id, contact_inbox_id, since],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(rich) = found.and_then(parse) {
            return Ok(rich["buttonPayloads"]
                .get(button_id)
                .is_some()
                .then_some(rich));
        }
    }
    let text: Option<Option<String>> = conn
        .query_row(
            r#"SELECT "contentAttributes" FROM "Message" WHERE "workspaceId" = ?1
               AND "conversationId" = ?2 AND "contactInboxId" = ?3 AND "createdAt" >= ?4
               AND json_type("contentAttributes", '$.richResponse.buttonPayloads.' || json_quote(?5)) IS NOT NULL
               ORDER BY "createdAt" DESC, "id" DESC LIMIT 1"#,
            params![ws, conversation_id, contact_inbox_id, since, button_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(text.and_then(parse))
}

/// The answer of [`try_run_rich_button_fallback`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    NotHandled,
    Handled { enqueue_automated_response: bool },
}

const HANDLED: Fallback = Fallback::Handled {
    enqueue_automated_response: false,
};

/// `recoverUnresolvedPollTap`: a poll tap that lost its payload shows the
/// Polls list.
async fn recover_unresolved_poll_tap(
    deps: &Deps,
    button_id: &str,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
) -> Result<Fallback, JobError> {
    tracing::warn!(
        workspace_id = conversation.workspace_id,
        conversation_id = conversation.id,
        button_id,
        reason = "poll_tap_unresolved",
        "[poll] poll button payload could not be resolved, sending the Polls list"
    );
    handle_poll_tap(deps, &ButtonPayload::PollList, conversation, contact_inbox).await?;
    Ok(HANDLED)
}

/// `tryRunRichButtonFallback`: a tapped button of a rich response (AI
/// reply or poll message) runs its stored payload.
pub async fn try_run_rich_button_fallback(
    deps: &Deps,
    button_id: &str,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    flow_context_id: &str,
    message_id: Option<i64>,
) -> Result<Fallback, JobError> {
    if message_id.is_none() {
        tracing::warn!(
            workspace_id = conversation.workspace_id,
            conversation_id = conversation.id,
            contact_id = conversation.contact_id,
            button_id,
            reason = "missing_message_id",
            "[rich-response] cannot resolve rich button payload"
        );
    }
    let is_poll = flow_context_id == POLL_FLOW_CONTEXT_ID;
    let since = now_ms()
        - if is_poll {
            POLL_LOOKBACK_MS
        } else {
            LOOKBACK_MS
        };
    let scope = (conversation.workspace_id, conversation.id, contact_inbox.id);
    let button = button_id.to_owned();
    let rich = deps
        .db
        .read(move |conn| {
            Ok(find_rich_response_by_button(
                conn, scope, message_id, &button, since,
            )?)
        })
        .await
        .map_err(db_fail)?;
    let unresolved = || async {
        if is_poll {
            recover_unresolved_poll_tap(deps, button_id, conversation, contact_inbox).await
        } else {
            Ok(Fallback::NotHandled)
        }
    };
    let Some(rich) = rich else {
        return unresolved().await;
    };
    let execution_id = str_of(&rich, "executionId").unwrap_or("").to_owned();
    let entry = &rich["buttonPayloads"][button_id];
    if entry.is_null() || str_of(entry, "executionId") != Some(execution_id.as_str()) {
        tracing::warn!(
            workspace_id = conversation.workspace_id,
            conversation_id = conversation.id,
            contact_id = conversation.contact_id,
            button_id,
            execution_id,
            reason = "rich_button_payload_not_found",
            "[rich-response] rich button payload not found"
        );
        return unresolved().await;
    }
    let payload = match parse_button_payload(&entry["payload"]) {
        Some(ButtonPayload::Unsupported(reason)) => Err(reason),
        Some(payload) => Ok(payload),
        None => Err("invalid_rich_button_payload".to_owned()),
    };
    let payload = match payload {
        Ok(payload) => payload,
        Err(reason) => {
            tracing::warn!(
                workspace_id = conversation.workspace_id,
                conversation_id = conversation.id,
                contact_id = conversation.contact_id,
                button_id,
                execution_id,
                reason,
                "[rich-response] unsupported rich button payload"
            );
            return Ok(HANDLED);
        }
    };
    let actions = match payload {
        ButtonPayload::Text => {
            return Ok(Fallback::Handled {
                enqueue_automated_response: true,
            });
        }
        ButtonPayload::PollOpen(_) | ButtonPayload::PollVote(..) | ButtonPayload::PollList => {
            handle_poll_tap(deps, &payload, conversation, contact_inbox).await?;
            return Ok(HANDLED);
        }
        ButtonPayload::SendFlow(flow_id) => vec![RichAction::SendFlow(flow_id)],
        ButtonPayload::Actions(actions) => actions,
        ButtonPayload::Unsupported(_) => return Ok(HANDLED),
    };
    let ctx = RichContext {
        conversation,
        contact_inbox,
        execution_id,
    };
    execute_rich_actions(deps, &actions, &ctx).await;
    Ok(HANDLED)
}

/// `sendPollStep`: one `sendFlowMessage` with the poll flow id and the
/// rich-response button payloads.
async fn send_poll_step(
    deps: &Deps,
    conversation_id: i64,
    step: Value,
    button_payloads: BTreeMap<String, Value>,
    execution_id: &str,
) -> Result<(), JobError> {
    deps.jobs
        .add(
            QueueName::Chat,
            crate::chat::SEND_FLOW_MESSAGE,
            json!({ "type": crate::chat::SEND_FLOW_MESSAGE, "data": {
                "conversationId": conversation_id.to_string(),
                "flowId": POLL_FLOW_CONTEXT_ID,
                "step": step,
                "richResponse": { "executionId": execution_id, "buttonPayloads": button_payloads },
            }}),
            JobOptions {
                job_id: Some(format!("poll-{conversation_id}-{execution_id}")),
                ..JobOptions::default()
            },
        )
        .await
        .map(|_| ())
        .map_err(db_fail)
}

/// `loadPollLocale`: the workspace language, time zone and bot strings.
async fn load_poll_locale(
    deps: &Deps,
    ws: i64,
) -> Result<(String, String, &'static PollBotStrings), JobError> {
    let (language, timezone) = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "language", "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                    [ws],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?
        .ok_or_else(|| fail("Workspace not found"))?;
    let language = language.unwrap_or_else(|| "en".into());
    let strings = get_poll_bot_strings(&language);
    Ok((language, timezone.unwrap_or_else(|| "UTC".into()), strings))
}

fn poll_card(poll: &Poll) -> PollCard {
    PollCard {
        id: poll.id.to_string(),
        name: poll.name.clone(),
        image_url: poll.image_url.clone(),
        starts_at: poll.starts_at,
        ends_at: poll.ends_at,
    }
}

/// `sendPollList` (`sendPollCard` = a list of one).
async fn send_poll_list(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    polls: &[PollCard],
) -> Result<(), JobError> {
    let (language, timezone, strings) = load_poll_locale(deps, conversation.workspace_id).await?;
    let execution_id = create_id();
    let message = build_poll_cards_message(
        polls,
        &contact_inbox.channel,
        strings,
        &language,
        &timezone,
        &execution_id,
        now_ms(),
    );
    send_poll_step(
        deps,
        conversation.id,
        message.step,
        message.button_payloads,
        &execution_id,
    )
    .await
}

/// `sendPollCard`.
pub async fn send_poll_card(
    deps: &Deps,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    poll: &Poll,
) -> Result<(), JobError> {
    send_poll_list(deps, conversation, contact_inbox, &[poll_card(poll)]).await
}

/// `sendCandidates`.
async fn send_candidates(
    deps: &Deps,
    conversation: &Conversation,
    poll: &PollWithCandidates,
) -> Result<(), JobError> {
    let (_, _, strings) = load_poll_locale(deps, conversation.workspace_id).await?;
    let execution_id = create_id();
    let candidates: Vec<CandidateCard> = poll
        .candidates
        .iter()
        .map(|c| CandidateCard {
            id: c.id.to_string(),
            name: c.name.clone(),
            description: c.description.clone(),
            image_url: c.image_url.clone(),
        })
        .collect();
    let message = build_candidates_message(
        &poll.poll.id.to_string(),
        &poll.poll.image_url,
        &candidates,
        strings,
        &execution_id,
    );
    send_poll_step(
        deps,
        conversation.id,
        message.step,
        message.button_payloads,
        &execution_id,
    )
    .await
}

/// `pollService.findUnscopedWithCandidates` + the workspace check.
pub async fn find_poll_in_workspace(
    deps: &Deps,
    poll_id: i64,
    ws: i64,
) -> Result<Option<PollWithCandidates>, JobError> {
    let found = deps
        .db
        .read(move |conn| {
            poll_repo::find_unscoped_with_candidates(conn, poll_id).map_err(crate::repo_err)
        })
        .await
        .map_err(db_fail)?;
    Ok(found.filter(|p| p.poll.workspace_id == ws))
}

/// `handlePollTap`.
pub async fn handle_poll_tap(
    deps: &Deps,
    payload: &ButtonPayload,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
) -> Result<(), JobError> {
    let ws = conversation.workspace_id;
    match *payload {
        ButtonPayload::PollList => {
            // WhatsApp cards are list rows: an ended poll is not actionable.
            let running_at = (contact_inbox.channel == "whatsapp").then(now_ms);
            let polls = deps
                .db
                .read(move |conn| {
                    poll_repo::list_recent(conn, ws, LIST_LIMIT, running_at)
                        .map_err(crate::repo_err)
                })
                .await
                .map_err(db_fail)?;
            if !polls.is_empty() {
                let cards: Vec<PollCard> = polls.iter().map(poll_card).collect();
                return send_poll_list(deps, conversation, contact_inbox, &cards).await;
            }
            let (_, _, strings) = load_poll_locale(deps, ws).await?;
            let mut step =
                fc::steps::send_text_step_default_fn(&json!({ "text": strings.no_open_polls }));
            let parsed = fc::steps::send_text_step_schema().safe_parse(&step);
            if parsed.success
                && let Some(data) = parsed.data
            {
                step = data;
            }
            send_poll_step(deps, conversation.id, step, BTreeMap::new(), &create_id()).await
        }
        ButtonPayload::PollVote(poll_id, candidate_id) => {
            let outcome = cast_vote(
                &deps.db,
                VoteInput {
                    workspace_id: ws,
                    poll_id,
                    candidate_id,
                    contact_id: conversation.contact_id,
                    contact_inbox_id: Some(contact_inbox.id),
                },
                now_ms(),
            )
            .await
            .map_err(|e| fail(e.to_string()))?;
            match outcome {
                CastVoteOutcome::PollNotFound => {
                    tracing::warn!(poll_id, "Poll tap: poll not found");
                    Ok(())
                }
                CastVoteOutcome::CandidateNotFound(poll) => {
                    send_candidates(deps, conversation, &poll).await
                }
                CastVoteOutcome::NotRunning(poll) => {
                    send_poll_card(deps, conversation, contact_inbox, &poll.poll).await
                }
                CastVoteOutcome::Voted {
                    after_vote_flow_id,
                    after_vote_flow_job_id,
                    ..
                } => {
                    // The after-vote flow first: the vote is committed, and
                    // the job id dedups a retry. The delay keeps it after the
                    // success message.
                    if let (Some(flow_id), Some(job_id)) =
                        (after_vote_flow_id, after_vote_flow_job_id)
                    {
                        deps.jobs
                            .add(
                                QueueName::Integration,
                                crate::flow::SEND_FLOW,
                                json!({ "type": crate::flow::SEND_FLOW, "data": {
                                    "conversationId": conversation.id.to_string(),
                                    "contactInboxId": contact_inbox.id.to_string(),
                                    "flowId": flow_id.to_string(),
                                    "origin": "webhook",
                                }}),
                                JobOptions {
                                    job_id: Some(job_id),
                                    delay_ms: Some(MESSAGE_ORDER_DELAY_MS as u64),
                                    ..JobOptions::default()
                                },
                            )
                            .await
                            .map_err(db_fail)?;
                    }
                    let (_, _, strings) = load_poll_locale(deps, ws).await?;
                    let execution_id = create_id();
                    let message = build_vote_registered_message(
                        &contact_inbox.channel,
                        strings,
                        &execution_id,
                    );
                    send_poll_step(
                        deps,
                        conversation.id,
                        message.step,
                        message.button_payloads,
                        &execution_id,
                    )
                    .await
                }
            }
        }
        ButtonPayload::PollOpen(poll_id) => {
            let Some(poll) = find_poll_in_workspace(deps, poll_id, ws).await? else {
                tracing::warn!(poll_id, "Poll tap: poll not found");
                return Ok(());
            };
            if get_poll_state(poll.poll.starts_at, poll.poll.ends_at, now_ms())
                == PollState::Running
            {
                send_candidates(deps, conversation, &poll).await
            } else {
                send_poll_card(deps, conversation, contact_inbox, &poll.poll).await
            }
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_payloads_and_actions_parse_like_the_schemas() {
        assert_eq!(
            parse_button_payload(
                &json!({ "type": "poll_vote", "pollId": "7", "candidateId": "9" })
            ),
            Some(ButtonPayload::PollVote(7, 9))
        );
        assert_eq!(
            parse_button_payload(&json!({ "type": "text", "text": "" })),
            None
        );
        assert_eq!(
            parse_button_payload(&json!({ "type": "actions", "actions": [
                { "action": "add_tag", "tag_name": "  vip " },
                { "action": "transfer_conversation_to", "value": "human" },
            ]})),
            Some(ButtonPayload::Actions(vec![
                RichAction::AddTag("vip".into()),
                RichAction::Transfer
            ]))
        );
        // One invalid action rejects the list.
        assert_eq!(
            parse_rich_actions(&json!([{ "action": "add_tag", "tag_name": "" }])),
            None
        );
        assert_eq!(parse_rich_actions(&json!([])), None);
    }

    #[test]
    fn the_store_lookup_finds_the_newest_message_with_the_button() {
        let conn = chatbotx_db::rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"CREATE TABLE "Message" ("id" INTEGER, "workspaceId" INTEGER, "conversationId" INTEGER,
                 "contactInboxId" INTEGER, "createdAt" INTEGER, "contentAttributes" TEXT);
               INSERT INTO "Message" VALUES
                 (1, 1, 2, 3, 100, '{"richResponse":{"executionId":"a","buttonPayloads":{"b\"1":{"executionId":"a"}}}}'),
                 (2, 1, 2, 3, 200, '{"richResponse":{"executionId":"c","buttonPayloads":{"b\"1":{"executionId":"c"}}}}'),
                 (3, 1, 2, 3, 300, '{"text":"tap"}');"#,
        )
        .unwrap();
        let found = find_rich_response_by_button(&conn, (1, 2, 3), Some(3), "b\"1", 0).unwrap();
        assert_eq!(found.unwrap()["executionId"], "c");
        assert!(
            find_rich_response_by_button(&conn, (1, 2, 3), None, "b\"1", 250)
                .unwrap()
                .is_none()
        );
        assert!(
            find_rich_response_by_button(&conn, (1, 2, 3), None, "x", 0)
                .unwrap()
                .is_none()
        );
    }
}
