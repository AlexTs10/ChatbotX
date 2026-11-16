//! Port of `packages/business/__tests__/ai-agent.service.test.ts` (Q.D6).
//! Adapted: the mocked drizzle client becomes a real migrated database; the
//! audit spy becomes a recording job enqueuer; the mocked `findFirst` row
//! becomes the seeded agent 1.
//! Not applicable (no read caches and no template guards, P3.6-D1 rule):
//! the `withCache` key/tag test, the three cache-invalidation tests and
//! "takes the guard for the tool tokens' files, functions and MCP servers".

mod common;
mod x1_support;

use std::sync::Arc;

use chatbotx_business::ai_agent::{self as service, AiAgentInput};
use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, WS, scalar_i64, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

fn audit() -> (AuditService, Arc<RecordingEnqueuer>) {
    let jobs = RecordingEnqueuer::new();
    (AuditService::new(jobs.clone()), jobs)
}

fn actor() -> AuditActor {
    AuditActor {
        user_id: Some("1001".into()),
        workspace_id: Some(WS.to_string()),
        ..AuditActor::default()
    }
}

fn details(jobs: &RecordingEnqueuer) -> Vec<String> {
    jobs.calls()
        .iter()
        .filter_map(|call| call.data["data"]["detail"].as_str().map(str::to_string))
        .collect()
}

fn last_detail(jobs: &RecordingEnqueuer) -> Option<String> {
    details(jobs).pop()
}

/// `createRequest` of the TS file.
fn create_request() -> AiAgentInput {
    AiAgentInput {
        is_default: Some(false),
        is_rich_response: Some(false),
        max_output_tokens: Some(1000),
        messages: Some(json!([])),
        models: Some(json!([])),
        name: Some("Support agent".into()),
        prompt: Some("Help customers".into()),
        temperature: Some(0.7),
        tools: Some(vec![]),
        ..AiAgentInput::default()
    }
}

/// `createRequest` with the stored model, prompt and tools (the TS
/// `...createRequest, models: aiAgent.models, prompt, tools` shape).
fn same_request() -> AiAgentInput {
    AiAgentInput {
        models: Some(json!([{ "provider": "openai", "model": "gpt-4o" }])),
        tools: Some(vec!["file:knowledge-1".into()]),
        ..create_request()
    }
}

/// Agent 1 of `WS` (the TS `aiAgent` fixture), with `models` / `messages` /
/// `tools` / `isDefault` as given.
async fn seeded_with(
    models: &str,
    messages: &str,
    tools: &str,
    is_default: bool,
) -> common::TestDb {
    let db = two_workspaces().await;
    let (models, messages, tools) = (models.to_string(), messages.to_string(), tools.to_string());
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "AIAgent" ("id", "workspaceId", "isDefault", "name", "prompt", "messages", "models",
                 "temperature", "maxOutputTokens", "tools", "webSearchAuthorizedDomains", "isRichResponse")
               VALUES (1, ?1, ?2, 'Support agent', 'Help customers', ?3, ?4, 0.7, 1000, ?5, '[]', 0)"#,
            params![WS, is_default, messages, models, tools],
        )?;
        Ok(())
    })
    .await;
    db
}

async fn seeded() -> common::TestDb {
    seeded_with(
        r#"[{"provider":"openai","model":"gpt-4o"}]"#,
        "[]",
        r#"["file:knowledge-1"]"#,
        true,
    )
    .await
}

async fn update_details(db: &common::TestDb, data: AiAgentInput) -> Vec<String> {
    let (audit, jobs) = audit();
    with_audit_context(actor(), service::update(db, &audit, WS, 1, data))
        .await
        .expect("update");
    details(&jobs)
}

// describe("aiAgentService.findDefault")

#[tokio::test]
async fn returns_the_cached_default_ai_agent_with_the_workspace_cache_tag() {
    let db = seeded().await;
    let found = service::find_default(&db, WS).await.expect("find");
    assert_eq!(found.map(|a| a.id), Some(1));
}

#[tokio::test]
async fn returns_undefined_when_the_workspace_has_no_default_ai_agent() {
    let db = seeded_with("[]", "[]", "[]", false).await;
    assert_eq!(service::find_default(&db, WS).await.expect("find"), None);
}

// describe("aiAgentService AI action validation")

