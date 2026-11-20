//! Q.P6-U2: the Google Calendar / Google Sheets `connection` adapters over
//! the connect-session engine. Ports
//! `integrations/google-calendar/__tests__/connection-oauth.test.ts` and
//! `integrations/google-sheets/__tests__/connection-oauth.test.ts`
//! (`authorizeUrl`, `exchangeCode`, `describe`; the mocked Google client
//! becomes a scripted upstream). `refreshAuth` / `verify` / the legacy
//! callback live in other modules and are not part of this file.

mod common;

use std::sync::Mutex;

use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::connection::oauth::{self, OAuthCredential, StartSession};
use chatbotx_business::connection::oauth_google;
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

fn credential(slug: &str) -> OAuthCredential {
    OAuthCredential {
        config: json!({ "clientId": "client-1", "clientSecret": "secret-1" }),
        callback_url: format!("https://app.example.test/integrations/{slug}/callback"),
    }
}

fn query(url: &str) -> std::collections::HashMap<String, String> {
    url::Url::parse(url)
        .expect("url")
        .query_pairs()
        .into_owned()
        .collect()
}

async fn start(db: &chatbotx_db::Db, provider: &str, slug: &str) -> (i64, String, String) {
    let session = oauth::start_session(
        db,
        &credential(slug),
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
    let url = session.next_action.as_ref().expect("action")["url"]
        .as_str()
        .expect("url")
        .to_string();
    let state = query(&url)["state"].clone();
    let (_, nonce) = state.split_once('.').expect("{sessionId}.{nonce}");
    (session.id, nonce.to_string(), url)
}

fn tokens(scope: &str) -> Value {
    json!({ "access_token": "access-token", "expires_in": 3600,
        "refresh_token": "refresh-token", "scope": scope })
}

/// "passes state through verbatim, not JSON/base64-wrapped" (both).
#[test]
fn google_connection_authorize_url_passes_state_through_verbatim() {
    for (provider, scope) in [
        (
            "googleCalendar",
            "https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/calendar.events",
        ),
        (
            "googleSheets",
            "https://www.googleapis.com/auth/spreadsheets openid email",
        ),
    ] {
        let url = oauth_google::authorize_url(
            provider,
            &json!({ "clientId": "client-1" }),
            "https://app.example.test/connections/callback",
            "session-1.abc123",
        );
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        let q = query(&url);
        assert_eq!(q["state"], "session-1.abc123");
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["prompt"], "consent");
        assert_eq!(q["scope"], scope);
        assert_eq!(q["client_id"], "client-1");
        assert_eq!(
            q["redirect_uri"],
            "https://app.example.test/connections/callback"
        );
    }
}

/// "returns the calendar-verified OAuth auth value", then the connect stores
/// the calendar connection.
#[tokio::test]
async fn google_calendar_exchange_returns_the_calendar_verified_auth_and_connects() {
    let auth = oauth_google::exchange_code(
        &Script::with(vec![
            tokens("https://www.googleapis.com/auth/calendar.events"),
            json!({ "id": "calendar@example.test", "summary": "Main" }),
        ]),
        "googleCalendar",
        &json!({ "clientId": "client-1", "clientSecret": "secret-1" }),
        "https://app.example.test/connections/callback",
        "auth-code",
    )
    .await
    .expect("exchange");
    assert_eq!(auth["authType"], "oauth2");
    assert_eq!(
        auth["redirectUrl"],
        "https://app.example.test/connections/callback"
    );
    assert_eq!(auth["tokens"]["accessToken"], "access-token");
    assert_eq!(auth["tokens"]["refreshToken"], "refresh-token");
    assert!(auth["tokens"]["expiresAt"].is_string());
    assert_eq!(
        auth["metadata"],
        json!({
            "scope": "https://www.googleapis.com/auth/calendar.events",
            "providerCalendarId": "calendar@example.test",
            "email": "calendar@example.test",
        })
    );

    let db = two_workspaces().await;
    let encryption = encryption();
    let (session_id, nonce, _) = start(&db, "googleCalendar", "google-calendar").await;
    let upstream = Script::with(vec![
        tokens("https://www.googleapis.com/auth/calendar.events"),
        json!({ "id": "calendar@example.test" }),
    ]);
    let session = oauth::complete_authorization(
        &db,
        &encryption,
        &upstream,
        session_id,
        &nonce,
        "auth-code",
        &credential("google-calendar"),
    )
    .await
    .expect("complete");
    let seen = upstream.seen.lock().unwrap().clone();
    assert_eq!(seen[0].url, "https://oauth2.googleapis.com/token");
    assert!(seen[0].body.as_deref().unwrap().contains("code=auth-code"));
    assert_eq!(
        seen[1].url,
        "https://www.googleapis.com/calendar/v3/users/me/calendarList/primary"
    );
    assert!(
        seen[1]
            .headers
            .contains(&("authorization".into(), "Bearer access-token".into()))
    );
    assert_eq!(session.status, "awaiting_selection");
    assert_eq!(
        session.targets,
        json!([{ "id": "calendar@example.test", "name": "calendar@example.test", "selectable": true }])
    );
    let result = oauth::connect_targets(
        &db,
        &encryption,
        &Script::default(),
        session_id,
        WS,
        &["calendar@example.test".into()],
        None,
    )
    .await
    .expect("connect");
    assert_eq!(result.outcomes[0]["status"], "connected");
    let connection = &result.connections[0];
    assert_eq!(
        (connection.provider.as_str(), connection.kind.as_str()),
        ("googleCalendar", "integration")
    );
    let (calendar, raw): (String, String) = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "providerCalendarId", "auth" FROM "IntegrationGoogleCalendar" WHERE "workspaceId" = 2001"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .expect("row");
    assert_eq!(calendar, "calendar@example.test");
    assert!(!raw.contains("refresh-token"), "the auth is sealed");
}

