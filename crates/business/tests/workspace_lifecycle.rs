//! `workspace-lifecycle/__tests__/service.test.ts` and `purge-batch.test.ts`
//! (Q.D11 slice 9) on a real database instead of the mocked repository: the
//! heavy-data purge empties the workspace's direct, partitioned, hypertable,
//! analytics, leaderboard and no-FK tables and leaves the other workspace
//! alone; an elapsed deadline is `WorkspacePurgeIncomplete`; the freeze stops
//! the runtime and returns what the caller removes from the schedule.

mod common;

use chatbotx_business::workspace::lifecycle::{self, PurgeError};
use chatbotx_db::rusqlite::{Connection, params};
use common::{OTHER_WS, WS, setup, two_workspaces};

const FAR: i64 = 4_070_908_800_000;

fn later() -> i64 {
    chatbotx_core::js::now_ms() + 60_000
}

/// One contact / inbox / conversation per workspace, with a message, an
/// attachment-free error log, a broadcast recipient, an analytics event, a
/// leaderboard score event and a flow analytics session.
fn heavy(tx: &Connection, ws: i64, base: i64) -> Result<(), chatbotx_db::DbError> {
    common::inbox(tx, base + 1, ws, "messenger")?;
    common::contact(tx, base + 2, ws)?;
    common::contact_inbox(
        tx,
        base + 3,
        base + 2,
        base + 1,
        "messenger",
        &format!("p{base}"),
    )?;
    common::conversation(tx, base + 4, ws, base + 2)?;
    common::flow(tx, base + 5, ws)?;
    tx.execute(
        r#"INSERT INTO "Message" ("id", "conversationId", "contactInboxId", "workspaceId", "messageType", "contentType", "senderType")
           VALUES (?1, ?2, ?3, ?4, 'incoming', 'text', 'contact')"#,
        params![base + 6, base + 4, base + 3, ws],
    )?;
    tx.execute(
        r#"INSERT INTO "ErrorLog" ("id", "action", "detail", "workspaceId") VALUES (?1, 'send', 'x', ?2)"#,
        params![base + 7, ws],
    )?;
    tx.execute(
        r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
           VALUES (?1, 'B', ?2, 'sent', 'now', 0, 'message', 'messenger')"#,
        params![base + 8, ws],
    )?;
    tx.execute(
        r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId") VALUES (?1, ?2, ?3, ?4)"#,
        params![base + 8, base + 2, base + 3, base + 4],
    )?;
    tx.execute(
        r#"INSERT INTO "AnalyticsContactEvent" ("eventId", "workspaceId", "contactId", "eventType", "occurredAt")
           VALUES (?1, ?2, ?3, 'contact_created', ?4)"#,
        params![format!("e{base}"), ws, base + 2, 1_700_000_000_000_i64],
    )?;
    tx.execute(
        r#"INSERT INTO "LeaderboardScoreEvent" ("id", "leaderboardId", "workspaceId", "contactId", "delta", "source")
           VALUES (?1, 1, ?2, ?3, 1, 'manual')"#,
        params![base + 9, ws, base + 2],
    )?;
    tx.execute(
        r#"INSERT INTO "FlowAnalyticsSession" ("id", "flowId", "workspaceId") VALUES (?1, ?2, ?3)"#,
        params![base + 10, base + 5, ws],
    )?;
    Ok(())
}

const TABLES: [&str; 8] = [
    "Message",
    "ErrorLog",
    "Contact",
    "Conversation",
    "AnalyticsContactEvent",
    "LeaderboardScoreEvent",
    "FlowAnalyticsSession",
    "Flow",
];

async fn counts(db: &chatbotx_db::Db, ws: i64) -> Vec<i64> {
    db.read(move |c| {
        let mut out = TABLES
            .iter()
            .map(|t| {
                c.query_row(
                    &format!(r#"SELECT COUNT(*) FROM "{t}" WHERE "workspaceId" = ?1"#),
                    [ws],
                    |r| r.get(0),
                )
            })
            .collect::<Result<Vec<i64>, _>>()?;
        out.push(c.query_row(
            r#"SELECT COUNT(*) FROM "ContactOnBroadcast" r JOIN "Broadcast" b ON b."id" = r."broadcastId" WHERE b."workspaceId" = ?1"#,
            [ws],
            |r| r.get(0),
        )?);
        Ok(out)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn heavy_purge_empties_the_workspace_tables_and_keeps_the_other_workspace() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        heavy(tx, WS, 100)?;
        heavy(tx, OTHER_WS, 200)
    })
    .await;
    let deleted = lifecycle::purge_workspace_heavy_data(&db, WS, later(), Some(2))
        .await
        .unwrap();
    assert!(deleted > 0, "{deleted}");
    let gone = counts(&db, WS).await;
    // `Flow` is not a heavy table: the workspace row delete cascades it later.
    assert_eq!(gone, [0, 0, 0, 0, 0, 0, 0, 1, 0]);
    assert_eq!(counts(&db, OTHER_WS).await, [1; 9]);
    // A second run has nothing left to delete.
    let again = lifecycle::purge_workspace_heavy_data(&db, WS, later(), None)
        .await
        .unwrap();
    assert_eq!(again, 0);
}

/// purge-batch.test.ts "throws incomplete before starting a batch when the
/// deadline has elapsed".
#[tokio::test]
async fn an_elapsed_deadline_is_incomplete_before_any_batch() {
    let db = two_workspaces().await;
    setup(&db, |tx| heavy(tx, WS, 100)).await;
    let err = lifecycle::purge_workspace_heavy_data(&db, WS, 0, None)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, PurgeError::Incomplete { workspace_id: WS, table } if table == "AnalyticsAgentResponseEvent"),
        "{err}"
    );
    assert_eq!(
        err.to_string(),
        "Workspace purge is incomplete for AnalyticsAgentResponseEvent"
    );
    assert_eq!(counts(&db, WS).await, [1; 9]);
    let freeze = lifecycle::freeze_workspace_runtime(&db, WS, 0)
        .await
        .unwrap_err();
    assert!(
        matches!(freeze, PurgeError::Incomplete { ref table, .. } if table == "ContactOnSequence")
    );
}

