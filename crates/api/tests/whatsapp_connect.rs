//! WhatsApp connect and reconnect (Q.P7-V4, `handlers/whatsapp_connect.rs`).
//! Ports `whatsapp-reconnect-action.test.ts`, `persist-reconnect-auth.test.ts`
//! and `whatsapp-connect-action.session.test.ts`: the TS tests mock Meta and
//! the services; here a local Graph server (`ApiState.graph_url`) answers and
//! the stored rows are checked. The cloud-only owner-access cases
//! (`trialExpired`, `macLimitReached`) are not ported.

mod rpc_common;

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::routing::get;
use chatbotx_db::repo::partials::credential::credential_aad_for;
use chatbotx_db::repo::whatsapp_signup_session as signup;
use chatbotx_db::rusqlite::params;
use rpc_common::{
    AGENT_TOKEN, Fixture, OUTSIDER, OUTSIDER_TOKEN, OWNER, OWNER_TOKEN, WS, action, call,
    encryption, fixture,
};
use serde_json::{Value, json};

const PHONE: &str = "100000000000001";
const PHONE_2: &str = "100000000000002";
const WABA: &str = "waba-1";

type Seen = Arc<Mutex<Vec<String>>>;

fn reply(status: u16, value: Value) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        StatusCode::from_u16(status).unwrap(),
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

fn seen_line(method: &str, uri: &Uri, body: &str) -> String {
    format!(
        "{method} {}?{} {body}",
        uri.path(),
        uri.query().unwrap_or_default()
    )
}

fn number(id: &str, name: &str) -> Value {
    json!({ "id": id, "display_phone_number": "+1 650-555-1234", "verified_name": name,
            "status": "CONNECTED", "platform_type": "CLOUD_API", "code_verification_status": "VERIFIED" })
}

async fn graph_get(State(seen): State<Seen>, uri: Uri) -> axum::response::Response {
    seen.lock().unwrap().push(seen_line("GET", &uri, ""));
    let query = uri.query().unwrap_or_default();
    match uri.path() {
        "/v23.0/oauth/access_token" => reply(200, json!({ "access_token": "fresh-token" })),
        "/debug_token" => reply(
            200,
            json!({ "data": { "is_valid": true, "granular_scopes": [
                { "scope": "whatsapp_business_management", "target_ids": ["other-waba", WABA] },
                { "scope": "whatsapp_business_messaging", "target_ids": [WABA] },
                { "scope": "whatsapp_business_manage_events", "target_ids": ["presence-1"] },
            ] } }),
        ),
        "/v23.0/other-waba" | "/v23.0/waba-1" if query.contains("metadata=1") => reply(
            200,
            json!({ "id": "x", "metadata": { "type": "whatsapp_business_account" } }),
        ),
        "/v23.0/waba-1" => reply(
            200,
            json!({ "owner_business_info": { "id": "business-1" } }),
        ),
        "/v23.0/other-waba/phone_numbers" => {
            reply(200, json!({ "data": [number(PHONE, "Other")] }))
        }
        "/v23.0/waba-1/phone_numbers" => reply(
            200,
            json!({ "data": [number(PHONE, "  Acme Support "), number(PHONE_2, "Acme Two")] }),
        ),
        "/v23.0/biz-app/extendedcredits" => reply(
            200,
            json!({ "data": [{ "id": "line-1", "legal_entity_name": "Acme Holdings Ltd" }] }),
        ),
        _ => reply(
            404,
            json!({ "error": { "message": "unknown node", "code": 100 } }),
        ),
    }
}

async fn graph_post(State(seen): State<Seen>, uri: Uri, body: String) -> axum::response::Response {
    seen.lock().unwrap().push(seen_line("POST", &uri, &body));
    match uri.path() {
        // `other-waba` refuses WABA-level use (the misuse subcode).
        "/v23.0/other-waba/assigned_users" => reply(
            400,
            json!({ "error": { "message": "Invalid", "code": 100, "error_subcode": 2_388_339 } }),
        ),
        _ => reply(200, json!({ "success": true })),
    }
}

async fn mock_graph(fx: &mut Fixture) -> Seen {
    let seen: Seen = Arc::default();
    let app = axum::Router::new()
        .route("/{*path}", get(graph_get).post(graph_post))
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    Arc::get_mut(&mut fx.api).expect("one api").graph_url =
        format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    seen
}

fn calls(seen: &Seen, needle: &str) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains(needle))
        .cloned()
        .collect()
}