#[tokio::test]
async fn rejects_an_inactive_or_unpublished_flow_action() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "name", "workspaceId", "active") VALUES (7, 'Demo', ?1, 1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let (audit, _) = audit();
    let error = service::create(
        &db,
        &audit,
        WS,
        AiAgentInput {
            action_rules: Some(json!([{
                "id": "rule-1",
                "when": "Customer asks for a demo",
                "actions": [{ "id": "flow-1", "type": "send_flow", "flowId": "7" }],
            }])),
            ..create_request()
        },
    )
    .await
    .expect_err("unpublished");
    assert_eq!(
        error.message(),
        "A selected flow must be active and published."
    );
}

// describe("aiAgentService audit messages")

#[tokio::test]
async fn create_logs_by_id() {
    let db = two_workspaces().await;
    let (audit, jobs) = audit();
    let created = with_audit_context(actor(), service::create(&db, &audit, WS, create_request()))
        .await
        .expect("create");
    assert_eq!(
        last_detail(&jobs),
        Some(format!("created a new AI Agent (#{})", created.id))
    );
}

#[tokio::test]
async fn delete_logs_one_entry_per_agent_by_id() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "AIAgent" ("id", "workspaceId", "name", "messages", "models", "temperature", "maxOutputTokens", "tools")
               VALUES (2, ?1, 'Second', '[]', '[]', 0.5, 100, '[]')"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let (audit, jobs) = audit();
    with_audit_context(actor(), service::delete(&db, &audit, WS, vec![1, 2]))
        .await
        .expect("delete");
    assert_eq!(
        details(&jobs),
        ["deleted an AI Agent (#1)", "deleted an AI Agent (#2)"]
    );
}

#[tokio::test]
async fn update_is_default_only_true_logs_set_as_default() {
    let db = seeded_with("[]", "[]", "[]", false).await;
    let data = AiAgentInput {
        is_default: Some(true),
        ..AiAgentInput::default()
    };
    assert_eq!(
        update_details(&db, data).await,
        ["set as default an AI Agent (#1)"]
    );
}

#[tokio::test]
async fn update_is_default_only_false_logs_unset_default() {
    let db = seeded().await;
    let data = AiAgentInput {
        is_default: Some(false),
        ..AiAgentInput::default()
    };
    assert_eq!(
        update_details(&db, data).await,
        ["unset default an AI Agent (#1)"]
    );
}

#[tokio::test]
async fn update_is_default_only_resubmitting_the_current_value_does_not_audit() {
    let db = seeded().await;
    let data = AiAgentInput {
        is_default: Some(true),
        ..AiAgentInput::default()
    };
    assert!(update_details(&db, data).await.is_empty());
}

fn with(models: Option<Value>, prompt: Option<&str>, tools: Option<&str>) -> AiAgentInput {
    let mut data = same_request();
    if let Some(models) = models {
        data.models = Some(models);
    }
    if let Some(prompt) = prompt {
        data.prompt = Some(prompt.into());
    }
    if let Some(tool) = tools {
        data.tools = Some(vec![tool.into()]);
    }
    data
}

fn gpt5() -> Option<Value> {
    Some(json!([{ "provider": "openai", "model": "gpt-5" }]))
}

#[tokio::test]
async fn update_model_only_change() {
    let db = seeded().await;
    assert_eq!(
        update_details(&db, with(gpt5(), None, None)).await,
        ["updated the AI Agent model (#1)"]
    );
}

#[tokio::test]
async fn update_instructions_only_change() {
    let db = seeded().await;
    assert_eq!(
        update_details(&db, with(None, Some("New instructions"), None)).await,
        ["updated the AI Agent instructions (#1)"]
    );
}

#[tokio::test]
async fn update_knowledge_base_only_change() {
    let db = seeded().await;
    assert_eq!(
        update_details(&db, with(None, None, Some("file:knowledge-2"))).await,
        ["updated the AI Agent knowledge base (#1)"]
    );
}

#[tokio::test]
async fn update_model_instructions_change() {
    let db = seeded().await;
    assert_eq!(
        update_details(&db, with(gpt5(), Some("New instructions"), None)).await,
        ["updated the AI Agent model and instructions (#1)"]
    );
}

#[tokio::test]
async fn update_model_knowledge_base_change_no_instructions() {
    let db = seeded().await;
    assert_eq!(
        update_details(&db, with(gpt5(), None, Some("file:knowledge-2"))).await,
        ["updated the AI Agent model and knowledge base (#1)"]
    );
}

