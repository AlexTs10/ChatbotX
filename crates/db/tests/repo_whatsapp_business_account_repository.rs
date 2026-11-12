//! Port of `packages/database/__tests__/whatsapp-business-account-repository.test.ts`
//! (P3.3). The mocked insert/update chains become real `WhatsappBusinessAccount` rows;
//! the asserted `setWhere` / `eq(revision)` guards are checked as the CAS outcome.

mod common;
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::whatsapp_business_account::{
    UpsertCredentialInput, find_by_waba, update_scope_cache, upsert_credential,
};
use chatbotx_db::repo::{self, Json};
use common::{fresh_db, seed};
use repo_common::{ms, ok};
use serde_json::json;

const WS: i64 = 2001;

fn credential() -> Json {
    Json(json!({"v": 1, "iv": "a".repeat(24), "text": "cipher", "tag": "b".repeat(32)}))
}

fn upsert_input(business_id: &str, expected_revision: i64) -> UpsertCredentialInput {
    UpsertCredentialInput {
        workspace_id: WS,
        waba_id: "waba-1".to_owned(),
        business_id: business_id.to_owned(),
        credential: credential(),
        granted_scopes: vec!["whatsapp_business_management".to_owned()],
        scope_checked_at: ms("2026-09-08T00:00:00.000Z"),
        expected_revision,
    }
}

/// Port of: packages/database/__tests__/whatsapp-business-account-repository.test.ts:45 "WhatsappBusinessAccountRepository > upsertCredential returns the inserted or current-revision row"
#[tokio::test]
async fn whatsapp_business_account_repository_upsert_credential_returns_the_inserted_or_current_revision_row()
 {
    let db = fresh_db().await;
    seed(&db).await;
    let inserted = ok(repo::write(&db, |tx| {
        upsert_credential(tx, &upsert_input("business-1", 0))
    })
    .await)
    .expect("inserted");
    assert_eq!(inserted.revision, 1);
    assert_eq!(inserted.credential, credential());
    assert_eq!(
        inserted.granted_scopes,
        Json(json!(["whatsapp_business_management"]))
    );
    // The current revision (1) replaces the credential and bumps the revision.
    let replaced = ok(repo::write(&db, |tx| {
        upsert_credential(tx, &upsert_input("business-2", 1))
    })
    .await)
    .expect("replaced");
    assert_eq!(replaced.id, inserted.id);
    assert_eq!(replaced.revision, 2);
    assert_eq!(replaced.business_id, "business-2");
}

/// Port of: packages/database/__tests__/whatsapp-business-account-repository.test.ts:78 "WhatsappBusinessAccountRepository > upsertCredential exposes a stale revision without overwriting"
#[tokio::test]
async fn whatsapp_business_account_repository_upsert_credential_exposes_a_stale_revision_without_overwriting()
 {
    let db = fresh_db().await;
    seed(&db).await;
    ok(repo::write(&db, |tx| {
        upsert_credential(tx, &upsert_input("business-1", 0))
    })
    .await);
    ok(repo::write(&db, |tx| {
        upsert_credential(tx, &upsert_input("business-1b", 1))
    })
    .await);
    // The row is at revision 2; a caller that read revision 1 is stale.
    let stale = ok(repo::write(&db, |tx| {
        upsert_credential(tx, &upsert_input("business-stale", 1))
    })
    .await);
    assert_eq!(stale, None);
    let row = ok(repo::read(&db, |c| find_by_waba(c, WS, "waba-1")).await).expect("row");
    assert_eq!(row.business_id, "business-1b");
    assert_eq!(row.revision, 2);
}

/// Port of: packages/database/__tests__/whatsapp-business-account-repository.test.ts:101 "WhatsappBusinessAccountRepository > updateScopeCache updates only its expected revision"
#[tokio::test]
async fn whatsapp_business_account_repository_update_scope_cache_updates_only_its_expected_revision()
 {
    let db = fresh_db().await;
    seed(&db).await;
    ok(repo::write(&db, |tx| {
        upsert_credential(tx, &upsert_input("business-1", 0))
    })
    .await);
    ok(repo::write(&db, |tx| {
        upsert_credential(tx, &upsert_input("business-1", 1))
    })
    .await);
    let scopes = vec!["whatsapp_business_messaging".to_owned()];
    let at = ms("2026-09-08T00:00:00.000Z");
    // Wrong revision, wrong workspace, wrong WABA: no change.
    let s = scopes.clone();
    assert_eq!(
        ok(repo::write(&db, move |tx| update_scope_cache(
            tx, WS, "waba-1", &s, at, 1
        ))
        .await),
        None
    );
    let s = scopes.clone();
    assert_eq!(
        ok(repo::write(&db, move |tx| update_scope_cache(
            tx, 9999, "waba-1", &s, at, 2
        ))
        .await),
        None
    );
    let s = scopes.clone();
    assert_eq!(
        ok(repo::write(&db, move |tx| update_scope_cache(
            tx, WS, "waba-x", &s, at, 2
        ))
        .await),
        None
    );
    let s = scopes.clone();
    let updated = ok(repo::write(&db, move |tx| {
        update_scope_cache(tx, WS, "waba-1", &s, at, 2)
    })
    .await)
    .expect("updated");
    assert_eq!(updated.revision, 3);
    assert_eq!(
        updated.granted_scopes,
        Json(json!(["whatsapp_business_messaging"]))
    );
    assert_eq!(updated.scope_checked_at, Some(at));
}
