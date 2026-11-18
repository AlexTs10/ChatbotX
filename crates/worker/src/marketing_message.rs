//! Marketing Messages in the worker: the `marketingMessage` queue
//! (`apps/worker/src/marketing-message`), the three schedule handlers
//! (`{enqueue,dispatch,finalize}-marketing-messages.ts`,
//! `process-marketing-message-campaign.ts`) and the integration job
//! `processMarketingMessageWebhook` (`marketing-message-webhook.ts`).

use std::sync::Arc;

use chatbotx_business::broadcast::send::is_blocked_workspace;
use chatbotx_business::marketing_messages::{content, delivery};
use chatbotx_db::repo::facebook_marketing_message as repo;
use chatbotx_db::repo::models::{ContactInbox, Conversation, FacebookMarketingMessage};
use chatbotx_db::repo::now_ms;
use chatbotx_jobs::options::Retention;
use chatbotx_jobs::queues::marketing_message::{
    MarketingMessageJobAction, marketing_message_campaign_job_id,
};
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use chatbotx_kv::cas::CasStore as _;
use chatbotx_kv::durable::DurableKv;
use serde_json::{Value, json};

use crate::channel::{IncomingMessage, Received};
use crate::schedule::broadcast::cron_lock;
use crate::{Deps, db_fail, fail, str_of};

/// `IntegrationJobAction.processMarketingMessageWebhook`.
pub const PROCESS_MARKETING_MESSAGE_WEBHOOK: &str = "processMarketingMessageWebhook";
const FALLBACK_TEXT: &str = "[Marketing message]";
const DISPATCH_WINDOW_MS: u64 = 55_000;

fn business(error: chatbotx_business::BusinessError) -> JobError {
    fail(error.to_string())
}

/// `enqueueMarketingMessageStep`: one campaign step on the dedicated queue.
async fn enqueue_step(deps: &Deps, campaign_row_id: i64, step: &str) -> Result<(), JobError> {
    let action = MarketingMessageJobAction::ProcessCampaign.as_str();
    let id = campaign_row_id.to_string();
    deps.jobs
        .add(
            QueueName::MarketingMessage,
            action,
            json!({ "type": action, "data": { "campaignRowId": id, "step": step } }),
            JobOptions {
                job_id: Some(marketing_message_campaign_job_id(&id, step)),
                attempts: Some(1),
                remove_on_complete: Some(Retention::Remove),
                remove_on_fail: Some(Retention::Remove),
                ..JobOptions::default()
            },
        )
        .await
        .map(drop)
        .map_err(|e| fail(e.to_string()))
}

async fn rows(
    deps: &Deps,
    f: impl FnOnce(
        &chatbotx_db::rusqlite::Connection,
    ) -> chatbotx_db::repo::Result<Vec<FacebookMarketingMessage>>
    + Send
    + 'static,
) -> Result<Vec<FacebookMarketingMessage>, JobError> {
    deps.db
        .read(move |conn| f(conn).map_err(crate::repo_err))
        .await
        .map_err(db_fail)
}

async fn enqueue_all(
    deps: &Deps,
    campaigns: &[FacebookMarketingMessage],
    step: impl Fn(&FacebookMarketingMessage) -> &'static str,
) -> i64 {
    let mut enqueued = 0;
    for campaign in campaigns {
        let step = step(campaign);
        match enqueue_step(deps, campaign.id, step).await {
            Ok(()) => enqueued += 1,
            Err(error) => {
                tracing::error!(
                    ?error,
                    campaign_row_id = campaign.id,
                    step,
                    "failed to enqueue marketing message campaign"
                );
            }
        }
    }
    enqueued
}

/// `enqueueMarketingMessages` (every minute): due campaigns → prepare / startDay.
pub async fn enqueue_marketing_messages(deps: &Deps) -> Result<Value, JobError> {
    let _guard = cron_lock(deps, "schedule:enqueue-marketing-messages")
        .lock()
        .await;
    let now = now_ms();
    let due = rows(deps, move |conn| repo::list_due_to_start(conn, now)).await?;
    let enqueued = enqueue_all(deps, &due, |c| {
        if c.status == "waitingNextDay" {
            "startDay"
        } else {
            "prepare"
        }
    })
    .await;
    Ok(json!({ "enqueued": enqueued }))
}

