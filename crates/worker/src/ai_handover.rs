//! Q.W2 slice 2: the AI hand-over "apply to all customers" engine
//! (`integration/handlers/ai-handover-bulk-toggle.ts`, integration job
//! `aiHandoverBulkToggle`) and its sweeper (`scanAiHandoverBulkRuns`,
//! schedule queue, every minute at second 36).
//!
//! ponytail: the per-batch channel calls (`createBulkThreadControl`: the
//! Messenger hand-over protocol, settle, batch accounting, pauses) come with
//! the thread-control channel units (Q.D12 / Q.I1b). Until then a Page's
//! thread control cannot be loaded, so a run walks the original's
//! "integration unavailable" path: the eligible threads are counted skipped
//! and the run fails with `integrationUnavailable`.

use chatbotx_business::ai_handover;
use chatbotx_core::utils::channel::parse_ai_handover_channel;
use chatbotx_db::repo::ai_handover_bulk_run::{self as runs, FinishInput, ProgressInput};
use chatbotx_db::repo::contact_inbox::count_bulk_ai_eligible;
use chatbotx_db::repo::models::AiHandoverBulkRun;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::queries::ai_handover_bulk_eligibility::AiHandoverBulkAction;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::JobError;
use chatbotx_kv::{DurableKv, SystemClock};
use serde_json::Value;

use crate::{Deps, db_fail, id_of, repo_err};

/// `AI_HANDOVER_BULK_MAX_ATTEMPTS`.
const MAX_ATTEMPTS: i64 = 5;
const PICK_BATCH: i64 = 100;
const RECONCILE_BATCH: i64 = 50;
const RECONCILE_CURSOR_KEY: &str = "schedule:scan-ai-handover-bulk-runs:cursor";
const RECONCILE_CURSOR_TTL_SECS: u64 = 60 * 60;

/// How a step of the chunk ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Continue,
    /// The run row is settled (finished or cancelled).
    Ended,
    /// The claim was lost.
    Abandoned,
}

struct Chunk<'a> {
    deps: &'a Deps,
    run: AiHandoverBulkRun,
    token: String,
    processed: i64,
    skipped: i64,
    failed: i64,
    cursor: Option<String>,
}

