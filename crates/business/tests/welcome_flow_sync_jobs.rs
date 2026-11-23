//! `syncWelcomeMessageFlows` / `deleteWelcomeMessageFlows` (Q.P7-V5):
//! `welcome-message-flows-handler.test.ts` against a scripted Graph upstream.
//! The mock-call assertions of the original are checked on the requests sent
//! and the `MessengerWelcomeMessageFlow` rows left behind; a failing row write
//! is a test trigger. The lock wait time has no Rust counterpart (an
//! in-process lock waits without a timeout).

mod channel_fake;
mod common;

use channel_fake::{FakeHttp, channel_row, graph_error, ok};
use chatbotx_business::connection::credentials::{UpstreamReply, UpstreamRequest};
use chatbotx_business::welcome_flow_sync::{delete, format_name, sync};
use chatbotx_core::flow_config as fc;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use common::{OTHER_WS, WS, rows, setup, two_workspaces};
use serde_json::{Value, json};

const FLOW: i64 = 7;
const PAGE_A: i64 = 100;
const PAGE_B: i64 = 101;
const LATEST: i64 = 73;

fn start_node(texts: &[&str]) -> Value {
    let mut node =
        fc::send_message_node_default_fn(&json!({ "dataProps": { "isStartNode": true } }));
    let steps: Vec<Value> = texts
        .iter()
        .map(|t| fc::send_text_step_default_fn(&json!({ "text": t })))
        .collect();
    node["data"]["details"]["steps"] = json!(steps);
    node["data"]["details"]["quickReplies"] = json!([]);
    node
}

fn version(
    tx: &Connection,
    id: i64,
    nodes: &Value,
    draft: bool,
    latest: bool,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
           VALUES (?1, ?2, ?3, ?4, '[]', ?5, ?6, 1)"#,
        params![id, WS, FLOW, nodes.to_string(), draft, latest],
    )?;
    Ok(())
}

fn page(tx: &Connection, id: i64, ws: i64, page_id: &str) -> Result<(), DbError> {
    let auth = json!({ "tokens": { "accessToken": "tok" }, "version": "v23.0" });
    channel_row(tx, "IntegrationMessenger", id, ws, page_id, &auth)
}

fn recorded(tx: &Connection, id: i64, integration: i64, meta_flow_id: &str) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "MessengerWelcomeMessageFlow" ("id", "workspaceId", "flowId", "integrationMessengerId", "pageId", "metaFlowId", "flowVersionId")
           VALUES (?1, ?2, ?3, ?4, 'p', ?5, 71)"#,
        params![id, WS, FLOW, integration, meta_flow_id],
    )?;
    Ok(())
}

/// Flow 7 "Signup" with published versions 71, 72 and 73 (latest, one "Hi"
/// message); page A (`pa`).
async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (?1, ?2, 'Signup')"#,
            params![FLOW, WS],
        )?;
        let node = json!([start_node(&["Hi"])]);
        version(tx, 71, &node, false, false)?;
        version(tx, 72, &node, false, false)?;
        version(tx, LATEST, &node, false, true)?;
        page(tx, PAGE_A, WS, "pa")
    })
    .await;
    db
}

async fn with_page_b(db: &Db) {
    setup(db, |tx| page(tx, PAGE_B, WS, "pb")).await;
}

/// Makes every `MessengerWelcomeMessageFlow` insert fail ("db down").
async fn break_inserts(db: &Db) {
    setup(db, |tx| {
        tx.execute_batch(
            r#"CREATE TRIGGER "test_db_down" BEFORE INSERT ON "MessengerWelcomeMessageFlow"
               BEGIN SELECT RAISE(ABORT, 'db down'); END;"#,
        )?;
        Ok(())
    })
    .await;
}

fn kind(r: &UpstreamRequest) -> &'static str {
    match r.method {
        "GET" => "get",
        "DELETE" => "delete",
        _ if r.body.as_deref().unwrap_or("").contains("\"flow_id\"") => "update",
        _ => "create",
    }
}

