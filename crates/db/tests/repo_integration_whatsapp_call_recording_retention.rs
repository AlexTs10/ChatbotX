//! Port of `packages/database/__tests__/integration/whatsapp-call-recording-retention.test.ts`
//! (P3.3): a fresh migrated SQLite file per test instead of a rolled-back transaction.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo6.rs"]
mod repo6;

use chatbotx_db::DbError;
use chatbotx_db::repo;
use chatbotx_db::repo::whatsapp_call::list_recordings_past_retention;
use chatbotx_db::rusqlite::Connection;
use repo_common::{ms, ok, setup};
use repo6::*;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn now() -> i64 {
    ms("2026-09-24T05:00:00.000Z")
}

/// One Inbox + IntegrationWhatsapp (`retention_days`) + ContactInbox of the seeded contact.
/// Returns the (inbox id, contact inbox id).
fn seed_inbox(tx: &Connection, retention_days: i64) -> Result<(i64, i64), DbError> {
    let inbox = 3100 + retention_days;
    let ci = 3300 + retention_days;
    ins!(tx, "Inbox", {
        "id" => inbox,
        "name" => format!("retention-{retention_days}d"),
        "channel" => "whatsapp".to_owned(),
        "sourceId" => format!("phone-{retention_days}"),
        "workspaceId" => 2001_i64,
    })?;
    ins!(tx, "IntegrationWhatsapp", {
        "id" => 9000 + retention_days,
        "auth" => "{}".to_owned(),
        "phoneNumberId" => format!("phone-{retention_days}"),
        "wabaId" => "waba-1".to_owned(),
        "businessId" => "business-1".to_owned(),
        "name" => format!("retention-{retention_days}d"),
        "workspaceId" => 2001_i64,
        "inboxId" => inbox,
        "callRecordingRetentionDays" => retention_days,
    })?;
    ins!(tx, "ContactInbox", {
        "id" => ci,
        "originalContactId" => 4001_i64,
        "contactId" => 4001_i64,
        "inboxId" => inbox,
        "channel" => "whatsapp".to_owned(),
        "source" => "whatsapp".to_owned(),
        "sourceId" => format!("wa-{retention_days}"),
    })?;
    Ok((inbox, ci))
}

/// A call on `(inbox, ci)` recorded `days_ago` days before `now()` (purged = no recording).
fn seed_call(
    tx: &Connection,
    id: i64,
    at: (i64, i64),
    days_ago: i64,
    purged: bool,
) -> Result<i64, DbError> {
    let recorded = now() - days_ago * DAY_MS;
    put_call(tx, id, recorded, "")?;
    tx.execute(
        "UPDATE \"WhatsappCall\" SET \"inboxId\" = ?2, \"contactInboxId\" = ?3, \"recordingPath\" = ?4, \
         \"recordedAt\" = ?5 WHERE \"id\" = ?1",
        chatbotx_db::rusqlite::params![
            id,
            at.0,
            at.1,
            (!purged).then_some("space/calls/rec.ogg"),
            (!purged).then_some(recorded)
        ],
    )?;
    Ok(id)
}

/// `seedRetentionFixture`: (expired ids, kept ids).
async fn seed_retention_fixture() -> (common::TestDb, Vec<i64>, Vec<i64>) {
    let (db, _) = call_db().await;
    let (expired, kept) = setup(&db, |tx| {
        let short = seed_inbox(tx, 30)?;
        let long = seed_inbox(tx, 90)?;
        let expired = vec![
            seed_call(tx, 11, short, 45, false)?,
            seed_call(tx, 12, long, 100, false)?,
        ];
        let kept = vec![
            seed_call(tx, 13, short, 10, false)?,
            seed_call(tx, 14, long, 45, false)?,
            seed_call(tx, 15, long, 100, true)?,
        ];
        Ok((expired, kept))
    })
    .await;
    (db, expired, kept)
}

/// Port of: packages/database/__tests__/integration/whatsapp-call-recording-retention.test.ts:202 "whatsappCallRepository.listRecordingsPastRetention against Postgres > returns recordings past their own inbox's retention and keeps the rest"
#[tokio::test]
async fn whatsapp_call_repository_list_recordings_past_retention_against_postgres_returns_recordings_past_their_own_inbox_s_retention_and_keeps_the_rest()
 {
    let (db, expired, kept) = seed_retention_fixture().await;
    let mut returned: Vec<i64> =
        ok(repo::read(&db, |c| list_recordings_past_retention(c, 100, now())).await)
            .iter()
            .map(|c| c.id)
            .collect();
    returned.sort_unstable();
    assert_eq!(returned, expired);
    assert!(kept.iter().all(|id| !returned.contains(id)));
}

/// Port of: packages/database/__tests__/integration/whatsapp-call-recording-retention.test.ts:219 "whatsappCallRepository.listRecordingsPastRetention against Postgres > bounds a page by limit"
#[tokio::test]
async fn whatsapp_call_repository_list_recordings_past_retention_against_postgres_bounds_a_page_by_limit()
 {
    let (db, _, _) = seed_retention_fixture().await;
    let rows = ok(repo::read(&db, |c| list_recordings_past_retention(c, 1, now())).await);
    assert_eq!(rows.len(), 1);
}
