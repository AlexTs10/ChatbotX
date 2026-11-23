//! `conversation/service.ts` actions of the public `/v1/conversations/{id}/*`
//! routes: `assignOne` (`resolveAssignmentTarget` + `updateAssignment`),
//! `archiveByIds` / `unarchiveByIds` (`updateArchived`), `setFollowed`,
//! `setBotEnabledByIds`, `updateReadStatus` and `markUnread`.
//!
//! The service writes the rows and returns the realtime envelope / job for
//! the caller (`ApiState`). Each change sends its flow-trigger event and its
//! `analytics:dashboard` row through [`crate::events::global`].
//!
//! ponytail: the actor (`assignedBy`, `archivedBy`, ...) and the dashboard
//! `triggerContext` are not passed by the callers yet; the events go without them.

use chatbotx_db::Db;
use chatbotx_db::repo::message::{
    FindLastByConversationOptions, SinceTimeInput, find_last_by_conversation, get_safe_since_time,
};
use chatbotx_db::repo::models::Conversation;
use chatbotx_db::repo::sql::{Sql, opt_int};
use chatbotx_db::repo::{iso, now_ms};
use chatbotx_db::rusqlite::OptionalExtension as _;
use chatbotx_jobs::QueueName;
use chatbotx_rt::protocol::{
    AssignmentRoute, ConversationRoute, route_for_assignment, route_for_conversation,
};
use chatbotx_rt::{Envelope, EventType};
use serde_json::{Value as Json, json};

use crate::conversation_list::find_or_fail_in;
use crate::errors::{BusinessError, Result};
use crate::tx;

/// The trigger event and the `analytics:dashboard` row of one conversation change.
/// `resolveAnalyticsAttribution`: the contact's latest connection; a failed read
/// leaves the row unattributed (the change is already committed).
pub(crate) async fn emit_change(
    db: &Db,
    workspace_id: i64,
    conversation_id: i64,
    contact_id: i64,
    trigger: Option<(&str, Json)>,
    dashboard_type: &str,
    extra: Json,
) {
    if let Some((event_type, metadata)) = trigger {
        crate::events::global::trigger(event_type, workspace_id, contact_id, metadata).await;
    }
    let attribution = chatbotx_db::repo::read(db, move |c| {
        chatbotx_db::repo::contact_inbox::list_latest_analytics_attribution(
            c,
            workspace_id,
            &[contact_id],
        )
    })
    .await
    .inspect_err(|err| {
        tracing::warn!(err = %err, workspace_id, "Conversation analytics attribution lookup failed; events stay unattributed");
    })
    .ok()
    .and_then(|rows| rows.into_iter().next());
    let mut payload = json!({
        "eventType": dashboard_type,
        "workspaceId": workspace_id.to_string(),
        "conversationId": conversation_id.to_string(),
        "occurredAt": iso(now_ms()),
    });
    if let Some((_, inbox_id, channel)) = attribution {
        payload["channel"] = json!(channel);
        payload["inboxId"] = json!(inbox_id.to_string());
    }
    if let (Json::Object(target), Json::Object(extra)) = (&mut payload, extra) {
        target.extend(extra);
    }
    crate::events::global::dashboard(payload).await;
}

/// `BOT_DISABLE_DURATION_MS`.
pub const BOT_DISABLE_DURATION_MS: i64 = 24 * 60 * 60 * 1000;
const YEAR_MS: i64 = 365 * 24 * 60 * 60 * 1000;

fn sid(id: Option<i64>) -> Option<String> {
    id.map(|id| id.to_string())
}

/// A job for the caller to enqueue.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionJob {
    pub queue: QueueName,
    pub name: &'static str,
    pub data: Json,
    pub job_id: Option<String>,
}

/// What `assignOne` did: the `conversationAssigned` event and the
/// `notifyConversationAssigned` job (a user assignee, no actor).
#[derive(Debug, Clone, PartialEq)]
pub struct Assigned {
    pub event: Option<Envelope>,
    pub job: Option<ActionJob>,
}

fn member_exists(conn: &chatbotx_db::rusqlite::Connection, ws: i64, user: i64) -> Result<bool> {
    let q = Sql::new(
        "SELECT 1 FROM \"WorkspaceMember\" WHERE \"workspaceId\" = ? AND \"userId\" = ? LIMIT 1",
        vec![ws.into(), user.into()],
    );
    Ok(!q.query_map(conn, |_| Ok(()))?.is_empty())
}

