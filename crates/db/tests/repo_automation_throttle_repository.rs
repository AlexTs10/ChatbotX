//! Port of `packages/database/__tests__/automation-throttle-repository.test.ts` (P3.2).
//! The mocked query chain becomes real `AutomationThrottle` rows (adapted). `claimId` must
//! be 36 characters (a UUID, `CHECK (length = 36)`), so the TS ids are padded UUIDs.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::automation_throttle::{self as t, ThrottleClaimOutcome, ThrottleSubject};
use chatbotx_db::repo::{self};
use common::{fresh_db, insert_contact_inbox, seed};
use repo_common::{NOW, int, ok};

fn claim_id(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

fn subject(subject_id: i64) -> ThrottleSubject {
    ThrottleSubject {
        workspace_id: 2001,
        contact_inbox_id: 4501,
        throttle_type: "defaultReply".into(),
        subject_id,
    }
}

async fn db() -> common::TestDb {
    let db = fresh_db().await;
    let s = seed(&db).await;
    repo_common::setup(&db, move |tx| {
        insert_contact_inbox(tx, s, 4501, "psid-1", None, None)?;
        Ok(())
    })
    .await;
    db
}

async fn claim(db: &Db, subject_id: i64, window: i64, id: u32, now: i64) -> ThrottleClaimOutcome {
    let cid = claim_id(id);
    ok(repo::write(db, move |tx| {
        t::claim_automation_throttle(tx, &subject(subject_id), window, &cid, now)
    })
    .await)
}

/// Port of: packages/database/__tests__/automation-throttle-repository.test.ts:58 "claimAutomationThrottle > returns 'won' with the DB-computed remainingSeconds on a fresh insert / re-trigger"
#[tokio::test]
async fn claim_automation_throttle_returns_won_with_the_db_computed_remaining_seconds_on_a_fresh_insert_re_trigger()
 {
    let db = db().await;
    assert_eq!(
        claim(&db, 0, 3600, 1, NOW).await,
        ThrottleClaimOutcome::Won {
            claim_id: claim_id(1),
            remaining_seconds: 3600
        }
    );
    // Re-trigger after the window: wins again with a full window.
    assert_eq!(
        claim(&db, 0, 3600, 2, NOW + 3_600_000).await,
        ThrottleClaimOutcome::Won {
            claim_id: claim_id(2),
            remaining_seconds: 3600
        }
    );
}

/// Port of: packages/database/__tests__/automation-throttle-repository.test.ts:83 "claimAutomationThrottle > windowSeconds 0 (unbounded) omits the setWhere predicate so the upsert always wins"
#[tokio::test]
async fn claim_automation_throttle_window_seconds_0_unbounded_omits_the_set_where_predicate_so_the_upsert_always_wins()
 {
    let db = db().await;
    assert_eq!(
        claim(&db, 0, 0, 1, NOW).await,
        ThrottleClaimOutcome::Won {
            claim_id: claim_id(1),
            remaining_seconds: 0
        }
    );
    assert_eq!(
        claim(&db, 0, 0, 2, NOW).await,
        ThrottleClaimOutcome::Won {
            claim_id: claim_id(2),
            remaining_seconds: 0
        }
    );
}

/// Port of: packages/database/__tests__/automation-throttle-repository.test.ts:105 "claimAutomationThrottle > falls back to a required SELECT for the DB-computed remainingSeconds when the conflict setWhere denies"
#[tokio::test]
async fn claim_automation_throttle_falls_back_to_a_required_select_for_the_db_computed_remaining_seconds_when_the_conflict_set_where_denies()
 {
    let db = db().await;
    claim(&db, 0, 3600, 1, NOW).await;
    assert_eq!(
        claim(&db, 0, 3600, 2, NOW + 1_800_000).await,
        ThrottleClaimOutcome::Denied {
            remaining_seconds: 1800
        }
    );
    // The holder keeps the row.
    let held =
        repo_common::scalar_text(&db, r#"SELECT "claimId" FROM "AutomationThrottle""#, vec![])
            .await;
    assert_eq!(held, Some(claim_id(1)));
}

// not-applicable: packages/database/__tests__/automation-throttle-repository.test.ts:123 "claimAutomationThrottle > returns remainingSeconds:0 (no-cache) when the denied-branch SELECT finds no row (raced with a concurrent release)" — the race needs a release between the upsert and the SELECT of one claim; with one SQLite writer both run in the same transaction, so no other write can come between them. The `None -> 0` branch is kept in the code.

/// Port of: packages/database/__tests__/automation-throttle-repository.test.ts:140 "claimAutomationThrottle > scopes the conflict target and setWhere to workspaceId+contactInboxId+throttleType+subjectId (type/subject isolation)"
#[tokio::test]
async fn claim_automation_throttle_scopes_the_conflict_target_and_set_where_to_workspace_id_contact_inbox_id_throttle_type_subject_id_type_subject_isolation()
 {
    let db = db().await;
    claim(&db, 0, 60, 1, NOW).await;
    // Another subject of the same contact inbox is its own throttle.
    assert_eq!(
        claim(&db, 42, 60, 4, NOW).await,
        ThrottleClaimOutcome::Won {
            claim_id: claim_id(4),
            remaining_seconds: 60
        }
    );
    let n = repo_common::scalar_i64(
        &db,
        r#"SELECT count(*) FROM "AutomationThrottle" WHERE "subjectId" = ?1"#,
        vec![int(42)],
    )
    .await;
    assert_eq!(n, Some(1));
}

/// Port of: packages/database/__tests__/automation-throttle-repository.test.ts:157 "releaseAutomationThrottle > deletes by claimId (CAS) scoped to the full subject and returns true when a row was removed"
#[tokio::test]
async fn release_automation_throttle_deletes_by_claim_id_cas_scoped_to_the_full_subject_and_returns_true_when_a_row_was_removed()
 {
    let db = db().await;
    claim(&db, 0, 3600, 1, NOW).await;
    claim(&db, 7, 3600, 1, NOW).await;
    let cid = claim_id(1);
    assert!(ok(repo::write(&db, move |tx| {
        t::release_automation_throttle(tx, &subject(0), &cid)
    })
    .await));
    let left =
        repo_common::scalar_i64(&db, r#"SELECT count(*) FROM "AutomationThrottle""#, vec![]).await;
    assert_eq!(left, Some(1));
}

/// Port of: packages/database/__tests__/automation-throttle-repository.test.ts:170 "releaseAutomationThrottle > returns false when the claimId no longer matches — a newer claim already replaced the row"
#[tokio::test]
async fn release_automation_throttle_returns_false_when_the_claim_id_no_longer_matches_a_newer_claim_already_replaced_the_row()
 {
    let db = db().await;
    claim(&db, 0, 0, 1, NOW).await;
    claim(&db, 0, 0, 2, NOW).await;
    let stale = claim_id(1);
    assert!(!ok(repo::write(&db, move |tx| {
        t::release_automation_throttle(tx, &subject(0), &stale)
    })
    .await));
}

/// Port of: packages/database/__tests__/automation-throttle-repository.test.ts:183 "purgeStaleAutomationThrottles > bulk-deletes stale rows and returns the DB-side rowCount (no rows materialized)"
#[tokio::test]
async fn purge_stale_automation_throttles_bulk_deletes_stale_rows_and_returns_the_db_side_row_count_no_rows_materialized()
 {
    let db = db().await;
    let old = NOW - 49 * 3_600_000;
    claim(&db, 1, 0, 1, old).await;
    claim(&db, 2, 0, 2, old).await;
    claim(&db, 3, 0, 3, NOW).await;
    assert_eq!(
        ok(repo::write(&db, |tx| t::purge_stale_automation_throttles(
            tx,
            t::STALE_RETENTION_HOURS,
            NOW
        ))
        .await),
        2
    );
}

/// Port of: packages/database/__tests__/automation-throttle-repository.test.ts:189 "purgeStaleAutomationThrottles > returns 0 when nothing is stale"
#[tokio::test]
async fn purge_stale_automation_throttles_returns_0_when_nothing_is_stale() {
    let db = db().await;
    claim(&db, 1, 0, 1, NOW).await;
    assert_eq!(
        ok(repo::write(&db, |tx| t::purge_stale_automation_throttles(
            tx,
            t::STALE_RETENTION_HOURS,
            NOW
        ))
        .await),
        0
    );
}
