//! Port of `packages/database/__tests__/integration/working-hours-constraints.test.ts`, the
//! constraint cases (P2.4 spec section 3.1, rows 9.1-9.3: `:132`, `:165`, `:186`). The
//! `replaceMembers` / `replaceIntervals` / `delete` / `findForUser` cases test the P3
//! working-hours repository. Class C: runs against a migrated SQLite file.
//!
//! Setup is adapted: the original creates the schedule with `workingHoursRepository.create`
//! (a P3 repository) and also seeds an owner `WorkspaceMember`; here the schedule is one raw
//! `INSERT` and the member row is left out (no constraint under test reads it). Every
//! statement that must fail runs as its own write job (the job rolls back, like the original's
//! nested transaction / savepoint); its `DbError` carries the mapped SQLSTATE and constraint.

mod common;

use chatbotx_db::rusqlite::{self, params};
use chatbotx_db::{Db, DbError};
use common::{fks, fresh_db, seed, seed_workspace, table_sql};

/// Inserts one `WorkingHours` schedule (the `workingHoursRepository.create` of the original).
fn insert_schedule(
    tx: &rusqlite::Connection,
    id: i64,
    workspace_id: i64,
    name: &str,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "WorkingHours" ("id", "workspaceId", "name") VALUES (?1, ?2, ?3)"#,
        params![id, workspace_id, name],
    )?;
    Ok(())
}

/// The `insertInterval` of the original.
fn insert_interval(
    tx: &rusqlite::Connection,
    id: i64,
    working_hours_id: i64,
    weekday: i64,
    start_minute: i64,
    end_minute: i64,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "WorkingHoursInterval" ("id", "workingHoursId", "weekday", "startMinute", "endMinute")
           VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![id, working_hours_id, weekday, start_minute, end_minute],
    )?;
    Ok(())
}

/// Inserts one `WorkingHoursMember` row.
fn insert_member(
    tx: &rusqlite::Connection,
    id: i64,
    working_hours_id: i64,
    workspace_id: i64,
    user_id: i64,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "WorkingHoursMember" ("id", "workingHoursId", "workspaceId", "userId")
           VALUES (?1, ?2, ?3, ?4)"#,
        params![id, working_hours_id, workspace_id, user_id],
    )?;
    Ok(())
}

/// Runs statements that must succeed, as one committed write job.
async fn accepts<F>(db: &Db, what: &str, statement: F)
where
    F: FnOnce(&rusqlite::Connection) -> Result<(), DbError> + Send + 'static,
{
    db.write(move |tx| statement(tx))
        .await
        .unwrap_or_else(|e| panic!("{what} must succeed: {e}"));
}

/// The `expectConstraintViolation` of the original: the statement fails in its own (rolled
/// back) write job and the error's constraint name contains `constraint`.
async fn expect_constraint_violation<F>(
    db: &Db,
    what: &str,
    statement: F,
    constraint: &str,
) -> DbError
where
    F: FnOnce(&rusqlite::Connection) -> Result<(), DbError> + Send + 'static,
{
    let error = db
        .write(move |tx| statement(tx))
        .await
        .err()
        .unwrap_or_else(|| panic!("{what} must be rejected"));
    assert!(
        error.constraint().is_some_and(|c| c.contains(constraint)),
        "{what}: constraint {:?} must contain {constraint} ({error})",
        error.constraint()
    );
    error
}

/// Port of: packages/database/__tests__/integration/working-hours-constraints.test.ts:132 "working hours constraints against Postgres > interval CHECKs accept 00:00-24:00 and reject out-of-range or empty ranges"
#[tokio::test]
async fn interval_checks_accept_00_00_24_00_and_reject_out_of_range_or_empty_ranges() {
    let db = fresh_db().await;
    let ws = seed(&db).await.workspace_id;
    let schedule = 6001;
    accepts(&db, "the schedule", move |tx| {
        insert_schedule(tx, schedule, ws, "Office")
    })
    .await;

    accepts(&db, "interval (0, 0, 1440)", move |tx| {
        insert_interval(tx, 6101, schedule, 0, 0, 1440)
    })
    .await;
    accepts(&db, "interval (6, 1439, 1440)", move |tx| {
        insert_interval(tx, 6102, schedule, 6, 1439, 1440)
    })
    .await;

    let cases: [(&str, i64, i64, i64, &str); 4] = [
        ("weekday 7", 7, 0, 60, "WorkingHoursInterval_weekday_check"),
        (
            "empty range 60-60",
            1,
            60,
            60,
            "WorkingHoursInterval_minutes_check",
        ),
        ("end 1441", 1, 0, 1441, "WorkingHoursInterval_minutes_check"),
        ("start -1", 1, -1, 60, "WorkingHoursInterval_minutes_check"),
    ];
    for (i, (what, weekday, start, end, constraint)) in cases.into_iter().enumerate() {
        let id = 6110 + i as i64;
        expect_constraint_violation(
            &db,
            what,
            move |tx| insert_interval(tx, id, schedule, weekday, start, end),
            constraint,
        )
        .await;
    }
}

