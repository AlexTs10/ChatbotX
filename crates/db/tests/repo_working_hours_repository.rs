//! Port of `packages/database/__tests__/working-hours-repository.test.ts` (P3.3).
//! The mocked transaction call order (lock, delete, insert, upsert, bump) becomes the
//! observable effect on real rows: replaced intervals / members, and the `updatedAt` bump
//! of the touched schedules (seeded with `updatedAt` = 0). Row locks and the advisory lock
//! are no-ops under the one SQLite writer (P3.2 rule).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo4.rs"]
mod repo4;

use chatbotx_db::repo;
use chatbotx_db::repo::working_hours::{
    IntervalInput, ReplaceMembersResult, delete_members_for_users, replace_intervals,
    replace_members,
};
use common::{fresh_db, seed};
use repo_common::{int, ok, rows, scalar_i64, setup};

const WS: i64 = 2001;
const OTHER_WS: i64 = 2002;
const WH1: i64 = 11;
const WH2: i64 = 12;
const WH3: i64 = 13;
const U1: i64 = 1101;
const U2: i64 = 1102;
const U3: i64 = 1103;
const U9: i64 = 1109;

/// Three schedules of WS (updatedAt 0); users u1..u3 are members of WS, u9 is not.
/// Interval (1, 0, 60) on WH1.
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        repo4::workspace(tx, 1009, OTHER_WS)?;
        for id in [WH1, WH2, WH3] {
            repo4::working_hours(tx, id, WS, &format!("wh-{id}"), 0)?;
        }
        for u in [U1, U2, U3] {
            repo4::user(tx, u)?;
            repo4::member(tx, WS, u)?;
        }
        repo4::user(tx, U9)?;
        ins!(tx, "WorkingHoursInterval", {
            "id" => 1_i64, "workingHoursId" => WH1, "weekday" => 1_i64, "startMinute" => 0_i64,
            "endMinute" => 60_i64,
        })?;
        Ok(())
    })
    .await;
    db
}

async fn updated_at(db: &common::TestDb, id: i64) -> i64 {
    scalar_i64(
        db,
        r#"SELECT "updatedAt" FROM "WorkingHours" WHERE "id" = ?1"#,
        vec![int(id)],
    )
    .await
    .expect("schedule")
}

async fn intervals(db: &common::TestDb, id: i64) -> Vec<(i64, i64, i64)> {
    rows(
        db,
        r#"SELECT "weekday", "startMinute", "endMinute" FROM "WorkingHoursInterval"
           WHERE "workingHoursId" = ?1 ORDER BY "weekday", "startMinute""#,
        vec![int(id)],
    )
    .await
    .into_iter()
    .map(|r| match (&r[0], &r[1], &r[2]) {
        (
            chatbotx_db::rusqlite::types::Value::Integer(a),
            chatbotx_db::rusqlite::types::Value::Integer(b),
            chatbotx_db::rusqlite::types::Value::Integer(c),
        ) => (*a, *b, *c),
        other => panic!("bad interval row {other:?}"),
    })
    .collect()
}

/// `(workingHoursId, userId)` of every member row of WS, sorted.
async fn members(db: &common::TestDb) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = rows(
        db,
        r#"SELECT "workingHoursId", "userId" FROM "WorkingHoursMember" WHERE "workspaceId" = ?1"#,
        vec![int(WS)],
    )
    .await
    .into_iter()
    .map(|r| match (&r[0], &r[1]) {
        (
            chatbotx_db::rusqlite::types::Value::Integer(a),
            chatbotx_db::rusqlite::types::Value::Integer(b),
        ) => (*a, *b),
        other => panic!("bad member row {other:?}"),
    })
    .collect();
    out.sort_unstable();
    out
}

