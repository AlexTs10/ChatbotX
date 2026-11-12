//! Port of `packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts`
//! (P3.3). The mocked tx becomes real `MessengerWelcomeMessageFlow` rows; the asserted
//! call order inside `replaceCurrent`'s transaction is checked as the final rows of one
//! write job (the partial unique index on `current` would reject the other order).

mod common;
#[path = "common/repo3.rs"]
mod repo3;
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::messenger_welcome_message_flow::{
    CurrentRowInput, PreviousAction, delete_by_ids, find_current, insert_current, list_all_by_flow,
    list_current_by_flow, mark_superseded, replace_current,
};
use chatbotx_db::repo::models::MessengerWelcomeMessageFlow;
use chatbotx_db::repo::{self, find_many};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
const FLOW: i64 = 61;
const PAGE: i64 = 11;

async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1009, OTHER).await;
    setup(&db, |tx| {
        repo3::flow(tx, FLOW, WS)?;
        repo3::flow(tx, 62, WS)?;
        repo3::integration_messenger(tx, PAGE, WS, 3101, "[]")?;
        repo3::integration_messenger(tx, 13, WS, 3103, "[]")?;
        repo3::integration_messenger(tx, 12, OTHER, 3102, "[]")?;
        Ok(())
    })
    .await;
    db
}

fn current(workspace_id: i64, flow_id: i64, page: i64, meta: &str) -> CurrentRowInput {
    CurrentRowInput {
        workspace_id,
        flow_id,
        integration_messenger_id: page,
        page_id: "p".to_owned(),
        meta_flow_id: meta.to_owned(),
        flow_version_id: 4,
    }
}

async fn put(db: &common::TestDb, row: CurrentRowInput) {
    ok(repo::write(db, move |tx| insert_current(tx, &row)).await);
}

async fn all(db: &common::TestDb) -> Vec<MessengerWelcomeMessageFlow> {
    ok(repo::read(db, |c| find_many(c, "ORDER BY \"id\"", &[])).await)
}

/// Port of: packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts:9 "messengerWelcomeMessageFlowRepository > insertCurrent writes a current row with every column"
#[tokio::test]
async fn messenger_welcome_message_flow_repository_insert_current_writes_a_current_row_with_every_column()
 {
    let db = seeded().await;
    put(&db, current(WS, FLOW, PAGE, "m")).await;
    let rows = all(&db).await;
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(
        (
            r.workspace_id,
            r.flow_id,
            r.integration_messenger_id,
            r.page_id.as_str(),
            r.meta_flow_id.as_str(),
            r.flow_version_id,
            r.status.as_str()
        ),
        (WS, FLOW, PAGE, "p", "m", Some(4), "current")
    );
}

/// Port of: packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts:35 "messengerWelcomeMessageFlowRepository > findCurrent looks up the current row of a flow on one page"
#[tokio::test]
async fn messenger_welcome_message_flow_repository_find_current_looks_up_the_current_row_of_a_flow_on_one_page()
 {
    let db = seeded().await;
    put(&db, current(WS, FLOW, PAGE, "old")).await;
    let old = all(&db).await[0].id;
    ok(repo::write(&db, move |tx| mark_superseded(tx, old)).await);
    put(&db, current(WS, FLOW, PAGE, "new")).await;
    put(&db, current(WS, FLOW, 13, "other-page")).await;
    let found = ok(repo::read(&db, |c| find_current(c, FLOW, PAGE)).await).expect("current");
    assert_eq!(found.meta_flow_id, "new");
    assert_eq!(
        ok(repo::read(&db, |c| find_current(c, 62, PAGE)).await),
        None
    );
}

