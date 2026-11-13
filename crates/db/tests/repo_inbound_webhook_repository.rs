//! Port of `packages/database/__tests__/inbound-webhook-repository.test.ts` (P3.2b).
//! The TS runs Drizzle over a fake `pg` client and pins the statement text and params;
//! here the statements run on a migrated temp file (adapted) and the rows show the atomic
//! claim, the claim-token fence of every worker write, the workspace scoping and the
//! summary-only list rows (the list row types have no payload / token / config fields).
//! The `integrationWhatsappRepository` and `contactInboxRepository` cases of the file are
//! ported here too. "Now" is the fixed [`NOW`].

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::chunked_purge::ChunkedPurgeStopReason;
use chatbotx_db::repo::contact_inbox::{self as ci, ContactAndInbox, IdentityField};
use chatbotx_db::repo::inbound_webhook::{
    self as ib, InboundWebhookIntake, InboundWebhookRequestCompletion,
    InboundWebhookRequestProgress, ListInboundWebhookRequestsInput,
    PurgeInboundWebhookRequestsOptions,
};
use chatbotx_db::repo::integration_whatsapp as iw;
use chatbotx_db::repo::models::InboundWebhookRequest;
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value;
use common::{fresh_db, seed, seed_workspace};
use repo_common::{NOW, insert_ci, insert_inbox, int, ok, scalar_i64, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
const HOOK: i64 = 301;
const MIN: i64 = 60_000;
const DAY: i64 = 86_400_000;
/// `leaseExpiredBefore` / `receivedBefore` / `createdAfter` of the sweeps.
const LEASE: i64 = NOW - 15 * MIN;
const RECEIVED: i64 = NOW - 2 * MIN;
const CREATED_AFTER: i64 = NOW - DAY;
/// `updatedAt` of webhook 301.
const HOOK_UPDATED: i64 = NOW - 2 * DAY;

/// Webhooks: 301 "Orders" (WS, configured, token `iwh_abc`), 302 "Leads" (WS, not
/// configured, newer), 401 "Foreign" (OTHER, inactive, token `iwh_xyz`).
async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER).await;
    setup(&db, |tx| {
        for (id, ws, name, active, token, resolution, at) in [
            (
                HOOK,
                WS,
                "Orders",
                true,
                "iwh_abc",
                Some("{\"mode\":\"byChannel\"}"),
                NOW - 2 * DAY,
            ),
            (302, WS, "Leads", true, "iwh_def", None, NOW - DAY),
            (401, OTHER, "Foreign", false, "iwh_xyz", None, NOW - DAY),
        ] {
            ins!(tx, "InboundWebhook", {
                "id" => id, "workspaceId" => ws, "name" => name.to_owned(), "active" => active,
                "token" => token.to_owned(), "inboxResolution" => resolution.map(str::to_owned),
                "identifiers" => "[]".to_owned(), "fieldMappings" => "[]".to_owned(),
                "actions" => "[]".to_owned(), "sampleJson" => "{\"big\":true}".to_owned(),
                "createdAt" => at, "updatedAt" => at,
            })?;
        }
        Ok(())
    })
    .await;
    db
}

/// Seeds one request of webhook 301 (WS).
fn req(
    tx: &Connection,
    id: i64,
    status: &str,
    created_at: i64,
    claimed_at: Option<i64>,
    claim_token: Option<&str>,
) -> Result<usize, chatbotx_db::DbError> {
    ins!(tx, "InboundWebhookRequest", {
        "id" => id, "workspaceId" => WS, "inboundWebhookId" => HOOK, "status" => status.to_owned(),
        "createdAt" => created_at, "claimedAt" => claimed_at,
        "claimToken" => claim_token.map(str::to_owned),
        "attempts" => i64::from(claimed_at.is_some()), "payload" => "{\"a\":1}".to_owned(),
    })
}

