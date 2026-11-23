//! `packages/ai/__tests__/ai-context-service.test.ts` and
//! `apps/worker/__tests__/summarize-conversation.test.ts`. The TypeScript
//! files mock the store and the summarizer; here the context lives in the
//! durable kv of the worker fixture and the summarizer is a scripted
//! provider ([`FakeHttp`]).

use std::sync::Arc;

use serde_json::{Value, json};

use super::context::*;
use super::llm::fake::FakeHttp;
use super::{Ai, llm};
use crate::steps::test_support::Fixture;

fn with_ai(fx: &mut Fixture, answers: Vec<(u16, Value)>) -> Arc<FakeHttp> {
    let http = Arc::new(FakeHttp::new(answers));
    fx.deps.ai = Arc::new(Ai {
        http: http.clone(),
        embedder: Arc::new(chatbotx_business::ai::knowledge::HttpEmbedder::new()),
    });
    http
}

/// A Muse integration with auto reply (the summarizer's model).
async fn summarizer(fx: &Fixture) {
    fx.exec(
        r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (70, 2, 'muse');
           INSERT INTO "IntegrationMuse" ("id", "auth", "workspaceId", "integrationId", "maxOutputTokens", "model", "autoReply")
             VALUES (72, '{"authType":"secretText","secretText":"k"}', 2, 70, 500, 'muse-spark-1.3', 1);"#,
    )
    .await;
}

async fn message(fx: &Fixture, id: i64, sender: &'static str, text: &'static str, created_at: i64) {
    fx.sql(move |tx| {
        tx.execute(
            r#"INSERT INTO "Message" ("id", "workspaceId", "conversationId", "contactInboxId", "senderType", "messageType", "contentType", "text", "createdAt")
               VALUES (?1, 2, 6, 5, ?2, 'incoming', 'text', ?3, ?4)"#,
            chatbotx_db::rusqlite::params![id, sender, text, created_at],
        )?;
        Ok(())
    })
    .await;
}

async fn store(fx: &Fixture, context: Value) {
    let kv = chatbotx_kv::DurableKv::new(fx.deps.db.clone(), Arc::new(chatbotx_kv::SystemClock));
    kv.put(&context_key(6), &context, None).await.unwrap();
}

fn user(content: &str) -> AiMessage {
    AiMessage::text("user", content, Some(content.to_owned()), None)
}

fn history(n: usize) -> Value {
    json!((0..n)
        .map(|i| json!({ "role": "user", "content": format!("old-{i}"), "messageId": format!("old-{i}"), "createdAt": i }))
        .collect::<Vec<_>>())
}

fn summarize_jobs(fx: &Fixture) -> usize {
    fx.jobs.named(SUMMARIZE_CONVERSATION).len()
}

mod get_or_init_context {
    use super::*;