impl Chunk<'_> {
    fn action(&self) -> AiHandoverBulkAction {
        if self.run.action == "enable" {
            AiHandoverBulkAction::Enable
        } else {
            AiHandoverBulkAction::Disable
        }
    }

    /// `writeProgress`: a guarded write; a cancelled run is wound down.
    async fn progress(&mut self, progress: ProgressInput) -> Result<Step, JobError> {
        let (id, token) = (self.run.id, self.token.clone());
        let p = progress.clone();
        let status = self
            .deps
            .db
            .write(move |tx| runs::record_progress(tx, id, &token, &p, now_ms()).map_err(repo_err))
            .await
            .map_err(db_fail)?;
        let Some(status) = status else {
            tracing::warn!(run_id = id, "[ai-handover-bulk] claim lost, abandoning");
            return Ok(Step::Abandoned);
        };
        self.processed += progress.add_processed.unwrap_or(0);
        self.skipped += progress.add_skipped.unwrap_or(0);
        self.failed += progress.add_failed.unwrap_or(0);
        if let Some(cursor) = progress.cursor_contact_inbox_id {
            self.cursor = cursor;
        }
        if status == "cancelling" {
            return self.finish("cancelled", None).await;
        }
        Ok(Step::Continue)
    }

    /// `finishRun`, then `reconcile` (best effort).
    async fn finish(&mut self, status: &str, error: Option<&str>) -> Result<Step, JobError> {
        let outcome = FinishInput {
            status: status.to_owned(),
            current_error: error.map(|e| Some(e.to_owned())),
            total_count: Some(self.processed + self.skipped + self.failed),
        };
        let (id, token) = (self.run.id, self.token.clone());
        let written = self
            .deps
            .db
            .write(move |tx| runs::finish(tx, id, &token, &outcome, now_ms()).map_err(repo_err))
            .await
            .map_err(db_fail)?;
        if written == 0 {
            tracing::warn!(run_id = id, "[ai-handover-bulk] finish lost the claim");
            return Ok(Step::Ended);
        }
        if let Err(err) = ai_handover::reconcile(
            &self.deps.db,
            Some(&self.deps.jobs),
            self.run.workspace_id,
            self.run.inbox_id,
        )
        .await
        {
            tracing::error!(err = %err, run_id = id, "[ai-handover-bulk] reconcile after the run ended failed");
        }
        Ok(Step::Ended)
    }

    /// `stopReasonOf`.
    async fn stop_reason(&self) -> Result<Option<&'static str>, JobError> {
        let (ws, inbox, enable) = (
            self.run.workspace_id,
            self.run.inbox_id,
            self.run.action == "enable",
        );
        self.deps
            .db
            .read(move |conn| {
                ai_handover::find_stop_reason(conn, ws, inbox, enable, now_ms())
                    .map_err(|e| chatbotx_db::DbError::other(e.to_string()))
            })
            .await
            .map_err(db_fail)
    }

    /// `countEligible` after `after` (the whole Page when `None`).
    async fn count_eligible(&self, after: Option<String>) -> Result<i64, JobError> {
        let run = self.run.clone();
        let action = self.action();
        let Some(channel) = parse_ai_handover_channel(Some(&run.channel)) else {
            return Ok(0);
        };
        self.deps
            .db
            .read(move |conn| {
                let input =
                    ai_handover::eligibility_of(action, run.requested_at, channel, now_ms());
                count_bulk_ai_eligible(
                    conn,
                    &input,
                    run.workspace_id,
                    run.inbox_id,
                    after.and_then(|a| a.parse().ok()),
                )
                .map_err(repo_err)
            })
            .await
            .map_err(db_fail)
    }

    /// `endPageUnusable`.
    async fn end_page_unusable(&mut self, reason: &str) -> Result<Step, JobError> {
        let remaining = self.count_eligible(self.cursor.clone()).await?;
        tracing::warn!(
            run_id = self.run.id,
            inbox_id = self.run.inbox_id,
            remaining,
            reason,
            "[ai-handover-bulk] page unusable"
        );
        match self
            .progress(ProgressInput {
                add_skipped: Some(remaining),
                ..ProgressInput::default()
            })
            .await?
        {
            Step::Continue => self.finish("failed", Some(reason)).await,
            other => Ok(other),
        }
    }

    /// `prepare` + `processPage` up to the thread-control load.
    async fn drive(&mut self) -> Result<(), JobError> {
        if self.run.status == "cancelling" {
            self.finish("cancelled", None).await?;
            return Ok(());
        }
        if let Some(reason) = self.stop_reason().await? {
            self.finish("cancelled", Some(reason)).await?;
            return Ok(());
        }
        // `recoverInFlightBatch`.
        if let Some(to) = self.run.in_flight_to_id.clone() {
            let disable = self.run.action == "disable";
            let step = self
                .progress(ProgressInput {
                    cursor_contact_inbox_id: disable.then_some(Some(to)),
                    current_error: disable.then(|| "batchFailed".to_owned()),
                    in_flight_from_id: Some(None),
                    in_flight_to_id: Some(None),
                    ..ProgressInput::default()
                })
                .await?;
            if step != Step::Continue {
                return Ok(());
            }
        }
        // `findRunInbox`.
        let (ws, inbox_id) = (self.run.workspace_id, self.run.inbox_id);
        let channel = self
            .deps
            .db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "channel" FROM "Inbox" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                        params![inbox_id, ws],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten())
            })
            .await
            .map_err(db_fail)?;
        let Some(channel) = channel.filter(|c| parse_ai_handover_channel(Some(c)).is_some()) else {
            self.finish("cancelled", Some("pageDisconnected")).await?;
            return Ok(());
        };
        // `countTotal`.
        if self.run.total_count.is_none() {
            let total = self.count_eligible(None).await?;
            let step = self
                .progress(ProgressInput {
                    total_count: Some(total),
                    ..ProgressInput::default()
                })
                .await?;
            if step != Step::Continue {
                return Ok(());
            }
        }
        // `createBulkThreadControl`: no Rust thread control yet (module doc).
        tracing::warn!(
            run_id = self.run.id,
            inbox_id,
            channel = %channel,
            "[ai-handover-bulk] integration unavailable"
        );
        self.end_page_unusable("integrationUnavailable").await?;
        Ok(())
    }
}

