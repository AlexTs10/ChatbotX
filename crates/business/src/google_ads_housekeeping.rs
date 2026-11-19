//! `google-ads/housekeeping.ts` + `processing-status.ts` + `redrive.ts`: the
//! Data Manager status poll (`requestStatus:retrieve`), the stranded /
//! stale-claim sweep and the daily setup sync. Every step is fenced by the
//! event's generation (and request id / claim token), so a stale run never
//! overwrites newer work.

use std::collections::HashMap;

use chatbotx_db::repo::google_ads_conversion_event::{
    self as events, ApplyProcessingResultInput, ClaimInput, EventRef, RedriveInput, StrandedInput,
};
use chatbotx_db::repo::integration_google_ads;
use chatbotx_db::repo::models::GoogleAdsConversionEvent as Event;
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_jobs::QueueName;
use chatbotx_jobs::store::JobState;
use serde_json::{Value, json};

use crate::connection::credentials::Upstream;
use crate::error_log::{ProviderErrorInput, Thrown, log_provider_error};
use crate::errors::Result;
use crate::google_ads_api::{self as api, Failure, RequestStatus};
use crate::google_ads_conversion::{
    self as gac, DeliverDeps, Lease, MAX_REDRIVE_GENERATIONS, MIN_CLICK_AGE_MS,
    PROCESSING_REDRIVE_DELAY_MS,
};
use crate::google_ads_setup::{self as setup, Setup, TokenFailure};
use crate::tx;

const HOUR_MS: i64 = 60 * 60 * 1000;
/// A request still unresolved this long after being sent is failed as timed out.
pub const PROCESSING_TIMEOUT_MS: i64 = 7 * 24 * HOUR_MS;
/// A pending event past this age with no live job is stranded.
pub const STRANDED_PENDING_AFTER_MS: i64 = 10 * 60 * 1000;
/// A lease older than this belongs to a crashed worker.
pub const STALE_CLAIM_AFTER_MS: i64 = 15 * 60 * 1000;
const PROCESSING_BACKOFF_MS: [i64; 4] = [3 * HOUR_MS, 6 * HOUR_MS, 12 * HOUR_MS, 24 * HOUR_MS];

const POLL_PAGE_SIZE: i64 = 100;
/// Safety ceiling on pages per run; the time budget normally ends the run first.
const MAX_POLL_PAGES: usize = 50;
/// Stay well inside the 10-minute cron cadence.
const POLL_TIME_BUDGET_MS: i64 = 5 * 60 * 1000;
const SYNC_PAGE_SIZE: i64 = 100;
pub const SWEEP_LIMIT: i64 = 200;
const MAX_ERROR_LENGTH: usize = 1000;
const LEGACY_REQUEST_ID_PREFIX: &str = "legacy:";

/// `nextProcessingBackoffMs`: `attempts` = polls that already ran.
pub fn next_processing_backoff_ms(attempts: i64) -> i64 {
    let index = usize::try_from(attempts.max(0)).unwrap_or(usize::MAX);
    PROCESSING_BACKOFF_MS[index.min(PROCESSING_BACKOFF_MS.len() - 1)]
}

// ---- processing-status.ts ------------------------------------------------------

const REASON_PREFIX: &str = "PROCESSING_ERROR_REASON_";

fn normalize_reason(reason: &str) -> &str {
    reason.strip_prefix(REASON_PREFIX).unwrap_or(reason)
}

/// `isRetryableProcessingReason`: Google's own transient reasons.
pub fn is_retryable_processing_reason(reason: &str) -> bool {
    matches!(
        normalize_reason(reason),
        "INTERNAL_ERROR" | "TOO_RECENT_CLICK" | "DESTINATION_TOO_RECENTLY_CREATED"
    )
}

/// Google already holds this conversion (a redrive re-sent it).
fn is_duplicate_processing_reason(reason: &str) -> bool {
    matches!(
        normalize_reason(reason),
        "DUPLICATE_TRANSACTION_ID" | "DUPLICATE_GCLID"
    )
}

