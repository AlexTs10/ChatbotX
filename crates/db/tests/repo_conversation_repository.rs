//! Port of `packages/database/__tests__/conversation-repository.test.ts` (P3.2b).
//! The TS mocks `db.insert` / `db.update` and checks the `values` / `set` / `where`
//! objects; here the statements run on a migrated temp file (adapted) and the rows show
//! the DM rows, the conflict skip, the assignment guard and the `assignedUserAt` stamp.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::conversation::{self as conv, AssignTargetInput};
use chatbotx_db::repo::models::Conversation;
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::{Db, WriteTx};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{insert_contact, insert_conv, ms, ok, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
const USER: i64 = 1001;
const USER2: i64 = 1002;
const TEAM: i64 = 31;
const OLD_ASSIGNED_AT: i64 = 1_700_000_000_000;

/// Contacts 4101..4104 (WS). Conversations: 91 (4101, unassigned), 92 (4102, assigned to
/// USER), 93 (4103, assigned to team 31), 94 (4104, unassigned, an old `assignedUserAt`).
async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, USER2, OTHER).await;
    setup(&db, |tx| {
        ins!(tx, "InboxTeam", { "id" => TEAM, "workspaceId" => WS, "name" => "Support".to_owned() })?;
        for (contact, conversation) in [(4101, 91), (4102, 92), (4103, 93), (4104, 94)] {
            insert_contact(tx, contact, WS)?;
            insert_conv(tx, conversation, WS, contact, None)?;
        }
        tx.execute(
            "UPDATE \"Conversation\" SET \"assignedUserId\" = ?1 WHERE \"id\" = 92",
            [USER],
        )?;
        tx.execute(
            "UPDATE \"Conversation\" SET \"assignedInboxTeamId\" = ?1 WHERE \"id\" = 93",
            [TEAM],
        )?;
        tx.execute(
            "UPDATE \"Conversation\" SET \"assignedUserAt\" = ?1 WHERE \"id\" = 94",
            [OLD_ASSIGNED_AT],
        )?;
        Ok(())
    })
    .await;
    db
}

