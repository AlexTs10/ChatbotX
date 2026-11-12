//! Port of `packages/database/__tests__/integration/connect-session-isolation.test.ts`
//! (P3.2). Runs against a migrated SQLite temp file (a fresh file per test instead of a
//! rolled-back transaction). The two-connection races become two concurrent `db.write`
//! jobs (one SQLite writer serializes them; the asserted outcome is the same).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::chunked_purge::ChunkedPurgeStopReason;
use chatbotx_db::repo::connect_session::{self as cs, ConnectSessionPatch};
use chatbotx_db::repo::models::ConnectSession;
use chatbotx_db::repo::{self, Json, now_ms};
use repo_common::csess::{future, seed_session};
use repo_common::{int, ok};
use serde_json::{Value, json};

const A: i64 = 2001;
const OWNER: i64 = 1001;
const B: i64 = 2002;

async fn db() -> common::TestDb {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    common::seed_workspace(&db, 1002, B).await;
    db
}

async fn session(db: &Db, id: i64, ws: i64, nonce: &'static str) -> i64 {
    let actor = if ws == A { OWNER } else { 1002 };
    repo_common::setup(db, move |tx| seed_session(tx, id, ws, actor, nonce)).await
}

async fn exec(db: &Db, sql: &'static str, args: Vec<chatbotx_db::rusqlite::types::Value>) {
    repo_common::handover::exec(db, sql, args).await;
}

async fn find(db: &Db, id: i64) -> Option<ConnectSession> {
    ok(repo::read(db, move |c| cs::find_by_id(c, id)).await)
}

async fn claim(
    db: &Db,
    id: i64,
    ws: i64,
    target: &'static str,
    owner: &'static str,
    lease: i64,
) -> bool {
    ok(repo::write(db, move |tx| {
        cs::claim_target(tx, id, ws, target, owner, lease, now_ms())
    })
    .await)
}

async fn release(db: &Db, id: i64, ws: i64, target: &'static str, owner: &'static str) {
    ok(repo::write(db, move |tx| {
        cs::release_target(tx, id, ws, target, owner, now_ms())
    })
    .await);
}

async fn append(db: &Db, id: i64, ws: i64, results: Value, ids: &[&str]) -> Option<ConnectSession> {
    let results: Vec<Value> = results.as_array().cloned().unwrap_or_default();
    let ids: Vec<String> = ids.iter().map(|s| (*s).to_owned()).collect();
    ok(repo::write(db, move |tx| {
        cs::append_results(tx, id, ws, &results, &ids, now_ms())
    })
    .await)
}

