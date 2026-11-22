//! The worker part of `broadcast/service.ts`: what the `schedule` jobs
//! (`prepareBroadcast`, `sendBroadcast`, `reconcileBroadcasts`,
//! `finalizeBroadcasts`, `purgeDraftBroadcastRecipients`) read and write.

use std::collections::HashMap;

use chatbotx_db::Db;
use chatbotx_db::repo::broadcast_purge::{PurgeInput, PurgeStopReason, purge_broadcast_recipients};
use chatbotx_db::repo::conversation::insert_dm_conversations_if_absent;
use chatbotx_db::repo::models::{Broadcast, BroadcastTarget, ContactInbox, Conversation};
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::Value;

use crate::errors::Result;
use crate::tx;

const DRAFT_PURGE_CHUNK_SIZE: i64 = 1000;
const DRAFT_PURGE_INTER_CHUNK_DELAY_MS: u64 = 50;
const DRAFT_PURGE_MAX_RUN_DURATION_MS: i64 = 30_000;

/// A broadcast row as the partials read it (`to_json` plus its `targets`).
#[derive(Debug, Clone)]
pub struct BroadcastForSend {
    pub row: Broadcast,
    pub json: Value,
}

pub(crate) fn with_targets(conn: &Connection, row: Broadcast) -> Result<BroadcastForSend> {
    let targets = repo::find_many::<BroadcastTarget>(
        conn,
        r#"WHERE "broadcastId" = ?1 ORDER BY "inboxId" ASC"#,
        &[&row.id],
    )?;
    let mut json = row.to_json();
    json["targets"] = Value::Array(targets.iter().map(BroadcastTarget::to_json).collect());
    Ok(BroadcastForSend { row, json })
}

fn ids(
    conn: &Connection,
    sql: &str,
    args: &[&dyn chatbotx_db::rusqlite::ToSql],
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(args, |row| row.get(0))?
        .collect::<chatbotx_db::rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

/// `listDueScheduled`: `scheduled` broadcasts due at or before `due_at`.
pub async fn list_due_scheduled(db: &Db, due_at: i64) -> Result<Vec<i64>> {
    tx::read(db, move |conn| {
        ids(
            conn,
            r#"SELECT "id" FROM "Broadcast" WHERE "schedulesAt" <= ?1 AND "status" = 'scheduled' AND "deletedAt" IS NULL"#,
            &[&due_at],
        )
    })
    .await
}

/// `listSendingAwaitingHandoff`.
pub async fn list_sending_awaiting_handoff(db: &Db) -> Result<Vec<i64>> {
    tx::read(db, |conn| {
        ids(
            conn,
            r#"SELECT "id" FROM "Broadcast" WHERE "status" = 'sending' AND "handoffCompletedAt" IS NULL AND "deletedAt" IS NULL"#,
            &[],
        )
    })
    .await
}

fn find_live(conn: &Connection, id: i64, status: &str) -> Result<Option<BroadcastForSend>> {
    repo::find_one::<Broadcast>(
        conn,
        r#""id" = ?1 AND "status" = ?2 AND "deletedAt" IS NULL"#,
        &[&id, &status],
    )?
    .map(|row| with_targets(conn, row))
    .transpose()
}

/// `findScheduledForPrepare`.
pub async fn find_scheduled_for_prepare(db: &Db, id: i64) -> Result<Option<BroadcastForSend>> {
    tx::read(db, move |conn| find_live(conn, id, "scheduled")).await
}

/// `listSendableById`: the `sending` row (zero or one).
pub async fn find_sendable(db: &Db, id: i64) -> Result<Option<BroadcastForSend>> {
    tx::read(db, move |conn| find_live(conn, id, "sending")).await
}

/// `isStillPreparing`: still `scheduled` in the same dispatch epoch.
pub async fn is_still_preparing(db: &Db, id: i64, resume_count: i64) -> Result<bool> {
    tx::read(db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT 1 FROM "Broadcast" WHERE "id" = ?1 AND "status" = 'scheduled' AND "resumeCount" = ?2 AND "deletedAt" IS NULL"#,
                params![id, resume_count],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    })
    .await
}

/// `resolveTemplateIntegrationMessengerId`: scoped to the workspace by the
/// template's page.
pub async fn resolve_template_integration_messenger_id(
    db: &Db,
    workspace_id: i64,
    template_id: i64,
) -> Result<Option<i64>> {
    tx::read(db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT t."integrationMessengerId" FROM "MessengerMessageTemplate" t
                   JOIN "IntegrationMessenger" i ON i."id" = t."integrationMessengerId"
                   WHERE t."id" = ?1 AND i."workspaceId" = ?2"#,
                params![template_id, workspace_id],
                |row| row.get(0),
            )
            .optional()?)
    })
    .await
}

