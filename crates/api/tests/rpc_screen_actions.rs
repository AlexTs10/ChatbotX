//! Port of `apps/builder/__tests__/inbound-webhook-actions.test.ts` (Q.D13a)
//! through `/rpc/actions/*`: the super-admin check of every inbound webhook
//! query and action, and the action results. Adapted: the mocked service
//! becomes a migrated database. Also the webhook, magic link and Google
//! Sheets screen reads and the inbox-teams page read (Q.D13a).

mod rpc_common;

use axum::http::StatusCode;
use rpc_common::{AGENT_TOKEN, OWNER_TOKEN, WS, action};
use serde_json::{Value, json};

async fn run(
    fx: &rpc_common::Fixture,
    name: &str,
    token: &str,
    args: Value,
) -> (StatusCode, Value) {
    action(
        fx,
        "actions",
        name,
        token,
        json!({ "args": args, "workspaceId": WS.to_string() }),
    )
    .await
}

#[tokio::test]
async fn inbound_webhook_actions_need_a_super_admin() {
    let fx = rpc_common::fixture().await;
    let ws = WS.to_string();
    for (name, args) in [
        (
            "listInboundWebhooks",
            json!([{ "workspaceId": ws, "page": 1, "perPage": 10 }]),
        ),
        (
            "createInboundWebhookAction",
            json!([ws, { "name": "Orders" }]),
        ),
        ("getInboundWebhookTokenAction", json!([ws, "1"])),
    ] {
        let (status, out) = run(&fx, name, AGENT_TOKEN, args).await;
        assert_ne!(status, StatusCode::OK, "{name}: {out}");
        assert!(
            out.to_string()
                .contains("You need to be a super admin to manage inbound webhooks"),
            "{name}: {out}"
        );
    }
}