fn run<T: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&Connection) -> repo::Result<T> + Send + 'static,
) -> impl std::future::Future<Output = T> + '_ {
    async move { ok(repo::read(db, f).await) }
}

fn write<T: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&mut chatbotx_db::WriteTx<'_>) -> repo::Result<T> + Send + 'static,
) -> impl std::future::Future<Output = T> + '_ {
    async move { ok(repo::write(db, f).await) }
}

async fn request(db: &Db, id: i64) -> Option<InboundWebhookRequest> {
    run(db, move |c| {
        repo::find_one::<InboundWebhookRequest>(c, "\"id\" = ?1", &[&id])
    })
    .await
}

async fn hook_counter(db: &Db) -> (Option<i64>, Option<i64>) {
    let sql = |col: &str| format!("SELECT \"{col}\" FROM \"InboundWebhook\" WHERE \"id\" = ?1");
    (
        scalar_i64(db, &sql("requestCount"), vec![int(HOOK)]).await,
        scalar_i64(db, &sql("updatedAt"), vec![int(HOOK)]).await,
    )
}

// ---------------------------------------------------------------------------
// claimInboundWebhookRequest
// ---------------------------------------------------------------------------

#[tokio::test]
async fn claim_inbound_webhook_request_claims_a_received_row_or_a_processing_row_whose_lease_expired_in_one_update()
 {
    let db = fixture().await;
    setup(&db, |tx| {
        req(tx, 1, "received", NOW - MIN, None, None)?;
        req(
            tx,
            2,
            "processing",
            NOW - 30 * MIN,
            Some(LEASE - 1),
            Some("old"),
        )?;
        req(
            tx,
            3,
            "processing",
            NOW - 30 * MIN,
            Some(LEASE + 1),
            Some("live"),
        )?;
        req(tx, 4, "processed", NOW - 30 * MIN, None, None)
    })
    .await;
    let claim = |id: i64| {
        write(&db, move |c| {
            ib::claim_inbound_webhook_request(c, id, "token-a", LEASE, NOW)
        })
    };
    let first = claim(1).await.expect("a received row is claimed");
    assert_eq!(first.status, "processing");
    assert_eq!(first.claim_token.as_deref(), Some("token-a"));
    assert_eq!(first.claimed_at, Some(NOW));
    assert_eq!(first.attempts, 1);
    let expired = claim(2).await.expect("an expired lease is reclaimed");
    assert_eq!(expired.claim_token.as_deref(), Some("token-a"));
    assert_eq!(expired.attempts, 2);
    // A live lease, a finished row, and a row someone already owns stay with their owner.
    assert_eq!(claim(3).await, None);
    assert_eq!(claim(4).await, None);
    assert_eq!(claim(1).await, None);
    assert_eq!(
        request(&db, 3).await.and_then(|r| r.claim_token),
        Some("live".to_owned())
    );
}

// ---------------------------------------------------------------------------
// fenced worker writes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Op {
    Checkpoint,
    Release,
    Complete,
}

fn apply(op: Op, c: &Connection, id: i64, token: &str) -> repo::Result<bool> {
    match op {
        Op::Checkpoint => ib::checkpoint_inbound_webhook_request(
            c,
            id,
            token,
            &InboundWebhookRequestProgress {
                contact_id: Some(Some(4001)),
                ..InboundWebhookRequestProgress::default()
            },
            NOW,
        ),
        Op::Release => ib::release_inbound_webhook_request(c, id, token),
        Op::Complete => ib::complete_inbound_webhook_request(
            c,
            id,
            token,
            &InboundWebhookRequestCompletion {
                status: "processed".to_owned(),
                ..InboundWebhookRequestCompletion::default()
            },
            NOW,
        ),
    }
}

