//! The broadcast jobs of `apps/worker/src/schedule/handlers`: enqueue,
//! prepare, send (`processBroadcastContacts`), reconcile, finalize, purge
//! and the draft recipients purge.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use chatbotx_business::audit::{AuditRecordInput, AuditService};
use chatbotx_business::broadcast::audience::{AudienceInput, audience_walk};
use chatbotx_business::broadcast::send::{self, BroadcastForSend, NewRecipient, PendingRecipient};
use chatbotx_core::flow_config::BROADCAST_PAYLOAD_TYPE;
use chatbotx_db::repo::broadcast_purge::{
    PurgeInput, PurgeStopReason, hard_delete_broadcast, has_broadcast_recipients,
    list_purgeable_broadcasts, purge_broadcast_recipients,
};
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::partials::broadcast::{
    BROADCAST_DISPATCH_WINDOW_MS, broadcast_sends_flow, broadcast_sends_template,
    has_broadcast_send_for_inbox, is_broadcast_outcome_grace_elapsed,
    resolve_broadcast_audience_range, resolve_broadcast_flow_send,
    resolve_broadcast_send_rate_per_minute, resolve_broadcast_target_inbox_ids,
    resolve_broadcast_template_send, resolve_broadcast_terminal_status, uses_broadcast_targets,
};
use chatbotx_db::repo::queries::contact_filter::FilterCriteria;
use chatbotx_jobs::deferral::deferrals_so_far;
use chatbotx_jobs::queues::broadcast::{
    PURGE_BROADCAST_CONCURRENCY, broadcast_prepare_job_id, broadcast_send_job_id,
};
use chatbotx_jobs::queues::integration::BROADCAST_SEND_PRIORITY;
use chatbotx_jobs::{JobCtx, JobError, JobOptions, NewJob, QueueName, Retention};
use chatbotx_kv::cas::CasStore as _;
use chatbotx_kv::durable::DurableKv;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use crate::{Deps, db_fail};

const ENQUEUE_BULK_SIZE: usize = 500;
const BROADCAST_SEND_JOB_RETENTION_SECONDS: u64 = 3600;
const NO_TEMPLATE_FOR_PAGE_REASON: &str = "no template selected for the contact's page";
const NO_FLOW_FOR_PAGE_REASON: &str = "no flow selected for the contact's page";
const NO_SEND_FOR_PAGE_REASON: &str = "no flow or template selected for the contact's page";
const MAX_MISSING_CONTACTS_FOR_THRESHOLD: i64 = 100;
const MISSING_RATE_THRESHOLD: f64 = 0.01;
const PURGE_CANDIDATE_LIMIT: i64 = 50;
const PURGE_CHUNK_SIZE: i64 = 1000;
const PURGE_INTER_CHUNK_DELAY_MS: u64 = 100;
const PURGE_MAX_RUN_DURATION_MS: i64 = 4 * 60 * 1000;
/// `PURGE_DRAFT_RECIPIENTS_POLICY`.
pub const PURGE_DRAFT_RETRY_DELAY_MS: i64 = 5000;
pub const PURGE_DRAFT_MAX_PASSES: u32 = 120;

fn biz(error: chatbotx_business::errors::BusinessError) -> JobError {
    db_fail(error)
}

fn queue_err(error: chatbotx_jobs::JobsError) -> JobError {
    db_fail(error)
}

/// `removeOnComplete: true, removeOnFail: true` under a fixed job id.
fn once(job_id: String) -> JobOptions {
    JobOptions {
        job_id: Some(job_id),
        remove_on_complete: Some(Retention::Remove),
        remove_on_fail: Some(Retention::Remove),
        ..JobOptions::default()
    }
}

/// `startOfMinute(now)`.
fn start_of_minute(now: i64) -> i64 {
    now - now.rem_euclid(60_000)
}