fn body(r: &UpstreamRequest) -> Value {
    serde_json::from_str(r.body.as_deref().unwrap_or("null")).unwrap()
}

/// Every call succeeds; a create answers `meta-new`; a lookup says "in an ad".
fn meta(
    overrides: impl Fn(&UpstreamRequest) -> Option<UpstreamReply> + Send + Sync + 'static,
) -> FakeHttp {
    FakeHttp::new(move |r| {
        overrides(r).unwrap_or_else(|| match kind(r) {
            "create" => ok(json!({ "flow_id": "meta-new" })),
            "get" => ok(json!({ "data": [{ "id": "meta1", "is_used_in_ad": true }] })),
            _ => ok(json!({ "success": true })),
        })
    })
}

fn calls(http: &FakeHttp) -> Vec<&'static str> {
    http.seen().iter().map(kind).collect()
}

fn job(reason: &str) -> Value {
    json!({ "workspaceId": WS.to_string(), "flowId": FLOW.to_string(), "reason": reason })
}

/// `(id, integration, metaFlowId, status, flowVersionId)`, by id.
async fn flow_rows(db: &Db) -> Vec<(i64, i64, String, String, Option<i64>)> {
    rows(
        db,
        r#"SELECT "id", "integrationMessengerId", "metaFlowId", "status", "flowVersionId"
           FROM "MessengerWelcomeMessageFlow" WHERE "workspaceId" = ?1 ORDER BY "id""#,
        WS,
    )
    .await
    .into_iter()
    .map(|r| {
        (
            r["id"].as_i64().unwrap(),
            r["integrationMessengerId"].as_i64().unwrap(),
            r["metaFlowId"].as_str().unwrap().to_owned(),
            r["status"].as_str().unwrap().to_owned(),
            r["flowVersionId"].as_i64(),
        )
    })
    .collect()
}

fn current_on(rows: &[(i64, i64, String, String, Option<i64>)], page: i64) -> Vec<String> {
    rows.iter()
        .filter(|r| r.1 == page && r.3 == "current")
        .map(|r| r.2.clone())
        .collect()
}

#[test]
fn format_welcome_message_flow_name_appends_the_version() {
    assert_eq!(format_name("Signup", 3), "Signup (v3)");
}

// ---- publish -----------------------------------------------------------------------

#[tokio::test]
async fn creates_a_meta_flow_named_with_the_version_and_stores_it_as_current() {
    let db = seeded().await;
    let http = meta(|_| None);
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    let seen = http.seen();
    assert_eq!(calls(&http), vec!["create"]);
    assert!(
        seen[0].url.ends_with("/v23.0/pa/welcome_message_flows"),
        "{}",
        seen[0].url
    );
    let sent = body(&seen[0]);
    assert_eq!(sent["name"], "Signup (v3)");
    assert_eq!(sent["eligible_platforms"], json!(["messenger"]));
    let messages: Value =
        serde_json::from_str(sent["welcome_message_flow"].as_str().unwrap()).unwrap();
    assert_eq!(messages[0]["message"]["text"], "Hi", "{messages}");
    let rows = flow_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].1, rows[0].2.as_str(), rows[0].3.as_str(), rows[0].4),
        (PAGE_A, "meta-new", "current", Some(LATEST))
    );
}

#[tokio::test]
async fn updates_an_existing_meta_flow_in_place() {
    let db = seeded().await;
    setup(&db, |tx| recorded(tx, 1, PAGE_A, "meta1")).await;
    let http = meta(|_| None);
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    assert_eq!(calls(&http), vec!["update"]);
    let sent = body(&http.seen()[0]);
    assert_eq!(
        (sent["flow_id"].as_str(), sent["name"].as_str()),
        (Some("meta1"), Some("Signup (v3)"))
    );
    assert!(sent["welcome_message_flow"].is_string());
    assert_eq!(
        flow_rows(&db).await,
        vec![(1, PAGE_A, "meta1".into(), "current".into(), Some(LATEST))]
    );
}

