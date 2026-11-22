//! `apps/worker/__tests__/ai-agent-handoff-reentry.test.ts` and
//! `packages/automated-response/__tests__/enqueue-handoff-reentry.test.ts`.
//! The TypeScript files mock `streamText` and the services; here the job
//! runs on the worker fixture with a scripted provider ([`FakeHttp`]) and
//! the assertions read the conversation row, the provider request and the
//! queued jobs. The analytics emit cases are n-a (the fixture has no event
//! buses); the outcome is visible through the bot state and the forward.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::ai::Ai;
use crate::ai::llm::fake::FakeHttp;
use crate::handoff_reentry::*;
use crate::steps::test_support::Fixture;

const TEXT: &str = "Tôi muốn xem thông tin tự động trước, chưa cần gặp nhân viên.";

fn with_ai(fx: &mut Fixture, answers: Vec<(u16, Value)>) -> Arc<FakeHttp> {
    let http = Arc::new(FakeHttp::new(answers));
    fx.deps.ai = Arc::new(Ai {
        http: http.clone(),
        embedder: Arc::new(chatbotx_business::ai::knowledge::HttpEmbedder::new()),
    });
    http
}

fn rules() -> Value {
    json!([{ "id": "return-to-bot", "when": "Khách muốn xem thông tin tự động trước.",
             "actions": [{ "id": "enable-bot", "type": "transfer_to_bot" }] }])
}

async fn setup(fx: &Fixture, text: &'static str, rules: Value) {
    fx.exec(
        r#"UPDATE "Conversation" SET "botEnabled" = 0 WHERE "id" = 6;
           INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (70, 2, 'muse');
           INSERT INTO "IntegrationMuse" ("id", "auth", "workspaceId", "integrationId", "maxOutputTokens", "model")
             VALUES (72, '{"authType":"secretText","secretText":"k"}', 2, 70, 500, 'muse-spark-1.3');"#,
    )
    .await;
    let rules = rules.to_string();
    fx.sql(move |tx| {
        tx.execute(
            r#"INSERT INTO "Message" ("id", "workspaceId", "conversationId", "contactInboxId", "senderType", "messageType", "contentType", "text", "createdAt")
               VALUES (80, 2, 6, 5, 'contact', 'incoming', 'text', ?1, ?2)"#,
            chatbotx_db::rusqlite::params![text, chatbotx_db::repo::now_ms()],
        )?;
        tx.execute(
            r#"INSERT INTO "AIAgent" ("id", "workspaceId", "name", "messages", "models", "tools", "temperature", "maxOutputTokens", "isDefault", "actionRules")
               VALUES (90, 2, 'Agent', '[]', '[{"provider":"muse","model":"muse-spark-1.3"}]', '[]', 0, 300, 1, ?1)"#,
            [rules],
        )?;
        Ok(())
    })
    .await;
}

fn select(when: &str, back: &str) -> (u16, Value) {
    let args = json!({ "matches": [{ "ruleId": "return-to-bot", "whenEvidence": when, "returnToBotEvidence": back }] });
    (
        200,
        json!({ "choices": [{ "message": { "role": "assistant", "content": null, "tool_calls": [{
            "id": "tool-1", "type": "function",
            "function": { "name": "select_handoff_reentry_rule", "arguments": args.to_string() } }] } }] }),
    )
}

async fn run(fx: &Fixture) {
    process(
        &fx.deps,
        &json!({ "conversationId": "6", "contactInboxId": "5", "messageId": "80" }),
    )
    .await
    .unwrap();
}

