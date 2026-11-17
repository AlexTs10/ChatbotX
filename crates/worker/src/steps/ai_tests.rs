//! Ported AI step tests (Q.W2 slice 1). The TypeScript files mock
//! `streamText` and the services; here the steps run through `runFlowNode`
//! on the worker fixture with a scripted provider ([`FakeHttp`]) and the
//! assertions read the rows, the provider requests and the error route.

use std::sync::Arc;

use serde_json::{Value, json};

use super::test_support::*;
use crate::ai::Ai;
use crate::ai::llm::fake::FakeHttp;

fn with_ai(fx: &mut Fixture, answers: Vec<(u16, Value)>) -> Arc<FakeHttp> {
    let http = Arc::new(FakeHttp::new(answers));
    fx.deps.ai = Arc::new(Ai {
        http: http.clone(),
        embedder: Arc::new(chatbotx_business::ai::knowledge::HttpEmbedder::new()),
    });
    http
}

async fn setup(fx: &Fixture) {
    fx.exec(
        r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (30, 'answer', 'shortText', 2);
           INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (70, 2, 'muse'), (71, 2, 'openaiCompatible');
           INSERT INTO "IntegrationMuse" ("id", "auth", "workspaceId", "integrationId", "maxOutputTokens", "model") VALUES (72, '{"authType":"secretText","secretText":"k"}', 2, 70, 500, 'muse-spark-1.3');
           INSERT INTO "IntegrationOpenaiCompatible" ("id", "auth", "baseURL", "defaultModel", "enabled", "integrationId", "name", "preset", "workspaceId")
             VALUES (73, NULL, 'http://llm.test/v1', 'local', 1, 71, 'Local', 'lmstudio', 2);"#,
    )
    .await;
}

async fn answer(fx: &Fixture) -> Option<String> {
    fx.text(r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = 3 AND "customFieldId" = 30"#)
        .await
}

/// Runs `step` with an error route to `n3`; `true` = the error route ran.
async fn run_step(fx: &Fixture, step: Value) -> bool {
    let mut step = step;
    step["id"] = json!("1");
    step["states"] = json!([{ "id": "err", "stateType": "error" }]);
    fx.node(
        json!([step]),
        json!([{ "sourceHandle": "err", "target": "n3" }]),
    )
    .await;
    fx.run(json!({})).await.unwrap();
    fx.jobs.send_flows().iter().any(|job| job["nodeId"] == "n3")
}

fn generate_text(extra: Value) -> Value {
    let mut step = json!({ "stepType": "aiGenerateText", "provider": "muse", "model": "muse-spark-1.3",
                           "text": "Hello", "outputFieldId": "30" });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        step[k] = v;
    }
    step
}

/// `ai-generate-text-handler.test.ts`.
mod ai_generate_text_handler {
    use super::*;

    #[tokio::test]
    async fn saves_the_generated_text() {
        let mut fx = Fixture::new().await;
        setup(&fx).await;
        let http = with_ai(&mut fx, vec![FakeHttp::openai_text("Hi there")]);
        assert!(!run_step(&fx, generate_text(json!({}))).await);
        assert_eq!(answer(&fx).await.as_deref(), Some("Hi there"));
        let (url, body) = &http.requests()[0];
        assert_eq!(url, "https://api.meta.ai/v1/chat/completions");
        assert_eq!(
            body["messages"][1],
            json!({ "role": "user", "content": "Hello" })
        );
    }

    #[tokio::test]
    async fn appends_the_channel_length_instruction_to_the_authors_system_prompt() {
        let mut fx = Fixture::new().await;
        setup(&fx).await;
        fx.exec(r#"UPDATE "ContactInbox" SET "channel" = 'instagram' WHERE "id" = 5"#)
            .await;
        let http = with_ai(&mut fx, vec![FakeHttp::openai_text("ok")]);
        run_step(
            &fx,
            generate_text(json!({ "system": "You are a stylist." })),
        )
        .await;
        assert_eq!(
            http.requests()[0].1["messages"][0]["content"],
            "You are a stylist.\n\nYour answer will be sent as one instagram chat message. Keep it under 800 characters including spaces. Use plain text, no markdown headings."
        );
    }

    #[tokio::test]
    async fn resolves_contact_variables_in_the_system_prompt_and_user_message() {
        let mut fx = Fixture::new().await;
        setup(&fx).await;
        super::super::tests::with_variables(&mut fx);
        let http = with_ai(&mut fx, vec![FakeHttp::openai_text("ok")]);
        run_step(
            &fx,
            generate_text(
                json!({ "system": "You help {{first_name}}.", "text": "Hi, I am {{first_name}}" }),
            ),
        )
        .await;
        let body = &http.requests()[0].1;
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .starts_with("You help Ada.")
        );
        assert_eq!(body["messages"][1]["content"], "Hi, I am Ada");
    }

