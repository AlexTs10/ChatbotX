//! Port of the `computeSnapshot` and `trackWindow` cases of
//! `packages/business/__tests__/response-sla-service.test.ts` (Q.D13a).
//! Adapted: the mocked schedule / override / workspace reads become rows of
//! a migrated database, the asserted repository calls the stored
//! `Conversation` snapshot. Not applicable: the cache, warm-up, lock and
//! savepoint-on-a-caller-connection cases (one process, one writer, no
//! cache), and the injected read failures.

mod common;

use chatbotx_business::conversation_actions;
use chatbotx_business::response_sla::{compute_snapshot, track_window};
use chatbotx_db::repo::response_sla::SlaSnapshot;
use chatbotx_db::rusqlite::params;
use common::{TestDb, WS, contact, conversation, setup, two_workspaces, workspace_member};

const AGENT: i64 = 1001; // the owner user of `WS`.
const SCHEDULE: i64 = 7001;
const CONV: i64 = 7101;

fn at(iso: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso)
        .expect("iso")
        .timestamp_millis()
}

/// Mon–Fri 09:00–17:00 for `AGENT`.
async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "WorkingHours" ("id", "workspaceId", "name") VALUES (?1, ?2, 'Office')"#,
            params![SCHEDULE, WS],
        )?;
        for weekday in 1..=5 {
            tx.execute(
                r#"INSERT INTO "WorkingHoursInterval" ("id", "workingHoursId", "weekday", "startMinute", "endMinute")
                   VALUES (?1, ?2, ?3, 540, 1020)"#,
                params![7200 + weekday, SCHEDULE, weekday],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "WorkingHoursMember" ("id", "workingHoursId", "workspaceId", "userId") VALUES (7301, ?1, ?2, ?3)"#,
            params![SCHEDULE, WS, AGENT],
        )?;
        workspace_member(tx, 7300, WS, AGENT)?;
        contact(tx, 7401, WS)?;
        conversation(tx, CONV, WS, 7401)?;
        Ok(())
    })
    .await;
    db
}

async fn exec(db: &TestDb, sql: &'static str, args: Vec<i64>) {
    setup(db, move |tx| {
        tx.execute(sql, chatbotx_db::rusqlite::params_from_iter(args))?;
        Ok(())
    })
    .await;
}

async fn snapshot(
    db: &TestDb,
    assigned: Option<i64>,
    start: &str,
    target: i64,
    percent: i64,
) -> Option<SlaSnapshot> {
    let start = at(start);
    db.write(move |tx| {
        Ok(compute_snapshot(tx, WS, assigned, start, target, percent).expect("compute"))
    })
    .await
    .expect("write")
}

async fn due(db: &TestDb, start: &str) -> Option<i64> {
    snapshot(db, Some(AGENT), start, 3600, 25)
        .await
        .map(|s| s.response_due_at)
}

#[tokio::test]
async fn friday_evening_rolls_over_the_weekend_and_overrides_move_it() {
    let db = db().await;
    assert_eq!(
        due(&db, "2026-06-05T16:30:00Z").await,
        Some(at("2026-06-08T09:30:00Z"))
    );
    // A workspace holiday on that Monday, then a closed Tuesday for this schedule.
    exec(
        &db,
        r#"INSERT INTO "WorkingHoursDateOverride" ("id", "workspaceId", "date", "ranges") VALUES (7501, ?1, '2026-06-08', '[]')"#,
        vec![WS],
    )
    .await;
    assert_eq!(
        due(&db, "2026-06-05T16:30:00Z").await,
        Some(at("2026-06-09T09:30:00Z"))
    );
    exec(
        &db,
        r#"INSERT INTO "WorkingHoursDateOverride" ("id", "workspaceId", "workingHoursId", "date", "ranges") VALUES (7502, ?1, ?2, '2026-06-09', '[]')"#,
        vec![WS, SCHEDULE],
    )
    .await;
    assert_eq!(
        due(&db, "2026-06-05T16:30:00Z").await,
        Some(at("2026-06-10T09:30:00Z"))
    );
}