async fn bot_enabled(fx: &Fixture) -> bool {
    fx.int(r#"SELECT "botEnabled" FROM "Conversation" WHERE "id" = 6"#).await == Some(1)
}

fn forwarded(fx: &Fixture) -> Vec<Value> {
    fx.jobs
        .named("processAutomatedResponse")
        .into_iter()
        .map(|j| j.data)
        .collect()
}

#[tokio::test]
async fn enables_bot_only_after_the_classifier_selects_the_configured_rule_with_valid_evidence() {
    let mut fx = Fixture::new().await;
    setup(&fx, TEXT, rules()).await;
    let http = with_ai(
        &mut fx,
        vec![select("muốn xem thông tin tự động trước", "chưa cần gặp nhân viên")],
    );
    run(&fx).await;
    let body = &http.requests()[0].1;
    assert_eq!(
        body["tool_choice"],
        json!({ "type": "function", "function": { "name": "select_handoff_reentry_rule" } })
    );
    assert_eq!(body["tools"][0]["function"]["name"], "select_handoff_reentry_rule");
    assert_eq!(body["temperature"], 0.0);
    assert_eq!(body["messages"][1], json!({ "role": "user", "content": TEXT }));
    assert_eq!(http.requests().len(), 1, "one step, no follow-up call");
    assert!(bot_enabled(&fx).await);
    assert_eq!(
        forwarded(&fx),
        [json!({ "type": "processAutomatedResponse", "data": {
            "conversationId": "6", "contactInboxId": "5", "messageId": "80" } })]
    );
}

#[tokio::test]
async fn forwards_the_trigger_message_when_the_bot_is_already_enabled() {
    let mut fx = Fixture::new().await;
    setup(&fx, TEXT, rules()).await;
    fx.exec(r#"UPDATE "Conversation" SET "botEnabled" = 1 WHERE "id" = 6"#).await;
    let http = with_ai(&mut fx, vec![]);
    run(&fx).await;
    assert_eq!(forwarded(&fx).len(), 1);
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn a_final_no_match_keeps_the_handoff() {
    let mut fx = Fixture::new().await;
    setup(&fx, TEXT, rules()).await;
    with_ai(&mut fx, vec![FakeHttp::openai_text("")]);
    run(&fx).await;
    assert!(!bot_enabled(&fx).await);
    assert!(forwarded(&fx).is_empty());
}

#[tokio::test]
async fn keeps_human_handoff_when_a_classifier_falsely_selects_alo() {
    let mut fx = Fixture::new().await;
    setup(&fx, "alo", rules()).await;
    with_ai(&mut fx, vec![select("alo", "alo")]);
    run(&fx).await;
    assert!(!bot_enabled(&fx).await);
    assert!(forwarded(&fx).is_empty());
}

#[tokio::test]
async fn keeps_human_handoff_when_classifier_evidence_is_not_quoted_from_the_message() {
    let mut fx = Fixture::new().await;
    setup(&fx, TEXT, rules()).await;
    with_ai(
        &mut fx,
        vec![select("tôi muốn dùng chatbot ngay", "không cần nhân viên")],
    );
    run(&fx).await;
    assert!(!bot_enabled(&fx).await);
    assert!(forwarded(&fx).is_empty());
}

#[tokio::test]
async fn does_not_call_a_model_when_the_workspace_is_inactive() {
    let mut fx = Fixture::new().await;
    setup(&fx, TEXT, rules()).await;
    fx.exec(r#"UPDATE "Workspace" SET "isActive" = 0 WHERE "id" = 2"#).await;
    let http = with_ai(&mut fx, vec![]);
    run(&fx).await;
    assert!(http.requests().is_empty());
    assert!(forwarded(&fx).is_empty());
}

#[tokio::test]
async fn does_not_create_a_model_call_when_no_standalone_transfer_to_bot_rule_exists() {
    let mut fx = Fixture::new().await;
    let mixed = json!([{ "id": "mixed-rule", "when": "unsafe to run during handoff", "actions": [
        { "id": "enable-bot", "type": "transfer_to_bot" }, { "id": "archive", "type": "archive" }] }]);
    setup(&fx, TEXT, mixed).await;
    let http = with_ai(&mut fx, vec![]);
    run(&fx).await;
    assert!(http.requests().is_empty());
    assert!(!bot_enabled(&fx).await);
}

#[test]
fn evidence_needs_eight_meaningful_characters_quoted_from_the_message() {
    assert!(has_valid_evidence(TEXT, "  CHƯA cần   gặp nhân viên "));
    assert!(!has_valid_evidence("alo", "alo"));
    assert!(!has_valid_evidence(TEXT, "không cần nhân viên"));
}

/// `enqueue-handoff-reentry.test.ts`.
#[tokio::test]
async fn enqueues_one_classifier_job_per_message_with_a_dedup_id() {
    let fx = Fixture::new().await;
    enqueue(&fx.deps, 6, 5, 80, 2).await;
    let jobs = fx.jobs.named(PROCESS_HANDOFF_REENTRY);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].opts.job_id.as_deref(), Some("handoff-reentry-80"));
    let dedup = jobs[0].opts.dedup.clone().unwrap();
    assert_eq!((dedup.id.as_str(), dedup.ttl_ms, dedup.extend, dedup.replace), ("handoff-reentry-80", Some(300_000), true, true));
    assert_eq!(
        jobs[0].data,
        json!({ "type": "processHandoffReentry", "data": {
            "conversationId": "6", "contactInboxId": "5", "messageId": "80" } })
    );
    // A queue failure is logged, not raised.
    fx.jobs.fail_next();
    enqueue(&fx.deps, 6, 5, 81, 2).await;
}
