//! Q.A1 §3: member scoping of the mobile-allowlisted `/v1` handlers
//! (Rust-native port of the cases of `mobile-conversations-member-scope`,
//! `mobile-contacts-member-scope` and `mobile-member-allowlist` tests, over
//! a real database instead of mocked services).
//!
//! Workspace `WS`: agent 1010 (assigned-only, team 50), agent 1011.
//! Conversations: 401 → 1010, 402 → team 50, 403 → 1011, 404 unassigned;
//! each on its own contact 301..304 (with email and phone).

mod handlers_common;

use std::sync::Arc;

use chatbotx_api::auth::MobileMember;
use chatbotx_api::handlers::{Call, Principal, dispatch_as};
use chatbotx_business::member_scope::resolve_conversation_scope;
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

const AGENT: i64 = 1010;

async fn seeded() -> Fixture {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            for user in [1010_i64, 1011, 1012, 1013] {
                tx.execute(
                    r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
                    params![user, format!("agent-{user}@example.test")],
                )?;
                tx.execute(
                    r#"INSERT INTO "WorkspaceMember" ("workspaceId", "userId", "role") VALUES (?1, ?2, 'agent')"#,
                    params![WS, user],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "InboxTeam" ("id", "workspaceId", "name") VALUES (50, ?1, 'team')"#,
                [WS],
            )?;
            tx.execute(
                r#"INSERT INTO "InboxTeamMember" ("id", "inboxTeamId", "userId") VALUES (1, 50, ?1)"#,
                [AGENT],
            )?;
            for (n, user, team) in [
                (1_i64, Some(1010_i64), None),
                (2, None, Some(50_i64)),
                (3, Some(1011), None),
                (4, None, None),
            ] {
                tx.execute(
                    r#"INSERT INTO "Contact" ("id", "firstName", "email", "phoneNumber", "workspaceId")
                       VALUES (?1, 'C', ?2, '+15550000', ?3)"#,
                    params![300 + n, format!("c{n}@example.test"), WS],
                )?;
                tx.execute(
                    r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "assignedUserId", "assignedInboxTeamId")
                       VALUES (?1, ?2, ?3, ?4, ?5)"#,
                    params![400 + n, WS, 300 + n, user, team],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    fx
}

async fn member(fx: &Fixture, user_id: i64, permissions: Value) -> Arc<MobileMember> {
    let conversation = resolve_conversation_scope(&fx.state.db, WS, user_id, &permissions)
        .await
        .expect("scope");
    Arc::new(MobileMember {
        user_id,
        permissions,
        conversation,
    })
}

fn assigned_only() -> Value {
    json!({ "onlyAssignedContacts": true })
}

fn full_access() -> Value {
    json!({ "contacts": true, "emailAndPhone": true, "flows": true })
}

async fn call_mobile(
    fx: &Fixture,
    member: &Arc<MobileMember>,
    route_id: &str,
    params: &[(&'static str, &str)],
    body: Option<Value>,
) -> (u16, Value) {
    let params: Vec<(&'static str, String)> =
        params.iter().map(|(k, v)| (*k, (*v).to_string())).collect();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let principal = Principal {
        mobile: Some(Arc::clone(member)),
        ..Principal::default()
    };
    let response = dispatch_as(
        &fx.state,
        route_id,
        Call {
            workspace_id: WS,
            params: &params,
            query: None,
            body: body.as_bytes(),
        },
        &principal,
    )
    .await
    .unwrap_or_else(|| panic!("{route_id} has no handler"));
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

fn ids(body: &Value) -> Vec<String> {
    let mut ids: Vec<String> = body["data"]
        .as_array()
        .expect("data array")
        .iter()
        .map(|row| row["id"].as_str().expect("id").to_string())
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn an_assigned_only_agent_lists_only_their_own_and_their_teams_conversations_masked() {
    let fx = seeded().await;
    let agent = member(&fx, AGENT, assigned_only()).await;
    let (status, body) = call_mobile(&fx, &agent, "conversations.list", &[], None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(ids(&body), ["401", "402"]);
    for row in body["data"].as_array().unwrap() {
        assert_eq!(row["contact"]["email"], Value::Null);
        assert_eq!(row["contact"]["phoneNumber"], Value::Null);
    }
}

#[tokio::test]
async fn an_agent_in_no_team_lists_only_their_own_conversations() {
    let fx = seeded().await;
    let agent = member(&fx, 1011, assigned_only()).await;
    let (status, body) = call_mobile(&fx, &agent, "conversations.list", &[], None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(ids(&body), ["403"]);
}

#[tokio::test]
async fn a_member_without_inbox_access_is_not_found_before_listing() {
    let fx = seeded().await;
    let none = member(&fx, 1012, json!({})).await;
    let (status, _) = call_mobile(&fx, &none, "conversations.list", &[], None).await;
    assert_eq!(status, 404);
    let (status, _) = call_mobile(&fx, &none, "contacts.list", &[], None).await;
    assert_eq!(status, 404);
    let (status, _) = call_mobile(
        &fx,
        &none,
        "contacts.get",
        &[("identifier", "id:301")],
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn full_access_sees_everything_with_pii_and_a_token_stays_workspace_level() {
    let fx = seeded().await;
    let full = member(&fx, 1013, full_access()).await;
    let (_, body) = call_mobile(&fx, &full, "conversations.list", &[], None).await;
    assert_eq!(ids(&body), ["401", "402", "403", "404"]);
    assert!(body["data"][0]["contact"]["email"].is_string());
    let (status, body) = call(&fx, "conversations.list", WS, &[], None, None).await;
    assert_eq!(status, 200);
    assert_eq!(ids(&body), ["401", "402", "403", "404"]);
    assert!(body["data"][0]["contact"]["email"].is_string());
}

#[tokio::test]
async fn out_of_scope_conversations_are_not_found_for_reads_and_actions() {
    let fx = seeded().await;
    let agent = member(&fx, AGENT, assigned_only()).await;
    for id in ["403", "404"] {
        let (status, _) = call_mobile(&fx, &agent, "conversations.get", &[("id", id)], None).await;
        assert_eq!(status, 404, "get {id}");
        let (status, _) =
            call_mobile(&fx, &agent, "conversations.archive", &[("id", id)], None).await;
        assert_eq!(status, 404, "archive {id}");
        let (status, _) = call_mobile(
            &fx,
            &agent,
            "messages.list",
            &[("conversationId", id)],
            None,
        )
        .await;
        assert_eq!(status, 404, "messages {id}");
    }
    let archived: Option<i64> = fx
        .state
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "archivedAt" FROM "Conversation" WHERE "id" = 403"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(archived, None, "a refused action writes nothing");

    let (status, body) =
        call_mobile(&fx, &agent, "conversations.get", &[("id", "402")], None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"]["contact"]["email"], Value::Null);
    let (status, _) =
        call_mobile(&fx, &agent, "conversations.archive", &[("id", "401")], None).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn contacts_and_their_sub_resources_follow_the_agents_scope() {
    let fx = seeded().await;
    let agent = member(&fx, AGENT, assigned_only()).await;
    let (status, body) = call_mobile(&fx, &agent, "contacts.list", &[], None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(ids(&body), ["301", "302"]);
    assert_eq!(body["data"][0]["email"], Value::Null);

    for route in [
        "contacts.get",
        "contacts.listTags",
        "contacts.listCustomFields",
    ] {
        let (status, _) = call_mobile(&fx, &agent, route, &[("identifier", "id:303")], None).await;
        assert_eq!(status, 404, "{route} out of scope");
    }
    let (status, _) = call_mobile(
        &fx,
        &agent,
        "contacts.delete",
        &[("identifier", "id:304")],
        None,
    )
    .await;
    assert_eq!(status, 404);
    let still_there: i64 = fx
        .state
        .db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT COUNT(*) FROM "Contact" WHERE "id" = 304"#,
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(still_there, 1);

    let (status, body) = call_mobile(
        &fx,
        &agent,
        "contacts.get",
        &[("identifier", "id:301")],
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["email"], Value::Null);
    assert_eq!(body["phoneNumber"], Value::Null);

    // `emailAndPhone` keeps the PII.
    let pii = member(
        &fx,
        1013,
        json!({ "contacts": true, "emailAndPhone": true }),
    )
    .await;
    let (_, body) = call_mobile(&fx, &pii, "contacts.get", &[("identifier", "id:303")], None).await;
    assert_eq!(body["email"], "c3@example.test");
}

#[tokio::test]
async fn a_mobile_member_never_reaches_an_unlisted_route() {
    let fx = seeded().await;
    let full = member(&fx, 1013, full_access()).await;
    for route in [
        "tags.create",
        "contacts.create",
        "contacts.count",
        "inboxes.list",
    ] {
        let (status, body) = call_mobile(&fx, &full, route, &[], Some(json!({}))).await;
        assert_eq!(status, 403, "{route}: {body}");
        assert_eq!(
            body["message"],
            "This operation is not available to mobile sessions"
        );
    }
}

// ---- end to end: a signed mobile JWT through the `/api/v1` router ----------

mod e2e {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, header};
    use chatbotx_api::auth::LiveAuthDeps;
    use chatbotx_api::handlers::ApiState;
    use chatbotx_api::routes::public_api_router_with;
    use chatbotx_api::session::{sign_jwt, signing_key};
    use chatbotx_kv::{Kv, KvConfig, SystemClock};
    use tower::ServiceExt as _;

    const ORIGIN: &str = "https://app.example.com";
    const SECRET: &str = "a-very-long-test-secret-for-better-auth-0123456789";

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    async fn token(fx: &Fixture, user_id: i64, audience: &str) -> String {
        let now = now_ms();
        let key = signing_key(&fx.state.db, SECRET, now).await.unwrap();
        sign_jwt(
            &key,
            &json!({
                "sub": user_id.to_string(),
                "iat": now / 1000,
                "exp": now / 1000 + 600,
                "deviceId": "dev-1",
                "mustChangePassword": false,
                "iss": ORIGIN,
                "aud": audience,
            }),
        )
    }

    fn router(fx: &Fixture, mobile_on: bool) -> axum::Router {
        let db = Arc::clone(&fx.state.db);
        let deps = LiveAuthDeps {
            kv: Kv::start(Arc::clone(&db), Arc::new(SystemClock), KvConfig::default()),
            db: Arc::clone(&db),
            clock: Arc::new(SystemClock),
            auth_base_url: format!("{ORIGIN}/api/auth"),
            mcp_resource_url: format!("{ORIGIN}/mcp"),
            is_cloud: false,
            mobile_audience: mobile_on.then(|| format!("{ORIGIN}/api")),
        };
        public_api_router_with(Arc::new(deps), Arc::new(ApiState::new(db, "http://app")))
    }

    async fn send(
        router: &axum::Router,
        method: &str,
        path: &str,
        token: &str,
        workspace: Option<&str>,
    ) -> (u16, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(ws) = workspace {
            request = request.header("x-workspace-id", ws);
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::from("{}")).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn set_permissions(fx: &Fixture, user: i64, permissions: Value) {
        fx.state
            .db
            .write(move |tx| {
                tx.execute(
                    r#"UPDATE "WorkspaceMember" SET "permissions" = ?1 WHERE "userId" = ?2"#,
                    params![permissions.to_string(), user],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_mobile_token_is_scoped_to_its_member_and_fails_closed_elsewhere() {
        let fx = seeded().await;
        set_permissions(&fx, AGENT, assigned_only()).await;
        let router = router(&fx, true);
        let jwt = token(&fx, AGENT, &format!("{ORIGIN}/api")).await;
        let ws = WS.to_string();

        let (status, body) = send(&router, "GET", "/api/v1/conversations", &jwt, Some(&ws)).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(ids(&body), ["401", "402"]);

        let (status, body) = send(&router, "POST", "/api/v1/tags", &jwt, Some(&ws)).await;
        assert_eq!(status, 403, "{body}");
        assert_eq!(
            body["message"],
            "This operation is not available to mobile sessions"
        );

        let (status, body) = send(&router, "GET", "/api/v1/conversations", &jwt, None).await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(body["code"], "WORKSPACE_SELECTION_REQUIRED");

        // Not a member of the other workspace.
        let other = handlers_common::OTHER_WS.to_string();
        let (status, body) =
            send(&router, "GET", "/api/v1/conversations", &jwt, Some(&other)).await;
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["message"], "Account has no access to this workspace");

        // A token for another audience is no mobile token.
        let mcp = token(&fx, AGENT, &format!("{ORIGIN}/mcp")).await;
        let (status, _) = send(&router, "GET", "/api/v1/conversations", &mcp, Some(&ws)).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn a_pending_password_change_and_mobile_tokens_off_are_refused() {
        let fx = seeded().await;
        set_permissions(&fx, AGENT, full_access()).await;
        let jwt = token(&fx, AGENT, &format!("{ORIGIN}/api")).await;
        let ws = WS.to_string();

        let (status, _) = send(
            &router(&fx, false),
            "GET",
            "/api/v1/conversations",
            &jwt,
            Some(&ws),
        )
        .await;
        assert_eq!(status, 401, "mobile tokens off: an invalid workspace token");

        fx.state
            .db
            .write(|tx| {
                tx.execute(
                    r#"UPDATE "User" SET "mustChangePassword" = 1 WHERE "id" = 1010"#,
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let (status, body) = send(
            &router(&fx, true),
            "GET",
            "/api/v1/conversations",
            &jwt,
            Some(&ws),
        )
        .await;
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["message"], "Password change required");
    }

    async fn send_body(
        router: &axum::Router,
        method: &str,
        path: &str,
        token: &str,
        workspace: Option<&str>,
        body: Value,
    ) -> u16 {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(ws) = workspace {
            request = request.header("x-workspace-id", ws);
        }
        router
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    async fn device_tokens(fx: &Fixture) -> Vec<(i64, Option<i64>)> {
        fx.state
            .db
            .read(|c| {
                let mut stmt =
                    c.prepare(r#"SELECT "userId", "workspaceId" FROM "UserDeviceToken""#)?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn me_routes_serve_only_the_signed_in_user() {
        let fx = seeded().await;
        let router = router(&fx, true);
        let jwt = token(&fx, AGENT, &format!("{ORIGIN}/api")).await;

        let (status, body) = send(&router, "GET", "/api/v1/me", &jwt, None).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["id"], AGENT.to_string());
        assert_eq!(body["mustChangePassword"], false);

        let (status, body) = send(&router, "GET", "/api/v1/me/workspaces", &jwt, None).await;
        assert_eq!(status, 200, "{body}");
        let ids: Vec<&str> = body["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [WS.to_string()]);

        let other = handlers_common::OTHER_WS.to_string();
        let device = json!({ "platform": "ios", "token": "ExponentPushToken[x]" });
        assert_eq!(
            send_body(
                &router,
                "PUT",
                "/api/v1/me/device-tokens",
                &jwt,
                Some(&other),
                device.clone()
            )
            .await,
            403
        );
        assert!(device_tokens(&fx).await.is_empty());
        let ws = WS.to_string();
        assert_eq!(
            send_body(
                &router,
                "PUT",
                "/api/v1/me/device-tokens",
                &jwt,
                Some(&ws),
                device
            )
            .await,
            200
        );
        assert_eq!(device_tokens(&fx).await, [(AGENT, Some(WS))]);

        // Another user cannot remove it.
        let other_jwt = token(&fx, 1011, &format!("{ORIGIN}/api")).await;
        let remove = json!({ "token": "ExponentPushToken[x]" });
        assert_eq!(
            send_body(
                &router,
                "DELETE",
                "/api/v1/me/device-tokens",
                &other_jwt,
                None,
                remove.clone()
            )
            .await,
            200
        );
        assert_eq!(device_tokens(&fx).await.len(), 1);
        assert_eq!(
            send_body(
                &router,
                "DELETE",
                "/api/v1/me/device-tokens",
                &jwt,
                None,
                remove
            )
            .await,
            200
        );
        assert!(device_tokens(&fx).await.is_empty());

        // A pending password change still reads `/v1/me`, nothing else.
        fx.state
            .db
            .write(|tx| {
                tx.execute(
                    r#"UPDATE "User" SET "mustChangePassword" = 1 WHERE "id" = 1010"#,
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let (status, body) = send(&router, "GET", "/api/v1/me", &jwt, None).await;
        assert_eq!(
            (status, body["mustChangePassword"].clone()),
            (200, json!(true))
        );
        let (status, _) = send(&router, "GET", "/api/v1/me/workspaces", &jwt, None).await;
        assert_eq!(status, 403);

        // Mobile tokens off: `/v1/me` refuses every token.
        let (status, _) = send(&self::router(&fx, false), "GET", "/api/v1/me", &jwt, None).await;
        assert_eq!(status, 401);
    }
}

// ---------------------------------------------------------------------------
// realtime.getConnectToken (port of parity p1-14 `60-realtime`, handler level)
// ---------------------------------------------------------------------------

const RT_SECRET: &str = "qa1-test-secret-qa1-test-secret-qa1-test";

fn with_realtime(mut fx: Fixture) -> Fixture {
    use chatbotx_rt::{NoPresence, NoRevokeRetry, Realtime, RealtimeDeps, RtConfig, SystemClock};
    let clock: Arc<dyn chatbotx_rt::Clock> = Arc::new(SystemClock);
    let realtime = Realtime::with_deps(
        RtConfig::new(RT_SECRET),
        RealtimeDeps {
            clock: Arc::clone(&clock),
            revoke_store: Arc::new(chatbotx_kv::DurableKv::new(Arc::clone(&fx.state.db), clock)),
            presence: Arc::new(NoPresence),
            revoke_retry: Arc::new(NoRevokeRetry),
        },
    );
    fx.state = fx.state.clone().with_realtime(realtime);
    fx
}

#[tokio::test]
async fn realtime_connect_token_needs_a_member() {
    let fx = with_realtime(seeded().await);
    let (status, body) = call(&fx, "realtime.getConnectToken", WS, &[], None, None).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["code"], "FORBIDDEN");
    assert_eq!(body["message"], "A signed-in member session is required");
}

#[tokio::test]
async fn realtime_connect_token_signs_the_member_scope_and_returns_the_socket_url() {
    let fx = with_realtime(seeded().await);
    let agent = member(&fx, AGENT, assigned_only()).await;
    let (status, body) = call_mobile(&fx, &agent, "realtime.getConnectToken", &[], None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["url"], "http://app/ws/");
    assert_eq!(body["socketUrl"], format!("ws://app/ws/rt/workspaces/{WS}"));
    let claims = chatbotx_rt::token::verify_member_connect_token(
        body["token"].as_str().unwrap(),
        &WS.to_string(),
        RT_SECRET,
        &chatbotx_rt::SystemClock,
    )
    .expect("valid token");
    assert_eq!(claims.user_id, AGENT.to_string());
    assert_eq!(claims.chat_scope, chatbotx_rt::token::ChatScope::Assigned);
    assert_eq!(claims.team_ids, vec!["50".to_string()]);
}

/// The input is validated before the scope check (oRPC `.input()` runs
/// before the handler): a bad body on an out-of-scope conversation is the
/// 422 `invalidRequestData`, a good one the 404 a missing conversation gets.
#[tokio::test]
async fn the_body_is_validated_before_the_scope_check() {
    let fx = seeded().await;
    let agent = member(&fx, AGENT, assigned_only()).await;
    let (status, body) = call_mobile(
        &fx,
        &agent,
        "conversations.assign",
        &[("id", "403")],
        Some(json!({ "assignedId": "nobody" })),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], "invalidRequestData", "{body}");
    let (status, _) = call_mobile(
        &fx,
        &agent,
        "conversations.assign",
        &[("id", "403")],
        Some(json!({ "assignedId": "u_1010" })),
    )
    .await;
    assert_eq!(status, 404);
}