/// Port of: packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts:46 "messengerWelcomeMessageFlowRepository > listCurrentByFlow scopes by workspace, flow and status"
#[tokio::test]
async fn messenger_welcome_message_flow_repository_list_current_by_flow_scopes_by_workspace_flow_and_status()
 {
    let db = seeded().await;
    put(&db, current(WS, FLOW, PAGE, "a")).await;
    put(&db, current(WS, FLOW, 13, "b")).await;
    put(&db, current(WS, 62, PAGE, "other-flow")).await;
    let b = all(&db).await[1].id;
    ok(repo::write(&db, move |tx| mark_superseded(tx, b)).await);
    put(&db, current(WS, FLOW, 13, "c")).await;
    let rows = ok(repo::read(&db, |c| list_current_by_flow(c, WS, FLOW)).await);
    assert_eq!(
        rows.iter()
            .map(|r| (r.integration_messenger_id, r.meta_flow_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(PAGE, "a"), (13, "c")]
    );
    assert!(ok(repo::read(&db, |c| list_current_by_flow(c, OTHER, FLOW)).await).is_empty());
}

/// Port of: packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts:59 "messengerWelcomeMessageFlowRepository > listAllByFlow returns current and superseded rows of a flow"
#[tokio::test]
async fn messenger_welcome_message_flow_repository_list_all_by_flow_returns_current_and_superseded_rows_of_a_flow()
 {
    let db = seeded().await;
    put(&db, current(WS, FLOW, PAGE, "a")).await;
    let a = all(&db).await[0].id;
    ok(repo::write(&db, move |tx| mark_superseded(tx, a)).await);
    put(&db, current(WS, FLOW, PAGE, "b")).await;
    put(&db, current(WS, 62, PAGE, "other-flow")).await;
    let mut metas: Vec<String> = ok(repo::read(&db, |c| list_all_by_flow(c, WS, FLOW)).await)
        .into_iter()
        .map(|r| r.meta_flow_id)
        .collect();
    metas.sort();
    assert_eq!(metas, vec!["a", "b"]);
}

/// Port of: packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts:70 "messengerWelcomeMessageFlowRepository > markSuperseded flips the status"
#[tokio::test]
async fn messenger_welcome_message_flow_repository_mark_superseded_flips_the_status() {
    let db = seeded().await;
    put(&db, current(WS, FLOW, PAGE, "a")).await;
    put(&db, current(WS, FLOW, 13, "b")).await;
    let a = all(&db).await[0].id;
    ok(repo::write(&db, move |tx| mark_superseded(tx, a)).await);
    let statuses: Vec<String> = all(&db).await.into_iter().map(|r| r.status).collect();
    assert_eq!(statuses, vec!["superseded", "current"]);
}

/// Port of: packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts:79 "messengerWelcomeMessageFlowRepository > replaceCurrent supersedes the previous row then inserts, in one transaction"
#[tokio::test]
async fn messenger_welcome_message_flow_repository_replace_current_supersedes_the_previous_row_then_inserts_in_one_transaction()
 {
    let db = seeded().await;
    put(&db, current(WS, FLOW, PAGE, "m1")).await;
    let prev = all(&db).await[0].id;
    ok(repo::write(&db, move |tx| {
        replace_current(
            tx,
            prev,
            PreviousAction::Supersede,
            &current(WS, FLOW, PAGE, "m2"),
        )
    })
    .await);
    let rows: Vec<(String, String)> = all(&db)
        .await
        .into_iter()
        .map(|r| (r.meta_flow_id, r.status))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("m1".to_owned(), "superseded".to_owned()),
            ("m2".to_owned(), "current".to_owned())
        ]
    );
}

/// Port of: packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts:115 "messengerWelcomeMessageFlowRepository > replaceCurrent deletes the previous row when it is gone from Meta"
#[tokio::test]
async fn messenger_welcome_message_flow_repository_replace_current_deletes_the_previous_row_when_it_is_gone_from_meta()
 {
    let db = seeded().await;
    put(&db, current(WS, FLOW, PAGE, "m1")).await;
    let prev = all(&db).await[0].id;
    ok(repo::write(&db, move |tx| {
        replace_current(
            tx,
            prev,
            PreviousAction::Delete,
            &current(WS, FLOW, PAGE, "m2"),
        )
    })
    .await);
    let rows: Vec<(String, String)> = all(&db)
        .await
        .into_iter()
        .map(|r| (r.meta_flow_id, r.status))
        .collect();
    assert_eq!(rows, vec![("m2".to_owned(), "current".to_owned())]);
}

/// Port of: packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts:145 "messengerWelcomeMessageFlowRepository > deleteByIds skips the query for an empty list"
#[tokio::test]
async fn messenger_welcome_message_flow_repository_delete_by_ids_skips_the_query_for_an_empty_list()
{
    let db = seeded().await;
    put(&db, current(WS, FLOW, PAGE, "a")).await;
    ok(repo::write(&db, |tx| delete_by_ids(tx, &[])).await);
    assert_eq!(all(&db).await.len(), 1);
}

/// Port of: packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts:151 "messengerWelcomeMessageFlowRepository > deleteByIds deletes the given rows"
#[tokio::test]
async fn messenger_welcome_message_flow_repository_delete_by_ids_deletes_the_given_rows() {
    let db = seeded().await;
    put(&db, current(WS, FLOW, PAGE, "a")).await;
    put(&db, current(WS, FLOW, 13, "b")).await;
    put(&db, current(WS, 62, PAGE, "c")).await;
    let ids: Vec<i64> = all(&db).await.iter().map(|r| r.id).collect();
    let (a, b) = (ids[0], ids[1]);
    ok(repo::write(&db, move |tx| delete_by_ids(tx, &[a, b])).await);
    let left: Vec<String> = all(&db).await.into_iter().map(|r| r.meta_flow_id).collect();
    assert_eq!(left, vec!["c"]);
}