    #[tokio::test]
    async fn keeps_history_empty_when_the_marker_is_the_latest_message() {
        let mut fx = Fixture::new().await;
        let http = with_ai(&mut fx, vec![]);
        message(&fx, 10, "contact", "already deleted", 1_000).await;
        fx.exec(r#"UPDATE "Conversation" SET "aiContextLastMessageId" = 10, "lastActivityAt" = 1000 WHERE "id" = 6"#)
            .await;
        let context = get_or_init(&fx.deps, 2, 6, &Value::Null)
            .await
            .unwrap()
            .unwrap();
        assert!(context.history.is_empty());
        assert_eq!(context.summary, "");
        assert_eq!(context.marker_message_id.as_deref(), Some("10"));
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn keeps_only_messages_newer_than_the_marker_when_timestamps_match() {
        let mut fx = Fixture::new().await;
        summarizer(&fx).await;
        with_ai(&mut fx, vec![FakeHttp::openai_text("summary")]);
        message(&fx, 9, "contact", "old", 1_000).await;
        message(&fx, 10, "contact", "new", 1_000).await;
        fx.exec(r#"UPDATE "Conversation" SET "aiContextLastMessageId" = 9, "lastActivityAt" = 1000 WHERE "id" = 6"#)
            .await;
        let context = get_or_init(&fx.deps, 2, 6, &Value::Null)
            .await
            .unwrap()
            .unwrap();
        let ids: Vec<_> = context
            .history
            .iter()
            .map(|m| m.message_id.clone().unwrap())
            .collect();
        assert_eq!(ids, ["10"]);
        assert_eq!(context.summary, "summary");
    }

    #[tokio::test]
    async fn assigns_a_unique_monotonic_seq_to_each_initial_history_entry() {
        let mut fx = Fixture::new().await;
        summarizer(&fx).await;
        with_ai(&mut fx, vec![FakeHttp::openai_text("summary")]);
        message(&fx, 1, "contact", "first", 1_000).await;
        message(&fx, 2, "bot", "second", 1_000).await;
        message(&fx, 3, "contact", "third", 1_000).await;
        let context = get_or_init(&fx.deps, 2, 6, &Value::Null)
            .await
            .unwrap()
            .unwrap();
        let seqs: Vec<_> = context.history.iter().map(|m| m.seq.unwrap()).collect();
        assert_eq!(seqs, [0, 1, 2]);
        assert_eq!(context.next_seq, 3);
        assert_eq!(context.history[1].role, "assistant");
        // Cached: the second read does not summarize again.
        let again = get_or_init(&fx.deps, 2, 6, &Value::Null)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(again.history, context.history);
    }

    #[tokio::test]
    async fn reinitializes_cached_context_when_its_marker_is_stale() {
        let fx = Fixture::new().await;
        message(&fx, 10, "contact", "marker", 1_000).await;
        fx.exec(r#"UPDATE "Conversation" SET "aiContextLastMessageId" = 10, "lastActivityAt" = 1000 WHERE "id" = 6"#)
            .await;
        store(
            &fx,
            json!({ "markerMessageId": null, "summary": "old personal data",
                           "history": [{ "role": "user", "content": "old message" }] }),
        )
        .await;
        let context = get_or_init(&fx.deps, 2, 6, &Value::Null)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(context.marker_message_id.as_deref(), Some("10"));
        assert_eq!(context.summary, "");
        assert!(context.history.is_empty());
    }

    #[tokio::test]
    async fn does_not_read_messages_when_the_conversation_is_outside_the_workspace() {
        let fx = Fixture::new().await;
        assert_eq!(get_or_init(&fx.deps, 99, 6, &Value::Null).await, Ok(None));
        assert!(get(&fx.deps.db, 6).await.is_none());
    }

    #[tokio::test]
    async fn rebuilds_a_legacy_cache_containing_a_system_message_instead_of_forwarding_it() {
        let fx = Fixture::new().await;
        store(&fx, json!({ "markerMessageId": null, "summary": "legacy", "nextSeq": 1,
                           "history": [{ "role": "system", "content": "legacy system instruction" }] }))
        .await;
        let context = get_or_init(&fx.deps, 2, 6, &Value::Null)
            .await
            .unwrap()
            .unwrap();
        assert!(context.history.is_empty());
        assert_eq!(context.summary, "");
    }
}

mod append_history {
    use super::*;

    #[tokio::test]
    async fn enqueues_a_summarize_job_when_history_exceeds_the_trigger_threshold_and_no_job_is_in_flight()
     {
        let fx = Fixture::new().await;
        store(
            &fx,
            json!({ "history": history(100), "summarizing": false }),
        )
        .await;
        let context = append_history(&fx.deps, 6, vec![user("new-1")])
            .await
            .unwrap();
        assert_eq!(summarize_jobs(&fx), 1);
        let job = &fx.jobs.named(SUMMARIZE_CONVERSATION)[0];
        assert_eq!(job.opts.job_id.as_deref(), Some("summarize-6"));
        assert_eq!(job.data["data"]["conversationId"], "6");
        assert!(
            context
                .history
                .iter()
                .any(|m| m.message_id.as_deref() == Some("new-1"))
        );
    }

    #[tokio::test]
    async fn does_not_enqueue_a_second_summarize_job_while_one_is_already_marked_in_flight() {
        let fx = Fixture::new().await;
        store(&fx, json!({ "history": history(100), "summarizing": true })).await;
        let context = append_history(&fx.deps, 6, vec![user("new-1")])
            .await
            .unwrap();
        assert_eq!(summarize_jobs(&fx), 0);
        assert!(context.needs_resummarize);
    }

    #[tokio::test]
    async fn assigns_increasing_seq_values_to_newly_appended_messages_starting_from_next_seq() {
        let fx = Fixture::new().await;
        store(&fx, json!({ "history": [{ "role": "user", "content": "old", "messageId": "old-1", "seq": 0 }],
                           "nextSeq": 1 }))
        .await;
        let context = append_history(&fx.deps, 6, vec![user("new-a"), user("new-b")])
            .await
            .unwrap();
        assert_eq!(context.next_seq, 3);
        let seqs: Vec<_> = context
            .history
            .iter()
            .map(|m| (m.message_id.clone().unwrap(), m.seq.unwrap()))
            .collect();
        assert_eq!(
            seqs,
            [
                ("old-1".into(), 0),
                ("new-a".into(), 1),
                ("new-b".into(), 2)
            ]
        );
    }

    #[tokio::test]
    async fn truncates_history_to_the_hard_cap_regardless_of_the_summarizing_flag() {
        let fx = Fixture::new().await;
        store(&fx, json!({ "history": history(149), "summarizing": true })).await;
        let new: Vec<_> = (0..5).map(|i| user(&format!("new-{i}"))).collect();
        let context = append_history(&fx.deps, 6, new).await.unwrap();
        assert_eq!(context.history.len(), 150);
        assert_eq!(
            context.history.last().unwrap().message_id.as_deref(),
            Some("new-4")
        );
        assert!(
            !context
                .history
                .iter()
                .any(|m| m.message_id.as_deref() == Some("old-0"))
        );
    }

    #[tokio::test]
    async fn skips_a_duplicate_and_does_nothing_without_a_cached_context() {
        let fx = Fixture::new().await;
        assert!(append_history(&fx.deps, 6, vec![user("a")]).await.is_none());
        store(&fx, json!({ "history": [{ "role": "user", "content": "a", "messageId": "a", "seq": 0 }], "nextSeq": 1 }))
            .await;
        let context = append_history(&fx.deps, 6, vec![user("a")]).await.unwrap();
        assert_eq!(context.history.len(), 1);
        assert_eq!(context.next_seq, 1);
    }
}

mod normalize {
    use super::*;

    #[test]
    fn canonicalizes_new_file_parts_and_preserves_their_mime_type() {
        let parts = json!([
            { "type": "text", "text": "describe this" },
            { "type": "file", "data": "https://example.com/photo.jpeg", "mediaType": "image/jpeg" },
        ]);
        let message = normalize_message(AiMessage {
            role: "user".into(),
            message_id: None,
            created_at: None,
            seq: None,
            content: parts.clone(),
        })
        .unwrap();
        assert_eq!(message.content, parts);
        assert!(message.message_id.is_some());
        assert_eq!(
            model_messages(std::slice::from_ref(&message)),
            [("user".to_owned(), parts)]
        );
        assert_eq!(
            turns(&[message]),
            [llm::Turn::User(vec![
                llm::Part::Text("describe this".into()),
                llm::Part::File {
                    url: "https://example.com/photo.jpeg".into(),
                    media_type: "image/jpeg".into()
                },
            ])]
        );
    }

    #[test]
    fn drops_system_and_tool_turns_instead_of_emitting_empty_model_messages() {
        let history: Vec<AiMessage> = serde_json::from_value(json!([
            { "role": "system", "content": "legacy instruction" },
            { "role": "user", "content": "hi" },
            { "role": "tool", "content": "tool output" },
            { "role": "assistant", "content": "hello" },
        ]))
        .unwrap();
        assert_eq!(
            model_messages(&history),
            [
                ("user".to_owned(), json!("hi")),
                ("assistant".to_owned(), json!("hello"))
            ]
        );
    }

    #[test]
    fn canonicalizes_legacy_image_parts_to_file_parts_with_their_mime_type() {
        let history: Vec<AiMessage> = serde_json::from_value(json!([{ "role": "user", "content": [
            { "type": "image", "image": "https://example.com/a.webp", "mimeType": "image/webp" },
            { "type": "image", "image": "data:image/gif;base64,AAAA" },
        ] }]))
        .unwrap();
        assert_eq!(
            model_messages(&history)[0].1,
            json!([
                { "type": "file", "data": "https://example.com/a.webp", "mediaType": "image/webp" },
                { "type": "file", "data": "data:image/gif;base64,AAAA", "mediaType": "image/gif" },
            ])
        );
    }
}

/// `summarize-conversation.test.ts`.
mod summarize_conversation {
    use super::*;

    fn twenty_five(summarizing: bool, started_at: Option<i64>) -> Value {
        json!({
            "summarizing": summarizing,
            "summarizingStartedAt": started_at,
            "summary": "old",
            "history": (0..25).map(|i| json!({ "role": "user", "content": format!("msg-{i}"), "messageId": format!("msg-{i}") })).collect::<Vec<_>>(),
        })
    }

    async fn run(fx: &Fixture) -> Result<(), chatbotx_jobs::JobError> {
        handle_summarize(&fx.deps, &json!({ "conversationId": "6" })).await
    }

    #[tokio::test]
    async fn does_not_re_enter_summarization_while_summarizing_is_fresh() {
        let mut fx = Fixture::new().await;
        let http = with_ai(&mut fx, vec![]);
        store(
            &fx,
            twenty_five(true, Some(chatbotx_db::repo::now_ms() - 1000)),
        )
        .await;
        run(&fx).await.unwrap();
        let context = get(&fx.deps.db, 6).await.unwrap();
        assert!(context.needs_resummarize);
        assert!(context.summarizing);
        assert_eq!(context.history.len(), 25);
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn self_heals_from_a_stale_summarizing_flag_left_by_a_crashed_job() {
        let mut fx = Fixture::new().await;
        summarizer(&fx).await;
        let http = with_ai(&mut fx, vec![FakeHttp::openai_text("recovered summary")]);
        store(
            &fx,
            twenty_five(true, Some(chatbotx_db::repo::now_ms() - 10 * 60 * 1000)),
        )
        .await;
        run(&fx).await.unwrap();
        let context = get(&fx.deps.db, 6).await.unwrap();
        assert_eq!(context.summary, "recovered summary");
        assert!(!context.summarizing);
        assert_eq!(context.history.len(), 15);
        assert_eq!(context.history[0].message_id.as_deref(), Some("msg-10"));
        // The existing summary is updated with the first ten messages.
        let prompt = http.requests()[0].1["messages"][0]["content"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            prompt.contains("old") && prompt.contains("user: msg-9") && !prompt.contains("msg-10")
        );
    }

    #[tokio::test]
    async fn succeeds_normally_and_leaves_summarizing_false() {
        let mut fx = Fixture::new().await;
        summarizer(&fx).await;
        with_ai(&mut fx, vec![FakeHttp::openai_text("new summary")]);
        store(&fx, twenty_five(false, None)).await;
        run(&fx).await.unwrap();
        let context = get(&fx.deps.db, 6).await.unwrap();
        assert_eq!(context.summary, "new summary");
        assert!(!context.summarizing && !context.needs_resummarize);
        assert_eq!(context.summarizing_started_at, None);
        // 15 left (<= 20): no follow-up job.
        assert_eq!(summarize_jobs(&fx), 0);
    }

    #[tokio::test]
    async fn logs_and_does_not_throw_when_the_follow_up_requeue_fails() {
        let mut fx = Fixture::new().await;
        summarizer(&fx).await;
        with_ai(&mut fx, vec![FakeHttp::openai_text("s1")]);
        let mut big = twenty_five(false, None);
        big["history"] = history(40);
        store(&fx, big).await;
        fx.jobs.fail_next();
        assert!(run(&fx).await.is_ok());
        let context = get(&fx.deps.db, 6).await.unwrap();
        assert_eq!(context.history.len(), 30);
        assert!(!context.summarizing);
    }

    #[tokio::test]
    async fn requeues_while_more_than_twenty_messages_are_left() {
        let mut fx = Fixture::new().await;
        summarizer(&fx).await;
        with_ai(&mut fx, vec![FakeHttp::openai_text("s1")]);
        let mut big = twenty_five(false, None);
        big["history"] = history(40);
        store(&fx, big).await;
        run(&fx).await.unwrap();
        assert_eq!(summarize_jobs(&fx), 1);
    }

    #[tokio::test]
    async fn resets_the_flags_at_or_under_the_history_limit() {
        let fx = Fixture::new().await;
        let mut small = twenty_five(true, Some(0));
        small["history"] = history(5);
        small["needsResummarize"] = json!(true);
        store(&fx, small).await;
        run(&fx).await.unwrap();
        let context = get(&fx.deps.db, 6).await.unwrap();
        assert!(!context.summarizing && !context.needs_resummarize);
    }
}
