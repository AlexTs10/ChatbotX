//! Port of `packages/database/__tests__/integration/contact-custom-field-pair-lock.test.ts`
//! (P3.2). Two Postgres connections become two concurrent `db.write` jobs on one migrated
//! SQLite file; the one writer serializes them (the asserted outcomes are the same).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::contact_custom_field as ccf;
use chatbotx_db::repo::{self, Error, new_id};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::{Db, WriteTx};
use repo_common::{int, ok};

const ROUNDS: usize = 25;
const CONTACTS: [i64; 2] = [10, 11];
const FIELDS: [i64; 2] = [1, 2];

async fn db() -> common::TestDb {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    repo_common::setup(&db, |tx| {
        for c in CONTACTS {
            repo_common::insert_contact(tx, c, 2001)?;
        }
        for (f, name) in FIELDS.iter().zip(["points", "visits"]) {
            ins!(tx, "CustomField", { "id" => *f, "name" => name, "type" => "number", "workspaceId" => 2001_i64 })?;
        }
        Ok(())
    })
    .await;
    db
}

fn upsert(tx: &Connection, contact: i64, field: i64, value: &str) -> repo::Result<()> {
    tx.execute(
        r#"INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value") VALUES (?1, ?2, ?3, ?4)
           ON CONFLICT ("contactId", "customFieldId") DO UPDATE SET "value" = excluded."value""#,
        chatbotx_db::rusqlite::params![new_id(), contact, field, value],
    )?;
    Ok(())
}

/// The lock -> read -> write shape of `applyFieldOperations`.
fn increment(
    contact: i64,
    field: i64,
) -> impl FnOnce(&mut WriteTx<'_>) -> repo::Result<()> + Send + 'static {
    move |tx| {
        ccf::lock_pairs(tx, &[(contact, field)])?;
        let current = ccf::list_values_for_pairs(tx, &[contact], &[field])?;
        let value: i64 = current.first().and_then(|r| r.2.parse().ok()).unwrap_or(0);
        upsert(tx, contact, field, &(value + 1).to_string())
    }
}

async fn read_value(db: &Db, contact: i64, field: i64) -> Option<String> {
    repo_common::scalar_text(db, r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = ?1 AND "customFieldId" = ?2"#, vec![int(contact), int(field)]).await
}

/// Port of: packages/database/__tests__/integration/contact-custom-field-pair-lock.test.ts:119 "contactCustomFieldRepository pair locks > two concurrent increments on a missing row both count"
#[tokio::test]
async fn contact_custom_field_repository_pair_locks_two_concurrent_increments_on_a_missing_row_both_count()
 {
    let db = db().await;
    for _ in 0..ROUNDS {
        repo_common::handover::exec(
            &db,
            r#"DELETE FROM "ContactCustomField" WHERE "contactId" = 10"#,
            vec![],
        )
        .await;
        let (a, b) = tokio::join!(
            repo::write(&db, increment(10, 1)),
            repo::write(&db, increment(10, 1))
        );
        ok(a);
        ok(b);
        assert_eq!(read_value(&db, 10, 1).await.as_deref(), Some("2"));
    }
}

/// Port of: packages/database/__tests__/integration/contact-custom-field-pair-lock.test.ts:136 "contactCustomFieldRepository pair locks > a plain set racing an increment on an existing row stays serial"
#[tokio::test]
async fn contact_custom_field_repository_pair_locks_a_plain_set_racing_an_increment_on_an_existing_row_stays_serial()
 {
    let db = db().await;
    for _ in 0..ROUNDS {
        ok(repo::write(&db, |tx| upsert(tx, 10, 2, "10")).await);
        let (a, b) = tokio::join!(
            repo::write(&db, increment(10, 2)),
            repo::write(&db, |tx| upsert(tx, 10, 2, "100"))
        );
        ok(a);
        ok(b);
        let v = read_value(&db, 10, 2).await.unwrap_or_default();
        assert!(v == "100" || v == "101", "{v}");
    }
}

/// Port of: packages/database/__tests__/integration/contact-custom-field-pair-lock.test.ts:167 "contactCustomFieldRepository pair locks > opposite-order multi-pair locks never deadlock"
#[tokio::test]
async fn contact_custom_field_repository_pair_locks_opposite_order_multi_pair_locks_never_deadlock()
{
    let db = db().await;
    let pairs: Vec<(i64, i64)> = CONTACTS
        .iter()
        .flat_map(|c| FIELDS.iter().map(move |f| (*c, *f)))
        .collect();
    for _ in 0..ROUNDS {
        let forward = pairs.clone();
        let mut backward = pairs.clone();
        backward.reverse();
        let (a, b) = tokio::join!(
            repo::write(&db, move |tx| ccf::lock_pairs(tx, &forward)),
            repo::write(&db, move |tx| ccf::lock_pairs(tx, &backward)),
        );
        ok(a);
        ok(b);
    }
}

/// Port of: packages/database/__tests__/integration/contact-custom-field-pair-lock.test.ts:184 "contactCustomFieldRepository pair locks > deleteValue returns what it removed, or null when there was nothing"
#[tokio::test]
async fn contact_custom_field_repository_pair_locks_delete_value_returns_what_it_removed_or_null_when_there_was_nothing()
 {
    let db = db().await;
    ok(repo::write(&db, |tx| upsert(tx, 11, 2, "7")).await);
    assert_eq!(
        ok(repo::write(&db, |tx| ccf::delete_value(tx, 11, 2)).await).as_deref(),
        Some("7")
    );
    assert_eq!(
        ok(repo::write(&db, |tx| ccf::delete_value(tx, 11, 2)).await),
        None
    );
}

/// Port of: packages/database/__tests__/integration/contact-custom-field-pair-lock.test.ts:205 "contactCustomFieldRepository pair locks > a rolled-back transaction leaves the value untouched"
#[tokio::test]
async fn contact_custom_field_repository_pair_locks_a_rolled_back_transaction_leaves_the_value_untouched()
 {
    let db = db().await;
    let r = repo::write(&db, |tx| {
        upsert(tx, 11, 1, "9")?;
        Err::<(), _>(Error::msg("boom"))
    })
    .await;
    assert_eq!(repo_common::err_message(r), "boom");
    assert_eq!(read_value(&db, 11, 1).await, None);
}