/// `ProcessingOutcome`.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome<'a> {
    Success(&'a RequestStatus),
    /// Only duplicate reasons: the conversion is recorded.
    Duplicate(&'a RequestStatus),
    Processing,
    Failed {
        status: &'a RequestStatus,
        retryable: bool,
    },
    /// Zero / several entries or an unknown status: poll again, never guess.
    Unknown,
}

/// `classifyRequestStatus` of a one-event, one-destination request.
pub fn classify_request_status(statuses: &[RequestStatus]) -> Outcome<'_> {
    let [status] = statuses else {
        return Outcome::Unknown;
    };
    let all = |f: fn(&str) -> bool| {
        !status.error_counts.is_empty() && status.error_counts.iter().all(|(r, _)| f(r))
    };
    match status.request_status.as_str() {
        "SUCCESS" => Outcome::Success(status),
        "PROCESSING" => Outcome::Processing,
        "FAILED" | "PARTIAL_SUCCESS" if all(is_duplicate_processing_reason) => {
            Outcome::Duplicate(status)
        }
        "FAILED" | "PARTIAL_SUCCESS" => Outcome::Failed {
            status,
            retryable: all(is_retryable_processing_reason),
        },
        _ => Outcome::Unknown,
    }
}

// ---- housekeeping.ts -----------------------------------------------------------

/// `REASON_PATTERN` (`^[A-Z0-9_]{1,80}$`), else `UNKNOWN`.
fn bound_reason(reason: &str) -> &str {
    let shaped = (1..=80).contains(&reason.len())
        && reason
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
    if shaped { reason } else { "UNKNOWN" }
}

fn clip(text: &str) -> String {
    text.chars().take(MAX_ERROR_LENGTH).collect()
}

fn counts(list: &[(String, i64)]) -> Value {
    Value::Array(
        list.iter()
            .map(|(reason, count)| json!({ "reason": bound_reason(reason), "recordCount": count }))
            .collect(),
    )
}

/// `toProcessingDetail`.
fn processing_detail(status: &RequestStatus) -> Value {
    json!({
        "requestStatus": clip(&status.request_status),
        "recordCount": status.record_count,
        "errorCounts": counts(&status.error_counts),
        "warningCounts": counts(&status.warning_counts),
    })
}

/// `safeReasons`: only enum-shaped reasons are kept.
fn safe_reasons(list: &[(String, i64)]) -> String {
    let joined = list
        .iter()
        .map(|(reason, _)| bound_reason(reason))
        .collect::<Vec<_>>()
        .join(", ");
    clip(&joined)
}

/// A poll only applies to the generation and request it read (`pollFence`).
fn fenced(event: &Event, to: &str, processing_status: &str) -> ApplyProcessingResultInput {
    ApplyProcessingResultInput {
        id: event.id,
        workspace_id: event.workspace_id,
        to: to.into(),
        processing_status: processing_status.into(),
        processing_attempts: event.processing_attempts + 1,
        expected_request_id: event.request_id.clone(),
        expected_attempt: event.attempt,
        ..ApplyProcessingResultInput::default()
    }
}

async fn apply(db: &chatbotx_db::Db, input: ApplyProcessingResultInput) -> Result<bool> {
    let applied = tx::write(db, move |conn| {
        Ok(events::apply_processing_result(conn, &input)?)
    })
    .await?;
    Ok(applied.is_some())
}

fn log_stale_poll(event: &Event) {
    tracing::warn!(
        event_id = event.id,
        attempt = event.attempt,
        "google ads: stale processing poll ignored, event moved on"
    );
}

/// `reschedulePoll`: still `sent`, next poll after the backoff.
async fn reschedule_poll(
    db: &chatbotx_db::Db,
    event: &Event,
    now: i64,
    processing_status: &str,
) -> Result<()> {
    apply(
        db,
        ApplyProcessingResultInput {
            next_processing_check_at: Some(
                now + next_processing_backoff_ms(event.processing_attempts),
            ),
            ..fenced(event, "sent", processing_status)
        },
    )
    .await?;
    Ok(())
}