fn team_exists(conn: &chatbotx_db::rusqlite::Connection, ws: i64, team: i64) -> Result<bool> {
    let q = Sql::new(
        "SELECT 1 FROM \"InboxTeam\" WHERE \"workspaceId\" = ? AND \"id\" = ? LIMIT 1",
        vec![ws.into(), team.into()],
    );
    Ok(!q.query_map(conn, |_| Ok(()))?.is_empty())
}

/// `resolveAssignmentTarget(.., "throw")`: `(assignedUserId, assignedInboxTeamId)`.
fn resolve_assignment_target(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    assigned_id: Option<&str>,
) -> Result<(Option<i64>, Option<i64>)> {
    let Some(assigned_id) = assigned_id else {
        return Ok((None, None));
    };
    if let Some(user) = assigned_id.strip_prefix("u_") {
        // A non-numeric id matches no member (Postgres: bigint cast error).
        return match user.parse::<i64>() {
            Ok(user) if member_exists(conn, workspace_id, user)? => Ok((Some(user), None)),
            _ => Err(BusinessError::exception(
                "User is not valid",
                "invalidAssignee",
                400,
            )),
        };
    }
    if let Some(team) = assigned_id.strip_prefix("t_") {
        return match team.parse::<i64>() {
            Ok(team) if team_exists(conn, workspace_id, team)? => Ok((None, Some(team))),
            _ => Err(BusinessError::not_found("Inbox team not found")),
        };
    }
    Err(BusinessError::exception(
        "assignedId must start with 'u_' or 't_'",
        "invalidAssignee",
        400,
    ))
}

/// `assignOne` for a workspace token (no `assignedBy`).
pub async fn assign_one(
    db: &Db,
    workspace_id: i64,
    conversation_id: i64,
    assigned_id: Option<String>,
) -> Result<Assigned> {
    let now = now_ms();
    let (assigned, change) = tx::write(db, move |conn| {
        let (user, team) = resolve_assignment_target(conn, workspace_id, assigned_id.as_deref())?;
        let Some(previous) = crate::conversation_list::find_in(conn, workspace_id, conversation_id)?
        else {
            return Ok((
                Assigned {
                    event: None,
                    job: None,
                },
                None,
            ));
        };
        // A reassignment recomputes the open window's SLA for the new agent.
        crate::response_sla::track_window(conn, workspace_id, &[conversation_id], true, |c| {
            Sql::new(
                "UPDATE \"Conversation\" SET \"assignedUserId\" = ?1, \"assignedInboxTeamId\" = ?2, \
                 \"assignedUserAt\" = CASE WHEN ?1 IS NULL THEN NULL WHEN \"assignedUserId\" IS NOT ?1 THEN ?3 ELSE \"assignedUserAt\" END, \
                 \"updatedAt\" = ?3 WHERE \"workspaceId\" = ?4 AND \"id\" = ?5",
                vec![
                    opt_int(user),
                    opt_int(team),
                    now.into(),
                    workspace_id.into(),
                    conversation_id.into(),
                ],
            )
            .execute(c)
        })?;
        let ids = vec![conversation_id.to_string()];
        let event = Envelope::routed(
            EventType::ConversationAssigned,
            json!({
                "conversationIds": ids,
                "assignedUserId": sid(user),
                "assignedInboxTeamId": sid(team),
            }),
            route_for_assignment(&AssignmentRoute {
                assigned_inbox_team_id: sid(team),
                assigned_user_id: sid(user),
                previous_assigned_inbox_team_ids: vec![sid(previous.assigned_inbox_team_id)],
                previous_assigned_user_ids: vec![sid(previous.assigned_user_id)],
            }),
        );
        let job = user.map(|user| ActionJob {
            queue: QueueName::Notification,
            name: "notifyConversationAssigned",
            data: json!({
                "type": "notifyConversationAssigned",
                "data": {
                    "workspaceId": workspace_id.to_string(),
                    "conversationId": conversation_id.to_string(),
                    "assignedUserId": user.to_string(),
                },
            }),
            job_id: Some(format!("notify-assigned-{conversation_id}-{user}")),
        });
        Ok((
            Assigned {
                event: Some(event),
                job,
            },
            Some((previous.contact_id, user.or(team))),
        ))
    })
    .await?;
    match change {
        Some((contact_id, Some(to))) => {
            let to = to.to_string();
            emit_change(
                db,
                workspace_id,
                conversation_id,
                contact_id,
                Some((
                    "conversationAssigned",
                    json!({ "conversationId": conversation_id.to_string(), "assignedTo": to }),
                )),
                "conversation:assigned",
                json!({ "toAssignee": to }),
            )
            .await;
        }
        Some((contact_id, None)) => {
            emit_change(
                db,
                workspace_id,
                conversation_id,
                contact_id,
                Some((
                    "conversationUnassigned",
                    json!({ "conversationId": conversation_id.to_string() }),
                )),
                "conversation:unassigned",
                json!({}),
            )
            .await;
        }
        None => {}
    }
    Ok(assigned)
}