/// Port of: packages/database/__tests__/working-hours-repository.test.ts:133 "workingHoursRepository write transactions > replaceIntervals locks the parent row before deleting and inserting"
#[tokio::test]
async fn working_hours_repository_write_transactions_replace_intervals_locks_the_parent_row_before_deleting_and_inserting()
 {
    let db = seeded().await;
    let done = ok(repo::write(&db, |tx| {
        replace_intervals(
            tx,
            WS,
            WH1,
            &[IntervalInput {
                weekday: 1,
                start_minute: 540,
                end_minute: 1020,
            }],
        )
    })
    .await);
    assert!(done);
    assert_eq!(intervals(&db, WH1).await, vec![(1, 540, 1020)]);
    assert!(updated_at(&db, WH1).await > 0);
}

/// Port of: packages/database/__tests__/working-hours-repository.test.ts:144 "workingHoursRepository write transactions > replaceIntervals with no intervals only clears the schedule"
#[tokio::test]
async fn working_hours_repository_write_transactions_replace_intervals_with_no_intervals_only_clears_the_schedule()
 {
    let db = seeded().await;
    assert!(ok(repo::write(&db, |tx| replace_intervals(
        tx,
        WS,
        WH1,
        &[]
    ))
    .await));
    assert!(intervals(&db, WH1).await.is_empty());
}

/// Port of: packages/database/__tests__/working-hours-repository.test.ts:154 "workingHoursRepository write transactions > a missing parent row reports false and writes nothing else"
#[tokio::test]
async fn working_hours_repository_write_transactions_a_missing_parent_row_reports_false_and_writes_nothing_else()
 {
    let db = seeded().await;
    setup(&db, |tx| repo4::working_hours_member(tx, WH1, WS, U2)).await;
    let done = ok(repo::write(&db, |tx| {
        replace_intervals(
            tx,
            OTHER_WS,
            WH1,
            &[IntervalInput {
                weekday: 1,
                start_minute: 0,
                end_minute: 60,
            }],
        )
    })
    .await);
    let result = ok(repo::write(&db, |tx| replace_members(tx, OTHER_WS, WH1, &[U1])).await);
    assert!(!done);
    assert_eq!(result, ReplaceMembersResult::NotFound);
    assert_eq!(intervals(&db, WH1).await, vec![(1, 0, 60)]);
    assert_eq!(members(&db).await, vec![(WH1, U2)]);
    assert_eq!(updated_at(&db, WH1).await, 0);
}

/// Port of: packages/database/__tests__/working-hours-repository.test.ts:172 "workingHoursRepository write transactions > replaceMembers locks first, prunes, then upserts sorted so a user moves schedules without deadlocking"
#[tokio::test]
async fn working_hours_repository_write_transactions_replace_members_locks_first_prunes_then_upserts_sorted_so_a_user_moves_schedules_without_deadlocking()
 {
    let db = seeded().await;
    setup(&db, |tx| {
        repo4::working_hours_member(tx, WH2, WS, U1)?;
        repo4::working_hours_member(tx, WH3, WS, U2)?;
        repo4::working_hours_member(tx, WH1, WS, U3)?;
        Ok(())
    })
    .await;
    let result = ok(repo::write(&db, |tx| replace_members(tx, WS, WH1, &[U2, U1, U2])).await);
    assert_eq!(result, ReplaceMembersResult::Ok);
    assert_eq!(members(&db).await, vec![(WH1, U1), (WH1, U2)]);
    assert!(updated_at(&db, WH1).await > 0);
    assert!(updated_at(&db, WH2).await > 0);
    assert!(updated_at(&db, WH3).await > 0);
}

/// Port of: packages/database/__tests__/working-hours-repository.test.ts:204 "workingHoursRepository write transactions > replaceMembers writes nothing and reports users that are not workspace members"
#[tokio::test]
async fn working_hours_repository_write_transactions_replace_members_writes_nothing_and_reports_users_that_are_not_workspace_members()
 {
    let db = seeded().await;
    setup(&db, |tx| repo4::working_hours_member(tx, WH1, WS, U3)).await;
    let result = ok(repo::write(&db, |tx| replace_members(tx, WS, WH1, &[U1, U9])).await);
    assert_eq!(result, ReplaceMembersResult::InvalidUsers(vec![U9]));
    assert_eq!(members(&db).await, vec![(WH1, U3)]);
}