#[tokio::test]
async fn an_update_failure_on_a_flow_used_in_an_ad_supersedes_the_row_and_creates_a_new_flow() {
    let db = seeded().await;
    setup(&db, |tx| recorded(tx, 1, PAGE_A, "meta1")).await;
    let http = meta(|r| (kind(r) == "update").then(|| graph_error(400, "used in ad", 100, None)));
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    assert_eq!(calls(&http), vec!["update", "get", "create"]);
    assert!(http.seen()[1].url.contains("flow_id=meta1"));
    let rows = flow_rows(&db).await;
    assert_eq!(rows[0].3, "superseded");
    assert_eq!(current_on(&rows, PAGE_A), vec!["meta-new".to_owned()]);
}

#[tokio::test]
async fn a_failed_create_after_a_used_in_ad_update_failure_keeps_the_old_row_current() {
    let db = seeded().await;
    setup(&db, |tx| recorded(tx, 1, PAGE_A, "meta1")).await;
    let http = meta(|r| match kind(r) {
        "update" => Some(graph_error(400, "used in ad", 100, None)),
        "create" => Some(graph_error(400, "Meta down", 100, None)),
        _ => None,
    });
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    assert_eq!(
        flow_rows(&db).await,
        vec![(1, PAGE_A, "meta1".into(), "current".into(), Some(71))]
    );
}

#[tokio::test]
async fn a_failed_row_swap_deletes_the_just_created_meta_flow() {
    let db = seeded().await;
    setup(&db, |tx| recorded(tx, 1, PAGE_A, "meta1")).await;
    break_inserts(&db).await;
    let http = meta(|r| (kind(r) == "update").then(|| graph_error(400, "used in ad", 100, None)));
    let _ = sync(&db, &http, &job("publish")).await;
    assert_eq!(calls(&http), vec!["update", "get", "create", "delete"]);
    assert_eq!(body(&http.seen()[3]), json!({ "flow_id": "meta-new" }));
    assert_eq!(
        current_on(&flow_rows(&db).await, PAGE_A),
        vec!["meta1".to_owned()]
    );
}

#[tokio::test]
async fn a_failed_insert_for_a_first_time_page_deletes_the_just_created_meta_flow() {
    let db = seeded().await;
    break_inserts(&db).await;
    let http = meta(|_| None);
    let _ = sync(&db, &http, &job("publish")).await;
    assert_eq!(calls(&http), vec!["create", "delete"]);
    assert_eq!(body(&http.seen()[1]), json!({ "flow_id": "meta-new" }));
    assert!(flow_rows(&db).await.is_empty());
}

#[tokio::test]
async fn an_update_failure_on_a_flow_still_editable_keeps_the_row_and_creates_nothing() {
    let db = seeded().await;
    setup(&db, |tx| recorded(tx, 1, PAGE_A, "meta1")).await;
    let http = meta(|r| match kind(r) {
        "update" => Some(graph_error(400, "rate limited", 100, None)),
        "get" => Some(ok(
            json!({ "data": [{ "id": "meta1", "is_used_in_ad": false }] }),
        )),
        _ => None,
    });
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    assert_eq!(calls(&http), vec!["update", "get"]);
    assert_eq!(
        flow_rows(&db).await,
        vec![(1, PAGE_A, "meta1".into(), "current".into(), Some(71))]
    );
}

#[tokio::test]
async fn an_update_failure_on_a_flow_gone_from_meta_drops_the_row_and_creates_a_new_flow() {
    let db = seeded().await;
    setup(&db, |tx| recorded(tx, 1, PAGE_A, "meta1")).await;
    let http = meta(|r| match kind(r) {
        "update" => Some(graph_error(400, "not found", 100, None)),
        "get" => Some(graph_error(400, "does not exist", 100, Some(33))),
        _ => None,
    });
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    let rows = flow_rows(&db).await;
    assert_eq!(
        rows.len(),
        1,
        "the old row is deleted, not superseded: {rows:?}"
    );
    assert_eq!(current_on(&rows, PAGE_A), vec!["meta-new".to_owned()]);
}