async fn fail_processing<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    status: &RequestStatus,
) -> Result<()> {
    let reasons = safe_reasons(&status.error_counts);
    let message = if reasons.is_empty() {
        clip(&status.request_status)
    } else {
        reasons
    };
    let processing_status = if status.request_status == "PARTIAL_SUCCESS" {
        "partial_success"
    } else {
        "failed"
    };
    let applied = apply(
        deps.db,
        ApplyProcessingResultInput {
            processing_detail: Some(processing_detail(status)),
            failure_stage: Some("processing".into()),
            error: Some(message.clone()),
            ..fenced(event, "failed", processing_status)
        },
    )
    .await?;
    if applied {
        log_provider_error(
            deps.events,
            ProviderErrorInput::new(
                "google-ads",
                &event.workspace_id.to_string(),
                Thrown::Error {
                    message: format!("Google Ads conversion not processed: {message}"),
                    stack: None,
                    status: Some(400),
                },
            ),
        )
        .await;
    } else {
        log_stale_poll(event);
    }
    Ok(())
}

/// `redriveAfterTransientFailure`: a new generation, 6 h later.
async fn redrive_after_transient_failure<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    status: &RequestStatus,
) -> Result<()> {
    if event.attempt >= MAX_REDRIVE_GENERATIONS {
        return fail_processing(deps, event, status).await;
    }
    let input = RedriveInput {
        id: event.id,
        workspace_id: event.workspace_id,
        from_statuses: vec!["sent".into()],
        expected_attempt: event.attempt,
        expected_request_id: event.request_id.clone(),
        expected_claim_token: None,
    };
    let redriven = tx::write(deps.db, move |conn| Ok(events::redrive(conn, &input)?)).await?;
    match redriven {
        Some(redriven) => {
            gac::enqueue_send(deps.jobs, &redriven, PROCESSING_REDRIVE_DELAY_MS).await?;
        }
        None => log_stale_poll(event),
    }
    Ok(())
}

/// `finishProcessed`: allowlisted fields only, no error counts.
async fn finish_processed(
    db: &chatbotx_db::Db,
    event: &Event,
    status: &RequestStatus,
    duplicate_recovery: bool,
) -> Result<()> {
    let mut detail = json!({
        "requestStatus": clip(&status.request_status),
        "recordCount": status.record_count,
        "errorCounts": [],
        "warningCounts": counts(&status.warning_counts),
    });
    if duplicate_recovery {
        detail["duplicateRecovery"] = json!(true);
    }
    let applied = apply(
        db,
        ApplyProcessingResultInput {
            processing_detail: Some(detail),
            ..fenced(event, "processed", "success")
        },
    )
    .await?;
    if !applied {
        log_stale_poll(event);
    }
    Ok(())
}

async fn apply_outcome<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    outcome: Outcome<'_>,
    now: i64,
) -> Result<()> {
    match outcome {
        Outcome::Success(status) => finish_processed(deps.db, event, status, false).await,
        Outcome::Duplicate(status) => finish_processed(deps.db, event, status, true).await,
        Outcome::Processing => reschedule_poll(deps.db, event, now, "processing").await,
        Outcome::Unknown => reschedule_poll(deps.db, event, now, "unknown").await,
        Outcome::Failed {
            status,
            retryable: true,
        } => redrive_after_transient_failure(deps, event, status).await,
        Outcome::Failed { status, .. } => fail_processing(deps, event, status).await,
    }
}

