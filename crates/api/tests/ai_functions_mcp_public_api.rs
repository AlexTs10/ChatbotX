//! Ports of `apps/builder/__tests__/ai-functions-public-api.test.ts` and
//! `ai-mcp-servers-public-api.test.ts` (Q.D6). Adapted: the captured oRPC
//! handlers become `handlers::dispatch` calls over a real migrated database;
//! "the service was called with" becomes the response and the stored rows.
//! The TS inputs (`{ name }` only) are completed to valid requests: the Rust
//! handler runs the input schema the mocked chain skipped.

mod handlers_common;

use chatbotx_api::routes::scopes_of_module;
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "AIFunction" ("id", "name", "dataCollect", "workspaceId") VALUES (7, 'Lookup order', '[]', ?1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "AIMCPServer" ("id", "name", "url", "auth", "availableTools", "selectedTools", "workspaceId")
                   VALUES (5, 'Docs MCP', 'https://docs.example.com/mcp', '{"type":"token","token":"s3cret"}', '{}', '[]', ?1)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

fn function_body(name: &str) -> Value {
    json!({ "name": name, "dataCollect": [] })
}

fn server_body(name: &str) -> Value {
    json!({
        "name": name,
        "url": "https://docs.example.com/mcp",
        "auth": { "type": "header", "headers": [{ "header": "X-Key", "value": "hidden" }] },
        "availableTools": { "search": { "description": "Search" } },
        "selectedTools": ["search"],
    })
}

// ---- ai-functions-public-api.test.ts ----

#[tokio::test]
async fn registers_the_ai_functions_public_router_under_the_automation_scope() {
    assert_eq!(
        scopes_of_module("features/ai-functions/api/public"),
        vec!["automation"]
    );
}

#[tokio::test]
async fn get_v1_ai_functions_delegates_to_ai_function_service_list_ai_functions() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "aiFunctions.list",
        WS,
        &[],
        Some("page=1&perPage=50"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pageCount"], json!(1));
    assert_eq!(body["data"][0]["id"], json!("7"));
    let (_, other) = call(&f, "aiFunctions.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other, json!({ "data": [], "pageCount": 0 }));
}

#[tokio::test]
async fn get_v1_ai_functions_id_delegates_to_ai_function_service_find_by() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(&f, "aiFunctions.get", WS, &[("id", "7")], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["workspaceId"], json!(WS.to_string()));
    let (status, _) = call(&f, "aiFunctions.get", OTHER_WS, &[("id", "7")], None, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn get_v1_ai_functions_id_throws_not_found_when_the_function_does_not_exist() {
    let f = fixture().await;
    let (status, body) = call(&f, "aiFunctions.get", WS, &[("id", "99")], None, None).await;
    assert_eq!(status, 404);
    assert_eq!(body["message"], json!("AI function not found"));
}

#[tokio::test]
async fn post_v1_ai_functions_delegates_to_ai_function_service_create_and_returns_the_created_row()
{
    let f = fixture().await;
    let (status, body) = call(
        &f,
        "aiFunctions.create",
        WS,
        &[],
        None,
        Some(function_body("Lookup order")),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["name"], json!("Lookup order"));
    let id: i64 = body["id"].as_str().expect("id").parse().expect("number");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "AIFunction" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!(WS))
    );
}

#[tokio::test]
async fn put_v1_ai_functions_id_delegates_to_ai_function_service_update_ai_function_with_no_translator_and_returns_its_result()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "aiFunctions.update",
        WS,
        &[("id", "7")],
        None,
        Some(function_body("Renamed")),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (body["id"].clone(), body["name"].clone()),
        (json!("7"), json!("Renamed"))
    );
}

#[tokio::test]
async fn delete_v1_ai_functions_id_delegates_to_ai_function_service_delete_ai_function_with_no_translator()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, _) = call(&f, "aiFunctions.delete", WS, &[("id", "7")], None, None).await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(&f, r#"SELECT "id" FROM "AIFunction" WHERE "id" = ?1"#, 7).await,
        None
    );
}

// ---- ai-mcp-servers-public-api.test.ts ----

#[tokio::test]
async fn registers_the_ai_mcp_servers_public_router_under_the_automation_scope() {
    assert_eq!(
        scopes_of_module("features/ai-mcp-servers/api/public"),
        vec!["automation"]
    );
}

#[tokio::test]
async fn get_v1_ai_mcp_servers_delegates_to_ai_mcp_server_service_list_ai_mcp_servers() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(&f, "aiMcpServers.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"][0]["auth"], json!({ "type": "token" }));
    assert!(!body.to_string().contains("s3cret"));
}

#[tokio::test]
async fn get_v1_ai_mcp_servers_id_delegates_to_ai_mcp_server_service_find_by() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(&f, "aiMcpServers.get", WS, &[("id", "5")], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["name"], json!("Docs MCP"));
}

