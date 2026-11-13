//! Port of `packages/database/__tests__/whatsapp-signup-session-repository.test.ts` (P3.2b).
//! The TS mocks `and` / `eq` / `gt` / `arrayContains` / `not` / `sql` and checks the filter
//! parts; here each call runs on a migrated temp file (adapted): every filter part is a
//! session it spares. The two "concurrent" claims run as two write jobs (one writer).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo11.rs"]
mod repo11;

use chatbotx_db::Db;
use chatbotx_db::repo::Json;
use chatbotx_db::repo::whatsapp_signup_session::{self as wss, ClaimInput};
use repo11::{OTHER_WS, WS, exec, ms, put_signup_session, read, signup_session, two_ws_db, write};
use serde_json::json;

const USER: i64 = 1001;
const OWNER: i64 = 1002;

fn now() -> i64 {
    ms("2026-01-01T00:00:00.000Z")
}

fn later() -> i64 {
    now() + 600_000
}

fn claim(phone: &str) -> ClaimInput {
    ClaimInput {
        id: 1,
        user_id: USER,
        owner_id: OWNER,
        phone_number_id: phone.to_owned(),
        now: now(),
    }
}

async fn claim_once(
    db: &Db,
    input: ClaimInput,
) -> Option<chatbotx_db::repo::models::WhatsappSignupSession> {
    write(db, move |c| {
        wss::claim_signup_session_phone_number(c, &input)
    })
    .await
}

// ---------------------------------------------------------------------------
// claimSignupSessionPhoneNumber
// ---------------------------------------------------------------------------

#[tokio::test]
async fn claim_signup_session_phone_number_claims_a_candidate_phone_number() {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1", "pn-2"], &[], later()).await;
    let row = claim_once(&db, claim("pn-1")).await.expect("claimed");
    assert_eq!(row.id, 1);
    assert_eq!(row.claimed_phone_number_ids, Json(json!(["pn-1"])));
    assert_eq!(row.consumed_at, None);
}

#[tokio::test]
async fn claim_signup_session_phone_number_returns_null_when_the_phone_number_is_not_a_candidate() {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1"], &[], later()).await;
    assert!(claim_once(&db, claim("pn-not-candidate")).await.is_none());
    assert_eq!(
        signup_session(&db, 1).await.claimed_phone_number_ids,
        Json(json!([]))
    );
}

#[tokio::test]
async fn claim_signup_session_phone_number_returns_null_when_the_phone_number_was_already_claimed()
{
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1", "pn-2"], &["pn-1"], later()).await;
    assert!(claim_once(&db, claim("pn-1")).await.is_none());
    assert_eq!(
        signup_session(&db, 1).await.claimed_phone_number_ids,
        Json(json!(["pn-1"]))
    );
}

#[tokio::test]
async fn claim_signup_session_phone_number_returns_null_when_the_session_has_expired() {
    let db = two_ws_db().await;
    // "expiresAt" > now: a session expiring exactly at `now` is expired.
    put_signup_session(&db, 1, &["pn-1"], &[], now()).await;
    assert!(claim_once(&db, claim("pn-1")).await.is_none());
}

#[tokio::test]
async fn claim_signup_session_phone_number_returns_null_for_a_different_user_or_owner() {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1"], &[], later()).await;
    let wrong_user = ClaimInput {
        user_id: OWNER,
        ..claim("pn-1")
    };
    assert!(claim_once(&db, wrong_user).await.is_none());
    let wrong_owner = ClaimInput {
        owner_id: USER,
        ..claim("pn-1")
    };
    assert!(claim_once(&db, wrong_owner).await.is_none());
    assert_eq!(
        signup_session(&db, 1).await.claimed_phone_number_ids,
        Json(json!([]))
    );
}

#[tokio::test]
async fn claim_signup_session_phone_number_two_concurrent_claims_of_the_same_phone_number_build_the_identical_filter_set_expressions()
 {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1", "pn-2"], &[], later()).await;
    let (first, second) = tokio::join!(
        claim_once(&db, claim("pn-1")),
        claim_once(&db, claim("pn-1"))
    );
    let results = [first, second];
    assert_eq!(results.iter().filter(|r| r.is_some()).count(), 1);
    assert_eq!(results.iter().filter(|r| r.is_none()).count(), 1);
    assert_eq!(
        signup_session(&db, 1).await.claimed_phone_number_ids,
        Json(json!(["pn-1"]))
    );
}

