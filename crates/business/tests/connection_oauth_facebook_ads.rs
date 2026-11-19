//! Q.F2-settings: the workspace Facebook Ads connect
//! (`connectFacebookAds` + the `flow: "facebookAds"` Messenger callback
//! branch), its disconnect and the settings row, over a migrated database
//! and a scripted Graph API (the originals mock the services).

mod common;

use std::sync::Mutex;

use chatbotx_business::connect_session;
use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::connection::oauth_facebook_ads::{
    Stored, complete, disconnect, settings_row, start,
};
use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_db::repo::partials::credential::platform_credential_aad;
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, WS, setup, two_workspaces};
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

const APP: &str = "https://app.test";
const USER: i64 = 1001;

fn encryption() -> Encryption {
    Encryption::new(Keys::new(&"a1".repeat(32)).expect("test key"))
}

/// `WS` with its owner as a member and the platform Messenger credential.
async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    let config = json!({ "clientId": "fb-app", "clientSecret": "fb-secret", "version": "v22.0" });
    let blob = encryption()
        .encrypt_object(&config, Some(&platform_credential_aad("messenger")))
        .expect("encrypt");
    let value = serde_json::to_string(&blob).expect("json");
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "PlatformCredential" ("id", "userId", "type", "publicConfig", "value") VALUES (5001, NULL, 'messenger', '{}', ?1)"#,
            params![value],
        )?;
        tx.execute(
            r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions") VALUES (5002, ?1, ?2, 'owner', '{"superAdmin":true}')"#,
            params![WS, USER],
        )?;
        Ok(())
    })
    .await;
    db
}

async fn session_of(db: &common::TestDb, url: &str) -> connect_session::SessionRow {
    let parsed = url::Url::parse(url).expect("url");
    let state = parsed
        .query_pairs()
        .find(|(k, _)| k == "state")
        .expect("state")
        .1
        .into_owned();
    let (_, nonce) = state.split_once('.').expect("signed state");
    connect_session::find_by_nonce(db, nonce)
        .await
        .expect("lookup")
        .expect("session")
}

fn grant() -> Script {
    Script::with(vec![
        json!({ "access_token": "short" }),
        json!({ "access_token": "long-token", "expires_in": 5_184_000 }),
    ])
}

async fn text(db: &common::TestDb, sql: &'static str) -> Option<String> {
    db.read(move |conn| Ok(conn.query_row(sql, [], |r| r.get::<_, Option<String>>(0))?))
        .await
        .expect("read")
}

