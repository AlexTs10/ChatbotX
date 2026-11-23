//! The `trigger` queue (`trigger/worker.ts`, `trigger-matcher`,
//! `trigger-executor`, `action-executor`, `actions/*`) and the bookkeeping
//! of `triggerService` (run limiter, contact history, daily stats).
//!
//! ponytail: the tag-sync / ads conversion fan-out of `addTag` waits for
//! those units; it is skipped with a log line. Actions run in order; triggers run one after another
//! (the original runs them with `Promise.allSettled`).

use std::collections::HashMap;
use std::sync::Mutex;

use chatbotx_business::conversation_actions::{self, ActionJob};
use chatbotx_business::pipeline::flow_actions::{
    Interpolate, InterpolateFuture, PipelineActionContext, PipelineFlowActionStatus,
    run_pipeline_action,
};
use chatbotx_business::pipeline::mutation::PipelineActor;
use chatbotx_business::{BusinessError, bot_field, contact_custom_field, contact_sequence, tag};
use chatbotx_core::flow_config::field_reference::{FieldReference, parse_field_reference};
use chatbotx_core::utils::custom_field_operation::FieldOperationType;
use chatbotx_core::utils::datetime::TemporalInputParsing;
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobCtx, JobError, JobOptions, QueueName};
use serde_json::{Map, Value, json};

use crate::condition::{self, Condition, EvalContext};
use crate::webhook::{condition_listens_to_source, is_matchable, workspace_timezone};
use crate::{Deps, db_fail, id_of, is_blocked_workspace, str_of};

/// `TRIGGER_CONTACT_RUN_LIMIT` runs per `TRIGGER_CONTACT_RUN_WINDOW_SECONDS`.
const RUN_LIMIT: i64 = 20;
const RUN_WINDOW_MS: i64 = 60_000;

/// An active trigger with its conditions (`TriggerWithConditions`).
pub struct ActiveTrigger {
    pub id: i64,
    pub actions: Vec<Value>,
    pub conditions: Vec<Condition>,
}

/// The job data (`TriggerEventData`).
struct TriggerEvent {
    workspace_id: i64,
    contact_id: i64,
    contact_id_text: String,
    event_type: String,
    metadata: Map<String, Value>,
    channel_originated: bool,
}

/// What one action did (`ActionExecutionOutcome`).
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    Skipped(String),
}

fn skipped(reason: &str) -> Result<Outcome, JobError> {
    Ok(Outcome::Skipped(reason.to_owned()))
}

fn biz(error: BusinessError) -> JobError {
    JobError::retryable(error.message())
}

/// The queue processor.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let data = &job.data["data"];
    if is_blocked_workspace(&deps.db, str_of(data, "workspaceId").unwrap_or("")).await? {
        return Ok(Value::Null);
    }
    if str_of(&job.data, "type") != Some("evaluateTriggers") {
        return Err(JobError::unrecoverable(
            "TriggerJobAction action is not defined",
        ));
    }
    if str_of(data, "source") == Some("worker") {
        tracing::info!("Skipping worker-emitted event to prevent loop");
        return Ok(Value::Null);
    }
    let occurrence = format!("{}:{}", job.job_id, job.id);
    evaluate(&deps, data, Some(&occurrence)).await?;
    Ok(Value::Null)
}

