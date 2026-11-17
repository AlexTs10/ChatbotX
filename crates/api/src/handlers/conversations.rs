//! `apps/builder/src/features/conversations/api/public.ts`
//! (`conversationsPublicRouter`, `/v1/conversations`, scope `inbox`).
//! Workspace tokens only (no member scope / masking).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chatbotx_business::contact::avatar::{AvatarInbox, resolve_contact_avatar_url_with};
use chatbotx_business::contact_filter::{parse_strict_param, to_filter_criteria};
use chatbotx_business::conversation_actions as actions;
use chatbotx_business::conversation_list::{self as list, ConversationRow, ListCursor, ListInput};
use chatbotx_db::repo::contact::ContactInboxWithInbox;
use chatbotx_db::repo::models::{InboxTeam, User};
use chatbotx_db::repo::now_ms;
use chatbotx_jobs::JobOptions;
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Seg, Str, key};
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json,
    opt_date, opt_id,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "conversations.list",
    "conversations.get",
    "conversations.assign",
    "conversations.archive",
    "conversations.unarchive",
    "conversations.read",
    "conversations.unread",
    "conversations.follow",
    "conversations.unfollow",
    "conversations.enableBot",
    "conversations.disableBot",
    "conversations.threadControl",
];

const STATUSES: &[&str] = &[
    "noAdminReply",
    "unread",
    "followUp",
    "archived",
    "blocked",
    "slaBreaching",
    "slaBreached",
];
const CHANNELS: &[&str] = &[
    "omnichannel",
    "webchat",
    "messenger",
    "whatsapp",
    "zalo",
    "smtp",
    "telegram",
    "instagram",
    "threads",
    "tiktok",
    "api",
];

/// `possibleErrorsOnThreadControl`.
const THREAD_CONTROL: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("threadControlUnsupported", 400),
    ("threadControlFailed", 400),
];

// ---- output ---------------------------------------------------------------

fn user_json(user: &User) -> Value {
    json!({
        "id": id(user.id),
        "createdAt": date(user.created_at),
        "updatedAt": date(user.updated_at),
        "name": user.name,
        "email": user.email,
        "emailVerified": user.email_verified,
        "image": user.image,
        "isAnonymous": user.is_anonymous,
        "mustChangePassword": user.must_change_password,
        "tenantId": id(user.tenant_id),
    })
}

fn team_json(team: &InboxTeam) -> Value {
    json!({
        "id": id(team.id),
        "createdAt": date(team.created_at),
        "updatedAt": date(team.updated_at),
        "workspaceId": id(team.workspace_id),
        "name": team.name,
    })
}

/// `resolveAdReferral`: `{adTitle, sourceUrl}` of a paid-ad referral.
fn ad_referral(referral: Option<&Value>) -> Value {
    let Some(r) = referral.and_then(Value::as_object) else {
        return Value::Null;
    };
    let present = |k: &str| r.get(k).is_some_and(|v| !v.is_null());
    let ctwa = r
        .get("ctwaClid")
        .is_some_and(|v| !v.is_null() && v.as_str() != Some(""));
    let paid = present("adId") && r.get("source").and_then(Value::as_str) == Some("ADS");
    if !(ctwa || paid) {
        return Value::Null;
    }
    let field = |k: &str| r.get(k).cloned().unwrap_or(Value::Null);
    json!({ "adTitle": field("adTitle"), "sourceUrl": field("sourceUrl") })
}