/// `archiveByIds` / `unarchiveByIds` for one id. Archiving closes the DM
/// waiting window (`humanReplyWindowSet({ close, actor: "archive" })`).
///
/// An archive enqueues the release of the threads the contact's channels still own
/// (`releaseOwnedThreads`; a failed enqueue is logged). `jobs` `None` = no queue.
pub async fn set_archived(
    db: &Db,
    jobs: Option<&dyn chatbotx_jobs::Enqueuer>,
    workspace_id: i64,
    id: i64,
    archived: bool,
) -> Result<()> {
    let now = now_ms();
    let contact_id = tx::write(db, move |conn| {
        let q = if archived {
            let closes = "\"sourceId\" IS NULL AND \"awaitingHumanReplySince\" IS NOT NULL AND \"awaitingHumanReplySince\" <= ?1";
            Sql::new(
                format!(
                    "UPDATE \"Conversation\" SET \"archivedAt\" = ?1, \"updatedAt\" = ?1, \
                     \"awaitingHumanReplySince\" = CASE WHEN {closes} THEN NULL ELSE \"awaitingHumanReplySince\" END, \
                     \"awaitingHumanReplyClosedAt\" = CASE WHEN \"sourceId\" IS NULL THEN max(coalesce(\"awaitingHumanReplyClosedAt\", ?1), ?1) ELSE \"awaitingHumanReplyClosedAt\" END, \
                     \"responseDueAt\" = CASE WHEN {closes} THEN NULL ELSE \"responseDueAt\" END, \
                     \"responseSlaBreachingAt\" = CASE WHEN {closes} THEN NULL ELSE \"responseSlaBreachingAt\" END, \
                     \"responseSlaTargetSeconds\" = CASE WHEN {closes} THEN NULL ELSE \"responseSlaTargetSeconds\" END \
                     WHERE \"workspaceId\" = ?2 AND \"id\" = ?3"
                ),
                vec![now.into(), workspace_id.into(), id.into()],
            )
        } else {
            Sql::new(
                "UPDATE \"Conversation\" SET \"archivedAt\" = NULL, \"updatedAt\" = ?1 WHERE \"workspaceId\" = ?2 AND \"id\" = ?3",
                vec![now.into(), workspace_id.into(), id.into()],
            )
        };
        q.execute(conn)?;
        Ok(crate::conversation_list::find_in(conn, workspace_id, id)?.map(|c| c.contact_id))
    })
    .await?;
    if let (true, Some(jobs), Some(contact_id)) = (archived, jobs, contact_id)
        && let Err(error) = crate::thread_control::release_owned_threads_for_contacts(
            db,
            jobs,
            workspace_id,
            &[(id, contact_id)],
            now,
        )
        .await
    {
        tracing::warn!(%error, workspace_id, "Unable to enqueue thread release after archiving conversations");
    }
    if let Some(contact_id) = contact_id {
        let trigger = archived.then(|| ("archived", json!({ "conversationId": id.to_string() })));
        let kind = if archived {
            "conversation:archived"
        } else {
            "conversation:unarchived"
        };
        emit_change(db, workspace_id, id, contact_id, trigger, kind, json!({})).await;
    }
    Ok(())
}