#[tokio::test]
async fn get_v1_ai_mcp_servers_id_throws_not_found_when_the_mcp_server_does_not_exist() {
    let f = fixture().await;
    let (status, body) = call(&f, "aiMcpServers.get", WS, &[("id", "99")], None, None).await;
    assert_eq!(status, 404);
    assert_eq!(body["message"], json!("AI MCP server not found"));
}

#[tokio::test]
async fn post_v1_ai_mcp_servers_delegates_to_ai_mcp_server_service_create_and_returns_the_created_row()
 {
    let f = fixture().await;
    let (status, body) = call(
        &f,
        "aiMcpServers.create",
        WS,
        &[],
        None,
        Some(server_body("Docs MCP")),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(
        body["auth"],
        json!({ "type": "header", "headers": [{ "header": "X-Key" }] })
    );
    assert!(!body.to_string().contains("hidden"));
}

#[tokio::test]
async fn put_v1_ai_mcp_servers_id_delegates_to_ai_mcp_server_service_update_and_returns_its_result()
{
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "aiMcpServers.update",
        WS,
        &[("id", "5")],
        None,
        Some(server_body("Renamed")),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (body["id"].clone(), body["name"].clone()),
        (json!("5"), json!("Renamed"))
    );
}

#[tokio::test]
async fn put_v1_ai_mcp_servers_id_throws_not_found_when_the_mcp_server_does_not_exist() {
    let f = fixture().await;
    let (status, body) = call(
        &f,
        "aiMcpServers.update",
        WS,
        &[("id", "99")],
        None,
        Some(server_body("Renamed")),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["message"], json!("AI MCP server not found"));
}

#[tokio::test]
async fn delete_v1_ai_mcp_servers_delegates_to_ai_mcp_server_service_delete() {
    let f = fixture().await;
    seed(&f).await;
    let (status, _) = call(&f, "aiMcpServers.delete", WS, &[("id", "5")], None, None).await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(&f, r#"SELECT "id" FROM "AIMCPServer" WHERE "id" = ?1"#, 5).await,
        None
    );
}

#[tokio::test]
async fn delete_v1_ai_mcp_servers_throws_not_found_when_the_mcp_server_does_not_exist() {
    let f = fixture().await;
    let (status, body) = call(&f, "aiMcpServers.delete", WS, &[("id", "99")], None, None).await;
    assert_eq!(status, 404);
    assert_eq!(body["message"], json!("AI MCP server not found"));
}

// ---- Rust-native: the /rpc AI procedures and actions (Q.D6) ----

async fn rpc(
    f: &Fixture,
    path: &str,
    input: Value,
) -> Result<Value, chatbotx_api::orpc::OrpcError> {
    let context = chatbotx_api::rpc_ai::RpcAiContext::new(WS, 1001);
    chatbotx_api::rpc_ai::call(&f.state, path, &context, &input)
        .await
        .expect("ported")
        .map(|out| out.body())
}

#[tokio::test]
async fn rpc_lists_functions_with_date_meta_and_actions_write() {
    let f = fixture().await;
    seed(&f).await;
    let body = rpc(
        &f,
        "aiFunctionsAPI.listAIFunctionsAuthenticatedAPI",
        json!({ "workspaceId": WS.to_string() }),
    )
    .await
    .expect("list");
    assert_eq!(body["json"]["data"][0]["name"], json!("Lookup order"));
    assert_eq!(body["meta"][0], json!([1, "data", 0, "createdAt"]));
    // Duplicate name: next-safe-action validation errors on `name`.
    let error = rpc(
        &f,
        "actions.createAIFunctionAction",
        json!({ "args": [WS.to_string(), function_body("Lookup order")], "workspaceId": WS.to_string() }),
    )
    .await
    .expect_err("taken");
    assert_eq!(
        error.data.expect("data")["validationErrors"]["name"]["_errors"][0],
        json!("AI Function name already exists")
    );
    let updated = rpc(
        &f,
        "actions.updateAIFunctionAction",
        json!({ "args": [WS.to_string(), "7", function_body("Renamed")] }),
    )
    .await
    .expect("update");
    assert_eq!(updated["json"]["name"], json!("Renamed"));
    rpc(
        &f,
        "actions.deleteAIFunctionAction",
        json!({ "args": [WS.to_string(), "7"] }),
    )
    .await
    .expect("delete");
    let error = rpc(
        &f,
        "actions.deleteAIMcpServerAction",
        json!({ "args": [WS.to_string(), "99"] }),
    )
    .await
    .expect_err("missing");
    assert_eq!(error.message, "MCP Server not found");
}

#[tokio::test]
async fn rpc_mcp_actions_refuse_an_unresolved_token_template() {
    let f = fixture().await;
    let mut body = server_body("Tokens");
    body["auth"] = json!({ "type": "token", "token": "{{bot_field:42}}" });
    let error = rpc(
        &f,
        "actions.createAIMcpServerAction",
        json!({ "args": [WS.to_string(), body] }),
    )
    .await
    .expect_err("invalid key");
    assert_eq!(
        error.data.expect("data")["validationErrors"]["auth"]["token"]["_errors"][0],
        json!("Invalid API key")
    );
}
