//! Q.I2: the TikTok / Zalo OAuth connect over the connect-session engine
//! (`integrations/{tiktok,zalo}/src/integration.ts` `connection` adapters)
//! and the TikTok `authorization.removed` quota release
//! (`connectionStateService.markUnhealthyByIdentifier` → `releaseQuotaEdge`),
//! over a migrated database and a scripted upstream.

mod common;

use std::sync::Mutex;

use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::connection::mark_unhealthy_by_identifier;
use chatbotx_business::connection::oauth::{self, OAuthCredential, StartSession};
use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_db::rusqlite::params;
use common::{WS, scalar_i64, setup, two_workspaces};
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

const TIKTOK_SCOPES: &str =
    "user.info.basic,user.info.username,message.list.read,message.list.send,message.list.manage";

#[tokio::test]
async fn tiktok_connects_through_a_session_and_a_revoke_releases_the_channel_quota() {
    let db = two_workspaces().await;
    let encryption = encryption();
    let (session_id, nonce) = start(&db, "tiktok").await;
    let upstream = Script::with(vec![
        json!({ "code": 0, "data": { "access_token": "at", "refresh_token": "rt", "expires_in": 86400,
            "refresh_token_expires_in": 864000, "open_id": "open-1", "scope": TIKTOK_SCOPES } }),
        json!({ "code": 0, "data": { "user": { "open_id": "open-1", "display_name": "Shop", "username": "shop" } } }),
    ]);
    let session = oauth::complete_authorization(
        &db,
        &encryption,
        &upstream,
        session_id,
        &nonce,
        "the-code",
        &credential("tiktok"),
    )
    .await
    .expect("complete");
    assert_eq!(session.status, "awaiting_selection");
    assert_eq!(
        session.targets,
        json!([{ "id": "open-1", "name": "Shop", "selectable": true }])
    );
    let seen = upstream.seen.lock().expect("lock").clone();
    assert_eq!(
        seen[0].url,
        "https://business-api.tiktok.com/open_api/v1.3/tt_user/oauth2/token/"
    );
    let body: Value = serde_json::from_str(seen[0].body.as_deref().expect("body")).expect("json");
    assert_eq!(body["auth_code"], "the-code");
    assert_eq!(
        body["redirect_uri"],
        "https://app.test/integrations/tiktok/callback"
    );
    assert!(
        seen[1]
            .headers
            .contains(&("Access-Token".into(), "at".into()))
    );
    // The grant is stored encrypted on the session, never as plain JSON.
    assert!(
        !session
            .encrypted_auth
            .as_deref()
            .expect("encrypted")
            .contains("\"at\"")
    );

    let result = oauth::connect_targets(
        &db,
        &encryption,
        &Script::default(),
        session_id,
        WS,
        &["open-1".into()],
        None,
    )
    .await
    .expect("connect");
    assert_eq!(result.outcomes[0]["status"], "connected");
    let connection = &result.connections[0];
    assert_eq!(
        (
            connection.provider.as_str(),
            connection.channel.as_deref(),
            connection.status.as_str()
        ),
        ("tiktok", Some("tiktok"), "connected")
    );
    let auth = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "auth" FROM "IntegrationTiktok" WHERE "openId" = 'open-1'"#,
                [],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .expect("satellite");
    let auth: Value = serde_json::from_str(&auth).expect("auth json");
    assert_eq!(auth["tokens"]["accessToken"], "at");
    assert_eq!(
        auth["metadata"]["features"],
        json!({ "messaging": true, "comments": true })
    );

    let inbox_id = connection.inbox_id.expect("inbox");
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "UserQuota" ("id", "userId", "channelsUsed") VALUES (1, 1001, 1)"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "WorkspaceUsage" ("id", "workspaceId", "channelsUsed") VALUES (1, ?1, 1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    assert!(
        mark_unhealthy_by_identifier(&db, "tiktok", "open-1", WS, inbox_id)
            .await
            .expect("revoke")
    );
    let used = |sql: &'static str, id: i64| scalar_i64(&db, sql, id);
    assert_eq!(
        used(
            r#"SELECT "channelsUsed" FROM "UserQuota" WHERE "userId" = ?1"#,
            1001
        )
        .await,
        Some(0)
    );
    assert_eq!(
        used(
            r#"SELECT "channelsUsed" FROM "WorkspaceUsage" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(0)
    );
    // A second removal of an already inactive connection releases nothing.
    mark_unhealthy_by_identifier(&db, "tiktok", "open-1", WS, inbox_id)
        .await
        .expect("revoke again");
    assert_eq!(
        used(
            r#"SELECT "channelsUsed" FROM "WorkspaceUsage" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(0)
    );
}

#[tokio::test]
async fn a_tiktok_grant_without_the_messaging_scopes_is_rejected() {
    let db = two_workspaces().await;
    let (session_id, nonce) = start(&db, "tiktok").await;
    let upstream = Script::with(vec![json!({ "code": 0, "data": {
        "access_token": "at", "open_id": "open-1", "scope": "user.info.basic,user.info.username" } })]);
    let error = oauth::complete_authorization(
        &db,
        &encryption(),
        &upstream,
        session_id,
        &nonce,
        "c",
        &credential("tiktok"),
    )
    .await
    .expect_err("missing scopes");
    assert_eq!(error.code(), Some("connectionCredentialsRejected"));
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "ConnectSession" WHERE "id" = ?1 AND "status" = 'failed'"#,
            session_id
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn zalo_connects_the_oa_of_the_grant() {
    let db = two_workspaces().await;
    let encryption = encryption();
    let (session_id, nonce) = start(&db, "zalo").await;
    let upstream = Script::with(vec![
        json!({ "access_token": "zat", "refresh_token": "zrt", "expires_in": "90000" }),
        json!({ "error": 0, "message": "Success", "data": { "oa_id": "4242", "name": "My OA" } }),
    ]);
    let session = oauth::complete_authorization(
        &db,
        &encryption,
        &upstream,
        session_id,
        &nonce,
        "zc",
        &credential("zalo"),
    )
    .await
    .expect("complete");
    assert_eq!(
        session.targets,
        json!([{ "id": "4242", "name": "My OA", "selectable": true }])
    );
    let seen = upstream.seen.lock().expect("lock").clone();
    assert_eq!(seen[0].url, "https://oauth.zaloapp.com/v4/oa/access_token");
    assert!(
        seen[0]
            .headers
            .contains(&("secret_key".into(), "cs".into()))
    );
    assert_eq!(
        seen[0].body.as_deref(),
        Some(
            "code=zc&app_id=ck&redirect_uri=https%3A%2F%2Fapp.test%2Fintegrations%2Fzalo%2Fcallback&grant_type=authorization_code"
        )
    );
    assert!(
        seen[1]
            .headers
            .contains(&("access_token".into(), "zat".into()))
    );
    let result = oauth::connect_targets(
        &db,
        &encryption,
        &Script::default(),
        session_id,
        WS,
        &["4242".into()],
        None,
    )
    .await
    .expect("connect");
    assert_eq!(result.outcomes[0]["status"], "connected");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "IntegrationZalo" WHERE "oaId" = '4242' AND "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(1)
    );
}

// ─── `updateTiktokSettingAction` (`apps/builder/__tests__/tiktok-settings-actions.test.ts`) ───

use chatbotx_business::connection::oauth_channels::{TiktokSettings, update_tiktok_settings};

const BROKER: &str = "https://broker.test";
const OWNER: i64 = 1001;

fn settings(user_id: Option<i64>, features: (bool, bool)) -> TiktokSettings {
    TiktokSettings {
        user_id,
        cloud: true,
        broker_origin: BROKER.into(),
        client_id: "id".into(),
        client_secret: "secret".into(),
        features,
    }
}

/// `(event_type, callback_url)` of each subscribe request, in order.
fn subscriptions(upstream: &Script) -> Vec<(String, String)> {
    upstream
        .seen
        .lock()
        .expect("lock")
        .iter()
        .map(|r| {
            assert_eq!(
                r.url,
                "https://business-api.tiktok.com/open_api/v1.3/business/webhook/update/"
            );
            let body: Value = serde_json::from_str(r.body.as_deref().expect("body")).expect("json");
            // Never narrowed to an `item_list` (a one-way door on TikTok's side).
            assert!(body.get("item_list").is_none());
            assert_eq!(
                (body["app_id"].as_str(), body["secret"].as_str()),
                (Some("id"), Some("secret"))
            );
            (
                body["event_type"].as_str().expect("event_type").to_string(),
                body["callback_url"]
                    .as_str()
                    .expect("callback_url")
                    .to_string(),
            )
        })
        .collect()
}

/// The stored `(id, decrypted config)` of the scope.
async fn stored_tiktok(db: &chatbotx_db::Db, user: Option<i64>) -> Option<(i64, Value)> {
    let encryption = encryption();
    chatbotx_business::tx::read(db, move |conn| {
        Ok(chatbotx_business::platform_credential::find_decrypted(
            conn,
            &encryption,
            user,
            "tiktok",
            Default::default(),
        )?
        .map(|c| (c.id, Value::Object(c.config))))
    })
    .await
    .expect("read")
}

fn ok_replies(n: usize) -> Script {
    Script::with(vec![json!({ "code": 0 }); n])
}

#[tokio::test]
async fn a_new_user_credential_subscribes_its_scoped_route_before_it_is_saved() {
    let db = two_workspaces().await;
    let upstream = ok_replies(2);
    let subscribed = update_tiktok_settings(
        &db,
        &encryption(),
        &upstream,
        settings(Some(OWNER), (true, true)),
    )
    .await
    .expect("save");
    assert_eq!(subscribed, (true, true));
    let (id, config) = stored_tiktok(&db, Some(OWNER)).await.expect("stored");
    let url = format!("{BROKER}/integrations/tiktok/webhook/c/{id}");
    assert_eq!(
        subscriptions(&upstream),
        vec![
            ("DIRECT_MESSAGE".into(), url.clone()),
            ("COMMENT".into(), url)
        ]
    );
    assert_eq!(
        config,
        json!({ "clientId": "id", "clientSecret": "secret", "features": { "messaging": true, "comments": true } })
    );
}

#[tokio::test]
async fn messaging_off_subscribes_and_stores_comments_only() {
    let db = two_workspaces().await;
    let upstream = ok_replies(1);
    update_tiktok_settings(
        &db,
        &encryption(),
        &upstream,
        settings(Some(OWNER), (false, true)),
    )
    .await
    .expect("save");
    let (id, config) = stored_tiktok(&db, Some(OWNER)).await.expect("stored");
    assert_eq!(
        subscriptions(&upstream),
        vec![(
            "COMMENT".into(),
            format!("{BROKER}/integrations/tiktok/webhook/c/{id}")
        )]
    );
    assert_eq!(
        config["features"],
        json!({ "messaging": false, "comments": true })
    );
}

#[tokio::test]
async fn an_existing_credential_keeps_its_id() {
    let db = two_workspaces().await;
    update_tiktok_settings(
        &db,
        &encryption(),
        &ok_replies(2),
        settings(Some(OWNER), (true, true)),
    )
    .await
    .expect("first save");
    let (first, _) = stored_tiktok(&db, Some(OWNER)).await.expect("stored");
    let upstream = ok_replies(2);
    update_tiktok_settings(
        &db,
        &encryption(),
        &upstream,
        settings(Some(OWNER), (true, true)),
    )
    .await
    .expect("second save");
    assert_eq!(
        stored_tiktok(&db, Some(OWNER)).await.expect("stored").0,
        first
    );
    assert_eq!(
        subscriptions(&upstream)[0].1,
        format!("{BROKER}/integrations/tiktok/webhook/c/{first}")
    );
}

#[tokio::test]
async fn an_active_tenant_subscribes_its_custom_domain_or_the_plain_broker_route() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Tenant" ("id", "ownerId", "status") VALUES (8, ?1, 'active')"#,
            params![OWNER],
        )?;
        Ok(())
    })
    .await;
    let upstream = ok_replies(2);
    update_tiktok_settings(
        &db,
        &encryption(),
        &upstream,
        settings(Some(OWNER), (true, true)),
    )
    .await
    .expect("save without domain");
    assert_eq!(
        subscriptions(&upstream)[0].1,
        format!("{BROKER}/integrations/tiktok/webhook")
    );

    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "CustomDomain" ("id", "tenantId", "domain", "status") VALUES (1, 8, 'chat.acme.com', 'active')"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let upstream = ok_replies(2);
    update_tiktok_settings(
        &db,
        &encryption(),
        &upstream,
        settings(Some(OWNER), (true, true)),
    )
    .await
    .expect("save with domain");
    assert_eq!(
        subscriptions(&upstream)[0].1,
        "https://chat.acme.com/integrations/tiktok/webhook"
    );
}