/// `dispatchMarketingMessages` (every minute): sending campaigns → dispatch.
pub async fn dispatch_marketing_messages(deps: &Deps) -> Result<Value, JobError> {
    let _guard = cron_lock(deps, "schedule:dispatch-marketing-messages")
        .lock()
        .await;
    let sending = rows(deps, |conn| repo::list_by_status(conn, "sending")).await?;
    let enqueued = enqueue_all(deps, &sending, |_| "dispatch").await;
    Ok(json!({ "enqueued": enqueued }))
}

/// `finalizeMarketingMessages` (every minute).
pub async fn finalize_marketing_messages(deps: &Deps) -> Result<Value, JobError> {
    let _guard = cron_lock(deps, "schedule:finalize-marketing-messages")
        .lock()
        .await;
    let sending = rows(deps, |conn| repo::list_by_status(conn, "sending")).await?;
    for campaign in &sending {
        if let Err(error) = delivery::finalize(&deps.db, campaign.id).await {
            tracing::error!(%error, campaign_row_id = campaign.id, "finalizeMarketingMessages: finalize failed");
        }
    }
    Ok(json!({ "checked": sending.len() }))
}

/// `claimDispatchWindow`: SET NX PX in the durable kv; a store failure fails open.
async fn claim_dispatch_window(deps: &Deps, campaign_row_id: i64) -> bool {
    let store = DurableKv::new(deps.db.clone(), Arc::new(chatbotx_kv::SystemClock));
    let key = format!("mm:{campaign_row_id}:dispatch-window");
    match store
        .set_if_absent(&key, &json!(true), DISPATCH_WINDOW_MS)
        .await
    {
        Ok(claimed) => claimed,
        Err(error) => {
            tracing::error!(%error, campaign_row_id, "Failed to claim marketing message dispatch window lease");
            true
        }
    }
}

/// The `marketingMessage` queue (`processMarketingMessageCampaign`).
pub async fn handle(deps: Deps, job: chatbotx_jobs::JobCtx) -> Result<Value, JobError> {
    if str_of(&job.data, "type") != Some(MarketingMessageJobAction::ProcessCampaign.as_str()) {
        return Ok(Value::Null);
    }
    let data = &job.data["data"];
    let Some(id) = crate::id_of(&data["campaignRowId"]) else {
        return Err(JobError::unrecoverable("processCampaign: bad data"));
    };
    let db = &deps.db;
    let Some(row) = rows(&deps, move |conn| {
        Ok(repo::find_by_id(conn, id)?.into_iter().collect())
    })
    .await?
    .pop() else {
        return Ok(Value::Null);
    };
    // A blocked owner pauses the campaign instead of the guard's silent skip.
    if is_blocked_workspace(db, row.workspace_id)
        .await
        .map_err(business)?
    {
        delivery::pause(db, row.id, "workspaceBlocked", None, None)
            .await
            .map_err(business)?;
        return Ok(Value::Null);
    }
    match str_of(data, "step") {
        Some("prepare") => delivery::prepare(db, row.id).await,
        Some("startDay") => delivery::start_day(db, row.id).await,
        Some("dispatch") if claim_dispatch_window(&deps, row.id).await => {
            delivery::dispatch_batch(db, row.id).await.map(drop)
        }
        _ => Ok(()),
    }
    .map_err(business)?;
    Ok(Value::Null)
}

