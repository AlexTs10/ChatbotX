//! Port of `packages/database/__tests__/integration/conversation-close-human-reply-window.test.ts`
//! (P3.2b). Real SQL on a migrated temp file. The two "concurrent" cases run on the
//! single writer (adapted): the second write job runs after the first commits, which
//! is what the Postgres row lock gives.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chatbotx_db::repo::conversation::{self as conv, AssignTargetInput};
use chatbotx_db::repo::models::Conversation;
use chatbotx_db::repo::response_sla::{self as sla, ResponseSlaPolicyInput, SlaSnapshot};
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::types::Value;
use chatbotx_db::{Db, WriteTx};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{ms, ok, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
const AGENT: i64 = 1003;

fn t0() -> i64 {
    ms("2026-10-18T09:00:00.000Z")
}

fn at(offset_seconds: i64) -> i64 {
    t0() + offset_seconds * 1000
}

fn snapshot() -> SlaSnapshot {
    SlaSnapshot {
        response_due_at: at(3600),
        response_sla_breaching_at: at(2700),
        response_sla_target_seconds: 3600,
    }
}

/// WS (owner 1001) with an enabled response SLA policy, OTHER (owner 1002), agent 1003.
async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER).await;
    setup(&db, |tx| {
        ins!(tx, "User", { "id" => AGENT, "email" => "agent@example.test".to_owned() })?;
        ins!(tx, "ResponseSlaPolicy", {
            "id" => 1_i64, "workspaceId" => WS, "enabled" => true,
            "firstResponseTargetSeconds" => 3600_i64, "breachingThresholdPercent" => 25_i64,
        })?;
        Ok(())
    })
    .await;
    db
}

