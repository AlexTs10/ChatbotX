//! The retention crons of `schedule/handlers/purge-*.ts` whose tables are in
//! Rust. One process runs the schedule queue, so the `distributedLock`
//! wrappers are not needed (see specs/Q.W2.md "Decisions").

use chatbotx_db::Db;
use chatbotx_db::repo::chunked_purge::{
    ChunkedPurgeOptions, ChunkedPurgeStopReason, chunked_purge,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::{
    automation_throttle, comment_automation_event, connect_session, inbound_webhook,
    mobile_refresh_token, now_ms, oauth_access_token, oauth_refresh_token,
    whatsapp_coexist_staging, whatsapp_signup_session,
};
use chatbotx_jobs::JobError;

use crate::{db_fail, repo_err};

const DAY_MS: i64 = 86_400_000;
/// The shared bounds of the 30-day purges.
const CHUNK_SIZE: i64 = 1000;
const INTER_CHUNK_DELAY_MS: u64 = 100;
const MAX_RUN_DURATION_MS: Option<i64> = Some(10 * 60 * 1000);
const MAX_CHUNKS_PER_RUN: i64 = 10_000;

fn report(name: &str, deleted: i64, stop: ChunkedPurgeStopReason) {
    if stop != ChunkedPurgeStopReason::Drained {
        tracing::warn!(deleted, stop_reason = ?stop, "{name}: stopped with a backlog remaining");
    } else if deleted > 0 {
        tracing::info!(deleted, "{name}: rows purged");
    }
}

/// `purgeErrorLogs` (30 days).
pub async fn purge_error_logs(db: &Db) -> Result<(), JobError> {
    let (deleted, stop) = chunked_purge(
        db,
        &ChunkedPurgeOptions {
            table: "ErrorLog".into(),
            filter: Sql::bind("\"createdAt\" < ", now_ms() - 30 * DAY_MS),
            order_by: "createdAt".into(),
            chunk_size: CHUNK_SIZE,
            inter_chunk_delay_ms: INTER_CHUNK_DELAY_MS,
            max_chunks: MAX_CHUNKS_PER_RUN,
            max_run_duration_ms: MAX_RUN_DURATION_MS,
        },
    )
    .await
    .map_err(db_fail)?;
    report("purgeErrorLogs", deleted, stop);
    Ok(())
}

/// `purgeInboundWebhookRequests` (30 days).
pub async fn purge_inbound_webhook_requests(db: &Db) -> Result<(), JobError> {
    let options = inbound_webhook::PurgeInboundWebhookRequestsOptions {
        retention_days: 30,
        chunk_size: CHUNK_SIZE,
        inter_chunk_delay_ms: INTER_CHUNK_DELAY_MS,
        max_chunks: MAX_CHUNKS_PER_RUN,
        max_run_duration_ms: MAX_RUN_DURATION_MS,
    };
    let (deleted, stop) = inbound_webhook::purge_inbound_webhook_requests(db, options, now_ms())
        .await
        .map_err(db_fail)?;
    report("purgeInboundWebhookRequests", deleted, stop);
    Ok(())
}

/// `purgeCommentAutomationEvents` (`COMMENT_AUTOMATION_ERROR_RETENTION_DAYS` = 30).
pub async fn purge_comment_automation_events(db: &Db) -> Result<(), JobError> {
    let options = comment_automation_event::PurgeOptions {
        retention_days: 30,
        chunk_size: CHUNK_SIZE,
        inter_chunk_delay_ms: INTER_CHUNK_DELAY_MS,
        max_chunks: MAX_CHUNKS_PER_RUN,
        max_run_duration_ms: MAX_RUN_DURATION_MS,
    };
    let (deleted, stop) =
        comment_automation_event::purge_failed_comment_automation_events(db, options, now_ms())
            .await
            .map_err(db_fail)?;
    report("purgeCommentAutomationEvents", deleted, stop);
    Ok(())
}

/// `purgeExpiredConnectSessions`: expire the due active sessions, then
/// delete terminal rows past 7 days.
pub async fn purge_expired_connect_sessions(db: &Db) -> Result<(), JobError> {
    let now = now_ms();
    let expired = db
        .write(move |tx| {
            connect_session::expire_due(
                tx,
                now,
                connect_session::ACTIVE_CONNECT_SESSION_STATUSES,
                now,
            )
            .map_err(repo_err)
        })
        .await
        .map_err(db_fail)?;
    if expired > 0 {
        tracing::info!(expired, "purgeExpiredConnectSessions: sessions expired");
    }
    let (deleted, stop) =
        connect_session::purge_old_terminal(db, 7, 500, INTER_CHUNK_DELAY_MS, 1000, None)
            .await
            .map_err(db_fail)?;
    if deleted > 0 {
        tracing::info!(
            deleted_terminal = deleted,
            "purgeExpiredConnectSessions: terminal rows purged"
        );
    }
    if stop != ChunkedPurgeStopReason::Drained {
        tracing::warn!(deleted_terminal = deleted, stop_reason = ?stop, "purgeExpiredConnectSessions: terminal purge stopped with a backlog remaining");
    }
    Ok(())
}

/// `purgeWhatsappSignupSessions`: up to 20 batches of 500.
pub async fn purge_whatsapp_signup_sessions(db: &Db) -> Result<(), JobError> {
    let mut total = 0;
    for _ in 0..20 {
        let deleted = db
            .write(|tx| {
                whatsapp_signup_session::purge_finished_signup_sessions(tx, now_ms(), 500)
                    .map_err(repo_err)
            })
            .await
            .map_err(db_fail)?;
        total += deleted;
        if deleted == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(INTER_CHUNK_DELAY_MS)).await;
    }
    if total > 0 {
        tracing::info!(deleted = total, "purgeWhatsappSignupSessions: rows purged");
    }
    Ok(())
}