/// Row 1 is `processing` under `token-a`; row 5 is `processed` and still carries
/// `token-a`. Only row 1 is written.
async fn only_touches_owned_row(op: Op) {
    let db = fixture().await;
    setup(&db, |tx| {
        req(
            tx,
            1,
            "processing",
            NOW - 5 * MIN,
            Some(NOW - MIN),
            Some("token-a"),
        )?;
        req(
            tx,
            5,
            "processed",
            NOW - 5 * MIN,
            Some(NOW - MIN),
            Some("token-a"),
        )
    })
    .await;
    let before5 = request(&db, 5).await;
    assert!(write(&db, move |c| apply(op, c, 1, "token-a")).await);
    assert!(!write(&db, move |c| apply(op, c, 5, "token-a")).await);
    assert_eq!(request(&db, 5).await, before5);
    let row = request(&db, 1).await.expect("row 1");
    match op {
        Op::Checkpoint => {
            assert_eq!(row.status, "processing");
            assert_eq!(row.contact_id, Some(4001));
            assert_eq!(row.claimed_at, Some(NOW));
        }
        Op::Release => {
            assert_eq!(row.status, "received");
            assert_eq!(row.claim_token, None);
        }
        Op::Complete => {
            assert_eq!(row.status, "processed");
            assert_eq!(row.processed_at, Some(NOW));
        }
    }
}

#[tokio::test]
async fn fenced_worker_writes_checkpoint_only_touches_a_processing_row_owned_by_the_claim_token() {
    only_touches_owned_row(Op::Checkpoint).await;
}

#[tokio::test]
async fn fenced_worker_writes_release_only_touches_a_processing_row_owned_by_the_claim_token() {
    only_touches_owned_row(Op::Release).await;
}

#[tokio::test]
async fn fenced_worker_writes_complete_only_touches_a_processing_row_owned_by_the_claim_token() {
    only_touches_owned_row(Op::Complete).await;
}

async fn reports_false_when_lost(op: Op) {
    let db = fixture().await;
    setup(&db, |tx| {
        req(
            tx,
            1,
            "processing",
            NOW - 5 * MIN,
            Some(NOW - MIN),
            Some("token-a"),
        )
    })
    .await;
    let before = request(&db, 1).await;
    assert!(!write(&db, move |c| apply(op, c, 1, "stale")).await);
    assert_eq!(request(&db, 1).await, before);
}

#[tokio::test]
async fn fenced_worker_writes_checkpoint_reports_false_when_the_claim_was_lost() {
    reports_false_when_lost(Op::Checkpoint).await;
}

#[tokio::test]
async fn fenced_worker_writes_release_reports_false_when_the_claim_was_lost() {
    reports_false_when_lost(Op::Release).await;
}

#[tokio::test]
async fn fenced_worker_writes_complete_reports_false_when_the_claim_was_lost() {
    reports_false_when_lost(Op::Complete).await;
}

#[tokio::test]
async fn fenced_worker_writes_checkpoint_renews_the_lease_even_when_there_is_no_progress_to_record()
{
    let db = fixture().await;
    setup(&db, |tx| {
        req(
            tx,
            1,
            "processing",
            NOW - 30 * MIN,
            Some(NOW - 10 * MIN),
            Some("t"),
        )
    })
    .await;
    let renewed = write(&db, |c| {
        ib::checkpoint_inbound_webhook_request(
            c,
            1,
            "t",
            &InboundWebhookRequestProgress::default(),
            NOW,
        )
    })
    .await;
    assert!(renewed);
    let row = request(&db, 1).await.expect("row 1");
    assert_eq!(row.claimed_at, Some(NOW));
    assert_eq!(row.contact_id, None);
}

#[tokio::test]
async fn fenced_worker_writes_release_hands_the_row_back_to_the_pool() {
    let db = fixture().await;
    setup(&db, |tx| {
        req(
            tx,
            1,
            "processing",
            NOW - 30 * MIN,
            Some(NOW - MIN),
            Some("t"),
        )
    })
    .await;
    assert!(write(&db, |c| ib::release_inbound_webhook_request(c, 1, "t")).await);
    let row = request(&db, 1).await.expect("row 1");
    assert_eq!(row.status, "received");
    assert_eq!(row.claim_token, None);
    assert_eq!(row.claimed_at, None);
}