    #[tokio::test]
    async fn reports_a_provider_error_as_a_step_error() {
        let mut fx = Fixture::new().await;
        setup(&fx).await;
        with_ai(
            &mut fx,
            vec![(
                404,
                json!({ "error": { "message": "The requested model was not found." } }),
            )],
        );
        assert!(run_step(&fx, generate_text(json!({}))).await);
        assert_eq!(answer(&fx).await, None);
    }

    #[tokio::test]
    async fn maps_a_402_to_the_insufficient_credits_message() {
        let mut fx = Fixture::new().await;
        setup(&fx).await;
        with_ai(
            &mut fx,
            vec![(402, json!({ "error": { "message": "Payment required" } }))],
        );
        assert!(run_step(&fx, generate_text(json!({}))).await);
        assert_eq!(answer(&fx).await, None);
    }
}

/// `ai-message-history-builders.test.ts`: remembered history starts after
/// the delete marker.
mod ai_message_history_builders {
    use super::*;

    #[tokio::test]
    async fn uses_the_authoritative_delete_marker_when_loading_remembered_history() {
        let mut fx = Fixture::new().await;
        setup(&fx).await;
        let now = chatbotx_db::repo::now_ms();
        fx.sql(move |tx| {
            for (id, sender, kind, text) in [
                (80, "contact", "incoming", "before marker"),
                (81, "bot", "outgoing", "marker"),
                (82, "contact", "incoming", "after"),
                (83, "bot", "activity", "note"),
            ] {
                tx.execute(
                    r#"INSERT INTO "Message" ("id", "workspaceId", "conversationId", "contactInboxId", "senderType", "messageType", "contentType", "text", "createdAt")
                       VALUES (?1, 2, 6, 5, ?2, ?3, 'text', ?4, ?5)"#,
                    chatbotx_db::rusqlite::params![id, sender, kind, text, now + id],
                )?;
            }
            tx.execute(r#"UPDATE "Conversation" SET "aiContextLastMessageId" = 81, "lastActivityAt" = ?1 WHERE "id" = 6"#, [now])?;
            Ok(())
        })
        .await;
        let http = with_ai(&mut fx, vec![FakeHttp::openai_text("ok")]);
        run_step(
            &fx,
            generate_text(json!({ "remember": true, "text": "instruction" })),
        )
        .await;
        let messages = http.requests()[0].1["messages"].clone();
        assert_eq!(
            messages.as_array().unwrap()[1..],
            [
                json!({ "role": "user", "content": "after" }),
                json!({ "role": "user", "content": "instruction" }),
            ]
        );
    }
}

/// `ai-generate-text-agent-openai-compatible.test.ts` and
/// `ai-agent-runner-openai-compatible.test.ts` (the runner's provider order).
mod ai_generate_text_agent {
    use super::*;

    async fn agent(fx: &Fixture, models: Value) {
        let models = models.to_string();
        fx.sql(move |tx| {
            tx.execute(
                r#"INSERT INTO "AIAgent" ("id", "workspaceId", "name", "prompt", "messages", "models", "tools", "temperature", "maxOutputTokens")
                   VALUES (90, 2, 'Agent', 'Help {{first_name}}', '[]', ?1, '[]', 0.5, 300)"#,
                [models],
            )?;
            Ok(())
        })
        .await;
    }

    fn step(extra: Value) -> Value {
        let mut step = json!({ "stepType": "aiGenerateTextAgent", "aiAgentId": "90", "provider": "muse",
                               "message": "Hi {{first_name}}", "outputFieldId": "30" });
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            step[k] = v;
        }
        step
    }

    #[tokio::test]
    async fn passes_the_preferred_openai_compatible_model_to_the_runner() {
        let mut fx = Fixture::new().await;
        setup(&fx).await;
        agent(
            &fx,
            json!([{ "provider": "muse", "model": "muse-spark-1.3" }]),
        )
        .await;
        let http = with_ai(&mut fx, vec![FakeHttp::openai_text("Compatible answer")]);
        let errored = run_step(
            &fx,
            step(json!({ "provider": "openaiCompatible", "integrationId": "73", "model": "qwen" })),
        )
        .await;
        assert!(!errored);
        let (url, body) = &http.requests()[0];
        assert_eq!(url, "http://llm.test/v1/chat/completions");
        assert_eq!(body["model"], "qwen");
        assert_eq!(answer(&fx).await.as_deref(), Some("Compatible answer"));
    }

