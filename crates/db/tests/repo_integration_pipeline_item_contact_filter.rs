//! Port of `packages/database/__tests__/integration/pipeline-item-contact-filter.test.ts`
//! (P3.2b slice D2): the `pipelineItem` truth table on a migrated SQLite temp file (one fresh
//! file per test instead of a rolled-back transaction). Items are written with the ported
//! `nextItemNumber` + `insertIdempotent`, as the TS.

mod common;
#[path = "common/repo7.rs"]
mod repo7;

use chatbotx_db::repo;
use chatbotx_db::repo::pipeline::next_item_number;
use chatbotx_db::repo::pipeline_item::insert_idempotent;
use chatbotx_db::repo::queries::contact_filter::{ContactWhereInput, build_contact_where};
use chatbotx_db::rusqlite::{self, params};
use chatbotx_db::{Db, DbError};
use repo7::{put_pipeline, put_stage, put_user};
use serde_json::{Value as Json, json};

/// `2026-06-01T00:00:00.000Z` (no condition here reads `now`).
const NOW: i64 = 1_780_272_000_000;

/// The TS `World` (ids of world `n`: workspace `100 n`, contacts `100 n + 1..4`).
#[derive(Debug, Clone, Copy)]
struct World {
    workspace_id: i64,
    sales_id: i64,
    support_id: i64,
    sales_stages: [i64; 2],
    support_stage: i64,
    open_high: i64,
    won_low: i64,
    no_item: i64,
    support_open: i64,
}

impl World {
    const fn ids(n: i64) -> World {
        let base = 100 * n;
        World {
            workspace_id: base,
            sales_id: base + 10,
            support_id: base + 20,
            sales_stages: [base + 11, base + 12],
            support_stage: base + 21,
            open_high: base + 1,
            won_low: base + 2,
            no_item: base + 3,
            support_open: base + 4,
        }
    }
}

fn item(
    tx: &rusqlite::Connection,
    w: World,
    pipeline_id: i64,
    stage_id: i64,
    contact_id: i64,
    status: &str,
    priority: &str,
) -> repo::Result<()> {
    let fx = repo7::PipeFx {
        user_id: 0,
        workspace_id: w.workspace_id,
        pipeline_id,
        stage_ids: [stage_id, stage_id],
        contact_ids: [contact_id, contact_id],
    };
    let mut values = repo7::item_values(fx);
    values.status = status.to_owned();
    values.priority = priority.to_owned();
    values.number = next_item_number(tx, w.workspace_id)?;
    values.title = format!("Item {}", values.number);
    insert_idempotent(tx, &values)?;
    Ok(())
}

/// The TS `seedWorld` rows of world `n` (without the items).
fn seed_rows(tx: &rusqlite::Connection, w: World, n: i64) -> Result<(), DbError> {
    put_user(tx, n)?;
    tx.execute(
        r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'pif', ?2)"#,
        params![w.workspace_id, n],
    )?;
    for contact in [w.open_high, w.won_low, w.no_item, w.support_open] {
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (?1, ?2)"#,
            params![contact, w.workspace_id],
        )?;
    }
    put_pipeline(tx, w.sales_id, w.workspace_id, "sales")?;
    put_stage(tx, w.sales_stages[0], w.workspace_id, w.sales_id, "Lead", 0)?;
    put_stage(tx, w.sales_stages[1], w.workspace_id, w.sales_id, "Won", 1)?;
    put_pipeline(tx, w.support_id, w.workspace_id, "support")?;
    put_stage(
        tx,
        w.support_stage,
        w.workspace_id,
        w.support_id,
        "Inbox",
        0,
    )?;
    Ok(())
}

/// The TS `seedWorld` for world `n`.
async fn seed_world(db: &Db, n: i64) -> World {
    let w = World::ids(n);
    db.write(move |tx| seed_rows(tx, w, n))
        .await
        .expect("seed world");
    repo::write(db, move |tx| {
        item(
            tx,
            w,
            w.sales_id,
            w.sales_stages[0],
            w.open_high,
            "open",
            "high",
        )?;
        item(
            tx,
            w,
            w.sales_id,
            w.sales_stages[1],
            w.won_low,
            "won",
            "low",
        )?;
        item(
            tx,
            w,
            w.support_id,
            w.support_stage,
            w.support_open,
            "open",
            "medium",
        )
    })
    .await
    .expect("seed items");
    w
}

/// The TS `run`: a fresh file with world 1.
async fn run() -> (common::TestDb, World) {
    let db = common::fresh_db().await;
    let w = seed_world(&db, 1).await;
    (db, w)
}

