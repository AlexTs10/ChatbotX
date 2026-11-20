//! The contact profile snapshot queue (`contactInboxService` claim /
//! reschedule / complete / list, `captureContactProfileSnapshot`,
//! `dispatchProfileSnapshots`, `enqueueProfileSnapshotJobs`,
//! `refreshProfileSnapshot`): Instagram relationship fields captured once
//! per contact inbox, with a lease, a retry budget and a recovery sweep.

use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::enqueue::Enqueuer;
use chatbotx_jobs::{JobOptions, QueueName, Retention};
use chatbotx_kv::DurableKv;
use serde_json::json;

use super::channel_profile::{ProfileError, ProfileSnapshot, fetch_profile_snapshot};
use crate::channel_api::ChannelHttp;
use crate::{BusinessError, Result, tx};

pub const PROFILE_SNAPSHOT_MAX_ATTEMPTS: i64 = 5;
pub const PROFILE_SNAPSHOT_CLAIM_LEASE_MS: i64 = 10 * 60 * 1000;
pub const PROFILE_SNAPSHOT_INITIAL_RETRY_MS: i64 = 30 * 1000;
pub const PROFILE_SNAPSHOT_MAX_RETRY_MS: i64 = 30 * 60 * 1000;
/// `profileSnapshotChannels`.
pub const PROFILE_SNAPSHOT_CHANNELS: &[&str] = &["instagram"];
/// `ProfileSnapshotJobAction.capture`.
pub const CAPTURE: &str = "capture";
const PAGE_SIZE: usize = 100;
const MAX_PAGES_PER_RUN: usize = 20;
const RECOVERY_CURSOR_TTL_SECONDS: u64 = 60 * 60 * 24;
pub const DUE_CURSOR_KEY: &str = "schedule:profile-snapshots:due-cursor";
pub const EXHAUSTED_CURSOR_KEY: &str = "schedule:profile-snapshots:exhausted-cursor";

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `supportsProfileSnapshot`.
pub fn supports_profile_snapshot(channel: &str) -> bool {
    PROFILE_SNAPSHOT_CHANNELS.contains(&channel)
}

/// `profileSnapshotJobId`.
pub fn profile_snapshot_job_id(contact_inbox_id: i64) -> String {
    format!("profile-snapshot-{contact_inbox_id}")
}

/// One snapshot job's ids (`ProfileSnapshotJobData["data"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotTarget {
    pub contact_inbox_id: i64,
    pub inbox_id: i64,
    pub workspace_id: i64,
}

/// A won claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub attempt: i64,
    pub channel: String,
    pub source_id: String,
}