/// Own: the dialog asks the Ads scopes of the Messenger app with a signed
/// state; the callback stores the long-lived token encrypted, attaches the
/// `Connection` row and completes the session; a second grant updates it.
#[tokio::test]
async fn connect_stores_the_ads_token_and_a_second_grant_updates_it() {
    let db = seeded().await;
    let url = start(&db, &encryption(), APP, WS, USER)
        .await
        .expect("start");
    assert!(
        url.starts_with("https://www.facebook.com/v22.0/dialog/oauth?"),
        "{url}"
    );
    assert!(url.contains("client_id=fb-app"));
    assert!(url.contains(
        "scope=ads_read%2Cads_management%2Cpages_manage_ads%2Cpages_read_engagement%2Cpages_show_list"
    ));
    assert!(
        url.contains("redirect_uri=https%3A%2F%2Fapp.test%2Fintegrations%2Fmessenger%2Fcallback")
    );
    let session = session_of(&db, &url).await;
    assert_eq!(session.provider, "facebookAds");
    assert_eq!(
        session.return_url.as_deref(),
        Some("/space/2001/settings/integrations")
    );

    let upstream = grant();
    let stored = complete(
        &db,
        &encryption(),
        &upstream,
        APP,
        &session,
        Some(USER),
        "the-code",
    )
    .await
    .expect("complete");
    assert_eq!(stored, Stored::Created);
    let seen = upstream.seen.lock().expect("lock").clone();
    assert!(seen[0].url.contains("code=the-code"));
    assert!(seen[1].url.contains("grant_type=fb_exchange_token"));
    assert!(seen[1].url.contains("fb_exchange_token=short"));
    let raw = text(&db, r#"SELECT "auth" FROM "IntegrationFacebookAds""#)
        .await
        .expect("auth");
    assert!(!raw.contains("long-token"), "stored encrypted");
    let auth = chatbotx_business::email_marketing::open_auth(&raw).expect("open");
    assert_eq!(auth["accessToken"], json!("long-token"));
    assert_eq!(auth["authType"], json!("custom"));
    assert_eq!(
        text(&db, r#"SELECT "status" || ':' || "sourceId" FROM "Connection" WHERE "provider" = 'facebookAds'"#).await,
        Some("connected:workspace".into())
    );
    let row = settings_row(&db, WS)
        .await
        .expect("row")
        .expect("connected");
    assert_eq!(row["status"], json!("active"));
    assert!(row["tokenExpiresAt"].is_string());
    assert!(row.get("auth").is_none());
    assert_eq!(settings_row(&db, OTHER_WS).await.expect("other"), None);
    // The used session cannot be replayed.
    assert!(
        complete(
            &db,
            &encryption(),
            &grant(),
            APP,
            &session,
            Some(USER),
            "again"
        )
        .await
        .is_err()
    );

    let url = start(&db, &encryption(), APP, WS, USER)
        .await
        .expect("restart");
    let session = session_of(&db, &url).await;
    let stored = complete(
        &db,
        &encryption(),
        &grant(),
        APP,
        &session,
        Some(USER),
        "code-2",
    )
    .await
    .expect("update");
    assert_eq!(stored, Stored::Updated);
    assert_eq!(
        text(
            &db,
            r#"SELECT CAST(COUNT(*) AS TEXT) FROM "IntegrationFacebookAds""#
        )
        .await,
        Some("1".into())
    );
}

/// Own: a callback for another user (or with no session) is a state
/// mismatch and calls nothing.
#[tokio::test]
async fn a_callback_for_another_user_is_refused() {
    let db = seeded().await;
    let url = start(&db, &encryption(), APP, WS, USER)
        .await
        .expect("start");
    let session = session_of(&db, &url).await;
    let upstream = grant();
    for user in [None, Some(1002)] {
        let error = complete(&db, &encryption(), &upstream, APP, &session, user, "code")
            .await
            .expect_err("mismatch");
        assert_eq!(error.code(), Some("connectionStateMismatch"));
    }
    assert!(upstream.seen.lock().expect("lock").is_empty());
}

/// Own (`disconnectFacebookAdsAction`): the token is revoked best effort,
/// then the row, its `Integration` and `Connection` go; nothing connected is
/// `false`.
#[tokio::test]
async fn disconnect_revokes_and_removes_the_integration() {
    let db = seeded().await;
    let url = start(&db, &encryption(), APP, WS, USER)
        .await
        .expect("start");
    let session = session_of(&db, &url).await;
    complete(
        &db,
        &encryption(),
        &grant(),
        APP,
        &session,
        Some(USER),
        "code",
    )
    .await
    .expect("connect");
    assert!(!disconnect(&db, &grant(), OTHER_WS).await.expect("other"));
    // The revoke fails: the disconnect still happens.
    let upstream = Script::default();
    assert!(disconnect(&db, &upstream, WS).await.expect("disconnect"));
    let seen = upstream.seen.lock().expect("lock").clone();
    assert_eq!(seen[0].method, "DELETE");
    assert!(
        seen[0]
            .url
            .starts_with("https://graph.facebook.com/v22.0/me/permissions?access_token=long-token")
    );
    for sql in [
        r#"SELECT CAST(COUNT(*) AS TEXT) FROM "IntegrationFacebookAds""#,
        r#"SELECT CAST(COUNT(*) AS TEXT) FROM "Integration" WHERE "integrationType" = 'facebookAds'"#,
        r#"SELECT CAST(COUNT(*) AS TEXT) FROM "Connection" WHERE "provider" = 'facebookAds'"#,
    ] {
        assert_eq!(text(&db, sql).await, Some("0".into()), "{sql}");
    }
}
