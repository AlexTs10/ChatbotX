//! The conversation steps of `integration/handlers/step-handlers.ts`.
//!
//! ponytail: the flow-trigger / analytics events of these services
//! (`conversationAssigned` trigger, `transferred_to_human`, ...), the
//! response-SLA window tracking and the thread release on archive come with
//! the event workers (Q.W2) and the SLA unit; the realtime envelopes and the
//! assignment notification job are sent here.

use chatbotx_business::conversation_actions::{self, ActionJob, Assigned};
use chatbotx_core::flow_config::{is_bulk_outbound_metadata, is_proactive_outbound_metadata};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobError, JobOptions};
use chatbotx_rt::Envelope;
use serde_json::{Value, json};

use super::contact::biz_fail;
use super::working_hours::list_on_shift_user_ids;
use super::{StepCtx, step_error};
use crate::channel::OutgoingContact;
use crate::db_fail;
use crate::flow::StepStatus;

/// The conversation steps; `None` = not a conversation step.
pub(super) async fn run(
    ctx: &StepCtx<'_, '_>,
    step_type: &str,
) -> Result<Option<StepStatus>, JobError> {
    let db = &ctx.run.deps.db;
    let (ws, id) = (ctx.workspace_id(), ctx.run.conversation.id);
    let status = match step_type {
        "archiveConversation" | "unarchiveConversation" => {
            conversation_actions::set_archived(
                db,
                Some(&*ctx.run.deps.jobs),
                ws,
                id,
                step_type == "archiveConversation",
            )
            .await
            .map_err(biz_fail)?;
            StepStatus::Success
        }
        "assignConversation" => {
            assign_or_skip(ctx).await?;
            StepStatus::Success
        }
        "autoAssignConversation" => auto_assign(ctx).await?,
        "unassignConversation" => {
            let assigned = conversation_actions::assign_one(db, ws, id, None)
                .await
                .map_err(biz_fail)?;
            publish_assigned(ctx, assigned, silent(ctx)).await?;
            StepStatus::Success
        }
        "markConversationAsUnread" => {
            let event = conversation_actions::mark_unread(db, ws, id)
                .await
                .map_err(biz_fail)?;
            if !silent(ctx) {
                publish(ctx, event).await;
            }
            StepStatus::Success
        }
        "markConversationAsRead" => {
            let event = conversation_actions::mark_read(db, ws, id)
                .await
                .map_err(biz_fail)?;
            publish(ctx, event).await;
            // Comment threads (`sourceId` = post id) are not a DM.
            if ctx.run.conversation.source_id.is_none() {
                send_read_receipt(ctx).await;
            }
            StepStatus::Success
        }
        "followConversation" | "unfollowConversation" => {
            conversation_actions::set_followed(db, ws, id, step_type == "followConversation")
                .await
                .map_err(biz_fail)?;
            StepStatus::Success
        }
        "disableBot" => {
            disable_bot(ctx).await?;
            StepStatus::Success
        }
        "enableBot" => {
            conversation_actions::set_bot_enabled(db, ws, id, true)
                .await
                .map_err(biz_fail)?;
            StepStatus::Success
        }
        "typing" => {
            crate::chat::send_typing_to_channel(
                ctx.run.deps,
                &json!({
                    "conversationId": id.to_string(),
                    "contactInboxId": ctx.run.contact_inbox.id.to_string(),
                    "typing": true,
                    "seconds": ctx.step.get("seconds"),
                }),
            )
            .await?;
            StepStatus::Success
        }
        "threadControl" => thread_control(ctx).await,
        _ => return Ok(None),
    };
    Ok(Some(status))
}

/// `isBulkOutboundMetadata(metadata, isBulkBroadcast)`: no realtime noise.
fn silent(ctx: &StepCtx<'_, '_>) -> bool {
    ctx.run
        .metadata()
        .is_some_and(|m| is_bulk_outbound_metadata(m, ctx.run.is_bulk_broadcast.unwrap_or(false)))
}

async fn publish(ctx: &StepCtx<'_, '_>, event: Option<Envelope>) {
    if let (Some(realtime), Some(event)) = (&ctx.run.deps.realtime, event) {
        realtime
            .queue_workspace_event(&ctx.workspace_id().to_string(), event)
            .await;
    }
}