#[tokio::test]
async fn an_override_that_opens_a_weekend_day_is_used() {
    let db = db().await;
    exec(
        &db,
        r#"INSERT INTO "WorkingHoursDateOverride" ("id", "workspaceId", "workingHoursId", "date", "ranges")
           VALUES (7501, ?1, ?2, '2026-06-06', '[{"startMinute":600,"endMinute":720}]')"#,
        vec![WS, SCHEDULE],
    )
    .await;
    // 30 min left Friday + 30 min Saturday from 10:00.
    assert_eq!(
        due(&db, "2026-06-05T16:30:00Z").await,
        Some(at("2026-06-06T10:30:00Z"))
    );
}

#[tokio::test]
async fn opening_and_closing_edges() {
    let db = db().await;
    assert_eq!(
        due(&db, "2026-06-01T07:00:00Z").await,
        Some(at("2026-06-01T10:00:00Z"))
    );
    assert_eq!(
        due(&db, "2026-06-01T17:00:00Z").await,
        Some(at("2026-06-02T10:00:00Z"))
    );
    assert_eq!(
        due(&db, "2026-06-01T16:00:00Z").await,
        Some(at("2026-06-01T17:00:00Z"))
    );
    let s = snapshot(&db, Some(AGENT), "2026-06-01T10:00:00Z", 60, 99)
        .await
        .expect("snapshot");
    assert!(s.response_sla_breaching_at > at("2026-06-01T10:00:00Z"));
    assert!(s.response_sla_breaching_at <= s.response_due_at);
    let s = snapshot(&db, Some(AGENT), "2026-06-05T16:50:00Z", 3600, 1)
        .await
        .expect("snapshot");
    assert!(s.response_sla_breaching_at <= s.response_due_at);
}

#[tokio::test]
async fn a_workspace_timezone_shifts_the_local_opening() {
    let db = db().await;
    exec(
        &db,
        r#"UPDATE "Workspace" SET "timezone" = 'Asia/Ho_Chi_Minh' WHERE "id" = ?1"#,
        vec![WS],
    )
    .await;
    // 09:00 local Monday = 02:00 UTC.
    assert_eq!(
        due(&db, "2026-06-01T01:00:00Z").await,
        Some(at("2026-06-01T03:00:00Z"))
    );
}

#[tokio::test]
async fn unassigned_or_unscheduled_is_calendar_time_and_unreachable_is_none() {
    let db = db().await;
    assert_eq!(
        snapshot(&db, None, "2026-06-01T10:00:00Z", 3600, 25).await,
        Some(SlaSnapshot {
            response_due_at: at("2026-06-01T11:00:00Z"),
            response_sla_breaching_at: at("2026-06-01T10:45:00Z"),
            response_sla_target_seconds: 3600,
        })
    );
    // A member without a schedule.
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (1009, 'other@example.test')"#,
            [],
        )?;
        workspace_member(tx, 7601, WS, 1009)?;
        Ok(())
    })
    .await;
    let s = snapshot(&db, Some(1009), "2026-06-01T10:00:00Z", 600, 50)
        .await
        .expect("snapshot");
    assert_eq!(s.response_due_at, at("2026-06-01T10:10:00Z"));
    assert_eq!(s.response_sla_breaching_at, at("2026-06-01T10:05:00Z"));
    // An empty schedule never reaches the deadline.
    exec(
        &db,
        r#"DELETE FROM "WorkingHoursInterval" WHERE "workingHoursId" = ?1"#,
        vec![SCHEDULE],
    )
    .await;
    assert_eq!(
        snapshot(&db, Some(AGENT), "2026-06-01T10:00:00Z", 600, 25).await,
        None
    );
}

