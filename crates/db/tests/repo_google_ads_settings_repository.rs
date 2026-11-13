//! Port of `packages/database/__tests__/google-ads-settings-repository.test.ts` (P3.2b).
//! The TS checks the mocked select / insert chains; here the repository runs on a migrated
//! temp file (adapted): rows of two workspaces, the upsert on the unique `workspaceId`.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo10.rs"]
mod repo10;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::google_ads_settings as gas;
use chatbotx_db::repo::{self, Json};
use repo_common::err_message;
use repo10::{OTHER_WS, WS, coexist_db, exec, read, write};
use serde_json::json;

fn settings() -> Json {
    Json(json!({
        "version": 1,
        "consent": {
            "adUserData": { "type": "granted" },
            "adPersonalization": { "type": "notProvided" },
        },
    }))
}

#[tokio::test]
async fn google_ads_settings_repository_find_by_workspace_id_reads_only_the_requested_workspace_s_row()
 {
    let db = coexist_db().await;
    exec(
        &db,
        r#"INSERT INTO "GoogleAdsSettings" ("id", "workspaceId", "settings") VALUES (1, 2001, '{"version":1}'), (2, 2002, '{"version":2}')"#,
    )
    .await;
    let row = read(&db, |c| gas::find_by_workspace_id(c, WS))
        .await
        .expect("row");
    assert_eq!((row.id, row.workspace_id), (1, WS));
    assert_eq!(row.settings, Json(json!({ "version": 1 })));
    let other = read(&db, |c| gas::find_by_workspace_id(c, OTHER_WS))
        .await
        .expect("row");
    assert_eq!(other.id, 2);
}

#[tokio::test]
async fn google_ads_settings_repository_find_by_workspace_id_returns_null_when_the_workspace_has_no_row()
 {
    let db = coexist_db().await;
    assert!(
        read(&db, |c| gas::find_by_workspace_id(c, OTHER_WS))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn google_ads_settings_repository_upsert_settings_inserts_the_document_and_replaces_only_it_on_workspace_conflict()
 {
    let db = coexist_db().await;
    let first = write(&db, |c| gas::upsert_settings(c, WS, &settings())).await;
    assert_eq!(first.workspace_id, WS);
    assert_eq!(first.settings, settings());
    let replaced = Json(json!({ "version": 1, "consent": {} }));
    let doc = replaced.clone();
    let second = write(&db, move |c| gas::upsert_settings(c, WS, &doc)).await;
    // Same row (id, createdAt kept); only the document changed.
    assert_eq!(second.id, first.id);
    assert_eq!(second.created_at, first.created_at);
    assert_eq!(second.settings, replaced);
    let count =
        repo_common::scalar_i64(&db, r#"SELECT count(*) FROM "GoogleAdsSettings""#, vec![]).await;
    assert_eq!(count, Some(1));
}

#[tokio::test]
async fn google_ads_settings_repository_upsert_settings_throws_when_the_statement_returns_no_row() {
    let db = coexist_db().await;
    // A trigger that drops the row stands in for a statement that returns nothing.
    exec(
        &db,
        r#"CREATE TRIGGER "test_swallow_settings" BEFORE INSERT ON "GoogleAdsSettings"
           BEGIN SELECT RAISE(IGNORE); END"#,
    )
    .await;
    let message =
        err_message(repo::write(&db, |tx| gas::upsert_settings(tx, WS, &settings())).await);
    assert!(
        message.contains("Failed to upsert Google Ads settings"),
        "{message}"
    );
}
