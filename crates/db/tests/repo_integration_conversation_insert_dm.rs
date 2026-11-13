//! Port of `packages/database/__tests__/integration/conversation-insert-dm.test.ts`
//! (P3.2b). Real SQL on a migrated temp file. The "two connections" case runs two write
//! jobs at once on the single writer (adapted): the second one runs after the first
//! commits and its inserts are skipped by `ON CONFLICT DO NOTHING`.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::conversation as conv;
use chatbotx_db::repo::models::Conversation;
use chatbotx_db::repo::{self};
use chatbotx_db::{Db, WriteTx};
use common::{fresh_db, seed};
use repo_common::{insert_contact, ok, setup};

const WS: i64 = 2001;

async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        for id in 4101..=4106 {
            insert_contact(tx, id, WS)?;
        }
        Ok(())
    })
    .await;
    db
}

async fn write<T, F>(db: &Db, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce(&mut WriteTx<'_>) -> repo::Result<T> + Send + 'static,
{
    ok(repo::write(db, f).await)
}

async fn insert(db: &Db, contact_ids: Vec<i64>) {
    write(db, move |c| {
        conv::insert_dm_conversations_if_absent(c, WS, &contact_ids)
    })
    .await;
}

async fn conversations_of(db: &Db, contact_id: i64) -> Vec<Conversation> {
    ok(repo::read(db, move |c| {
        repo::find_many::<Conversation>(
            c,
            "WHERE \"workspaceId\" = ?1 AND \"contactId\" = ?2 ORDER BY \"id\"",
            &[&WS, &contact_id],
        )
    })
    .await)
}

#[tokio::test]
async fn opens_one_dm_source_id_null_for_each_contact_that_has_none() {
    let db = fixture().await;
    insert(&db, vec![4101, 4102]).await;
    for contact in [4101, 4102] {
        let rows = conversations_of(&db, contact).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_id, None);
    }
}

#[tokio::test]
async fn opens_a_dm_next_to_a_comment_thread_without_touching_the_thread() {
    let db = fixture().await;
    setup(&db, |tx| {
        ins!(tx, "Conversation", {
            "id" => 91_i64, "workspaceId" => WS, "contactId" => 4103_i64,
            "sourceId" => "post-1".to_owned(),
        })
    })
    .await;
    insert(&db, vec![4103]).await;
    let mut sources: Vec<Option<String>> = conversations_of(&db, 4103)
        .await
        .into_iter()
        .map(|r| r.source_id)
        .collect();
    sources.sort();
    assert_eq!(sources, vec![None, Some("post-1".to_owned())]);
}

#[tokio::test]
async fn is_idempotent_an_existing_dm_is_kept_and_no_error_is_raised() {
    let db = fixture().await;
    insert(&db, vec![4104]).await;
    let first = conversations_of(&db, 4104).await;
    insert(&db, vec![4104]).await;
    let rows = conversations_of(&db, 4104).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, first[0].id);
}

#[tokio::test]
async fn two_connections_opening_the_same_dms_at_once_leave_exactly_one_dm_each() {
    let db = fixture().await;
    tokio::join!(insert(&db, vec![4105, 4106]), insert(&db, vec![4105, 4106]));
    for contact in [4105, 4106] {
        assert_eq!(conversations_of(&db, contact).await.len(), 1);
    }
}