fn owner_of(s: &ConnectSession, target: &str) -> Option<String> {
    s.target_claims
        .0
        .get(target)
        .and_then(|c| c.get("ownerToken"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn past() -> i64 {
    now_ms() - 60_000
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:104 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > findByIdForWorkspace returns the session when the workspace matches"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_find_by_id_for_workspace_returns_the_session_when_the_workspace_matches()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-1").await;
    let found = ok(repo::read(&db, move |c| cs::find_by_id_for_workspace(c, id, A)).await);
    assert_eq!(found.map(|s| s.id), Some(id));
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:121 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > findByIdForWorkspace rejects a session id from another workspace"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_find_by_id_for_workspace_rejects_a_session_id_from_another_workspace()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-2").await;
    assert!(ok(repo::read(&db, move |c| cs::find_by_id_for_workspace(c, id, B)).await).is_none());
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:140 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > findByStateNonceHash resolves the globally unique OAuth callback nonce"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_find_by_state_nonce_hash_resolves_the_globally_unique_o_auth_callback_nonce()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-nonce").await;
    let found = ok(repo::read(&db, |c| cs::find_by_state_nonce_hash(c, "iso-hash-nonce")).await);
    assert_eq!(found.map(|s| s.id), Some(id));
    assert!(
        ok(repo::read(&db, |c| cs::find_by_state_nonce_hash(c, "missing-nonce")).await).is_none()
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:163 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > rejects a session with both actor foreign keys"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_rejects_a_session_with_both_actor_foreign_keys()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        ins!(tx, "WorkspaceApiToken", { "id" => 77_i64, "workspaceId" => A, "name" => "Connect session actor constraint", "permission" => "full", "tokenHash" => "actor-constraint", "tokenPrefix" => "cbx_ws_actor" })?;
        Ok(())
    })
    .await;
    let err = db
        .write(|tx| {
            ins!(tx, "ConnectSession", { "id" => 9_i64, "workspaceId" => A, "provider" => "messenger", "purpose" => "connect", "actorUserId" => OWNER, "actorTokenId" => 77_i64, "stateNonceHash" => "actor-constraint", "expiresAt" => future() })?;
            Ok(())
        })
        .await
        .expect_err("must fail");
    assert_eq!(err.sqlstate(), Some("23514"));
    assert_eq!(err.constraint(), Some("ConnectSession_actor_at_most_one"));
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:197 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > countActiveByWorkspaceId never counts another workspace's pending sessions"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_count_active_by_workspace_id_never_counts_another_workspace_s_pending_sessions()
 {
    let db = db().await;
    session(&db, 1, A, "iso-hash-3").await;
    session(&db, 2, A, "iso-hash-4").await;
    assert_eq!(
        ok(repo::read(&db, |c| cs::count_active_by_workspace_id(c, B, now_ms())).await),
        0
    );
    assert_eq!(
        ok(repo::read(&db, |c| cs::count_active_by_workspace_id(c, A, now_ms())).await),
        2
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:226 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > countActiveByWorkspaceId excludes terminal and expired active statuses"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_count_active_by_workspace_id_excludes_terminal_and_expired_active_statuses()
 {
    let db = db().await;
    session(&db, 1, A, "iso-hash-count-active").await;
    session(&db, 2, A, "iso-hash-count-terminal").await;
    session(&db, 3, A, "iso-hash-count-expired").await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET status = 'completed', "consumedAt" = ?1 WHERE id = 2"#,
        vec![int(now_ms())],
    )
    .await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET "expiresAt" = ?1 WHERE id = 3"#,
        vec![int(past())],
    )
    .await;
    assert_eq!(
        ok(repo::read(&db, |c| cs::count_active_by_workspace_id(c, A, now_ms())).await),
        1
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:261 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > expireDue expires only due sessions in the requested statuses"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_expire_due_expires_only_due_sessions_in_the_requested_statuses()
 {
    let db = db().await;
    session(&db, 1, A, "iso-hash-expire-due").await;
    session(&db, 2, A, "iso-hash-expire-future").await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET "expiresAt" = ?1 WHERE id = 1"#,
        vec![int(past())],
    )
    .await;
    let n = ok(repo::write(&db, |tx| {
        cs::expire_due(tx, now_ms(), &["awaiting_selection"], now_ms())
    })
    .await);
    assert_eq!(n, 1);
    let s = find(&db, 1).await.expect("session");
    assert_eq!(s.status, "expired");
    assert_eq!(s.encrypted_auth, None);
    assert!(s.consumed_at.is_some());
    assert_eq!(
        find(&db, 2).await.map(|s| s.status).as_deref(),
        Some("awaiting_selection")
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:294 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > claimTarget's compare-and-set: a second claim of the SAME target after it's already claimed returns false and does not replace the lease"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_claim_target_s_compare_and_set_a_second_claim_of_the_same_target_after_it_s_already_claimed_returns_false_and_does_not_replace_the_lease()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-5").await;
    assert!(claim(&db, id, A, "t1", "owner-a", future()).await);
    assert!(!claim(&db, id, A, "t1", "owner-b", future()).await);
    assert_eq!(
        owner_of(&find(&db, id).await.unwrap(), "t1").as_deref(),
        Some("owner-a")
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:336 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > claimTarget rejects completed and expired sessions"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_claim_target_rejects_completed_and_expired_sessions()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-claim-state-expiry").await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET status = 'completed', "consumedAt" = ?1 WHERE id = 1"#,
        vec![int(now_ms())],
    )
    .await;
    assert!(!claim(&db, id, A, "t1", "owner-a", future()).await);
    exec(&db, r#"UPDATE "ConnectSession" SET status = 'awaiting_selection', "consumedAt" = NULL, "expiresAt" = ?1 WHERE id = 1"#, vec![int(past())]).await;
    assert!(!claim(&db, id, A, "t1", "owner-a", future()).await);
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:384 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > releaseTarget preserves a claim outside its workspace, state, and expiry guard"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_release_target_preserves_a_claim_outside_its_workspace_state_and_expiry_guard()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-release-guard").await;
    claim(&db, id, A, "t1", "owner-a", future()).await;
    release(&db, id, B, "t1", "owner-a").await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET status = 'completed', "consumedAt" = ?1 WHERE id = 1"#,
        vec![int(now_ms())],
    )
    .await;
    release(&db, id, A, "t1", "owner-a").await;
    exec(&db, r#"UPDATE "ConnectSession" SET status = 'awaiting_selection', "consumedAt" = NULL, "expiresAt" = ?1 WHERE id = 1"#, vec![int(past())]).await;
    release(&db, id, A, "t1", "owner-a").await;
    assert_eq!(
        owner_of(&find(&db, id).await.unwrap(), "t1").as_deref(),
        Some("owner-a")
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:456 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > claimTarget lets exactly one of two concurrent claims win"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_claim_target_lets_exactly_one_of_two_concurrent_claims_win()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        ins!(tx, "ConnectSession", { "id" => 1_i64, "workspaceId" => A, "provider" => "messenger", "purpose" => "connect", "actorUserId" => OWNER, "stateNonceHash" => "iso-hash-race", "status" => "awaiting_selection", "step" => "select", "targets" => r#"[{"id":"race-target","name":"R","selectable":true}]"#, "expiresAt" => future() })?;
        Ok(())
    })
    .await;
    let lease = future();
    let (a, b) = tokio::join!(
        repo::write(&db, move |tx| cs::claim_target(
            tx,
            1,
            A,
            "race-target",
            "owner-a",
            lease,
            now_ms()
        )),
        repo::write(&db, move |tx| cs::claim_target(
            tx,
            1,
            A,
            "race-target",
            "owner-b",
            lease,
            now_ms()
        )),
    );
    let wins = [ok(a), ok(b)].iter().filter(|w| **w).count();
    assert_eq!(wins, 1);
    let owner = owner_of(&find(&db, 1).await.unwrap(), "race-target").unwrap_or_default();
    assert!(owner == "owner-a" || owner == "owner-b");
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:549 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > claimTarget lets a new owner take over only an EXPIRED lease, never one still in flight (regression: a failed claimant's release must not drop a concurrent in-flight claim)"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_claim_target_lets_a_new_owner_take_over_only_an_expired_lease_never_one_still_in_flight_regression_a_failed_claimant_s_release_must_not_drop_a_concurrent_in_flight_claim()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-lease-takeover").await;
    assert!(claim(&db, id, A, "t1", "owner-a", past()).await);
    assert!(claim(&db, id, A, "t1", "owner-b", future()).await);
    release(&db, id, A, "t1", "owner-a").await;
    assert_eq!(
        owner_of(&find(&db, id).await.unwrap(), "t1").as_deref(),
        Some("owner-b")
    );
    assert!(!claim(&db, id, A, "t1", "owner-c", future()).await);
}

