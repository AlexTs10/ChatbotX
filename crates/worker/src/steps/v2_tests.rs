//! Ported step tests of Q.P7-V2 (`questionnaires`). As `u4_tests.rs`: the
//! step runs through `runFlowNode` on a migrated SQLite file. Node `n1` holds
//! one `questionnaires` step (`q`) of questionnaire 40 (questions 41: email,
//! with an image, and 42: text). Adapted: the mocked services become rows;
//! "chatQueueAdd was called with" becomes the recorded `sendChatMessage` job.

use serde_json::{Value, json};

use super::test_support::*;

const CHAT: &str = "sendChatMessage";

async fn questionnaire(fx: &Fixture) {
    fx.exec(
        r#"INSERT INTO "Questionnaire" ("id", "workspaceId", "name", "enableCustomFieldMapping") VALUES (40, 2, 'Survey', 0);
           INSERT INTO "QuestionnaireQuestion" ("id", "questionnaireId", "title", "type", "orderNo", "image", "retryMessage")
             VALUES (41, 40, 'What is your email?', 'email', 0, '{"mode":"url","url":"https://example.com/question.png"}',
                     'Please enter a valid email address');
           INSERT INTO "QuestionnaireQuestion" ("id", "questionnaireId", "title", "type", "orderNo")
             VALUES (42, 40, 'What is your name?', 'text', 1);"#,
    )
    .await;
}

async fn step(fx: &Fixture, mode: &str) {
    fx.node(
        json!([{ "id": "q", "stepType": "questionnaires", "mode": mode, "questionnaireId": "40" }]),
        json!([]),
    )
    .await;
}

async fn setup(fx: &Fixture) {
    questionnaire(fx).await;
    step(fx, "start").await;
}

/// An in-progress submission at `current`.
async fn submission(fx: &Fixture, status: &'static str, current: Option<i64>) {
    fx.sql(move |tx| {
        tx.execute(
            r#"INSERT INTO "QuestionnaireSubmission" ("id", "workspaceId", "questionnaireId", "contactId", "conversationId",
                 "status", "currentQuestionId", "currentQuestionSentAt")
               VALUES (43, 2, 40, 3, 6, ?1, ?2, 1767225600000)"#,
            chatbotx_db::rusqlite::params![status, current],
        )?;
        Ok(())
    })
    .await;
}

async fn set_challenge(fx: &Fixture, challenge: Value) {
    let attributes = json!({ "challenge": challenge }).to_string();
    fx.sql(move |tx| {
        tx.execute(
            r#"UPDATE "Conversation" SET "additionalAttributes" = ?1 WHERE "id" = 6"#,
            [attributes],
        )?;
        Ok(())
    })
    .await;
}

fn pending() -> Value {
    json!({ "type": "step", "data": { "stepId": "q" } })
}

