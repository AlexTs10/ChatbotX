//! Port of `packages/database/__tests__/integration/working-hours-members-concurrency.test.ts`
//! (P3.3). Two Postgres connections -> two concurrent `repo::write` jobs (the one SQLite writer
//! serializes them, so a deadlock cannot happen; the asserted outcome is the same). The TS tests
//! share one workspace; each port seeds what its case reads.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo4.rs"]
mod repo4;

use chatbotx_db::repo;
use chatbotx_db::repo::working_hours::{
    ReplaceMembersResult, create, delete, delete_members_for_users, replace_members,
};
use common::{fresh_db, seed};
use repo_common::{int, ms, ok, scalar_i64, setup};

const ROUNDS: i64 = 20;
const WS: i64 = 2001;
const A: i64 = 31;
const B: i64 = 32;
const USERS: [i64; 3] = [1201, 1202, 1203];

async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        for u in USERS {
            repo4::user(tx, u)?;
            repo4::member(tx, WS, u)?;
        }
        repo4::working_hours(tx, A, WS, "A", 0)?;
        repo4::working_hours(tx, B, WS, "B", 0)?;
        Ok(())
    })
    .await;
    db
}

fn old() -> i64 {
    ms("2000-01-01T00:00:00Z")
}

async fn updated_at_of(db: &common::TestDb, id: i64) -> i64 {
    scalar_i64(
        db,
        r#"SELECT "updatedAt" FROM "WorkingHours" WHERE "id" = ?1"#,
        vec![int(id)],
    )
    .await
    .expect("schedule")
}

async fn age_schedules(db: &common::TestDb) {
    let old = old();
    setup(db, move |tx| {
        tx.execute(
            r#"UPDATE "WorkingHours" SET "updatedAt" = ?1 WHERE "id" IN (?2, ?3)"#,
            chatbotx_db::rusqlite::params![old, A, B],
        )?;
        Ok(())
    })
    .await;
}

/// Port of: packages/database/__tests__/integration/working-hours-members-concurrency.test.ts:96 "workingHoursRepository.replaceMembers concurrent saves > opposite-order moves of the same users on two schedules never deadlock"
#[tokio::test]
async fn working_hours_repository_replace_members_concurrent_saves_opposite_order_moves_of_the_same_users_on_two_schedules_never_deadlock()
 {
    let db = seeded().await;
    for _ in 0..ROUNDS {
        let forward = USERS.to_vec();
        let mut backward = USERS.to_vec();
        backward.reverse();
        let (first, second) = tokio::join!(
            repo::write(&db, move |tx| replace_members(tx, WS, A, &forward)),
            repo::write(&db, move |tx| replace_members(tx, WS, B, &backward)),
        );
        assert_eq!(ok(first), ReplaceMembersResult::Ok);
        assert_eq!(ok(second), ReplaceMembersResult::Ok);
    }
}

/// Port of: packages/database/__tests__/integration/working-hours-members-concurrency.test.ts:137 "workingHoursRepository.replaceMembers concurrent saves > moving a member bumps the schedule that lost them"
#[tokio::test]
async fn working_hours_repository_replace_members_concurrent_saves_moving_a_member_bumps_the_schedule_that_lost_them()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| replace_members(tx, WS, A, &[USERS[0]])).await);
    age_schedules(&db).await;
    ok(repo::write(&db, |tx| replace_members(tx, WS, B, &[USERS[0]])).await);
    assert!(updated_at_of(&db, A).await > old());
    assert!(updated_at_of(&db, B).await > old());
}

/// Port of: packages/database/__tests__/integration/working-hours-members-concurrency.test.ts:157 "workingHoursRepository.replaceMembers concurrent saves > removing a workspace member bumps the schedule they were on"
#[tokio::test]
async fn working_hours_repository_replace_members_concurrent_saves_removing_a_workspace_member_bumps_the_schedule_they_were_on()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| replace_members(tx, WS, A, &[USERS[1]])).await);
    age_schedules(&db).await;
    ok(repo::write(&db, |tx| delete_members_for_users(tx, WS, &[USERS[1]])).await);
    assert!(updated_at_of(&db, A).await > old());
    assert_eq!(updated_at_of(&db, B).await, old());
}

/// Port of: packages/database/__tests__/integration/working-hours-members-concurrency.test.ts:175 "workingHoursRepository.replaceMembers concurrent saves > moving a member while their old schedule is deleted never deadlocks"
#[tokio::test]
async fn working_hours_repository_replace_members_concurrent_saves_moving_a_member_while_their_old_schedule_is_deleted_never_deadlocks()
 {
    let db = seeded().await;
    let target = ok(repo::write(&db, |tx| create(tx, WS, "target")).await);
    for round in 0..ROUNDS {
        let old = ok(repo::write(&db, move |tx| create(tx, WS, &format!("old-{round}"))).await);
        ok(repo::write(&db, move |tx| replace_members(tx, WS, old.id, &[USERS[2]])).await);
        let target_id = target.id;
        let (moved, deleted) = tokio::join!(
            repo::write(&db, move |tx| replace_members(
                tx,
                WS,
                target_id,
                &[USERS[2]]
            )),
            repo::write(&db, move |tx| delete(tx, WS, old.id)),
        );
        assert_eq!(ok(moved), ReplaceMembersResult::Ok);
        ok(deleted);
    }
}