/// `resolveGoogleAdsClick`: the click id type, never the id.
fn google_ads_click(referral: Option<&Value>) -> Value {
    let Some(r) = referral.and_then(Value::as_object) else {
        return Value::Null;
    };
    let truthy = |k: &str| {
        r.get(k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    };
    if !(truthy("gclid") || truthy("gbraid")) {
        return Value::Null;
    }
    json!({
        "clickIdType": if truthy("gclid") { "gclid" } else { "gbraid" },
        "receivedAt": r.get("googleClickReceivedAt").cloned().unwrap_or(Value::Null),
    })
}

/// `mapConversationContactInboxes` (full) / `mapCompactConversationContactInboxes`.
fn contact_inbox_json(row: &ContactInboxWithInbox, compact: bool) -> Value {
    let ci = &row.contact_inbox;
    let mut out = Map::new();
    out.insert("id".into(), id(ci.id));
    out.insert("contactId".into(), id(ci.contact_id));
    out.insert("inboxId".into(), id(ci.inbox_id));
    out.insert("channel".into(), json!(ci.channel));
    out.insert("source".into(), json!(ci.source));
    out.insert("sourceId".into(), json!(ci.source_id));
    out.insert("sourceUserId".into(), json!(ci.source_user_id));
    out.insert("sourceUsername".into(), json!(ci.source_username));
    out.insert("language".into(), json!(ci.language));
    out.insert("lastMessageAt".into(), opt_date(ci.last_message_at));
    out.insert(
        "lastIncomingMessageAt".into(),
        opt_date(ci.last_incoming_message_at),
    );
    out.insert(
        "contactLastReadAt".into(),
        opt_date(ci.contact_last_read_at),
    );
    if compact {
        out.insert(
            "inbox".into(),
            json!({
                "id": id(row.inbox.id),
                "name": row.inbox.name,
                "channel": row.inbox.channel,
            }),
        );
        return Value::Object(out);
    }
    let referral = ci.referral.as_ref().map(|r| &r.0);
    out.insert("inbox".into(), json!({ "name": row.inbox.name }));
    out.insert("adReferral".into(), ad_referral(referral));
    out.insert("googleAdsClick".into(), google_ads_click(referral));
    out.insert("threadControlState".into(), json!(ci.thread_control_state));
    out.insert("threadOwnerRole".into(), json!(ci.thread_owner_role));
    out.insert(
        "threadControlUpdatedAt".into(),
        opt_date(ci.thread_control_updated_at),
    );
    out.insert(
        "threadOwnerExpiresAt".into(),
        opt_date(ci.thread_owner_expires_at),
    );
    out.insert("threadOwnerAppId".into(), json!(ci.thread_owner_app_id));
    out.insert(
        "threadControlLastEvent".into(),
        json!(ci.thread_control_last_event),
    );
    Value::Object(out)
}

/// One `listConversationsItemResource` / `findConversationResponse.data`.
pub(crate) fn conversation_json(state: &ApiState, row: &ConversationRow, compact: bool) -> Value {
    let c = &row.conversation;
    let contact = row.contact.as_ref().map_or(Value::Null, |contact| {
        let mut out = super::contacts::contact_json(contact);
        let inboxes: Vec<AvatarInbox> = row
            .contact_inboxes
            .iter()
            .map(|r| AvatarInbox {
                id: r.contact_inbox.id,
                channel: r.contact_inbox.channel.clone(),
                last_message_at: r.contact_inbox.last_message_at,
            })
            .collect();
        // `(key) => key`: a stored avatar key stays raw.
        let avatar = resolve_contact_avatar_url_with(
            contact.workspace_id,
            contact.avatar.as_deref(),
            &inboxes,
            &super::contacts::tenant_urls(state),
            now_ms(),
            &|key| Some(key.to_owned()),
        );
        out.insert("avatar".into(), json!(avatar));
        Value::Object(out)
    });
    let messages: Vec<Value> = row
        .last_message
        .iter()
        .map(|m| {
            let mut value = chatbotx_business::message::message_json(m);
            // `attachmentCountOnly`: the count, not the rows.
            if let Some(object) = value.as_object_mut() {
                object.remove("attachments");
            }
            value
        })
        .collect();
    json!({
        "id": id(c.id),
        "createdAt": date(c.created_at),
        "updatedAt": date(c.updated_at),
        "botEnabled": c.bot_enabled,
        "botResumeAt": opt_date(c.bot_resume_at),
        "archivedAt": opt_date(c.archived_at),
        "additionalAttributes": c.additional_attributes.as_ref().map_or(Value::Null, |a| a.0.clone()),
        "contactLastReadAt": opt_date(c.contact_last_read_at),
        "agentLastReadAt": opt_date(c.agent_last_read_at),
        "aiContextLastMessageId": opt_id(c.ai_context_last_message_id),
        "lastActivityAt": opt_date(c.last_activity_at),
        "followed": c.followed,
        "assignedUserId": opt_id(c.assigned_user_id),
        "assignedInboxTeamId": opt_id(c.assigned_inbox_team_id),
        "workspaceId": id(c.workspace_id),
        "contactId": id(c.contact_id),
        "sourceId": c.source_id,
        "lastStep": c.last_step,
        "currentStep": c.current_step,
        "adminRepliedAt": opt_date(c.admin_replied_at),
        "contactRepliedAt": opt_date(c.contact_replied_at),
        "awaitingHumanReplySince": opt_date(c.awaiting_human_reply_since),
        "assignedUserAt": opt_date(c.assigned_user_at),
        "responseDueAt": opt_date(c.response_due_at),
        "responseSlaBreachingAt": opt_date(c.response_sla_breaching_at),
        "responseSlaTargetSeconds": c.response_sla_target_seconds,
        "contact": contact,
        "contactInboxes": row.contact_inboxes.iter().map(|r| contact_inbox_json(r, compact)).collect::<Vec<_>>(),
        "assignedUser": row.assigned_user.as_ref().map_or(Value::Null, user_json),
        "assignedInboxTeam": row.assigned_inbox_team.as_ref().map_or(Value::Null, team_json),
        "messages": messages,
    })
}

// ---- cursor ---------------------------------------------------------------

/// `encodeCursor({ lastActivityAt, id })` (base64 JSON).
pub(crate) fn encode_cursor(cursor: &ListCursor) -> String {
    let body = json!({
        "lastActivityAt": cursor.last_activity_at.map(chatbotx_db::repo::iso),
        "id": cursor.id.to_string(),
    });
    STANDARD.encode(body.to_string())
}

/// `decodeCursor(.., conversationCursorSchema)`; `None` = the first page.
fn decode_cursor(text: &str) -> Option<ListCursor> {
    let bytes = super::contact_inboxes::lenient_base64(text)?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let object = value.as_object()?;
    let last_activity_at = match object.get("lastActivityAt")? {
        Value::Null => None,
        Value::String(text) => Some(chatbotx_core::js::parse_date(text)?),
        Value::Number(n) => Some(n.as_i64()?),
        _ => return None,
    };
    let id = match object.get("id")? {
        Value::String(t) if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) => {
            t.parse().ok()?
        }
        _ => return None,
    };
    Some(ListCursor {
        last_activity_at,
        id,
    })
}

