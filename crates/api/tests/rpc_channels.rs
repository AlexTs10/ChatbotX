//! The channel settings screens under `/rpc/actions` (Q.D12-channels,
//! `rpc_procedures/channels.rs`). Ports the action logic of
//! `apps/builder/src/features/integration-{whatsapp,messenger,webchat}` and
//! the `whatsapps/[id]`, `messengers/[id]`, `webchats/[id]` page loaders:
//! the TS tests mock Meta and the services; here a local Graph server
//! (`ApiState.graph_url`) answers and the stored rows are checked. Also
//! `coexist-messenger-api.test.ts` (the Messenger / Instagram coexist
//! procedures).

mod rpc_common;

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::routing::get;
use chatbotx_db::rusqlite::params;
use rpc_common::{AGENT_TOKEN, Fixture, OWNER, OWNER_TOKEN, WS, action, call, fixture, scalar};
use serde_json::{Value, json};

const OTHER_WS: i64 = 202;

async fn seed(fx: &Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'Other', ?2)"#,
                params![OTHER_WS, OWNER],
            )?;
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions") VALUES (403, ?1, ?2, 'owner', '{}')"#,
                params![OTHER_WS, OWNER],
            )?;
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId", "markReadOnOutbound")
                   VALUES (6, ?1, 'Wa', 'whatsapp', 'wa-6', 0), (7, ?1, 'Fb', 'messenger', 'fb-7', 1),
                          (8, ?2, 'Fb2', 'messenger', 'fb-8', 0), (9, ?1, 'Fb3', 'messenger', 'fb-9', 0),
                          (10, ?1, 'Site', 'webchat', 'wc-10', 0)"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "displayPhoneNumber", "registrationStatus", "workspaceId", "inboxId")
                   VALUES (101, '{"tokens":{"accessToken":"wa-secret"},"metadata":{"businessId":"biz-1"}}',
                           '100000000000001', '200000000123456', 'biz-1', 'Wa', '+1 555', 'pending_verification', ?1, 6)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId")
                   VALUES (102, '{"tokens":{"accessToken":"fb-secret"}}', 'page-1', 'Main', '[]', '[]', '[]', ?1, 7),
                          (103, '{}', 'page-2', 'Other page', '[]', '[]', '[]', ?2, 8),
                          (104, '{}', 'page-3', 'Third page', '[]', '[]', '[]', ?1, 9)"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "MessengerMessageTemplate" ("id", "name", "integrationMessengerId", "sourceId", "language", "category", "status", "components")
                   VALUES (11, 'welcome', 102, 's-11', 'en', 'UTILITY', 'APPROVED', '[]'),
                          (12, 'promo_a', 102, 's-12', 'en', 'MARKETING', 'APPROVED', '[]'),
                          (13, 'promo_b', 102, 's-13', 'en', 'MARKETING', 'PENDING', '[]')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWebchat" ("id", "auth", "name", "authorizedDomains", "conversationStarters", "persistentMenus", "brandColor", "workspaceId", "inboxId")
                   VALUES (105, '{}', 'Site', '[]', '[]', '[]', '#000000', ?1, 10)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name", "active") VALUES (61, ?1, 'On', 1), (62, ?1, 'Off', 0)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

async fn act(fx: &Fixture, token: &str, name: &str, args: Value) -> (StatusCode, Value) {
    let (status, body) = action(
        fx,
        "actions",
        name,
        token,
        json!({ "workspaceId": WS.to_string(), "args": args }),
    )
    .await;
    (status, body["json"].clone())
}

fn ws() -> String {
    WS.to_string()
}

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

async fn graph_get(State(seen): State<Seen>, uri: Uri) -> axum::response::Response {
    seen.lock().unwrap().push(format!("GET {}", uri.path()));
    match uri.path() {
        "/v23.0/100000000000001" => reply(
            200,
            json!({ "display_phone_number": "+1 555 0100", "verified_name": "Acme", "conversational_automation": { "prompts": ["Hi"] } }),
        ),
        "/v23.0/200000000123456/phone_numbers" => reply(
            200,
            json!({ "data": [{ "id": "100000000000001", "code_verification_status": "VERIFIED", "status": "PENDING", "platform_type": "NOT_APPLICABLE" }] }),
        ),
        "/v23.0/100000000000001/routing_roles" => reply(
            400,
            json!({ "error": { "message": "Conversation routing is not enabled for this number", "code": 100 } }),
        ),
        _ => reply(
            400,
            json!({ "error": { "message": "WABA unavailable", "code": 1 } }),
        ),
    }
}

