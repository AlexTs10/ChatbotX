//! Port of `apps/builder/__tests__/ai-agents-public-api.test.ts` (Q.D6).
//! Adapted: the captured oRPC handlers become `handlers::dispatch` calls over
//! a real migrated database; "the service was called with" becomes the
//! response and the stored rows. The TS inputs (`{ name }` only) are
//! completed to valid requests: the Rust handler runs the input schema the
//! mocked chain skipped.

mod handlers_common;

use chatbotx_api::routes::scopes_of_module;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

fn agent_body(name: &str) -> Value {
    json!({
        "name": name,
        "prompt": "You answer questions.",
        "messages": [{ "role": "user", "content": "Hello" }],
        "models": [{ "provider": "openai", "model": "gpt-4o-mini" }],
        "temperature": 0.4,
        "maxOutputTokens": 500,
        "tools": [],
        "webSearchAuthorizedDomains": [{ "value": " Docs.Example.com " }],
        "isDefault": false,
    })
}

async fn create(f: &Fixture, body: Value) -> (u16, Value) {
    call(f, "aiAgents.create", WS, &[], None, Some(body)).await
}

async fn created_id(f: &Fixture) -> String {
    let (status, body) = create(f, agent_body("Support agent")).await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("id").to_string()
}

#[tokio::test]
async fn registers_the_ai_agents_public_router_under_the_automation_scope() {
    assert_eq!(
        scopes_of_module("features/ai-agents/api/public"),
        vec!["automation"]
    );
}

// describe("GET /v1/ai-agents")

#[tokio::test]
async fn get_v1_ai_agents_delegates_to_ai_agent_service_list_ai_agents() {
    let f = fixture().await;
    created_id(&f).await;
    let (status, body) = call(
        &f,
        "aiAgents.list",
        WS,
        &[],
        Some("page=1&perPage=50"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pageCount"], json!(1));
    assert_eq!(body["data"][0]["workspaceId"], json!(WS.to_string()));
    assert!(body["data"][0].get("actionRules").is_none());
    let (_, other) = call(&f, "aiAgents.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other, json!({ "data": [], "pageCount": 0 }));
}

// describe("GET /v1/ai-agents/{id}")

#[tokio::test]
async fn get_v1_ai_agents_id_delegates_to_ai_agent_service_find_by() {
    let f = fixture().await;
    let id = created_id(&f).await;
    let (status, body) = call(&f, "aiAgents.get", WS, &[("id", id.as_str())], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["webSearchAuthorizedDomains"],
        json!(["docs.example.com"])
    );
    let (status, _) = call(
        &f,
        "aiAgents.get",
        OTHER_WS,
        &[("id", id.as_str())],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn get_v1_ai_agents_id_throws_not_found_when_the_agent_does_not_exist() {
    let f = fixture().await;
    let (status, body) = call(&f, "aiAgents.get", WS, &[("id", "99")], None, None).await;
    assert_eq!(status, 404);
    assert_eq!(body["message"], json!("AI agent not found"));
}

// describe("POST /v1/ai-agents")

#[tokio::test]
async fn post_v1_ai_agents_delegates_to_ai_agent_service_create_and_return() {
    let f = fixture().await;
    let (status, body) = create(&f, agent_body("Support agent")).await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["name"], json!("Support agent"));
    assert_eq!(body["isRichResponse"], json!(false));
    let id: i64 = body["id"].as_str().expect("id").parse().expect("number");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "AIAgent" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!(WS))
    );
}

#[tokio::test]
async fn two_creates_with_the_same_name_return_distinct_ids() {
    let f = fixture().await;
    let first = created_id(&f).await;
    let second = created_id(&f).await;
    assert_ne!(first, second);
}

// describe("PATCH /v1/ai-agents/{id}")

#[tokio::test]
async fn patch_v1_ai_agents_id_delegates_to_ai_agent_service_update_ai_agent_and_returns_its_result()
 {
    let f = fixture().await;
    let id = created_id(&f).await;
    let (status, body) = call(
        &f,
        "aiAgents.update",
        WS,
        &[("id", id.as_str())],
        None,
        Some(json!({ "name": "Renamed" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (body["id"].clone(), body["name"].clone()),
        (json!(id), json!("Renamed"))
    );
    assert_eq!(body["prompt"], json!("You answer questions."));
}

// describe("DELETE /v1/ai-agents/{id}")

#[tokio::test]
async fn delete_v1_ai_agents_id_delegates_to_ai_agent_service_delete() {
    let f = fixture().await;
    let id = created_id(&f).await;
    let (status, _) = call(
        &f,
        "aiAgents.delete",
        WS,
        &[("id", id.as_str())],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = call(&f, "aiAgents.get", WS, &[("id", id.as_str())], None, None).await;
    assert_eq!(status, 404);
    // A missing id is no error (parity p1-02 ai-agents).
    let (status, _) = call(&f, "aiAgents.delete", WS, &[("id", "99")], None, None).await;
    assert_eq!(status, 204);
}

// ---- Rust-native: the input schema (parity p1-02 ai-agents) ----

#[tokio::test]
async fn rejects_invalid_input_and_keeps_the_private_keys_out() {
    let f = fixture().await;
    let mut bodies = Vec::new();
    for (key, value) in [
        ("actionPrompt", json!("secret")),
        ("temperature", json!(2.5)),
        ("maxOutputTokens", json!(32_769)),
        ("name", json!("  ")),
        (
            "models",
            json!([{ "provider": "openai", "model": "no-such-model" }]),
        ),
        ("messages", json!([{ "role": "robot", "content": "x" }])),
        (
            "webSearchAuthorizedDomains",
            json!([{ "value": "https://example.com/a" }]),
        ),
    ] {
        let mut body = agent_body("Bad");
        body[key] = value;
        bodies.push(body);
    }
    let mut without_prompt = agent_body("Bad");
    without_prompt
        .as_object_mut()
        .expect("object")
        .remove("prompt");
    bodies.push(without_prompt);
    for body in bodies {
        let (status, error) = create(&f, body.clone()).await;
        assert_eq!(
            (status, error["code"].clone()),
            (422, json!("invalidRequestData")),
            "{body}"
        );
    }
    let id = created_id(&f).await;
    let (status, body) = call(
        &f,
        "aiAgents.update",
        WS,
        &[("id", id.as_str())],
        None,
        Some(json!({ "actionPrompt": "private", "isDefault": true })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.get("actionPrompt").is_none());
    assert_eq!(body["isDefault"], json!(true));
    let (status, _) = call(
        &f,
        "aiAgents.update",
        WS,
        &[("id", "99")],
        None,
        Some(json!({ "temperature": -1 })),
    )
    .await;
    assert_eq!(status, 422);
}

// ---- delete-ai-agent-action.test.ts ----

#[tokio::test]
async fn calls_ai_agent_service_delete_with_the_bound_workspace_id_and_requested_ids() {
    let f = fixture().await;
    let first = created_id(&f).await;
    let second = created_id(&f).await;
    let context = chatbotx_api::rpc_ai::RpcAiContext::new(WS, 1001);
    let input = json!({ "args": [WS.to_string(), { "ids": [first, second] }] });
    chatbotx_api::rpc_ai::call(&f.state, "actions.deleteAIAgentAction", &context, &input)
        .await
        .expect("ported")
        .expect("delete");
    let (_, list) = call(&f, "aiAgents.list", WS, &[], None, None).await;
    assert_eq!(list, json!({ "data": [], "pageCount": 0 }));
}