// ---- input ----------------------------------------------------------------

/// `jsonQueryParam`: a query string parsed as JSON; empty or unparsable =
/// absent (no filter).
fn json_query(input: &Input, name: &str) -> Option<Value> {
    match input.get(name)? {
        Value::String(text) if text.is_empty() => None,
        Value::String(text) => serde_json::from_str(text).ok(),
        Value::Null => None,
        other => Some(other.clone()),
    }
}

/// `z.array(conversationStatuses)` of a JSON query param.
fn statuses(input: &mut Input, name: &str) -> Vec<String> {
    let Some(value) = json_query(input, name) else {
        return Vec::new();
    };
    let Value::Array(items) = value else {
        input.issue(
            &key(name),
            "invalid_type",
            "Invalid input: expected array, received object",
        );
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let path = [Seg::Key(name.to_owned()), Seg::Index(index)];
        if let Some(tag) = input.enum_at(&path, Some(item), STATUSES, true) {
            out.push(tag);
        }
    }
    out
}

fn opt_enum(input: &mut Input, name: &str, options: &[&str]) -> Option<String> {
    let value = input.get(name).cloned();
    input.enum_at(&key(name), value.as_ref(), options, false)
}

pub(crate) fn list_input(input: &mut Input, workspace_id: i64) -> (ListInput, bool) {
    let bot_category = opt_enum(input, "botCategory", &["bot", "human", "all"]);
    let assigned_id = input.opt_string("assignedId", Str::default());
    let channel = opt_enum(input, "channel", CHANNELS);
    let _status = statuses(input, "status");
    let keyword = input.opt_string("keyword", Str::default());
    let bot_enabled = match input.get("botEnabled").cloned() {
        Some(Value::String(text)) if text == "true" => Some(true),
        Some(Value::String(text)) if text == "false" => Some(false),
        Some(other) => {
            input.bool_at(&key("botEnabled"), Some(&other), false);
            None
        }
        None => None,
    };
    let tags = statuses(input, "tags");
    let contact_filter = match json_query(input, "contactFilter") {
        None => None,
        Some(value) => match parse_strict_param(Some(&value)) {
            Ok(parsed) => parsed.as_ref().map(to_filter_criteria),
            Err(issues) => {
                input.extend_issues(issues);
                None
            }
        },
    };
    let compact = opt_enum(input, "view", &["full", "compact"]).as_deref() == Some("compact");
    let cursor = input.opt_string("cursor", Str::default());
    let per_page = input.opt_coerce_int(
        "perPage",
        Num {
            min: Some(1.0),
            ..Num::default()
        },
    );
    (
        ListInput {
            workspace_id,
            bot_category,
            assigned_id,
            channel,
            keyword,
            bot_enabled,
            tags,
            contact_filter,
            per_page,
            cursor: cursor.as_deref().and_then(decode_cursor),
            hide_email_and_phone: false,
        },
        compact,
    )
}