/// `conversationService.findOrCreateDMByContactIds`: the DM of each
/// contact, opened when missing.
pub async fn find_or_create_dms(
    db: &Db,
    workspace_id: i64,
    contact_ids: Vec<i64>,
) -> Result<Vec<Conversation>> {
    tx::write(db, move |tx| {
        let mut unique = contact_ids;
        unique.sort_unstable();
        unique.dedup();
        let find = |conn: &Connection| -> Result<Vec<Conversation>> {
            Ok(repo::find_many::<Conversation>(
                conn,
                r#"WHERE "workspaceId" = ?1 AND "sourceId" IS NULL AND "contactId" IN (SELECT value FROM json_each(?2))"#,
                &[&workspace_id, &repo::id_list(&unique)],
            )?)
        };
        if unique.is_empty() {
            return Ok(Vec::new());
        }
        let existing = find(tx)?;
        let missing: Vec<i64> = unique
            .iter()
            .copied()
            .filter(|id| !existing.iter().any(|c| c.contact_id == *id))
            .collect();
        if missing.is_empty() {
            return Ok(existing);
        }
        insert_dm_conversations_if_absent(tx, workspace_id, &missing)?;
        // One re-read covers the rows just inserted and any a concurrent writer won.
        find(tx)
    })
    .await
}

/// A `ContactOnBroadcast` row to write.
#[derive(Debug, Clone, Copy)]
pub struct NewRecipient {
    pub contact_id: i64,
    pub contact_inbox_id: i64,
    pub conversation_id: i64,
}

/// `insertRecipients`: rows actually written (one recipient per contact).
pub async fn insert_recipients(
    db: &Db,
    broadcast_id: i64,
    recipients: Vec<NewRecipient>,
) -> Result<i64> {
    if recipients.is_empty() {
        return Ok(0);
    }
    tx::write(db, move |tx| {
        let mut stmt = tx.prepare_cached(
            r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId")
               VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING"#,
        )?;
        let mut written = 0;
        for r in &recipients {
            written += stmt.execute(params![broadcast_id, r.contact_id, r.contact_inbox_id, r.conversation_id])?;
        }
        Ok(written as i64)
    })
    .await
}

/// `countRecipients`.
pub async fn count_recipients(db: &Db, broadcast_id: i64) -> Result<i64> {
    tx::read(db, move |conn| {
        Ok(chatbotx_db::repo::broadcast::count_audience(
            conn,
            broadcast_id,
        )?)
    })
    .await
}

/// `promoteAfterPrepare`: the CAS out of `scheduled`, pinned to the epoch.
pub async fn promote_after_prepare(
    db: &Db,
    broadcast_id: i64,
    status: &'static str,
    contact_count: i64,
    promotion_epoch: i64,
) -> Result<bool> {
    tx::write(db, move |tx| {
        Ok(tx.execute(
            r#"UPDATE "Broadcast" SET "status" = ?2, "contactCount" = ?3, "updatedAt" = ?5
               WHERE "id" = ?1 AND "status" = 'scheduled' AND "deletedAt" IS NULL AND "resumeCount" = ?4"#,
            params![broadcast_id, status, contact_count, promotion_epoch, now_ms()],
        )? > 0)
    })
    .await
}

/// A pending recipient with its `conversation` and `contactInbox` as JSON
/// (`None` = the row is gone).
#[derive(Debug, Clone)]
pub struct PendingRecipient {
    pub contact_id: i64,
    pub contact_inbox_id: i64,
    pub conversation_id: i64,
    pub conversation: Option<Value>,
    pub contact_inbox: Option<ContactInbox>,
}

/// `listPendingRecipients`: unsent, unfailed rows in `contactInboxId` order.
pub async fn list_pending_recipients(
    db: &Db,
    broadcast_id: i64,
    limit: i64,
) -> Result<Vec<PendingRecipient>> {
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare_cached(
            r#"SELECT "contactId", "contactInboxId", "conversationId" FROM "ContactOnBroadcast"
               WHERE "broadcastId" = ?1 AND "sent" = 0 AND "failedAt" IS NULL
               ORDER BY "contactInboxId" ASC LIMIT ?2"#,
        )?;
        let rows = stmt
            .query_map(params![broadcast_id, limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()?;
        let conversation_ids: Vec<i64> = rows.iter().map(|r| r.2).collect();
        let inbox_ids: Vec<i64> = rows.iter().map(|r| r.1).collect();
        let conversations: HashMap<i64, Value> = repo::find_many::<Conversation>(
            conn,
            r#"WHERE "id" IN (SELECT value FROM json_each(?1))"#,
            &[&repo::id_list(&conversation_ids)],
        )?
        .into_iter()
        .map(|c| (c.id, c.to_json()))
        .collect();
        let mut contact_inboxes: HashMap<i64, ContactInbox> = repo::find_many::<ContactInbox>(
            conn,
            r#"WHERE "id" IN (SELECT value FROM json_each(?1))"#,
            &[&repo::id_list(&inbox_ids)],
        )?
        .into_iter()
        .map(|c| (c.id, c))
        .collect();
        Ok(rows
            .into_iter()
            .map(
                |(contact_id, contact_inbox_id, conversation_id)| PendingRecipient {
                    contact_id,
                    contact_inbox_id,
                    conversation_id,
                    conversation: conversations.get(&conversation_id).cloned(),
                    contact_inbox: contact_inboxes.remove(&contact_inbox_id),
                },
            )
            .collect())
    })
    .await
}

