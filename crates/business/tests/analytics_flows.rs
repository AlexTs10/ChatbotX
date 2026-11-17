//! Flow analytics (Q.D11 slice 8b): `getFlowStats`, `resetStatsSession`
//! and the smart-delay node counts, with the seed and expectations of the
//! parity group p1-03 `analytics-flows.test.ts` (the flow versions are
//! written directly instead of through `flows.create`).

mod common;

use chatbotx_business::analytics::flows::{self, DelayCounts, NodeStats};
use chatbotx_db::rusqlite::{Connection, params};
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};

const FLOW: i64 = 40;
const SESSION: i64 = 41;
const PUBLISHED: i64 = 42;
const DRAFT: i64 = 43;

fn ms(iso: &str) -> i64 {
    chatbotx_core::js::parse_date(iso).expect("iso")
}

/// The `SPEC` flow: a send node with Yes / No buttons, a wait node, a plain
/// send node.
fn nodes() -> Value {
    json!([
        { "id": "n-buttons", "type": "sendMessage", "data": { "details": {
            "steps": [{ "id": "s1", "type": "text", "buttons": [{ "id": "yes" }, { "id": "no" }] }] } } },
        { "id": "n-wait", "type": "wait", "data": { "details": { "steps": [{ "id": "w1" }] } } },
        { "id": "n-plain", "type": "sendMessage", "data": { "details": { "steps": [{ "id": "s2", "type": "text" }] } } },
    ])
}

fn version(tx: &Connection, id: i64, flow: i64, draft: bool, nodes: &Value) {
    tx.execute(
        r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "startNodeId")
           VALUES (?1, ?2, ?3, ?4, '[]', ?5, 0)"#,
        params![id, WS, flow, nodes.to_string(), draft],
    )
    .unwrap();
}

/// K1 read after the delivery, K2 never read, K3 read before it, K4 failed.
async fn seeded(published: bool) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        common::flow(tx, FLOW, WS)?;
        version(tx, DRAFT, FLOW, true, &nodes());
        if published {
            version(tx, PUBLISHED, FLOW, false, &nodes());
            tx.execute(r#"UPDATE "Flow" SET "currentVersionId" = ?1 WHERE "id" = ?2"#, [PUBLISHED, FLOW])?;
        }
        tx.execute(r#"UPDATE "Flow" SET "draftVersionId" = ?1 WHERE "id" = ?2"#, [DRAFT, FLOW])?;
        tx.execute(
            r#"INSERT INTO "FlowAnalyticsSession" ("id", "flowId", "workspaceId") VALUES (?1, ?2, ?3)"#,
            [SESSION, FLOW, WS],
        )?;
        common::inbox(tx, 11, WS, "messenger")?;
        for (k, read) in [(1, Some("2025-06-02T00:00:00Z")), (2, None), (3, Some("2025-05-31T00:00:00Z")), (4, None)] {
            common::contact(tx, 100 + k, WS)?;
            common::contact_inbox(tx, 200 + k, 100 + k, 11, "messenger", &format!("psid-{k}"))?;
            common::conversation(tx, 300 + k, WS, 100 + k)?;
            tx.execute(
                r#"UPDATE "Conversation" SET "contactLastReadAt" = ?1 WHERE "id" = ?2"#,
                params![read.map(ms), 300 + k],
            )?;
        }
        let stat = |k: i64, node: &str, kind: &str, at: &str, button: Option<&str>| {
            tx.execute(
                r#"INSERT INTO "FlowNodeStat" ("workspaceId", "flowId", "analyticsId", "nodeId", "buttonId", "contactId", "contactInboxId", "eventType", "occurredAt")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
                params![WS, FLOW, SESSION, node, button, 100 + k, 200 + k, kind, ms(at)],
            )
        };
        let at = "2025-06-01T10:00:00Z";
        stat(1, "n-buttons", "message:delivered", at, None)?;
        stat(2, "n-buttons", "message:delivered", at, None)?;
        stat(3, "n-buttons", "message:delivered", at, None)?;
        stat(4, "n-buttons", "message:failed", at, None)?;
        stat(1, "n-buttons", "flow:clicked", "2025-06-01T10:01:00Z", Some("yes"))?;
        stat(1, "n-buttons", "flow:clicked", "2025-06-01T10:02:00Z", Some("yes"))?;
        stat(2, "n-buttons", "flow:clicked", "2025-06-01T10:03:00Z", Some("yes"))?;
        stat(3, "n-buttons", "flow:clicked", "2025-06-01T10:04:00Z", Some("no"))?;
        stat(1, "n-plain", "message:delivered", "2025-06-03T00:00:00Z", None)?;
        Ok(())
    })
    .await;
    db
}

async fn stats(db: &chatbotx_db::Db, ws: i64, flow: i64) -> Vec<(String, NodeStats)> {
    db.read(move |c| {
        flows::flow_stats(c, ws, flow).map_err(|e| chatbotx_db::DbError::other(e.to_string()))
    })
    .await
    .unwrap()
}

