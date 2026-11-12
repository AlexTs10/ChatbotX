//! Port of `packages/database/__tests__/connect-session-repository.test.ts` (P3.2).
//! The mocked client becomes real rows of a migrated SQLite file (adapted).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::connect_session as cs;
use chatbotx_db::repo::{self, now_ms};
use repo_common::csess::seed_session;
use repo_common::ok;
use serde_json::json;

/// Port of: packages/database/__tests__/connect-session-repository.test.ts:32 "connectSessionRepository > rejects malformed nextAction data read from the database"
#[tokio::test]
async fn connect_session_repository_rejects_malformed_next_action_data_read_from_the_database() {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    repo_common::setup(&db, |tx| {
        seed_session(tx, 1, 2001, 1001, "n")?;
        tx.execute(r#"UPDATE "ConnectSession" SET "nextAction" = '{"type":"unknown"}', "targets" = '[]' WHERE "id" = 1"#, [])?;
        Ok(())
    })
    .await;
    let err = repo::read(&db, |c| cs::find_by_id(c, 1))
        .await
        .expect_err("malformed");
    assert!(err.message().contains("[\"type\"]"), "{err}");
}

/// Port of: packages/database/__tests__/connect-session-repository.test.ts:45 "connectSessionRepository > completes reconnects only for the matching unexpired claimed session"
#[tokio::test]
async fn connect_session_repository_completes_reconnects_only_for_the_matching_unexpired_claimed_session()
 {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    common::seed_workspace(&db, 1002, 2002).await;
    repo_common::setup(&db, |tx| {
        seed_session(tx, 1, 2001, 1001, "a")?;
        seed_session(tx, 2, 2001, 1001, "b")?;
        seed_session(tx, 3, 2001, 1001, "c")?;
        tx.execute(
            r#"UPDATE "ConnectSession" SET "status" = 'authorized' WHERE "id" IN (1, 3)"#,
            [],
        )?;
        tx.execute(
            r#"UPDATE "ConnectSession" SET "expiresAt" = ?1 WHERE "id" = 3"#,
            [now_ms() - 1000],
        )?;
        Ok(())
    })
    .await;
    let result =
        json!({ "targetId": "target-1", "status": "connected", "connectionId": "connection-1" });
    for (id, ws) in [(1, 2002), (2, 2001), (3, 2001), (99, 2001)] {
        let r = result.clone();
        let out = ok(repo::write(&db, move |tx| {
            cs::complete_reconnect(tx, id, ws, &r, now_ms())
        })
        .await);
        assert!(out.is_none(), "session {id} in {ws}");
    }
    let r = result.clone();
    assert!(
        ok(repo::write(&db, move |tx| cs::complete_reconnect(
            tx,
            1,
            2001,
            &r,
            now_ms()
        ))
        .await)
        .is_some()
    );
}