const AUTH: &str = r#"{"iv":"x","ciphertext":"y","keyId":"k"}"#;

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:617 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > appendResults does not let a non-selectable/unknown id's outcome count toward completion or success"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_append_results_does_not_let_a_non_selectable_unknown_id_s_outcome_count_toward_completion_or_success()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-append-1").await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET "encryptedAuth" = ?1 WHERE id = 1"#,
        vec![repo_common::text(AUTH)],
    )
    .await;
    let updated = append(&db, id, A, json!([{ "targetId": "t1", "status": "connected", "connectionId": "conn-1" }, { "targetId": "unknown-id", "status": "failed", "reason": "unknown" }]), &["conn-1"]).await.expect("updated");
    assert_eq!(
        (updated.status.as_str(), updated.step.as_str()),
        ("awaiting_selection", "select")
    );
    assert!(updated.encrypted_auth.is_some());
    let completed = append(
        &db,
        id,
        A,
        json!([{ "targetId": "t2", "status": "connected", "connectionId": "conn-2" }]),
        &["conn-2"],
    )
    .await
    .expect("completed");
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.encrypted_auth, None);
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:667 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > appendResults does not treat a non-selectable target's duplicated outcome as the success that completes the batch"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_append_results_does_not_treat_a_non_selectable_target_s_duplicated_outcome_as_the_success_that_completes_the_batch()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        ins!(tx, "ConnectSession", { "id" => 1_i64, "workspaceId" => A, "provider" => "messenger", "purpose" => "connect", "actorUserId" => OWNER, "stateNonceHash" => "iso-hash-append-2", "status" => "awaiting_selection", "step" => "select", "targets" => r#"[{"id":"t1","name":"A","selectable":true},{"id":"already","name":"Already connected","selectable":false,"alreadyConnected":"other_workspace"}]"#, "expiresAt" => future() })?;
        Ok(())
    })
    .await;
    let updated = append(
        &db,
        1,
        A,
        json!([{ "targetId": "already", "status": "duplicated", "reason": "alreadyConnected" }]),
        &[],
    )
    .await;
    assert_eq!(
        updated.map(|s| s.status).as_deref(),
        Some("awaiting_selection")
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:718 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > appendResults is a no-op (0 rows, returns undefined) on an already-terminal session — a replayed/duplicate batch cannot re-complete or overwrite a finished session"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_append_results_is_a_no_op_0_rows_returns_undefined_on_an_already_terminal_session_a_replayed_duplicate_batch_cannot_re_complete_or_overwrite_a_finished_session()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-append-3").await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET status = 'completed', "consumedAt" = ?1 WHERE id = 1"#,
        vec![int(now_ms())],
    )
    .await;
    assert!(
        append(
            &db,
            id,
            A,
            json!([{ "targetId": "t1", "status": "connected" }]),
            &[]
        )
        .await
        .is_none()
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:744 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > appendResults rejects an expired awaiting-selection session"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_append_results_rejects_an_expired_awaiting_selection_session()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-append-expiry").await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET "expiresAt" = ?1 WHERE id = 1"#,
        vec![int(past())],
    )
    .await;
    assert!(
        append(
            &db,
            id,
            A,
            json!([{ "targetId": "t1", "status": "connected" }]),
            &["conn-1"]
        )
        .await
        .is_none()
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:770 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > does not mutate an active session outside its workspace"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_does_not_mutate_an_active_session_outside_its_workspace()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-cross-workspace-mutation").await;
    let patch = ConnectSessionPatch {
        status: Some("awaiting_selection".into()),
        step: Some("callback".into()),
        ..Default::default()
    };
    let r = ok(repo::write(&db, move |tx| {
        cs::update_where_status_in(tx, id, B, &patch, &["awaiting_selection"], false, now_ms())
    })
    .await);
    assert!(r.is_none());
    assert!(
        append(
            &db,
            id,
            B,
            json!([{ "targetId": "t1", "status": "connected" }]),
            &["conn-cross-workspace"]
        )
        .await
        .is_none()
    );
    let s = ok(repo::read(&db, move |c| cs::find_by_id_for_workspace(c, id, A)).await)
        .expect("session");
    assert_eq!(
        (s.status.as_str(), s.step.as_str()),
        ("awaiting_selection", "select")
    );
    assert_eq!(s.results, Json(json!([])));
    assert_eq!(s.result_connection_ids, Json(json!([])));
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:819 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > does not update a session outside the allowed statuses"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_does_not_update_a_session_outside_the_allowed_statuses()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-guard-1").await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET status = 'completed', "consumedAt" = ?1 WHERE id = 1"#,
        vec![int(now_ms())],
    )
    .await;
    let patch = ConnectSessionPatch {
        status: Some("expired".into()),
        consumed_at: Some(Some(now_ms())),
        encrypted_auth: Some(None),
        ..Default::default()
    };
    let r = ok(repo::write(&db, move |tx| {
        cs::update_where_status_in(
            tx,
            id,
            A,
            &patch,
            &["pending", "authorized", "awaiting_selection"],
            false,
            now_ms(),
        )
    })
    .await);
    assert!(r.is_none());
    assert_eq!(
        find(&db, id).await.map(|s| s.status).as_deref(),
        Some("completed")
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:854 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > does not update an expired active session when expiration is required"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_does_not_update_an_expired_active_session_when_expiration_is_required()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-active-guard-1").await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET "expiresAt" = ?1 WHERE id = 1"#,
        vec![int(past())],
    )
    .await;
    let patch = ConnectSessionPatch {
        status: Some("awaiting_selection".into()),
        step: Some("select".into()),
        ..Default::default()
    };
    let r = ok(repo::write(&db, move |tx| {
        cs::update_where_status_in(
            tx,
            id,
            A,
            &patch,
            &["pending", "authorized", "awaiting_selection"],
            true,
            now_ms(),
        )
    })
    .await);
    assert!(r.is_none());
    assert_eq!(
        find(&db, id).await.map(|s| s.step).as_deref(),
        Some("select")
    );
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:886 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > enforces terminal cleanup and active reconnect target invariants"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_enforces_terminal_cleanup_and_active_reconnect_target_invariants()
 {
    let db = db().await;
    session(&db, 1, A, "iso-hash-invariants").await;
    let at = now_ms();
    for sql in [
        r#"UPDATE "ConnectSession" SET status = 'completed' WHERE id = 1"#.to_owned(),
        format!(
            r#"UPDATE "ConnectSession" SET status = 'completed', "consumedAt" = {at}, "encryptedAuth" = '{AUTH}' WHERE id = 1"#
        ),
        r#"UPDATE "ConnectSession" SET "errorCode" = 'provider_error' WHERE id = 1"#.to_owned(),
        format!(
            r#"UPDATE "ConnectSession" SET status = 'completed', "consumedAt" = {at}, "errorCode" = 'provider_error' WHERE id = 1"#
        ),
        r#"UPDATE "ConnectSession" SET purpose = 'reconnect' WHERE id = 1"#.to_owned(),
    ] {
        let err = db
            .write(move |tx| Ok(tx.execute(&sql, [])?))
            .await
            .expect_err("must fail");
        assert_eq!(err.sqlstate(), Some("23514"));
    }
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:954 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > releaseTarget lets a retry claim the same target again after a failed attempt (claim -> release -> claim)"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_release_target_lets_a_retry_claim_the_same_target_again_after_a_failed_attempt_claim_release_claim()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-release-1").await;
    assert!(claim(&db, id, A, "t1", "owner-a", future()).await);
    release(&db, id, A, "t1", "owner-a").await;
    assert!(claim(&db, id, A, "t1", "owner-b", future()).await);
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:1000 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > appendResults keeps retryable failures selectable until retried targets resolve"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_append_results_keeps_retryable_failures_selectable_until_retried_targets_resolve()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-retryable-1").await;
    exec(
        &db,
        r#"UPDATE "ConnectSession" SET "encryptedAuth" = ?1 WHERE id = 1"#,
        vec![repo_common::text(AUTH)],
    )
    .await;
    let after = append(&db, id, A, json!([{ "targetId": "t1", "status": "failed", "reason": "providerRejected" }, { "targetId": "t2", "status": "limitReached", "reason": "workspaceLimit" }]), &[]).await.expect("after");
    assert_eq!(
        (after.error_code.clone(), after.status.as_str()),
        (None, "awaiting_selection")
    );
    assert!(after.encrypted_auth.is_some());
    let done = append(&db, id, A, json!([{ "targetId": "t1", "status": "connected", "connectionId": "conn-1" }, { "targetId": "t2", "status": "duplicated", "reason": "alreadyConnected" }]), &["conn-1"]).await.expect("done");
    assert_eq!(done.status, "completed");
    assert_eq!(done.encrypted_auth, None);
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:1057 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > appendResults completes an all-duplicated selectable batch without connection ids"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_append_results_completes_an_all_duplicated_selectable_batch_without_connection_ids()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-all-duplicates-1").await;
    let s = append(&db, id, A, json!([{ "targetId": "t1", "status": "duplicated", "reason": "alreadyConnected" }, { "targetId": "t2", "status": "duplicated", "reason": "alreadyConnected" }]), &[]).await.expect("s");
    assert_eq!(s.status, "completed");
    assert_eq!(s.result_connection_ids, Json(json!([])));
}