    #[tokio::test]
    async fn resolves_contact_variables_in_the_step_message_and_the_prompt() {
        let mut fx = Fixture::new().await;
        setup(&fx).await;
        super::super::tests::with_variables(&mut fx);
        agent(
            &fx,
            json!([{ "provider": "muse", "model": "muse-spark-1.3" }]),
        )
        .await;
        let http = with_ai(&mut fx, vec![FakeHttp::openai_text("ok")]);
        run_step(&fx, step(json!({}))).await;
        let body = &http.requests()[0].1;
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .starts_with("Help Ada")
        );
        assert_eq!(body["messages"][1]["content"], "Hi Ada");
        assert_eq!(body["max_tokens"], 300 + 1024);
    }

    #[tokio::test]
    async fn returns_a_clear_error_when_the_openai_compatible_integration_cannot_respond() {
        let mut fx = Fixture::new().await;
        setup(&fx).await;
        fx.exec(r#"UPDATE "IntegrationOpenaiCompatible" SET "enabled" = 0"#)
            .await;
        agent(&fx, json!([])).await;
        let http = with_ai(&mut fx, vec![]);
        let errored = run_step(
            &fx,
            step(json!({ "provider": "openaiCompatible", "integrationId": "73", "model": "qwen" })),
        )
        .await;
        assert!(errored);
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn skips_a_disabled_dynamic_provider_and_continues_with_the_native_fallback() {
        let mut fx = Fixture::new().await;
        setup(&fx).await;
        fx.exec(r#"UPDATE "IntegrationOpenaiCompatible" SET "enabled" = 0"#)
            .await;
        agent(
            &fx,
            json!([{ "kind": "openaiCompatible", "integrationId": "73", "model": "qwen" },
                   { "provider": "muse", "model": "muse-spark-1.3" }]),
        )
        .await;
        // A provider failure moves on to the next model too.
        let http = with_ai(&mut fx, vec![FakeHttp::openai_text("Native answer")]);
        let errored = run_step(&fx, step(json!({ "provider": "muse" }))).await;
        assert!(!errored);
        assert_eq!(http.requests().len(), 1);
        assert_eq!(answer(&fx).await.as_deref(), Some("Native answer"));
    }
}

/// `ai-delete-message-history.test.ts`.
mod ai_delete_message_history {
    use super::*;

    async fn message(fx: &Fixture, id: i64, conversation: i64, created_at: i64) {
        fx.sql(move |tx| {
            tx.execute(
                r#"INSERT INTO "Message" ("id", "workspaceId", "conversationId", "contactInboxId", "senderType", "messageType", "contentType", "text", "createdAt")
                   VALUES (?1, 2, ?2, 5, 'contact', 'incoming', 'text', 't', ?3)"#,
                chatbotx_db::rusqlite::params![id, conversation, created_at],
            )?;
            Ok(())
        })
        .await;
    }

    async fn marker(fx: &Fixture, conversation: i64) -> Option<i64> {
        fx.deps
            .db
            .read(move |conn| {
                Ok(conn.query_row(
                    r#"SELECT "aiContextLastMessageId" FROM "Conversation" WHERE "id" = ?1"#,
                    [conversation],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn falls_back_to_complete_history_and_deletes_the_cache_before_the_marker_moves() {
        let fx = Fixture::new().await;
        let now = chatbotx_db::repo::now_ms();
        // The only message is older than the narrow window of lastMessageAt.
        message(&fx, 80, 6, now - 10 * 24 * 3_600_000).await;
        fx.sql(move |tx| {
            tx.execute(
                r#"UPDATE "ContactInbox" SET "lastMessageAt" = ?1 WHERE "id" = 5"#,
                [now],
            )?;
            Ok(())
        })
        .await;
        let kv =
            chatbotx_kv::DurableKv::new(fx.deps.db.clone(), Arc::new(chatbotx_kv::SystemClock));
        kv.put("ai:ctx:v1:6", &json!({ "summary": "old" }), None)
            .await
            .unwrap();
        assert!(!run_step(&fx, json!({ "stepType": "aiDeleteMessageHistory" })).await);
        assert_eq!(marker(&fx, 6).await, Some(80));
        assert!(!kv.exists("ai:ctx:v1:6").await.unwrap());
    }

    #[tokio::test]
    async fn uses_one_complete_history_query_when_last_message_at_is_missing() {
        let fx = Fixture::new().await;
        message(&fx, 80, 6, 1_000).await;
        message(&fx, 81, 6, 2_000).await;
        assert!(!run_step(&fx, json!({ "stepType": "aiDeleteMessageHistory" })).await);
        assert_eq!(marker(&fx, 6).await, Some(81));
    }

    #[tokio::test]
    async fn no_message_clears_the_marker() {
        let fx = Fixture::new().await;
        fx.exec(r#"UPDATE "Conversation" SET "aiContextLastMessageId" = 5 WHERE "id" = 6"#)
            .await;
        assert!(!run_step(&fx, json!({ "stepType": "aiDeleteMessageHistory" })).await);
        assert_eq!(marker(&fx, 6).await, None);
    }

    #[tokio::test]
    async fn also_resets_the_canonical_dm_conversation_from_a_comment_conversation() {
        let fx = Fixture::new().await;
        fx.exec(
            r#"UPDATE "Conversation" SET "sourceId" = 'post-1' WHERE "id" = 6;
               INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (7, 2, 3);"#,
        )
        .await;
        message(&fx, 80, 6, 1_000).await;
        message(&fx, 81, 7, 2_000).await;
        assert!(!run_step(&fx, json!({ "stepType": "aiDeleteMessageHistory" })).await);
        assert_eq!(marker(&fx, 6).await, Some(80));
        assert_eq!(marker(&fx, 7).await, Some(81));
    }
}