/// `setFollowed` (404 "Conversation not found").
pub async fn set_followed(db: &Db, workspace_id: i64, id: i64, followed: bool) -> Result<()> {
    let now = now_ms();
    let contact_id = tx::write(db, move |conn| {
        let conversation = find_or_fail_in(conn, workspace_id, id)?;
        Sql::new(
            "UPDATE \"Conversation\" SET \"followed\" = ?1, \"updatedAt\" = ?2 WHERE \"workspaceId\" = ?3 AND \"id\" = ?4",
            vec![followed.into(), now.into(), workspace_id.into(), id.into()],
        )
        .execute(conn)?;
        Ok(conversation.contact_id)
    })
    .await?;
    let trigger = followed.then(|| ("followUp", json!({ "conversationId": id.to_string() })));
    let kind = if followed {
        "conversation:followed"
    } else {
        "conversation:unfollowed"
    };
    emit_change(db, workspace_id, id, contact_id, trigger, kind, json!({})).await;
    Ok(())
}

/// `setBotEnabledByIds` for one id: enabling clears `botResumeAt`; a pause
/// resumes after [`BOT_DISABLE_DURATION_MS`] and drops a quick-reply
/// challenge (`sqlDropQuickReplyChallenge`).
pub async fn set_bot_enabled(db: &Db, workspace_id: i64, id: i64, enabled: bool) -> Result<()> {
    let now = now_ms();
    let contact_id = tx::write(db, move |conn| {
        let q = if enabled {
            Sql::new(
                "UPDATE \"Conversation\" SET \"botEnabled\" = 1, \"botResumeAt\" = NULL, \"updatedAt\" = ?1 \
                 WHERE \"workspaceId\" = ?2 AND \"id\" = ?3",
                vec![now.into(), workspace_id.into(), id.into()],
            )
        } else {
            Sql::new(
                "UPDATE \"Conversation\" SET \"botEnabled\" = 0, \"botResumeAt\" = ?1, \"updatedAt\" = ?2, \
                 \"additionalAttributes\" = CASE WHEN json_extract(\"additionalAttributes\", '$.challenge.type') = 'quickReply' \
                 THEN json_remove(\"additionalAttributes\", '$.challenge') ELSE \"additionalAttributes\" END \
                 WHERE \"workspaceId\" = ?3 AND \"id\" = ?4",
                vec![
                    (now + BOT_DISABLE_DURATION_MS).into(),
                    now.into(),
                    workspace_id.into(),
                    id.into(),
                ],
            )
        };
        q.execute(conn)?;
        Ok(crate::conversation_list::find_in(conn, workspace_id, id)?.map(|c| c.contact_id))
    })
    .await?;
    // `enableBotState` / `disableBotState`.
    if let Some(contact_id) = contact_id {
        let (trigger, kind) = if enabled {
            (
                "conversationTransferredToBot",
                "conversation:transferred_to_bot",
            )
        } else {
            (
                "conversationTransferredToHuman",
                "conversation:transferred_to_human",
            )
        };
        let metadata = json!({ "conversationId": id.to_string() });
        emit_change(
            db,
            workspace_id,
            id,
            contact_id,
            Some((trigger, metadata)),
            kind,
            json!({}),
        )
        .await;
    }
    Ok(())
}