/// `claimProfileSnapshot`: a due pending row of a snapshot channel gets
/// `attempts + 1` and a lease (CAS on the attempt count).
pub async fn claim(db: &Db, target: SnapshotTarget) -> Result<Option<Claim>> {
    let now = repo::now_ms();
    tx::write(db, move |tx| {
        if !repo::workspace::lock_workspace_for_child_write(tx, target.workspace_id)
            .map_err(BusinessError::Repo)?
        {
            return Ok(None);
        }
        let candidate: Option<(Option<i64>, String, String)> = tx
            .query_row(
                r#"SELECT ci."profileSnapshotAttempts", i."channel", ci."sourceId"
                   FROM "ContactInbox" ci JOIN "Inbox" i ON i."id" = ci."inboxId"
                   WHERE ci."id" = ?1 AND ci."inboxId" = ?2 AND i."workspaceId" = ?3
                     AND i."channel" = 'instagram' AND ci."profileSnapshotState" = 'pending'
                     AND ci."profileSnapshotNextAttemptAt" <= ?4
                     AND COALESCE(ci."profileSnapshotAttempts", 0) < ?5
                   LIMIT 1"#,
                params![
                    target.contact_inbox_id,
                    target.inbox_id,
                    target.workspace_id,
                    now,
                    PROFILE_SNAPSHOT_MAX_ATTEMPTS
                ],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(repo_err)?;
        let Some((attempts, channel, source_id)) = candidate else {
            return Ok(None);
        };
        let previous = attempts.unwrap_or(0);
        let attempt = previous + 1;
        let claimed = tx
            .execute(
                r#"UPDATE "ContactInbox" SET "profileSnapshotAttempts" = ?2, "profileSnapshotNextAttemptAt" = ?3
                   WHERE "id" = ?1 AND "profileSnapshotState" = 'pending'
                     AND COALESCE("profileSnapshotAttempts", 0) = ?4"#,
                params![
                    target.contact_inbox_id,
                    attempt,
                    now + PROFILE_SNAPSHOT_CLAIM_LEASE_MS,
                    previous
                ],
            )
            .map_err(repo_err)?;
        Ok((claimed > 0).then_some(Claim {
            attempt,
            channel,
            source_id,
        }))
    })
    .await
}

/// `rescheduleProfileSnapshot`: back to `pending` after
/// `min(30 s · 2^(attempt-1), 30 min)`, or `failed` once the budget is spent.
/// The new state, `None` when the row moved on.
pub async fn reschedule(db: &Db, target: SnapshotTarget, attempt: i64) -> Result<Option<String>> {
    let exhausted = attempt >= PROFILE_SNAPSHOT_MAX_ATTEMPTS;
    let delay = PROFILE_SNAPSHOT_INITIAL_RETRY_MS
        .saturating_mul(1_i64 << (attempt - 1).clamp(0, 30))
        .min(PROFILE_SNAPSHOT_MAX_RETRY_MS);
    let next = (!exhausted).then(|| repo::now_ms() + delay);
    let state = if exhausted { "failed" } else { "pending" };
    tx::write(db, move |tx| {
        if !repo::workspace::lock_workspace_for_child_write(tx, target.workspace_id)
            .map_err(BusinessError::Repo)?
        {
            return Ok(None);
        }
        tx.query_row(
            r#"UPDATE "ContactInbox" SET "profileSnapshotNextAttemptAt" = ?3, "profileSnapshotState" = ?4
               WHERE "id" = ?1 AND "inboxId" = ?2 AND "profileSnapshotState" = 'pending'
                 AND "profileSnapshotAttempts" = ?5
                 AND EXISTS (SELECT 1 FROM "Inbox" WHERE "id" = ?2 AND "workspaceId" = ?6)
               RETURNING "profileSnapshotState""#,
            params![
                target.contact_inbox_id,
                target.inbox_id,
                next,
                state,
                attempt,
                target.workspace_id
            ],
            |r| r.get(0),
        )
        .optional()
        .map_err(repo_err)
    })
    .await
}

/// `completeProfileSnapshot`: the outcome (`captured` / `unavailable` /
/// `failed`); the snapshot values land only while all four are still null.
pub async fn complete(
    db: &Db,
    target: SnapshotTarget,
    attempt: i64,
    outcome: &'static str,
    snapshot: &ProfileSnapshot,
    only_if_lease_expired: bool,
) -> Result<bool> {
    let now = repo::now_ms();
    let snapshot = snapshot.clone();
    tx::write(db, move |tx| {
        if !repo::workspace::lock_workspace_for_child_write(tx, target.workspace_id)
            .map_err(BusinessError::Repo)?
        {
            return Ok(false);
        }
        let lease = if only_if_lease_expired {
            r#" AND "profileSnapshotNextAttemptAt" <= ?10"#
        } else {
            " AND ?10 IS NOT NULL"
        };
        let sql = format!(
            r#"UPDATE "ContactInbox" SET
                 "followsBusiness" = CASE WHEN {empty} THEN ?3 ELSE "followsBusiness" END,
                 "businessFollowsContact" = CASE WHEN {empty} THEN ?4 ELSE "businessFollowsContact" END,
                 "accountVerified" = CASE WHEN {empty} THEN ?5 ELSE "accountVerified" END,
                 "followerCount" = CASE WHEN {empty} THEN ?6 ELSE "followerCount" END,
                 "profileSnapshotNextAttemptAt" = NULL, "profileSnapshotState" = ?7
               WHERE "id" = ?1 AND "inboxId" = ?2 AND "profileSnapshotState" = 'pending'
                 AND "profileSnapshotAttempts" = ?8
                 AND EXISTS (SELECT 1 FROM "Inbox" WHERE "id" = "ContactInbox"."inboxId" AND "workspaceId" = ?9)
{lease}"#,
            empty = r#"("followsBusiness" IS NULL AND "businessFollowsContact" IS NULL
                        AND "accountVerified" IS NULL AND "followerCount" IS NULL)"#,
        );
        let changed = tx
            .execute(
                &sql,
                params![
                    target.contact_inbox_id,
                    target.inbox_id,
                    snapshot.follows_business,
                    snapshot.business_follows_contact,
                    snapshot.account_verified,
                    snapshot.follower_count,
                    outcome,
                    attempt,
                    target.workspace_id,
                    now
                ],
            )
            .map_err(repo_err)?;
        Ok(changed > 0)
    })
    .await
}