/// `processMarketingMessageWebhook`: parse, apply, and mirror an echo with a
/// PSID into the inbox as an outgoing message (never creates contacts). An
/// unparseable value is logged and dropped (no retry).
pub async fn process_webhook(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let page_id = str_of(data, "pageId").unwrap_or_default().to_owned();
    let field = str_of(data, "field").unwrap_or_default();
    let Some(event) = delivery::parse_webhook(field, &data["value"]) else {
        tracing::warn!(
            page_id,
            field,
            "Unrecognized marketing message webhook payload"
        );
        return Ok(());
    };
    let Some(campaign) = delivery::apply_webhook(&deps.db, &event)
        .await
        .map_err(business)?
    else {
        return Ok(());
    };
    if event.field != "marketing_message_echoes" {
        return Ok(());
    }
    let Some(psid) = event.psid else {
        return Ok(());
    };
    let Some(mid) = event.mid else {
        tracing::debug!(
            page_id,
            campaign_row_id = campaign.id,
            "Marketing message echo has a PSID but no mid; not mirrored to the inbox"
        );
        return Ok(());
    };
    // Only the mirror is guarded: the status update above applies regardless.
    if is_blocked_workspace(&deps.db, campaign.workspace_id)
        .await
        .map_err(business)?
    {
        return Ok(());
    }
    mirror_echo(deps, &page_id, &campaign, psid, mid).await
}

async fn mirror_echo(
    deps: &Deps,
    page_id: &str,
    campaign: &FacebookMarketingMessage,
    psid: String,
    mid: String,
) -> Result<(), JobError> {
    let inbox = deps
        .channels
        .resolve_by_identifier(&deps.db, deps.realtime.as_ref(), "messenger", page_id)
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?
        .map(|(_, _, inbox)| inbox)
        .ok_or_else(|| {
            JobError::unrecoverable(format!("Integration not found: messenger {page_id}"))
        })?;
    // Tenant guard: the Page must belong to the campaign's workspace.
    if inbox.workspace_id != campaign.workspace_id {
        tracing::warn!(
            page_id,
            campaign_row_id = campaign.id,
            "Marketing message echo page does not match campaign workspace"
        );
        return Ok(());
    }
    let (inbox_id, ws) = (inbox.inbox_id, campaign.workspace_id);
    let found = deps
        .db
        .read(move |conn| {
            let contact_inbox: Option<ContactInbox> = chatbotx_db::repo::find_one(
                conn,
                r#""inboxId" = ?1 AND "sourceId" = ?2
                   AND EXISTS (SELECT 1 FROM "Contact" c WHERE c."id" = "ContactInbox"."contactId" AND c."workspaceId" = ?3)
                   ORDER BY "lastMessageAt" DESC NULLS LAST"#,
                &[&inbox_id, &psid, &ws],
            )
            .map_err(crate::repo_err)?;
            let Some(contact_inbox) = contact_inbox else {
                return Ok(None);
            };
            let conversation: Option<Conversation> = chatbotx_db::repo::find_one(
                conn,
                r#""workspaceId" = ?1 AND "contactId" = ?2 AND "sourceId" IS NULL"#,
                &[&ws, &contact_inbox.contact_id],
            )
            .map_err(crate::repo_err)?;
            Ok(conversation.map(|c| (contact_inbox, c)))
        })
        .await
        .map_err(db_fail)?;
    let Some((contact_inbox, conversation)) = found else {
        return Ok(());
    };
    let summary = content::parse(&campaign.content.0)
        .map(|c| content::summarize(&c))
        .unwrap_or_default();
    let mut incoming = IncomingMessage::incoming(Some(mid));
    incoming.message_type = "outgoing".into();
    incoming.text = Some(if summary.is_empty() {
        FALLBACK_TEXT.to_owned()
    } else {
        summary
    });
    incoming.content_attributes = Some(json!({
        "metadata": { "type": "marketingMessage", "campaignRowId": campaign.id.to_string() },
    }));
    // A redelivered webhook hits the sourceId dedup of the insert; an echo of
    // our own send is not agent activity, so the conversation stays unread.
    crate::inbound::save_message(
        deps,
        &inbox,
        &contact_inbox,
        &conversation,
        &incoming,
        &Received::default(),
        None,
    )
    .await
    .map(drop)
}
