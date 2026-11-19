//! Port of the builder calling action tests (`calling-action-client-gates`,
//! `calls-page-access-gate`, `calls-page-invalid-filter-params`,
//! `list-whatsapp-calls-action` / `-query`, `list-call-filter-options-query`,
//! the `get-call-*` / `generate-call-ai-summary` / `list-call-summary-providers`
//! actions, `whatsapp-*-voip-*-action`, `whatsapp-request-call-permission-action`,
//! `whatsapp-resolve-outbound-call-mode-action`, `fix-whatsapp-calls-subscription-action`).
//! Adapted: the mocked services become the `/rpc/actions/*` router over a
//! migrated in-memory database; Meta is never reached (every case stops at a
//! gate or a local check first).
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::handlers::ApiState;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::rpc::{RpcState, rpc_router_with};
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings, sign_cookie_value};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const SECRET: &str = "whatsapp-calling-actions-secret";
const NOW_MS: i64 = 1_800_000_000_000;
const WS: i64 = 301;
const ADMIN: i64 = 11;
const AGENT: i64 = 12;
const ANALYST: i64 = 13;
const NOBODY: i64 = 14;
const OTHER_AGENT: i64 = 15;
/// Call 900 on conversation 70 (assigned to `OTHER_AGENT`); call 901 on 71 (assigned to `AGENT`).
const CALL_OTHER: i64 = 900;
const CALL_MINE: i64 = 901;

fn now_ms() -> i64 {
    NOW_MS
}

