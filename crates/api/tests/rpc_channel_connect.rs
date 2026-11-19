//! The account pickers' connect procedures (`rpc/channel_connect.rs`,
//! `connectSessionCandidate` of `connect-session-candidate.ts`): the session
//! must be the caller's own `connect` session, awaiting selection, in a
//! workspace the caller is a member of; only a selectable target connects.

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_db::rusqlite::params;
use rpc_common::{
    AGENT_TOKEN, Fixture, NOW_MS, OUTSIDER, OUTSIDER_TOKEN, OWNER, OWNER_TOKEN, WS, call, fixture,
};
use serde_json::{Value, json};

const PATH: &str = "/rpc/integrationMessengerAPIs/connectMessengerPageAPI";

async fn seed(fx: &Fixture) {
    let targets = r#"[{"id":"p1","name":"P","selectable":true},{"id":"p2","name":"Q","selectable":false,"alreadyConnected":"other_workspace"}]"#;
    fx.db()
        .write(move |tx| {
            for (id, actor, owner) in [
                (501, OWNER, Some("1")),
                (502, OUTSIDER, Some("1")),
                (503, OWNER, None),
            ] {
                tx.execute(
                    r#"INSERT INTO "ConnectSession" ("id", "createdAt", "updatedAt", "workspaceId", "provider", "purpose",
                           "actorUserId", "platformOwnerId", "stateNonceHash", "status", "targets", "expiresAt")
                       VALUES (?1, ?2, ?2, ?3, 'messenger', 'connect', ?4, ?5, ?6, 'awaiting_selection', ?7, ?8)"#,
                    params![id, NOW_MS, WS, actor, owner, format!("nonce-{id}"), targets, NOW_MS * 2],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
}

async fn connect(fx: &Fixture, token: &str, session: &str, page: &str) -> Value {
    let (status, body) = call(
        fx,
        "POST",
        PATH,
        Some(token),
        json!({ "sessionId": session, "pageId": page }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["json"].clone()
}

#[tokio::test]
async fn only_the_session_owner_connects_a_selectable_target() {
    let fx = fixture().await;
    seed(&fx).await;

    // Another member, an unknown session, a session of a non-member.
    let other = connect(&fx, AGENT_TOKEN, "501", "p1").await;
    assert_eq!(
        other,
        json!({ "kind": "sessionError", "code": "sessionExpired" })
    );
    let unknown = connect(&fx, OWNER_TOKEN, "999", "p1").await;
    assert_eq!(unknown["code"], json!("sessionExpired"));
    let outsider = connect(&fx, OUTSIDER_TOKEN, "502", "p1").await;
    assert_eq!(outsider["code"], json!("notMember"));
    let missing = connect(&fx, OWNER_TOKEN, "503", "p1").await;
    assert_eq!(missing["code"], json!("credentialMissing"));

    // Targets the session did not offer as selectable.
    let duplicated = connect(&fx, OWNER_TOKEN, "501", "p2").await;
    assert_eq!(duplicated["kind"], json!("outcome"));
    assert_eq!(duplicated["outcome"]["name"], json!("Q"));
    assert_eq!(duplicated["outcome"]["status"], json!("duplicated"));
    assert_eq!(duplicated["outcome"]["reason"], json!("alreadyConnected"));
    let unlisted = connect(&fx, OWNER_TOKEN, "501", "p9").await;
    assert_eq!(unlisted["outcome"]["reason"], json!("notSelectable"));

    // Input validation.
    let (status, _) = call(
        &fx,
        "POST",
        PATH,
        Some(OWNER_TOKEN),
        json!({ "sessionId": "501" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
