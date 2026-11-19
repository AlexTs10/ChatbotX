//! Q.D12-channels: the Instagram login (`instagram`) and Instagram via
//! Facebook (`instagramFacebook`) OAuth connects over the connect-session
//! engine (`integrations/instagram{,-facebook}/src/integration.ts`
//! `connection` adapters), over a migrated database and a scripted upstream.

mod common;

use std::sync::Mutex;

use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::connection::oauth::{self, OAuthCredential, StartSession};
use chatbotx_core::encryption::{Encryption, Keys};
use common::{WS, two_workspaces};
use serde_json::{Value, json};

#[derive(Default)]
struct Script {
    replies: Mutex<Vec<UpstreamReply>>,
    seen: Mutex<Vec<UpstreamRequest>>,
}

impl Script {
    fn with(replies: Vec<Value>) -> Script {
        Script {
            replies: Mutex::new(
                replies
                    .into_iter()
                    .rev()
                    .map(|v| Ok((200, v.to_string())))
                    .collect(),
            ),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl Upstream for Script {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        self.seen.lock().expect("lock").push(request);
        self.replies
            .lock()
            .expect("lock")
            .pop()
            .unwrap_or_else(|| Err("no scripted reply".into()))
    }
}

fn encryption() -> Encryption {
    Encryption::new(Keys::new(&"a1".repeat(32)).expect("test key"))
}

fn credential(provider: &str) -> OAuthCredential {
    OAuthCredential {
        config: json!({ "clientId": "ck", "clientSecret": "cs" }),
        callback_url: format!("https://app.test/integrations/{provider}/callback"),
    }
}

/// Starts a session and returns `(sessionId, nonce)` from its authorize URL state.
async fn start(db: &chatbotx_db::Db, provider: &str) -> (i64, String) {
    let session = oauth::start_session(
        db,
        &credential(provider),
        StartSession {
            workspace_id: WS,
            provider: provider.into(),
            purpose: "connect",
            target_connection_id: None,
            platform_owner_id: Some(1001),
            origin_host: None,
            return_url: None,
            actor: None,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("start: {e}"));
    let url = url::Url::parse(
        session.next_action.as_ref().expect("action")["url"]
            .as_str()
            .expect("url"),
    )
    .expect("authorize url");
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .expect("state");
    let (id, nonce) = state.split_once('.').expect("{sessionId}.{nonce}");
    assert_eq!(id, session.id.to_string());
    (session.id, nonce.to_string())
}

async fn text(db: &chatbotx_db::Db, sql: &'static str) -> Vec<String> {
    db.read(move |conn| {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
    .expect("read")
}

#[tokio::test]
async fn instagram_via_facebook_lists_linked_accounts_and_connects_one() {
    let db = two_workspaces().await;
    let encryption = encryption();
    let (session_id, nonce) = start(&db, "instagramFacebook").await;
    let upstream = Script::with(vec![
        json!({ "access_token": "short" }),
        json!({ "access_token": "long" }),
        json!({ "data": [
            { "id": "page-1", "name": "Page", "access_token": "pt-1", "instagram_business_account": { "id": "ig-1" } },
            { "id": "page-2", "name": "No IG", "access_token": "pt-2" },
        ] }),
        json!({ "id": "ig-1", "name": "Shop", "username": "shop" }),
    ]);
    let session = oauth::complete_authorization(
        &db,
        &encryption,
        &upstream,
        session_id,
        &nonce,
        "the-code",
        &credential("instagram-facebook"),
    )
    .await
    .expect("complete");
    assert_eq!(session.status, "awaiting_selection");
    assert_eq!(
        session.targets,
        json!([{ "id": "ig-1", "name": "Shop", "selectable": true }])
    );
    let seen = upstream.seen.lock().expect("lock").clone();
    assert!(seen[1].url.contains("grant_type=fb_exchange_token"));
    assert!(
        seen[2]
            .url
            .starts_with("https://graph.facebook.com/v23.0/me/accounts?")
    );
    assert!(seen[3].url.contains("access_token=pt-1"));

    let subscribe = Script::with(vec![json!({ "success": true })]);
    let result = oauth::connect_targets(
        &db,
        &encryption,
        &subscribe,
        session_id,
        WS,
        &["ig-1".into()],
        None,
    )
    .await
    .expect("connect");
    assert_eq!(result.outcomes[0]["status"], "connected");
    assert_eq!(result.connections[0].status, "connected");
    let sub = subscribe.seen.lock().expect("lock").clone();
    assert_eq!(
        sub[0].url,
        "https://graph.facebook.com/v23.0/page-1/subscribed_apps"
    );
    assert!(
        sub[0]
            .headers
            .contains(&("authorization".into(), "Bearer pt-1".into()))
    );
    assert_eq!(
        text(&db, r#"SELECT "type" || ':' || "igId" || ':' || "pageId" || ':' || "username" FROM "IntegrationInstagram""#).await,
        ["facebook:ig-1:page-1:shop"]
    );
    assert_eq!(
        text(
            &db,
            r#"SELECT "channel" || ':' || "sourceId" FROM "Inbox" WHERE "channel" = 'instagram'"#
        )
        .await,
        ["instagram:ig-1"]
    );

    // The same account through Instagram login is greyed out.
    let (login_id, login_nonce) = start(&db, "instagram").await;
    let login = Script::with(vec![
        json!({ "access_token": "s", "user_id": 5 }),
        json!({ "access_token": "L" }),
        json!({ "id": "pg", "user_id": "ig-1", "username": "shop", "account_type": "BUSINESS" }),
    ]);
    let session = oauth::complete_authorization(
        &db,
        &encryption,
        &login,
        login_id,
        &login_nonce,
        "c",
        &credential("instagram"),
    )
    .await
    .expect("complete");
    assert_eq!(
        session.targets,
        json!([{ "id": "ig-1", "name": "shop", "selectable": false, "alreadyConnected": "this_workspace" }])
    );
}

#[tokio::test]
async fn instagram_login_needs_a_business_account_and_a_failed_subscribe_degrades() {
    let db = two_workspaces().await;
    let encryption = encryption();
    let (session_id, nonce) = start(&db, "instagram").await;
    let personal = Script::with(vec![
        json!({ "access_token": "s", "user_id": 5 }),
        json!({ "access_token": "L" }),
        json!({ "id": "pg", "user_id": "ig-9", "username": "me", "account_type": "PERSONAL" }),
    ]);
    let refused = oauth::complete_authorization(
        &db,
        &encryption,
        &personal,
        session_id,
        &nonce,
        "c",
        &credential("instagram"),
    )
    .await;
    assert!(refused.is_err());
    let seen = personal.seen.lock().expect("lock").clone();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, "https://api.instagram.com/oauth/access_token");
    assert!(
        seen[0]
            .body
            .as_deref()
            .unwrap()
            .contains("grant_type=authorization_code")
    );
    assert!(seen[1].url.contains("grant_type=ig_exchange_token"));

    let (session_id, nonce) = start(&db, "instagram").await;
    let business = Script::with(vec![
        json!({ "access_token": "s", "user_id": 5 }),
        json!({ "access_token": "L" }),
        json!({ "id": "pg", "user_id": 99, "username": "biz", "name": "Biz", "account_type": "CREATOR" }),
    ]);
    let session = oauth::complete_authorization(
        &db,
        &encryption,
        &business,
        session_id,
        &nonce,
        "c",
        &credential("instagram"),
    )
    .await
    .expect("complete");
    assert_eq!(
        session.targets,
        json!([{ "id": "99", "name": "Biz", "selectable": true }])
    );
    let subscribe = Script::with(vec![json!({ "success": false })]);
    let result = oauth::connect_targets(
        &db,
        &encryption,
        &subscribe,
        session_id,
        WS,
        &["99".into()],
        None,
    )
    .await
    .expect("connect");
    assert_eq!(result.connections[0].status, "degraded");
    let sub = subscribe.seen.lock().expect("lock").clone();
    assert_eq!(
        sub[0].url,
        "https://graph.instagram.com/v22.0/pg/subscribed_apps"
    );
    assert_eq!(
        text(
            &db,
            r#"SELECT "type" || ':' || "pageId" || ':' || "name" FROM "IntegrationInstagram""#
        )
        .await,
        ["instagram:pg:Biz"]
    );
}