/// `refreshProfileSnapshot` (a live fetch for the system fields): a non-null
/// value replaces the stored one; `username` only fills a missing
/// `sourceUsername`.
pub async fn refresh_profile_snapshot(
    db: &Db,
    contact_inbox_id: i64,
    inbox_id: i64,
    snapshot: &ProfileSnapshot,
) -> Result<()> {
    let username = snapshot.username.clone().filter(|u| !u.is_empty());
    if username.is_none()
        && snapshot.follows_business.is_none()
        && snapshot.business_follows_contact.is_none()
        && snapshot.account_verified.is_none()
        && snapshot.follower_count.is_none()
    {
        return Ok(());
    }
    let snapshot = snapshot.clone();
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "ContactInbox" SET
                 "sourceUsername" = COALESCE("sourceUsername", ?3),
                 "followsBusiness" = COALESCE(?4, "followsBusiness"),
                 "businessFollowsContact" = COALESCE(?5, "businessFollowsContact"),
                 "accountVerified" = COALESCE(?6, "accountVerified"),
                 "followerCount" = COALESCE(?7, "followerCount")
               WHERE "id" = ?1 AND "inboxId" = ?2"#,
            params![
                contact_inbox_id,
                inbox_id,
                username,
                snapshot.follows_business,
                snapshot.business_follows_contact,
                snapshot.account_verified,
                snapshot.follower_count
            ],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}

/// A recovery row of [`list_due`] / [`list_exhausted`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRow {
    pub target: SnapshotTarget,
    pub attempt: Option<i64>,
    pub next_attempt_at: Option<i64>,
}

/// `serializeProfileSnapshotCursor`: `{iso nextAttemptAt}|{id}`.
pub fn serialize_cursor(row: &RecoveryRow) -> Option<String> {
    let at = chatbotx_core::js::to_iso_string(row.next_attempt_at?)?;
    Some(format!("{at}|{}", row.target.contact_inbox_id))
}

/// `parseProfileSnapshotCursor`.
pub fn parse_cursor(cursor: Option<&str>) -> Option<(i64, i64)> {
    let (at, id) = cursor?.split_once('|')?;
    Some((chatbotx_core::js::parse_date(at)?, id.parse().ok()?))
}