#[tokio::test]
async fn inbound_webhook_actions_run_for_a_super_admin() {
    let fx = rpc_common::fixture().await;
    let ws = WS.to_string();

    let (status, out) = run(
        &fx,
        "createInboundWebhookAction",
        OWNER_TOKEN,
        json!([ws, { "name": " Orders " }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let id = out["json"]["id"].as_str().expect("id").to_owned();

    let (status, out) = run(
        &fx,
        "listInboundWebhooks",
        OWNER_TOKEN,
        json!([{ "workspaceId": ws, "page": 1, "perPage": 10 }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["data"][0]["name"], "Orders", "{out}");
    assert_eq!(out["json"]["pageCount"], 1, "{out}");

    let (_, out) = run(
        &fx,
        "findInboundWebhookForEditor",
        OWNER_TOKEN,
        json!([{ "workspaceId": ws, "id": id }]),
    )
    .await;
    let token = out["json"]["token"].as_str().expect("token").to_owned();
    assert!(token.starts_with("iwh_"), "{out}");

    let (_, out) = run(
        &fx,
        "getInboundWebhookTokenAction",
        OWNER_TOKEN,
        json!([ws, id]),
    )
    .await;
    assert_eq!(out["json"]["token"], token.as_str(), "{out}");

    let (status, out) = run(
        &fx,
        "regenerateInboundWebhookTokenAction",
        OWNER_TOKEN,
        json!([ws, id]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (_, out) = run(
        &fx,
        "getInboundWebhookTokenAction",
        OWNER_TOKEN,
        json!([ws, id]),
    )
    .await;
    assert_ne!(out["json"]["token"], token.as_str(), "{out}");

    let (status, out) = run(
        &fx,
        "updateInboundWebhookSettingsAction",
        OWNER_TOKEN,
        json!([ws, id, { "name": "Leads" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");

    // A schema failure is the action's validation errors.
    let (status, out) = run(
        &fx,
        "updateInboundWebhookSettingsAction",
        OWNER_TOKEN,
        json!([ws, id, { "name": "" }]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert!(out.to_string().contains("validationErrors"), "{out}");

    let (status, out) = run(
        &fx,
        "listInboundWebhookRequests",
        OWNER_TOKEN,
        json!([{ "workspaceId": ws, "inboundWebhookId": id, "page": 1, "perPage": 10 }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["data"], json!([]), "{out}");

    let (status, out) = run(
        &fx,
        "deleteInboundWebhooksAction",
        OWNER_TOKEN,
        json!([ws, { "ids": [id] }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (status, _) = run(
        &fx,
        "findInboundWebhookForEditor",
        OWNER_TOKEN,
        json!([{ "workspaceId": ws, "id": id }]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

async fn seed(fx: &rpc_common::Fixture, sql: &'static str) {
    fx.db()
        .write(move |tx| {
            tx.execute_batch(sql)?;
            Ok(())
        })
        .await
        .expect("seed");
}

#[tokio::test]
async fn webhook_screen_reads_need_a_super_admin_and_return_the_rows() {
    let fx = rpc_common::fixture().await;
    let ws = WS.to_string();
    seed(
        &fx,
        r#"INSERT INTO "Webhook" ("id", "workspaceId", "name", "url", "active") VALUES (9001, 201, 'Orders', '', 1);"#,
    )
    .await;
    let search =
        json!([{ "workspaceId": ws, "page": 1, "perPage": 10, "name": "", "folderId": null }]);
    // `resolveGuardedWorkspaceId(…, "superAdmin")`: a 404 for an agent.
    for (name, args) in [
        ("getWebhooks", search.clone()),
        ("findWebhook", json!([{ "workspaceId": ws, "id": "9001" }])),
        ("webhookStoreSource", json!([])),
    ] {
        let (status, out) = run(&fx, name, AGENT_TOKEN, args).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{name}: {out}");
    }
    let (status, out) = run(&fx, "getWebhooks", OWNER_TOKEN, search).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["data"][0]["name"], "Orders", "{out}");
    assert_eq!(out["json"]["data"][0]["conditions"], json!([]), "{out}");
    assert_eq!(out["json"]["pageCount"], 1, "{out}");
    let (_, out) = run(
        &fx,
        "findWebhook",
        OWNER_TOKEN,
        json!([{ "workspaceId": ws, "id": "9001" }]),
    )
    .await;
    assert_eq!(out["json"]["name"], "Orders", "{out}");
    let (status, out) = run(
        &fx,
        "findWebhook",
        OWNER_TOKEN,
        json!([{ "workspaceId": ws, "id": "9999" }]),
    )
    .await;
    assert_eq!(
        (status, out["json"].clone()),
        (StatusCode::OK, Value::Null),
        "{out}"
    );
    let (status, out) = run(&fx, "webhookStoreSource", OWNER_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert!(out["json"].is_string(), "{out}");
}

#[tokio::test]
async fn magic_link_and_sheets_reads_need_a_member_only() {
    let fx = rpc_common::fixture().await;
    let ws = WS.to_string();
    seed(
        &fx,
        r#"INSERT INTO "MagicLink" ("id", "workspaceId", "name", "url") VALUES (9101, 201, 'promo', 'https://example.com');
           INSERT INTO "Spreadsheet" ("id", "workspaceId", "name", "url", "spreadsheetId") VALUES (9201, 201, 'Leads', 'https://docs.google.com/spreadsheets/d/g/edit', 'g');"#,
    )
    .await;
    let (status, out) = run(
        &fx,
        "listMagicLinks",
        AGENT_TOKEN,
        json!([{ "workspaceId": ws, "page": 1, "perPage": 10, "keyword": null, "sort": [{ "id": "createdAt", "desc": true }] }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["data"][0]["name"], "promo", "{out}");
    let (_, out) = run(
        &fx,
        "findMagicLink",
        AGENT_TOKEN,
        json!([{ "workspaceId": ws, "id": "9101" }]),
    )
    .await;
    assert_eq!(
        out["json"],
        json!({ "id": "9101", "name": "promo" }),
        "{out}"
    );
    let (_, out) = run(
        &fx,
        "findMagicLink",
        AGENT_TOKEN,
        json!([{ "workspaceId": ws, "id": "9999" }]),
    )
    .await;
    assert_eq!(out["json"], Value::Null, "{out}");
    let (status, out) = run(
        &fx,
        "listSpreadsheets",
        AGENT_TOKEN,
        json!([{ "workspaceId": ws }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["data"][0]["name"], "Leads", "{out}");
    let (status, out) = run(&fx, "findIntegrationGoogleSheet", AGENT_TOKEN, json!([])).await;
    assert_eq!(
        (status, out["json"].clone()),
        (StatusCode::OK, Value::Null),
        "{out}"
    );
}

#[tokio::test]
async fn the_inbox_teams_page_read_carries_the_response_sla_policy() {
    let fx = rpc_common::fixture().await;
    let (status, out) = run(&fx, "workingHoursListPage", OWNER_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["responseSla"], Value::Null, "{out}");
    seed(
        &fx,
        r#"INSERT INTO "ResponseSlaPolicy" ("id", "workspaceId", "enabled", "firstResponseTargetSeconds") VALUES (9301, 201, 1, 600);"#,
    )
    .await;
    let (_, out) = run(&fx, "workingHoursListPage", OWNER_TOKEN, json!([])).await;
    assert_eq!(out["json"]["responseSla"]["enabled"], true, "{out}");
    assert_eq!(
        out["json"]["responseSla"]["firstResponseTargetSeconds"], 600,
        "{out}"
    );
}