/// Port of: packages/database/__tests__/integration/working-hours-constraints.test.ts:165 "working hours constraints against Postgres > the composite FK rejects a member whose workspaceId differs from the schedule's"
///
/// Replacement: SQLite FK errors carry no constraint name (P2.3 spec 2.4). The insert fails
/// with SQLSTATE 23503, and the catalog shows the composite FK
/// `(workingHoursId, workspaceId) -> WorkingHours (id, workspaceId)` with ON DELETE CASCADE,
/// declared as `WorkingHoursMember_workingHoursId_workspaceId_fkey` in the table SQL.
#[tokio::test]
async fn the_composite_fk_rejects_a_member_whose_workspace_id_differs_from_the_schedules() {
    let db = fresh_db().await;
    let a = seed(&db).await.workspace_id;
    let (b_owner, b) = seed_workspace(&db, 1002, 2002).await;
    let schedule = 6201;
    accepts(&db, "the schedule of workspace a", move |tx| {
        insert_schedule(tx, schedule, a, "Office")
    })
    .await;

    let error = db
        .write(move |tx| insert_member(tx, 6301, schedule, b, b_owner))
        .await
        .expect_err("a member of workspace b on a schedule of workspace a must be rejected");
    assert_eq!(error.sqlstate(), Some("23503"), "{error}");

    let all = fks(&db, "WorkingHoursMember").await;
    let composite: Vec<_> = all
        .iter()
        .filter(|fk| fk.parent == "WorkingHours")
        .collect();
    assert_eq!(
        composite
            .iter()
            .map(|fk| (fk.from.as_str(), fk.to.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("workingHoursId", Some("id")),
            ("workspaceId", Some("workspaceId"))
        ],
        "composite FK columns: {all:?}"
    );
    assert!(
        composite.iter().all(|fk| fk.id == composite[0].id),
        "one FK, not two: {all:?}"
    );
    assert!(
        composite.iter().all(|fk| fk.on_delete == "CASCADE"),
        "{all:?}"
    );

    let sql = table_sql(&db, "WorkingHoursMember")
        .await
        .expect("table WorkingHoursMember");
    assert!(
        sql.contains(
            r#"CONSTRAINT "WorkingHoursMember_workingHoursId_workspaceId_fkey" FOREIGN KEY"#
        ),
        "{sql}"
    );
}

/// Port of: packages/database/__tests__/integration/working-hours-constraints.test.ts:186 "working hours constraints against Postgres > a user has at most one schedule per workspace"
#[tokio::test]
async fn a_user_has_at_most_one_schedule_per_workspace() {
    let db = fresh_db().await;
    let s = seed(&db).await;
    let (ws, owner) = (s.workspace_id, s.user_id);
    let (first, second) = (6401, 6402);
    accepts(&db, "two schedules and the first membership", move |tx| {
        insert_schedule(tx, first, ws, "A")?;
        insert_schedule(tx, second, ws, "B")?;
        insert_member(tx, 6501, first, ws, owner)
    })
    .await;

    let error = expect_constraint_violation(
        &db,
        "a second schedule for the same user",
        move |tx| insert_member(tx, 6502, second, ws, owner),
        "WorkingHoursMember_workspaceId_userId_key",
    )
    .await;
    assert_eq!(error.sqlstate(), Some("23505"), "{error}");
    assert_eq!(
        error.constraint(),
        Some("WorkingHoursMember_workspaceId_userId_key")
    );
}