async fn challenge(fx: &Fixture) -> Value {
    fx.text(r#"SELECT json_extract("additionalAttributes", '$.challenge') FROM "Conversation" WHERE "id" = 6"#)
        .await
        .map(|c| serde_json::from_str(&c).unwrap())
        .unwrap_or(Value::Null)
}

/// A contact message of conversation 6.
async fn incoming(fx: &Fixture, id: i64, text: &'static str, created_at: i64) {
    fx.sql(move |tx| {
        tx.execute(
            r#"INSERT INTO "Message" ("id", "workspaceId", "conversationId", "contactInboxId", "senderType", "messageType", "contentType", "text", "createdAt")
               VALUES (?1, 2, 6, 5, 'contact', 'incoming', 'text', ?2, ?3)"#,
            chatbotx_db::rusqlite::params![id, text, created_at],
        )?;
        tx.execute(r#"UPDATE "Conversation" SET "lastActivityAt" = ?1 WHERE "id" = 6"#, [created_at])?;
        Ok(())
    })
    .await;
}

fn chats(fx: &Fixture) -> Vec<Value> {
    fx.jobs
        .named(CHAT)
        .into_iter()
        .map(|j| j.data["data"].clone())
        .collect()
}

async fn submission_status(fx: &Fixture) -> Option<String> {
    fx.text(r#"SELECT "status" FROM "QuestionnaireSubmission" WHERE "id" = 43"#)
        .await
}

/// `questionnaire-engine.test.ts`.
mod questionnaire_engine {
    use super::*;

    #[tokio::test]
    async fn sends_question_image_in_the_same_chat_job_and_stores_zero_attempts_on_first_send() {
        let fx = Fixture::new().await;
        setup(&fx).await;
        fx.run(json!({ "isBulkBroadcast": true })).await.unwrap();
        let sent = chats(&fx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["text"], "What is your email?");
        assert_eq!(sent[0]["url"], "https://example.com/question.png");
        assert_eq!(sent[0]["isBulkBroadcast"], true);
        let stored = challenge(&fx).await;
        assert_eq!(stored["type"], "step");
        assert_eq!(stored["data"]["attempts"], 0);
        assert_eq!(stored["data"]["stepId"], "q");
        assert_eq!(stored["data"]["nodeId"], "n1");
        assert_eq!(
            fx.int(
                r#"SELECT "currentQuestionId" FROM "QuestionnaireSubmission" WHERE "contactId" = 3"#
            )
            .await,
            Some(41)
        );
    }

    #[tokio::test]
    async fn offers_messengers_native_email_quick_reply_for_an_email_question() {
        let mut fx = Fixture::new().await;
        fx.on_channel("messenger").await;
        setup(&fx).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            chats(&fx)[0]["quickReplies"],
            json!([{ "id": "messenger:native-quick-reply:user_email", "label": "What is your email?",
                     "buttonType": "postback", "postback": "messenger:native-quick-reply:user_email" }])
        );
    }

    #[tokio::test]
    async fn offers_messengers_native_phone_quick_reply_for_a_phone_question() {
        let mut fx = Fixture::new().await;
        fx.on_channel("messenger").await;
        setup(&fx).await;
        fx.exec(r#"UPDATE "QuestionnaireQuestion" SET "type" = 'phone', "title" = 'What is your phone?' WHERE "id" = 41"#)
            .await;
        fx.run(json!({})).await.unwrap();
        let replies = &chats(&fx)[0]["quickReplies"];
        assert_eq!(
            replies[0]["id"],
            "messenger:native-quick-reply:user_phone_number"
        );
        assert_eq!(
            replies[0]["postback"],
            "messenger:native-quick-reply:user_phone_number"
        );
    }

    #[tokio::test]
    async fn does_not_offer_a_native_quick_reply_outside_messenger() {
        let fx = Fixture::new().await;
        setup(&fx).await;
        fx.run(json!({})).await.unwrap();
        assert!(chats(&fx)[0].get("quickReplies").is_none());
    }

    #[tokio::test]
    async fn stops_the_flow_when_the_contact_already_submitted_the_questionnaire() {
        let fx = Fixture::new().await;
        setup(&fx).await;
        submission(&fx, "completed", None).await;
        fx.run(json!({})).await.unwrap();
        assert!(chats(&fx).is_empty());
        assert_eq!(challenge(&fx).await, Value::Null);
    }

    #[tokio::test]
    async fn starts_the_configured_questionnaire_when_a_stale_challenge_has_no_active_submission() {
        let fx = Fixture::new().await;
        setup(&fx).await;
        set_challenge(
            &fx,
            json!({ "type": "step", "data": { "stepId": "old-step" } }),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(chats(&fx).len(), 1);
        assert_eq!(challenge(&fx).await["data"]["stepId"], "q");
        assert_eq!(
            fx.text(r#"SELECT "status" FROM "QuestionnaireSubmission" WHERE "contactId" = 3"#)
                .await
                .as_deref(),
            Some("inProgress")
        );
    }

    #[tokio::test]
    async fn ends_an_active_questionnaire_and_clears_the_challenge() {
        let fx = Fixture::new().await;
        questionnaire(&fx).await;
        step(&fx, "end").await;
        submission(&fx, "inProgress", Some(41)).await;
        set_challenge(&fx, pending()).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(submission_status(&fx).await.as_deref(), Some("cancelled"));
        assert_eq!(challenge(&fx).await, Value::Null);
    }

    #[tokio::test]
    async fn deletes_an_applicant_and_clears_the_challenge() {
        let fx = Fixture::new().await;
        questionnaire(&fx).await;
        step(&fx, "deleteApplicant").await;
        submission(&fx, "inProgress", Some(41)).await;
        set_challenge(&fx, pending()).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(submission_status(&fx).await, None);
        assert_eq!(challenge(&fx).await, Value::Null);
    }

    #[tokio::test]
    async fn retries_invalid_answers_with_incremented_challenge_attempts() {
        let fx = Fixture::new().await;
        setup(&fx).await;
        submission(&fx, "inProgress", Some(41)).await;
        set_challenge(&fx, pending()).await;
        incoming(&fx, 80, "not-an-email", chatbotx_db::repo::now_ms()).await;
        fx.run(json!({ "stepChallenge": { "attempts": 1 } }))
            .await
            .unwrap();
        assert_eq!(challenge(&fx).await["data"]["attempts"], 2);
        assert_eq!(chats(&fx)[0]["text"], "Please enter a valid email address");
        assert_eq!(
            fx.int(r#"SELECT "currentQuestionId" FROM "QuestionnaireSubmission" WHERE "id" = 43"#)
                .await,
            Some(41)
        );
    }

    #[tokio::test]
    async fn uses_the_trigger_message_when_challenge_job_carries_the_message_partition_key() {
        let fx = Fixture::new().await;
        setup(&fx).await;
        fx.exec(r#"UPDATE "QuestionnaireQuestion" SET "deletedAt" = 1 WHERE "id" = 42"#)
            .await;
        submission(&fx, "inProgress", Some(41)).await;
        set_challenge(&fx, pending()).await;
        let now = chatbotx_db::repo::now_ms();
        incoming(&fx, 80, "valid@example.com", now - 1_000).await;
        // A later message is not the answer: the trigger message is.
        incoming(&fx, 81, "later", now).await;
        fx.run(json!({ "triggerMessageId": "80", "triggerMessageCreatedAt": now - 1_000 }))
            .await
            .unwrap();
        assert_eq!(submission_status(&fx).await.as_deref(), Some("completed"));
        assert_eq!(
            fx.text(r#"SELECT "value" FROM "QuestionnaireAnswer" WHERE "submissionId" = 43"#)
                .await
                .as_deref(),
            Some(r#""valid@example.com""#)
        );
        assert_eq!(
            fx.int(
                r#"SELECT "lastAnsweredMessageId" FROM "QuestionnaireSubmission" WHERE "id" = 43"#
            )
            .await,
            Some(80)
        );
        assert!(fx.jobs.named("sendFlow").is_empty());
    }

    #[tokio::test]
    async fn resends_the_current_question_when_answer_processing_was_already_advanced_for_the_trigger_message()
     {
        let fx = Fixture::new().await;
        setup(&fx).await;
        submission(&fx, "inProgress", Some(42)).await;
        fx.exec(
            r#"UPDATE "QuestionnaireSubmission" SET "lastAnsweredMessageId" = 80 WHERE "id" = 43"#,
        )
        .await;
        set_challenge(&fx, pending()).await;
        fx.run(json!({ "triggerMessageId": "80" })).await.unwrap();
        assert_eq!(chats(&fx)[0]["text"], "What is your name?");
        assert_eq!(
            fx.int(
                r#"SELECT "currentQuestionSentAt" FROM "QuestionnaireSubmission" WHERE "id" = 43"#
            )
            .await,
            Some(1_767_225_600_000)
        );
        assert_eq!(
            fx.int(r#"SELECT COUNT(*) FROM "QuestionnaireAnswer""#)
                .await,
            Some(0)
        );
    }

    #[tokio::test]
    async fn starts_the_configured_trigger_flow_after_questionnaire_completion() {
        let fx = Fixture::new().await;
        setup(&fx).await;
        fx.exec(r#"UPDATE "Questionnaire" SET "triggerFlowId" = 10 WHERE "id" = 40"#)
            .await;
        submission(&fx, "inProgress", Some(42)).await;
        set_challenge(&fx, pending()).await;
        incoming(&fx, 80, "Ada", chatbotx_db::repo::now_ms()).await;
        fx.run(json!({ "sendFrom": "inbox" })).await.unwrap();
        let flows = fx.jobs.send_flows();
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0]["conversationId"], "6");
        assert_eq!(flows[0]["contactInboxId"], "5");
        assert_eq!(flows[0]["flowId"], "10");
        assert_eq!(flows[0]["sendFrom"], "inbox");
        assert_eq!(challenge(&fx).await, Value::Null);
    }

    #[tokio::test]
    async fn scores_the_leaderboard_survey_rule_when_a_questionnaire_completes() {
        let mut fx = Fixture::new().await;
        fx.on_channel("whatsapp").await;
        setup(&fx).await;
        let now = chatbotx_db::repo::now_ms();
        fx.sql(move |tx| {
            tx.execute(
                r#"INSERT INTO "Leaderboard" ("id", "workspaceId", "name", "startsAt", "endsAt", "status", "platform", "inboxId", "scope", "rules", "tagIds", "rewardSettings")
                   VALUES (93, 2, 'Board', ?1, ?2, 'active', 'whatsapp', 4, '{}', '{"survey":{"enabled":true,"points":1}}', '[]', '{}')"#,
                chatbotx_db::rusqlite::params![now - 86_400_000, now + 86_400_000],
            )?;
            Ok(())
        })
        .await;
        submission(&fx, "inProgress", Some(42)).await;
        set_challenge(&fx, pending()).await;
        incoming(&fx, 80, "Ada", now).await;
        fx.run(json!({})).await.unwrap();
        let awards = fx.jobs.named("award");
        assert_eq!(awards.len(), 1, "{:?}", fx.jobs.all());
        let award = &awards[0].data["data"];
        assert_eq!(award["rule"], "survey");
        assert_eq!(award["inboxId"], "4");
        assert_eq!(award["contactId"], "3");
        assert_eq!(award["sourceRef"], "questionnaire:43");
    }

    // "parses serialized conversation timestamps" and "falls back to a
    // concrete sinceTime": not-applicable, the Rust conversation row holds
    // typed epoch-ms timestamps (the lookback is covered by the retry test).

    #[tokio::test]
    async fn clears_challenge_and_returns_success_when_the_current_question_is_missing() {
        let fx = Fixture::new().await;
        setup(&fx).await;
        submission(&fx, "inProgress", Some(41)).await;
        fx.exec(r#"UPDATE "QuestionnaireQuestion" SET "deletedAt" = 1 WHERE "id" = 41"#)
            .await;
        set_challenge(&fx, pending()).await;
        incoming(&fx, 80, "a@b.co", chatbotx_db::repo::now_ms()).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(challenge(&fx).await, Value::Null);
        assert!(chats(&fx).is_empty());
    }
}

/// `questionnaires-handler.test.ts`.
mod questionnaires_handler {
    use super::*;

    #[tokio::test]
    async fn clears_the_challenge_and_returns_error_when_the_engine_throws() {
        // The engine fails at the chat send; a stray challenge is cleared.
        let fx = Fixture::new().await;
        setup(&fx).await;
        set_challenge(&fx, json!({ "type": "quickReply", "data": {} })).await;
        fx.jobs.fail_next();
        fx.run(json!({})).await.unwrap();
        assert_eq!(challenge(&fx).await, Value::Null);
        assert!(chats(&fx).is_empty());
    }
}