/// `handoffExecutorService.execute` (`handOffIfBotEnabled`): pauses the bot
/// only while it is still on (the idempotency guard), optionally reopens the
/// waiting-for-human window (SLA tracked), cancels the quick-reply follow-ups
/// and sends the transfer events. `false` = already handed off.
pub async fn hand_off_if_bot_enabled(
    db: &Db,
    workspace_id: i64,
    id: i64,
    reopen_unanswered_window: bool,
    reason: &str,
    source: &str,
    metadata: Json,
) -> Result<bool> {
    let now = now_ms();
    let contact_id = tx::write(db, move |conn| {
        let pause = |c: &chatbotx_db::rusqlite::Connection| {
            c.query_row(
                r#"UPDATE "Conversation" SET "botEnabled" = 0, "botResumeAt" = ?1, "updatedAt" = ?2,
                     "additionalAttributes" = CASE WHEN json_extract("additionalAttributes", '$.challenge.type') = 'quickReply'
                       THEN json_remove("additionalAttributes", '$.challenge') ELSE "additionalAttributes" END,
                     "awaitingHumanReplySince" = CASE WHEN ?5 = 1
                       AND "awaitingHumanReplySince" IS NULL AND "sourceId" IS NULL
                       AND "contactRepliedAt" > COALESCE("adminRepliedAt", -9223372036854775808)
                       AND "contactRepliedAt" > COALESCE("awaitingHumanReplyClosedAt", -9223372036854775808)
                       THEN "contactRepliedAt" ELSE "awaitingHumanReplySince" END
                   WHERE "workspaceId" = ?3 AND "id" = ?4 AND "botEnabled" = 1
                   RETURNING "contactId""#,
                chatbotx_db::rusqlite::params![
                    now + BOT_DISABLE_DURATION_MS,
                    now,
                    workspace_id,
                    id,
                    reopen_unanswered_window
                ],
                |r| r.get::<_, i64>(0),
            )
            .optional()
        };
        // A handoff that reopens the unanswered window starts the response SLA.
        let contact_id = if reopen_unanswered_window {
            crate::response_sla::track_window(conn, workspace_id, &[id], false, pause)?
        } else {
            pause(conn)?
        };
        if contact_id.is_some() {
            // Best effort in the original; here the same transaction.
            crate::smart_delay::cancel_quick_reply_follow_ups_in(
                conn,
                workspace_id,
                None,
                &[id],
                None,
                None,
            )?;
        }
        Ok(contact_id)
    })
    .await?;
    let Some(contact_id) = contact_id else {
        return Ok(false);
    };
    let mut metadata = match metadata {
        Json::Object(map) => map,
        _ => Default::default(),
    };
    metadata.insert("handoffReason".into(), json!(reason));
    metadata.insert(
        "triggerContext".into(),
        json!({ "triggerSource": "worker", "triggerHandler": "handoffExecutor", "triggerType": source }),
    );
    emit_change(
        db,
        workspace_id,
        id,
        contact_id,
        Some((
            "conversationTransferredToHuman",
            json!({ "conversationId": id.to_string() }),
        )),
        "conversation:transferred_to_human",
        json!({ "metadata": metadata }),
    )
    .await;
    Ok(true)
}

fn update_read_status_in(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    id: i64,
    agent_last_read_at: Option<i64>,
    now: i64,
) -> Result<Option<Envelope>> {
    let updated = Sql::new(
        "UPDATE \"Conversation\" SET \"agentLastReadAt\" = ?1, \"updatedAt\" = ?2 \
         WHERE \"workspaceId\" = ?3 AND \"id\" = ?4 RETURNING \"assignedInboxTeamId\", \"assignedUserId\"",
        vec![
            opt_int(agent_last_read_at),
            now.into(),
            workspace_id.into(),
            id.into(),
        ],
    )
    .query_map(conn, |row| {
        Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?))
    })?;
    let Some((team, user)) = updated.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(Envelope::routed(
        EventType::ConversationUpdated,
        json!({
            "conversationIds": [id.to_string()],
            "changes": { "agentLastReadAt": agent_last_read_at.map(iso) },
        }),
        route_for_conversation(
            Some(&ConversationRoute {
                assigned_inbox_team_id: sid(team),
                assigned_user_id: sid(user),
                inbox_id: None,
            }),
            None,
        ),
    )))
}

/// `updateReadStatus({ agentLastReadAt: now })`: the `conversationUpdated`
/// event, `None` when no row changed.
pub async fn mark_read(db: &Db, workspace_id: i64, id: i64) -> Result<Option<Envelope>> {
    let now = now_ms();
    tx::write(db, move |conn| {
        update_read_status_in(conn, workspace_id, id, Some(now), now)
    })
    .await
}

/// `markUnread`: `agentLastReadAt` = the second-newest incoming message
/// (one or none: `null`). 404 "Conversation not found".
pub async fn mark_unread(db: &Db, workspace_id: i64, id: i64) -> Result<Option<Envelope>> {
    let now = now_ms();
    tx::write(db, move |conn| {
        let conversation: Conversation = find_or_fail_in(conn, workspace_id, id)?;
        let anchor = conversation
            .last_activity_at
            .unwrap_or(conversation.created_at);
        let last_two = find_last_by_conversation(
            conn,
            conversation.id,
            &FindLastByConversationOptions {
                limit: Some(2),
                message_types: vec!["incoming".into()],
                since_time: get_safe_since_time(Some(SinceTimeInput::Date(anchor)), Some(YEAR_MS)),
                workspace_id: Some(workspace_id),
                ..FindLastByConversationOptions::default()
            },
        )?;
        let agent_last_read_at = if last_two.len() >= 2 {
            Some(last_two[1].message.created_at)
        } else {
            None
        };
        update_read_status_in(conn, workspace_id, id, agent_last_read_at, now)
    })
    .await
}