/// `evaluateTriggers`: match, then run each trigger. Returns the run trigger ids.
pub async fn evaluate(
    deps: &Deps,
    data: &Value,
    occurrence: Option<&str>,
) -> Result<Vec<i64>, JobError> {
    let (Some(workspace_id), Some(contact_id)) = (
        data.get("workspaceId").and_then(id_of),
        data.get("contactId").and_then(id_of),
    ) else {
        return Ok(Vec::new());
    };
    let event = TriggerEvent {
        workspace_id,
        contact_id,
        contact_id_text: contact_id.to_string(),
        event_type: str_of(data, "eventType").unwrap_or("").to_owned(),
        metadata: data
            .get("eventData")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
        channel_originated: data.get("channelOriginated") == Some(&Value::Bool(true)),
    };
    let matched = find_matching_triggers(deps, &event).await?;
    if matched.is_empty() {
        return Ok(Vec::new());
    }
    tracing::info!(
        "Found {} triggers for event type {}",
        matched.len(),
        event.event_type
    );
    // `extractContactInboxId`: a snowflake-id string only.
    let contact_inbox_id = event
        .metadata
        .get("contactInboxId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse::<i64>().ok());
    // `decisionFlowOriginSchema` (strict `{ flowId, profileId }` digit strings).
    let decision_flow_id = event
        .metadata
        .get("decisionFlowOrigin")
        .and_then(Value::as_object)
        .filter(|o| o.len() == 2)
        .and_then(|o| {
            let digits = |k: &str| {
                o.get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            };
            digits("profileId")?;
            digits("flowId").map(str::to_owned)
        });
    let mut ran = Vec::new();
    for trigger in &matched {
        let input = RunInput {
            contact_inbox_id,
            decision_flow_id: decision_flow_id.as_deref(),
            occurrence,
            channel_originated: event.channel_originated,
        };
        match execute(deps, trigger, &event, &input).await {
            Ok(true) => ran.push(trigger.id),
            Ok(false) => {}
            Err(err) => {
                tracing::error!(err = %err, "Failed to execute trigger {} for contact {}", trigger.id, event.contact_id)
            }
        }
    }
    Ok(ran)
}

async fn list_active_with_conditions(
    deps: &Deps,
    workspace_id: i64,
) -> Result<Vec<ActiveTrigger>, JobError> {
    deps.db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT t."id", t."actions", c."type", c."sourceId", c."operator", c."value"
                   FROM "Trigger" t LEFT JOIN "Condition" c ON c."triggerId" = t."id"
                   WHERE t."workspaceId" = ?1 AND t."active" = 1 ORDER BY t."id", c."id""#,
            )?;
            let rows = stmt.query_map([workspace_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?;
            let mut out: Vec<ActiveTrigger> = Vec::new();
            for row in rows {
                let (id, actions, r#type, source_id, operator, value) = row?;
                if out.last().is_none_or(|t| t.id != id) {
                    out.push(ActiveTrigger {
                        id,
                        actions: serde_json::from_str::<Vec<Value>>(&actions).unwrap_or_default(),
                        conditions: Vec::new(),
                    });
                }
                if let (Some(r#type), Some(last)) = (r#type, out.last_mut()) {
                    last.conditions.push(Condition {
                        r#type,
                        source_id,
                        operator,
                        value: value.and_then(|v| serde_json::from_str(&v).ok()),
                    });
                }
            }
            Ok(out)
        })
        .await
        .map_err(db_fail)
}

/// `TriggerMatcherService.findMatchingTriggers`: every condition of the
/// event's type must match.
async fn find_matching_triggers(
    deps: &Deps,
    event: &TriggerEvent,
) -> Result<Vec<ActiveTrigger>, JobError> {
    if !is_matchable(&event.event_type) {
        return Ok(Vec::new());
    }
    let source_id = event.metadata.get("sourceId").and_then(Value::as_str);
    let candidates: Vec<ActiveTrigger> = list_active_with_conditions(deps, event.workspace_id)
        .await?
        .into_iter()
        .filter(|t| {
            t.conditions.iter().any(|c| {
                c.r#type == event.event_type
                    && condition_listens_to_source(
                        &event.event_type,
                        c.source_id.as_deref(),
                        source_id,
                    )
            })
        })
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let Some(timezone) = workspace_timezone(deps, event.workspace_id).await? else {
        return Ok(Vec::new());
    };
    let ctx = EvalContext {
        metadata: &event.metadata,
        contact_id: &event.contact_id_text,
        workspace_timezone: Some(&timezone),
        now_ms: now_ms(),
    };
    let mut matched = Vec::new();
    'triggers: for trigger in candidates {
        for c in trigger
            .conditions
            .iter()
            .filter(|c| c.r#type == event.event_type)
        {
            match condition::evaluate(&deps.db, c, &ctx).await {
                Ok(true) => {}
                Ok(false) => continue 'triggers,
                Err(err) => return Err(JobError::retryable(err)),
            }
        }
        matched.push(trigger);
    }
    Ok(matched)
}

/// (trigger, contact) → (runs, window start ms).
type RunWindows = HashMap<(i64, i64), (i64, i64)>;

struct RunInput<'a> {
    contact_inbox_id: Option<i64>,
    decision_flow_id: Option<&'a str>,
    occurrence: Option<&'a str>,
    channel_originated: bool,
}

/// `tryAcquireContactRun`: at most 20 runs per trigger and contact a minute.
/// ponytail: an in-process window map (one process); expired windows are
/// dropped when the map passes 10 000 keys.
fn try_acquire_contact_run(trigger_id: i64, contact_id: i64, now: i64) -> bool {
    static RUNS: Mutex<Option<RunWindows>> = Mutex::new(None);
    let mut guard = RUNS.lock().unwrap_or_else(|e| e.into_inner());
    let runs = guard.get_or_insert_with(HashMap::new);
    if runs.len() > 10_000 {
        runs.retain(|_, (_, start)| now - *start < RUN_WINDOW_MS);
    }
    let entry = runs.entry((trigger_id, contact_id)).or_insert((0, now));
    if now - entry.1 >= RUN_WINDOW_MS {
        *entry = (0, now);
    }
    entry.0 += 1;
    if entry.0 == RUN_LIMIT + 1 {
        tracing::warn!(
            trigger_id,
            contact_id,
            "Trigger run skipped: per-contact run limit exceeded (possible automation loop)"
        );
    }
    entry.0 <= RUN_LIMIT
}

/// `TriggerExecutorService.execute`: `Ok(false)` = the run limiter refused.
async fn execute(
    deps: &Deps,
    trigger: &ActiveTrigger,
    event: &TriggerEvent,
    input: &RunInput<'_>,
) -> Result<bool, JobError> {
    if !try_acquire_contact_run(trigger.id, event.contact_id, now_ms()) {
        return Ok(false);
    }
    let mut scope = Scope::default();
    for (index, action) in trigger.actions.iter().enumerate() {
        let ctx = ActionCtx {
            deps,
            action,
            trigger_id: trigger.id,
            workspace_id: event.workspace_id,
            contact_id: event.contact_id,
            input,
            occurrence_key: input
                .occurrence
                .map(|o| format!("trigger:{o}:{}:{index}", trigger.id)),
        };
        match execute_action(&ctx, &mut scope).await {
            Ok(Outcome::Skipped(reason)) => {
                tracing::debug!(trigger_id = trigger.id, reason, "Trigger action skipped")
            }
            Ok(Outcome::Succeeded) => {}
            Err(err) => {
                tracing::error!(err = %err, "Failed to execute action for trigger {} for contact {}", trigger.id, event.contact_id)
            }
        }
    }
    // Bookkeeping failures are logged, never retried (the actions already ran).
    let (trigger_id, contact_id, workspace_id) = (trigger.id, event.contact_id, event.workspace_id);
    let result = deps
        .db
        .write(move |tx| {
            let now = now_ms();
            tx.execute(
                r#"INSERT INTO "TriggerContactHistory" ("id", "triggerId", "contactId", "workspaceId", "firstEnteredAt") VALUES (?1, ?2, ?3, ?4, ?5)"#,
                params![new_id(), trigger_id, contact_id, workspace_id, now],
            )?;
            increment_stats(tx, trigger_id, workspace_id, now, true)?;
            Ok(())
        })
        .await;
    if let Err(err) = result {
        tracing::error!(err = %err, "Trigger {trigger_id} actions ran for contact {contact_id} but bookkeeping failed; not retrying");
    }
    tracing::info!("Successfully executed trigger {trigger_id} for contact {contact_id}");
    Ok(true)
}

/// `executeActions` of the date/time sweep: every action in order with the
/// key `datetime:{trigger}:{contact}:{index}:{occurrence}`; failures are
/// logged (no run limiter, history or stats, like the original).
pub(crate) async fn run_datetime_actions(
    deps: &Deps,
    trigger_id: i64,
    workspace_id: i64,
    contact_id: i64,
    actions: &[Value],
    occurrence: &str,
) {
    let input = RunInput {
        contact_inbox_id: None,
        decision_flow_id: None,
        occurrence: None,
        channel_originated: false,
    };
    let mut scope = Scope::default();
    for (index, action) in actions.iter().enumerate() {
        let ctx = ActionCtx {
            deps,
            action,
            trigger_id,
            workspace_id,
            contact_id,
            input: &input,
            occurrence_key: Some(format!(
                "datetime:{trigger_id}:{contact_id}:{index}:{occurrence}"
            )),
        };
        if let Err(err) = execute_action(&ctx, &mut scope).await {
            tracing::error!(err = %err, "Failed to execute action for trigger {trigger_id} for contact {contact_id}");
        }
    }
}

/// One inbound-webhook action (`ActionExecutor.execute`, source
/// `inboundWebhook`): the request's contact inbox is the threaded one and
/// `occurrence_key` keeps a re-run from sending a flow twice.
pub(crate) async fn run_inbound_webhook_action(
    deps: &Deps,
    webhook_id: i64,
    workspace_id: i64,
    contact_id: i64,
    contact_inbox_id: i64,
    action: &Value,
    occurrence_key: String,
) -> Result<Outcome, JobError> {
    let input = RunInput {
        contact_inbox_id: Some(contact_inbox_id),
        decision_flow_id: None,
        occurrence: None,
        channel_originated: false,
    };
    let ctx = ActionCtx {
        deps,
        action,
        trigger_id: webhook_id,
        workspace_id,
        contact_id,
        input: &input,
        occurrence_key: Some(occurrence_key),
    };
    execute_action(&ctx, &mut Scope::default()).await
}

/// `incrementStats` for the day (local midnight; the worker runs in UTC).
fn increment_stats(
    tx: &chatbotx_db::rusqlite::Connection,
    trigger_id: i64,
    workspace_id: i64,
    now: i64,
    success: bool,
) -> chatbotx_db::rusqlite::Result<usize> {
    let day = now - now.rem_euclid(86_400_000);
    let (ok, failed) = if success { (1, 0) } else { (0, 1) };
    tx.execute(
        r#"INSERT INTO "TriggerStat" ("id", "triggerId", "workspaceId", "date", "totalContacts", "totalExecutions", "successCount", "failureCount")
           VALUES (?1, ?2, ?3, ?4, 1, 1, ?5, ?6)
           ON CONFLICT ("triggerId", "date") DO UPDATE SET "totalContacts" = "totalContacts" + 1, "totalExecutions" = "totalExecutions" + 1,
             "successCount" = "successCount" + ?5, "failureCount" = "failureCount" + ?6"#,
        params![new_id(), trigger_id, workspace_id, day, ok, failed],
    )
}

struct ActionCtx<'a> {
    deps: &'a Deps,
    action: &'a Value,
    trigger_id: i64,
    workspace_id: i64,
    contact_id: i64,
    input: &'a RunInput<'a>,
    occurrence_key: Option<String>,
}

