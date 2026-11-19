//! `apps/worker/src/default/handlers/process-inbound-webhook-request.ts` and
//! `schedule/handlers/redispatch-inbound-webhook-requests.ts`.
//!
//! The claim is exclusive and leased, every write after it is fenced by the
//! claim token, and each action result is checkpointed, so duplicate jobs,
//! retries and reclaims neither apply twice nor lose work.

use chatbotx_business::errors::{BusinessError, sanitize_public_text};
use chatbotx_business::inbound_webhook as service;
use chatbotx_business::inbound_webhook_apply::{Applied, apply_payload};
use chatbotx_db::repo::inbound_webhook::{
    InboundWebhookRequestCompletion as Completion, InboundWebhookRequestProgress as Progress,
};
use chatbotx_db::repo::models::{InboundWebhook, InboundWebhookRequest};
use chatbotx_jobs::queues::inbound_webhook::{
    InboundWebhookRequestData, enqueue_process_inbound_webhook_request,
};
use chatbotx_jobs::{JobCtx, JobError};
use serde_json::{Value, json};

use crate::trigger::{Outcome, run_inbound_webhook_action};
use crate::{Deps, str_of};

const PROCESSING_FAILED_MESSAGE: &str = "The request could not be processed";
const ACTION_FAILED_MESSAGE: &str = "The action failed";
/// Claims after which an infrastructure error stops being transient.
const MAX_CLAIMS_FOR_TRANSIENT_ERRORS: i64 = 10;
const CHECKPOINT_ATTEMPTS: u64 = 3;
const CHECKPOINT_RETRY_DELAY_MS: u64 = 150;
const REDISPATCH_BATCH_SIZE: i64 = 500;
/// `inboundWebhookActionTypes`.
const ACTION_TYPES: &[&str] = &[
    "startAnotherFlow",
    "addTag",
    "removeTag",
    "setCustomField",
    "clearCustomField",
    "transferConversationToHuman",
];

/// The applied target of a request.
struct Target {
    inbox_id: i64,
    contact_id: i64,
    contact_inbox_id: i64,
    field_errors: Vec<Value>,
}

fn or_null(items: &[Value]) -> Option<Value> {
    (!items.is_empty()).then(|| Value::Array(items.to_vec()))
}

fn biz(err: BusinessError) -> Failure {
    Failure::Business(err)
}

/// Why an attempt failed.
enum Failure {
    Business(BusinessError),
}

impl Failure {
    /// `isBusinessError`: a 4xx exception fails again.
    fn is_business(&self) -> bool {
        matches!(self, Failure::Business(e) if e.status().is_some_and(|s| s < 500))
    }

    fn message(&self) -> String {
        match self {
            Failure::Business(e) => e.message(),
        }
    }
}

fn request_id(data: &Value) -> Option<i64> {
    str_of(data, "requestId").and_then(|id| id.parse().ok())
}

/// `processInboundWebhookRequest`.
pub async fn process_request(deps: &Deps, job: &JobCtx, data: &Value) -> Result<(), JobError> {
    let Some(id) = request_id(data) else {
        return Err(JobError::unrecoverable(
            "processInboundWebhookRequest: bad data",
        ));
    };
    let Some((request, token)) = service::claim_request(&deps.db, id)
        .await
        .map_err(|e| JobError::retryable(e.message()))?
    else {
        return Ok(());
    };
    match process_claimed(deps, &request, &token).await {
        Ok(()) => Ok(()),
        Err(failure) => {
            let message = failure.message();
            tracing::error!(request_id = id, err = %message, "Inbound webhook request failed");
            settle_failed_attempt(deps, job, &request, &token, &failure).await;
            Err(JobError::retryable(message))
        }
    }
}

/// `skipBlockedInboundWebhookRequest`: the owner is blocked, so the request
/// closes as skipped (out of the re-dispatch scan).
pub async fn skip_blocked_request(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(id) = request_id(data) else {
        return Ok(());
    };
    let fail = |e: BusinessError| JobError::retryable(e.message());
    if let Some((request, token)) = service::claim_request(&deps.db, id).await.map_err(fail)? {
        let completion = Completion {
            status: "skipped".into(),
            skip_reason: Some(Some("workspaceBlocked".into())),
            ..Completion::default()
        };
        service::complete_request(&deps.db, request.id, token, completion)
            .await
            .map_err(fail)?;
    }
    Ok(())
}

async fn complete(
    deps: &Deps,
    request: &InboundWebhookRequest,
    token: &str,
    completion: Completion,
) -> Result<(), Failure> {
    service::complete_request(&deps.db, request.id, token.to_owned(), completion)
        .await
        .map(|_| ())
        .map_err(biz)
}