/// `loadPollContext`: the setup and a fresh access token, once per workspace
/// and run. `None` = blocked owner, no setup or a grant that needs reauth.
async fn poll_context<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    workspace_id: i64,
) -> Result<Option<(Setup, String)>> {
    if crate::broadcast::send::is_blocked_workspace(deps.db, workspace_id).await? {
        return Ok(None);
    }
    let Some(setup) = setup::get_setup(deps.db, workspace_id).await? else {
        return Ok(None);
    };
    if setup.connection_status.as_deref() == Some("needs_reauth") {
        return Ok(None);
    }
    match setup::fresh_auth(deps.db, deps.up, &setup.integration, false).await {
        Ok(auth) => {
            let token = auth["tokens"]["accessToken"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            Ok(Some((setup, token)))
        }
        Err(TokenFailure::Revoked) => Ok(None),
        Err(TokenFailure::Other(message)) => Err(crate::errors::BusinessError::exception(
            message,
            "internalServerError",
            500,
        )),
    }
}

type PollCache = HashMap<i64, Option<(Setup, String)>>;

async fn poll_one<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    now: i64,
    cache: &mut PollCache,
) -> std::result::Result<(), String> {
    if event
        .sent_at
        .is_some_and(|sent| now - sent > PROCESSING_TIMEOUT_MS)
    {
        apply(
            deps.db,
            ApplyProcessingResultInput {
                failure_stage: Some("timeout".into()),
                error: Some("Google did not finish processing the request in time".into()),
                ..fenced(event, "failed", "timed_out")
            },
        )
        .await
        .map_err(|e| e.message())?;
        return Ok(());
    }
    // A legacy upload completes synchronously: never poll it.
    if event.upload_method == "legacy"
        || event
            .request_id
            .as_deref()
            .is_some_and(|id| id.starts_with(LEGACY_REQUEST_ID_PREFIX))
    {
        tracing::warn!(
            event_id = event.id,
            "google ads: legacy event reached the poller, skipped"
        );
        return Ok(());
    }
    if let std::collections::hash_map::Entry::Vacant(slot) = cache.entry(event.workspace_id) {
        // Cached either way (as the original caches the rejected promise).
        let loaded = poll_context(deps, event.workspace_id).await;
        let failed = loaded
            .as_ref()
            .err()
            .map(crate::errors::BusinessError::message);
        slot.insert(loaded.ok().flatten());
        if let Some(reason) = failed {
            return Err(reason);
        }
    }
    let context = cache.get(&event.workspace_id).and_then(Option::as_ref);
    let (Some((setup, token)), Some(request_id)) = (context, event.request_id.as_deref()) else {
        return reschedule_poll(deps.db, event, now, "unknown")
            .await
            .map_err(|e| e.message());
    };
    if setup.integration.customer_id != event.customer_id {
        return reschedule_poll(deps.db, event, now, "unknown")
            .await
            .map_err(|e| e.message());
    }
    let statuses = api::retrieve_request_status(deps.up, token, request_id)
        .await
        .map_err(|failure: Failure| failure.public_message(&[&event.click_id]))?;
    apply_outcome(deps, event, classify_request_status(&statuses), now)
        .await
        .map_err(|e| e.message())
}

/// `pollGoogleAdsProcessingStatus`: every due `sent` event, page after page,
/// until a short page or the time budget. One failure never stops the run;
/// the event is rescheduled. Every handled event moves its next check
/// forward, so re-querying from the start never revisits a row.
// ponytail: sequential (the original polls 4 at a time); one SQLite writer
// gains little from parallel polls.
pub async fn poll_processing_status<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    now: i64,
) -> Result<()> {
    let mut cache = PollCache::new();
    let deadline = now_ms() + POLL_TIME_BUDGET_MS;
    for _ in 0..MAX_POLL_PAGES {
        let due = tx::read(deps.db, move |conn| {
            Ok(events::list_due_for_processing_check(
                conn,
                now,
                POLL_PAGE_SIZE,
            )?)
        })
        .await?;
        for event in &due {
            if let Err(reason) = poll_one(deps, event, now, &mut cache).await {
                tracing::warn!(
                    %reason,
                    event_id = event.id,
                    "google ads: processing status poll failed"
                );
                if let Err(error) = reschedule_poll(deps.db, event, now, "unknown").await {
                    tracing::warn!(
                        error = %error.message(),
                        event_id = event.id,
                        "google ads: processing status poll could not be rescheduled"
                    );
                }
            }
        }
        if (due.len() as i64) < POLL_PAGE_SIZE || now_ms() >= deadline {
            break;
        }
    }
    Ok(())
}