async fn publish_assigned(
    ctx: &StepCtx<'_, '_>,
    assigned: Assigned,
    silent: bool,
) -> Result<(), JobError> {
    if !silent {
        publish(ctx, assigned.event).await;
    }
    if let Some(ActionJob {
        queue,
        name,
        data,
        job_id,
    }) = assigned.job
    {
        ctx.run
            .deps
            .jobs
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
            .map_err(db_fail)?;
    }
    Ok(())
}

/// The `u_<userId>` / `t_<teamId>` target, `None` = invalid.
enum Target {
    User(i64),
    Team(i64),
}

fn target_of(id: &str) -> Option<Target> {
    if let Some(user) = id.strip_prefix("u_") {
        return user.parse().ok().map(Target::User);
    }
    id.strip_prefix("t_")
        .and_then(|team| team.parse().ok())
        .map(Target::Team)
}

/// `assignOneOrSkip`: an invalid target or an off-shift agent is a no-op.
async fn assign_or_skip(ctx: &StepCtx<'_, '_>) -> Result<(), JobError> {
    let deps = &ctx.run.deps;
    let ws = ctx.workspace_id();
    let assigned_id = ctx.str("assignedId").unwrap_or("").to_owned();
    if let Some(Target::User(user)) = target_of(&assigned_id) {
        let on_shift = list_on_shift_user_ids(&deps.db, ws, vec![user], now_ms())
            .await
            .map_err(db_fail)?;
        if !on_shift.contains(&user) {
            tracing::debug!(
                conversation_id = ctx.run.conversation.id,
                "Assignment skipped: the target agent is off shift"
            );
            return Ok(());
        }
    }
    match conversation_actions::assign_one(&deps.db, ws, ctx.run.conversation.id, Some(assigned_id))
        .await
    {
        Ok(assigned) => publish_assigned(ctx, assigned, silent(ctx)).await,
        // `resolveAssignmentTarget(.., "ignore")`.
        Err(err) if matches!(err.code(), Some("invalidAssignee" | "notFound")) => Ok(()),
        Err(err) => Err(biz_fail(err)),
    }
}

