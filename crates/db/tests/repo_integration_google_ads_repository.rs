//! Port of `packages/database/__tests__/integration-google-ads-repository.test.ts` (P3.2b).
//! The TS renders the mocked where / orderBy with `PgDialect` and checks the SQL text; here
//! the queries run on a migrated temp file (adapted): Google Ads rows of two workspaces.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo10.rs"]
mod repo10;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::connect_session;
use chatbotx_db::repo::integration_google_ads::{self as iga, SyncTarget};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::types::Value;
use repo10::{OTHER_WS, WS, coexist_db, put_google_ads, put_session, read, write};

/// Workspace 2003 (owner 1003).
const THIRD_WS: i64 = 2003;

/// Google Ads rows 11 (WS, customer "1"), 12 (OTHER_WS, "2") and 13 (THIRD_WS, "1").
async fn ads_db() -> common::TestDb {
    let db = coexist_db().await;
    common::seed_workspace(&db, 1003, THIRD_WS).await;
    db.write(|tx| {
        put_google_ads(tx, 11, WS, "1")?;
        put_google_ads(tx, 12, OTHER_WS, "2")?;
        put_google_ads(tx, 13, THIRD_WS, "1")?;
        Ok(())
    })
    .await
    .expect("google ads rows");
    db
}

#[tokio::test]
async fn integration_google_ads_repository_find_by_workspace_id_scopes_by_workspace() {
    let db = ads_db().await;
    let row = read(&db, |c| iga::find_by_workspace_id(c, WS))
        .await
        .expect("row");
    assert_eq!((row.id, row.workspace_id), (11, WS));
    assert!(
        read(&db, |c| iga::find_by_workspace_id(c, 2999))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn integration_google_ads_repository_exists_by_workspace_id_is_workspace_scoped_and_false_true_when_a_row_exists()
 {
    let db = ads_db().await;
    assert!(read(&db, |c| iga::exists_by_workspace_id(c, WS)).await);
}

#[tokio::test]
async fn integration_google_ads_repository_exists_by_workspace_id_is_workspace_scoped_and_false_true_when_no_row_exists()
 {
    let db = ads_db().await;
    assert!(!read(&db, |c| iga::exists_by_workspace_id(c, 2999)).await);
}

#[tokio::test]
async fn integration_google_ads_repository_update_setup_is_workspace_scoped_and_writes_only_the_given_values()
 {
    let db = ads_db().await;
    db.write(|tx| {
        Ok(tx.execute_batch(
            r#"UPDATE "IntegrationGoogleAds" SET "setupError" = 'boom', "descriptiveName" = 'Shop' WHERE "id" = 11"#,
        )?)
    })
    .await
    .expect("setup error");
    let values = || {
        vec![
            ("conversionCustomerId", Value::Text("123".into())),
            ("setupError", Value::Null),
        ]
    };
    // Another workspace's id is fenced out.
    let fenced = write(&db, move |c| iga::update_setup(c, 11, OTHER_WS, &values())).await;
    assert!(fenced.is_none());
    let row = write(&db, move |c| iga::update_setup(c, 11, WS, &values()))
        .await
        .expect("row");
    assert_eq!(row.conversion_customer_id.as_deref(), Some("123"));
    assert_eq!(row.setup_error, None);
    // Columns outside the values are left alone.
    assert_eq!(row.descriptive_name.as_deref(), Some("Shop"));
    assert_eq!(row.customer_id, "1");
}

#[tokio::test]
async fn integration_google_ads_repository_list_sync_targets_first_page_has_no_id_filter_orders_by_id_asc_and_applies_the_limit()
 {
    let db = ads_db().await;
    let all = read(&db, |c| iga::list_sync_targets(c, None, 100)).await;
    assert_eq!(
        all,
        vec![
            SyncTarget {
                id: 11,
                workspace_id: WS
            },
            SyncTarget {
                id: 12,
                workspace_id: OTHER_WS
            },
            SyncTarget {
                id: 13,
                workspace_id: THIRD_WS
            },
        ]
    );
    let two = read(&db, |c| iga::list_sync_targets(c, None, 2)).await;
    assert_eq!(two.iter().map(|t| t.id).collect::<Vec<_>>(), vec![11, 12]);
}

#[tokio::test]
async fn integration_google_ads_repository_list_sync_targets_after_id_is_a_strict_keyset_lower_bound()
 {
    let db = ads_db().await;
    let page = read(&db, |c| iga::list_sync_targets(c, Some(11), 5)).await;
    assert_eq!(page.iter().map(|t| t.id).collect::<Vec<_>>(), vec![12, 13]);
    let one = read(&db, |c| iga::list_sync_targets(c, Some(11), 1)).await;
    assert_eq!(one.iter().map(|t| t.id).collect::<Vec<_>>(), vec![12]);
}

#[tokio::test]
async fn integration_google_ads_repository_list_connected_customer_ids_returns_distinct_customer_ids()
 {
    let db = ads_db().await;
    let mut ids = read(&db, iga::list_connected_customer_ids).await;
    ids.sort();
    assert_eq!(ids, vec!["1".to_owned(), "2".to_owned()]);
}

#[tokio::test]
async fn connect_session_repository_find_latest_in_flight_by_provider_filters_workspace_provider_active_statuses_and_expiry_newest_first()
 {
    let db = coexist_db().await;
    let now = now_ms();
    let later = now + 3_600_000;
    db.write(move |tx| {
        put_session(tx, 1, WS, "googleAds", "pending", now - 5_000, later)?;
        put_session(tx, 2, WS, "googleAds", "authorized", now - 4_000, later)?;
        // Newer but: expired, terminal, other provider, other workspace.
        put_session(tx, 3, WS, "googleAds", "pending", now - 3_000, now - 1_000)?;
        put_session(tx, 4, WS, "googleAds", "cancelled", now - 2_000, later)?;
        put_session(tx, 5, WS, "messenger", "pending", now - 1_000, later)?;
        put_session(tx, 6, OTHER_WS, "googleAds", "pending", now, later)?;
        Ok(())
    })
    .await
    .expect("sessions");
    let found = read(&db, move |c| {
        connect_session::find_latest_in_flight_by_provider(c, WS, "googleAds", now)
    })
    .await;
    assert_eq!(found.map(|s| s.id), Some(2));
    let none = read(&db, move |c| {
        connect_session::find_latest_in_flight_by_provider(c, WS, "googleSheets", now)
    })
    .await;
    assert!(none.is_none());
}