#[tokio::test]
async fn the_platform_scope_subscribes_the_broker_route() {
    let db = two_workspaces().await;
    let upstream = ok_replies(2);
    update_tiktok_settings(&db, &encryption(), &upstream, settings(None, (true, true)))
        .await
        .expect("save");
    assert_eq!(
        subscriptions(&upstream)[0].1,
        format!("{BROKER}/integrations/tiktok/webhook")
    );
    assert!(stored_tiktok(&db, None).await.is_some());
}

#[tokio::test]
async fn a_failed_message_subscription_saves_nothing() {
    let db = two_workspaces().await;
    let upstream = Script::with(vec![
        json!({ "code": 40001, "message": "subscription failed" }),
    ]);
    let error = update_tiktok_settings(
        &db,
        &encryption(),
        &upstream,
        settings(Some(OWNER), (true, true)),
    )
    .await
    .expect_err("rejected");
    assert_eq!(error.to_string(), "tiktokError: subscription failed");
    assert!(stored_tiktok(&db, Some(OWNER)).await.is_none());
}

#[tokio::test]
async fn a_failed_comment_subscription_still_saves_while_messaging_is_on() {
    let db = two_workspaces().await;
    let upstream = Script::with(vec![
        json!({ "code": 0 }),
        json!({ "code": 40001, "message": "bad event" }),
    ]);
    let subscribed = update_tiktok_settings(
        &db,
        &encryption(),
        &upstream,
        settings(Some(OWNER), (true, true)),
    )
    .await
    .expect("save");
    assert_eq!(subscribed, (true, false));
    assert!(stored_tiktok(&db, Some(OWNER)).await.is_some());
}

#[tokio::test]
async fn a_failed_comment_subscription_fails_the_save_with_messaging_off() {
    let db = two_workspaces().await;
    let upstream = Script::with(vec![json!({ "code": 40001, "message": "bad event" })]);
    update_tiktok_settings(
        &db,
        &encryption(),
        &upstream,
        settings(Some(OWNER), (false, true)),
    )
    .await
    .expect_err("rejected");
    assert!(stored_tiktok(&db, Some(OWNER)).await.is_none());
}

#[tokio::test]
async fn comments_off_subscribes_direct_messages_only() {
    let db = two_workspaces().await;
    let upstream = ok_replies(1);
    let subscribed = update_tiktok_settings(
        &db,
        &encryption(),
        &upstream,
        settings(Some(OWNER), (true, false)),
    )
    .await
    .expect("save");
    assert_eq!(subscribed, (true, false));
    let events: Vec<String> = subscriptions(&upstream)
        .into_iter()
        .map(|(e, _)| e)
        .collect();
    assert_eq!(events, vec!["DIRECT_MESSAGE".to_string()]);
}