#[tokio::test]
async fn claim_signup_session_phone_number_the_set_expression_appends_the_id_and_computes_consumed_at_via_a_cardinality_comparing_case_the_exact_result_only_resolves_against_real_row_data()
 {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1", "pn-2"], &["pn-1"], later()).await;
    let row = claim_once(&db, claim("pn-2")).await.expect("claimed");
    assert_eq!(row.claimed_phone_number_ids, Json(json!(["pn-1", "pn-2"])));
    // Every candidate is claimed: consumedAt is set.
    assert!(row.consumed_at.is_some());
}

#[tokio::test]
async fn claim_signup_session_phone_number_distinct_phone_number_ids_build_independent_per_id_filters_and_are_each_claimed()
 {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1", "pn-2", "pn-3"], &[], later()).await;
    let first = claim_once(&db, claim("pn-1")).await.expect("first");
    let second = claim_once(&db, claim("pn-2")).await.expect("second");
    assert_eq!(first.claimed_phone_number_ids, Json(json!(["pn-1"])));
    assert_eq!(
        second.claimed_phone_number_ids,
        Json(json!(["pn-1", "pn-2"]))
    );
    assert_eq!(second.consumed_at, None);
}

// ---------------------------------------------------------------------------
// bindSignupSessionWorkspace
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bind_signup_session_workspace_sets_the_workspace_id_when_it_is_null() {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1"], &[], later()).await;
    write(&db, |c| wss::bind_signup_session_workspace(c, 1, WS)).await;
    assert_eq!(signup_session(&db, 1).await.workspace_id, Some(WS));
}

#[tokio::test]
async fn bind_signup_session_workspace_second_call_is_a_no_op_the_where_clause_still_targets_an_unset_workspace_id()
 {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1"], &[], later()).await;
    exec(
        &db,
        r#"UPDATE "WhatsappSignupSession" SET "workspaceId" = 2001 WHERE "id" = 1"#,
    )
    .await;
    write(&db, |c| wss::bind_signup_session_workspace(c, 1, OTHER_WS)).await;
    assert_eq!(signup_session(&db, 1).await.workspace_id, Some(WS));
}

#[tokio::test]
async fn bind_signup_session_workspace_is_scoped_by_id() {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1"], &[], later()).await;
    put_signup_session(&db, 42, &["pn-1"], &[], later()).await;
    write(&db, |c| wss::bind_signup_session_workspace(c, 42, WS)).await;
    assert_eq!(signup_session(&db, 42).await.workspace_id, Some(WS));
    assert_eq!(signup_session(&db, 1).await.workspace_id, None);
}

// ---------------------------------------------------------------------------
// findActiveSignupSessionForUser
// ---------------------------------------------------------------------------

#[tokio::test]
async fn find_active_signup_session_for_user_is_scoped_by_id_and_user_id_only_never_owner_id() {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1"], &[], later()).await;
    let row = read(&db, |c| {
        wss::find_active_signup_session_for_user(c, 1, USER, now())
    })
    .await
    .expect("found");
    // The session's owner (1002) is not the user; ownerId is never part of the filter.
    assert_eq!((row.id, row.user_id, row.owner_id), (1, USER, OWNER));
    assert!(
        read(&db, |c| wss::find_active_signup_session_for_user(
            c,
            1,
            OWNER,
            now()
        ))
        .await
        .is_none()
    );
}

#[tokio::test]
async fn find_active_signup_session_for_user_returns_null_when_nothing_matches_expired_wrong_user_or_unknown_id()
 {
    let db = two_ws_db().await;
    put_signup_session(&db, 1, &["pn-1"], &[], now()).await;
    put_signup_session(&db, 2, &["pn-1"], &[], later()).await;
    for (id, user) in [(1, USER), (2, OWNER), (3, USER)] {
        assert!(
            read(&db, move |c| wss::find_active_signup_session_for_user(
                c,
                id,
                user,
                now()
            ))
            .await
            .is_none()
        );
    }
}