/// `createTriggerActionScope`: the memoized conversation and contact inbox.
#[derive(Default)]
struct Scope {
    conversation: Option<Option<Conversation>>,
    contact_inbox: Option<Option<ContactInbox>>,
}

impl Scope {
    /// `findLatestCreatedByContact`.
    async fn conversation(
        &mut self,
        ctx: &ActionCtx<'_>,
    ) -> Result<Option<Conversation>, JobError> {
        if self.conversation.is_none() {
            let (ws, contact) = (ctx.workspace_id, ctx.contact_id);
            let row = ctx
                .deps
                .db
                .read(move |c| {
                    Ok(c.query_row(
                        &format!(r#"{} WHERE "workspaceId" = ?1 AND "contactId" = ?2 ORDER BY "createdAt" DESC, "id" DESC LIMIT 1"#, Conversation::select_sql()),
                        params![ws, contact],
                        Conversation::from_row,
                    )
                    .optional()?)
                })
                .await
                .map_err(db_fail)?;
            self.conversation = Some(row);
        }
        Ok(self.conversation.clone().flatten())
    }

    /// `resolveActionContactInbox`: the threaded inbox, else the most recent.
    async fn contact_inbox(
        &mut self,
        ctx: &ActionCtx<'_>,
    ) -> Result<Option<ContactInbox>, JobError> {
        if self.contact_inbox.is_none() {
            let (ws, contact, threaded) =
                (ctx.workspace_id, ctx.contact_id, ctx.input.contact_inbox_id);
            let row = ctx
                .deps
                .db
                .read(move |c| {
                    let base = format!(
                        r#"{} WHERE "contactId" = ?1 AND "inboxId" IN (SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?2)"#,
                        ContactInbox::select_sql()
                    );
                    if let Some(id) = threaded
                        && let Some(row) = c
                            .query_row(&format!(r#"{base} AND "id" = ?3"#), params![contact, ws, id], ContactInbox::from_row)
                            .optional()?
                    {
                        return Ok(Some(row));
                    }
                    Ok(c.query_row(
                        &format!(r#"{base} ORDER BY "lastMessageAt" IS NULL, "lastMessageAt" DESC LIMIT 1"#),
                        params![contact, ws],
                        ContactInbox::from_row,
                    )
                    .optional()?)
                })
                .await
                .map_err(db_fail)?;
            self.contact_inbox = Some(row);
        }
        Ok(self.contact_inbox.clone().flatten())
    }
}

fn ids(action: &Value, key: &str) -> Vec<i64> {
    action
        .get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(id_of).collect())
        .unwrap_or_default()
}

/// `ActionExecutor.execute`.
async fn execute_action(ctx: &ActionCtx<'_>, scope: &mut Scope) -> Result<Outcome, JobError> {
    let Some(conversation) = scope.conversation(ctx).await? else {
        tracing::warn!("No conversation found for contact {}", ctx.contact_id);
        return skipped("noConversation");
    };
    let db = &ctx.deps.db;
    let (ws, contact) = (ctx.workspace_id, conversation.contact_id);
    let action_type = str_of(ctx.action, "type").unwrap_or("");
    let outcome = match action_type {
        "addTag" | "removeTag" => {
            let tag_ids = ids(ctx.action, "tagIds");
            let add = action_type == "addTag";
            let changed = if add {
                tag::attach_to_contact(db, ws, contact, &tag_ids).await
            } else {
                tag::detach_from_contact(db, ws, contact, &tag_ids).await
            }
            .map_err(biz)?;
            let pairs: Vec<(i64, i64)> = changed.iter().map(|t| (contact, *t)).collect();
            let jobs = ctx.deps.jobs.as_ref();
            if add {
                chatbotx_business::tag_sync::enqueue_attach_many(jobs, ws, &pairs).await;
            } else {
                chatbotx_business::tag_sync::enqueue_detach_many(jobs, ws, &pairs).await;
            }
            Outcome::Succeeded
        }
        "addSequence" | "removeSequence" => {
            return sequence_action(ctx, action_type, contact).await;
        }
        "setCustomField" => return set_custom_field(ctx, scope, &conversation).await,
        "clearCustomField" => {
            let field = str_of(ctx.action, "customFieldId").unwrap_or("");
            match parse_field_reference(field) {
                FieldReference::BotField { id } => {
                    bot_field::clear_value_by_key(db, ws, &id)
                        .await
                        .map_err(biz)?;
                }
                FieldReference::CustomField { key } => {
                    if let Ok(field_id) = key.parse::<i64>() {
                        contact_custom_field::delete_by_custom_field_id(
                            db,
                            ws,
                            &[contact],
                            field_id,
                        )
                        .await
                        .map_err(biz)?;
                    }
                }
            }
            Outcome::Succeeded
        }
        "archiveConversation" | "unarchiveConversation" => {
            scope.conversation = None;
            conversation_actions::set_archived(
                db,
                Some(&*ctx.deps.jobs),
                ws,
                conversation.id,
                action_type == "archiveConversation",
            )
            .await
            .map_err(biz)?;
            Outcome::Succeeded
        }
        "assignConversation" | "unassignConversation" => {
            scope.conversation = None;
            let target = (action_type == "assignConversation")
                .then(|| str_of(ctx.action, "assignedId").unwrap_or("").to_owned());
            let assigned = conversation_actions::assign_one(db, ws, conversation.id, target)
                .await
                .map_err(biz)?;
            if let (Some(realtime), Some(event)) = (&ctx.deps.realtime, assigned.event) {
                realtime.queue_workspace_event(&ws.to_string(), event).await;
            }
            if let Some(ActionJob {
                queue,
                name,
                data,
                job_id,
            }) = assigned.job
            {
                ctx.deps
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
            Outcome::Succeeded
        }
        "disableBot" | "enableBot" | "transferConversationToHuman" => {
            scope.conversation = None;
            conversation_actions::set_bot_enabled(
                db,
                ws,
                conversation.id,
                action_type == "enableBot",
            )
            .await
            .map_err(biz)?;
            if action_type == "transferConversationToHuman"
                && ctx.action.get("notifyAdmins") == Some(&Value::Bool(true))
            {
                tracing::info!("Notifying admins for conversation {}", conversation.id);
            }
            Outcome::Succeeded
        }
        "startAnotherFlow" | "startExternalStep" => {
            return start_flow(ctx, scope, &conversation, action_type).await;
        }
        "pipeline" => return pipeline(ctx, scope, &conversation).await,
        "sendGoogleAdsConversion" => return google_ads_conversion(ctx, scope, &conversation).await,
        "runGoogleSheet" => {
            // A trigger-only action (`source.kind !== "trigger"`).
            if ctx
                .occurrence_key
                .as_deref()
                .is_some_and(|k| k.starts_with("inboundWebhook:"))
            {
                return skipped("unsupportedSource");
            }
            let Some(contact_inbox) = scope.contact_inbox(ctx).await? else {
                tracing::warn!(
                    "No contact inbox found for contact {}, skipping runGoogleSheet action",
                    ctx.contact_id
                );
                return skipped("noContactInbox");
            };
            if let Some((step_type, step)) = crate::spreadsheet::step_of_action(ctx.action) {
                let run = crate::spreadsheet::SheetRun {
                    deps: ctx.deps,
                    http: chatbotx_business::email_marketing::safe_http(),
                    conversation: &conversation,
                    contact_inbox: &contact_inbox,
                    sent_by_user_id: None,
                };
                // The step's own error is its result; the action still succeeds.
                if let Err(failure) = crate::spreadsheet::execute(&run, &step_type, &step).await {
                    crate::spreadsheet::report(&run, &step_type, &failure).await;
                }
            }
            Outcome::Succeeded
        }
        "sendMetaCapiEvent" => return meta_capi_event(ctx, scope, &conversation).await,
        other => {
            tracing::warn!("Unknown action type: {other}");
            Outcome::Skipped("unknownAction".to_owned())
        }
    };
    Ok(outcome)
}

/// `sendMetaCapiEvent` (`integration-actions.ts`): a trigger-sourced action
/// on a Meta channel inbox, checked with the flow step's field schema.
async fn meta_capi_event(
    ctx: &ActionCtx<'_>,
    scope: &mut Scope,
    conversation: &Conversation,
) -> Result<Outcome, JobError> {
    use chatbotx_core::flow_config::steps::{
        meta_capi_event_fields_schema, with_meta_capi_event_refinements,
    };
    if ctx
        .occurrence_key
        .as_deref()
        .is_some_and(|k| k.starts_with("inboundWebhook:"))
    {
        return skipped("unsupportedSource");
    }
    let (ws, contact, trigger) = (ctx.workspace_id, ctx.contact_id, ctx.trigger_id);
    let Some(contact_inbox) = scope.contact_inbox(ctx).await? else {
        tracing::warn!(
            "No contact inbox found for contact {contact}, skipping sendMetaCapiEvent action"
        );
        return skipped("noContactInbox");
    };
    if chatbotx_business::meta_capi_send::capi_channel(&contact_inbox.channel).is_none() {
        tracing::warn!(
            "Unsupported Meta CAPI trigger channel: {}",
            contact_inbox.channel
        );
        return skipped("unsupportedChannel");
    }
    let parsed =
        with_meta_capi_event_refinements(meta_capi_event_fields_schema()).safe_parse(ctx.action);
    let Some(action) = parsed.data.clone().filter(|_| parsed.success) else {
        let detail = chatbotx_business::meta_capi_send::prettify(parsed.issues());
        tracing::warn!("Invalid Meta CAPI trigger action for trigger {trigger}: {detail}");
        chatbotx_business::meta_capi_send::report_input_failure(
            ctx.deps.events.as_ref(),
            ws,
            contact,
            &contact_inbox.source_id,
            format!("Invalid Meta CAPI trigger action (trigger {trigger}):\n{detail}"),
        )
        .await;
        return skipped("invalidAction");
    };
    crate::meta_capi_producer::produce(crate::meta_capi_producer::Produce {
        deps: ctx.deps,
        conversation,
        contact_inbox: &contact_inbox,
        source: "triggerAction",
        scope: "trigger",
        scope_id: trigger.to_string(),
        fields: &action,
        sent_by_user_id: None,
    })
    .await
    .map_err(JobError::retryable)?;
    Ok(Outcome::Succeeded)
}

/// `sendGoogleAdsConversion` (`integration-actions.ts`): the threaded inbox
/// when it carries a Google click, else the contact's latest click inbox;
/// the stored action is checked with the flow step's schema first.
async fn google_ads_conversion(
    ctx: &ActionCtx<'_>,
    scope: &mut Scope,
    conversation: &Conversation,
) -> Result<Outcome, JobError> {
    use chatbotx_core::flow_config::steps::{
        google_ads_conversion_fields_schema, with_google_ads_conversion_refinements,
    };
    use chatbotx_db::repo::contact_inbox as inboxes;

    use crate::google_ads_producer::{Produce, Produced, produce, report};
    let (ws, contact) = (ctx.workspace_id, ctx.contact_id);
    let threaded = scope.contact_inbox(ctx).await?.map(|c| c.id);
    let click =
        ctx.deps
            .db
            .read(move |c| {
                let found = match threaded {
                    Some(id) => inboxes::find_google_click_attribution(c, ws, id)
                        .map_err(crate::repo_err)?,
                    None => None,
                };
                let found = match found {
                    Some(row) => Some(row),
                    None => inboxes::find_latest_google_click_inbox_by_contact(c, ws, contact)
                        .map_err(crate::repo_err)?,
                };
                let Some(row) = found else { return Ok(None) };
                Ok(c.query_row(
                    &format!(r#"{} WHERE "id" = ?1"#, ContactInbox::select_sql()),
                    [row.id],
                    ContactInbox::from_row,
                )
                .optional()?)
            })
            .await
            .map_err(db_fail)?;
    let Some(click) = click else {
        tracing::warn!(
            "No contact inbox with a Google click for contact {contact}, skipping sendGoogleAdsConversion action"
        );
        return skipped("noGoogleClick");
    };
    let trigger = ctx.trigger_id;
    let parsed = with_google_ads_conversion_refinements(google_ads_conversion_fields_schema())
        .safe_parse(ctx.action);
    let Some(action) = parsed.data.clone().filter(|_| parsed.success) else {
        let code =
            chatbotx_core::utils::google_click::google_ads_conversion_error_code("invalidInput")
                .unwrap_or_default();
        let issues: Vec<String> = parsed
            .issues()
            .iter()
            .map(|issue| {
                let path: Vec<String> = issue.path.iter().map(ToString::to_string).collect();
                if path.is_empty() {
                    issue.message.clone()
                } else {
                    format!("{}: {}", path.join("."), issue.message)
                }
            })
            .collect();
        let detail = crate::google_ads_producer::describe_input_failure(
            code,
            &serde_json::json!({}),
            &issues,
        );
        tracing::warn!("Invalid Google Ads trigger action for trigger {trigger}: {detail}");
        report(
            ctx.deps,
            ws,
            contact,
            &click.source_id,
            format!("{detail}\nTrigger: {trigger}"),
        )
        .await;
        return skipped("invalidAction");
    };
    let produced = produce(Produce {
        deps: ctx.deps,
        conversation,
        contact_inbox: &click,
        source: "triggerAction",
        scope_id: trigger.to_string(),
        fields: &action,
        occurrence_key: ctx
            .occurrence_key
            .as_ref()
            .map(|key| format!("{key}:{contact}")),
        sent_by_user_id: None,
        report_suffix: format!("\nTrigger: {trigger}"),
    })
    .await;
    match produced {
        Produced::Queued => Ok(Outcome::Succeeded),
        Produced::Refused("invalidConsentConfig" | "invalidConsentValue") => {
            tracing::warn!("Google Ads conversion not recorded for trigger {trigger}: consent");
            skipped("configRefused")
        }
        Produced::Refused(status) => {
            tracing::warn!("Google Ads conversion not recorded for trigger {trigger}: {status}");
            skipped(status)
        }
        Produced::Failed(reason) => {
            tracing::error!(
                "Failed to record Google Ads conversion for trigger {trigger}: {reason}"
            );
            skipped("recordFailed")
        }
    }
}

/// `addSequence` / `removeSequence` (1..=`MAX_SEQUENCES_PER_TRIGGER_ACTION`
/// ids; only the workspace's own sequences).
async fn sequence_action(
    ctx: &ActionCtx<'_>,
    action_type: &str,
    contact: i64,
) -> Result<Outcome, JobError> {
    let raw = ctx.action.get("sequenceIds").and_then(Value::as_array);
    let Some(raw) =
        raw.filter(|a| !a.is_empty() && a.len() <= 50 && a.iter().all(Value::is_string))
    else {
        tracing::warn!("Invalid {action_type} action payload, skipping");
        return skipped("invalidAction");
    };
    let ws = ctx.workspace_id;
    let mut seen = std::collections::HashSet::new();
    let candidates: Vec<i64> = raw
        .iter()
        .filter_map(id_of)
        .filter(|id| seen.insert(*id))
        .collect();
    let owned: Vec<i64> = ctx
        .deps
        .db
        .read(move |c| {
            let mut stmt = c.prepare(r#"SELECT "id" FROM "Sequence" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#)?;
            let found: Vec<i64> = stmt
                .query_map(params![ws, Value::from(candidates.clone()).to_string()], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            Ok(candidates.into_iter().filter(|id| found.contains(id)).collect())
        })
        .await
        .map_err(db_fail)?;
    if owned.is_empty() {
        return if action_type == "addSequence" {
            skipped("sequenceNotFound")
        } else {
            Ok(Outcome::Succeeded)
        };
    }
    if action_type == "addSequence" {
        contact_sequence::subscribe_contacts(&ctx.deps.db, ws, &[contact], &owned)
            .await
            .map_err(biz)?;
    } else {
        contact_sequence::remove_contact_sequences_for_contacts(
            &ctx.deps.db,
            ws,
            &[contact],
            &owned,
        )
        .await
        .map_err(biz)?;
    }
    Ok(Outcome::Succeeded)
}

/// `setCustomField` via `applyCustomFieldOperation`.
async fn set_custom_field(
    ctx: &ActionCtx<'_>,
    scope: &mut Scope,
    conversation: &Conversation,
) -> Result<Outcome, JobError> {
    let Some(field) = str_of(ctx.action, "customFieldId").filter(|f| !f.is_empty()) else {
        return skipped("invalidAction");
    };
    let mut value = ctx
        .action
        .get("value")
        .map(chatbotx_core::js::string_of)
        .unwrap_or_default();
    // `{{variables}}` are resolved before the write (no spintax).
    if value.contains("{{")
        && let Some(variables) = ctx.deps.variables.clone()
        && let Some(contact_inbox) = scope.contact_inbox(ctx).await?
    {
        match variables
            .resolve(
                conversation,
                &contact_inbox,
                &Value::String(value.clone()),
                None,
                None,
                false,
            )
            .await
        {
            Ok(Value::String(out)) => value = out,
            Ok(_) => {}
            Err(err) => return Err(JobError::retryable(err)),
        }
    }
    let input = contact_custom_field::SetValueByKeyInput {
        workspace_id: ctx.workspace_id,
        contact_id: conversation.contact_id,
        keyword: field.to_owned(),
        value,
        source_timezone_override: str_of(ctx.action, "timezone").map(str::to_owned),
        temporal_input_parsing: Some(TemporalInputParsing::Lenient),
        fill_empty_temporal_with_now: true,
        operation: str_of(ctx.action, "operation").and_then(FieldOperationType::parse),
        touch_unchanged: false,
        allow_bot_fields: true,
    };
    match contact_custom_field::set_value_by_key(&ctx.deps.db, input).await {
        Ok(_) => Ok(Outcome::Succeeded),
        Err(err) if err.code() == Some("invalidFieldOperation") => Ok(Outcome::Skipped(format!(
            "invalidFieldOperation:{}",
            err.message()
        ))),
        Err(err) => Err(biz(err)),
    }
}

/// `startAnotherFlow` / `startExternalStep`: enqueue `sendFlow`.
async fn start_flow(
    ctx: &ActionCtx<'_>,
    scope: &mut Scope,
    conversation: &Conversation,
    action_type: &str,
) -> Result<Outcome, JobError> {
    let flow_id = str_of(ctx.action, "flowId").unwrap_or("").to_owned();
    let node_id = str_of(ctx.action, "nodeId")
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    let numeric = !flow_id.is_empty() && flow_id.bytes().all(|b| b.is_ascii_digit());
    if action_type == "startExternalStep" && (!numeric || node_id.is_none()) {
        return skipped("invalidAction");
    }
    let Some(contact_inbox) = scope.contact_inbox(ctx).await? else {
        tracing::warn!(
            "No contact inbox found for contact {}, skipping {action_type} action",
            ctx.contact_id
        );
        return skipped("noContactInbox");
    };
    if ctx.input.decision_flow_id == Some(flow_id.as_str()) {
        tracing::info!(flow_id, "Skipping Decision-origin Flow self-loop");
        return Ok(Outcome::Succeeded);
    }
    let ws = ctx.workspace_id;
    let (flow, node) = (flow_id.parse::<i64>().unwrap_or(-1), node_id.clone());
    // `resolveStartTarget` (`findActiveById` for startAnotherFlow).
    let status = ctx
        .deps
        .db
        .read(move |c| {
            let row: Option<(bool, Option<i64>)> = c
                .query_row(
                    r#"SELECT "active", "currentVersionId" FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![flow, ws],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((active, version)) = row else { return Ok("flowNotFound") };
            if !active {
                return Ok("flowInactive");
            }
            let Some(version) = version else { return Ok("flowNotPublished") };
            let Some(node) = node else { return Ok("ready") };
            let found: Option<i64> = c
                .query_row(
                    r#"SELECT 1 FROM "FlowVersion", json_each("FlowVersion"."nodes") WHERE "FlowVersion"."id" = ?1 AND json_extract(value, '$.id') = ?2"#,
                    params![version, node],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(if found.is_some() { "ready" } else { "nodeNotFound" })
        })
        .await
        .map_err(db_fail)?;
    if status != "ready" {
        if action_type == "startAnotherFlow" {
            tracing::warn!(
                "Flow {flow_id} not found or not active, skipping startAnotherFlow action"
            );
            return skipped("flowInactive");
        }
        return skipped(status);
    }
    let mut data = json!({
        "conversationId": conversation.id.to_string(),
        "contactInboxId": contact_inbox.id.to_string(),
        "flowId": flow_id,
    });
    if let Some(node) = node_id {
        data["nodeId"] = json!(node);
    }
    if ctx.input.channel_originated {
        data["origin"] = json!("channel");
    }
    ctx.deps
        .jobs
        .add(
            QueueName::Integration,
            "sendFlow",
            json!({ "type": "sendFlow", "data": data }),
            JobOptions {
                // `enqueueSendFlow`: a deterministic id for inbound webhooks.
                job_id: ctx
                    .occurrence_key
                    .as_deref()
                    .filter(|k| k.starts_with("inboundWebhook:"))
                    .map(|k| format!("send-flow-{}", k.replace(':', "-"))),
                ..JobOptions::default()
            },
        )
        .await
        .map_err(db_fail)?;
    Ok(Outcome::Succeeded)
}

/// `pipeline` via `runPipelineAction` (actor `trigger`).
async fn pipeline(
    ctx: &ActionCtx<'_>,
    scope: &mut Scope,
    conversation: &Conversation,
) -> Result<Outcome, JobError> {
    let variables = ctx.deps.variables.clone();
    let contact_inbox = scope.contact_inbox(ctx).await?;
    let conversation = conversation.clone();
    // ponytail: a resolver failure keeps the text as written (as the pipeline step).
    let interpolate: Interpolate = std::sync::Arc::new(move |text: String| {
        let (variables, conversation, contact_inbox) = (
            variables.clone(),
            conversation.clone(),
            contact_inbox.clone(),
        );
        Box::pin(async move {
            let (Some(variables), Some(contact_inbox)) =
                (variables.filter(|_| text.contains("{{")), contact_inbox)
            else {
                return text;
            };
            match variables
                .resolve(
                    &conversation,
                    &contact_inbox,
                    &Value::String(text.clone()),
                    None,
                    None,
                    false,
                )
                .await
            {
                Ok(Value::String(out)) => out,
                _ => text,
            }
        }) as InterpolateFuture
    });
    let outcome = run_pipeline_action(
        &ctx.deps.db,
        ctx.action,
        &PipelineActionContext {
            workspace_id: ctx.workspace_id,
            contact_id: ctx.contact_id,
            actor: PipelineActor {
                kind: "trigger".into(),
                user_id: None,
                source_id: Some(ctx.trigger_id.to_string()),
            },
            base_idempotency_key: ctx.occurrence_key.clone(),
            interpolate,
        },
    )
    .await
    .map_err(biz)?;
    Ok(match outcome.status {
        PipelineFlowActionStatus::Success => Outcome::Succeeded,
        PipelineFlowActionStatus::Skip => {
            Outcome::Skipped(outcome.message.unwrap_or_else(|| "pipelineSkipped".into()))
        }
        PipelineFlowActionStatus::Error => Outcome::Skipped("pipelineError".into()),
    })
}

#[cfg(test)]
#[path = "trigger_tests.rs"]
mod tests;