async fn list(
    db: &Db,
    cursor: Option<&str>,
    limit: usize,
    exhausted: bool,
) -> Result<Vec<RecoveryRow>> {
    let cursor = parse_cursor(cursor);
    let now = repo::now_ms();
    let budget = if exhausted { ">=" } else { "<" };
    let after = if cursor.is_some() {
        r#" AND (ci."profileSnapshotNextAttemptAt", ci."id") > (?3, ?4)"#
    } else {
        " AND ?3 IS NULL AND ?4 IS NULL"
    };
    let sql = format!(
        r#"SELECT ci."id", i."id", i."workspaceId", ci."profileSnapshotAttempts", ci."profileSnapshotNextAttemptAt"
           FROM "ContactInbox" ci
           JOIN "Inbox" i ON i."id" = ci."inboxId"
           JOIN "Workspace" w ON w."id" = i."workspaceId"
           WHERE i."channel" = 'instagram' AND w."purgeStartedAt" IS NULL
             AND w."scheduledDeletionAt" IS NULL AND ci."profileSnapshotState" = 'pending'
             AND ci."profileSnapshotNextAttemptAt" <= ?1
             AND COALESCE(ci."profileSnapshotAttempts", 0) {budget} ?2{after}
           ORDER BY ci."profileSnapshotNextAttemptAt", ci."id" LIMIT {limit}"#
    );
    let rows = tx::read(db, move |conn| {
        let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
        stmt.query_map(
            params![
                now,
                PROFILE_SNAPSHOT_MAX_ATTEMPTS,
                cursor.map(|c| c.0),
                cursor.map(|c| c.1)
            ],
            |r| {
                Ok(RecoveryRow {
                    target: SnapshotTarget {
                        contact_inbox_id: r.get(0)?,
                        inbox_id: r.get(1)?,
                        workspace_id: r.get(2)?,
                    },
                    attempt: r.get(3)?,
                    next_attempt_at: r.get(4)?,
                })
            },
        )
        .and_then(Iterator::collect::<std::result::Result<Vec<_>, _>>)
        .map_err(repo_err)
    })
    .await?;
    Ok(if exhausted {
        rows.into_iter().filter(|r| r.attempt.is_some()).collect()
    } else {
        rows
    })
}

/// `listDueProfileSnapshots`.
pub async fn list_due(db: &Db, cursor: Option<&str>, limit: usize) -> Result<Vec<RecoveryRow>> {
    list(db, cursor, limit, false).await
}

/// `listExhaustedProfileSnapshots`.
pub async fn list_exhausted(
    db: &Db,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Vec<RecoveryRow>> {
    list(db, cursor, limit, true).await
}

/// `enqueueProfileSnapshotJobs`: one `capture` job per contact inbox (job id
/// `profile-snapshot-<id>`, one attempt, removed when done).
pub async fn enqueue_profile_snapshot_jobs(
    jobs: &dyn Enqueuer,
    contact_inbox_ids: &[i64],
    inbox_id: i64,
    workspace_id: i64,
) -> std::result::Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for &id in contact_inbox_ids.iter().filter(|id| seen.insert(**id)) {
        let data = json!({
            "type": CAPTURE,
            "data": {
                "contactInboxId": id.to_string(),
                "inboxId": inbox_id.to_string(),
                "workspaceId": workspace_id.to_string(),
            },
        });
        let opts = JobOptions {
            attempts: Some(1),
            job_id: Some(profile_snapshot_job_id(id)),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::Remove),
            ..JobOptions::default()
        };
        jobs.add(QueueName::ProfileSnapshot, CAPTURE, data, opts)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

const EMPTY: ProfileSnapshot = ProfileSnapshot {
    follows_business: None,
    business_follows_contact: None,
    account_verified: None,
    follower_count: None,
    username: None,
};

/// `snapshotOutcomeForError` (Meta Graph policy): no integration → `unavailable`;
/// a transport error, 429, 5xx or code 4 / 17 / 613 → `retry`; code 230 or
/// 100 / 33 → `unavailable`; else `failed`.
fn outcome_for_error(error: &ProfileError) -> &'static str {
    let ProfileError::Provider(failure) = error else {
        return "unavailable";
    };
    let Some(status) = failure.status else {
        return "retry";
    };
    if failure.code == Some(230) || (failure.code == Some(100) && failure.sub_code == Some(33)) {
        return "unavailable";
    }
    if status == 429 || status >= 500 || failure.code.is_some_and(|c| [4, 17, 613].contains(&c)) {
        return "retry";
    }
    "failed"
}

