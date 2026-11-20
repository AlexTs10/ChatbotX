//! Q.P6-U2: the Meta Catalog OAuth connect (`connectMetaCatalogAction` +
//! the `flow: "metaCatalog"` branch of the Messenger callback,
//! `storeMetaCatalogConnection` → `integrationMetaCatalogService.upsert`)
//! over a migrated database and a scripted Graph API. The original state is
//! unsigned base64 JSON; here it is a signed connect session (as the other
//! Meta ads flows).

mod common;

use std::sync::Mutex;

use chatbotx_business::ads::connect::{Caller, Flow, complete, flow_of, start};
use chatbotx_business::connect_session;
use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::meta_catalog as mc;
use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_db::repo::partials::credential::platform_credential_aad;
use chatbotx_db::rusqlite::params;
use common::{WS, setup, two_workspaces};
use serde_json::{Value, json};

#[derive(Default)]
struct Script {
    replies: Mutex<Vec<UpstreamReply>>,
    seen: Mutex<Vec<UpstreamRequest>>,
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

fn grant(token: &str) -> Script {
    let replies = [
        json!({ "access_token": "short" }),
        json!({ "access_token": token, "expires_in": 5_184_000 }),
    ];
    Script {
        replies: Mutex::new(
            replies
                .into_iter()
                .rev()
                .map(|v: Value| Ok((200, v.to_string())))
                .collect(),
        ),
        seen: Mutex::new(Vec::new()),
    }
}

const APP: &str = "https://app.test";
const USER: i64 = 1001;

fn encryption() -> Encryption {
    Encryption::new(Keys::new(&"a1".repeat(32)).expect("test key"))
}

async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    let config = json!({ "clientId": "fb-app", "clientSecret": "fb-secret" });
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
            r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions") VALUES (5002, ?1, ?2, 'agent', '{}')"#,
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

fn member(user_id: i64) -> Caller {
    Caller {
        user_id: Some(user_id),
        is_member: true,
        super_admin: false,
    }
}

async fn connection(db: &common::TestDb) -> (i64, String, Option<i64>, String) {
    db.read(|conn| {
        Ok(conn.query_row(
            r#"SELECT "id", "status", "deletedAt", "importStatus" FROM "IntegrationMetaCatalog" WHERE "workspaceId" = 2001"#,
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?)
    })
    .await
    .expect("connection")
}

/// The dialog asks `catalog_management` again (`auth_type=rerequest`) with a
/// signed state; the callback stores the long-lived token encrypted on a new
/// connection, which `resolveAuth` reads back.
#[tokio::test]
async fn connect_stores_the_catalog_token_on_a_new_connection() {
    let db = seeded().await;
    let url = start(
        &db,
        &encryption(),
        APP,
        WS,
        USER,
        Flow::MetaCatalog,
        format!("/space/{WS}/products"),
    )
    .await
    .expect("start");
    assert!(
        url.starts_with("https://www.facebook.com/v24.0/dialog/oauth?"),
        "{url}"
    );
    assert!(url.contains("auth_type=rerequest") && url.contains("scope=catalog_management"));
    let session = session_of(&db, &url).await;
    assert_eq!(flow_of(&session), Some(Flow::MetaCatalog));
    assert_eq!(session.return_url.as_deref(), Some("/space/2001/products"));

    let upstream = grant("catalog-token");
    let done = complete(
        &db,
        &encryption(),
        APP,
        &upstream,
        &session,
        member(USER),
        "code-1",
    )
    .await
    .expect("complete");
    assert!(done);
    let (id, status, deleted_at, _) = connection(&db).await;
    assert_eq!((status.as_str(), deleted_at), ("active", None));
    let auth = mc::resolve_auth(&db, mc::process_encryption(), id)
        .await
        .expect("auth");
    assert_eq!(auth.access_token, "catalog-token");
    let raw: String = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "encryptedAuth" FROM "IntegrationMetaCatalog" WHERE "id" = ?1"#,
                [id],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("raw");
    assert!(
        !raw.contains("catalog-token"),
        "the token is stored encrypted"
    );
    let after = connect_session::find_by_nonce(&db, &session_nonce(&url))
        .await
        .expect("lookup")
        .expect("session");
    assert_eq!(after.status, "completed");
}

fn session_nonce(url: &str) -> String {
    let parsed = url::Url::parse(url).expect("url");
    let state = parsed
        .query_pairs()
        .find(|(k, _)| k == "state")
        .expect("state")
        .1
        .into_owned();
    state.split_once('.').expect("signed").1.to_string()
}

/// A reconnect after a disconnect revives the soft-deleted connection with
/// the new token and an idle import state.
#[tokio::test]
async fn a_reconnect_revives_the_soft_deleted_connection() {
    let db = seeded().await;
    for (round, token) in ["first-token", "second-token"].into_iter().enumerate() {
        let url = start(
            &db,
            &encryption(),
            APP,
            WS,
            USER,
            Flow::MetaCatalog,
            format!("/space/{WS}/products"),
        )
        .await
        .expect("start");
        let session = session_of(&db, &url).await;
        let upstream = grant(token);
        assert!(
            complete(
                &db,
                &encryption(),
                APP,
                &upstream,
                &session,
                member(USER),
                "code"
            )
            .await
            .expect("complete")
        );
        if round == 0 {
            mc::disconnect(&db, WS).await.expect("disconnect");
            let (_, status, deleted_at, import) = connection(&db).await;
            assert_eq!(status, "invalid");
            assert!(deleted_at.is_some());
            assert_eq!(import, "failed");
        }
    }
    let (id, status, deleted_at, import) = connection(&db).await;
    assert_eq!(
        (status.as_str(), deleted_at, import.as_str()),
        ("active", None, "idle")
    );
    let auth = mc::resolve_auth(&db, mc::process_encryption(), id)
        .await
        .expect("auth");
    assert_eq!(auth.access_token, "second-token");
    let count: i64 = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT count(*) FROM "Integration" WHERE "integrationType" = 'metaCatalog'"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("count");
    assert_eq!(count, 1);
}

/// The callback of another user (a forwarded link) stores nothing.
#[tokio::test]
async fn another_users_callback_stores_nothing() {
    let db = seeded().await;
    let url = start(
        &db,
        &encryption(),
        APP,
        WS,
        USER,
        Flow::MetaCatalog,
        format!("/space/{WS}/products"),
    )
    .await
    .expect("start");
    let session = session_of(&db, &url).await;
    let upstream = grant("stolen");
    let done = complete(
        &db,
        &encryption(),
        APP,
        &upstream,
        &session,
        member(9999),
        "code",
    )
    .await
    .expect("complete");
    assert!(!done);
    assert!(upstream.seen.lock().unwrap().is_empty());
    let count: i64 = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT count(*) FROM "IntegrationMetaCatalog""#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("count");
    assert_eq!(count, 0);
}

/// No Messenger app credential: the catalog error, no session.
#[tokio::test]
async fn no_app_credential_is_the_catalog_settings_error() {
    let db = two_workspaces().await;
    let error = start(
        &db,
        &encryption(),
        APP,
        WS,
        USER,
        Flow::MetaCatalog,
        format!("/space/{WS}/products"),
    )
    .await
    .expect_err("no credential");
    assert_eq!(error.message(), "Meta app settings are not configured.");
}