/// `runAiHandoverBulkToggle`: one chunk of a claimed run.
pub async fn run_bulk_toggle(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let (Some(run_id), Some(ws)) = (
        data.get("runId").and_then(id_of),
        data.get("workspaceId").and_then(id_of),
    ) else {
        return Ok(());
    };
    let run = deps
        .db
        .write(move |tx| runs::claim(tx, run_id, ws, now_ms()).map_err(repo_err))
        .await
        .map_err(db_fail)?;
    let Some(run) = run else {
        return Ok(());
    };
    let Some(token) = run.claim_token.clone() else {
        return Ok(());
    };
    let mut chunk = Chunk {
        deps,
        processed: run.processed_count,
        skipped: run.skipped_count,
        failed: run.failed_count,
        cursor: run.cursor_contact_inbox_id.clone(),
        run,
        token,
    };
    chunk.drive().await.inspect_err(|err| {
        tracing::error!(err = %err, run_id, "[ai-handover-bulk] chunk crashed");
    })
}

/// `scanAiHandoverBulkRuns`: fail the exhausted runs, re-dispatch the due
/// ones (a still-queued previous chunk refunds the attempt), then reconcile
/// the Pages whose latest revision has no run, from a stored cursor.
// ponytail: no distributed lock; one process runs the schedule queue.
pub async fn scan(deps: &Deps) -> Result<(), JobError> {
    let db = &deps.db;
    let picked = db
        .write(|tx| {
            let now = now_ms();
            runs::mark_max_attempts_failed(tx, MAX_ATTEMPTS, now).map_err(repo_err)?;
            runs::pick_due(tx, MAX_ATTEMPTS, PICK_BATCH, now).map_err(repo_err)
        })
        .await
        .map_err(db_fail)?;
    if !picked.is_empty() {
        tracing::info!(count = picked.len(), "scanAiHandoverBulkRuns: picked runs");
    }
    for run in picked {
        // `refundIfPreviousChunkQueued`.
        if run.status != "cancelling" && previous_chunk_queued(deps, run.id, run.chunk_seq).await {
            let (id, attempts, seq) = (run.id, run.attempts, run.chunk_seq);
            db.write(move |tx| runs::refund_dispatch(tx, id, attempts, seq).map_err(repo_err))
                .await
                .map_err(db_fail)?;
            continue;
        }
        if let Err(err) =
            ai_handover::enqueue_chunk(&*deps.jobs, run.id, run.workspace_id, run.chunk_seq, None)
                .await
        {
            tracing::error!(err = %err, run_id = run.id, "scanAiHandoverBulkRuns: enqueue failed");
        }
    }
    reconcile_awaiting(deps).await
}