/// `purgeCoexistStaging`: processed rows past 48 h, unparseable rows past 7 days.
pub async fn purge_coexist_staging(db: &Db) -> Result<(), JobError> {
    let bounds = whatsapp_coexist_staging::PurgeBounds {
        chunk_size: 500,
        inter_chunk_delay_ms: INTER_CHUNK_DELAY_MS,
        max_chunks: 20,
        max_run_duration_ms: None,
    };
    let (deleted, _) = whatsapp_coexist_staging::purge_processed(db, 48, now_ms(), bounds)
        .await
        .map_err(db_fail)?;
    if deleted > 0 {
        tracing::info!(deleted, "purgeCoexistStaging: rows purged");
    }
    let (deleted, _) = whatsapp_coexist_staging::purge_parse_failed(db, 7, now_ms(), bounds)
        .await
        .map_err(db_fail)?;
    if deleted > 0 {
        tracing::info!(deleted, "purgeCoexistStaging: unparseable rows purged");
    }
    Ok(())
}

/// `purgeExpiredOAuthTokens`: access, refresh and mobile refresh tokens.
pub async fn purge_expired_oauth_tokens(db: &Db) -> Result<(), JobError> {
    let now = now_ms();
    let deleted = db
        .write(move |tx| {
            Ok(
                oauth_access_token::delete_expired(tx, now).map_err(repo_err)?
                    + oauth_refresh_token::delete_expired(tx, now).map_err(repo_err)?
                    + mobile_refresh_token::delete_expired(tx, now).map_err(repo_err)?,
            )
        })
        .await
        .map_err(db_fail)?;
    if deleted > 0 {
        tracing::info!(deleted, "purgeExpiredOAuthTokens: rows purged");
    }
    Ok(())
}

/// `purgeAutomationThrottle` (`STALE_RETENTION_HOURS` = 48).
pub async fn purge_automation_throttle(db: &Db) -> Result<(), JobError> {
    let deleted = db
        .write(|tx| {
            automation_throttle::purge_stale_automation_throttles(tx, 48, now_ms())
                .map_err(repo_err)
        })
        .await
        .map_err(db_fail)?;
    if deleted > 0 {
        tracing::info!(deleted, "purgeAutomationThrottle: rows purged");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::test_support::Fixture;

    #[tokio::test]
    async fn purges_only_rows_past_retention() {
        let f = Fixture::new().await;
        let old = now_ms() - 31 * DAY_MS;
        let recent = now_ms() - DAY_MS;
        f.sql(move |tx| {
            for (id, at) in [(1, old), (2, recent)] {
                tx.execute(
                    r#"INSERT INTO "ErrorLog" ("id", "createdAt", "updatedAt", "action", "detail", "workspaceId") VALUES (?1, ?2, ?2, 'openai', 'x', 2)"#,
                    (id, at),
                )?;
            }
            Ok(())
        })
        .await;
        purge_error_logs(&f.deps.db).await.unwrap();
        assert_eq!(
            f.int(r#"SELECT MIN("id") * 10 + COUNT(*) FROM "ErrorLog""#)
                .await,
            Some(21)
        );
        // the other purges run their SQL against the schema
        let db = &f.deps.db;
        purge_inbound_webhook_requests(db).await.unwrap();
        purge_comment_automation_events(db).await.unwrap();
        purge_expired_connect_sessions(db).await.unwrap();
        purge_whatsapp_signup_sessions(db).await.unwrap();
        purge_coexist_staging(db).await.unwrap();
        purge_expired_oauth_tokens(db).await.unwrap();
        purge_automation_throttle(db).await.unwrap();
    }
}