#[tokio::test]
async fn a_failed_lookup_after_a_failed_update_leaves_the_row_untouched() {
    let db = seeded().await;
    setup(&db, |tx| recorded(tx, 1, PAGE_A, "meta1")).await;
    let http = meta(|r| match kind(r) {
        "update" | "get" => Some(graph_error(400, "5xx", 100, None)),
        _ => None,
    });
    let _ = sync(&db, &http, &job("publish")).await;
    assert_eq!(calls(&http), vec!["update", "get"]);
    assert_eq!(
        flow_rows(&db).await,
        vec![(1, PAGE_A, "meta1".into(), "current".into(), Some(71))]
    );
}

#[tokio::test]
async fn a_transient_page_failure_fails_after_every_page_ran_so_the_job_retries() {
    let db = seeded().await;
    with_page_b(&db).await;
    let http = meta(|r| {
        r.url
            .contains("/pa/")
            .then(|| graph_error(503, "rate limited", 2, None))
    });
    assert!(sync(&db, &http, &job("publish")).await.is_err());
    let rows = flow_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, PAGE_B);
}

#[tokio::test]
async fn one_failing_page_does_not_stop_the_others() {
    let db = seeded().await;
    with_page_b(&db).await;
    let http = meta(|r| {
        r.url
            .contains("/pa/")
            .then(|| graph_error(400, "token expired", 190, None))
    });
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    let rows = flow_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, PAGE_B);
}

async fn zero_messages(db: &Db) {
    setup(db, |tx| {
        tx.execute(
            r#"UPDATE "FlowVersion" SET "nodes" = ?1 WHERE "id" = ?2"#,
            params![json!([start_node(&[])]).to_string(), LATEST],
        )?;
        recorded(tx, 1, PAGE_A, "meta1")
    })
    .await;
}

#[tokio::test]
async fn zero_messages_deletes_the_current_meta_flow_and_creates_nothing() {
    let db = seeded().await;
    zero_messages(&db).await;
    let http = meta(|_| None);
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    assert_eq!(calls(&http), vec!["delete"]);
    assert_eq!(body(&http.seen()[0]), json!({ "flow_id": "meta1" }));
    assert!(flow_rows(&db).await.is_empty());
}

#[tokio::test]
async fn zero_messages_with_a_failed_delete_keeps_the_row_as_superseded() {
    let db = seeded().await;
    zero_messages(&db).await;
    let http = meta(|r| (kind(r) == "delete").then(|| graph_error(400, "used in ad", 100, None)));
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    assert_eq!(flow_rows(&db).await[0].3, "superseded");
}

#[tokio::test]
async fn names_a_restored_version_by_its_own_number() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "FlowVersion" SET "isLatest" = ("id" = 72) WHERE "flowId" = ?1"#,
            params![FLOW],
        )?;
        recorded(tx, 1, PAGE_A, "meta1")
    })
    .await;
    let http = meta(|_| None);
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    assert_eq!(body(&http.seen()[0])["name"], "Signup (v2)");
}

#[tokio::test]
async fn every_page_gets_the_same_version_name() {
    let db = seeded().await;
    with_page_b(&db).await;
    let http = meta(|_| None);
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    let names: Vec<Value> = http
        .seen()
        .iter()
        .map(|r| body(r)["name"].clone())
        .collect();
    assert_eq!(names, vec![json!("Signup (v3)"), json!("Signup (v3)")]);
}

#[tokio::test]
async fn no_start_node_is_treated_as_zero_messages() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "FlowVersion" SET "nodes" = '[]' WHERE "id" = ?1"#,
            params![LATEST],
        )?;
        Ok(())
    })
    .await;
    let http = meta(|_| None);
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    assert!(http.seen().is_empty());
    assert!(flow_rows(&db).await.is_empty());
}