/// `assignConversationPublicRequest.assignedId`: `null` or
/// `/^[ut]_\S+$/` after trim.
fn assigned_id(input: &mut Input) -> Option<Option<String>> {
    match input.get("assignedId").cloned() {
        Some(Value::Null) => Some(None),
        Some(value) => {
            let text = input.string_at(
                &key("assignedId"),
                Some(&value),
                Str::trimmed(0, usize::MAX),
                true,
            )?;
            let rest = text.strip_prefix("u_").or_else(|| text.strip_prefix("t_"));
            if rest.is_some_and(|r| !r.is_empty() && !r.chars().any(char::is_whitespace)) {
                Some(Some(text))
            } else {
                input.issue(
                    &key("assignedId"),
                    "invalid_format",
                    "assignedId must start with 'u_' or 't_'",
                );
                None
            }
        }
        None => {
            input.string_at(&key("assignedId"), None, Str::default(), true);
            None
        }
    }
}

// ---- handlers -------------------------------------------------------------

pub(crate) async fn queue_jobs(state: &ApiState, job: Option<actions::ActionJob>) {
    let (Some(jobs), Some(job)) = (&state.jobs, job) else {
        return;
    };
    let opts = JobOptions {
        job_id: job.job_id,
        ..JobOptions::default()
    };
    if let Err(error) = jobs.add(job.queue, job.name, job.data, opts).await {
        tracing::warn!(%error, "conversation action job enqueue failed");
    }
}

fn success() -> Reply {
    Ok(json(200, json!({ "success": true })))
}