async fn reconnect(
    db: &Db,
    id: i64,
    ws: i64,
    target: &'static str,
    conn: &'static str,
) -> Option<ConnectSession> {
    let result = json!({ "targetId": target, "status": "connected", "connectionId": conn });
    ok(repo::write(db, move |tx| {
        cs::complete_reconnect(tx, id, ws, &result, now_ms())
    })
    .await)
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:1091 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > completeReconnect only completes an unexpired authorized session in its workspace and clears encrypted auth"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_complete_reconnect_only_completes_an_unexpired_authorized_session_in_its_workspace_and_clears_encrypted_auth()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-reconnect").await;
    exec(&db, r#"UPDATE "ConnectSession" SET status = 'authorized', step = 'authorize', "encryptedAuth" = ?1 WHERE id = 1"#, vec![repo_common::text(AUTH)]).await;
    assert!(reconnect(&db, id, B, "t1", "connection-1").await.is_none());
    let done = reconnect(&db, id, A, "t1", "connection-1")
        .await
        .expect("done");
    assert_eq!(
        (done.status.as_str(), done.step.as_str()),
        ("completed", "done")
    );
    assert_eq!(done.encrypted_auth, None);
    assert_eq!(done.result_connection_ids, Json(json!(["connection-1"])));
    assert!(reconnect(&db, id, A, "t2", "connection-2").await.is_none());
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:1162 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > completeReconnect rejects an expired authorized session"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_complete_reconnect_rejects_an_expired_authorized_session()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-reconnect-expired").await;
    exec(&db, r#"UPDATE "ConnectSession" SET status = 'authorized', step = 'authorize', "expiresAt" = ?1 WHERE id = 1"#, vec![int(past())]).await;
    assert!(reconnect(&db, id, A, "t1", "connection-1").await.is_none());
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:1198 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > appendResults under REAL concurrency: two separate connections each completing a different target merge into one completed session with no lost update"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_append_results_under_real_concurrency_two_separate_connections_each_completing_a_different_target_merge_into_one_completed_session_with_no_lost_update()
 {
    let db = db().await;
    let id = session(&db, 1, A, "iso-hash-concurrent").await;
    let one = |target: &'static str, conn: &'static str| {
        let results =
            vec![json!({ "targetId": target, "status": "connected", "connectionId": conn })];
        let ids = vec![conn.to_owned()];
        move |tx: &mut chatbotx_db::WriteTx<'_>| {
            cs::append_results(tx, id, A, &results, &ids, now_ms())
        }
    };
    let (a, b) = tokio::join!(
        repo::write(&db, one("t1", "conn-t1")),
        repo::write(&db, one("t2", "conn-t2"))
    );
    ok(a);
    ok(b);
    let s = find(&db, id).await.expect("final");
    assert_eq!(s.status, "completed");
    let mut targets: Vec<String> = s
        .results
        .0
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["targetId"].as_str().unwrap().to_owned())
        .collect();
    targets.sort();
    assert_eq!(targets, vec!["t1", "t2"]);
    let mut ids: Vec<String> = s
        .result_connection_ids
        .0
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["conn-t1", "conn-t2"]);
}