async fn graph_post(State(seen): State<Seen>, uri: Uri, body: String) -> axum::response::Response {
    seen.lock()
        .unwrap()
        .push(format!("POST {} {body}", uri.path()));
    if body.contains("\"code\":\"000000\"") {
        return reply(
            400,
            json!({ "error": { "message": "Invalid code", "code": 136_025, "error_user_msg": "The code is wrong." } }),
        );
    }
    if body.contains("VOICE") {
        return reply(
            400,
            json!({ "error": { "message": "Voice unavailable", "code": 1 } }),
        );
    }
    reply(200, json!({ "success": true }))
}

/// Points `graph_url` at a local Graph server; the calls it saw.
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

#[tokio::test]
async fn channel_pages_need_super_admin_and_never_return_tokens() {
    let mut fx = fixture().await;
    seed(&fx).await;
    mock_graph(&mut fx).await;
    for (name, input) in [
        (
            "whatsappChannelPage",
            json!({ "id": "101", "tab": "links" }),
        ),
        ("messengerChannelPage", json!({ "id": "102" })),
        ("messengerTemplatesPage", json!({ "id": "102" })),
    ] {
        let (status, body) = act(&fx, AGENT_TOKEN, name, json!([ws(), input])).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{name} {body}");
    }

    let (status, links) = act(
        &fx,
        OWNER_TOKEN,
        "whatsappChannelPage",
        json!([ws(), { "id": "101", "tab": "links" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{links}");
    assert_eq!(
        links["integrationWhatsapp"],
        json!({ "id": "101", "workspaceId": ws(), "businessId": "biz-1", "wabaId": "200000000123456" })
    );
    assert!(links["urls"]["analytics"].as_str().is_some());

    let (_, settings) = act(
        &fx,
        OWNER_TOKEN,
        "whatsappChannelPage",
        json!([ws(), { "id": "101", "tab": "settings" }]),
    )
    .await;
    assert_eq!(settings["automation"]["prompts"], json!(["Hi"]));
    assert_eq!(settings["pixRedirectEnabled"], json!(false));

    let (_, health) = act(
        &fx,
        OWNER_TOKEN,
        "whatsappChannelPage",
        json!([ws(), { "id": "101", "tab": "accountHealths" }]),
    )
    .await;
    assert_eq!(health["displayPhoneNumber"], "+1 555 0100");
    // The WABA call fails: `{}`, the page still loads.
    assert_eq!(health["waba"], json!({}));
    assert_eq!(health["registrationStatus"], "pending_verification");
    assert!(
        health["businessManagerUrl"]
            .as_str()
            .unwrap()
            .ends_with("business_id=biz-1&waba_id=200000000123456")
    );

    let (status, page) = act(
        &fx,
        OWNER_TOKEN,
        "messengerChannelPage",
        json!([ws(), { "id": "102" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page["integrationMessenger"].get("auth").is_none());
    assert_eq!(page["markReadOnOutbound"], json!(true));
    assert_eq!(page["aiHandover"]["enabled"], json!(false));
    for body in [&links, &settings, &health, &page] {
        let text = body.to_string();
        assert!(!text.contains("secret"), "{text}");
    }

    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "messengerChannelPage",
        json!([ws(), { "id": "103" }]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn messenger_templates_page_filters_pages_and_lists_clone_targets() {
    let fx = fixture().await;
    seed(&fx).await;
    let (status, page) = act(
        &fx,
        OWNER_TOKEN,
        "messengerTemplatesPage",
        json!([ws(), { "id": "102", "name": "promo", "page": 1, "perPage": 1 }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let ids: Vec<&Value> = page["templates"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| &t["id"])
        .collect();
    assert_eq!(ids, [&json!("13")]);
    assert_eq!(page["templates"]["pageCount"], 2);
    // Every admin workspace's page but the source page, by name.
    assert_eq!(
        page["channels"],
        json!([{ "id": "103", "name": "Other page" }, { "id": "104", "name": "Third page" }])
    );
}

#[tokio::test]
async fn messenger_template_actions_check_the_page_and_clone_targets() {
    let fx = fixture().await;
    seed(&fx).await;
    let (status, body) = act(
        &fx,
        OWNER_TOKEN,
        "deleteMessengerMessageTemplateAction",
        json!([ws(), "103", "11"]),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let (status, body) = act(
        &fx,
        OWNER_TOKEN,
        "deleteMessengerMessageTemplateAction",
        json!([ws(), "102", "11"]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT count(*) FROM "MessengerMessageTemplate" WHERE "id" = ?1"#,
            11
        )
        .await,
        json!(0)
    );

    // The source page itself is not a target (`pageId` is unique, so a
    // second row of the same page cannot exist).
    let (status, body) = act(
        &fx,
        OWNER_TOKEN,
        "cloneMessengerMessageTemplateAction",
        json!([ws(), "102", "12", { "targetIntegrationMessengerIds": ["102"] }]),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "cloneMessengerMessageTemplateAction",
        json!([ws(), "102", "12", { "targetIntegrationMessengerIds": [] }]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn whatsapp_settings_actions_need_super_admin_and_an_active_flow() {
    let fx = fixture().await;
    seed(&fx).await;
    let (status, body) = act(
        &fx,
        AGENT_TOKEN,
        "updatePixRedirectAction",
        json!([ws(), "101", { "pixRedirectEnabled": true }]),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{body}");
    let (status, body) = act(
        &fx,
        OWNER_TOKEN,
        "updatePixRedirectAction",
        json!([ws(), "101", { "pixRedirectEnabled": true }]),
    )
    .await;
    assert_eq!(
        (status, &body),
        (StatusCode::OK, &json!({ "pixRedirectEnabled": true }))
    );

    for (name, id) in [
        ("integration-whatsapp.updateHandoverResumeFlowAction", "101"),
        (
            "integration-messenger.updateHandoverResumeFlowAction",
            "102",
        ),
    ] {
        let (status, body) = act(
            &fx,
            OWNER_TOKEN,
            name,
            json!([ws(), id, { "handoverResumeFlowId": "62" }]),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{name}");
        assert_eq!(
            body["message"],
            "The selected flow was not found or is not active"
        );
        let (status, body) = act(
            &fx,
            OWNER_TOKEN,
            name,
            json!([ws(), id, { "handoverResumeFlowId": "61" }]),
        )
        .await;
        assert_eq!(
            (status, &body),
            (StatusCode::OK, &json!({ "handoverResumeFlowId": "61" }))
        );
    }
    let (status, body) = act(
        &fx,
        OWNER_TOKEN,
        "integration-messenger.updateHandoverResumeFlowAction",
        json!([ws(), "103", { "handoverResumeFlowId": null }]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["message"], "Messenger page not found");
}

#[tokio::test]
async fn verification_code_has_a_cooldown_and_a_refusal_frees_it() {
    let mut fx = fixture().await;
    seed(&fx).await;
    let seen = mock_graph(&mut fx).await;
    let request = |method: &str| json!([ws(), { "integrationId": "101", "codeMethod": method }]);

    let (status, body) = act(
        &fx,
        OWNER_TOKEN,
        "requestWhatsappVerificationCodeAction",
        request("VOICE"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "verificationCodeRequestedAt" FROM "IntegrationWhatsapp" WHERE "id" = ?1"#,
            101
        )
        .await,
        Value::Null
    );

    let (status, body) = act(
        &fx,
        OWNER_TOKEN,
        "requestWhatsappVerificationCodeAction",
        request("SMS"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "sent");
    let (_, body) = act(
        &fx,
        OWNER_TOKEN,
        "requestWhatsappVerificationCodeAction",
        request("SMS"),
    )
    .await;
    assert_eq!(body["status"], "cooldown");
    assert!(body["remainingSeconds"].as_i64().unwrap() > 0);
    let sent = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.contains("/request_code"))
        .count();
    assert_eq!(sent, 2);
}

#[tokio::test]
async fn verifying_the_code_registers_the_number() {
    let mut fx = fixture().await;
    seed(&fx).await;
    let seen = mock_graph(&mut fx).await;
    let verify = |code: &str| json!([ws(), { "integrationId": "101", "code": code }]);

    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "verifyWhatsappPhoneCodeAction",
        verify("12a456"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = act(
        &fx,
        OWNER_TOKEN,
        "verifyWhatsappPhoneCodeAction",
        verify("000000"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "The code is wrong.");

    let (status, body) = act(
        &fx,
        OWNER_TOKEN,
        "verifyWhatsappPhoneCodeAction",
        verify(" 123456 "),
    )
    .await;
    assert_eq!(
        (status, &body),
        (StatusCode::OK, &json!({ "status": "registered" })),
        "{body}"
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "registrationStatus" FROM "IntegrationWhatsapp" WHERE "id" = ?1"#,
            101
        )
        .await,
        json!("registered")
    );
    // The pin is the last 6 digits of phoneNumberId + wabaId.
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|c| c.contains("/register") && c.contains("\"pin\":\"123457\""))
    );
}

#[tokio::test]
async fn routing_roles_sync_reads_a_disabled_routing_as_empty() {
    let mut fx = fixture().await;
    seed(&fx).await;
    mock_graph(&mut fx).await;
    let input = json!({ "workspaceId": ws(), "integrationWhatsappId": "101" });
    let (status, body) = call(
        &fx,
        "POST",
        "/rpc/integrationWhatsappAPIs/syncWhatsappRoutingRolesAPI",
        Some(OWNER_TOKEN),
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["json"]["enabled"], json!(false));
    assert_eq!(body["json"]["roles"], json!([]));
    let (_, stored) = call(
        &fx,
        "POST",
        "/rpc/integrationWhatsappAPIs/getWhatsappRoutingRolesByInboxAPI",
        Some(OWNER_TOKEN),
        json!({ "workspaceId": ws(), "inboxId": "6" }),
    )
    .await;
    assert_eq!(stored["json"]["enabled"], json!(false));
}

#[tokio::test]
async fn webchat_update_is_super_admin_only_and_sets_mark_read() {
    let fx = fixture().await;
    seed(&fx).await;
    let id = 105;
    let update = json!([ws(), id.to_string(), {
        "name": "Renamed",
        "authorizedDomains": [{ "value": "example.com" }],
        "markReadOnOutbound": true,
    }]);
    let (status, body) = act(&fx, AGENT_TOKEN, "updateWebchatAction", update.clone()).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let (status, body) = act(&fx, OWNER_TOKEN, "updateWebchatAction", update).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, page) = act(
        &fx,
        OWNER_TOKEN,
        "webchatChannelPage",
        json!([ws(), { "id": id.to_string() }]),
    )
    .await;
    assert_eq!(page["integrationWebchat"]["name"], "Renamed");
    assert_eq!(
        page["integrationWebchat"]["authorizedDomains"],
        json!(["example.com"])
    );
    assert_eq!(page["markReadOnOutbound"], json!(true));
}

#[tokio::test]
async fn messenger_coexist_reuses_the_live_run_and_disable_fails_it() {
    let fx = fixture().await;
    seed(&fx).await;
    let set = |id: &str, enabled: bool| json!({ "workspaceId": ws(), "integrationId": id, "enabled": enabled, "aiReadsSyncedHistory": true });
    let path = "/rpc/integrationMessengerAPIs/setCoexistMessengerAPI";
    let (status, first) = call(&fx, "POST", path, Some(OWNER_TOKEN), set("102", true)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["json"]["success"], json!(true));
    let run_id = first["json"]["runId"].as_str().expect("runId").to_owned();
    let (_, again) = call(&fx, "POST", path, Some(OWNER_TOKEN), set("102", true)).await;
    assert_eq!(again["json"]["runId"], json!(run_id));
    assert_eq!(
        scalar(&fx, r#"SELECT "coexistEnabled" || ':' || "coexistAiReadsSyncedHistory" FROM "IntegrationMessenger" WHERE "id" = ?1"#, 102).await,
        json!("1:1")
    );
    // `aiReadsSyncedHistory` defaults to false.
    let (_, plain) = call(
        &fx,
        "POST",
        path,
        Some(OWNER_TOKEN),
        json!({ "workspaceId": ws(), "integrationId": "104", "enabled": true }),
    )
    .await;
    assert_eq!(plain["json"]["success"], json!(true));
    assert_eq!(
        scalar(&fx, r#"SELECT "coexistEnabled" || ':' || "coexistAiReadsSyncedHistory" FROM "IntegrationMessenger" WHERE "id" = ?1"#, 104).await,
        json!("1:0")
    );
    // A page of another workspace is `not_found`.
    let (_, other) = call(&fx, "POST", path, Some(OWNER_TOKEN), set("103", true)).await;
    assert_eq!(
        other["json"],
        json!({ "success": false, "reason": "not_found" })
    );
    let (_, off) = call(&fx, "POST", path, Some(OWNER_TOKEN), set("102", false)).await;
    assert_eq!(off["json"], json!({ "success": true }));
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "status" || ':' || "currentError" FROM "CoexistSyncRun" WHERE "id" = ?1"#,
            run_id.parse().unwrap()
        )
        .await,
        json!("failed:Coexist disabled")
    );
    // Instagram takes the same path; there is no row 102 there.
    let (_, ig) = call(
        &fx,
        "POST",
        "/rpc/integrationInstagramAPIs/setCoexistInstagramAPI",
        Some(OWNER_TOKEN),
        set("102", true),
    )
    .await;
    assert_eq!(ig["json"]["reason"], json!("not_found"));
}