/// `markContactFailed`.
pub async fn mark_contact_failed(
    db: &Db,
    broadcast_id: i64,
    contact_id: i64,
    reason: String,
) -> Result<()> {
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "ContactOnBroadcast" SET "failedAt" = ?3, "errorContent" = ?4 WHERE "broadcastId" = ?1 AND "contactId" = ?2"#,
            params![broadcast_id, contact_id, now_ms(), reason],
        )?;
        Ok(())
    })
    .await
}

/// `markContactSentIfSending`: only while the broadcast is still `sending`.
pub async fn mark_contact_sent_if_sending(
    db: &Db,
    broadcast_id: i64,
    contact_id: i64,
) -> Result<()> {
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "ContactOnBroadcast" SET "sent" = 1 WHERE "broadcastId" = ?1 AND "contactId" = ?2
               AND EXISTS (SELECT 1 FROM "Broadcast" b WHERE b."id" = ?1 AND b."status" = 'sending')"#,
            params![broadcast_id, contact_id],
        )?;
        Ok(())
    })
    .await
}

/// `markHandoffCompleted`: idempotent.
pub async fn mark_handoff_completed(db: &Db, broadcast_id: i64) -> Result<bool> {
    tx::write(db, move |tx| {
        Ok(tx.execute(
            r#"UPDATE "Broadcast" SET "handoffCompletedAt" = ?2 WHERE "id" = ?1 AND "status" = 'sending' AND "handoffCompletedAt" IS NULL"#,
            params![broadcast_id, now_ms()],
        )? > 0)
    })
    .await
}

/// `BroadcastAwaitingFinalization`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwaitingFinalization {
    pub id: i64,
    pub workspace_id: i64,
    pub contact_count: Option<i64>,
    pub handoff_completed_at: i64,
}

/// `listAwaitingFinalization`: `sending` rows whose hand-off completed.
pub async fn list_awaiting_finalization(db: &Db) -> Result<Vec<AwaitingFinalization>> {
    tx::read(db, |conn| {
        let mut stmt = conn.prepare_cached(
            r#"SELECT "id", "workspaceId", "contactCount", "handoffCompletedAt" FROM "Broadcast"
               WHERE "status" = 'sending' AND "handoffCompletedAt" IS NOT NULL"#,
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AwaitingFinalization {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    contact_count: row.get(2)?,
                    handoff_completed_at: row.get(3)?,
                })
            })?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

/// `countRecipientOutcomes`: `(completed, failed)`.
pub async fn count_recipient_outcomes(db: &Db, broadcast_id: i64) -> Result<(i64, i64)> {
    tx::read(db, move |conn| {
        Ok(conn.query_row(
            r#"SELECT count(*) FILTER (WHERE "deliveredAt" IS NOT NULL OR "failedAt" IS NOT NULL),
                      count(*) FILTER (WHERE "failedAt" IS NOT NULL)
               FROM "ContactOnBroadcast" WHERE "broadcastId" = ?1"#,
            [broadcast_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    })
    .await
}

/// `completeSending`: the terminal transition; a lost race is a no-op.
pub async fn complete_sending(db: &Db, broadcast_id: i64, status: &'static str) -> Result<bool> {
    tx::write(db, move |tx| {
        Ok(tx.execute(
            r#"UPDATE "Broadcast" SET "status" = ?2, "updatedAt" = ?3
               WHERE "id" = ?1 AND "status" = 'sending' AND "handoffCompletedAt" IS NOT NULL"#,
            params![broadcast_id, status, now_ms()],
        )? > 0)
    })
    .await
}

/// `purgeDraftRecipients`: one bounded pass; `true` once drained.
pub async fn purge_draft_recipients(db: &Db, broadcast_id: i64) -> Result<bool> {
    let outcome = purge_broadcast_recipients(
        db,
        &PurgeInput {
            broadcast_id,
            chunk_size: DRAFT_PURGE_CHUNK_SIZE,
            inter_chunk_delay_ms: DRAFT_PURGE_INTER_CHUNK_DELAY_MS,
            max_run_duration_ms: DRAFT_PURGE_MAX_RUN_DURATION_MS,
            max_chunks: None,
            only_while_draft: true,
        },
    )
    .await?;
    Ok(outcome.stop_reason == PurgeStopReason::Drained)
}

/// `isBlockedWorkspace` (`withBlockedOwnerGuard`): the workspace is gone or
/// scheduled for deletion.
// ponytail: the cloud-only `ownerBlocked` reason (owner quota access state)
// waits for the user-quota port; self-hosted never produces it.
pub async fn is_blocked_workspace(db: &Db, workspace_id: i64) -> Result<bool> {
    tx::read(db, move |conn| {
        let row: Option<Option<i64>> = conn
            .query_row(
                r#"SELECT "scheduledDeletionAt" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |row| row.get(0),
            )
            .optional()?;
        let blocked = !matches!(row, Some(None));
        if blocked {
            tracing::info!(workspace_id, "Skipping workspace job for frozen workspace");
        }
        Ok(blocked)
    })
    .await
}