#[tokio::test]
async fn update_instructions_knowledge_base_change_no_model() {
    let db = seeded().await;
    assert_eq!(
        update_details(
            &db,
            with(None, Some("New instructions"), Some("file:knowledge-2"))
        )
        .await,
        ["updated the AI Agent instructions and knowledge base (#1)"]
    );
}

#[tokio::test]
async fn update_model_instructions_knowledge_base_change() {
    let db = seeded().await;
    assert_eq!(
        update_details(
            &db,
            with(gpt5(), Some("New instructions"), Some("file:knowledge-2"))
        )
        .await,
        ["updated the AI Agent model, instructions and knowledge base (#1)"]
    );
}

#[tokio::test]
async fn update_messages_only_change_falls_back_to_generic_message() {
    let db = seeded().await;
    let data = AiAgentInput {
        messages: Some(json!([{ "role": "user", "content": "Hi" }])),
        ..same_request()
    };
    assert_eq!(
        update_details(&db, data).await,
        ["updated an AI Agent (#1)"]
    );
}

#[tokio::test]
async fn update_other_field_change_e_g_name_falls_back_to_generic_message() {
    let db = seeded().await;
    let data = AiAgentInput {
        name: Some("Renamed agent".into()),
        ..same_request()
    };
    assert_eq!(
        update_details(&db, data).await,
        ["updated an AI Agent (#1)"]
    );
}

#[tokio::test]
async fn update_unchanged_payload_does_not_audit() {
    let db = seeded().await;
    assert!(update_details(&db, same_request()).await.is_empty());
}

#[tokio::test]
async fn update_reordering_file_tools_does_not_count_as_a_knowledge_base_change() {
    let db = seeded_with(
        r#"[{"provider":"openai","model":"gpt-4o"}]"#,
        "[]",
        r#"["file:a","file:b"]"#,
        true,
    )
    .await;
    let data = AiAgentInput {
        tools: Some(vec!["file:b".into(), "file:a".into()]),
        ..same_request()
    };
    assert!(update_details(&db, data).await.is_empty());
}

#[tokio::test]
async fn update_does_not_flag_models_as_changed_when_only_jsonb_key_order_differs() {
    let db = seeded_with(
        r#"[{"model":"gpt-4o","provider":"openai"}]"#,
        "[]",
        r#"["file:knowledge-1"]"#,
        true,
    )
    .await;
    assert!(update_details(&db, same_request()).await.is_empty());
}

#[tokio::test]
async fn update_does_not_flag_messages_as_changed_when_only_jsonb_key_order_differs() {
    let db = seeded_with(
        r#"[{"provider":"openai","model":"gpt-4o"}]"#,
        r#"[{"content":"Hi","role":"user"}]"#,
        r#"["file:knowledge-1"]"#,
        true,
    )
    .await;
    let data = AiAgentInput {
        messages: Some(json!([{ "role": "user", "content": "Hi" }])),
        ..same_request()
    };
    assert!(update_details(&db, data).await.is_empty());
}

// ---- Rust-native: workspace scoping and the default flag ----

#[tokio::test]
async fn a_new_default_clears_the_previous_default_and_other_workspaces_are_not_touched() {
    let db = seeded().await;
    let (audit, _) = audit();
    let other = service::create(
        &db,
        &audit,
        OTHER_WS,
        AiAgentInput {
            is_default: Some(true),
            ..create_request()
        },
    )
    .await
    .expect("other");
    let second = service::create(
        &db,
        &audit,
        WS,
        AiAgentInput {
            is_default: Some(true),
            web_search_authorized_domains: Some(vec![
                " Docs.Example.com ".into(),
                "docs.example.com".into(),
            ]),
            ..create_request()
        },
    )
    .await
    .expect("second");
    assert_eq!(
        second.web_search_authorized_domains.0,
        json!(["docs.example.com"])
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "isDefault" FROM "AIAgent" WHERE "id" = ?1"#,
            1
        )
        .await,
        Some(0)
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "isDefault" FROM "AIAgent" WHERE "id" = ?1"#,
            other.id
        )
        .await,
        Some(1)
    );
    assert!(
        service::update(&db, &audit, OTHER_WS, 1, create_request())
            .await
            .is_err()
    );
    assert_eq!(
        service::delete(&db, &audit, OTHER_WS, vec![1])
            .await
            .expect("delete"),
        Vec::<i64>::new()
    );
}