#[tokio::test]
async fn a_deleted_or_unpublished_flow_makes_no_graph_calls() {
    let db = seeded().await;
    let http = meta(|_| None);
    let mut gone = job("publish");
    gone["flowId"] = json!("999");
    assert_eq!(sync(&db, &http, &gone).await, Ok(()));
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "FlowVersion" SET "isDraft" = 1 WHERE "flowId" = ?1"#,
            params![FLOW],
        )?;
        Ok(())
    })
    .await;
    assert_eq!(sync(&db, &http, &job("publish")).await, Ok(()));
    assert!(http.seen().is_empty());
}

// ---- rename ------------------------------------------------------------------------

#[tokio::test]
async fn rename_updates_the_name_only_on_pages_that_already_have_a_row() {
    let db = seeded().await;
    with_page_b(&db).await;
    setup(&db, |tx| recorded(tx, 1, PAGE_A, "meta1")).await;
    let http = meta(|_| None);
    assert_eq!(sync(&db, &http, &job("rename")).await, Ok(()));
    assert_eq!(calls(&http), vec!["update"]);
    assert_eq!(
        body(&http.seen()[0]),
        json!({ "flow_id": "meta1", "name": "Signup (v3)" })
    );
}

#[tokio::test]
async fn rename_of_a_never_synced_flow_makes_zero_graph_calls() {
    let db = seeded().await;
    let http = meta(|_| None);
    assert_eq!(sync(&db, &http, &job("rename")).await, Ok(()));
    assert!(http.seen().is_empty());
}

#[tokio::test]
async fn a_failed_rename_is_logged_and_never_falls_back_to_create() {
    let db = seeded().await;
    setup(&db, |tx| recorded(tx, 1, PAGE_A, "meta1")).await;
    let http = meta(|_| Some(graph_error(400, "used in ad", 100, None)));
    assert_eq!(sync(&db, &http, &job("rename")).await, Ok(()));
    assert_eq!(calls(&http), vec!["update"]);
    assert_eq!(flow_rows(&db).await[0].3, "current");
}

#[tokio::test]
async fn a_transient_rename_failure_fails_so_the_job_retries() {
    let db = seeded().await;
    setup(&db, |tx| recorded(tx, 1, PAGE_A, "meta1")).await;
    let http = meta(|_| Some(graph_error(400, "rate limited", 613, None)));
    assert!(sync(&db, &http, &job("rename")).await.is_err());
}

// ---- delete ------------------------------------------------------------------------

#[tokio::test]
async fn delete_removes_every_row_whose_meta_delete_succeeded() {
    let db = seeded().await;
    setup(&db, |tx| {
        // One `current` row per (flow, page): the superseded one goes in first.
        recorded(tx, 2, PAGE_A, "m2")?;
        tx.execute(
            r#"UPDATE "MessengerWelcomeMessageFlow" SET "status" = 'superseded' WHERE "id" = 2"#,
            [],
        )?;
        recorded(tx, 1, PAGE_A, "m1")
    })
    .await;
    let http = meta(|r| {
        body(r)["flow_id"]
            .eq("m2")
            .then(|| graph_error(400, "used in ad", 100, None))
    });
    let data = json!({ "workspaceId": WS.to_string(), "flowIds": [FLOW.to_string()] });
    delete(&db, &http, &data).await.expect("delete job");
    assert_eq!(calls(&http), vec!["delete", "delete"]);
    let left: Vec<i64> = flow_rows(&db).await.iter().map(|r| r.0).collect();
    assert_eq!(left, vec![2]);
}

#[tokio::test]
async fn delete_skips_rows_whose_integration_is_in_another_workspace() {
    let db = seeded().await;
    setup(&db, |tx| {
        page(tx, 300, OTHER_WS, "px")?;
        recorded(tx, 1, 300, "m1")
    })
    .await;
    let http = meta(|_| None);
    let data = json!({ "workspaceId": WS.to_string(), "flowIds": [FLOW.to_string()] });
    delete(&db, &http, &data).await.expect("delete job");
    assert!(http.seen().is_empty());
    assert_eq!(flow_rows(&db).await.len(), 1);
}