#[tokio::test]
async fn fenced_worker_writes_complete_stamps_processed_at_and_the_terminal_status() {
    let db = fixture().await;
    setup(&db, |tx| {
        req(
            tx,
            1,
            "processing",
            NOW - 30 * MIN,
            Some(NOW - MIN),
            Some("t"),
        )
    })
    .await;
    let done = write(&db, |c| {
        ib::complete_inbound_webhook_request(
            c,
            1,
            "t",
            &InboundWebhookRequestCompletion {
                status: "skipped".to_owned(),
                skip_reason: Some(Some("inboxNotFound".to_owned())),
                ..InboundWebhookRequestCompletion::default()
            },
            NOW,
        )
    })
    .await;
    assert!(done);
    let row = request(&db, 1).await.expect("row 1");
    assert_eq!(row.status, "skipped");
    assert_eq!(row.skip_reason.as_deref(), Some("inboxNotFound"));
    assert_eq!(row.processed_at, Some(NOW));
}

// ---------------------------------------------------------------------------
// insertInboundWebhookRequest
// ---------------------------------------------------------------------------

fn request_values() -> Vec<(&'static str, Value)> {
    vec![
        ("workspaceId", Value::Integer(WS)),
        ("inboundWebhookId", Value::Integer(HOOK)),
        ("idempotencyKey", Value::Text("key-1".to_owned())),
        ("payload", Value::Text("{\"a\":1}".to_owned())),
    ]
}

