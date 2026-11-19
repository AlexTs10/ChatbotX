//! The settings integration pages under `/rpc/actions/<name>` (Q.F2-settings):
//! the email-marketing connect / disconnect actions and page query, the Make
//! and ChatbotX MCP pages, `connectPaypalAction` and `listIntegrationSmtps`.
//! Ports of `apps/builder/__tests__/integration-actions-create-disconnect.test.ts`
//! (the original mocks the connection engine; here the rows are real).

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_business::connection::api_key;
use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_db::rusqlite::params;
use rpc_common::{AGENT_TOKEN, OUTSIDER_TOKEN, OWNER, OWNER_TOKEN, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// Every probe answers 200 with an ActiveCampaign / Drip shaped body.
struct Ok200;

impl Upstream for Ok200 {
    async fn send(&self, _request: UpstreamRequest) -> UpstreamReply {
        Ok((200, r#"{"accounts":[{"id":"1"}],"lists":[]}"#.into()))
    }
}

fn ws() -> String {
    WS.to_string()
}

async fn act(
    fx: &rpc_common::Fixture,
    token: &str,
    name: &str,
    args: Value,
) -> (StatusCode, Value) {
    let (status, body) = call(
        fx,
        "POST",
        &format!("/rpc/actions/{name}"),
        Some(token),
        json!({ "workspaceId": ws(), "args": args }),
    )
    .await;
    (status, body["json"].clone())
}

async fn audits(fx: &rpc_common::Fixture) -> Vec<(String, String)> {
    fx.db()
        .read(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "action", "detail" FROM "AuditLog" ORDER BY "createdAt", "id""#,
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
}

/// Port of: apps/builder/__tests__/integration-actions-create-disconnect.test.ts:43 "createDisconnectAction > routes through connectionService.disconnect when a Connection row exists"
#[tokio::test]
async fn create_disconnect_action_routes_through_connection_service_disconnect_when_a_connection_row_exists()
 {
    let fx = fixture().await;
    let provider = api_key::api_key_provider("activeCampaign").unwrap();
    let mut config = serde_json::Map::new();
    config.insert("apiUrl".into(), json!("https://acme.api-us1.com"));
    config.insert("apiKey".into(), json!("k"));
    let row = api_key::connect(fx.db(), &Ok200, WS, provider, &config, None, false)
        .await
        .expect("connect");
    let (_, page) = act(
        &fx,
        OWNER_TOKEN,
        "emailIntegrationPage",
        json!(["activeCampaign"]),
    )
    .await;
    assert_eq!(page, json!({ "connected": true }));

    let (status, out) = act(
        &fx,
        OWNER_TOKEN,
        "disconnectActiveCampaignAction",
        json!([ws()]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    // The integration row is deleted; its `Connection` row goes by cascade.
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "Connection" WHERE "id" = ?1"#,
            row.id
        )
        .await,
        json!(0)
    );
    let (_, page) = act(
        &fx,
        OWNER_TOKEN,
        "emailIntegrationPage",
        json!(["activeCampaign"]),
    )
    .await;
    assert_eq!(page, json!({ "connected": false }));
    assert!(audits(&fx).await.contains(&(
        "disconnect".to_string(),
        format!("disconnected the ActiveCampaign integration (#{})", row.id)
    )));
}

/// Port of: apps/builder/__tests__/integration-actions-create-disconnect.test.ts:67 "createDisconnectAction > falls back to the legacy service.disconnect when no Connection row exists yet (pre-backfill)"
#[tokio::test]
async fn create_disconnect_action_falls_back_to_the_legacy_service_disconnect_when_no_connection_row_exists_yet()
 {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (701, ?1, 'drip')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationDrip" ("id", "workspaceId", "integrationId", "auth") VALUES (702, ?1, 701, '{}')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (_, page) = act(&fx, OWNER_TOKEN, "emailIntegrationPage", json!(["drip"])).await;
    assert_eq!(page, json!({ "connected": true }));
    let (status, _) = act(&fx, OWNER_TOKEN, "disconnectDripAction", json!([ws()])).await;
    assert_eq!(status, StatusCode::OK);
    let (_, page) = act(&fx, OWNER_TOKEN, "emailIntegrationPage", json!(["drip"])).await;
    assert_eq!(page, json!({ "connected": false }));
}

/// Own: the connect schema runs before the provider probe (an invalid
/// `apiUrl` / an empty key is a 400 and no upstream call).
#[tokio::test]
async fn connect_actions_check_the_provider_schema_first() {
    let fx = fixture().await;
    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "connectActiveCampaignAction",
        json!([ws(), { "apiUrl": "not a url", "apiKey": "k" }]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "connectDripAction",
        json!([ws(), { "apiToken": "  " }]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = act(&fx, OWNER_TOKEN, "connectKlaviyoAction", json!([ws(), {}])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Own: the SSRF guard refuses a private ActiveCampaign `apiUrl` (no
/// credentials leave for an internal host).
#[tokio::test]
async fn connect_active_campaign_refuses_a_private_api_url() {
    let fx = fixture().await;
    let (status, out) = act(
        &fx,
        OWNER_TOKEN,
        "connectActiveCampaignAction",
        json!([ws(), { "apiUrl": "http://127.0.0.1:9", "apiKey": "k" }]),
    )
    .await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "{out}"
    );
    let (_, page) = act(
        &fx,
        OWNER_TOKEN,
        "emailIntegrationPage",
        json!(["activeCampaign"]),
    )
    .await;
    assert_eq!(page, json!({ "connected": false }));
}

/// Own: the page queries need a member (the `/rpc` workspace check).
#[tokio::test]
async fn page_queries_need_a_workspace_member() {
    let fx = fixture().await;
    let (status, _) = act(
        &fx,
        OUTSIDER_TOKEN,
        "emailIntegrationPage",
        json!(["klaviyo"]),
    )
    .await;
    assert!(status.is_client_error());
    let (status, _) = act(&fx, OUTSIDER_TOKEN, "chatbotxMcpPage", json!([])).await;
    assert!(status.is_client_error());
}

/// Own: Make without a platform credential, no MCP grants, no SMTP channels.
#[tokio::test]
async fn make_mcp_and_smtp_pages_answer_their_empty_state() {
    let fx = fixture().await;
    let (status, make) = act(&fx, OWNER_TOKEN, "makeIntegrationPage", json!([])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(make, json!({ "inviteUrl": null }));
    let (status, mcp) = act(&fx, OWNER_TOKEN, "chatbotxMcpPage", json!([])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mcp["grants"], json!([]));
    assert!(mcp["mcpUrl"].as_str().is_some());
    let (status, smtp) = act(
        &fx,
        OWNER_TOKEN,
        "listIntegrationSmtps",
        json!([{ "workspaceId": ws() }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(smtp, json!({ "data": [] }));
    let _ = OWNER;
}

/// Own: `connectPaypalAction` checks its schema, then `assertWorkspaceSuperAdmin`.
#[tokio::test]
async fn connect_paypal_needs_a_super_admin() {
    let fx = fixture().await;
    let input = json!([ws(), { "clientId": "id", "clientSecret": "secret" }]);
    let (status, out) = act(&fx, AGENT_TOKEN, "connectPaypalAction", input).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "connectPaypalAction",
        json!([ws(), { "clientId": "" }]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Zalo row 901 on inbox 911 of `WS`.
async fn seed_zalo(fx: &rpc_common::Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId")
                   VALUES (911, 'OA', 'zalo', 'oa-1', ?1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationZalo" ("id", "auth", "oaId", "name", "workspaceId", "inboxId")
                   VALUES (901, '{"tokens":{"accessToken":"secret"}}', 'oa-1', 'Zalo OA', ?1, 911)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

/// Port of: apps/builder/__tests__/update-mark-read-on-outbound.action.test.ts:36 "updateMarkReadOnOutboundAction > delegates to inboxService and returns the updated setting"
#[tokio::test]
async fn update_mark_read_on_outbound_action_delegates_to_inbox_service_and_returns_the_updated_setting()
 {
    let fx = fixture().await;
    seed_zalo(&fx).await;
    let (status, out) = act(
        &fx,
        OWNER_TOKEN,
        "updateMarkReadOnOutboundAction",
        json!([ws(), "911", { "enabled": true }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out, json!({ "markReadOnOutbound": true }));
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "markReadOnOutbound" FROM "Inbox" WHERE "id" = ?1"#,
            911
        )
        .await,
        json!(1)
    );
}

/// Port of: apps/builder/__tests__/update-mark-read-on-outbound.action.test.ts:52 "updateMarkReadOnOutboundAction > propagates a service rejection"
#[tokio::test]
async fn update_mark_read_on_outbound_action_propagates_a_service_rejection() {
    let fx = fixture().await;
    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "updateMarkReadOnOutboundAction",
        json!([ws(), "999", { "enabled": false }]),
    )
    .await;
    assert!(status.is_client_error());
}

/// Port of: apps/builder/src/features/integration-zalo/actions/__tests__/toggle-tag-sync.test.ts:148 "toggleZaloTagSyncAction > enabled: true > calls zaloIntegrationService.updateTagSync with a truthy enabled flag"
#[tokio::test]
async fn toggle_zalo_tag_sync_action_enabled_true_calls_zalo_integration_service_update_tag_sync_with_a_truthy_enabled_flag()
 {
    let fx = fixture().await;
    seed_zalo(&fx).await;
    let (status, out) = act(
        &fx,
        OWNER_TOKEN,
        "toggleZaloTagSyncAction",
        json!([ws(), "901", { "enabled": true }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (_, list) = act(&fx, OWNER_TOKEN, "listIntegrationZalo", json!([])).await;
    assert!(list["data"][0]["syncTagEnabledAt"].is_string());
}

/// Port of: apps/builder/src/features/integration-zalo/actions/__tests__/toggle-tag-sync.test.ts:163 "toggleZaloTagSyncAction > enabled: false > calls zaloIntegrationService.updateTagSync with a falsy enabled flag"
#[tokio::test]
async fn toggle_zalo_tag_sync_action_enabled_false_calls_zalo_integration_service_update_tag_sync_with_a_falsy_enabled_flag()
 {
    let fx = fixture().await;
    seed_zalo(&fx).await;
    for enabled in [true, false] {
        let (status, _) = act(
            &fx,
            OWNER_TOKEN,
            "toggleZaloTagSyncAction",
            json!([ws(), "901", { "enabled": enabled }]),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (_, list) = act(&fx, OWNER_TOKEN, "listIntegrationZalo", json!([])).await;
    assert_eq!(list["data"][0]["syncTagEnabledAt"], Value::Null);
}

/// Port of: apps/builder/src/features/integration-zalo/actions/__tests__/toggle-tag-sync.test.ts:179 "toggleZaloTagSyncAction > unknown integration id > surfaces the service's notFound error as serverError"
#[tokio::test]
async fn toggle_zalo_tag_sync_action_unknown_integration_id_surfaces_the_service_s_not_found_error_as_server_error()
 {
    let fx = fixture().await;
    let (status, body) = call(
        &fx,
        "POST",
        "/rpc/actions/toggleZaloTagSyncAction",
        Some(OWNER_TOKEN),
        json!({ "workspaceId": ws(), "args": [ws(), "999", { "enabled": true }] }),
    )
    .await;
    assert!(status.is_client_error());
    assert!(
        body.to_string().contains("Zalo channel not found"),
        "{body}"
    );
}

/// Own: the channel lists answer no credentials; an outsider is refused; the
/// Zalo disconnect removes the row and audits.
#[tokio::test]
async fn zalo_list_hides_credentials_and_disconnect_audits() {
    let fx = fixture().await;
    seed_zalo(&fx).await;
    let (status, _) = act(&fx, OUTSIDER_TOKEN, "listIntegrationZalo", json!([])).await;
    assert!(status.is_client_error());
    let (_, list) = act(&fx, OWNER_TOKEN, "listIntegrationZalo", json!([])).await;
    assert_eq!(list["data"][0]["id"], json!("901"));
    assert!(!list.to_string().contains("secret"));
    let (_, tiktoks) = act(&fx, OWNER_TOKEN, "listIntegrationTiktoks", json!([])).await;
    assert_eq!(tiktoks, json!({ "data": [] }));
    let (_, cred) = act(&fx, OWNER_TOKEN, "channelCredentialPage", json!(["zalo"])).await;
    assert_eq!(cred, json!({ "clientId": null }));
    let (status, _) = act(&fx, OWNER_TOKEN, "channelCredentialPage", json!(["stripe"])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, out) = act(
        &fx,
        OWNER_TOKEN,
        "disconnectZaloAction",
        json!([ws(), "901"]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert!(audits(&fx).await.contains(&(
        "disconnect".to_string(),
        "disconnected the Zalo channel (#901)".to_string()
    )));
    let (_, list) = act(&fx, OWNER_TOKEN, "listIntegrationZalo", json!([])).await;
    assert_eq!(list, json!({ "data": [] }));
    // A Zalo row without a `Connection` cannot start the signed reconnect.
    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "reconnectZaloAction",
        json!([ws(), "901"]),
    )
    .await;
    assert!(status.is_client_error());
}

/// Own: the AI Router connections need a super admin (404 otherwise); the
/// list never carries the credential. The Facebook Ads page loads for a
/// member and shows nothing connected; its connect / disconnect need a super
/// admin.
#[tokio::test]
async fn decision_connections_and_facebook_ads_guards() {
    let fx = fixture().await;
    let body = json!({
        "credential": "sk-secret",
        "modelCatalog": ["model-a"],
        "name": "Router",
        "providerKind": "typesafe",
    });
    let (status, _) = act(
        &fx,
        AGENT_TOKEN,
        "createDecisionConnectionAction",
        json!([ws(), body]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, created) = act(
        &fx,
        OWNER_TOKEN,
        "createDecisionConnectionAction",
        json!([ws(), body]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "createDecisionConnectionAction",
        json!([ws(), { "name": "x", "modelCatalog": ["m"], "providerKind": "typesafe", "extra": 1 }]),
    )
    .await;
    assert!(status.is_client_error(), "strict schema: {status}");
    let (status, page) = act(&fx, OWNER_TOKEN, "decisionConnectionsPage", json!([ws()])).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["connections"][0]["name"], json!("Router"));
    assert!(!page.to_string().contains("sk-secret"));
    let id = created["id"].as_str().unwrap().to_string();
    let (status, toggled) = act(
        &fx,
        OWNER_TOKEN,
        "toggleDecisionConnectionAction",
        json!([ws(), { "id": id, "enabled": false }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{toggled}");
    assert_eq!(toggled["status"], json!("disabled"));

    let (status, page) = act(&fx, AGENT_TOKEN, "facebookAdsSettingsPage", json!([ws()])).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page, json!({ "integration": null }));
    for name in ["connectFacebookAds", "disconnectFacebookAdsAction"] {
        let (status, _) = act(&fx, AGENT_TOKEN, name, json!([ws()])).await;
        assert!(status.is_client_error(), "{name}: {status}");
    }
    // Nothing connected.
    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "disconnectFacebookAdsAction",
        json!([ws()]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