fn buttons(clicks: &[(&str, i64)]) -> Vec<(String, i64)> {
    clicks.iter().map(|(b, n)| (b.to_string(), *n)).collect()
}

fn zero(button_clicks: &[(&str, i64)]) -> NodeStats {
    NodeStats {
        buttons: buttons(button_clicks),
        ..NodeStats::default()
    }
}

#[tokio::test]
async fn aggregates_the_live_session_and_reset_starts_an_empty_one() {
    let db = seeded(true).await;
    assert_eq!(
        stats(&db, WS, FLOW).await,
        [
            (
                "n-buttons".to_string(),
                NodeStats {
                    sent: 4,
                    seen: 1,
                    delivered: 3,
                    clicked: 4,
                    failed: 1,
                    buttons: buttons(&[("yes", 2), ("no", 1)]),
                }
            ),
            (
                "n-plain".to_string(),
                NodeStats {
                    sent: 1,
                    delivered: 1,
                    ..NodeStats::default()
                }
            ),
        ]
    );
    db.write(|tx| {
        flows::reset_stats_session(tx, WS, FLOW, 1, 99)
            .map_err(|e| chatbotx_db::DbError::other(e.to_string()))
    })
    .await
    .unwrap();
    let open = common::scalar_i64(
        &db,
        r#"SELECT "id" FROM "FlowAnalyticsSession" WHERE "flowId" = ?1 AND "deletedAt" IS NULL"#,
        FLOW,
    )
    .await;
    assert_eq!(open, Some(99));
    assert_eq!(
        stats(&db, WS, FLOW).await,
        [
            ("n-buttons".to_string(), zero(&[("yes", 0), ("no", 0)])),
            ("n-plain".to_string(), zero(&[])),
        ]
    );
}

#[tokio::test]
async fn is_empty_for_a_draft_only_flow_an_unknown_flow_and_another_workspace() {
    let draft = seeded(false).await;
    assert!(stats(&draft, WS, FLOW).await.is_empty());
    let db = seeded(true).await;
    assert!(stats(&db, WS, 1).await.is_empty());
    assert!(stats(&db, OTHER_WS, FLOW).await.is_empty());
}

#[tokio::test]
async fn reset_of_an_unknown_or_foreign_flow_is_a_no_op() {
    let db = seeded(true).await;
    for (ws, flow) in [(OTHER_WS, FLOW), (WS, 1)] {
        db.write(move |tx| {
            flows::reset_stats_session(tx, ws, flow, 1, 99)
                .map_err(|e| chatbotx_db::DbError::other(e.to_string()))
        })
        .await
        .unwrap();
    }
    let open = common::scalar_i64(
        &db,
        r#"SELECT "id" FROM "FlowAnalyticsSession" WHERE "flowId" = ?1 AND "deletedAt" IS NULL"#,
        FLOW,
    )
    .await;
    assert_eq!(open, Some(SESSION));
}

#[tokio::test]
async fn smart_delay_counts_waiting_and_sent_contacts_per_wait_node() {
    let db = seeded(true).await;
    setup(&db, |tx| {
        for (status, step) in [
            ("pending", Some("w1")),
            ("pending", Some("w1")),
            ("scheduled", Some("w1")),
            ("completed", Some("w1")),
            ("completed", Some("w1")),
            ("completed", Some("w1")),
            ("canceled", Some("w1")),
            ("failed", Some("w1")),
            ("pending", Some("not-a-step-of-this-flow")),
            ("pending", None),
        ] {
            tx.execute(
                r#"INSERT INTO "ContactOnSmartDelay" ("workspaceId", "flowId", "flowVersionId", "contactInboxId", "conversationId", "nodeId", "stepId", "type", "status", "triggerAt")
                   VALUES (?1, ?2, ?3, 201, 301, 'n-wait', ?4, 'waitNode', ?5, 4070908800000)"#,
                params![WS, FLOW, DRAFT, step, status],
            )?;
        }
        Ok(())
    })
    .await;
    let nodes = nodes().as_array().unwrap().clone();
    let counts = db
        .read(move |c| {
            flows::smart_delay_stats(c, WS, FLOW, &nodes)
                .map_err(|e| chatbotx_db::DbError::other(e.to_string()))
        })
        .await
        .unwrap();
    assert_eq!(
        counts,
        [(
            "n-wait".to_string(),
            DelayCounts {
                waiting: 3,
                sent: 3
            }
        )]
    );
    // Nothing waiting: zeros; no wait node: nothing.
    let empty = db
        .read(|c| {
            let plain = vec![json!({ "id": "n-plain", "type": "sendMessage" })];
            let wait = vec![json!({ "id": "n-wait", "type": "wait", "data": { "details": { "steps": [{ "id": "w9" }] } } })];
            Ok((
                flows::smart_delay_stats(c, WS, FLOW, &wait).unwrap(),
                flows::smart_delay_stats(c, WS, FLOW, &plain).unwrap(),
            ))
        })
        .await
        .unwrap();
    assert_eq!(empty.0, [("n-wait".to_string(), DelayCounts::default())]);
    assert!(empty.1.is_empty());
}