async fn process_claimed(
    deps: &Deps,
    request: &InboundWebhookRequest,
    token: &str,
) -> Result<(), Failure> {
    let webhook = service::find_or_fail(&deps.db, request.workspace_id, request.inbound_webhook_id)
        .await
        .map_err(biz)?;
    if !webhook.active {
        let completion = Completion {
            status: "skipped".into(),
            skip_reason: Some(Some("webhookInactive".into())),
            ..Completion::default()
        };
        return complete(deps, request, token, completion).await;
    }
    let resumed = find_applied_target(deps, request).await?;
    let target = match resumed {
        Some(target) => target,
        None => match apply_and_record(deps, request, token, &webhook).await? {
            Some(target) => target,
            None => return Ok(()),
        },
    };
    let Some(results) = run_actions(deps, request, token, &webhook, &target).await? else {
        return Ok(());
    };
    let has_problem =
        !target.field_errors.is_empty() || results.iter().any(|r| r["status"] != "succeeded");
    let completion = Completion {
        status: if has_problem {
            "completedWithErrors"
        } else {
            "processed"
        }
        .into(),
        progress: Progress {
            inbox_id: Some(Some(target.inbox_id)),
            contact_id: Some(Some(target.contact_id)),
            contact_inbox_id: Some(Some(target.contact_inbox_id)),
            field_errors: Some(or_null(&target.field_errors)),
            action_results: Some(or_null(&results)),
        },
        ..Completion::default()
    };
    complete(deps, request, token, completion).await
}

/// `findAppliedTarget`: what an earlier attempt applied, while that contact
/// inbox still belongs to the contact.
async fn find_applied_target(
    deps: &Deps,
    request: &InboundWebhookRequest,
) -> Result<Option<Target>, Failure> {
    let (Some(contact_id), Some(contact_inbox_id), Some(inbox_id), Some(_)) = (
        request.contact_id,
        request.contact_inbox_id,
        request.inbox_id,
        request.action_results.as_ref(),
    ) else {
        return Ok(None);
    };
    let ws = request.workspace_id;
    let still_owned = chatbotx_business::tx::read(&deps.db, move |c| {
        Ok(chatbotx_db::repo::contact_inbox::find_by_id_for_contact(
            c,
            ws,
            contact_id,
            contact_inbox_id,
        )?
        .is_some())
    })
    .await
    .map_err(biz)?;
    Ok(still_owned.then(|| Target {
        inbox_id,
        contact_id,
        contact_inbox_id,
        field_errors: request
            .field_errors
            .as_ref()
            .and_then(|j| j.0.as_array().cloned())
            .unwrap_or_default(),
    }))
}

/// `applyAndRecord`: `None` ends the attempt (skipped, or the claim is lost).
async fn apply_and_record(
    deps: &Deps,
    request: &InboundWebhookRequest,
    token: &str,
    webhook: &InboundWebhook,
) -> Result<Option<Target>, Failure> {
    match apply_payload(&deps.db, webhook, request.payload.0.clone())
        .await
        .map_err(biz)?
    {
        Applied::Skipped {
            reason,
            inbox_id,
            field_errors,
        } => {
            let completion = Completion {
                status: "skipped".into(),
                skip_reason: Some(Some(reason.into())),
                progress: Progress {
                    inbox_id: Some(inbox_id),
                    field_errors: Some(or_null(&field_errors)),
                    ..Progress::default()
                },
                ..Completion::default()
            };
            complete(deps, request, token, completion).await?;
            Ok(None)
        }
        Applied::Processed {
            inbox_id,
            contact_id,
            contact_inbox_id,
            field_errors,
            ..
        } => {
            let progress = Progress {
                inbox_id: Some(Some(inbox_id)),
                contact_id: Some(Some(contact_id)),
                contact_inbox_id: Some(Some(contact_inbox_id)),
                field_errors: Some(or_null(&field_errors)),
                action_results: Some(Some(
                    request
                        .action_results
                        .as_ref()
                        .map_or_else(|| json!([]), |j| j.0.clone()),
                )),
            };
            let owned =
                service::checkpoint_request(&deps.db, request.id, token.to_owned(), progress)
                    .await
                    .map_err(biz)?;
            Ok(owned.then_some(Target {
                inbox_id,
                contact_id,
                contact_inbox_id,
                field_errors,
            }))
        }
    }
}

/// `checkpointActionResults`: retried a few times, since the action's side
/// effect is already done.
async fn checkpoint_results(
    deps: &Deps,
    request: &InboundWebhookRequest,
    token: &str,
    results: &[Value],
) -> Result<bool, Failure> {
    let mut attempt = 1;
    loop {
        let progress = Progress {
            action_results: Some(Some(Value::Array(results.to_vec()))),
            ..Progress::default()
        };
        match service::checkpoint_request(&deps.db, request.id, token.to_owned(), progress).await {
            Ok(owned) => return Ok(owned),
            Err(err) if attempt >= CHECKPOINT_ATTEMPTS => return Err(biz(err)),
            Err(err) => {
                tracing::warn!(request_id = request.id, attempt, err = %err, "Inbound webhook checkpoint failed; retrying");
                tokio::time::sleep(std::time::Duration::from_millis(
                    CHECKPOINT_RETRY_DELAY_MS * attempt,
                ))
                .await;
                attempt += 1;
            }
        }
    }
}

