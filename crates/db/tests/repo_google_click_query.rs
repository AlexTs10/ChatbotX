//! Port of `packages/database/__tests__/google-click-query.test.ts` (P3.2b). The TS renders
//! the predicate / mocked chains with `PgDialect`; here the predicate text is checked and
//! the predicate and the contact-inbox lookups run on a migrated temp file (adapted).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo10.rs"]
mod repo10;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::contact_inbox as ci;
use chatbotx_db::repo::queries::google_click::google_click_predicate;
use chatbotx_db::repo::sql::Sql;
use repo10::{OTHER_WS, WS, click_db, read, seed_referral};
use serde_json::json;

/// Contact inboxes 3101 (gclid, received 10:00), 3102 (gbraid, 11:00), 3103 (Meta only) and
/// 3104 (no referral) of the seeded contact.
async fn clicks_db() -> common::TestDb {
    let db = click_db().await;
    db.write(|tx| {
        let s = common::Seed {
            user_id: 1001,
            workspace_id: WS,
            inbox_id: 3001,
            contact_id: 4001,
        };
        for (id, source) in [(3102, "wa-2"), (3103, "wa-3"), (3104, "wa-4")] {
            common::insert_contact_inbox(tx, s, id, source, None, None)?;
        }
        Ok(())
    })
    .await
    .expect("contact inboxes");
    seed_referral(
        &db,
        3101,
        Some(json!({ "gclid": "g1", "googleClickReceivedAt": "2026-10-01T10:00:00.000Z" })),
    )
    .await;
    seed_referral(
        &db,
        3102,
        Some(json!({ "gbraid": "b1", "googleClickReceivedAt": "2026-10-01T11:00:00.000Z" })),
    )
    .await;
    seed_referral(&db, 3103, Some(json!({ "ctwaClid": "c1", "gclid": null }))).await;
    db
}

#[tokio::test]
async fn google_click_predicate_matches_a_recorded_gclid_or_gbraid_and_nothing_meta_related() {
    let predicate = google_click_predicate();
    assert!(
        predicate.text.contains("->>'gclid' IS NOT NULL"),
        "{}",
        predicate.text
    );
    assert!(predicate.text.contains("->>'gbraid' IS NOT NULL"));
    assert!(!predicate.text.contains("ctwaClid"));
    let db = clicks_db().await;
    let ids = read(&db, move |c| {
        let mut q = Sql::raw(r#"SELECT "id" FROM "ContactInbox" WHERE "#);
        q.append(predicate);
        q.push(r#" ORDER BY "id""#);
        q.query_map(c, |r| r.get::<_, i64>(0))
    })
    .await;
    assert_eq!(ids, vec![3101, 3102]);
}

#[tokio::test]
async fn contact_inbox_repository_google_click_lookups_find_google_click_attribution_is_workspace_scoped_through_inbox_and_requires_a_click()
 {
    let db = clicks_db().await;
    let row = read(&db, |c| ci::find_google_click_attribution(c, WS, 3101)).await;
    assert_eq!(row.map(|r| r.id), Some(3101));
    // Another workspace is fenced out by the Inbox join; a Meta-only row is not a click.
    assert!(
        read(&db, |c| ci::find_google_click_attribution(
            c, OTHER_WS, 3101
        ))
        .await
        .is_none()
    );
    assert!(
        read(&db, |c| ci::find_google_click_attribution(c, WS, 3103))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn contact_inbox_repository_google_click_lookups_find_google_click_attribution_returns_null_when_no_row_matches()
 {
    let db = clicks_db().await;
    assert!(
        read(&db, |c| ci::find_google_click_attribution(c, WS, 9999))
            .await
            .is_none()
    );
    assert!(
        read(&db, |c| ci::find_google_click_attribution(c, WS, 3104))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn contact_inbox_repository_google_click_lookups_find_latest_google_click_inbox_by_contact_orders_by_the_newest_click()
 {
    let db = clicks_db().await;
    let row = read(&db, |c| {
        ci::find_latest_google_click_inbox_by_contact(c, WS, 4001)
    })
    .await;
    assert_eq!(row.map(|r| r.id), Some(3102));
    // A click without a receivedAt sorts last (DESC NULLS LAST).
    seed_referral(&db, 3104, Some(json!({ "gclid": "g4" }))).await;
    let row = read(&db, |c| {
        ci::find_latest_google_click_inbox_by_contact(c, WS, 4001)
    })
    .await;
    assert_eq!(row.map(|r| r.id), Some(3102));
    assert!(
        read(&db, |c| ci::find_latest_google_click_inbox_by_contact(
            c, OTHER_WS, 4001
        ))
        .await
        .is_none()
    );
}

#[tokio::test]
async fn contact_inbox_repository_google_click_lookups_click_lookups_select_the_inbox_source_id_for_error_log_attribution()
 {
    let db = clicks_db().await;
    let latest = read(&db, |c| {
        ci::find_latest_google_click_inbox_by_contact(c, WS, 4001)
    })
    .await;
    assert_eq!(latest.map(|r| r.source_id), Some("wa-2".to_owned()));
    let direct = read(&db, |c| ci::find_google_click_attribution(c, WS, 3101)).await;
    assert_eq!(direct.map(|r| r.source_id), Some("wa-1".to_owned()));
}