/// `integrationQueue.getJob(previous).getState()` in a queued state; the
/// job rows live in the same SQLite file (`JobStore`).
async fn previous_chunk_queued(deps: &Deps, run_id: i64, chunk_seq: i64) -> bool {
    let job_id = ai_handover::bulk_job_id(run_id, chunk_seq - 1);
    let state = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT state FROM jobs WHERE queue = 'integration' AND job_id = ?1",
                    params![job_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await;
    match state {
        Ok(state) => matches!(state.as_deref(), Some("waiting" | "delayed")),
        Err(err) => {
            tracing::warn!(err = %err, run_id, "could not look up the queued chunk job");
            false
        }
    }
}

async fn reconcile_awaiting(deps: &Deps) -> Result<(), JobError> {
    let kv = DurableKv::new(deps.db.clone(), std::sync::Arc::new(SystemClock));
    let cursor: Option<i64> = kv
        .get::<Value>(RECONCILE_CURSOR_KEY)
        .await
        .ok()
        .flatten()
        .as_ref()
        .and_then(id_of);
    let list = |after: Option<i64>| {
        deps.db.read(move |conn| {
            runs::list_inboxes_awaiting_run(conn, RECONCILE_BATCH, after).map_err(repo_err)
        })
    };
    let mut pages = list(cursor).await.map_err(db_fail)?;
    if pages.is_empty() && cursor.is_some() {
        pages = list(None).await.map_err(db_fail)?;
    }
    for &(ws, inbox) in &pages {
        if let Err(err) = ai_handover::reconcile(&deps.db, Some(&deps.jobs), ws, inbox).await {
            tracing::error!(err = %err, inbox_id = inbox, "scanAiHandoverBulkRuns: reconcile failed");
        }
    }
    let result = match pages.last() {
        Some(&(_, last)) if pages.len() as i64 == RECONCILE_BATCH => {
            kv.put(
                RECONCILE_CURSOR_KEY,
                &last.to_string(),
                Some(RECONCILE_CURSOR_TTL_SECS),
            )
            .await
        }
        _ => kv.delete(&[RECONCILE_CURSOR_KEY]).await,
    };
    if let Err(err) = result {
        tracing::warn!(err = %err, "scanAiHandoverBulkRuns: cursor write failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! `ai-handover-bulk-toggle.test.ts` (claiming, preconditions, the
    //! integration-unavailable path, resuming, reconcile after the end) and
    //! `scan-ai-handover-bulk-runs.test.ts` (dispatch, refund, exhausted
    //! runs, the reconcile cursor). The batch cases need the thread control
    //! (Q.I1b).
    use chatbotx_db::rusqlite::params;
    use serde_json::json;

    use super::*;
    use crate::steps::test_support::Fixture;

    const WS: i64 = 2;
    const PAGE: i64 = 40;
    const RUN: i64 = 900;

    async fn exec(fx: &Fixture, sql: &'static str, args: Vec<i64>) {
        fx.deps
            .db
            .write(move |tx| {
                tx.execute(sql, chatbotx_db::rusqlite::params_from_iter(args.iter()))?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// A connected Page with `threads` contacts that wrote an hour ago, and
    /// a run row of `action` / `status`.
    async fn page(threads: i64, action: &str, status: &str) -> Fixture {
        let fx = Fixture::new().await;
        let (action, status) = (action.to_owned(), status.to_owned());
        fx.deps
            .db
            .write(move |tx| {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId", "status")
                       VALUES (?1, ?2, 'Page', 'messenger', 'page-1', 'connected')"#,
                    params![PAGE, WS],
                )?;
                for i in 0..threads {
                    tx.execute(
                        r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'C', ?2)"#,
                        params![5000 + i, WS],
                    )?;
                    tx.execute(
                        r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel",
                             "source", "sourceId", "lastIncomingMessageAt")
                           VALUES (?1, ?2, ?2, ?3, 'messenger', 'messenger', ?4, ?5)"#,
                        params![6000 + i, 5000 + i, PAGE, format!("psid-{i}"), now_ms() - 3_600_000],
                    )?;
                    tx.execute(
                        r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (?1, ?2, ?3)"#,
                        params![7000 + i, WS, 5000 + i],
                    )?;
                }
                if action == "disable" {
                    tx.execute(
                        r#"UPDATE "ContactInbox" SET "threadControlState" = 'standby', "threadOwnerRole" = 'ai_agent'"#,
                        [],
                    )?;
                }
                tx.execute(
                    r#"INSERT INTO "AIHandoverSettings" ("id", "workspaceId", "inboxId", "channel", "enabled",
                         "scheduleEnabled", "timeRanges", "applyToAllCustomers", "applyToAllRevision")
                       VALUES (9001, ?1, ?2, 'messenger', 1, 0, '[]', ?3, 1)"#,
                    params![WS, PAGE, action == "enable"],
                )?;
                tx.execute(
                    r#"INSERT INTO "AIHandoverBulkRun" ("id", "createdAt", "updatedAt", "workspaceId", "inboxId",
                         "channel", "revision", "action", "status", "message", "requestedAt", "attempts",
                         "chunkSeq", "processedCount", "skippedCount", "failedCount")
                       VALUES (?1, 0, 0, ?2, ?3, 'messenger', 1, ?4, ?5, 'bye', ?6, 1, 1, 0, 0, 0)"#,
                    params![RUN, WS, PAGE, action, status, now_ms()],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        fx
    }

    async fn run_row(fx: &Fixture, id: i64) -> AiHandoverBulkRun {
        fx.deps
            .db
            .read(move |conn| {
                use chatbotx_db::repo::Model;
                let sql = format!(
                    r#"SELECT {} FROM "AIHandoverBulkRun" WHERE "id" = ?1"#,
                    AiHandoverBulkRun::columns_sql(None)
                );
                Ok(conn.query_row(&sql, [id], AiHandoverBulkRun::from_row)?)
            })
            .await
            .unwrap()
    }

    async fn toggle(fx: &Fixture, ws: i64) {
        run_bulk_toggle(
            &fx.deps,
            &json!({ "runId": RUN.to_string(), "workspaceId": ws.to_string() }),
        )
        .await
        .unwrap();
    }

    // claiming

    #[tokio::test]
    async fn a_run_this_worker_did_not_win_terminal_or_held_by_another_does_nothing() {
        let fx = page(2, "enable", "completed").await;
        toggle(&fx, WS).await;
        let run = run_row(&fx, RUN).await;
        assert_eq!(run.status, "completed");
        assert_eq!(run.skipped_count, 0);
        assert!(run.claim_token.is_none());
    }

    #[tokio::test]
    async fn the_claim_is_scoped_to_the_jobs_workspace_so_a_stray_job_cannot_take_the_lease() {
        let fx = page(2, "enable", "pending").await;
        toggle(&fx, 999).await;
        let run = run_row(&fx, RUN).await;
        assert_eq!(run.status, "pending");
        assert!(run.claim_token.is_none());
    }

    #[tokio::test]
    async fn a_run_cancelled_before_this_chunk_started_is_wound_down_not_walked() {
        let fx = page(2, "enable", "cancelling").await;
        toggle(&fx, WS).await;
        let run = run_row(&fx, RUN).await;
        assert_eq!(run.status, "cancelled");
        assert_eq!(run.skipped_count, 0);
        assert_eq!(run.total_count, Some(0));
    }

    // preconditions

    #[tokio::test]
    async fn a_run_stops_cancelled_with_the_reason_when_the_page_was_disconnected_before_any_batch()
    {
        let fx = page(2, "disable", "pending").await;
        exec(
            &fx,
            r#"UPDATE "Inbox" SET "status" = 'disconnected' WHERE "id" = ?1"#,
            vec![PAGE],
        )
        .await;
        toggle(&fx, WS).await;
        let run = run_row(&fx, RUN).await;
        assert_eq!(run.status, "cancelled");
        assert_eq!(run.current_error.as_deref(), Some("pageDisconnected"));
        assert_eq!(run.skipped_count, 0);
    }

    #[tokio::test]
    async fn a_run_stops_cancelled_with_the_reason_when_the_automation_is_no_longer_running() {
        let fx = page(2, "enable", "pending").await;
        exec(
            &fx,
            r#"UPDATE "AIHandoverSettings" SET "enabled" = 0 WHERE "id" = 9001"#,
            vec![],
        )
        .await;
        toggle(&fx, WS).await;
        let run = run_row(&fx, RUN).await;
        assert_eq!(run.status, "cancelled");
        assert_eq!(run.current_error.as_deref(), Some("automationStopped"));
    }

    #[tokio::test]
    async fn a_disable_does_not_need_the_automation_to_keep_running_behind_it() {
        let fx = page(1, "disable", "pending").await;
        exec(
            &fx,
            r#"UPDATE "AIHandoverSettings" SET "enabled" = 0 WHERE "id" = 9001"#,
            vec![],
        )
        .await;
        toggle(&fx, WS).await;
        let run = run_row(&fx, RUN).await;
        // Changed: the TS mock completes; without a Rust thread control the
        // run walks the integration-unavailable path (module doc).
        assert_ne!(run.current_error.as_deref(), Some("automationStopped"));
        assert_eq!(run.status, "failed");
    }

    #[tokio::test]
    async fn the_total_is_counted_once_only_while_it_is_unknown() {
        let fx = page(3, "enable", "pending").await;
        exec(
            &fx,
            r#"UPDATE "AIHandoverBulkRun" SET "totalCount" = 42 WHERE "id" = ?1"#,
            vec![RUN],
        )
        .await;
        toggle(&fx, WS).await;
        // A known total is not recounted; the finish writes what was walked.
        let run = run_row(&fx, RUN).await;
        assert_eq!(run.total_count, Some(3));
    }

    // failures

    #[tokio::test]
    async fn a_page_whose_integration_cannot_be_loaded_fails_the_run_and_counts_its_threads_skipped()
     {
        let fx = page(7, "enable", "pending").await;
        toggle(&fx, WS).await;
        let run = run_row(&fx, RUN).await;
        assert_eq!(run.status, "failed");
        assert_eq!(run.current_error.as_deref(), Some("integrationUnavailable"));
        assert_eq!(run.skipped_count, 7);
        assert_eq!(run.total_count, Some(7));
        assert!(run.claim_token.is_none());
    }

    // accounting that must add up / resuming

    #[tokio::test]
    async fn a_resumed_run_that_hits_an_unusable_page_counts_only_what_is_ahead_of_its_cursor() {
        let fx = page(3, "enable", "pending").await;
        exec(
            &fx,
            r#"UPDATE "AIHandoverBulkRun" SET "processedCount" = 2, "cursorContactInboxId" = '6001',
                 "totalCount" = 3 WHERE "id" = ?1"#,
            vec![RUN],
        )
        .await;
        toggle(&fx, WS).await;
        let run = run_row(&fx, RUN).await;
        assert_eq!(run.processed_count, 2);
        assert_eq!(run.skipped_count, 1);
        assert_eq!(run.total_count, Some(3));
    }

    #[tokio::test]
    async fn a_disable_that_crashed_with_a_batch_in_flight_skips_past_it_never_re_sends() {
        let fx = page(3, "disable", "pending").await;
        exec(
            &fx,
            r#"UPDATE "AIHandoverBulkRun" SET "inFlightFromId" = '6000', "inFlightToId" = '6001',
                 "totalCount" = 3 WHERE "id" = ?1"#,
            vec![RUN],
        )
        .await;
        toggle(&fx, WS).await;
        let run = run_row(&fx, RUN).await;
        assert_eq!(run.cursor_contact_inbox_id.as_deref(), Some("6001"));
        assert!(run.in_flight_from_id.is_none() && run.in_flight_to_id.is_none());
        // Only the thread after the recovered batch is still ahead.
        assert_eq!(run.skipped_count, 1);
    }

    #[tokio::test]
    async fn an_enable_that_crashed_with_a_batch_in_flight_repeats_it_a_pass_is_idempotent() {
        let fx = page(3, "enable", "pending").await;
        exec(
            &fx,
            r#"UPDATE "AIHandoverBulkRun" SET "inFlightFromId" = '6000', "inFlightToId" = '6001',
                 "totalCount" = 3 WHERE "id" = ?1"#,
            vec![RUN],
        )
        .await;
        toggle(&fx, WS).await;
        let run = run_row(&fx, RUN).await;
        assert!(run.cursor_contact_inbox_id.is_none());
        assert!(run.in_flight_to_id.is_none());
        assert_eq!(run.skipped_count, 3);
    }

    // reconciling after a run ends

    #[tokio::test]
    async fn a_failed_run_asks_for_the_pages_next_due_run() {
        let fx = page(1, "enable", "pending").await;
        exec(
            &fx,
            r#"UPDATE "AIHandoverSettings" SET "applyToAllRevision" = 2 WHERE "id" = 9001"#,
            vec![],
        )
        .await;
        toggle(&fx, WS).await;
        assert_eq!(run_row(&fx, RUN).await.status, "failed");
        let chunks = fx.jobs.named("aiHandoverBulkToggle");
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        assert_ne!(chunks[0].data["data"]["runId"], json!(RUN.to_string()));
    }

    // scanAiHandoverBulkRuns

    #[tokio::test]
    async fn terminalizes_exhausted_runs_and_dispatches_each_due_run_as_its_next_chunk() {
        let fx = page(1, "enable", "pending").await;
        fx.exec(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId", "status")
               VALUES (41, 2, 'Page 2', 'messenger', 'page-2', 'connected')"#,
        )
        .await;
        exec(
            &fx,
            r#"INSERT INTO "AIHandoverBulkRun" ("id", "createdAt", "updatedAt", "workspaceId", "inboxId",
                 "channel", "revision", "action", "status", "requestedAt", "attempts", "chunkSeq",
                 "processedCount", "skippedCount", "failedCount", "lastHeartbeatAt", "claimToken")
               VALUES (901, 0, 0, ?1, ?2, 'messenger', 0, 'enable', 'running', 0, 5, 5, 0, 0, 0, 0, 't')"#,
            vec![WS, 41],
        )
        .await;
        scan(&fx.deps).await.unwrap();
        let exhausted = run_row(&fx, 901).await;
        assert_eq!(exhausted.status, "failed");
        assert_eq!(exhausted.current_error.as_deref(), Some("maxAttempts"));
        let chunks = fx.jobs.named("aiHandoverBulkToggle");
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].opts.job_id.as_deref(),
            Some("ai-handover-bulk-900-chunk-2")
        );
        assert_eq!(run_row(&fx, RUN).await.attempts, 2);
    }

    #[tokio::test]
    async fn does_not_dispatch_a_run_whose_previous_chunk_is_still_queued() {
        let fx = page(1, "enable", "pending").await;
        exec(
            &fx,
            r#"INSERT INTO "jobs" ("queue", "name", "job_id", "data", "opts", "state", "ready_seq",
                 "run_at_ms", "created_at_ms")
               VALUES ('integration', 'aiHandoverBulkToggle', 'ai-handover-bulk-900-chunk-1', '{}', '{}',
                 'waiting', 1, 0, 0)"#,
            vec![],
        )
        .await;
        for _ in 0..3 {
            scan(&fx.deps).await.unwrap();
        }
        assert!(fx.jobs.named("aiHandoverBulkToggle").is_empty());
        let run = run_row(&fx, RUN).await;
        assert_eq!((run.attempts, run.chunk_seq), (1, 1));
    }

    #[tokio::test]
    async fn keeps_dispatching_when_one_runs_enqueue_fails() {
        let fx = page(1, "enable", "pending").await;
        fx.exec(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId", "status")
               VALUES (41, 2, 'Page 2', 'messenger', 'page-2', 'connected')"#,
        )
        .await;
        exec(
            &fx,
            r#"INSERT INTO "AIHandoverBulkRun" ("id", "createdAt", "updatedAt", "workspaceId", "inboxId",
                 "channel", "revision", "action", "status", "requestedAt", "attempts", "chunkSeq",
                 "processedCount", "skippedCount", "failedCount")
               VALUES (901, 1, 0, ?1, ?2, 'messenger', 0, 'enable', 'pending', 0, 0, 0, 0, 0, 0)"#,
            vec![WS, 41],
        )
        .await;
        fx.jobs.fail_next();
        scan(&fx.deps).await.unwrap();
        assert_eq!(fx.jobs.named("aiHandoverBulkToggle").len(), 1);
    }

    #[tokio::test]
    async fn reconciles_a_page_owed_a_run_and_keeps_no_cursor_for_a_short_list() {
        let fx = page(1, "enable", "completed").await;
        exec(
            &fx,
            r#"UPDATE "AIHandoverSettings" SET "applyToAllRevision" = 2 WHERE "id" = 9001"#,
            vec![],
        )
        .await;
        scan(&fx.deps).await.unwrap();
        assert_eq!(fx.jobs.named("aiHandoverBulkToggle").len(), 1);
        let kv = DurableKv::new(fx.deps.db.clone(), std::sync::Arc::new(SystemClock));
        assert!(!kv.exists(RECONCILE_CURSOR_KEY).await.unwrap());
    }
}