/// `matching(tx, world, condition)`: `buildContactWhere` over `"Contact"`, sorted ids.
async fn matching(db: &Db, w: World, condition: Json) -> Vec<i64> {
    let mut c = condition;
    c["field"] = json!("pipelineItem");
    let input = ContactWhereInput {
        workspace_id: w.workspace_id,
        keyword: None,
        contact_filter: Some(
            serde_json::from_value(json!({ "operator": "and", "conditions": [c] }))
                .expect("criteria"),
        ),
        now: NOW,
    };
    let q = build_contact_where(&input);
    let sql = format!(
        "SELECT \"Contact\".\"id\" FROM \"Contact\" WHERE {} ORDER BY 1",
        q.text
    );
    db.read(move |conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(q.params.iter()), |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(rows)
    })
    .await
    .expect("matching")
}

fn sorted(ids: &[i64]) -> Vec<i64> {
    let mut out = ids.to_vec();
    out.sort_unstable();
    out
}

fn id(v: i64) -> Json {
    json!(v.to_string())
}

// ts:224 ported
#[tokio::test]
async fn exists_has_has_no_item_any_pipeline_or_one_any_status_or_one() {
    let (db, w) = run().await;
    let has_any = matching(
        &db,
        w,
        json!({ "attribute": "exists", "pipelineId": null, "operator": "isNotEmpty", "value": "any" }),
    )
    .await;
    assert_eq!(has_any, sorted(&[w.open_high, w.won_low, w.support_open]));
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "exists", "pipelineId": null, "operator": "isEmpty", "value": "any" }),
        )
        .await,
        vec![w.no_item]
    );
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "exists", "pipelineId": id(w.sales_id), "operator": "isNotEmpty", "value": "open" }),
        )
        .await,
        vec![w.open_high]
    );
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "exists", "pipelineId": id(w.sales_id), "operator": "isEmpty", "value": "open" }),
        )
        .await,
        sorted(&[w.won_low, w.no_item, w.support_open])
    );
}

// ts:262 ported
#[tokio::test]
async fn status_in_matches_not_in_also_matches_contacts_without_an_item() {
    let (db, w) = run().await;
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "status", "pipelineId": null, "operator": "in", "value": ["won"] }),
        )
        .await,
        vec![w.won_low]
    );
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "status", "pipelineId": null, "operator": "notIn", "value": ["won"] }),
        )
        .await,
        sorted(&[w.open_high, w.no_item, w.support_open])
    );
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "status", "pipelineId": id(w.support_id), "operator": "in", "value": ["open", "won"] }),
        )
        .await,
        vec![w.support_open]
    );
}

// ts:292 ported
#[tokio::test]
async fn stage_needs_the_pipeline_not_in_includes_no_row_contacts() {
    let (db, w) = run().await;
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "stage", "pipelineId": id(w.sales_id), "operator": "in", "value": [id(w.sales_stages[0])] }),
        )
        .await,
        vec![w.open_high]
    );
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "stage", "pipelineId": id(w.sales_id), "operator": "notIn", "value": [id(w.sales_stages[0])] }),
        )
        .await,
        sorted(&[w.won_low, w.no_item, w.support_open])
    );
    // A stage of another pipeline matches nothing inside this pipeline.
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "stage", "pipelineId": id(w.sales_id), "operator": "in", "value": [id(w.support_stage)] }),
        )
        .await,
        Vec::<i64>::new()
    );
    // Without a pipeline the condition fails closed.
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "stage", "pipelineId": null, "operator": "in", "value": [id(w.sales_stages[0])] }),
        )
        .await,
        Vec::<i64>::new()
    );
}

// ts:333 ported
#[tokio::test]
async fn priority_in_not_in() {
    let (db, w) = run().await;
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "priority", "pipelineId": null, "operator": "in", "value": ["high", "low"] }),
        )
        .await,
        sorted(&[w.open_high, w.won_low])
    );
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "priority", "pipelineId": id(w.sales_id), "operator": "notIn", "value": ["high"] }),
        )
        .await,
        sorted(&[w.won_low, w.no_item, w.support_open])
    );
}

// ts:354 ported
#[tokio::test]
async fn a_pipeline_of_another_workspace_never_matches() {
    let (db, w) = run().await;
    let foreign = seed_world(&db, 2).await;
    assert_eq!(
        matching(
            &db,
            w,
            json!({ "attribute": "exists", "pipelineId": id(foreign.sales_id), "operator": "isNotEmpty", "value": "any" }),
        )
        .await,
        Vec::<i64>::new()
    );
}
