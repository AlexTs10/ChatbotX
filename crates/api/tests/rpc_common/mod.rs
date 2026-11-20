//! Shared fixture of the Q.D11 `/rpc` tests: a migrated in-memory database
//! with a workspace, its owner (super admin), an agent member and an outsider,
//! one session each, and a call helper.

#![allow(dead_code)]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::handlers::ApiState;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::rpc::{RpcState, rpc_router_with};
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings, sign_cookie_value};
use chatbotx_business::keys::Edition;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt as _;

pub const SECRET: &str = "q-d11-rpc-secret";
pub const NOW_MS: i64 = 1_800_000_000_000;
pub const OWNER: i64 = 101;
pub const AGENT: i64 = 102;
pub const OUTSIDER: i64 = 103;
pub const WS: i64 = 201;
pub const OWNER_TOKEN: &str = "ownertoken";
pub const AGENT_TOKEN: &str = "agenttoken";
pub const OUTSIDER_TOKEN: &str = "outsidertoken";

fn now_ms() -> i64 {
    NOW_MS
}

/// The test `encryptUtils` key.
pub fn encryption() -> chatbotx_core::encryption::Encryption {
    chatbotx_core::encryption::Encryption::new(
        chatbotx_core::encryption::Keys::new(&"a1".repeat(32)).expect("test key"),
    )
}

pub struct Fixture {
    pub auth: Arc<AuthState>,
    pub api: Arc<ApiState>,
}

impl Fixture {
    pub fn db(&self) -> &Db {
        &self.api.db
    }
}

pub async fn fixture() -> Fixture {
    fixture_as(Edition::Community).await
}

/// [`fixture`] for an edition.
pub async fn fixture_as(edition: Edition) -> Fixture {
    let db = Db::open_in_memory().expect("open db");
    db.migrate().await.expect("migrate");
    seed(&db).await;
    fixture_on(db, edition)
}

/// The users, sessions, workspace and members of [`fixture`].
pub async fn seed(db: &Db) {
    db.write(|tx| {
        for (id, email, name) in [
            (OWNER, "owner@rpc.test", "Owner"),
            (AGENT, "agent@rpc.test", "Agent"),
            (OUTSIDER, "outsider@rpc.test", "Outsider"),
        ] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email", "name", "emailVerified") VALUES (?1, ?2, ?3, 1)"#,
                params![id, email, name],
            )?;
        }
        for (id, token, user) in [
            (301, OWNER_TOKEN, OWNER),
            (302, AGENT_TOKEN, AGENT),
            (303, OUTSIDER_TOKEN, OUTSIDER),
        ] {
            tx.execute(
                r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt") VALUES (?1, ?2, ?3, ?4)"#,
                params![id, token, user, NOW_MS + 60_000],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'Acme', ?2)"#,
            params![WS, OWNER],
        )?;
        let full = r#"{"superAdmin":true,"analytics":true,"flows":true,"contacts":true,"onlyAssignedContacts":true,"emailAndPhone":true,"broadcast":true,"ecommerce":true}"#;
        for (id, user, role, permissions) in [
            (401, OWNER, "owner", full),
            (402, AGENT, "agent", r#"{"contacts":true}"#),
        ] {
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions") VALUES (?1, ?2, ?3, ?4, ?5)"#,
                params![id, WS, user, role, permissions],
            )?;
        }
        Ok(())
    })
    .await
    .expect("seed");
}

/// The fixture state on a migrated, [`seed`]ed database.
pub fn fixture_on(db: Db, edition: Edition) -> Fixture {
    let db = Arc::new(db);
    let auth = Arc::new(AuthState {
        db: Arc::clone(&db),
        settings: AuthSettings {
            secret: SECRET.to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled: false,
            trusted_origins: TrustedOriginSettings {
                broker_url: None,
                builder_url: "http://app".to_string(),
                next_phase: None,
            },
            extra_trusted_origins: None,
        },
        mailer: Arc::new(NoMailer),
        now_ms,
    });
    // Audit rows are written inline (checked by the tests).
    let audit = chatbotx_business::audit::AuditService::direct(Arc::clone(&db), true);
    let api = Arc::new(
        ApiState::new(db, "http://app")
            .with_edition(edition)
            .with_audit(audit)
            .with_encryption(Arc::new(encryption())),
    );
    Fixture { auth, api }
}

/// One `/rpc` call (`POST` body `{ json }`, `GET` `?data=`).
pub async fn call(
    fixture: &Fixture,
    method: &str,
    path: &str,
    token: Option<&str>,
    input: Value,
) -> (StatusCode, Value) {
    let router = rpc_router_with(RpcState {
        auth: Arc::clone(&fixture.auth),
        api: Some(Arc::clone(&fixture.api)),
        realtime_secret: None,
    });
    let payload = json!({ "json": input }).to_string();
    let mut builder = Request::builder().method(method);
    builder = if method == "GET" {
        let data: String = payload.bytes().map(|byte| format!("%{byte:02X}")).collect();
        builder.uri(format!("{path}?data={data}"))
    } else {
        builder
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
    };
    if let Some(token) = token {
        builder = builder.header(
            header::COOKIE,
            format!(
                "better-auth.session_token={}",
                sign_cookie_value(token, SECRET)
            ),
        );
    }
    let body = if method == "GET" {
        Body::empty()
    } else {
        Body::from(payload)
    };
    let response = router.oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    (status, value)
}

/// A server action (`POST /rpc/<key>/<name>`).
pub async fn action(
    fixture: &Fixture,
    key: &str,
    name: &str,
    token: &str,
    input: Value,
) -> (StatusCode, Value) {
    call(
        fixture,
        "POST",
        &format!("/rpc/{key}/{name}"),
        Some(token),
        input,
    )
    .await
}

/// One text/integer column of a row (`sql` has one `?1`).
pub async fn scalar(fixture: &Fixture, sql: &'static str, id: i64) -> Value {
    fixture
        .db()
        .read(move |conn| {
            Ok(conn.query_row(sql, [id], |row| {
                Ok(match row.get_ref(0)? {
                    chatbotx_db::rusqlite::types::ValueRef::Null => Value::Null,
                    chatbotx_db::rusqlite::types::ValueRef::Integer(i) => json!(i),
                    chatbotx_db::rusqlite::types::ValueRef::Real(f) => json!(f),
                    chatbotx_db::rusqlite::types::ValueRef::Text(t) => {
                        json!(String::from_utf8_lossy(t))
                    }
                    chatbotx_db::rusqlite::types::ValueRef::Blob(_) => Value::Null,
                })
            })?)
        })
        .await
        .expect("scalar")
}