/// The platform WhatsApp app settings (`PlatformCredential`, no owner).
async fn seed_credential(fx: &Fixture) {
    let config = json!({
        "clientId": "app-1", "clientSecret": "secret-1", "version": "v23.0", "configId": "cfg",
        "systemUserId": "sys-user", "systemUserToken": "sys-token", "businessId": "biz-app",
        "businessName": "Acme", "verifyToken": "verify-1",
    });
    let aad = credential_aad_for(None, "whatsapp");
    let blob = encryption()
        .encrypt_object(&config, Some(&aad))
        .expect("encrypt");
    let value = serde_json::to_string(&blob).expect("json");
    fx.db()
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "PlatformCredential" ("id", "type", "publicConfig", "value") VALUES (900, 'whatsapp', '{}', ?1)"#,
                params![value],
            )?;
            Ok(())
        })
        .await
        .expect("credential");
}

/// The first column of the first row.
async fn one(fx: &Fixture, sql: String) -> Value {
    fx.db()
        .read(move |conn| {
            Ok(conn.query_row(&sql, [], |row| {
                use chatbotx_db::rusqlite::types::ValueRef;
                Ok(match row.get_ref(0)? {
                    ValueRef::Integer(i) => json!(i),
                    ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                    _ => Value::Null,
                })
            })?)
        })
        .await
        .unwrap()
}

// ---- reconnect -------------------------------------------------------------

/// A connected number of `WS` (inbox 6) whose `Connection` needs re-auth.
async fn seed_integration(fx: &Fixture, waba: &str, connection_ws: i64) {
    let waba = waba.to_owned();
    fx.db()
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (202, 'Other', ?1)"#,
                params![OWNER],
            )?;
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId", "status")
                   VALUES (6, ?1, 'Wa', 'whatsapp', ?2, 'disconnected')"#,
                params![WS, PHONE],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
                   VALUES (301, '{"tokens":{"accessToken":"old-token"}}', ?1, ?2, 'business-1', 'Wa', ?3, 6)"#,
                params![PHONE, waba, WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Connection" ("id", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId", "displayName", "status", "statusReason", "lastError")
                   VALUES (311, ?1, 'whatsapp', 'channel', 'whatsapp', 6, ?2, 'Wa', 'needs_reauth', 'token_revoked', 'expired')"#,
                params![connection_ws, PHONE],
            )?;
            Ok(())
        })
        .await
        .expect("integration");
}

async fn reconnect(fx: &Fixture, token: &str, code: &str) -> (StatusCode, Value) {
    action(
        fx,
        "actions",
        "reconnectWhatsappAction",
        token,
        json!({ "workspaceId": WS.to_string(), "args": [WS.to_string(), "301", { "code": code }] }),
    )
    .await
}