#[tokio::test]
async fn a_scheduled_agent_counts_breaching_in_working_time_too() {
    let db = db().await;
    exec(
        &db,
        r#"DELETE FROM "WorkingHoursInterval" WHERE "workingHoursId" = ?1 AND "weekday" > 1"#,
        vec![SCHEDULE],
    )
    .await;
    let s = snapshot(&db, Some(AGENT), "2026-06-01T16:30:00Z", 3600, 50)
        .await
        .expect("snapshot");
    assert_eq!(s.response_due_at, at("2026-06-08T09:30:00Z"));
    assert_eq!(s.response_sla_breaching_at, at("2026-06-01T17:00:00Z"));
}

/// `(responseDueAt, responseSlaBreachingAt, responseSlaTargetSeconds)`.
async fn stored(db: &TestDb) -> (Option<i64>, Option<i64>, Option<i64>) {
    db.read(|c| {
        Ok(c.query_row(
            r#"SELECT "responseDueAt", "responseSlaBreachingAt", "responseSlaTargetSeconds" FROM "Conversation" WHERE "id" = ?1"#,
            [CONV],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    })
    .await
    .expect("read")
}

async fn policy(db: &TestDb, enabled: bool, next: Option<i64>) {
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "ResponseSlaPolicy" ("id", "workspaceId", "enabled", "firstResponseTargetSeconds",
                 "nextResponseTargetSeconds", "breachingThresholdPercent") VALUES (7701, ?1, ?2, 600, ?3, 20)"#,
            params![WS, enabled, next],
        )?;
        Ok(())
    })
    .await;
}

/// Opens the waiting window at `since` through [`track_window`].
async fn open_window(db: &TestDb, since: i64) {
    db.write(move |tx| {
        track_window(tx, WS, &[CONV], false, |c| {
            c.execute(
                r#"UPDATE "Conversation" SET "awaitingHumanReplySince" = COALESCE("awaitingHumanReplySince", ?1) WHERE "id" = ?2"#,
                params![since, CONV],
            )
        })?;
        Ok(())
    })
    .await
    .expect("open");
}

#[tokio::test]
async fn without_an_enabled_policy_it_only_runs_the_statement() {
    let db = db().await;
    open_window(&db, 1_000).await;
    assert_eq!(stored(&db).await, (None, None, None));
    exec(
        &db,
        r#"UPDATE "Conversation" SET "awaitingHumanReplySince" = NULL WHERE "id" = ?1"#,
        vec![CONV],
    )
    .await;
    policy(&db, false, None).await;
    open_window(&db, 1_000).await;
    assert_eq!(stored(&db).await, (None, None, None));
}

#[tokio::test]
async fn an_opened_window_gets_the_first_response_deadline_but_never_retroactively() {
    let db = db().await;
    policy(&db, true, None).await;
    open_window(&db, 1_000).await;
    assert_eq!(
        stored(&db).await,
        (Some(1_000 + 600_000), Some(1_000 + 480_000), Some(600))
    );
    // Another customer message in the open window changes nothing.
    exec(
        &db,
        r#"UPDATE "Conversation" SET "responseDueAt" = 5 WHERE "id" = ?1"#,
        vec![CONV],
    )
    .await;
    open_window(&db, 9_000).await;
    assert_eq!(stored(&db).await.0, Some(5));
}