/// `isSendJobLive`: the job of this generation can still run.
async fn is_send_job_live(jobs: &dyn chatbotx_jobs::Enqueuer, event: &Event) -> Result<bool> {
    let state = jobs
        .job_state(
            QueueName::Integration,
            &gac::send_job_id(event.id, event.attempt),
        )
        .await
        .map_err(|e| {
            crate::errors::BusinessError::exception(e.to_string(), "internalServerError", 500)
        })?;
    Ok(matches!(
        state,
        JobState::Waiting | JobState::Delayed | JobState::Active | JobState::Prioritized
    ))
}

/// `redriveAndEnqueue`: the next generation, behind the 6 h click gate.
async fn redrive_and_enqueue(
    deps_db: &chatbotx_db::Db,
    jobs: &dyn chatbotx_jobs::Enqueuer,
    input: RedriveInput,
    now: i64,
) -> Result<Option<Event>> {
    let redriven = tx::write(deps_db, move |conn| Ok(events::redrive(conn, &input)?)).await?;
    if let Some(redriven) = &redriven {
        let delay = gac::send_delay_ms(redriven.google_click_received_at, now);
        gac::enqueue_send(jobs, redriven, delay).await?;
    }
    Ok(redriven)
}

/// `failExhaustedStranded`: claimed under a fresh lease, then failed.
async fn fail_exhausted_stranded<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    redriven: &Event,
) -> Result<()> {
    let lease = Lease {
        id: redriven.id,
        workspace_id: redriven.workspace_id,
        claim_token: new_id().to_string(),
    };
    let claim = ClaimInput {
        id: lease.id,
        workspace_id: lease.workspace_id,
        claim_token: lease.claim_token.clone(),
        attempt: redriven.attempt,
    };
    let claimed = tx::write(deps.db, move |conn| {
        Ok(events::claim_for_sending(conn, &claim)?)
    })
    .await?;
    if let Some(claimed) = claimed {
        let failure = Failure::Network("Delivery stranded too many times".into());
        gac::fail(deps, &claimed, &lease, &failure, false).await?;
    }
    Ok(())
}

async fn redrive_stranded<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    event: &Event,
    now: i64,
) -> Result<()> {
    let input = RedriveInput {
        id: event.id,
        workspace_id: event.workspace_id,
        from_statuses: vec![event.status.clone()],
        expected_attempt: event.attempt,
        expected_request_id: None,
        expected_claim_token: Some(event.claim_token.clone()),
    };
    if event.attempt < MAX_REDRIVE_GENERATIONS {
        redrive_and_enqueue(deps.db, deps.jobs, input, now).await?;
        return Ok(());
    }
    let redriven = tx::write(deps.db, move |conn| Ok(events::redrive(conn, &input)?)).await?;
    if let Some(redriven) = redriven {
        fail_exhausted_stranded(deps, &redriven).await?;
    }
    Ok(())
}

async fn touch(db: &chatbotx_db::Db, event: &Event) -> Result<()> {
    let (r, status, attempt) = (
        EventRef {
            id: event.id,
            workspace_id: event.workspace_id,
        },
        event.status.clone(),
        event.attempt,
    );
    tx::write(db, move |conn| {
        Ok(events::touch_stranded(conn, r, &status, attempt)?)
    })
    .await
}