/// `stepAutoAssignConversation`: the least-loaded of the on-shift users and
/// the teams (users first on a tie).
async fn auto_assign(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let deps = &ctx.run.deps;
    let ws = ctx.workspace_id();
    let ids: Vec<String> = ctx
        .step
        .get("assignedIds")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Ok(step_error(ctx, "No assignees configured"));
    }
    let (mut users, mut teams) = (Vec::new(), Vec::new());
    for id in &ids {
        match target_of(id) {
            Some(Target::User(u)) => users.push(u),
            Some(Target::Team(t)) => teams.push(t),
            None => {}
        }
    }
    let since = match ctx.str("rule") {
        Some("LAST_HOUR") => Some(1),
        Some("LAST_8HOURS") => Some(8),
        Some("LAST_24HOURS") => Some(24),
        _ => None,
    }
    .map(|hours: i64| now_ms() - hours * 3_600_000);
    let (users, teams) = deps
        .db
        .read(move |conn| {
            let exists = |sql: &str, id: i64| -> Result<bool, chatbotx_db::DbError> {
                Ok(conn
                    .query_row(sql, params![ws, id], |_| Ok(()))
                    .optional()?
                    .is_some())
            };
            let mut live_users = Vec::new();
            for u in users {
                if !live_users.contains(&u)
                    && exists(
                        r#"SELECT 1 FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                        u,
                    )?
                {
                    live_users.push(u);
                }
            }
            let mut live_teams = Vec::new();
            for t in teams {
                if !live_teams.contains(&t)
                    && exists(
                        r#"SELECT 1 FROM "InboxTeam" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                        t,
                    )?
                {
                    live_teams.push(t);
                }
            }
            Ok((live_users, live_teams))
        })
        .await
        .map_err(db_fail)?;
    let users = prefer_on_shift(ctx, users, teams.is_empty()).await?;
    // (key, user, team, count) in insertion order: users, then teams.
    let mut allocation: Vec<(Option<i64>, Option<i64>, i64)> = users
        .iter()
        .map(|u| (Some(*u), None, 0))
        .chain(teams.iter().map(|t| (None, Some(*t), 0)))
        .collect();
    if allocation.is_empty() {
        return Ok(step_error(ctx, "No eligible agents found for allocation"));
    }
    let (count_users, count_teams) = (users.clone(), teams.clone());
    let counts: Vec<(Option<i64>, Option<i64>, i64)> = deps
        .db
        .read(move |conn| {
            let marks = |n: usize| vec!["?"; n.max(1)].join(", ");
            let mut sql = format!(
                r#"SELECT "assignedUserId", "assignedInboxTeamId", count("id") FROM "Conversation"
                   WHERE "workspaceId" = ? AND ("assignedUserId" IN ({}) OR "assignedInboxTeamId" IN ({}))"#,
                marks(count_users.len()),
                marks(count_teams.len())
            );
            let mut values: Vec<chatbotx_db::rusqlite::types::Value> = vec![ws.into()];
            values.extend(pad(&count_users));
            values.extend(pad(&count_teams));
            if let Some(since) = since {
                sql.push_str(r#" AND "createdAt" >= ?"#);
                values.push(since.into());
            }
            sql.push_str(r#" GROUP BY "assignedUserId", "assignedInboxTeamId""#);
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(chatbotx_db::rusqlite::params_from_iter(values), |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(db_fail)?;
    for (user, team, count) in counts {
        // A row counts for its user, and (when it has one) for its team.
        if let Some(slot) = allocation
            .iter_mut()
            .find(|a| user.is_some() && a.0 == user)
        {
            slot.2 = count;
        }
        if let Some(slot) = allocation
            .iter_mut()
            .find(|a| team.is_some() && a.1 == team)
        {
            slot.2 = count;
        }
    }
    let (user, team, _) = allocation
        .iter()
        .fold(
            None::<&(Option<i64>, Option<i64>, i64)>,
            |best, a| match best {
                Some(b) if b.2 <= a.2 => Some(b),
                _ => Some(a),
            },
        )
        .copied()
        .unwrap_or_default();
    let target = match (user, team) {
        (Some(u), _) => format!("u_{u}"),
        (_, Some(t)) => format!("t_{t}"),
        _ => return Ok(step_error(ctx, "No eligible agents found for allocation")),
    };
    let assigned =
        conversation_actions::assign_one(&deps.db, ws, ctx.run.conversation.id, Some(target))
            .await
            .map_err(biz_fail)?;
    publish_assigned(ctx, assigned, false).await?;
    Ok(StepStatus::Success)
}

/// An `IN ()` needs one value: `NULL` matches nothing.
fn pad(ids: &[i64]) -> Vec<chatbotx_db::rusqlite::types::Value> {
    if ids.is_empty() {
        return vec![chatbotx_db::rusqlite::types::Value::Null];
    }
    ids.iter().map(|id| (*id).into()).collect()
}

/// `preferOnShiftUsers`.
async fn prefer_on_shift(
    ctx: &StepCtx<'_, '_>,
    users: Vec<i64>,
    fall_back_to_all_users: bool,
) -> Result<Vec<i64>, JobError> {
    if users.is_empty() {
        return Ok(users);
    }
    let on_shift = list_on_shift_user_ids(
        &ctx.run.deps.db,
        ctx.workspace_id(),
        users.clone(),
        now_ms(),
    )
    .await
    .map_err(db_fail)?;
    let on: Vec<i64> = users
        .iter()
        .copied()
        .filter(|u| on_shift.contains(u))
        .collect();
    if on.len() == users.len() || (on.is_empty() && fall_back_to_all_users) {
        return Ok(users);
    }
    tracing::debug!(
        conversation_id = ctx.run.conversation.id,
        skipped_off_shift = users.len() - on.len(),
        "Auto assign skipped off-shift agents"
    );
    Ok(on)
}

/// `stepDisableBot`: only a run known to react to the contact's message
/// reopens the waiting-for-human window (`reopenIfUnansweredSince`); a pause
/// cancels the conversation's quick-reply follow-ups (best effort).
async fn disable_bot(ctx: &StepCtx<'_, '_>) -> Result<(), JobError> {
    let run = ctx.run;
    let (ws, id) = (ctx.workspace_id(), run.conversation.id);
    let reopen = run.data.get("startedByContactMessage") == Some(&json!(true))
        && run.is_bulk_broadcast != Some(true)
        && !run.metadata().is_some_and(is_proactive_outbound_metadata);
    conversation_actions::set_bot_enabled(&run.deps.db, ws, id, false)
        .await
        .map_err(biz_fail)?;
    let result = run
        .deps
        .db
        .write(move |tx| {
            if reopen {
                // Only a pause that reopens the waiting window can start an SLA.
                chatbotx_business::response_sla::track_window(tx, ws, &[id], false, |c| {
                    c.execute(
                        r#"UPDATE "Conversation" SET "awaitingHumanReplySince" = CASE
                             WHEN "awaitingHumanReplySince" IS NULL AND "sourceId" IS NULL
                              AND "contactRepliedAt" > COALESCE("adminRepliedAt", -9223372036854775808)
                              AND "contactRepliedAt" > COALESCE("awaitingHumanReplyClosedAt", -9223372036854775808)
                             THEN "contactRepliedAt" ELSE "awaitingHumanReplySince" END
                           WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                        params![ws, id],
                    )
                })?;
            }
            tx.execute(
                r#"UPDATE "ContactOnSmartDelay" SET "status" = 'canceled'
                   WHERE "workspaceId" = ?1 AND "conversationId" = ?2 AND "type" = 'quickReplyFollowUp'
                     AND "status" NOT IN ('completed', 'failed', 'canceled')"#,
                params![ws, id],
            )?;
            Ok(())
        })
        .await;
    if let Err(err) = result {
        tracing::warn!(err = %err, conversation_id = id, "quick reply follow-up cancel on bot pause failed");
    }
    Ok(())
}

/// `sendReadReceipt`: Messenger, Instagram and WhatsApp only; best effort.
async fn send_read_receipt(ctx: &StepCtx<'_, '_>) {
    let run = ctx.run;
    let contact_inbox = run.contact_inbox.clone();
    let channel = contact_inbox.channel.clone();
    if !matches!(channel.as_str(), "messenger" | "instagram" | "whatsapp") {
        return;
    }
    let message_source_id = if channel == "whatsapp" {
        let (conversation_id, contact_inbox_id) = (run.conversation.id, contact_inbox.id);
        let found = run
            .deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "sourceId" FROM "Message" WHERE "conversationId" = ?1 AND "contactInboxId" = ?2
                           AND "messageType" = 'incoming' AND "sourceId" IS NOT NULL
                           ORDER BY "createdAt" DESC, "id" DESC LIMIT 1"#,
                        params![conversation_id, contact_inbox_id],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?)
            })
            .await;
        match found {
            Ok(Some(id)) => Some(id),
            Ok(None) => {
                tracing::debug!(
                    conversation_id = run.conversation.id,
                    "stepMarkConversationAsRead: no incoming WhatsApp message to mark read"
                );
                return;
            }
            Err(err) => {
                tracing::warn!(err = %err, channel = %channel, "stepMarkConversationAsRead: channel receipt failed");
                return;
            }
        }
    } else {
        None
    };
    let deps = &run.deps;
    let sent = async {
        let (handler, channel_ctx) = deps
            .channels
            .resolve(
                &deps.db,
                deps.realtime.as_ref(),
                ctx.workspace_id(),
                &contact_inbox,
            )
            .await?;
        handler
            .agent_mark_as_read(
                &channel_ctx,
                OutgoingContact {
                    contact_inbox: contact_inbox.clone(),
                    source_conversation_id: run.conversation.source_id.clone(),
                },
                message_source_id,
            )
            .await
    }
    .await;
    if let Err(err) = sent {
        tracing::warn!(err = ?err, channel = %channel, conversation_id = run.conversation.id, "stepMarkConversationAsRead: channel receipt failed");
    }
}

/// `stepThreadControl`: the channel thread-control actions are the
/// WhatsApp / Messenger units (Q.D12, Q.I1); until then every channel
/// answers "not supported", which the original routes to the Error path.
async fn thread_control(ctx: &StepCtx<'_, '_>) -> StepStatus {
    let is_pass = ctx.str("action") == Some("pass");
    if is_pass
        && let Some(note) = ctx.str("metadata").filter(|n| !n.is_empty())
        && let Err(err) = ctx.resolve_variables(&json!({ "text": note }), false).await
    {
        tracing::warn!(err = %err, contact_inbox_id = ctx.run.contact_inbox.id, "Thread control step failed");
        return step_error(ctx, &err);
    }
    // ponytail: `requestThreadControlAction` (Q.D12).
    let message = format!(
        "Thread control is not supported for channel {}",
        ctx.run.contact_inbox.channel
    );
    tracing::warn!(
        contact_inbox_id = ctx.run.contact_inbox.id,
        action = ctx.str("action"),
        "Thread control step failed"
    );
    step_error(ctx, &message)
}