#[tokio::test]
async fn a_later_reply_window_uses_the_next_target_and_none_means_no_sla() {
    let db = db().await;
    policy(&db, true, None).await;
    exec(
        &db,
        r#"UPDATE "Conversation" SET "adminRepliedAt" = 500 WHERE "id" = ?1"#,
        vec![CONV],
    )
    .await;
    open_window(&db, 1_000).await;
    assert_eq!(stored(&db).await, (None, None, None));
    exec(
        &db,
        r#"UPDATE "Conversation" SET "awaitingHumanReplySince" = NULL WHERE "id" = ?1"#,
        vec![CONV],
    )
    .await;
    exec(&db, r#"UPDATE "ResponseSlaPolicy" SET "nextResponseTargetSeconds" = 120 WHERE "workspaceId" = ?1"#, vec![WS]).await;
    open_window(&db, 1_000).await;
    assert_eq!(stored(&db).await.2, Some(120));
}

#[tokio::test]
async fn a_comment_thread_never_gets_an_sla() {
    let db = db().await;
    policy(&db, true, None).await;
    exec(
        &db,
        r#"UPDATE "Conversation" SET "sourceId" = 'thread-1' WHERE "id" = ?1"#,
        vec![CONV],
    )
    .await;
    open_window(&db, 1_000).await;
    assert_eq!(stored(&db).await, (None, None, None));
}

#[tokio::test]
async fn a_reassignment_recomputes_from_the_assignment_instant_and_keeps_the_target() {
    let db = db().await;
    policy(&db, true, None).await;
    open_window(&db, 1_000).await;
    // An agent without a schedule: calendar time from the assignment instant.
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (1009, 'other@example.test')"#,
            [],
        )?;
        workspace_member(tx, 7601, WS, 1009)?;
        Ok(())
    })
    .await;
    conversation_actions::assign_one(&db, WS, CONV, Some("u_1009".into()))
        .await
        .expect("assign");
    let assigned_at: i64 = db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "assignedUserAt" FROM "Conversation" WHERE "id" = ?1"#,
                [CONV],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("read");
    assert_eq!(
        stored(&db).await,
        (
            Some(assigned_at + 600_000),
            Some(assigned_at + 480_000),
            Some(600)
        )
    );
    // Unassigning recomputes in calendar time from the window start.
    conversation_actions::assign_one(&db, WS, CONV, None)
        .await
        .expect("unassign");
    assert_eq!(stored(&db).await.0, Some(1_000 + 600_000));
}

#[tokio::test]
async fn reassigning_to_an_agent_whose_calendar_cannot_supply_the_target_drops_the_deadline() {
    let db = db().await;
    policy(&db, true, None).await;
    open_window(&db, 1_000).await;
    exec(
        &db,
        r#"DELETE FROM "WorkingHoursInterval" WHERE "workingHoursId" = ?1"#,
        vec![SCHEDULE],
    )
    .await;
    conversation_actions::assign_one(&db, WS, CONV, Some(format!("u_{AGENT}")))
        .await
        .expect("assign");
    assert_eq!(stored(&db).await, (None, None, None));
}

/// `recordAgentReply` (`agent-reply-metric.ts`): an agent reply closes the open
/// window and keeps the reply time; an API reply closes it too.
#[tokio::test]
async fn an_agent_reply_closes_the_open_window() {
    use chatbotx_business::message::{AgentReply, record_agent_reply};
    let db = db().await;
    let since = at("2026-06-01T09:00:00Z");
    exec(
        &db,
        r#"UPDATE "Conversation" SET "awaitingHumanReplySince" = ?1 WHERE "id" = ?2"#,
        vec![since, CONV],
    )
    .await;
    let reply = AgentReply {
        workspace_id: WS,
        conversation_id: CONV,
        contact_id: 7401,
        channel: "messenger".into(),
        inbox_id: 1,
        at: since + 60_000,
        admin_id: Some(AGENT),
        event_id: "reply:1".into(),
    };
    record_agent_reply(&db, reply.clone()).await;
    let row = db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "awaitingHumanReplySince", "adminRepliedAt" FROM "Conversation" WHERE "id" = ?1"#,
                [CONV],
                |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
            )?)
        })
        .await
        .expect("read");
    assert_eq!(row, (None, Some(since + 60_000)));

    // A window opened after the reply stays open.
    let later = since + 120_000;
    exec(
        &db,
        r#"UPDATE "Conversation" SET "awaitingHumanReplySince" = ?1 WHERE "id" = ?2"#,
        vec![later, CONV],
    )
    .await;
    record_agent_reply(
        &db,
        AgentReply {
            admin_id: None,
            ..reply
        },
    )
    .await;
    let open = db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "awaitingHumanReplySince" FROM "Conversation" WHERE "id" = ?1"#,
                [CONV],
                |r| r.get::<_, Option<i64>>(0),
            )?)
        })
        .await
        .expect("read");
    assert_eq!(open, Some(later));
}
