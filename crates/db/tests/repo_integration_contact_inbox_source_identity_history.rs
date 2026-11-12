//! Port of `packages/database/__tests__/integration/contact-inbox-source-identity-history.test.ts`
//! (P3.2). The row-lock race (B waits on A's lock) becomes two `db.write` jobs; the one
//! SQLite writer serializes them in submit order (A, then B), which is the same order the
//! test forces. History and the cap are asserted as in the original.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::contact_inbox::{
    self as ci, CONTACT_INBOX_IDENTITY_HISTORY_LIMIT, IdentityFields, IdentityHistoryAppend,
};
use chatbotx_db::repo::{self};
use repo_common::ok;
use serde_json::{Value, json};

fn some(v: &str) -> Option<Option<String>> {
    Some(Some(v.to_owned()))
}

fn append(at: &str, reason: &str) -> IdentityHistoryAppend {
    IdentityHistoryAppend {
        changed_at: at.to_owned(),
        reason: reason.to_owned(),
    }
}

/// Port of: packages/database/__tests__/integration/contact-inbox-source-identity-history.test.ts:102 "contactInboxRepository identity history against Postgres > atomically appends locked concurrent updates in order and caps history"
#[tokio::test]
async fn contact_inbox_repository_identity_history_against_postgres_atomically_appends_locked_concurrent_updates_in_order_and_caps_history()
 {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    repo_common::setup(&db, |tx| {
        repo_common::insert_inbox(tx, 3100, 2001, "whatsapp", "identity-history")?;
        ins!(tx, "ContactInbox", { "id" => 9_i64, "originalContactId" => 4001_i64, "contactId" => 4001_i64, "inboxId" => 3100_i64, "channel" => "whatsapp", "source" => "whatsapp", "sourceId" => "phone-original", "sourceUserId" => "user-original", "sourceParentUserId" => "parent-original" })?;
        Ok(())
    })
    .await;
    let a_guard = IdentityFields {
        source_user_id: some("user-original"),
        ..Default::default()
    };
    let a_set = IdentityFields {
        source_user_id: some("user-after-a"),
        ..Default::default()
    };
    let b_guard = IdentityFields {
        source_id: some("phone-original"),
        ..Default::default()
    };
    let b_set = IdentityFields {
        source_id: some("phone-after-b"),
        ..Default::default()
    };
    let (a, b) = tokio::join!(
        repo::write(&db, move |tx| ci::update_identity_guarded(
            tx,
            9,
            &a_guard,
            &a_set,
            Some(&append("2026-09-28T01:00:00.000Z", "userIdChanged"))
        )),
        repo::write(&db, move |tx| ci::update_identity_guarded(
            tx,
            9,
            &b_guard,
            &b_set,
            Some(&append("2026-09-28T01:00:01.000Z", "phoneChanged"))
        )),
    );
    ok(a);
    let updated = ok(b).expect("b updated");
    assert_eq!(updated.source_id, "phone-after-b");
    assert_eq!(updated.source_user_id.as_deref(), Some("user-after-a"));
    assert_eq!(
        ci::identity_history(&updated),
        json!([
            { "sourceId": "phone-original", "sourceUserId": "user-original", "sourceParentUserId": "parent-original", "changedAt": "2026-09-28T01:00:00.000Z", "reason": "userIdChanged" },
            { "sourceId": "phone-original", "sourceUserId": "user-after-a", "sourceParentUserId": "parent-original", "changedAt": "2026-09-28T01:00:01.000Z", "reason": "phoneChanged" },
        ])
    );
    let seeded: Vec<Value> = (0..CONTACT_INBOX_IDENTITY_HISTORY_LIMIT)
        .map(|i| json!({ "sourceId": format!("seed-source-{i}"), "sourceUserId": format!("seed-user-{i}"), "sourceParentUserId": format!("seed-parent-{i}"), "changedAt": format!("2026-09-28T02:00:{i:02}.000Z"), "reason": "userIdChanged" }))
        .collect();
    let text = Value::Array(seeded.clone()).to_string();
    repo_common::handover::exec(
        &db,
        r#"UPDATE "ContactInbox" SET "sourceIdentityHistory" = ?1 WHERE "id" = 9"#,
        vec![repo_common::text(&text)],
    )
    .await;
    let guard = IdentityFields {
        source_parent_user_id: some("parent-original"),
        ..Default::default()
    };
    let set = IdentityFields {
        source_parent_user_id: some("parent-after-cap"),
        ..Default::default()
    };
    let capped = ok(repo::write(&db, move |tx| {
        ci::update_identity_guarded(
            tx,
            9,
            &guard,
            &set,
            Some(&append("2026-09-28T02:00:10.000Z", "parentFallback")),
        )
    })
    .await)
    .expect("capped");
    let history = ci::identity_history(&capped);
    assert_eq!(
        history.as_array().map(Vec::len),
        Some(CONTACT_INBOX_IDENTITY_HISTORY_LIMIT)
    );
    let mut expected: Vec<Value> = seeded[1..].to_vec();
    expected.push(json!({ "sourceId": "phone-after-b", "sourceUserId": "user-after-a", "sourceParentUserId": "parent-original", "changedAt": "2026-09-28T02:00:10.000Z", "reason": "parentFallback" }));
    assert_eq!(history, Value::Array(expected));
}
