//! Port of `apps/builder/__tests__/leaderboards-actions.test.ts`. Adapted:
//! the mocked `leaderboardService` becomes the real `/rpc/actions/<name>`
//! call over a migrated database; "the service was called with" becomes the
//! stored `Leaderboard` / `LeaderboardParticipant` rows.

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_core::js::now_ms;
use chatbotx_db::repo::iso;
use chatbotx_db::rusqlite::params;
use rpc_common::{OUTSIDER_TOKEN, OWNER_TOKEN, WS, action, fixture, scalar};
use serde_json::{Value, json};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const INBOX: i64 = 11;
const CONTACT: i64 = 31;

async fn seeded() -> rpc_common::Fixture {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, 'Page', 'messenger', 'src-11')"#,
                params![INBOX, WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "createdAt", "updatedAt") VALUES (?1, ?2, ?3, ?3)"#,
                params![CONTACT, WS, now_ms()],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    fx
}

fn config(ends_in_ms: i64) -> Value {
    let now = now_ms();
    json!({
        "name": "October top fans",
        "platform": "messenger",
        "inboxId": INBOX.to_string(),
        "startsAt": iso(now - DAY_MS),
        "endsAt": iso(now + ends_in_ms),
        "rules": { "comments": { "enabled": true, "points": 30 } },
    })
}

async fn act(
    fx: &rpc_common::Fixture,
    token: &str,
    name: &str,
    args: Value,
) -> (StatusCode, Value) {
    let (status, body) = action(
        fx,
        "actions",
        name,
        token,
        json!({ "workspaceId": WS.to_string(), "args": args }),
    )
    .await;
    (status, body["json"].clone())
}

async fn status_of(fx: &rpc_common::Fixture, id: &Value) -> Value {
    let id: i64 = id.as_str().expect("id").parse().expect("number");
    scalar(
        fx,
        r#"SELECT "status" FROM "Leaderboard" WHERE "id" = ?1"#,
        id,
    )
    .await
}

#[tokio::test]
async fn creates_a_draft_with_the_view_url() {
    let fx = seeded().await;
    let (status, out) = act(
        &fx,
        OWNER_TOKEN,
        "createLeaderboardAction",
        json!([{}, config(30 * DAY_MS)]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["published"], false);
    assert!(out["publishError"].is_null());
    assert_eq!(status_of(&fx, &out["id"]).await, json!("draft"));
    let (_, found) = act(
        &fx,
        OWNER_TOKEN,
        "findLeaderboard",
        json!([{ "id": out["id"] }]),
    )
    .await;
    let view = found["viewUrl"].as_str().expect("viewUrl");
    assert!(
        view.contains(&format!("leaderboardId={}", out["id"].as_str().unwrap())),
        "{view}"
    );
}

#[tokio::test]
async fn publishes_right_after_creating_when_publish_is_set() {
    let fx = seeded().await;
    let mut body = config(30 * DAY_MS);
    body["publish"] = json!(true);
    let (status, out) = act(
        &fx,
        OWNER_TOKEN,
        "createLeaderboardAction",
        json!([{}, body]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["published"], true);
    assert_eq!(status_of(&fx, &out["id"]).await, json!("active"));
}

#[tokio::test]
async fn keeps_the_draft_and_reports_the_publish_error() {
    let fx = seeded().await;
    // Ends in one second: valid to save, then already over when publish runs.
    let mut body = config(1);
    body["publish"] = json!(true);
    std::thread::sleep(std::time::Duration::from_millis(5));
    let (status, out) = act(
        &fx,
        OWNER_TOKEN,
        "createLeaderboardAction",
        json!([{}, body]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["published"], false);
    assert!(
        out["publishError"].as_str().is_some_and(|m| !m.is_empty()),
        "{out}"
    );
    assert_eq!(status_of(&fx, &out["id"]).await, json!("draft"));
}

#[tokio::test]
async fn adjust_passes_the_delta_reason_and_acting_user() {
    let fx = seeded().await;
    let mut body = config(30 * DAY_MS);
    body["publish"] = json!(true);
    let (_, created) = act(
        &fx,
        OWNER_TOKEN,
        "createLeaderboardAction",
        json!([{}, body]),
    )
    .await;
    let id = created["id"].clone();
    let (status, out) = act(
        &fx,
        OWNER_TOKEN,
        "adjustParticipantScoreAction",
        json!([{}, { "id": id, "contactId": CONTACT.to_string(), "delta": 50, "reason": "Live quiz winner" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["score"], 50);
    let actor = scalar(
        &fx,
        r#"SELECT "actorUserId" FROM "LeaderboardAuditLog" WHERE "contactId" = ?1"#,
        CONTACT,
    )
    .await;
    assert_eq!(actor, json!(rpc_common::OWNER));
}

#[tokio::test]
async fn a_non_member_cannot_list_leaderboards() {
    let fx = seeded().await;
    let (status, _) = act(&fx, OUTSIDER_TOKEN, "listLeaderboards", json!([{}])).await;
    assert_ne!(status, StatusCode::OK);
}