#[tokio::test]
async fn insert_inbound_webhook_request_ignores_a_duplicate_idempotency_key_returns_null_and_leaves_the_counter_alone()
 {
    let db = fixture().await;
    let first = write(&db, |c| {
        ib::insert_inbound_webhook_request(c, &request_values())
    })
    .await;
    assert!(first.is_some());
    let duplicate = write(&db, |c| {
        ib::insert_inbound_webhook_request(c, &request_values())
    })
    .await;
    assert_eq!(duplicate, None);
    assert_eq!(hook_counter(&db).await.0, Some(1));
    assert_eq!(
        scalar_i64(
            &db,
            "SELECT count(*) FROM \"InboundWebhookRequest\"",
            vec![]
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn insert_inbound_webhook_request_counts_an_accepted_request_in_the_same_transaction_as_its_row()
 {
    let db = fixture().await;
    let inserted = write(&db, |c| {
        ib::insert_inbound_webhook_request(c, &request_values())
    })
    .await;
    assert_eq!(inserted.map(|r| r.inbound_webhook_id), Some(HOOK));
    // updatedAt is written back unchanged: a delivery is not a config change.
    assert_eq!(hook_counter(&db).await, (Some(1), Some(HOOK_UPDATED)));
}

// ---------------------------------------------------------------------------
// listStaleInboundWebhookRequests / failAbandonedInboundWebhookRequests
// ---------------------------------------------------------------------------

/// 11 received long enough, 12 received too recently, 13 processing past its lease,
/// 14 processing with a live lease, 15 finished, 16 received but older than the give-up
/// age.
async fn stale_fixture() -> common::TestDb {
    let db = fixture().await;
    setup(&db, |tx| {
        req(tx, 11, "received", NOW - 5 * MIN, None, None)?;
        req(tx, 12, "received", NOW - MIN, None, None)?;
        req(
            tx,
            13,
            "processing",
            NOW - 30 * MIN,
            Some(LEASE - 1),
            Some("a"),
        )?;
        req(
            tx,
            14,
            "processing",
            NOW - 40 * MIN,
            Some(LEASE + 1),
            Some("b"),
        )?;
        req(tx, 15, "processed", NOW - 50 * MIN, None, None)?;
        req(tx, 16, "received", NOW - 2 * DAY, None, None)
    })
    .await;
    db
}

#[tokio::test]
async fn list_stale_inbound_webhook_requests_selects_unclaimed_for_too_long_and_lease_expired_rows_oldest_first()
 {
    let db = stale_fixture().await;
    let rows = run(&db, |c| {
        ib::list_stale_inbound_webhook_requests(c, RECEIVED, LEASE, CREATED_AFTER, 500)
    })
    .await;
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![13, 11]);
    assert_eq!(
        (rows[0].workspace_id, rows[0].inbound_webhook_id),
        (WS, HOOK)
    );
    let bounded = run(&db, |c| {
        ib::list_stale_inbound_webhook_requests(c, RECEIVED, LEASE, CREATED_AFTER, 1)
    })
    .await;
    assert_eq!(bounded.iter().map(|r| r.id).collect::<Vec<_>>(), vec![13]);
}

#[tokio::test]
async fn list_stale_inbound_webhook_requests_only_returns_requests_younger_than_the_give_up_age() {
    let db = stale_fixture().await;
    let rows = run(&db, |c| {
        ib::list_stale_inbound_webhook_requests(c, RECEIVED, LEASE, CREATED_AFTER, 10)
    })
    .await;
    assert!(rows.iter().all(|r| r.id != 16));
    let all_ages = run(&db, |c| {
        ib::list_stale_inbound_webhook_requests(c, RECEIVED, LEASE, NOW - 3 * DAY, 10)
    })
    .await;
    assert_eq!(
        all_ages.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![16, 13, 11]
    );
}

#[tokio::test]
async fn fail_abandoned_inbound_webhook_requests_closes_only_unfinished_requests_older_than_the_give_up_age_as_abandoned()
 {
    let db = fixture().await;
    setup(&db, |tx| {
        req(tx, 21, "received", NOW - 2 * DAY, None, None)?;
        req(
            tx,
            22,
            "processing",
            NOW - 2 * DAY,
            Some(LEASE - 1),
            Some("a"),
        )?;
        req(tx, 23, "processed", NOW - 2 * DAY, None, None)?;
        req(tx, 24, "received", NOW - 5 * MIN, None, None)
    })
    .await;
    let closed = write(&db, |c| {
        ib::fail_abandoned_inbound_webhook_requests(c, NOW - DAY, RECEIVED, LEASE, NOW)
    })
    .await;
    assert_eq!(closed, 2);
    for id in [21, 22] {
        let row = request(&db, id).await.expect("closed row");
        assert_eq!(row.status, "failed");
        assert_eq!(row.error_code.as_deref(), Some(ib::ERROR_CODE_ABANDONED));
        assert_eq!(row.claim_token, None);
        assert_eq!(row.processed_at, Some(NOW));
    }
    assert_eq!(
        request(&db, 23).await.map(|r| r.status),
        Some("processed".to_owned())
    );
    assert_eq!(
        request(&db, 24).await.map(|r| r.status),
        Some("received".to_owned())
    );
}

// ---------------------------------------------------------------------------
// workspace scoping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn workspace_scoping_delete_inbound_webhooks_scopes_by_workspace_and_skips_the_query_for_no_ids()
 {
    let db = fixture().await;
    let none = write(&db, |c| ib::delete_inbound_webhooks(c, WS, &[])).await;
    assert!(none.is_empty());
    let mut removed = write(&db, |c| ib::delete_inbound_webhooks(c, WS, &[HOOK, 401])).await;
    removed.sort_unstable();
    assert_eq!(removed, vec![HOOK]);
    assert_eq!(
        scalar_i64(
            &db,
            "SELECT count(*) FROM \"InboundWebhook\" WHERE \"id\" = 401",
            vec![]
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn workspace_scoping_find_inbound_webhook_intake_by_token_is_the_one_deliberately_unscoped_lookup_and_reads_four_fields_only()
 {
    let db = fixture().await;
    let intake = |token: &'static str| {
        run(&db, move |c| {
            ib::find_inbound_webhook_intake_by_token(c, token)
        })
    };
    // Another workspace's webhook is found by its token alone.
    assert_eq!(
        intake("iwh_xyz").await,
        Some(InboundWebhookIntake {
            id: 401,
            workspace_id: OTHER,
            active: false,
            is_configured: false,
        })
    );
    assert_eq!(
        intake("iwh_abc").await,
        Some(InboundWebhookIntake {
            id: HOOK,
            workspace_id: WS,
            active: true,
            is_configured: true,
        })
    );
    assert_eq!(intake("nope").await, None);
}

// ---------------------------------------------------------------------------
// list queries read summary columns only
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_queries_read_summary_columns_only_a_request_list_never_selects_the_payload_field_errors_or_action_results()
 {
    let db = fixture().await;
    setup(&db, |tx| {
        req(tx, 31, "processed", NOW - 3 * MIN, None, None)?;
        req(tx, 32, "received", NOW - 2 * MIN, None, None)?;
        req(tx, 33, "processed", NOW - MIN, None, None)
    })
    .await;
    let list = |status: Option<&'static str>, offset: i64| {
        run(&db, move |c| {
            ib::list_inbound_webhook_requests_paginated(
                c,
                &ListInboundWebhookRequestsInput {
                    workspace_id: WS,
                    inbound_webhook_id: HOOK,
                    status: status.map(str::to_owned),
                    limit: 10,
                    offset,
                },
            )
        })
    };
    let page = list(None, 0).await;
    assert_eq!(page.total, 3);
    assert_eq!(
        page.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![33, 32, 31]
    );
    assert_eq!(page.rows[0].created_at, NOW - MIN);
    let processed = list(Some("processed"), 1).await;
    assert_eq!(processed.total, 2);
    assert_eq!(
        processed.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![31]
    );
}

#[tokio::test]
async fn list_queries_read_summary_columns_only_a_webhook_list_never_selects_the_token_config_or_sample()
 {
    let db = fixture().await;
    let page = run(&db, |c| ib::list_inbound_webhooks_paginated(c, WS, 10, 0)).await;
    assert_eq!(page.total, 2);
    assert_eq!(
        page.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![302, HOOK]
    );
    assert_eq!(page.rows[1].name, "Orders");
    assert_eq!(page.rows[1].updated_at, HOOK_UPDATED);
    let second = run(&db, |c| ib::list_inbound_webhooks_paginated(c, WS, 10, 1)).await;
    assert_eq!(
        (
            second.total,
            second.rows.iter().map(|r| r.id).collect::<Vec<_>>()
        ),
        (2, vec![HOOK])
    );
}

// ---------------------------------------------------------------------------
// purgeInboundWebhookRequests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn purge_inbound_webhook_requests_deletes_by_age_alone_so_an_abandoned_in_flight_request_cannot_leak()
 {
    let db = fixture().await;
    setup(&db, |tx| {
        req(tx, 41, "received", NOW - 31 * DAY, None, None)?;
        req(
            tx,
            42,
            "processing",
            NOW - 31 * DAY,
            Some(NOW - 31 * DAY),
            Some("a"),
        )?;
        req(tx, 43, "processed", NOW - 31 * DAY, None, None)?;
        req(tx, 44, "received", NOW - DAY, None, None)
    })
    .await;
    let options = PurgeInboundWebhookRequestsOptions {
        retention_days: 30,
        chunk_size: 1000,
        inter_chunk_delay_ms: 0,
        max_chunks: 10,
        max_run_duration_ms: None,
    };
    let (deleted, reason) = ok(ib::purge_inbound_webhook_requests(&db, options, NOW).await);
    assert_eq!(deleted, 3);
    assert_eq!(reason, ChunkedPurgeStopReason::Drained);
    assert!(request(&db, 44).await.is_some());
}

// ---------------------------------------------------------------------------
// integrationWhatsappRepository.listByDisplayPhoneDigits
// ---------------------------------------------------------------------------

/// WhatsApp integrations: 3101..3103 (WS) show the same number in three formats, 3104
/// (WS) another number, 3105 (OTHER) the same number, 3106 (WS) a legacy `''`.
async fn whatsapp_fixture() -> common::TestDb {
    let db = fixture().await;
    setup(&db, |tx| {
        for (id, ws, display) in [
            (3101_i64, WS, Some("+84 901 234 567")),
            (3102, WS, Some("84-901-234-567")),
            (3103, WS, Some("(84) 901234567")),
            (3104, WS, Some("+1 555 0100")),
            (3105, OTHER, Some("+84 901 234 567")),
            (3106, WS, None),
        ] {
            insert_inbox(tx, id, ws, "whatsapp", &format!("wa-{id}"))?;
            ins!(tx, "IntegrationWhatsapp", {
                "id" => id + 1000, "auth" => "{}".to_owned(), "phoneNumberId" => format!("pn-{id}"),
                "wabaId" => "waba".to_owned(), "businessId" => "biz".to_owned(),
                "name" => format!("WA {id}"), "workspaceId" => ws, "inboxId" => id,
                "displayPhoneNumber" => display.unwrap_or("").to_owned(),
            })?;
        }
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn integration_whatsapp_repository_list_by_display_phone_digits_never_matches_an_empty_digit_string_legacy_rows_default_to()
 {
    let db = whatsapp_fixture().await;
    let rows = run(&db, |c| iw::list_by_display_phone_digits(c, WS, "")).await;
    assert!(rows.is_empty());
}

#[tokio::test]
async fn integration_whatsapp_repository_list_by_display_phone_digits_compares_digits_only_inside_one_workspace_and_fetches_at_most_two_rows()
 {
    let db = whatsapp_fixture().await;
    let rows = run(&db, |c| {
        iw::list_by_display_phone_digits(c, WS, "84901234567")
    })
    .await;
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert!([3101, 3102, 3103].contains(&row.inbox_id), "{row:?}");
        assert_eq!(row.id, row.inbox_id + 1000);
    }
    let other = run(&db, |c| {
        iw::list_by_display_phone_digits(c, OTHER, "84901234567")
    })
    .await;
    assert_eq!(
        other.iter().map(|r| r.inbox_id).collect::<Vec<_>>(),
        vec![3105]
    );
}

// ---------------------------------------------------------------------------
// contactInboxRepository identity finders
// ---------------------------------------------------------------------------

/// Inbox 3201 (WS) and 3202 (WS). Contacts (first, last, phone, email) and their rows on
/// an inbox with `lastMessageAt`:
/// - 501 "An" "Nguyen", +84901234567, a@b.co: 5011 on 3201 (NOW - 10 min)
/// - 502 "Binh", +84901234567: 5021 on 3201 (NOW - 1 min)
/// - 503 (OTHER) "Chi", +84901234567: 5031 on 3201 (NOW)
/// - 504 "Dung", +84901234567: 5041 on 3202 (NOW)
/// - 505 "An  " " Nguyen": 5051 on 3201 (NULL)
/// - 506 "An": 5061 (NOW - 5 min), 5062 (NOW - 3 min), 5063 (NULL) on 3201
/// - 507 "An" "": 5071 on 3201 (NOW - 20 min)
async fn identity_fixture() -> common::TestDb {
    let db = fixture().await;
    setup(&db, |tx| {
        insert_inbox(tx, 3201, WS, "webchat", "wc-3201")?;
        insert_inbox(tx, 3202, WS, "webchat", "wc-3202")?;
        let phone = Some("+84901234567");
        for (id, ws, first, last, phone, email) in [
            (501_i64, WS, "An", Some("Nguyen"), phone, Some("a@b.co")),
            (502, WS, "Binh", None, phone, None),
            (503, OTHER, "Chi", None, phone, Some("a@b.co")),
            (504, WS, "Dung", None, phone, None),
            (505, WS, "An  ", Some(" Nguyen"), None, None),
            (506, WS, "An", None, None, None),
            (507, WS, "An", Some(""), None, None),
        ] {
            ins!(tx, "Contact", {
                "id" => id, "workspaceId" => ws, "firstName" => first.to_owned(),
                "lastName" => last.map(str::to_owned), "phoneNumber" => phone.map(str::to_owned),
                "email" => email.map(str::to_owned),
            })?;
        }
        for (id, contact, inbox, last) in [
            (5011_i64, 501_i64, 3201_i64, Some(NOW - 10 * MIN)),
            (5021, 502, 3201, Some(NOW - MIN)),
            (5031, 503, 3201, Some(NOW)),
            (5041, 504, 3202, Some(NOW)),
            (5051, 505, 3201, None),
            (5061, 506, 3201, Some(NOW - 5 * MIN)),
            (5062, 506, 3201, Some(NOW - 3 * MIN)),
            (5063, 506, 3201, None),
            (5071, 507, 3201, Some(NOW - 20 * MIN)),
        ] {
            insert_ci(tx, id, contact, inbox, "webchat", &format!("src-{id}"))?;
            tx.execute(
                "UPDATE \"ContactInbox\" SET \"lastMessageAt\" = ?1 WHERE \"id\" = ?2",
                chatbotx_db::rusqlite::params![last, id],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

fn pair(contact_id: i64, contact_inbox_id: i64) -> ContactAndInbox {
    ContactAndInbox {
        contact_id,
        contact_inbox_id,
    }
}

#[tokio::test]
async fn contact_inbox_repository_find_latest_by_inbox_and_contact_identity_matches_one_profile_field_on_one_inbox_and_one_workspace_newest_activity_first()
 {
    let db = identity_fixture().await;
    let found = run(&db, |c| {
        ci::find_latest_by_inbox_and_contact_identity(
            c,
            WS,
            3201,
            IdentityField::PhoneNumber,
            "+84901234567",
        )
    })
    .await;
    // 503 is another workspace's contact, 504 is on another inbox.
    assert_eq!(found, Some(pair(502, 5021)));
}

#[tokio::test]
async fn contact_inbox_repository_find_latest_by_inbox_and_contact_identity_matches_by_email() {
    let db = identity_fixture().await;
    let found = run(&db, |c| {
        ci::find_latest_by_inbox_and_contact_identity(c, WS, 3201, IdentityField::Email, "a@b.co")
    })
    .await;
    assert_eq!(found, Some(pair(501, 5011)));
}

#[tokio::test]
async fn contact_inbox_repository_find_by_inbox_and_full_name_matches_the_normalised_name_on_one_inbox_and_one_workspace_newest_activity_first()
 {
    let db = identity_fixture().await;
    let found = run(&db, |c| {
        ci::find_by_inbox_and_full_name(c, WS, 3201, "An Nguyen")
    })
    .await;
    // 505 is stored as "An    Nguyen": compared whitespace-collapsed and trimmed.
    assert_eq!(found, vec![pair(501, 5011), pair(505, 5051)]);
}

#[tokio::test]
async fn contact_inbox_repository_find_by_inbox_and_full_name_counts_people_not_connections_deduplicates_by_contact_in_sql_before_the_limit()
 {
    let db = identity_fixture().await;
    let found = run(&db, |c| ci::find_by_inbox_and_full_name(c, WS, 3201, "An")).await;
    // Three connections of 506 still leave room for 507 ("An " trimmed).
    assert_eq!(found, vec![pair(506, 5062), pair(507, 5071)]);
}

#[tokio::test]
async fn contact_inbox_repository_find_by_inbox_and_full_name_returns_no_rows_when_nothing_matches()
{
    let db = identity_fixture().await;
    let found = run(&db, |c| {
        ci::find_by_inbox_and_full_name(c, WS, 3201, "Nobody")
    })
    .await;
    assert!(found.is_empty());
}
