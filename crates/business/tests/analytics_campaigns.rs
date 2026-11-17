//! Broadcast and sequence step delivery stats (Q.D11 slice 8b), with the
//! seed and expectations of the parity group p1-03
//! `analytics-campaigns.test.ts`: four outcomes (delivered + seen + clicked,
//! delivered, failed, nothing yet); an unknown or foreign id counts zeros.

mod common;

use chatbotx_business::analytics::campaigns::{self, DeliveryStats};
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, WS, setup, two_workspaces};

const BROADCAST: i64 = 50;
const EMPTY_BROADCAST: i64 = 51;
const SEQUENCE: i64 = 60;
const STEP: i64 = 61;
const OTHER_STEP: i64 = 62;

/// `(deliveredAt, seenAt, clickedAt, failedAt)`.
type Outcome = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);

/// The four outcomes.
const OUTCOMES: [Outcome; 4] = [
    (Some(1), Some(2), Some(3), None),
    (Some(1), None, None, None),
    (None, None, None, Some(1)),
    (None, None, None, None),
];

async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        common::inbox(tx, 11, WS, "messenger")?;
        for k in 1..=5 {
            common::contact(tx, 100 + k, WS)?;
            common::contact_inbox(tx, 200 + k, 100 + k, 11, "messenger", &format!("psid-{k}"))?;
            common::conversation(tx, 300 + k, WS, 100 + k)?;
        }
        for id in [BROADCAST, EMPTY_BROADCAST] {
            tx.execute(
                r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel")
                   VALUES (?1, 'Promo', ?2, 'sent', 'now', 0, 'message', 'messenger')"#,
                params![id, WS],
            )?;
        }
        tx.execute(r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, 'Drip', ?2)"#, params![SEQUENCE, WS])?;
        for (order, step) in [(0, STEP), (1, OTHER_STEP)] {
            tx.execute(
                r#"INSERT INTO "SequenceStep" ("id", "order", "delayDays", "sequenceId") VALUES (?1, ?2, 0, ?3)"#,
                params![step, order, SEQUENCE],
            )?;
        }
        for (i, (delivered, seen, clicked, failed)) in OUTCOMES.into_iter().enumerate() {
            let k = i as i64 + 1;
            tx.execute(
                r#"INSERT INTO "ContactOnBroadcast" ("broadcastId", "contactId", "contactInboxId", "conversationId", "deliveredAt", "seenAt", "clickedAt", "failedAt")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
                params![BROADCAST, 100 + k, 200 + k, 300 + k, delivered, seen, clicked, failed],
            )?;
            dispatch(tx, k, STEP, (delivered, seen, clicked, failed))?;
        }
        // The other step's dispatch is not counted for STEP.
        dispatch(tx, 5, OTHER_STEP, (Some(1), None, None, None))?;
        Ok(())
    })
    .await;
    db
}

fn dispatch(
    tx: &chatbotx_db::rusqlite::Connection,
    k: i64,
    step: i64,
    (delivered, seen, clicked, failed): Outcome,
) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT OR IGNORE INTO "ContactOnSequence" ("id", "contactId", "sequenceId", "workspaceId") VALUES (?1, ?2, ?3, ?4)"#,
        params![400 + k, 100 + k, SEQUENCE, WS],
    )?;
    tx.execute(
        r#"INSERT INTO "SequenceDispatch" ("runAtMs", "idempotencyKey", "workspaceId", "sequenceId", "contactId", "contactInboxId", "stepId", "enrollmentId", "deliveredAt", "seenAt", "clickedAt", "failedAt")
           VALUES (0, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
        params![format!("k{k}-{step}"), WS, SEQUENCE, 100 + k, 200 + k, step, 400 + k, delivered, seen, clicked, failed],
    )?;
    Ok(())
}

async fn read(
    db: &chatbotx_db::Db,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection) -> chatbotx_db::repo::Result<DeliveryStats>
    + Send
    + 'static,
) -> DeliveryStats {
    db.read(move |c| f(c).map_err(|e| chatbotx_db::DbError::other(e.to_string())))
        .await
        .unwrap()
}

const FOUR: DeliveryStats = DeliveryStats {
    delivered: 2,
    seen: 1,
    clicked: 1,
    failed: 1,
};

#[tokio::test]
async fn broadcast_stats_count_the_recipients_outcomes() {
    let db = seeded().await;
    let got = read(&db, |c| campaigns::broadcast_stats(c, WS, BROADCAST)).await;
    assert_eq!(got, FOUR);
    assert_eq!(got.sent(), 3);
    for (ws, id) in [(WS, EMPTY_BROADCAST), (WS, 1), (OTHER_WS, BROADCAST)] {
        let zero = read(&db, move |c| campaigns::broadcast_stats(c, ws, id)).await;
        assert_eq!(zero, DeliveryStats::default(), "{ws} {id}");
    }
}

#[tokio::test]
async fn sequence_step_stats_count_the_step_dispatches() {
    let db = seeded().await;
    let got = read(&db, |c| {
        campaigns::sequence_step_stats(c, WS, SEQUENCE, STEP)
    })
    .await;
    assert_eq!(got, FOUR);
    assert_eq!(got.sent(), 3);
    let other = read(&db, |c| {
        campaigns::sequence_step_stats(c, WS, SEQUENCE, OTHER_STEP)
    })
    .await;
    assert_eq!(
        other,
        DeliveryStats {
            delivered: 1,
            ..DeliveryStats::default()
        }
    );
    for (ws, step) in [(WS, 1), (OTHER_WS, STEP)] {
        let zero = read(&db, move |c| {
            campaigns::sequence_step_stats(c, ws, SEQUENCE, step)
        })
        .await;
        assert_eq!(zero, DeliveryStats::default(), "{ws} {step}");
    }
}