/// "exchanges the code and returns the complete Sheets auth", then a
/// second grant of the same account is a duplicate.
#[tokio::test]
async fn google_sheets_exchange_returns_the_complete_auth_and_connects_once() {
    let auth = oauth_google::exchange_code(
        &Script::with(vec![
            tokens("https://www.googleapis.com/auth/spreadsheets"),
            json!({ "sub": "google-account-1", "email": "owner@example.test" }),
        ]),
        "googleSheets",
        &json!({ "clientId": "client-1", "clientSecret": "secret-1" }),
        "https://app.example.test/integrations/google-sheets/callback",
        "auth-code",
    )
    .await
    .expect("exchange");
    assert_eq!(
        auth["metadata"],
        json!({
            "scope": "https://www.googleapis.com/auth/spreadsheets",
            "accountId": "google-account-1",
            "email": "owner@example.test",
        })
    );

    let db = two_workspaces().await;
    let encryption = encryption();
    for round in 0..2 {
        let (session_id, nonce, _) = start(&db, "googleSheets", "google-sheets").await;
        let upstream = Script::with(vec![
            tokens("https://www.googleapis.com/auth/spreadsheets"),
            json!({ "sub": "google-account-1", "email": "owner@example.test" }),
        ]);
        let session = oauth::complete_authorization(
            &db,
            &encryption,
            &upstream,
            session_id,
            &nonce,
            "auth-code",
            &credential("google-sheets"),
        )
        .await
        .expect("complete");
        let seen = upstream.seen.lock().unwrap().clone();
        assert_eq!(
            seen[1].url,
            "https://oauth2.googleapis.com/tokeninfo?access_token=access-token"
        );
        let result = oauth::connect_targets(
            &db,
            &encryption,
            &Script::default(),
            session.id,
            WS,
            &["google-account-1".into()],
            None,
        )
        .await
        .expect("connect");
        let expected = if round == 0 {
            "connected"
        } else {
            "duplicated"
        };
        assert_eq!(result.outcomes[0]["status"], expected);
    }
    let rows: i64 = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT count(*) FROM "IntegrationGoogleSheet" WHERE "workspaceId" = 2001"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("count");
    assert_eq!(rows, 1);
}

/// "rejects an exchange without a stable Google account id".
#[tokio::test]
async fn google_sheets_rejects_an_exchange_without_a_stable_account_id() {
    let error = oauth_google::exchange_code(
        &Script::with(vec![
            tokens("https://www.googleapis.com/auth/spreadsheets"),
            json!({}),
        ]),
        "googleSheets",
        &json!({ "clientId": "client-1", "clientSecret": "secret-1" }),
        "https://app.example.test/integrations/google-sheets/callback",
        "auth-code",
    )
    .await
    .expect_err("no account id");
    assert_eq!(
        error,
        oauth::ProviderFailure::Rejected(
            "Google Sheets token info has no stable account id".into()
        )
    );
}

/// "uses the immutable Google account id as the connection identity".
#[test]
fn google_sheets_describe_uses_the_immutable_account_id() {
    let auth =
        json!({ "metadata": { "accountId": "google-account-1", "email": "owner@example.test" } });
    assert_eq!(
        oauth_google::describe("googleSheets", &auth).expect("describe"),
        ("google-account-1".into(), "owner@example.test".into())
    );
}