/// `parkBlockedStranded`: a blocked owner's row is never sent. A stale
/// `sending` row goes back to `pending` on the SAME generation; everything
/// else is only touched (it rotates behind the rows that need work).
async fn park_blocked_stranded(db: &chatbotx_db::Db, event: &Event) -> Result<()> {
    if let (true, Some(token)) = (event.status == "sending", event.claim_token.clone()) {
        let (r, attempt) = (
            EventRef {
                id: event.id,
                workspace_id: event.workspace_id,
            },
            event.attempt,
        );
        let released = tx::write(db, move |conn| {
            Ok(events::release_claim(conn, r, &token, attempt)?)
        })
        .await?;
        if released.is_some() {
            return Ok(());
        }
        tracing::warn!(
            event_id = event.id,
            attempt = event.attempt,
            "google ads: stale lease already moved on, release skipped"
        );
    }
    touch(db, event).await
}

async fn sweep_one<U: Upstream>(deps: &DeliverDeps<'_, U>, event: &Event, now: i64) -> Result<()> {
    if event.status == "pending" && is_send_job_live(deps.jobs, event).await? {
        // Rotate it behind the rows that still need work.
        return touch(deps.db, event).await;
    }
    if crate::broadcast::send::is_blocked_workspace(deps.db, event.workspace_id).await? {
        return park_blocked_stranded(deps.db, event).await;
    }
    redrive_stranded(deps, event, now).await
}

/// `sweepStrandedGoogleAdsEvents`: events whose delivery job is gone (lost
/// enqueue, crashed worker) move to a fresh generation. A pending event is
/// redriven only when its generation's job can no longer run.
pub async fn sweep_stranded<U: Upstream>(
    deps: &DeliverDeps<'_, U>,
    now: i64,
    limit: i64,
) -> Result<()> {
    let input = StrandedInput {
        pending_older_than: now - STRANDED_PENDING_AFTER_MS,
        six_hour_gate_before: now - MIN_CLICK_AGE_MS,
        sending_older_than: now - STALE_CLAIM_AFTER_MS,
        limit,
    };
    let stranded = tx::read(deps.db, move |conn| {
        Ok(events::list_stranded(conn, &input)?)
    })
    .await?;
    for event in &stranded {
        if let Err(error) = sweep_one(deps, event, now).await {
            tracing::warn!(
                error = %error.message(),
                event_id = event.id,
                "google ads: stranded event could not be redriven"
            );
        }
    }
    Ok(())
}

/// `syncGoogleAdsSetups`: daily, each active account's conversion customer
/// and actions are read again; a revoked grant flips the connection to
/// `needs_reauth` here rather than on the next conversion.
pub async fn sync_setups(
    db: &chatbotx_db::Db,
    up: &impl Upstream,
    encryption: &chatbotx_core::encryption::Encryption,
    app_url: &str,
) -> Result<()> {
    let mut after_id = None;
    loop {
        let targets = tx::read(db, move |conn| {
            Ok(integration_google_ads::list_sync_targets(
                conn,
                after_id,
                SYNC_PAGE_SIZE,
            )?)
        })
        .await?;
        for target in &targets {
            let ws = target.workspace_id;
            let synced: Result<()> = async {
                if crate::broadcast::send::is_blocked_workspace(db, ws).await? {
                    return Ok(());
                }
                let active = setup::get_setup(db, ws).await?.is_some_and(|s| {
                    s.connection_status
                        .as_deref()
                        .is_some_and(crate::connection::is_active)
                });
                if active {
                    let credential = setup::credential(db, encryption, app_url, ws).await?;
                    setup::refresh_setup(db, up, credential.as_ref(), ws).await?;
                }
                Ok(())
            }
            .await;
            if let Err(error) = synced {
                tracing::warn!(
                    error = %error.message(),
                    workspace_id = ws,
                    "google ads: daily setup sync failed"
                );
            }
        }
        if (targets.len() as i64) < SYNC_PAGE_SIZE {
            return Ok(());
        }
        after_id = targets.last().map(|t| t.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_reasons() {
        assert_eq!(bound_reason("TOO_RECENT_CLICK"), "TOO_RECENT_CLICK");
        assert_eq!(bound_reason("gclid=abc"), "UNKNOWN");
        assert_eq!(bound_reason(""), "UNKNOWN");
    }
}