// ponytail: `distributedLock.runExclusive` is a process-local mutex (one
// server process); a second process needs the kv locks.
fn cron_lock(deps: &Deps, key: &'static str) -> &'static AsyncMutex<()> {
    type Locks = Mutex<HashMap<(usize, &'static str), &'static AsyncMutex<()>>>;
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    // One lock per database (a deployment), so parallel tests do not share it.
    let db = Arc::as_ptr(&deps.db) as usize;
    let mut map = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.entry((db, key))
        .or_insert_with(|| Box::leak(Box::new(AsyncMutex::new(()))))
}

/// `claimDispatchWindow`: SET NX PX (`BROADCAST_DISPATCH_WINDOW_MS`) in the
/// durable kv; a store failure fails open (logged), as the original.
async fn claim_dispatch_window(deps: &Deps, broadcast_id: i64) -> bool {
    let store = DurableKv::new(deps.db.clone(), Arc::new(chatbotx_kv::SystemClock));
    let key = format!("broadcast:{broadcast_id}:dispatch-window");
    match store
        .set_if_absent(&key, &json!(true), BROADCAST_DISPATCH_WINDOW_MS as u64)
        .await
    {
        Ok(claimed) => claimed,
        Err(err) => {
            tracing::error!(err = %err, broadcast_id, "Failed to claim broadcast dispatch window lease");
            true
        }
    }
}

fn send_broadcast_job(broadcast_id: i64) -> (Value, JobOptions) {
    let id = broadcast_id.to_string();
    (
        json!({ "type": "sendBroadcast", "data": { "broadcastId": id } }),
        JobOptions {
            attempts: Some(1),
            ..once(broadcast_send_job_id(&id))
        },
    )
}

/// `enqueueBroadcast`: one `prepareBroadcast` job per due broadcast.
pub async fn enqueue_broadcast(deps: &Deps) -> Result<Value, JobError> {
    let due = send::list_due_scheduled(&deps.db, start_of_minute(now_ms()))
        .await
        .map_err(biz)?;
    for batch in due.chunks(ENQUEUE_BULK_SIZE) {
        let jobs = batch
            .iter()
            .map(|id| {
                let id = id.to_string();
                NewJob {
                    name: "prepareBroadcast".into(),
                    data: json!({ "type": "prepareBroadcast", "data": { "broadcastId": id } }),
                    opts: once(broadcast_prepare_job_id(&id)),
                }
            })
            .collect();
        deps.jobs
            .add_bulk(QueueName::Schedule, jobs)
            .await
            .map_err(queue_err)?;
    }
    Ok(json!({ "scanned": due.len(), "enqueued": due.len() }))
}

/// `prepareBroadcast`: writes the audience as recipients, promotes the row
/// out of `scheduled` (pinned to the dispatch epoch) and queues the send.
pub async fn prepare_broadcast(deps: &Deps, broadcast_id: i64) -> Result<Value, JobError> {
    let db = &deps.db;
    let Some(BroadcastForSend {
        row,
        json: broadcast,
    }) = send::find_scheduled_for_prepare(db, broadcast_id)
        .await
        .map_err(biz)?
    else {
        tracing::warn!(broadcast_id, "Broadcast not found or not scheduled");
        return Ok(Value::Null);
    };
    if send::is_blocked_workspace(db, row.workspace_id)
        .await
        .map_err(biz)?
    {
        return Ok(Value::Null);
    }
    let promotion_epoch = row.resume_count;
    let mut contact_count = send::count_recipients(db, broadcast_id)
        .await
        .map_err(biz)?;
    let purge_stale = || {
        chatbotx_business::broadcast::write::queue_draft_recipients_purge(
            Some(&deps.jobs),
            row.workspace_id,
            broadcast_id,
            "stalePrepare",
        )
    };
    let mut integration_messenger_id = row.integration_messenger_id;
    if integration_messenger_id.is_none() && row.subaction == "messengerTemplateMessage" {
        if let Some(template_id) = row.template_id {
            integration_messenger_id =
                send::resolve_template_integration_messenger_id(db, row.workspace_id, template_id)
                    .await
                    .map_err(biz)?;
        }
    }
    let input = AudienceInput {
        workspace_id: row.workspace_id,
        channels: Some(if row.channel.is_empty() {
            Vec::new()
        } else {
            vec![row.channel.clone()]
        }),
        inbox_ids: resolve_broadcast_target_inbox_ids(&broadcast)
            .map(|ids| ids.iter().filter_map(crate::id_of).collect()),
        integration_whatsapp_id: row.integration_whatsapp_id,
        integration_messenger_id,
        contact_filter: row
            .contact_filter
            .as_ref()
            .and_then(|f| serde_json::from_value::<FilterCriteria>(f.0.clone()).ok()),
        can_view_email_and_phone: true,
        subaction: Some(row.subaction.clone()),
        restrict_to_assigned_user_id: None,
        audience_range: resolve_broadcast_audience_range(
            row.audience_range_start,
            row.audience_range_end,
        ),
    };
    let mut walk = audience_walk(db, input, None).await.map_err(biz)?;
    loop {
        let chunk = walk.next_chunk(db).await.map_err(biz)?;
        if chunk.is_empty() {
            break;
        }
        let conversations = send::find_or_create_dms(
            db,
            row.workspace_id,
            chunk.iter().map(|ci| ci.contact_id).collect(),
        )
        .await
        .map_err(biz)?;
        let by_contact: HashMap<i64, i64> =
            conversations.iter().map(|c| (c.contact_id, c.id)).collect();
        let recipients: Vec<NewRecipient> = chunk
            .iter()
            .filter_map(|ci| {
                by_contact
                    .get(&ci.contact_id)
                    .map(|conversation_id| NewRecipient {
                        contact_id: ci.contact_id,
                        contact_inbox_id: ci.id,
                        conversation_id: *conversation_id,
                    })
            })
            .collect();
        contact_count += send::insert_recipients(db, broadcast_id, recipients)
            .await
            .map_err(biz)?;
        // Checked after writing: a chunk written after `moveToDraft` looked
        // for leftovers would be left behind, so queue its cleanup and stop.
        if !send::is_still_preparing(db, broadcast_id, promotion_epoch)
            .await
            .map_err(biz)?
        {
            purge_stale().await;
            break;
        }
    }
    let status = if contact_count > 0 { "sending" } else { "sent" };
    let promoted =
        send::promote_after_prepare(db, broadcast_id, status, contact_count, promotion_epoch)
            .await
            .map_err(biz)?;
    if !promoted {
        if contact_count > 0 {
            purge_stale().await;
        }
        tracing::warn!(
            broadcast_id,
            promotion_epoch,
            "prepareBroadcast: lost the promotion race, skipping sendBroadcast enqueue"
        );
        return Ok(Value::Null);
    }
    if status == "sent" {
        return Ok(Value::Null);
    }
    let (data, opts) = send_broadcast_job(broadcast_id);
    deps.jobs
        .add(QueueName::Schedule, "sendBroadcast", data, opts)
        .await
        .map_err(queue_err)?;
    Ok(Value::Null)
}

fn inbox_of(recipient: &PendingRecipient) -> Option<String> {
    recipient
        .contact_inbox
        .as_ref()
        .map(|ci| ci.inbox_id.to_string())
}

/// The first rule a recipient breaks, in the original order.
fn invalid_recipient(recipient: &PendingRecipient, broadcast: &Value) -> Option<&'static str> {
    let inbox = inbox_of(recipient);
    let flow = broadcast_sends_flow(broadcast);
    let template = broadcast_sends_template(broadcast);
    // `conversationId` is NOT NULL here, so the original's first rule
    // ("missing conversation for flow send") cannot apply.
    if flow
        && !inbox
            .as_deref()
            .is_some_and(|i| resolve_broadcast_flow_send(broadcast, i).is_some())
    {
        return Some(NO_FLOW_FOR_PAGE_REASON);
    }
    if template && !(recipient.conversation.is_some() && recipient.contact_inbox.is_some()) {
        return Some("missing conversation/contactInbox for template send");
    }
    if template
        && !inbox
            .as_deref()
            .is_some_and(|i| resolve_broadcast_template_send(broadcast, i).is_some())
    {
        return Some(NO_TEMPLATE_FOR_PAGE_REASON);
    }
    if uses_broadcast_targets(broadcast)
        && !inbox
            .as_deref()
            .is_some_and(|i| has_broadcast_send_for_inbox(broadcast, i))
    {
        return Some(NO_SEND_FOR_PAGE_REASON);
    }
    None
}

/// `downstreamJobOptions`.
fn downstream(job_id: String) -> JobOptions {
    JobOptions {
        job_id: Some(job_id),
        priority: Some(BROADCAST_SEND_PRIORITY),
        remove_on_complete: Some(Retention::KeepAge {
            age_secs: BROADCAST_SEND_JOB_RETENTION_SECONDS,
            count: Some(100_000),
        }),
        ..JobOptions::default()
    }
}

/// `broadcastContactSendJobId` (suffixed with the dispatch epoch).
fn contact_send_job_id(
    broadcast_id: i64,
    contact_id: i64,
    kind: &str,
    resume_count: i64,
) -> String {
    format!("broadcast-send-contact-{broadcast_id}-{contact_id}-{kind}-r{resume_count}")
}

/// `enqueueBroadcastContact`: the flow job and/or the template job.
async fn enqueue_contact(
    deps: &Deps,
    broadcast: &BroadcastForSend,
    recipient: &PendingRecipient,
) -> Result<(), JobError> {
    let row = &broadcast.row;
    let json = &broadcast.json;
    let Some(contact_inbox) = &recipient.contact_inbox else {
        return Ok(());
    };
    let inbox = contact_inbox.inbox_id.to_string();
    let metadata = json!({
        "type": BROADCAST_PAYLOAD_TYPE,
        "broadcastId": row.id.to_string(),
        "contactInboxId": recipient.contact_inbox_id.to_string(),
    });
    let flow_id = broadcast_sends_flow(json)
        .then(|| resolve_broadcast_flow_send(json, &inbox))
        .flatten()
        .filter(|f| !(f.is_null() || f == &json!("")));
    if let Some(flow_id) = flow_id {
        let data = json!({ "type": "sendFlow", "data": {
            "flowId": flow_id,
            "conversationId": recipient.conversation_id.to_string(),
            "contactInboxId": recipient.contact_inbox_id.to_string(),
            "initialBroadcastDispatch": true,
            "metadata": metadata,
        }});
        let opts = downstream(contact_send_job_id(
            row.id,
            recipient.contact_id,
            "flow",
            row.resume_count,
        ));
        deps.jobs
            .add(QueueName::Integration, "sendFlow", data, opts)
            .await
            .map_err(queue_err)?;
    }
    let Some(template) = broadcast_sends_template(json)
        .then(|| resolve_broadcast_template_send(json, &inbox))
        .flatten()
    else {
        return Ok(());
    };
    let mut payload = json!({
        "conversation": recipient.conversation,
        "contactInbox": contact_inbox.to_json(),
        "templateId": template.template_id,
        "broadcastId": row.id.to_string(),
        "metadata": metadata,
    });
    let action = if row.channel == "messenger" {
        // `templateData` is `{ ...templateParams, buttons }`: the buttons go apart.
        let mut params = template
            .template_data
            .as_object()
            .cloned()
            .unwrap_or_default();
        let buttons = params.remove("buttons");
        if !params.is_empty() {
            payload["templateData"] = Value::Object(params);
        }
        if let Some(buttons) = buttons {
            payload["buttons"] = buttons;
        }
        "sendMessengerTemplateMessage"
    } else {
        if !template.template_data.is_null() {
            payload["templateData"] = template.template_data.clone();
        }
        "sendWhatsappTemplateMessage"
    };
    let opts = downstream(contact_send_job_id(
        row.id,
        recipient.contact_id,
        "template",
        row.resume_count,
    ));
    deps.jobs
        .add(
            QueueName::Chat,
            action,
            json!({ "type": action, "data": payload }),
            opts,
        )
        .await
        .map_err(queue_err)?;
    Ok(())
}

/// `processBroadcastContacts` (`sendBroadcast`): hands one batch (the send
/// rate) of pending recipients to their send jobs; the hand-off completes
/// on a short batch.
// ponytail: recipients are handed off one by one (the original runs 100 at
// once); SQLite has one writer, so parallel hand-offs would only queue.
pub async fn process_broadcast_contacts(deps: &Deps, broadcast_id: i64) -> Result<Value, JobError> {
    let db = &deps.db;
    let Some(broadcast) = send::find_sendable(db, broadcast_id).await.map_err(biz)? else {
        return Ok(json!({ "processed": 0 }));
    };
    if send::is_blocked_workspace(db, broadcast.row.workspace_id)
        .await
        .map_err(biz)?
    {
        return Ok(json!({ "processed": 0 }));
    }
    if !claim_dispatch_window(deps, broadcast_id).await {
        tracing::debug!(
            broadcast_id,
            "processBroadcastContacts: dispatch window lease still held, skipping this tick"
        );
        return Ok(json!({ "processed": 0 }));
    }
    let batch_size = resolve_broadcast_send_rate_per_minute(broadcast.row.send_rate_per_minute);
    let recipients = send::list_pending_recipients(db, broadcast_id, batch_size)
        .await
        .map_err(biz)?;
    if recipients.is_empty() {
        send::mark_handoff_completed(db, broadcast_id)
            .await
            .map_err(biz)?;
        return Ok(json!({ "processed": 0 }));
    }
    let mut processed = 0;
    let mut retryable: Option<JobError> = None;
    for recipient in &recipients {
        let outcome: Result<bool, JobError> = async {
            if let Some(reason) = invalid_recipient(recipient, &broadcast.json) {
                send::mark_contact_failed(
                    db,
                    broadcast_id,
                    recipient.contact_id,
                    reason.to_owned(),
                )
                .await
                .map_err(biz)?;
                return Ok(false);
            }
            enqueue_contact(deps, &broadcast, recipient).await?;
            send::mark_contact_sent_if_sending(db, broadcast_id, recipient.contact_id)
                .await
                .map_err(biz)?;
            Ok(true)
        }
        .await;
        match outcome {
            Ok(true) => processed += 1,
            Ok(false) => {}
            Err(err) => {
                tracing::error!(err = %err, contact_id = recipient.contact_id, "Retryable error sending broadcast contact");
                retryable.get_or_insert(err);
            }
        }
    }
    if let Some(err) = retryable {
        return Err(err);
    }
    // A full batch: more rows remain; `reconcileBroadcasts` drives the next one.
    if recipients.len() as i64 != batch_size {
        send::mark_handoff_completed(db, broadcast_id)
            .await
            .map_err(biz)?;
    }
    Ok(json!({ "processed": processed }))
}

/// `reconcileBroadcasts`: a `sendBroadcast` job for every `sending` row
/// still awaiting its hand-off.
pub async fn reconcile_broadcasts(deps: &Deps) -> Result<Value, JobError> {
    let _guard = cron_lock(deps, "schedule:reconcile-broadcasts")
        .lock()
        .await;
    let ids = send::list_sending_awaiting_handoff(&deps.db)
        .await
        .map_err(biz)?;
    for id in &ids {
        let (data, opts) = send_broadcast_job(*id);
        deps.jobs
            .add(QueueName::Schedule, "sendBroadcast", data, opts)
            .await
            .map_err(queue_err)?;
    }
    Ok(json!({ "reconciled": ids.len() }))
}

/// `finalizeBroadcasts`: `sending → sent | failed` once every recipient
/// has an outcome (within the tolerance) or the grace window elapsed.
pub async fn finalize_broadcasts(deps: &Deps) -> Result<Value, JobError> {
    let _guard = cron_lock(deps, "schedule:finalize-broadcasts").lock().await;
    let db = &deps.db;
    let audit = AuditService::new(deps.jobs.clone());
    let now = now_ms();
    let (mut finalized, mut failed) = (0, 0);
    for broadcast in send::list_awaiting_finalization(db).await.map_err(biz)? {
        let total = broadcast.contact_count;
        let has_contacts = total.is_some_and(|t| t > 0);
        let (completed, failed_count) = if has_contacts {
            send::count_recipient_outcomes(db, broadcast.id)
                .await
                .map_err(biz)?
        } else {
            (0, 0)
        };
        let total_n = total.unwrap_or(0);
        let missing = if has_contacts { total_n - completed } else { 0 };
        let within_threshold = has_contacts
            && missing <= MAX_MISSING_CONTACTS_FOR_THRESHOLD
            && missing as f64 / total_n as f64 <= MISSING_RATE_THRESHOLD;
        let complete = !has_contacts || completed >= total_n || within_threshold;
        if !(complete || is_broadcast_outcome_grace_elapsed(broadcast.handoff_completed_at, now)) {
            continue;
        }
        let status = resolve_broadcast_terminal_status(total, failed_count);
        if !send::complete_sending(db, broadcast.id, status)
            .await
            .map_err(biz)?
        {
            continue;
        }
        let (action, detail) = if status == "failed" {
            failed += 1;
            (
                "broadcast_failed",
                format!("broadcast failed (#{})", broadcast.id),
            )
        } else {
            finalized += 1;
            (
                "broadcast_sent",
                format!("sent a broadcast (#{})", broadcast.id),
            )
        };
        audit
            .record(AuditRecordInput {
                workspace_id: Some(broadcast.workspace_id.to_string()),
                source: Some("schedule:finalizeBroadcasts".into()),
                ..AuditRecordInput::new(action, detail)
            })
            .await;
    }
    tracing::info!(finalized, failed, "finalizeBroadcasts completed");
    Ok(json!({ "skipped": false, "finalized": finalized, "failed": failed }))
}

/// `purgeBroadcasts`: chunk-deletes the recipients of soft-deleted
/// broadcasts and hard-deletes each drained row. A held lock skips the tick.
pub async fn purge_broadcasts(deps: &Deps) -> Result<Value, JobError> {
    let Ok(_guard) = cron_lock(deps, "schedule:purge-broadcasts").try_lock() else {
        tracing::warn!("purgeBroadcasts: skipped because a local purge is still running");
        return Ok(Value::Null);
    };
    let db = &deps.db;
    let deadline = now_ms() + PURGE_MAX_RUN_DURATION_MS;
    let candidates = db
        .read(|conn| {
            list_purgeable_broadcasts(conn, PURGE_CANDIDATE_LIMIT).map_err(crate::repo_err)
        })
        .await
        .map_err(db_fail)?;
    let _ = PURGE_BROADCAST_CONCURRENCY; // ponytail: one stream (SQLite has one writer).
    let (mut recipients_deleted, mut broadcasts_deleted) = (0, 0);
    for id in &candidates {
        let outcome = purge_one(deps, *id, deadline).await;
        match outcome {
            Ok((deleted, hard)) => {
                recipients_deleted += deleted;
                broadcasts_deleted += i64::from(hard);
            }
            Err(err) => {
                tracing::error!(err = %err, "purgeBroadcasts: one broadcast failed to purge")
            }
        }
    }
    tracing::info!(
        claimed = candidates.len(),
        recipients_deleted,
        broadcasts_deleted,
        "purgeBroadcasts: summary"
    );
    Ok(Value::Null)
}

async fn purge_one(deps: &Deps, broadcast_id: i64, deadline: i64) -> Result<(i64, bool), JobError> {
    let db = &deps.db;
    let outcome = purge_broadcast_recipients(
        db,
        &PurgeInput {
            broadcast_id,
            chunk_size: PURGE_CHUNK_SIZE,
            inter_chunk_delay_ms: PURGE_INTER_CHUNK_DELAY_MS,
            max_run_duration_ms: (deadline - now_ms()).max(0),
            max_chunks: None,
            only_while_draft: false,
        },
    )
    .await
    .map_err(db_fail)?;
    if outcome.stop_reason != PurgeStopReason::Drained {
        return Ok((outcome.deleted, false));
    }
    let hard = db
        .write(move |tx| {
            if has_broadcast_recipients(tx, broadcast_id).map_err(crate::repo_err)? {
                return Ok(false);
            }
            hard_delete_broadcast(tx, broadcast_id).map_err(crate::repo_err)
        })
        .await
        .map_err(db_fail)?;
    Ok((outcome.deleted, hard))
}

/// `purgeDraftBroadcastRecipients`: one bounded pass; an undrained draft
/// waits [`PURGE_DRAFT_RETRY_DELAY_MS`] in the delayed set, at most
/// [`PURGE_DRAFT_MAX_PASSES`] times.
pub async fn purge_draft_broadcast_recipients(
    deps: &Deps,
    job: &JobCtx,
    workspace_id: i64,
    broadcast_id: i64,
) -> Result<Value, JobError> {
    if send::is_blocked_workspace(&deps.db, workspace_id)
        .await
        .map_err(biz)?
    {
        return Ok(Value::Null);
    }
    if send::purge_draft_recipients(&deps.db, broadcast_id)
        .await
        .map_err(biz)?
    {
        return Ok(Value::Null);
    }
    let passes = deferrals_so_far(job.attempts_started, job.attempts_made);
    draft_purge_park(broadcast_id, passes, job.lease_token.is_some(), now_ms())
}

/// The undrained branch of `purgeDraftBroadcastRecipients`: park the job
/// [`PURGE_DRAFT_RETRY_DELAY_MS`] for another pass, or stop (warned) after
/// [`PURGE_DRAFT_MAX_PASSES`] or without the worker lease token.
pub fn draft_purge_park(
    broadcast_id: i64,
    passes: u32,
    has_token: bool,
    now: i64,
) -> Result<Value, JobError> {
    if !has_token || passes >= PURGE_DRAFT_MAX_PASSES {
        tracing::warn!(
            broadcast_id,
            passes,
            "purgeDraftBroadcastRecipients: leftover recipients remain, stopping until the draft is re-scheduled"
        );
        return Ok(Value::Null);
    }
    Err(JobError::Delayed {
        run_at_ms: now + PURGE_DRAFT_RETRY_DELAY_MS,
    })
}