type State = (String, bool, bool, String, String, String);

async fn state(db: &chatbotx_db::Db, ws: i64, base: i64) -> State {
    db.read(move |c| {
        let text = |sql: &str, p: &[&dyn chatbotx_db::rusqlite::ToSql]| -> Result<String, chatbotx_db::rusqlite::Error> {
            c.query_row(sql, p, |r| r.get(0))
        };
        let broadcasts = text(
            r#"SELECT group_concat("status", ',') FROM (SELECT "status" FROM "Broadcast" WHERE "workspaceId" = ?1 AND "id" >= ?2 ORDER BY "id")"#,
            params![ws, base + 20],
        )?;
        let (armed, paused): (Option<i64>, Option<i64>) = c.query_row(
            r#"SELECT "nextRunAt", "pausedAt" FROM "BroadcastRecurrence" WHERE "id" = ?1"#,
            [base + 30],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let enrollment = text(r#"SELECT "status" FROM "ContactOnSequence" WHERE "id" = ?1"#, params![base + 42])?;
        let dispatch = text(r#"SELECT "status" FROM "SequenceDispatch" WHERE "id" = ?1"#, params![base + 43])?;
        let delays = text(
            r#"SELECT group_concat("status", ',') FROM (SELECT "status" FROM "ContactOnSmartDelay" WHERE "workspaceId" = ?1 ORDER BY "id")"#,
            params![ws],
        )?;
        Ok((broadcasts, armed.is_some(), paused.is_some(), enrollment, dispatch, delays))
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn freeze_stops_broadcasts_recurrences_sequences_and_smart_delays() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for (ws, base) in [(WS, 100), (OTHER_WS, 200)] {
            heavy(tx, ws, base)?;
            for (k, status) in [(20, "scheduled"), (21, "sending"), (22, "recurring"), (23, "draft")] {
                tx.execute(
                    r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
                       VALUES (?1, 'B', ?2, ?3, 'now', 0, 'message', 'messenger')"#,
                    params![base + k, ws, status],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "BroadcastRecurrence" ("id", "workspaceId", "broadcastId", "rule", "nextRunAt") VALUES (?1, ?2, ?3, '{}', ?4)"#,
                params![base + 30, ws, base + 22, FAR],
            )?;
            tx.execute(r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, 'S', ?2)"#, params![base + 40, ws])?;
            tx.execute(
                r#"INSERT INTO "SequenceStep" ("id", "order", "delayDays", "sequenceId") VALUES (?1, 0, 0, ?2)"#,
                params![base + 41, base + 40],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactOnSequence" ("id", "contactId", "sequenceId", "workspaceId", "status", "nextRunAt") VALUES (?1, ?2, ?3, ?4, 'active', ?5)"#,
                params![base + 42, base + 2, base + 40, ws, FAR],
            )?;
            tx.execute(
                r#"INSERT INTO "SequenceDispatch" ("id", "runAtMs", "idempotencyKey", "workspaceId", "sequenceId", "contactId", "contactInboxId", "stepId", "enrollmentId", "status", "bucket")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', 7)"#,
                params![base + 43, FAR, format!("d{base}"), ws, base + 40, base + 2, base + 3, base + 41, base + 42],
            )?;
            for (k, status) in [(50, "pending"), (51, "scheduled"), (52, "completed")] {
                tx.execute(
                    r#"INSERT INTO "ContactOnSmartDelay" ("id", "workspaceId", "flowId", "contactInboxId", "conversationId", "type", "status", "triggerAt")
                       VALUES (?1, ?2, ?3, ?4, ?5, 'waitNode', ?6, ?7)"#,
                    params![base + k, ws, base + 5, base + 3, base + 4, status, FAR + k],
                )?;
            }
        }
        Ok(())
    })
    .await;

    let out = lifecycle::freeze_workspace_runtime(&db, WS, later())
        .await
        .unwrap();
    assert_eq!(out.cancelled_broadcasts, 3);
    assert_eq!(out.completed_enrollments, 1);
    assert_eq!(out.canceled_dispatches, [(143, 7)]);
    let mut delays = out.canceled_smart_delays.clone();
    delays.sort();
    assert_eq!(delays, [(150, FAR + 50), (151, FAR + 51)]);

    assert_eq!(
        state(&db, WS, 100).await,
        (
            "cancelled,cancelled,cancelled,draft".to_owned(),
            false,
            true,
            "completed".to_owned(),
            "canceled".to_owned(),
            "canceled,canceled,completed".to_owned()
        )
    );
    assert_eq!(
        state(&db, OTHER_WS, 200).await,
        (
            "scheduled,sending,recurring,draft".to_owned(),
            true,
            false,
            "active".to_owned(),
            "pending".to_owned(),
            "pending,scheduled,completed".to_owned()
        )
    );
    // Nothing left to freeze.
    let again = lifecycle::freeze_workspace_runtime(&db, WS, later())
        .await
        .unwrap();
    assert_eq!(again, lifecycle::FrozenRuntime::default());
}