/// The path id of an action route (422 when malformed).
fn path_id(call: &Call<'_>) -> Result<i64, crate::orpc::OrpcError> {
    let mut input = Input::from_body(call.params, call.body);
    let conversation = input.digits_id("id");
    input.finish()?;
    Ok(conversation.unwrap_or(-1))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let found = |e| business_error(e, FINDING);
    let mutated = |e| business_error(e, MUTATING);
    match route_id {
        "conversations.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (request, compact) = list_input(&mut input, ws);
            input.finish()?;
            let page = list::list(db, request)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page
                .rows
                .iter()
                .map(|row| conversation_json(state, row, compact))
                .collect();
            Ok(json(
                200,
                json!({
                    "data": data,
                    "nextCursor": page.next_cursor.as_ref().map(encode_cursor),
                    "prevCursor": page.prev_cursor.as_ref().map(encode_cursor),
                }),
            ))
        }
        "conversations.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let conversation = input.digits_id("id");
            input.finish()?;
            let row = list::find(db, ws, conversation.unwrap_or(-1))
                .await
                .map_err(found)?;
            Ok(json(
                200,
                json!({ "data": conversation_json(state, &row, false) }),
            ))
        }
        "conversations.assign" => {
            let mut input = Input::from_body(call.params, call.body);
            let assigned = assigned_id(&mut input);
            let conversation = input.digits_id("id");
            input.finish()?;
            let conversation = conversation.unwrap_or(-1);
            list::find_or_fail(db, ws, conversation)
                .await
                .map_err(mutated)?;
            let done = actions::assign_one(db, ws, conversation, assigned.flatten())
                .await
                .map_err(mutated)?;
            if let Some(event) = done.event {
                state.queue_realtime(ws, event).await;
            }
            queue_jobs(state, done.job).await;
            success()
        }
        "conversations.archive" | "conversations.unarchive" => {
            let conversation = path_id(&call)?;
            list::find_or_fail(db, ws, conversation)
                .await
                .map_err(mutated)?;
            actions::set_archived(
                db,
                state.jobs.as_deref(),
                ws,
                conversation,
                route_id == "conversations.archive",
            )
            .await
            .map_err(mutated)?;
            success()
        }
        "conversations.read" | "conversations.unread" => {
            let conversation = path_id(&call)?;
            list::find_or_fail(db, ws, conversation)
                .await
                .map_err(mutated)?;
            let event = if route_id == "conversations.read" {
                actions::mark_read(db, ws, conversation).await
            } else {
                actions::mark_unread(db, ws, conversation).await
            }
            .map_err(mutated)?;
            if let Some(event) = event {
                state.queue_realtime(ws, event).await;
            }
            success()
        }
        "conversations.follow" | "conversations.unfollow" => {
            let conversation = path_id(&call)?;
            list::find_or_fail(db, ws, conversation)
                .await
                .map_err(mutated)?;
            actions::set_followed(db, ws, conversation, route_id == "conversations.follow")
                .await
                .map_err(mutated)?;
            success()
        }
        "conversations.enableBot" | "conversations.disableBot" => {
            let conversation = path_id(&call)?;
            list::find_or_fail(db, ws, conversation)
                .await
                .map_err(mutated)?;
            actions::set_bot_enabled(db, ws, conversation, route_id == "conversations.enableBot")
                .await
                .map_err(mutated)?;
            success()
        }
        _ => thread_control(state, call).await,
    }
}

/// `conversations.threadControl` (take / release / pass / sync).
async fn thread_control(state: &ApiState, call: Call<'_>) -> Reply {
    use chatbotx_business::thread_control as tc;
    let mut input = Input::from_body(call.params, call.body);
    let contact_inbox = input.digits_id("contactInboxId");
    let value = input.get("action").cloned();
    let action = input.enum_at(
        &key("action"),
        value.as_ref(),
        &["take", "release", "pass", "sync"],
        true,
    );
    let value = input.get("targetRole").cloned();
    let target_role = input.enum_at(&key("targetRole"), value.as_ref(), tc::roles::ALL, false);
    let metadata = input.opt_string(
        "metadata",
        Str::trimmed(
            1,
            chatbotx_db::repo::partials::thread_control::THREAD_CONTROL_METADATA_MAX_LENGTH,
        ),
    );
    // `refineThreadControlHandover`.
    if let Some(action) = &action {
        if target_role.is_some() && action != "pass" {
            input.issue(
                &key("targetRole"),
                "custom",
                "targetRole is only allowed with the `pass` action",
            );
        }
        if metadata.is_some() && !matches!(action.as_str(), "take" | "pass") {
            input.issue(
                &key("metadata"),
                "custom",
                "metadata is only allowed with the `take` or `pass` action",
            );
        }
    }
    let conversation = input.digits_id("id");
    input.finish()?;
    let (action, ws) = (action.unwrap_or_default(), call.workspace_id);
    let errors = |e| business_error(e, THREAD_CONTROL);
    let conversation = list::find_or_fail(&state.db, ws, conversation.unwrap_or(-1))
        .await
        .map_err(errors)?;
    let contact_inbox = contact_inbox.unwrap_or(-1);
    let graph = state.graph_url.as_str();
    let rt = state.realtime.as_ref();
    let result = if action == "sync" {
        tc::sync_conversation_owner(&state.db, rt, graph, &conversation, contact_inbox).await
    } else {
        tc::request_conversation_action(
            &state.db,
            rt,
            graph,
            &conversation,
            contact_inbox,
            &action,
            target_role.as_deref(),
            metadata.as_deref(),
        )
        .await
    };
    match result {
        Ok(row) => Ok(json(
            200,
            json!({ "status": "applied", "snapshot": tc::snapshot_json(&row) }),
        )),
        Err(error) => {
            let status = tc::map_error(error, &action).map_err(errors)?;
            Ok(json(200, json!({ "status": status })))
        }
    }
}