async fn run<T, F>(db: &Db, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> repo::Result<T> + Send + 'static,
{
    ok(repo::read(db, f).await)
}

async fn write<T, F>(db: &Db, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce(&mut WriteTx<'_>) -> repo::Result<T> + Send + 'static,
{
    ok(repo::write(db, f).await)
}

async fn conversations_of(db: &Db, contact_id: i64) -> Vec<Conversation> {
    run(db, move |c| {
        repo::find_many::<Conversation>(
            c,
            "WHERE \"contactId\" = ?1 ORDER BY \"id\"",
            &[&contact_id],
        )
    })
    .await
}

async fn conversation(db: &Db, id: i64) -> Conversation {
    run(db, move |c| {
        repo::find_one::<Conversation>(c, "\"id\" = ?1", &[&id])
    })
    .await
    .expect("conversation row")
}

/// Contacts 4201, 4202 (WS) without conversations.
async fn dm_fixture() -> common::TestDb {
    let db = fixture().await;
    setup(&db, |tx| {
        insert_contact(tx, 4201, WS)?;
        insert_contact(tx, 4202, WS)
    })
    .await;
    db
}

#[tokio::test]
async fn conversation_repository_insert_dm_conversations_if_absent_inserts_one_dm_source_id_null_per_contact_each_with_its_own_id()
 {
    let db = dm_fixture().await;
    write(&db, |c| {
        conv::insert_dm_conversations_if_absent(c, WS, &[4201, 4202])
    })
    .await;
    let mut ids = Vec::new();
    for contact in [4201, 4202] {
        let rows = conversations_of(&db, contact).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (
                rows[0].workspace_id,
                rows[0].contact_id,
                rows[0].source_id.clone()
            ),
            (WS, contact, None)
        );
        ids.push(rows[0].id);
    }
    assert_ne!(ids[0], ids[1]);
}

#[tokio::test]
async fn conversation_repository_insert_dm_conversations_if_absent_skips_conflicts_instead_of_failing_so_a_dm_opened_concurrently_is_not_an_error()
 {
    let db = dm_fixture().await;
    setup(&db, |tx| insert_conv(tx, 95, WS, 4201, None)).await;
    write(&db, |c| {
        conv::insert_dm_conversations_if_absent(c, WS, &[4201, 4202])
    })
    .await;
    let kept = conversations_of(&db, 4201).await;
    assert_eq!(kept.iter().map(|r| r.id).collect::<Vec<_>>(), vec![95]);
    assert_eq!(conversations_of(&db, 4202).await.len(), 1);
}

#[tokio::test]
async fn conversation_repository_insert_dm_conversations_if_absent_uses_the_transaction_handle_it_is_given()
 {
    let db = dm_fixture().await;
    // The insert runs on the caller's write job: it sees its own row, and a failing job
    // rolls it back.
    let result = repo::write(&db, |c| {
        conv::insert_dm_conversations_if_absent(c, WS, &[4201])?;
        let seen: i64 = c.query_row(
            "SELECT count(*) FROM \"Conversation\" WHERE \"contactId\" = 4201",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(seen, 1);
        Err::<(), _>(repo::Error::msg("roll back"))
    })
    .await;
    assert!(result.is_err());
    assert!(conversations_of(&db, 4201).await.is_empty());
}

#[tokio::test]
async fn conversation_repository_assign_user_if_unassigned_scopes_the_update_to_the_workspace_and_conversation_id_guarded_by_both_assignment_columns_being_null()
 {
    let db = fixture().await;
    let foreign = write(&db, |c| {
        conv::assign_user_if_unassigned(c, OTHER, 91, USER, None)
    })
    .await;
    assert!(foreign.is_empty());
    let team = write(&db, |c| {
        conv::assign_user_if_unassigned(c, WS, 93, USER, None)
    })
    .await;
    assert!(team.is_empty());
    let before = now_ms();
    let claimed = write(&db, |c| {
        conv::assign_user_if_unassigned(c, WS, 91, USER, None)
    })
    .await;
    assert_eq!(claimed.len(), 1);
    assert_eq!(
        (
            claimed[0].id,
            claimed[0].contact_id,
            claimed[0].assigned_user_id
        ),
        (91, 4101, Some(USER))
    );
    assert!(claimed[0].assigned_user_at.is_some_and(|at| at >= before));
    assert_eq!(conversation(&db, 93).await.assigned_user_id, None);
}

#[tokio::test]
async fn conversation_repository_assign_user_if_unassigned_stamps_assigned_user_at_with_the_supplied_instant()
 {
    let db = fixture().await;
    let at = ms("2026-10-18T09:00:00.000Z");
    let claimed = write(&db, move |c| {
        conv::assign_user_if_unassigned(c, WS, 91, USER, Some(at))
    })
    .await;
    assert_eq!(claimed.first().and_then(|r| r.assigned_user_at), Some(at));
}

#[tokio::test]
async fn conversation_repository_assign_user_if_unassigned_returns_an_empty_array_when_the_guard_excludes_the_row_already_assigned()
 {
    let db = fixture().await;
    let claimed = write(&db, |c| {
        conv::assign_user_if_unassigned(c, WS, 92, USER2, None)
    })
    .await;
    assert!(claimed.is_empty());
    assert_eq!(conversation(&db, 92).await.assigned_user_id, Some(USER));
}

#[tokio::test]
async fn conversation_repository_assign_target_if_unassigned_sets_assigned_user_at_when_a_user_is_assigned()
 {
    let db = fixture().await;
    let at = ms("2026-10-18T09:00:00.000Z");
    let input = AssignTargetInput {
        workspace_id: WS,
        conversation_id: 91,
        assigned_user_id: Some(USER),
        assigned_at: Some(at),
        ..AssignTargetInput::default()
    };
    let rows = write(&db, move |c| conv::assign_target_if_unassigned(c, &input)).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].assigned_inbox_team_id,
            rows[0].assigned_user_id,
            rows[0].assigned_user_at
        ),
        (None, Some(USER), Some(at))
    );
}

#[tokio::test]
async fn conversation_repository_assign_target_if_unassigned_does_not_touch_assigned_user_at_for_a_team_only_assignment()
 {
    let db = fixture().await;
    let input = AssignTargetInput {
        workspace_id: WS,
        conversation_id: 94,
        assigned_inbox_team_id: Some(TEAM),
        assigned_at: Some(ms("2026-10-18T09:00:00.000Z")),
        ..AssignTargetInput::default()
    };
    let rows = write(&db, move |c| conv::assign_target_if_unassigned(c, &input)).await;
    assert_eq!(rows.len(), 1);
    let row = conversation(&db, 94).await;
    assert_eq!(
        (
            row.assigned_inbox_team_id,
            row.assigned_user_id,
            row.assigned_user_at
        ),
        (Some(TEAM), None, Some(OLD_ASSIGNED_AT))
    );
}