/// `runActions`: the actions not recorded yet, one checkpoint each; `None` =
/// the claim was lost.
async fn run_actions(
    deps: &Deps,
    request: &InboundWebhookRequest,
    token: &str,
    webhook: &InboundWebhook,
    target: &Target,
) -> Result<Option<Vec<Value>>, Failure> {
    let actions = webhook.actions.0.as_array().cloned().unwrap_or_default();
    let type_at = |i: usize| {
        actions
            .get(i)
            .and_then(|a| str_of(a, "type"))
            .map(str::to_owned)
    };
    // A result counts only for the action still at that position.
    let mut results: Vec<Value> = request
        .action_results
        .as_ref()
        .and_then(|j| j.0.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            let index = r["index"].as_u64().and_then(|i| usize::try_from(i).ok());
            index.is_some_and(|i| type_at(i).as_deref() == r["type"].as_str())
        })
        .collect();
    for (index, action) in actions.iter().enumerate() {
        if results
            .iter()
            .any(|r| r["index"].as_u64() == Some(index as u64))
        {
            continue;
        }
        let kind = action.get("type").map_or_else(
            || "undefined".to_owned(),
            |t| t.as_str().map_or_else(|| t.to_string(), str::to_owned),
        );
        let result = if !ACTION_TYPES.contains(&kind.as_str()) {
            json!({ "index": index, "type": kind, "status": "skipped", "reason": "unsupportedAction" })
        } else {
            let key = format!("inboundWebhook:{}:{index}", request.id);
            match run_inbound_webhook_action(
                deps,
                webhook.id,
                webhook.workspace_id,
                target.contact_id,
                target.contact_inbox_id,
                action,
                key,
            )
            .await
            {
                Ok(Outcome::Succeeded) => {
                    json!({ "index": index, "type": kind, "status": "succeeded" })
                }
                Ok(Outcome::Skipped(reason)) => {
                    json!({ "index": index, "type": kind, "status": "skipped", "reason": reason })
                }
                Err(err) => {
                    tracing::error!(request_id = request.id, index, action_type = %kind, err = %err, "Inbound webhook action failed");
                    json!({ "index": index, "type": kind, "status": "failed", "error": ACTION_FAILED_MESSAGE })
                }
            }
        };
        results.push(result);
        if !checkpoint_results(deps, request, token, &results).await? {
            return Ok(None);
        }
    }
    Ok(Some(results))
}

/// `settleFailedAttempt`: the last attempt of an error that will not recover
/// closes the request as `failed`; otherwise it goes back to `received`.
async fn settle_failed_attempt(
    deps: &Deps,
    job: &JobCtx,
    request: &InboundWebhookRequest,
    token: &str,
    failure: &Failure,
) {
    let will_not_recover =
        failure.is_business() || request.attempts >= MAX_CLAIMS_FOR_TRANSIENT_ERRORS;
    let result = if job.is_final_attempt() && will_not_recover {
        let (code, message) = match failure {
            Failure::Business(e) if e.code().is_some() => {
                (e.code().unwrap_or_default().to_owned(), e.message())
            }
            _ => (
                "systemError".to_owned(),
                PROCESSING_FAILED_MESSAGE.to_owned(),
            ),
        };
        let message: String = sanitize_public_text(&message).chars().take(1024).collect();
        let completion = Completion {
            status: "failed".into(),
            error_code: Some(Some(code)),
            error_message: Some(Some(message)),
            ..Completion::default()
        };
        service::complete_request(&deps.db, request.id, token.to_owned(), completion).await
    } else {
        service::release_request(&deps.db, request.id, token.to_owned()).await
    };
    if let Err(err) = result {
        tracing::error!(request_id = request.id, err = %err, "Could not settle a failed inbound webhook attempt");
    }
}

/// `redispatchInboundWebhookRequests`: closes requests too old to retry,
/// then re-enqueues the unfinished ones (the job id is deterministic).
pub async fn redispatch(deps: &Deps) -> Result<(), JobError> {
    let fail = |e: BusinessError| JobError::retryable(e.message());
    let abandoned = service::abandon_stale_requests(&deps.db)
        .await
        .map_err(fail)?;
    if abandoned > 0 {
        tracing::warn!(abandoned, "Closed inbound webhook requests as abandoned");
    }
    let stale = service::list_stale_requests(&deps.db, REDISPATCH_BATCH_SIZE)
        .await
        .map_err(fail)?;
    let mut failed = 0usize;
    for row in &stale {
        let data = InboundWebhookRequestData {
            workspace_id: row.workspace_id.to_string(),
            inbound_webhook_id: row.inbound_webhook_id.to_string(),
            request_id: row.id.to_string(),
        };
        if enqueue_process_inbound_webhook_request(deps.jobs.as_ref(), data)
            .await
            .is_err()
        {
            failed += 1;
        }
    }
    if failed > 0 {
        tracing::error!(failed, "Could not re-enqueue some inbound webhook requests");
    }
    if !stale.is_empty() {
        tracing::info!(requeued = stale.len() - failed, "Re-dispatched requests");
    }
    Ok(())
}