#[tokio::test]
async fn reconnect_rejects_a_non_super_admin_before_any_call() {
    let mut fx = fixture().await;
    let seen = mock_graph(&mut fx).await;
    seed_credential(&fx).await;
    seed_integration(&fx, WABA, WS).await;
    let (status, body) = reconnect(&fx, AGENT_TOKEN, "code-1").await;
    assert!(status.is_client_error());
    assert!(
        body.to_string()
            .contains("You need to be a workspace super admin to do this."),
        "{body}"
    );
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reconnect_replaces_auth_and_resubscribes_with_automatic_events() {
    let mut fx = fixture().await;
    let seen = mock_graph(&mut fx).await;
    seed_credential(&fx).await;
    seed_integration(&fx, WABA, WS).await;
    let (status, body) = reconnect(&fx, OWNER_TOKEN, "code-1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["json"],
        json!({ "ok": true, "hasCapiScope": true, "scope": "whatsapp_business_manage_events", "resubscribed": true }),
        "{body}"
    );

    // The exchange uses the broker callback; the resolver is hinted with the
    // connected number and probes with the system user (`other-waba` is
    // refused with the misuse subcode, `waba-1` is usable).
    let exchange = calls(&seen, "/oauth/access_token");
    assert!(exchange[0].contains("code=code-1"), "{exchange:?}");
    assert!(
        exchange[0].contains("redirect_uri=http%3A%2F%2Fapp%2Fintegrations%2Fwhatsapp%2Fcallback")
    );
    let probes = calls(&seen, "/assigned_users");
    assert_eq!(probes.len(), 2, "{probes:?}");
    assert!(
        probes[0].starts_with("POST /v23.0/other-waba/assigned_users?user=sys-user&tasks=MANAGE")
    );
    assert!(probes[1].starts_with("POST /v23.0/waba-1/assigned_users?user=sys-user"));

    let subscribe = calls(&seen, "/subscribed_apps");
    assert_eq!(subscribe.len(), 1);
    assert!(
        subscribe[0].contains(r#""automatic_events""#),
        "{subscribe:?}"
    );

    let auth: Value = serde_json::from_str(
        one(
            &fx,
            r#"SELECT "auth" FROM "IntegrationWhatsapp" WHERE "id" = 301"#.into(),
        )
        .await
        .as_str()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(auth["tokens"]["accessToken"], "fresh-token");
    assert_eq!(auth["authType"], "oauth2");
    assert_eq!(auth["metadata"]["wabaId"], WABA);
    assert_eq!(auth["metadata"]["businessId"], "business-1");
    assert_eq!(auth["metadata"]["phoneNumber"]["id"], PHONE);
    assert_eq!(
        auth["redirectUrl"],
        "http://app/integrations/whatsapp/callback"
    );
    assert_eq!(
        auth["metadata"]["webhookUrl"],
        "http://app/integrations/whatsapp/webhook"
    );
    assert_eq!(
        one(
            &fx,
            r#"SELECT "hasCapiScope" FROM "IntegrationWhatsapp" WHERE "id" = 301"#.into()
        )
        .await,
        1
    );
    // `commitReconnect`: the inbox's connection is connected again.
    assert_eq!(
        one(&fx, r#"SELECT "status" || ':' || coalesce("statusReason", '-') || ':' || coalesce("lastError", '-') FROM "Connection" WHERE "id" = 311"#.into()).await,
        "connected:-:-"
    );
    assert_eq!(
        one(&fx, r#"SELECT "status" FROM "Inbox" WHERE "id" = 6"#.into()).await,
        "connected"
    );

    // `upsertCurrentCredential`: the sealed `{ accessToken, apiVersion }`.
    let (sealed, scopes, business): (String, String, String) = fx
        .db()
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "credential", "grantedScopes", "businessId" FROM "WhatsappBusinessAccount" WHERE "workspaceId" = ?1 AND "wabaId" = ?2"#,
                params![WS, WABA],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .unwrap();
    let blob = serde_json::from_str(&sealed).unwrap();
    let plain: Value = encryption()
        .decrypt_object(&blob, Some(&format!("whatsapp-waba:{WS}:{WABA}")))
        .unwrap();
    assert_eq!(
        plain,
        json!({ "accessToken": "fresh-token", "apiVersion": "v23.0" })
    );
    assert_eq!(business, "business-1");
    let scopes: Vec<String> = serde_json::from_str(&scopes).unwrap();
    assert_eq!(
        scopes,
        [
            "whatsapp_business_management",
            "whatsapp_business_messaging",
            "whatsapp_business_manage_events"
        ]
    );
}

#[tokio::test]
async fn reconnect_refuses_a_different_waba_and_an_empty_code() {
    let mut fx = fixture().await;
    mock_graph(&mut fx).await;
    seed_credential(&fx).await;
    seed_integration(&fx, "waba-9", WS).await;
    let (status, body) = reconnect(&fx, OWNER_TOKEN, "code-1").await;
    assert!(status.is_client_error());
    assert!(
        body.to_string().contains(
            "Reconnect returned a different WhatsApp Business Account. Please reconnect the same account."
        ),
        "{body}"
    );
    let (status, body) = reconnect(&fx, OWNER_TOKEN, "  ").await;
    assert!(status.is_client_error());
    assert!(body.to_string().contains("validationErrors"), "{body}");
}

#[tokio::test]
async fn reconnect_commit_failure_aborts_before_the_credential_and_the_subscribe() {
    let mut fx = fixture().await;
    let seen = mock_graph(&mut fx).await;
    seed_credential(&fx).await;
    // The inbox's connection belongs to another workspace: `reconnectInbox`
    // fails and the auth write rolls back with it.
    seed_integration(&fx, WABA, 202).await;
    let (status, body) = reconnect(&fx, OWNER_TOKEN, "code-1").await;
    assert!(status.is_client_error());
    assert!(body.to_string().contains("Connection not found"), "{body}");
    assert!(calls(&seen, "/subscribed_apps").is_empty());
    assert_eq!(
        one(
            &fx,
            r#"SELECT COUNT(*) FROM "WhatsappBusinessAccount""#.into()
        )
        .await,
        0
    );
    let auth = one(
        &fx,
        r#"SELECT "auth" FROM "IntegrationWhatsapp" WHERE "id" = 301"#.into(),
    )
    .await;
    assert!(auth.to_string().contains("old-token"), "{auth}");
}

// ---- connect from a signup session ----------------------------------------

const CONNECT: &str = "/rpc/integrationWhatsappAPIs/connectWhatsappNumberAPI";

async fn session(fx: &Fixture, user: i64, workspace: Option<i64>, candidates: &[&str]) -> String {
    let sealed = encryption().encrypt_text("session-token", None).unwrap();
    let input = signup::CreateInput {
        user_id: user,
        owner_id: user,
        workspace_id: workspace,
        waba_id: WABA.into(),
        business_id: "business-1".into(),
        encrypted_access_token: serde_json::to_value(sealed).unwrap(),
        api_version: "v23.0".into(),
        candidate_phone_number_ids: candidates.iter().map(|c| (*c).to_owned()).collect(),
        now: chatbotx_db::repo::now_ms(),
    };
    fx.db()
        .write(move |tx| {
            Ok(signup::create_signup_session(tx, &input)
                .expect("signup session")
                .id)
        })
        .await
        .unwrap()
        .to_string()
}

async fn connect(fx: &Fixture, token: &str, session: &str, phone: &str) -> Value {
    let (status, body) = call(
        fx,
        "POST",
        CONNECT,
        Some(token),
        json!({ "signupSessionId": session, "phoneNumberId": phone, "connectExisting": false }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["json"].clone()
}

#[tokio::test]
async fn connect_refuses_foreign_or_unknown_sessions_and_non_members() {
    let mut fx = fixture().await;
    let seen = mock_graph(&mut fx).await;
    let own = session(&fx, OWNER, None, &[PHONE]).await;

    // No credential yet: `credentialMissing` after the session checks.
    assert_eq!(
        connect(&fx, OWNER_TOKEN, &own, PHONE).await,
        json!({ "kind": "sessionError", "code": "credentialMissing" })
    );
    seed_credential(&fx).await;
    for (token, id) in [
        (AGENT_TOKEN, own.as_str()),
        (OWNER_TOKEN, "999"),
        (OWNER_TOKEN, "abc"),
    ] {
        assert_eq!(
            connect(&fx, token, id, PHONE).await,
            json!({ "kind": "sessionError", "code": "sessionExpired" })
        );
    }
    // A session bound to a workspace the user is not a member of.
    let outsider = session(&fx, OUTSIDER, Some(WS), &[PHONE]).await;
    assert_eq!(
        connect(&fx, OUTSIDER_TOKEN, &outsider, PHONE).await["code"],
        json!("notMember")
    );
    assert!(seen.lock().unwrap().is_empty());

    // Input validation (`transferPhoneNumber: true` is refused).
    for input in [
        json!({ "signupSessionId": own, "phoneNumberId": PHONE }),
        json!({ "signupSessionId": own, "phoneNumberId": PHONE, "connectExisting": false, "transferPhoneNumber": true }),
    ] {
        let (status, _) = call(&fx, "POST", CONNECT, Some(OWNER_TOKEN), input).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn connect_refuses_non_candidates_and_connected_numbers_without_meta_calls() {
    let mut fx = fixture().await;
    let seen = mock_graph(&mut fx).await;
    seed_credential(&fx).await;
    let id = session(&fx, OWNER, Some(WS), &[PHONE]).await;
    let out = connect(&fx, OWNER_TOKEN, &id, PHONE_2).await;
    assert_eq!(out["outcome"]["status"], "failed");
    assert_eq!(out["outcome"]["reason"], "notSelectable");
    assert_eq!(out["outcome"]["name"], PHONE_2);

    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (6, ?1, 'Wa', 'whatsapp', ?2)"#,
                params![WS, PHONE],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
                   VALUES (301, '{}', ?1, ?2, 'business-1', 'Wa', ?3, 6)"#,
                params![PHONE, WABA, WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let out = connect(&fx, OWNER_TOKEN, &id, PHONE).await;
    assert_eq!(out["outcome"]["status"], "duplicated");
    assert_eq!(out["outcome"]["reason"], "alreadyConnected");
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn connect_creates_and_binds_the_workspace_then_reuses_it() {
    let mut fx = fixture().await;
    let seen = mock_graph(&mut fx).await;
    seed_credential(&fx).await;
    let id = session(&fx, OWNER, None, &[PHONE, PHONE_2]).await;

    let first = connect(&fx, OWNER_TOKEN, &id, PHONE).await;
    assert_eq!(first["type"], "connected", "{first}");
    assert_eq!(first["isManual"], false);
    let ws: i64 = first["workspaceId"].as_str().unwrap().parse().unwrap();
    assert_ne!(ws, WS);
    assert_eq!(first["redirectUrl"], format!("/space/{ws}"));
    let outcome = &first["outcome"];
    assert_eq!(outcome["status"], "connected");
    assert_eq!(outcome["name"], "Acme Support");
    assert_eq!(outcome["coexistEligible"], false);
    assert!(outcome.get("warning").is_none(), "{first}");
    assert_eq!(outcome["extra"]["requiresPhoneVerification"], false);
    assert_eq!(outcome["extra"]["registrationError"], Value::Null);

    // The new workspace is named after the number and bound to the session.
    assert_eq!(
        one(
            &fx,
            format!(r#"SELECT "name" FROM "Workspace" WHERE "id" = {ws}"#)
        )
        .await,
        "Acme Support"
    );
    assert_eq!(
        one(
            &fx,
            format!(r#"SELECT "workspaceId" FROM "WhatsappSignupSession" WHERE "id" = {id}"#)
        )
        .await,
        ws
    );
    let row = fx
        .db()
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT w."displayPhoneNumber", w."wabaId", i."status", c."status", c."createdBy"
                   FROM "IntegrationWhatsapp" w JOIN "Inbox" i ON i."id" = w."inboxId"
                   JOIN "Connection" c ON c."inboxId" = i."id" WHERE w."phoneNumberId" = ?1 AND w."workspaceId" = ?2"#,
                params![PHONE, ws],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        row,
        (
            "16505551234".into(),
            WABA.into(),
            "connected".into(),
            "connected".into(),
            OWNER
        )
    );
    // First number of the WABA: system user, credit line, plain subscribe.
    assert_eq!(
        calls(
            &seen,
            "POST /v23.0/waba-1/assigned_users?user=sys-user&tasks=MANAGE"
        )
        .len(),
        1
    );
    assert_eq!(
        calls(
            &seen,
            "whatsapp_credit_sharing_and_attach?waba_id=waba-1&waba_currency=USD"
        )
        .len(),
        1
    );
    let subscribe = calls(&seen, "/subscribed_apps");
    assert_eq!(subscribe.len(), 1);
    assert!(!subscribe[0].contains("automatic_events"));
    assert!(
        one(&fx, format!(r#"SELECT "provisionedAt" FROM "WhatsappBusinessAccount" WHERE "workspaceId" = {ws}"#)).await.is_i64()
    );

    // The second number reuses the bound workspace; the WABA is provisioned.
    let second = connect(&fx, OWNER_TOKEN, &id, PHONE_2).await;
    assert_eq!(second["workspaceId"], first["workspaceId"], "{second}");
    assert_eq!(calls(&seen, "/subscribed_apps").len(), 1);
    assert_eq!(
        one(&fx, r#"SELECT COUNT(*) FROM "Workspace""#.into()).await,
        2
    );
    // A connected number again: duplicated before any Meta call.
    let before = seen.lock().unwrap().len();
    let again = connect(&fx, OWNER_TOKEN, &id, PHONE).await;
    assert_eq!(again["outcome"]["status"], "duplicated");
    assert_eq!(seen.lock().unwrap().len(), before);
}

#[tokio::test]
async fn connect_maps_a_refused_claim_and_a_number_connected_elsewhere() {
    let mut fx = fixture().await;
    mock_graph(&mut fx).await;
    seed_credential(&fx).await;

    // Claimed already (no integration row yet): the claim in the
    // transaction is refused → the session expired.
    let id = session(&fx, OWNER, Some(WS), &[PHONE, PHONE_2]).await;
    let session_id: i64 = id.parse().unwrap();
    fx.db()
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "WhatsappSignupSession" SET "claimedPhoneNumberIds" = ?2 WHERE "id" = ?1"#,
                params![session_id, format!(r#"["{PHONE}"]"#)],
            )?;
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (202, 'Other', ?1)"#,
                params![OUTSIDER],
            )?;
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId", "status") VALUES (8, 202, 'Wa', 'whatsapp', ?1, 'connected')"#,
                params![PHONE_2],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        connect(&fx, OWNER_TOKEN, &id, PHONE).await,
        json!({ "kind": "sessionError", "code": "sessionExpired" })
    );
    // Connected in another workspace: duplicated, named by the raw id.
    let out = connect(&fx, OWNER_TOKEN, &id, PHONE_2).await;
    assert_eq!(
        out,
        json!({ "kind": "outcome", "outcome": { "sourceId": PHONE_2, "name": PHONE_2, "coexistEligible": false, "status": "duplicated", "reason": "alreadyConnected" } })
    );
    assert_eq!(
        one(&fx, r#"SELECT COUNT(*) FROM "IntegrationWhatsapp""#.into()).await,
        0
    );
}