/// Port of: packages/database/__tests__/working-hours-repository.test.ts:217 "workingHoursRepository write transactions > replaceMembers with an empty list removes all members without inserting"
#[tokio::test]
async fn working_hours_repository_write_transactions_replace_members_with_an_empty_list_removes_all_members_without_inserting()
 {
    let db = seeded().await;
    setup(&db, |tx| {
        repo4::working_hours_member(tx, WH1, WS, U1)?;
        repo4::working_hours_member(tx, WH2, WS, U2)?;
        Ok(())
    })
    .await;
    let result = ok(repo::write(&db, |tx| replace_members(tx, WS, WH1, &[])).await);
    assert_eq!(result, ReplaceMembersResult::Ok);
    assert_eq!(members(&db).await, vec![(WH2, U2)]);
}

/// Port of: packages/database/__tests__/working-hours-repository.test.ts:227 "workingHoursRepository write transactions > replaceMembers does not bump anything when nobody moves between schedules"
#[tokio::test]
async fn working_hours_repository_write_transactions_replace_members_does_not_bump_anything_when_nobody_moves_between_schedules()
 {
    let db = seeded().await;
    setup(&db, |tx| repo4::working_hours_member(tx, WH2, WS, U2)).await;
    ok(repo::write(&db, |tx| replace_members(tx, WS, WH1, &[U1])).await);
    assert_eq!(updated_at(&db, WH2).await, 0);
    assert_eq!(updated_at(&db, WH3).await, 0);
    assert_eq!(members(&db).await, vec![(WH1, U1), (WH2, U2)]);
}

/// Port of: packages/database/__tests__/working-hours-repository.test.ts:240 "workingHoursRepository write transactions > deleteMembersForUsers bumps the schedules that lost a member without waiting on locked rows"
#[tokio::test]
async fn working_hours_repository_write_transactions_delete_members_for_users_bumps_the_schedules_that_lost_a_member_without_waiting_on_locked_rows()
 {
    let db = seeded().await;
    setup(&db, |tx| {
        repo4::working_hours_member(tx, WH2, WS, U1)?;
        repo4::working_hours_member(tx, WH3, WS, U2)?;
        Ok(())
    })
    .await;
    ok(repo::write(&db, |tx| delete_members_for_users(tx, WS, &[U1])).await);
    assert_eq!(members(&db).await, vec![(WH3, U2)]);
    assert!(updated_at(&db, WH2).await > 0);
    assert_eq!(updated_at(&db, WH3).await, 0);
}

/// Port of: packages/database/__tests__/working-hours-repository.test.ts:250 "workingHoursRepository write transactions > deleteMembersForUsers leaves schedules alone when the user was on none"
#[tokio::test]
async fn working_hours_repository_write_transactions_delete_members_for_users_leaves_schedules_alone_when_the_user_was_on_none()
 {
    let db = seeded().await;
    setup(&db, |tx| repo4::working_hours_member(tx, WH2, WS, U2)).await;
    ok(repo::write(&db, |tx| delete_members_for_users(tx, WS, &[U1])).await);
    for id in [WH1, WH2, WH3] {
        assert_eq!(updated_at(&db, id).await, 0);
    }
    assert_eq!(members(&db).await, vec![(WH2, U2)]);
}

/// Port of: packages/database/__tests__/working-hours-repository.test.ts:261 "workingHoursRepository write transactions > deleteMembersForUsers with no users does nothing"
#[tokio::test]
async fn working_hours_repository_write_transactions_delete_members_for_users_with_no_users_does_nothing()
 {
    let db = seeded().await;
    setup(&db, |tx| repo4::working_hours_member(tx, WH2, WS, U2)).await;
    ok(repo::write(&db, |tx| delete_members_for_users(tx, WS, &[])).await);
    assert_eq!(members(&db).await, vec![(WH2, U2)]);
    assert_eq!(updated_at(&db, WH2).await, 0);
}