/// `newConversation`: a fresh contact of WS and its DM, with `values` set.
async fn new_conversation(db: &Db, id: i64, values: Vec<(&'static str, Value)>) -> i64 {
    setup(db, move |tx| {
        ins!(tx, "Contact", { "id" => id + 100_000, "workspaceId" => WS })?;
        ins!(tx, "Conversation", { "id" => id, "workspaceId" => WS, "contactId" => id + 100_000 })?;
        for (col, value) in values {
            tx.execute(
                &format!("UPDATE \"Conversation\" SET \"{col}\" = ?1 WHERE \"id\" = ?2"),
                chatbotx_db::rusqlite::params![value, id],
            )?;
        }
        Ok(())
    })
    .await;
    id
}

fn snapshot_values(since: i64) -> Vec<(&'static str, Value)> {
    let s = snapshot();
    vec![
        ("awaitingHumanReplySince", Value::Integer(since)),
        ("responseDueAt", Value::Integer(s.response_due_at)),
        (
            "responseSlaBreachingAt",
            Value::Integer(s.response_sla_breaching_at),
        ),
        (
            "responseSlaTargetSeconds",
            Value::Integer(s.response_sla_target_seconds),
        ),
    ]
}

async fn read(db: &Db, id: i64) -> Conversation {
    ok(repo::read(db, move |c| {
        repo::find_one::<Conversation>(c, "\"id\" = ?1", &[&id])
    })
    .await)
    .expect("conversation row")
}

async fn write<T, F>(db: &Db, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce(&mut WriteTx<'_>) -> repo::Result<T> + Send + 'static,
{
    ok(repo::write(db, f).await)
}

async fn close(
    db: &Db,
    workspace_id: i64,
    id: i64,
    replied_at: i64,
) -> Option<conv::CloseHumanReplyWindowResult> {
    write(db, move |c| {
        conv::close_human_reply_window(c, workspace_id, id, replied_at)
    })
    .await
}

async fn set_snapshot(db: &Db, id: i64, expected_since: i64) -> bool {
    write(db, move |c| {
        sla::set_snapshot_if_window_unchanged(c, WS, id, expected_since, snapshot())
    })
    .await
}

#[tokio::test]
async fn closes_an_open_window_reports_its_start_and_records_the_reply() {
    let db = fixture().await;
    let id = new_conversation(
        &db,
        1,
        vec![("awaitingHumanReplySince", Value::Integer(t0()))],
    )
    .await;
    let result = close(&db, WS, id, at(300)).await.expect("in the workspace");
    assert_eq!(result.closed_awaiting_since, Some(t0()));
    assert_eq!(result.previous_admin_replied_at, None);
    let row = read(&db, id).await;
    assert_eq!(row.awaiting_human_reply_since, None);
    assert_eq!(row.awaiting_human_reply_closed_at, Some(at(300)));
    assert_eq!(row.admin_replied_at, Some(at(300)));
    assert_eq!(row.agent_last_read_at, Some(at(300)));
    assert_eq!(row.last_activity_at, Some(at(300)));
}

#[tokio::test]
async fn reports_the_previous_admin_replied_at_and_the_assignment() {
    let db = fixture().await;
    let id = new_conversation(
        &db,
        1,
        vec![
            ("awaitingHumanReplySince", Value::Integer(t0())),
            ("adminRepliedAt", Value::Integer(at(-3600))),
            ("assignedUserId", Value::Integer(AGENT)),
            ("assignedUserAt", Value::Integer(at(60))),
        ],
    )
    .await;
    let result = close(&db, WS, id, at(300)).await.expect("in the workspace");
    assert_eq!(result.previous_admin_replied_at, Some(at(-3600)));
    assert_eq!(result.assigned_user_id, Some(AGENT));
    assert_eq!(result.assigned_user_at, Some(at(60)));
}

#[tokio::test]
async fn a_window_opened_after_the_reply_since_replied_at_is_left_open_and_not_reported() {
    let db = fixture().await;
    let id = new_conversation(
        &db,
        1,
        vec![("awaitingHumanReplySince", Value::Integer(at(600)))],
    )
    .await;
    let result = close(&db, WS, id, at(300)).await.expect("in the workspace");
    assert_eq!(result.closed_awaiting_since, None);
    let row = read(&db, id).await;
    assert_eq!(row.awaiting_human_reply_since, Some(at(600)));
    assert_eq!(row.awaiting_human_reply_closed_at, Some(at(300)));
}

#[tokio::test]
async fn the_closed_at_marker_never_moves_backwards() {
    let db = fixture().await;
    let id = new_conversation(
        &db,
        1,
        vec![("awaitingHumanReplyClosedAt", Value::Integer(at(900)))],
    )
    .await;
    close(&db, WS, id, at(300)).await;
    assert_eq!(
        read(&db, id).await.awaiting_human_reply_closed_at,
        Some(at(900))
    );
}

#[tokio::test]
async fn two_concurrent_replies_on_two_connections_exactly_one_claims_the_window() {
    let db = fixture().await;
    let id = new_conversation(
        &db,
        1,
        vec![("awaitingHumanReplySince", Value::Integer(t0()))],
    )
    .await;
    let (a, b) = tokio::join!(close(&db, WS, id, at(300)), close(&db, WS, id, at(301)));
    let claims: Vec<Option<i64>> = [a, b]
        .into_iter()
        .map(|r| r.expect("in the workspace").closed_awaiting_since)
        .collect();
    assert_eq!(claims.iter().filter(|c| **c == Some(t0())).count(), 1);
    assert_eq!(claims.iter().filter(|c| c.is_none()).count(), 1);
    assert_eq!(
        read(&db, id).await.awaiting_human_reply_closed_at,
        Some(at(301))
    );
}

#[tokio::test]
async fn another_workspace_cannot_close_the_window() {
    let db = fixture().await;
    let id = new_conversation(
        &db,
        1,
        vec![("awaitingHumanReplySince", Value::Integer(t0()))],
    )
    .await;
    assert_eq!(close(&db, OTHER, id, at(300)).await, None);
    assert_eq!(read(&db, id).await.awaiting_human_reply_since, Some(t0()));
}

#[tokio::test]
async fn assign_user_if_unassigned_stamps_assigned_user_at() {
    let db = fixture().await;
    let id = new_conversation(&db, 1, vec![]).await;
    let claimed = write(&db, move |c| {
        conv::assign_user_if_unassigned(c, WS, id, AGENT, Some(at(10)))
    })
    .await;
    assert_eq!(
        claimed.first().and_then(|r| r.assigned_user_at),
        Some(at(10))
    );
}

#[tokio::test]
async fn assign_target_if_unassigned_stamps_assigned_user_at_only_when_a_user_is_set() {
    let db = fixture().await;
    let team_only = new_conversation(&db, 1, vec![]).await;
    let with_user = new_conversation(&db, 2, vec![]).await;
    setup(&db, |tx| ins!(tx, "InboxTeam", { "id" => 31_i64, "workspaceId" => WS, "name" => "chrw-team".to_owned() })).await;
    write(&db, move |c| {
        conv::assign_target_if_unassigned(
            c,
            &AssignTargetInput {
                workspace_id: WS,
                conversation_id: team_only,
                assigned_inbox_team_id: Some(31),
                assigned_user_id: None,
                assigned_at: Some(at(10)),
            },
        )
    })
    .await;
    write(&db, move |c| {
        conv::assign_target_if_unassigned(
            c,
            &AssignTargetInput {
                workspace_id: WS,
                conversation_id: with_user,
                assigned_inbox_team_id: None,
                assigned_user_id: Some(AGENT),
                assigned_at: Some(at(20)),
            },
        )
    })
    .await;
    assert_eq!(read(&db, team_only).await.assigned_user_at, None);
    assert_eq!(read(&db, with_user).await.assigned_user_at, Some(at(20)));
}

// ---------------------------------------------------------------------------
// response SLA snapshot
// ---------------------------------------------------------------------------

#[tokio::test]
async fn response_sla_snapshot_no_snapshot_is_written_while_the_policy_is_switched_off() {
    let db = fixture().await;
    let id = new_conversation(
        &db,
        1,
        vec![("awaitingHumanReplySince", Value::Integer(at(0)))],
    )
    .await;
    let set_enabled = |enabled: bool| {
        setup(&db, move |tx| {
            tx.execute(
                "UPDATE \"ResponseSlaPolicy\" SET \"enabled\" = ?1 WHERE \"workspaceId\" = ?2",
                chatbotx_db::rusqlite::params![enabled, WS],
            )?;
            Ok(())
        })
    };
    set_enabled(false).await;
    assert!(!set_snapshot(&db, id, at(0)).await);
    assert_eq!(read(&db, id).await.response_due_at, None);
    set_enabled(true).await;
    assert!(set_snapshot(&db, id, at(0)).await);
}

#[tokio::test]
async fn response_sla_snapshot_closing_the_window_clears_all_three_snapshot_fields_and_returns_the_closed_due_target()
 {
    let db = fixture().await;
    let id = new_conversation(&db, 1, snapshot_values(at(0))).await;
    let claim = close(&db, WS, id, at(600)).await.expect("in the workspace");
    assert_eq!(
        claim.closed_response_due_at,
        Some(snapshot().response_due_at)
    );
    assert_eq!(claim.closed_response_sla_target_seconds, Some(3600));
    let row = read(&db, id).await;
    assert_eq!(row.response_due_at, None);
    assert_eq!(row.response_sla_breaching_at, None);
    assert_eq!(row.response_sla_target_seconds, None);
}

#[tokio::test]
async fn response_sla_snapshot_a_window_opened_after_the_reply_keeps_its_snapshot_and_reports_nothing_closed()
 {
    let db = fixture().await;
    let id = new_conversation(&db, 1, snapshot_values(at(900))).await;
    let claim = close(&db, WS, id, at(600)).await.expect("in the workspace");
    assert_eq!(claim.closed_response_due_at, None);
    let row = read(&db, id).await;
    assert_eq!(row.response_due_at, Some(snapshot().response_due_at));
    assert_eq!(row.response_sla_target_seconds, Some(3600));
}

#[tokio::test]
async fn response_sla_snapshot_the_snapshot_is_written_only_while_the_window_is_the_one_it_was_computed_for()
 {
    let db = fixture().await;
    let id = new_conversation(
        &db,
        1,
        vec![("awaitingHumanReplySince", Value::Integer(at(0)))],
    )
    .await;
    assert!(!set_snapshot(&db, id, at(5)).await);
    assert!(set_snapshot(&db, id, at(0)).await);
    assert_eq!(
        read(&db, id).await.response_due_at,
        Some(snapshot().response_due_at)
    );
    // A close that raced in leaves no stale deadline: the guard fails.
    close(&db, WS, id, at(60)).await;
    assert!(!set_snapshot(&db, id, at(0)).await);
    assert_eq!(read(&db, id).await.response_due_at, None);
}

#[tokio::test]
async fn response_sla_snapshot_a_concurrent_reply_waits_for_the_lock_holding_sla_write_then_closes_and_reports_the_snapshot_it_set()
 {
    let db = fixture().await;
    let id = new_conversation(
        &db,
        1,
        vec![("awaitingHumanReplySince", Value::Integer(at(0)))],
    )
    .await;
    let started = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&started);
    let sla_write = write(&db, move |c| {
        sla::read_window_states(c, WS, &[id], true)?;
        flag.store(true, Ordering::SeqCst);
        // Give the close time to run into the held writer.
        std::thread::sleep(std::time::Duration::from_millis(300));
        sla::set_snapshot_if_window_unchanged(c, WS, id, at(0), snapshot())
    });
    let closing = async {
        while !started.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        close(&db, WS, id, at(600)).await
    };
    let (written, claim) = tokio::join!(sla_write, closing);
    assert!(written);
    // The close ran after the SLA write committed, so it saw and cleared it.
    let claim = claim.expect("in the workspace");
    assert_eq!(
        claim.closed_response_due_at,
        Some(snapshot().response_due_at)
    );
    let row = read(&db, id).await;
    assert_eq!(row.response_due_at, None);
    assert_eq!(row.response_sla_target_seconds, None);
}

#[tokio::test]
async fn response_sla_snapshot_clear_workspace_snapshots_drops_every_snapshot_of_the_workspace_only()
 {
    let db = fixture().await;
    let mine = new_conversation(&db, 1, snapshot_values(at(0))).await;
    setup(&db, |tx| {
        ins!(tx, "Contact", { "id" => 900_i64, "workspaceId" => OTHER })?;
        ins!(tx, "Conversation", {
            "id" => 9_i64, "workspaceId" => OTHER, "contactId" => 900_i64,
            "awaitingHumanReplySince" => at(0), "responseDueAt" => at(3600),
        })
    })
    .await;
    write(&db, |c| sla::clear_workspace_snapshots(c, WS)).await;
    let row = read(&db, mine).await;
    assert_eq!(row.response_due_at, None);
    assert_eq!(row.response_sla_breaching_at, None);
    assert_eq!(row.response_sla_target_seconds, None);
    // The window itself is untouched: only the SLA snapshot goes.
    assert_eq!(row.awaiting_human_reply_since, Some(at(0)));
    assert_eq!(read(&db, 9).await.response_due_at, Some(at(3600)));
}

#[tokio::test]
async fn response_sla_snapshot_a_comment_thread_never_receives_a_snapshot() {
    let db = fixture().await;
    let id = new_conversation(
        &db,
        1,
        vec![
            ("awaitingHumanReplySince", Value::Integer(at(0))),
            ("sourceId", Value::Text("post-1".to_owned())),
        ],
    )
    .await;
    assert!(!set_snapshot(&db, id, at(0)).await);
}

#[tokio::test]
async fn response_sla_snapshot_window_states_are_read_in_id_order_and_scoped_to_the_workspace() {
    let db = fixture().await;
    let first = new_conversation(&db, 1, vec![]).await;
    let second = new_conversation(&db, 2, vec![]).await;
    let states = write(&db, move |c| {
        sla::read_window_states(c, WS, &[second, first], true)
    })
    .await;
    let foreign = ok(repo::read(&db, move |c| {
        sla::read_window_states(c, OTHER, &[first], false)
    })
    .await);
    assert_eq!(
        states.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![first, second]
    );
    assert!(foreign.is_empty());
}

#[tokio::test]
async fn response_sla_snapshot_policy_upsert_keeps_one_row_per_workspace_and_validates_through_the_checks()
 {
    let db = fixture().await;
    let upsert =
        |data: ResponseSlaPolicyInput| repo::write(&db, move |c| sla::upsert_policy(c, WS, data));
    ok(upsert(ResponseSlaPolicyInput {
        enabled: true,
        first_response_target_seconds: 300,
        next_response_target_seconds: None,
        breaching_threshold_percent: 25,
    })
    .await);
    let updated = ok(upsert(ResponseSlaPolicyInput {
        enabled: false,
        first_response_target_seconds: 600,
        next_response_target_seconds: Some(900),
        breaching_threshold_percent: 10,
    })
    .await);
    assert_eq!(updated.first_response_target_seconds, 600);
    assert!(!updated.enabled);
    let count = repo_common::scalar_i64(
        &db,
        "SELECT count(*) FROM \"ResponseSlaPolicy\" WHERE \"workspaceId\" = ?1",
        vec![repo_common::int(WS)],
    )
    .await;
    assert_eq!(count, Some(1));
    let invalid = upsert(ResponseSlaPolicyInput {
        enabled: true,
        first_response_target_seconds: 0,
        next_response_target_seconds: None,
        breaching_threshold_percent: 25,
    })
    .await;
    assert!(invalid.is_err());
    assert!(ok(repo::write(&db, |c| sla::delete_policy(c, WS)).await));
}
