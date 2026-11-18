//! Q.D7: the Google Ads OAuth connect over the connect-session engine
//! (`integrations/google-ads/src/integration.ts` `connection` adapter): the
//! code exchange, the token-info identity, the customer candidates, the
//! `IntegrationGoogleAds` store and the in-place replacement of an inactive
//! account, over a migrated database and a scripted upstream.

mod common;

use std::sync::Mutex;

use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::connection::oauth::{self, OAuthCredential, StartSession};
use chatbotx_business::email_marketing::open_auth;
use chatbotx_business::google_ads_api::{ADS_SCOPE, DATA_MANAGER_SCOPE};
use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_db::rusqlite::params;
use common::{WS, setup, two_workspaces};
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

fn credential() -> OAuthCredential {
    OAuthCredential {
        config: json!({ "clientId": "ck", "clientSecret": "cs", "developerToken": "dev-token" }),
        callback_url: "https://app.test/integrations/google-ads/callback".into(),
    }
}

async fn start(db: &chatbotx_db::Db) -> (i64, String) {
    let session = oauth::start_session(
        db,
        &credential(),
        StartSession {
            workspace_id: WS,
            provider: "googleAds".into(),
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
    assert_eq!(url.host_str(), Some("accounts.google.com"));
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .expect("state");
    let (_, nonce) = state.split_once('.').expect("{sessionId}.{nonce}");
    (session.id, nonce.to_string())
}

fn grant(customer: &str, name: &str) -> Script {
    Script::with(vec![
        json!({ "access_token": "at-1", "refresh_token": "rt-1", "expires_in": 3600,
            "scope": format!("{ADS_SCOPE} {DATA_MANAGER_SCOPE} openid email") }),
        json!({ "sub": "google-user-1", "email": "owner@example.com" }),
        json!({ "resourceNames": [format!("customers/{customer}")] }),
        json!([{ "results": [{ "customer": { "id": customer, "descriptiveName": name,
            "manager": false, "status": "ENABLED", "currencyCode": "EUR" } }] }]),
    ])
}

async fn connect(db: &chatbotx_db::Db, customer: &str, name: &str) -> Value {
    let encryption = encryption();
    let (session_id, nonce) = start(db).await;
    let upstream = grant(customer, name);
    let session = oauth::complete_authorization(
        db,
        &encryption,
        &upstream,
        session_id,
        &nonce,
        "the-code",
        &credential(),
    )
    .await
    .expect("complete");
    assert_eq!(session.status, "awaiting_selection");
    let seen = upstream.seen.lock().expect("lock").clone();
    assert_eq!(seen[0].url, "https://oauth2.googleapis.com/token");
    let body = seen[0].body.as_deref().expect("form");
    assert!(body.contains("grant_type=authorization_code") && body.contains("code=the-code"));
    assert!(seen[2].url.ends_with("customers:listAccessibleCustomers"));
    assert!(
        seen[2]
            .headers
            .contains(&("developer-token".into(), "dev-token".into()))
    );
    let result = oauth::connect_targets(
        db,
        &encryption,
        &Script::default(),
        session_id,
        WS,
        &[customer.to_string()],
        Some(1001),
    )
    .await
    .expect("connect");
    assert_eq!(
        result.outcomes[0]["status"], "connected",
        "{:?}",
        result.outcomes
    );
    session.targets
}

async fn satellite(db: &chatbotx_db::Db) -> (i64, String, Option<String>, String) {
    db.read(|conn| {
        Ok(conn.query_row(
            r#"SELECT "integrationId", "customerId", "conversionActions", "auth"
               FROM "IntegrationGoogleAds" WHERE "workspaceId" = ?1"#,
            params![WS],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?)
    })
    .await
    .expect("satellite")
}

#[tokio::test]
async fn google_ads_connects_a_customer_and_replaces_an_inactive_one() {
    let db = two_workspaces().await;
    let targets = connect(&db, "1234567890", "Shop").await;
    assert_eq!(
        targets,
        json!([{ "id": "1234567890", "name": "Shop (123-456-7890)", "selectable": true }])
    );
    let (integration_id, customer, _, auth) = satellite(&db).await;
    assert_eq!(customer, "1234567890");
    // Stored sealed, and the developer token never reaches the row.
    assert!(!auth.contains("at-1"));
    let opened = open_auth(&auth).expect("sealed auth opens");
    assert_eq!(opened["tokens"]["refreshToken"], "rt-1");
    assert_eq!(opened["metadata"]["uploadMethod"], "dataManager");
    assert_eq!(opened["metadata"]["accountId"], "google-user-1");
    assert!(opened["metadata"].get("developerToken").is_none());
    let connection: (String, String, i64) = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "status", "kind", "integrationId" FROM "Connection"
                   WHERE "workspaceId" = ?1 AND "provider" = 'googleAds'"#,
                params![WS],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .expect("connection");
    assert_eq!(
        connection,
        ("connected".into(), "integration".into(), integration_id)
    );

    // A second account while the first is active: refused.
    let encryption = encryption();
    let (session_id, nonce) = start(&db).await;
    oauth::complete_authorization(
        &db,
        &encryption,
        &grant("2222222222", "Other"),
        session_id,
        &nonce,
        "c2",
        &credential(),
    )
    .await
    .expect("complete");
    let refused = oauth::connect_targets(
        &db,
        &encryption,
        &Script::default(),
        session_id,
        WS,
        &["2222222222".into()],
        Some(1001),
    )
    .await
    .expect("outcomes");
    assert_eq!(refused.outcomes[0]["status"], "duplicated");

    // After a disconnect, another customer takes the same integration and
    // starts with an empty setup.
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Connection" SET "status" = 'disconnected', "statusReason" = 'manual',
                   "disconnectedAt" = 1 WHERE "provider" = 'googleAds'"#,
            [],
        )?;
        tx.execute(
            r#"UPDATE "IntegrationGoogleAds" SET "conversionActions" = '[{"id":"1"}]', "conversionCustomerId" = '1234567890'"#,
            [],
        )?;
        Ok(())
    })
    .await;
    connect(&db, "3333333333", "Third").await;
    let (same_integration, customer, actions, _) = satellite(&db).await;
    assert_eq!(same_integration, integration_id);
    assert_eq!(customer, "3333333333");
    assert_eq!(actions, None);
    let rows: i64 = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT COUNT(*) FROM "Connection" WHERE "provider" = 'googleAds' AND "status" = 'connected'"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("count");
    assert_eq!(rows, 1);
}