/// `captureContactProfileSnapshot`.
pub async fn capture(db: &Db, http: &dyn ChannelHttp, target: SnapshotTarget) -> Result<()> {
    let Some(claim) = claim(db, target).await? else {
        return Ok(());
    };
    if !supports_profile_snapshot(&claim.channel) {
        complete(db, target, claim.attempt, "unavailable", &EMPTY, false).await?;
        return Ok(());
    }
    let fetched = fetch_profile_snapshot(
        db,
        http,
        target.workspace_id,
        &claim.channel,
        target.inbox_id,
        &claim.source_id,
    )
    .await
    .and_then(|s| s.ok_or_else(|| ProfileError::Integration("unsupported_channel".into())));
    let error = match fetched {
        Ok(snapshot) => {
            complete(db, target, claim.attempt, "captured", &snapshot, false).await?;
            return Ok(());
        }
        Err(error) => error,
    };
    let outcome = outcome_for_error(&error);
    if outcome == "retry" {
        if reschedule(db, target, claim.attempt).await?.as_deref() == Some("failed") {
            tracing::error!(err = %error, ?target, attempt = claim.attempt, "Profile snapshot retry budget exhausted");
        }
        return Ok(());
    }
    if complete(db, target, claim.attempt, outcome, &EMPTY, false).await? {
        if outcome == "failed" {
            tracing::error!(err = %error, ?target, attempt = claim.attempt, outcome, "Profile snapshot completed without profile fields");
        } else {
            tracing::warn!(err = %error, ?target, attempt = claim.attempt, outcome, "Profile snapshot completed without profile fields");
        }
    }
    Ok(())
}

/// `dispatchProfileSnapshots`: the exhausted rows whose lease ran out become
/// `failed`, then the due rows are enqueued again; each walk resumes from its
/// cursor (up to 20 pages of 100 per run). Blocked workspaces are skipped.
pub async fn dispatch(db: &Db, jobs: &dyn Enqueuer, kv: &DurableKv) -> Result<()> {
    walk(db, kv, EXHAUSTED_CURSOR_KEY, true, |row| async move {
        let attempt = row.attempt.unwrap_or_default();
        if complete(db, row.target, attempt, "failed", &EMPTY, true).await? {
            tracing::error!(target = ?row.target, outcome = "failed", reason = "retryExhausted", "Profile snapshot retry budget exhausted during recovery");
        }
        Ok(())
    })
    .await?;
    walk(db, kv, DUE_CURSOR_KEY, false, |row| async move {
        let t = row.target;
        if let Err(error) =
            enqueue_profile_snapshot_jobs(jobs, &[t.contact_inbox_id], t.inbox_id, t.workspace_id).await
        {
            tracing::warn!(%error, target = ?t, "Profile snapshot recovery enqueue failed; intent remains pending");
        }
        Ok(())
    })
    .await
}

/// `recoverSnapshotPages`.
async fn walk<F, Fut>(db: &Db, kv: &DurableKv, key: &str, exhausted: bool, process: F) -> Result<()>
where
    F: Fn(RecoveryRow) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let kv_err = |e: chatbotx_kv::KvError| BusinessError::exception(e.to_string(), "kvError", 500);
    let mut cursor: Option<String> = kv.get(key).await.map_err(kv_err)?;
    for _ in 0..MAX_PAGES_PER_RUN {
        let rows = list(db, cursor.as_deref(), PAGE_SIZE, exhausted).await?;
        if rows.is_empty() {
            return kv.delete(&[key]).await.map_err(kv_err);
        }
        for row in &rows {
            if crate::broadcast::send::is_blocked_workspace(db, row.target.workspace_id).await? {
                continue;
            }
            process(row.clone()).await?;
        }
        if rows.len() < PAGE_SIZE {
            return kv.delete(&[key]).await.map_err(kv_err);
        }
        cursor = rows.last().and_then(serialize_cursor);
        let Some(next) = &cursor else {
            return Ok(());
        };
        kv.put(key, next, Some(RECOVERY_CURSOR_TTL_SECONDS))
            .await
            .map_err(kv_err)?;
    }
    Ok(())
}