/// Port of: packages/database/__tests__/integration/connect-session-isolation.test.ts:1306 "ConnectSession cross-workspace isolation and claimTarget atomicity against Postgres > retries a chunk-capped terminal purge and preserves recent sessions"
#[tokio::test]
async fn connect_session_cross_workspace_isolation_and_claim_target_atomicity_against_postgres_retries_a_chunk_capped_terminal_purge_and_preserves_recent_sessions()
 {
    let db = db().await;
    let old = now_ms() - 31 * 24 * 60 * 60 * 1000;
    repo_common::setup(&db, move |tx| {
        for (id, at) in [(1_i64, old), (2, old), (3, now_ms())] {
            ins!(tx, "ConnectSession", { "id" => id, "workspaceId" => A, "provider" => "messenger", "purpose" => "connect", "actorUserId" => OWNER, "stateNonceHash" => format!("iso-hash-purge-{id}"), "status" => "completed", "step" => "done", "targets" => "[]", "expiresAt" => future(), "consumedAt" => at })?;
        }
        Ok(())
    })
    .await;
    let purge = || cs::purge_old_terminal(&db, 30, 1, 0, 1, None);
    assert_eq!(ok(purge().await), (1, ChunkedPurgeStopReason::ChunkCap));
    let left = [find(&db, 1).await.is_some(), find(&db, 2).await.is_some()];
    assert_eq!(left.iter().filter(|x| **x).count(), 1);
    assert_eq!(ok(purge().await), (1, ChunkedPurgeStopReason::ChunkCap));
    assert!(find(&db, 1).await.is_none() && find(&db, 2).await.is_none());
    assert_eq!(
        ok(cs::purge_old_terminal(&db, 30, 1, 0, 10, None).await),
        (0, ChunkedPurgeStopReason::Drained)
    );
    assert!(find(&db, 3).await.is_some());
}