async fn router() -> axum::Router {
    let db = Db::open_in_memory().expect("open db");
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        let members = [
            (ADMIN, r#"{"superAdmin":true}"#),
            (AGENT, r#"{"onlyAssignedContacts":true}"#),
            (ANALYST, r#"{"analytics":true}"#),
            (NOBODY, r#"{"inbox":true}"#),
            (OTHER_AGENT, r#"{"contacts":true}"#),
        ];
        for (id, _) in members {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email", "name", "emailVerified") VALUES (?1, ?2, ?3, 1)"#,
                params![id, format!("u{id}@calls.test"), format!("User {id}")],
            )?;
            tx.execute(
                r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt") VALUES (?1, ?2, ?1, ?3)"#,
                params![id, format!("token{id}"), NOW_MS + 60_000],
            )?;
        }
        tx.execute(r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'ws', ?2)"#, params![WS, ADMIN])?;
        for (id, permissions) in members {
            let role = if id == ADMIN { "owner" } else { "agent" };
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions") VALUES (?1, ?2, ?1, ?3, ?4)"#,
                params![id, WS, role, permissions],
            )?;
        }
        let auth = json!({ "tokens": { "accessToken": "tok" }, "metadata": { "phoneNumber": { "id": "PH" }, "isManual": true } }).to_string();
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (61, ?1, 'Wa', 'whatsapp', 'wa-61')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId", "displayPhoneNumber")
               VALUES (62, ?1, 'PH', 'w', 'b', 'Wa', ?2, 61, '+1 415 555 0100')"#,
            params![auth, WS],
        )?;
        tx.execute(r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (63, ?1, 'Ada')"#, params![WS])?;
        tx.execute(
            r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
               VALUES (64, 63, 63, 61, 'whatsapp', 'direct', '4915112345678')"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "createdAt", "workspaceId", "contactId", "assignedUserId", "sourceId")
               VALUES (70, 0, ?1, 63, ?2, 'c70'), (71, 0, ?1, 63, ?3, 'c71')"#,
            params![WS, OTHER_AGENT, AGENT],
        )?;
        tx.execute(
            r#"INSERT INTO "WhatsappCall" ("id", "createdAt", "wacid", "direction", "status", "outcome", "answeredByUserId", "workspaceId", "inboxId", "contactInboxId", "conversationId")
               VALUES (?1, 1, 'wacid.other', 'userInitiated', 'completed', 'completed', ?3, ?4, 61, 64, 70),
                      (?2, 2, 'wacid.mine', 'userInitiated', 'failed', 'failed', NULL, ?4, 61, 64, 71)"#,
            params![CALL_OTHER, CALL_MINE, OTHER_AGENT, WS],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
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
    let mut api = ApiState::new(Arc::clone(&db), "http://app");
    // Meta is never reached: a closed port fails fast if a case regresses.
    api.graph_url = "http://127.0.0.1:9".to_string();
    rpc_router_with(RpcState {
        auth,
        api: Some(Arc::new(api)),
        realtime_secret: None,
    })
}

/// `POST /rpc/actions/<name>` as `user` with `args` after the workspace id.
async fn action(
    router: &axum::Router,
    name: &str,
    user: i64,
    args: Vec<Value>,
) -> (StatusCode, Value) {
    let mut all = vec![json!(WS.to_string())];
    all.extend(args);
    let request = Request::builder()
        .method("POST")
        .uri(format!("/rpc/actions/{name}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!(
                "better-auth.session_token={}",
                sign_cookie_value(&format!("token{user}"), SECRET)
            ),
        )
        .body(Body::from(
            json!({ "json": { "workspaceId": WS.to_string(), "args": all } }).to_string(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body["json"].clone())
}

fn id(value: i64) -> Value {
    json!(value.to_string())
}

#[tokio::test]
async fn calling_actions_refuse_a_member_without_contacts_access() {
    let r = router().await;
    for (name, input) in [
        (
            "answerWhatsappVoipCallAction",
            json!({ "whatsappCallId": id(CALL_MINE), "sdpAnswer": "v=0" }),
        ),
        (
            "initiateOutboundVoipCallAction",
            json!({ "conversationId": "71", "sdpOffer": "v=0" }),
        ),
        ("getPendingIncomingVoipCallAction", Value::Null),
        (
            "resolveOutboundCallModeAction",
            json!({ "conversationId": "71" }),
        ),
        (
            "outboundVoipTurnCredentialsAction",
            json!({ "attemptId": "a" }),
        ),
    ] {
        let (status, body) = action(&r, name, NOBODY, vec![input]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{name}: {body}");
        assert_eq!(
            body["message"],
            "You don't have access to calling in this workspace."
        );
    }
}

#[tokio::test]
async fn admin_calling_actions_need_a_workspace_super_admin() {
    let r = router().await;
    for name in [
        "fixWhatsappCallsSubscriptionAction",
        "updateWhatsappCallingSettingsAction",
        "updateWhatsappCallHoursAction",
    ] {
        let (status, body) = action(&r, name, OTHER_AGENT, vec![json!("62"), json!({})]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
        assert_eq!(
            body["message"],
            "You need to be a workspace super admin to do this."
        );
    }
}

#[tokio::test]
async fn fix_calls_subscription_explains_a_manually_connected_number() {
    let r = router().await;
    let (status, body) = action(
        &r,
        "fixWhatsappCallsSubscriptionAction",
        ADMIN,
        vec![json!("62")],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("This number was connected manually")
    );
}

#[tokio::test]
async fn calls_page_is_open_to_contacts_access_or_analytics_only() {
    let r = router().await;
    let (status, _) = action(&r, "callsPage", NOBODY, vec![json!({})]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = action(&r, "listWhatsappCallsAction", NOBODY, vec![json!({})]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body["message"],
        "You don't have access to call history in this workspace."
    );
    let (status, page) = action(&r, "callsPage", ANALYST, vec![json!({})]).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["data"].as_array().unwrap().len(), 2);
    assert_eq!(page["showAgentFilter"], true);
}

#[tokio::test]
async fn the_history_scope_follows_the_member_permissions() {
    let r = router().await;
    // contacts: only the calls the member answered or started.
    let (_, page) = action(&r, "listWhatsappCallsAction", OTHER_AGENT, vec![json!({})]).await;
    let ids: Vec<&str> = page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![CALL_OTHER.to_string()]);
    assert!(page["data"][0].get("recordingPath").is_some());
    // onlyAssignedContacts: also only on its assigned conversations; it took
    // part in no call (901 on its conversation was never answered).
    let (_, page) = action(&r, "listWhatsappCallsAction", AGENT, vec![json!({})]).await;
    assert_eq!(page["data"], json!([]));
    // The missed chip keeps the failed inbound call only.
    let (_, page) = action(
        &r,
        "listWhatsappCallsAction",
        ADMIN,
        vec![json!({ "activity": "missed" })],
    )
    .await;
    assert_eq!(page["data"].as_array().unwrap().len(), 1);
    assert_eq!(page["data"][0]["kind"], "missed");
}

#[tokio::test]
async fn calls_page_drops_a_non_admin_agent_filter_and_lists_options() {
    let r = router().await;
    let (status, page) = action(
        &r,
        "callsPage",
        AGENT,
        vec![json!({ "agentUserId": OTHER_AGENT.to_string() })],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["showAgentFilter"], false);
    assert_eq!(page["agentUserId"], Value::Null);
    assert_eq!(page["agentOptions"], json!([]));
    assert_eq!(page["inboxOptions"], json!([{ "id": "61", "name": "Wa" }]));
    let (_, page) = action(&r, "callsPage", ADMIN, vec![json!({})]).await;
    assert_eq!(page["agentOptions"].as_array().unwrap().len(), 5);
}

#[tokio::test]
async fn invalid_filter_params_are_rejected() {
    let r = router().await;
    for input in [json!({ "activity": "bogus" }), json!({ "inboxId": "abc" })] {
        let (status, _) = action(&r, "listWhatsappCallsAction", ADMIN, vec![input]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, body) = action(
        &r,
        "listWhatsappCallsAction",
        ADMIN,
        vec![json!({ "cursor": "%%%" })],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Whatsapp calls: cursor failed to decode");
}

#[tokio::test]
async fn call_artifacts_need_read_access_to_the_call() {
    let r = router().await;
    for name in [
        "getCallSummaryAction",
        "getCallTranscriptAction",
        "getCallRecordingUrlAction",
    ] {
        let (status, body) = action(
            &r,
            name,
            AGENT,
            vec![json!({ "whatsappCallId": id(CALL_OTHER) })],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{name}: {body}");
        assert_eq!(
            body["message"],
            "You don't have permission to view this call."
        );
    }
    let (status, body) = action(
        &r,
        "generateCallAiSummaryAction",
        AGENT,
        vec![json!({ "whatsappCallId": id(CALL_OTHER), "provider": "openai" })],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn call_artifacts_answer_for_an_allowed_reader() {
    let r = router().await;
    let (status, body) = action(
        &r,
        "getCallSummaryAction",
        ADMIN,
        vec![json!({ "whatsappCallId": id(CALL_OTHER) })],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], Value::Null);
    let (status, body) = action(
        &r,
        "getCallTranscriptAction",
        ADMIN,
        vec![json!({ "whatsappCallId": id(CALL_OTHER) })],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["segments"], json!([]));
    assert_eq!(body["hasSpeakers"], false);
    let (status, body) = action(
        &r,
        "getCallRecordingUrlAction",
        ADMIN,
        vec![json!({ "whatsappCallId": id(CALL_OTHER) })],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = action(&r, "listCallSummaryProvidersAction", AGENT, vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["providers"], json!([]));
}

#[tokio::test]
async fn voip_actions_without_a_live_call_control() {
    let r = router().await;
    let (status, body) = action(
        &r,
        "answerWhatsappVoipCallAction",
        AGENT,
        vec![json!({ "whatsappCallId": id(CALL_MINE), "sdpAnswer": "v=0" })],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "cannotAnswer");
    let (_, body) = action(
        &r,
        "heartbeatActiveVoipCallAction",
        AGENT,
        vec![json!({ "wacid": "wacid.mine" })],
    )
    .await;
    assert_eq!(body, json!({ "ok": false }));
    let (_, body) = action(
        &r,
        "getOutboundVoipAnswerAction",
        AGENT,
        vec![json!({ "whatsappCallId": id(CALL_MINE) })],
    )
    .await;
    assert_eq!(body, json!({ "sdp": null }));
    let (_, body) = action(&r, "getPendingIncomingVoipCallAction", AGENT, vec![]).await;
    assert_eq!(body["calls"], json!([]));
    let (status, body) = action(
        &r,
        "getWhatsappVoipTurnCredentialsAction",
        AGENT,
        vec![json!({ "whatsappCallId": id(CALL_MINE) })],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["message"],
        "You are not the agent assigned to this call"
    );
    let (status, body) = action(
        &r,
        "getWhatsappVoipTurnCredentialsAction",
        AGENT,
        vec![json!({ "whatsappCallId": "999999" })],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Call not found");
}

#[tokio::test]
async fn hangup_finds_only_calls_of_the_workspace_and_its_reserved_agent() {
    let r = router().await;
    let (status, body) = action(
        &r,
        "hangupWhatsappVoipCallAction",
        AGENT,
        vec![json!({ "whatsappCallId": "999999" })],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Call not found");
    // No control and not the owner of the ended call.
    let (status, body) = action(
        &r,
        "hangupWhatsappVoipCallAction",
        AGENT,
        vec![json!({ "whatsappCallId": id(CALL_OTHER) })],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["message"],
        "You are not the agent assigned to this call"
    );
    // The owner of an already-ended call: nothing left to end, still `hungUp`.
    let (status, body) = action(
        &r,
        "hangupWhatsappVoipCallAction",
        OTHER_AGENT,
        vec![json!({ "whatsappCallId": id(CALL_OTHER) })],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "hungUp": true }));
}

#[tokio::test]
async fn outbound_turn_credentials_are_scoped_to_the_caller() {
    let r = router().await;
    let (status, body) = action(
        &r,
        "outboundVoipTurnCredentialsAction",
        AGENT,
        vec![json!({ "attemptId": "attempt-1" })],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["iceServers"][0]["urls"],
        "stun:stun.l.google.com:19302"
    );
    let (status, _) = action(
        &r,
        "outboundVoipTurnCredentialsAction",
        AGENT,
        vec![json!({ "attemptId": "x".repeat(201) })],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn outbound_dial_stops_at_the_access_and_country_checks() {
    let r = router().await;
    // An assigned-only agent may not dial another agent's conversation.
    let (_, body) = action(
        &r,
        "initiateOutboundVoipCallAction",
        AGENT,
        vec![json!({ "conversationId": "70", "sdpOffer": "v=0" })],
    )
    .await;
    assert_eq!(body["outcome"], "callAccessDenied");
    // The business number is in the US: Meta refuses business-initiated calls there.
    let (_, body) = action(
        &r,
        "initiateOutboundVoipCallAction",
        AGENT,
        vec![json!({ "conversationId": "71", "sdpOffer": "v=0" })],
    )
    .await;
    assert_eq!(body["outcome"], "ineligibleNumber");
    let (_, body) = action(
        &r,
        "resolveOutboundCallModeAction",
        AGENT,
        vec![json!({ "conversationId": "70" })],
    )
    .await;
    assert_eq!(
        body,
        json!({ "mode": "none", "reason": "callAccessDenied" })
    );
    // Calling never mirrored as on: no Meta read.
    let (_, body) = action(
        &r,
        "resolveOutboundCallModeAction",
        AGENT,
        vec![json!({ "conversationId": "71" })],
    )
    .await;
    assert_eq!(
        body,
        json!({ "mode": "none", "reason": "callingNotEnabled" })
    );
}

#[tokio::test]
async fn request_call_permission_mirrors_the_outbound_dial_gate() {
    let r = router().await;
    let (status, body) = action(
        &r,
        "requestCallPermissionAction",
        AGENT,
        vec![json!("70"), json!({ "text": "May we call?" })],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body["message"],
        "You don't have permission to handle this call"
    );
    let (status, _) = action(
        &r,
        "requestCallPermissionAction",
        AGENT,
        vec![json!("71"), json!({ "text": "   " })],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
