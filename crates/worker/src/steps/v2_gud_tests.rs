//! Ported cases of `get-user-data.test.ts` (Q.P7-V2). As `u4_tests.rs`: the
//! step runs through `runFlowNode` on a migrated SQLite file. Node `n1` holds
//! one `getUserData` step (`g`) whose states route success → n2, skip → n3,
//! error → n4; its output field is custom field 30. Adapted: the mocked
//! services become rows; "updateChallenge was called with" becomes the stored
//! challenge; the run's `ctx.variables.conversation` is its `stepChallenge`.
//! Not ported here: the lazy attachment store cases (no store in Rust,
//! DECISIONS), the signed date-picker link (needs `ENCRYPTION_KEY`; the token
//! itself is tested in crates/core), and the cases that only assert a mock's
//! call order.

use chatbotx_db::repo::{iso, now_ms};
use serde_json::{Value, json};

use super::test_support::*;

const CHAT: &str = "sendChatMessage";
const FLOW_MESSAGE: &str = "sendFlowMessage";
const ID: &str = "4242";

fn gud(overrides: Value) -> Value {
    let mut step = json!({
        "id": "g", "stepType": "getUserData", "message": "Please enter your email", "replyFormat": "RF03",
        "outputFieldId": "30", "retryMessage": "Please try again", "skipButtonLabel": "Skip", "autoSkip": false,
        "autoSkipTimeUnit": "hours", "autoSkipTimeValue": 1, "autoSkipFailAttempts": 3,
        "states": [{ "id": "ok", "stateType": "success" }, { "id": "sk", "stateType": "skip" },
                   { "id": "er", "stateType": "error" }],
    });
    for (k, v) in overrides.as_object().cloned().unwrap_or_default() {
        step[k] = v;
    }
    step
}

async fn fixture(overrides: Value) -> Fixture {
    let fx = Fixture::new().await;
    flow(&fx, overrides).await;
    fx
}

async fn flow(fx: &Fixture, overrides: Value) {
    let empty = |id: &str| json!({ "id": id, "data": { "details": { "steps": [] } } });
    fx.flow(
        json!([{ "id": "n1", "data": { "isStartNode": true, "details": { "steps": [gud(overrides)] } } },
               empty("n2"), empty("n3"), empty("n4")]),
        json!([{ "sourceHandle": "ok", "target": "n2" }, { "sourceHandle": "sk", "target": "n3" },
               { "sourceHandle": "er", "target": "n4" }]),
    )
    .await;
    fx.exec(r#"INSERT OR IGNORE INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (30, 'answer', 'shortText', 2);"#)
        .await;
}

/// This step's pending challenge (`challengeId` `None` = a legacy one).
fn challenge(attempts: i64, challenge_id: Option<&str>, started_at: i64) -> Value {
    let mut data = json!({
        "flowId": "10", "flowVersionId": "11", "nodeId": "n1", "stepId": "g", "attempts": attempts,
        "lastAttemptAt": iso(now_ms()), "startedAt": iso(started_at),
    });
    if let Some(id) = challenge_id {
        data["challengeId"] = json!(id);
    }
    json!({ "type": "step", "data": data })
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

async fn stored(fx: &Fixture) -> Value {
    fx.text(r#"SELECT json_extract("additionalAttributes", '$.challenge') FROM "Conversation" WHERE "id" = 6"#)
        .await
        .map(|c| serde_json::from_str(&c).unwrap())
        .unwrap_or(Value::Null)
}

async fn message(
    fx: &Fixture,
    id: i64,
    content_type: &'static str,
    text: Option<&'static str>,
    attrs: Option<Value>,
    at: i64,
) {
    let attrs = attrs.map(|a| a.to_string());
    fx.sql(move |tx| {
        tx.execute(
            r#"INSERT INTO "Message" ("id", "workspaceId", "conversationId", "contactInboxId", "senderType", "messageType",
                 "contentType", "text", "contentAttributes", "createdAt")
               VALUES (?1, 2, 6, 5, 'contact', 'incoming', ?2, ?3, ?4, ?5)"#,
            chatbotx_db::rusqlite::params![id, content_type, text, attrs, at],
        )?;
        tx.execute(r#"UPDATE "Conversation" SET "lastActivityAt" = ?1 WHERE "id" = 6"#, [at])?;
        Ok(())
    })
    .await;
}

async fn reply(fx: &Fixture, text: &'static str) {
    message(fx, 80, "text", Some(text), None, now_ms()).await;
}

/// A typed reply re-entering the step at `attempts` of cycle `ID`.
async fn answer(fx: &Fixture, attempts: i64) -> Result<(), chatbotx_jobs::JobError> {
    set_challenge(fx, challenge(attempts, Some(ID), now_ms())).await;
    run_answer(fx, attempts, Some(ID), now_ms(), json!({})).await
}

async fn run_answer(
    fx: &Fixture,
    attempts: i64,
    challenge_id: Option<&str>,
    started_at: i64,
    extra: Value,
) -> Result<(), chatbotx_jobs::JobError> {
    let mut vars = json!({ "attempts": attempts, "lastAttemptAt": iso(now_ms()), "startedAt": iso(started_at) });
    if let Some(id) = challenge_id {
        vars["challengeId"] = json!(id);
    }
    let mut data = json!({ "startFromStepId": "g", "stepChallenge": vars });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        data[k] = v;
    }
    fx.run(data).await
}

fn routed(fx: &Fixture) -> Option<String> {
    fx.jobs
        .send_flows()
        .first()
        .and_then(|j| j["nodeId"].as_str().map(str::to_owned))
}

async fn field(fx: &Fixture) -> Option<String> {
    fx.text(r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = 3 AND "customFieldId" = 30"#)
        .await
}

async fn last_input_failure(fx: &Fixture) -> Option<String> {
    fx.text(r#"SELECT "lastInputFailure" FROM "ContactInbox" WHERE "id" = 5"#)
        .await
}

/// The `sendFlowMessage` steps' texts, in order.
fn flow_texts(fx: &Fixture) -> Vec<String> {
    fx.jobs
        .named(FLOW_MESSAGE)
        .iter()
        .map(|j| {
            j.data["data"]["step"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

fn flow_messages(fx: &Fixture) -> Vec<Value> {
    fx.jobs
        .named(FLOW_MESSAGE)
        .into_iter()
        .map(|j| j.data["data"].clone())
        .collect()
}

fn chats(fx: &Fixture) -> Vec<Value> {
    fx.jobs
        .named(CHAT)
        .into_iter()
        .map(|j| j.data["data"].clone())
        .collect()
}

/// `getUserData — validation logic`.
mod validation_logic {
    use super::*;

    async fn judged(format: &str, text: &'static str) -> Fixture {
        let fx = fixture(json!({ "replyFormat": format })).await;
        reply(&fx, text).await;
        answer(&fx, 1).await.unwrap();
        fx
    }

    async fn accepted(format: &str, text: &'static str, saved: &str) {
        let fx = judged(format, text).await;
        assert_eq!(routed(&fx).as_deref(), Some("n2"), "{format} {text}");
        assert_eq!(field(&fx).await.as_deref(), Some(saved), "{format} {text}");
    }

    async fn retried(format: &str, text: &'static str) {
        let fx = judged(format, text).await;
        assert_eq!(routed(&fx), None, "{format} {text}");
        assert_eq!(field(&fx).await, None, "{format} {text}");
        assert_eq!(stored(&fx).await["data"]["attempts"], 2, "{format} {text}");
    }

    #[tokio::test]
    async fn anchors_the_message_lookup_on_conversation_last_activity_at() {
        // An answer older than a year before the last activity is out of range.
        let fx = fixture(json!({})).await;
        let now = now_ms();
        message(
            &fx,
            79,
            "text",
            Some("old@example.com"),
            None,
            now - 400 * 86_400_000,
        )
        .await;
        fx.exec(r#"UPDATE "Conversation" SET "lastActivityAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER) WHERE "id" = 6"#)
            .await;
        answer(&fx, 1).await.unwrap();
        assert_eq!(field(&fx).await, None);
    }

    #[tokio::test]
    async fn email_format() {
        accepted("RF03", "user@example.com", "user@example.com").await;
        retried("RF03", "not-an-email").await;
    }

    #[tokio::test]
    async fn number_format() {
        accepted("RF01", "42", "42").await;
        accepted("RF01", "3.14", "3.14").await;
        retried("RF01", "abc").await;
    }

    #[tokio::test]
    async fn phone_format() {
        accepted("RF04", "+1-555-123-4567", "+1-555-123-4567").await;
        retried("RF04", "hello").await;
    }

    #[tokio::test]
    async fn link_format() {
        accepted("RF07", "https://example.com", "https://example.com").await;
        retried("RF07", "not a url").await;
    }

    #[tokio::test]
    async fn default_free_text_format() {
        accepted("RF02", "anything goes", "anything goes").await;
    }

    #[tokio::test]
    async fn attachment_with_text_based_format_returns_retry_even_with_text() {
        let fx = fixture(json!({ "replyFormat": "RF03" })).await;
        reply(&fx, "user@example.com").await;
        fx.exec(
            r#"INSERT INTO "Attachment" ("id", "workspaceId", "conversationId", "messageId", "messageCreatedAt", "fileType", "mimeType", "originPath")
               SELECT 90, 2, 6, 80, "createdAt", 'image', 'image/jpeg', '/image.jpg' FROM "Message" WHERE "id" = 80;"#,
        )
        .await;
        answer(&fx, 1).await.unwrap();
        assert_eq!(field(&fx).await, None);
        assert_eq!(stored(&fx).await["data"]["attempts"], 2);
    }

    #[tokio::test]
    async fn re_prompts_when_the_attachment_is_permanently_unavailable() {
        let fx = fixture(json!({ "replyFormat": "RF05" })).await;
        message(&fx, 80, "text", None, None, now_ms()).await;
        fx.exec(
            r#"INSERT INTO "Attachment" ("id", "workspaceId", "conversationId", "messageId", "messageCreatedAt", "fileType", "mimeType", "originPath")
               SELECT 90, 2, 6, 80, "createdAt", 'image', 'image/jpeg', 'failed:https://cdn.example/photo.jpg' FROM "Message" WHERE "id" = 80;"#,
        )
        .await;
        answer(&fx, 1).await.unwrap();
        assert_eq!(field(&fx).await, None);
        assert_eq!(routed(&fx), None);
    }

    #[tokio::test]
    async fn location_pin_returns_success_with_lat_lng() {
        let fx = fixture(json!({ "replyFormat": "RF08" })).await;
        message(
            &fx,
            80,
            "location",
            None,
            Some(json!({ "latitude": 10.5, "longitude": 106.75 })),
            now_ms(),
        )
        .await;
        answer(&fx, 1).await.unwrap();
        assert_eq!(field(&fx).await.as_deref(), Some("10.5,106.75"));
    }

    #[tokio::test]
    async fn typed_coordinate_pair_returns_success_and_plain_text_retries() {
        accepted("RF08", "10.5, 106.75", "10.5,106.75").await;
        retried("RF08", "Hanoi").await;
    }

    #[tokio::test]
    async fn no_last_message_returns_retry() {
        let fx = fixture(json!({})).await;
        answer(&fx, 1).await.unwrap();
        assert_eq!(routed(&fx), None);
        assert_eq!(stored(&fx).await["data"]["attempts"], 2);
    }
}

/// `getUserData — attempt counter`.
mod attempt_counter {
    use super::*;

    #[tokio::test]
    async fn increments_attempts_from_1_to_2_and_from_2_to_3() {
        for from in [1, 2] {
            let fx = fixture(json!({})).await;
            reply(&fx, "bad").await;
            answer(&fx, from).await.unwrap();
            assert_eq!(stored(&fx).await["data"]["attempts"], from + 1);
            assert_eq!(stored(&fx).await["data"]["challengeId"], ID);
        }
    }

    #[tokio::test]
    async fn sends_the_retry_message_first_then_the_question_again() {
        let fx = fixture(json!({})).await;
        reply(&fx, "bad").await;
        answer(&fx, 1).await.unwrap();
        assert_eq!(
            flow_texts(&fx),
            vec!["Please try again", "Please enter your email"]
        );
    }

    #[tokio::test]
    async fn a_blank_retry_message_sends_only_the_question_again() {
        let fx = fixture(json!({ "retryMessage": "   " })).await;
        reply(&fx, "bad").await;
        answer(&fx, 1).await.unwrap();
        assert_eq!(flow_texts(&fx), vec!["Please enter your email"]);
    }

    #[tokio::test]
    async fn sends_the_retry_message_without_a_skip_button_then_the_question_with_one() {
        let fx = fixture(json!({})).await;
        reply(&fx, "bad").await;
        answer(&fx, 1).await.unwrap();
        let sent = flow_messages(&fx);
        assert!(sent[0].get("presetQuickReplies").is_none());
        assert_eq!(sent[1]["presetQuickReplies"][0]["label"], "Skip");
    }

    #[tokio::test]
    async fn the_loser_of_two_concurrent_invalid_replies_sends_nothing() {
        let fx = fixture(json!({})).await;
        reply(&fx, "bad").await;
        // The stored attempt already moved on: the compare-and-set loses.
        set_challenge(&fx, challenge(2, Some(ID), now_ms())).await;
        run_answer(&fx, 1, Some(ID), now_ms(), json!({}))
            .await
            .unwrap();
        assert!(fx.jobs.named(FLOW_MESSAGE).is_empty());
        assert_eq!(stored(&fx).await["data"]["attempts"], 2);
    }

    #[tokio::test]
    async fn a_legacy_challenge_is_upgraded_in_place_and_its_skip_button_carries_the_new_id() {
        let fx = fixture(json!({})).await;
        reply(&fx, "bad").await;
        set_challenge(&fx, challenge(1, None, now_ms())).await;
        run_answer(&fx, 1, None, now_ms(), json!({})).await.unwrap();
        let upgraded = stored(&fx).await;
        let id = upgraded["data"]["challengeId"]
            .as_str()
            .expect("new id")
            .to_owned();
        assert_eq!(upgraded["data"]["attempts"], 2);
        let postback = flow_messages(&fx)[1]["presetQuickReplies"][0]["postback"].clone();
        assert_eq!(
            chatbotx_core::flow_config::step_input_resolution::decode_step_input_skip_postback(
                postback.as_str()
            ),
            Some(id)
        );
    }
}

/// `getUserData — auto-skip` and `Auto Skip deadline`.
mod auto_skip {
    use super::*;

    #[tokio::test]
    async fn records_timeout_when_skipping_after_auto_skip_time_elapses() {
        let fx = fixture(json!({ "autoSkip": true })).await;
        reply(&fx, "bad").await;
        let two_hours_ago = now_ms() - 2 * 3_600_000;
        set_challenge(&fx, challenge(1, Some(ID), two_hours_ago)).await;
        run_answer(&fx, 1, Some(ID), two_hours_ago, json!({}))
            .await
            .unwrap();
        assert_eq!(routed(&fx).as_deref(), Some("n3"));
        assert_eq!(last_input_failure(&fx).await.as_deref(), Some("timeout"));
        assert_eq!(stored(&fx).await, Value::Null);
    }

    #[tokio::test]
    async fn skips_after_exceeding_max_attempts() {
        let fx = fixture(json!({ "autoSkip": true })).await;
        reply(&fx, "bad").await;
        answer(&fx, 3).await.unwrap();
        assert_eq!(routed(&fx).as_deref(), Some("n3"));
        assert_eq!(
            last_input_failure(&fx).await.as_deref(),
            Some("invalid_input_attempts")
        );
    }

    #[tokio::test]
    async fn a_deadline_still_in_the_future_does_not_skip() {
        let fx = fixture(json!({ "autoSkip": true })).await;
        reply(&fx, "bad").await;
        answer(&fx, 1).await.unwrap();
        assert_eq!(routed(&fx), None);
        assert_eq!(stored(&fx).await["data"]["attempts"], 2);
    }

    #[tokio::test]
    async fn an_auto_skip_that_lost_the_claim_records_no_failure() {
        let fx = fixture(json!({ "autoSkip": true })).await;
        reply(&fx, "bad").await;
        set_challenge(&fx, challenge(3, Some("newer"), now_ms())).await;
        run_answer(&fx, 3, Some(ID), now_ms(), json!({}))
            .await
            .unwrap();
        assert_eq!(routed(&fx), None);
        assert_eq!(last_input_failure(&fx).await, None);
        assert_eq!(stored(&fx).await["data"]["challengeId"], "newer");
    }
}

/// `getUserData — challenge lifecycle` and `claim first`.
mod challenge_lifecycle {
    use super::*;

    #[tokio::test]
    async fn clears_challenge_after_successful_input() {
        let fx = fixture(json!({})).await;
        reply(&fx, "user@example.com").await;
        fx.exec(r#"UPDATE "ContactInbox" SET "lastInputFailure" = 'timeout' WHERE "id" = 5"#)
            .await;
        answer(&fx, 1).await.unwrap();
        assert_eq!(stored(&fx).await, Value::Null);
        assert_eq!(last_input_failure(&fx).await, None);
    }

    #[tokio::test]
    async fn keeps_challenge_while_retrying_invalid_input() {
        let fx = fixture(json!({})).await;
        reply(&fx, "bad").await;
        answer(&fx, 1).await.unwrap();
        assert_eq!(stored(&fx).await["data"]["stepId"], "g");
    }

    #[tokio::test]
    async fn a_valid_reply_that_lost_to_the_timeout_saves_no_field_and_clears_no_failure() {
        let fx = fixture(json!({})).await;
        reply(&fx, "user@example.com").await;
        fx.exec(r#"UPDATE "ContactInbox" SET "lastInputFailure" = 'timeout' WHERE "id" = 5"#)
            .await;
        // The timer already consumed the cycle: no challenge is pending.
        run_answer(&fx, 1, Some(ID), now_ms(), json!({}))
            .await
            .unwrap();
        assert_eq!(field(&fx).await, None);
        assert_eq!(last_input_failure(&fx).await.as_deref(), Some("timeout"));
        assert_eq!(routed(&fx), None);
    }

    #[tokio::test]
    async fn a_valid_reply_never_clears_another_steps_id_less_challenge() {
        let fx = fixture(json!({})).await;
        reply(&fx, "user@example.com").await;
        let mut other = challenge(1, None, now_ms());
        other["data"]["stepId"] = json!("other-step");
        set_challenge(&fx, other.clone()).await;
        run_answer(&fx, 1, None, now_ms(), json!({})).await.unwrap();
        assert_eq!(stored(&fx).await, other);
        assert_eq!(field(&fx).await, None);
    }

    #[tokio::test]
    async fn legacy_challenge_without_id_still_uses_the_unconditional_clear() {
        let fx = fixture(json!({})).await;
        reply(&fx, "user@example.com").await;
        set_challenge(&fx, challenge(1, None, now_ms())).await;
        run_answer(&fx, 1, None, now_ms(), json!({})).await.unwrap();
        assert_eq!(routed(&fx).as_deref(), Some("n2"));
        assert_eq!(stored(&fx).await, Value::Null);
    }

    #[tokio::test]
    async fn an_older_reply_does_not_consume_the_newer_cycle_or_save_its_answer() {
        let fx = fixture(json!({})).await;
        reply(&fx, "user@example.com").await;
        set_challenge(&fx, challenge(1, Some("newer"), now_ms())).await;
        run_answer(&fx, 1, Some(ID), now_ms(), json!({}))
            .await
            .unwrap();
        assert_eq!(field(&fx).await, None);
        assert_eq!(stored(&fx).await["data"]["challengeId"], "newer");
        // Nor does an older invalid reply advance it.
        let fx = fixture(json!({})).await;
        reply(&fx, "bad").await;
        set_challenge(&fx, challenge(1, Some("newer"), now_ms())).await;
        run_answer(&fx, 1, Some(ID), now_ms(), json!({}))
            .await
            .unwrap();
        assert_eq!(stored(&fx).await["data"]["attempts"], 1);
        assert!(fx.jobs.named(FLOW_MESSAGE).is_empty());
    }
}

/// `getUserData — first send (no challenge state)`.
mod first_send {
    use super::*;

    #[tokio::test]
    async fn sends_message_and_returns_wait_when_no_challenge_active() {
        let fx = fixture(json!({})).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(flow_texts(&fx), vec!["Please enter your email"]);
        assert_eq!(routed(&fx), None);
        let written = stored(&fx).await;
        assert_eq!(written["type"], "step");
        assert_eq!(written["data"]["stepId"], "g");
        assert_eq!(written["data"]["nodeId"], "n1");
        assert_eq!(written["data"]["attempts"], 1);
        assert!(written["data"]["startedAt"].is_string());
        let id = written["data"]["challengeId"].as_str().unwrap().to_owned();
        let postback = flow_messages(&fx)[0]["presetQuickReplies"][0]["postback"].clone();
        assert_eq!(
            chatbotx_core::flow_config::step_input_resolution::decode_step_input_skip_postback(
                postback.as_str()
            ),
            Some(id)
        );
    }

    #[tokio::test]
    async fn uses_the_latest_flow_version_marker_in_both_challenge_and_job() {
        let fx = fixture(json!({})).await;
        fx.run(json!({ "flowVersionId": null })).await.unwrap();
        assert!(stored(&fx).await["data"].get("flowVersionId").is_none());
        assert!(flow_messages(&fx)[0].get("flowVersionId").is_none());
    }

    #[tokio::test]
    async fn writes_appointment_id_into_challenge_state_and_the_first_prompt_job() {
        let fx = fixture(json!({})).await;
        fx.run(json!({ "appointmentId": "77" })).await.unwrap();
        assert_eq!(stored(&fx).await["data"]["appointmentId"], "77");
        assert_eq!(flow_messages(&fx)[0]["appointmentId"], "77");
    }

    #[tokio::test]
    async fn carries_the_inbox_sender_into_the_first_prompt_job() {
        let fx = fixture(json!({})).await;
        fx.run(json!({ "sendFrom": "inbox", "sentByUserId": "1" }))
            .await
            .unwrap();
        let sent = &flow_messages(&fx)[0];
        assert_eq!(
            (&sent["sendFrom"], &sent["sentByUserId"]),
            (&json!("inbox"), &json!("1"))
        );
    }

    #[tokio::test]
    async fn reuses_the_id_of_this_steps_pending_challenge_but_never_another_steps() {
        let fx = fixture(json!({})).await;
        set_challenge(&fx, challenge(1, Some(ID), now_ms())).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(stored(&fx).await["data"]["challengeId"], ID);
        let fx = fixture(json!({})).await;
        let mut other = challenge(1, Some(ID), now_ms());
        other["data"]["stepId"] = json!("other-step");
        set_challenge(&fx, other).await;
        fx.run(json!({})).await.unwrap();
        assert_ne!(stored(&fx).await["data"]["challengeId"], ID);
    }

    #[tokio::test]
    async fn clears_the_challenge_it_created_after_a_first_prompt_enqueue_failure() {
        let fx = fixture(json!({})).await;
        fx.jobs.fail_next();
        fx.run(json!({})).await.unwrap();
        assert_eq!(stored(&fx).await, Value::Null);
        assert_eq!(routed(&fx).as_deref(), Some("n4"));
    }
}

/// `getUserData — comment-triggered private reply`.
mod comment_anchor {
    use super::*;

    #[tokio::test]
    async fn forwards_the_comment_anchor_so_the_channel_can_send_the_prompt_comment_anchored() {
        let fx = fixture(json!({})).await;
        let anchor = json!({ "postId": "p", "commentId": "c", "spent": false });
        fx.run(json!({ "commentAnchor": anchor })).await.unwrap();
        assert_eq!(flow_messages(&fx)[0]["commentAnchor"], anchor);
    }

    #[tokio::test]
    async fn sends_no_comment_anchor_key_when_the_run_did_not_start_from_a_comment() {
        let fx = fixture(json!({})).await;
        fx.run(json!({})).await.unwrap();
        assert!(flow_messages(&fx)[0].get("commentAnchor").is_none());
    }
}

/// `getUserData — webview submission (RF09/RF10)`.
mod webview_submission {
    use super::*;

    fn selection(step_id: &str, challenge_id: &str) -> Value {
        json!({ "metadata": { "type": "getUserDataWebviewSelection", "stepId": step_id,
                              "challengeId": challenge_id, "selectedValue": "2026-07-20T00:00:00.000Z" },
                "startFromStepId": "g" })
    }

    #[tokio::test]
    async fn claims_the_challenge_and_saves_the_selected_value_on_success() {
        let fx = fixture(json!({ "replyFormat": "RF09" })).await;
        set_challenge(&fx, challenge(1, Some(ID), now_ms())).await;
        fx.run(selection("g", ID)).await.unwrap();
        assert_eq!(
            field(&fx).await.as_deref(),
            Some("2026-07-20T00:00:00.000Z")
        );
        assert_eq!(stored(&fx).await, Value::Null);
        assert_eq!(routed(&fx).as_deref(), Some("n2"));
    }

    #[tokio::test]
    async fn duplicate_submission_challenge_already_consumed_is_a_no_op() {
        let fx = fixture(json!({ "replyFormat": "RF09" })).await;
        fx.run(selection("g", ID)).await.unwrap();
        assert_eq!(field(&fx).await, None);
        assert_eq!(routed(&fx), None);
    }

    #[tokio::test]
    async fn stale_challenge_id_from_a_previous_challenge_cycle_is_a_no_op() {
        let fx = fixture(json!({ "replyFormat": "RF09" })).await;
        set_challenge(&fx, challenge(1, Some("newer"), now_ms())).await;
        fx.run(selection("g", ID)).await.unwrap();
        assert_eq!(field(&fx).await, None);
        assert_eq!(stored(&fx).await["data"]["challengeId"], "newer");
    }

    #[tokio::test]
    async fn metadata_for_a_different_step_falls_through_to_the_normal_flow_instead_of_claiming() {
        let fx = fixture(json!({ "replyFormat": "RF09" })).await;
        fx.run(selection("other-step", ID)).await.unwrap();
        assert_eq!(field(&fx).await, None);
        // No challenge variables: the step asks its question.
        assert_eq!(stored(&fx).await["data"]["stepId"], "g");
    }
}

/// `getUserData — date/datetime webview channel gating` and the non-date
/// formats.
mod prompt_paths {
    use super::*;

    #[tokio::test]
    async fn whatsapp_is_not_url_quick_reply_capable_and_falls_through_to_the_plain_text_prompt() {
        let mut fx = Fixture::new().await;
        fx.on_channel("whatsapp").await;
        flow(&fx, json!({ "replyFormat": "RF09" })).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(flow_texts(&fx), vec!["Please enter your email"]);
        assert!(chats(&fx).is_empty());
    }

    #[tokio::test]
    async fn text_reply_format_still_enqueues_via_send_flow_message() {
        let fx = fixture(json!({ "replyFormat": "RF02" })).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(flow_messages(&fx)[0]["step"]["stepType"], "sendText");
        assert!(chats(&fx).is_empty());
    }
}

/// `getUserData — WhatsApp native location request (RF08)`.
mod whatsapp_location {
    use super::*;

    async fn whatsapp(overrides: Value) -> Fixture {
        let mut fx = Fixture::new().await;
        fx.on_channel("whatsapp").await;
        let mut overrides = overrides;
        overrides["replyFormat"] = json!("RF08");
        flow(&fx, overrides).await;
        fx.run(json!({})).await.unwrap();
        fx
    }

    #[tokio::test]
    async fn whatsapp_location_format_sends_the_reserved_native_location_request_marker() {
        let fx = whatsapp(json!({})).await;
        let sent = chats(&fx);
        assert_eq!(sent[0]["text"], "Please enter your email");
        assert_eq!(
            sent[0]["quickReplies"],
            json!([{ "id": "whatsapp:native:location_request", "label": "Send location",
                     "buttonType": "postback", "postback": "whatsapp:native:location_request" }])
        );
    }

    #[tokio::test]
    async fn sends_the_skip_button_as_its_own_message_after_the_native_request() {
        let fx = whatsapp(json!({ "skipButtonLabel": "No thanks" })).await;
        let companion = &flow_messages(&fx)[0];
        assert_eq!(companion["step"]["text"], "No thanks");
        assert_eq!(companion["presetQuickReplies"][0]["label"], "No thanks");
    }

    #[tokio::test]
    async fn sends_no_companion_message_when_the_skip_label_is_blank() {
        let fx = whatsapp(json!({ "skipButtonLabel": " " })).await;
        assert!(fx.jobs.named(FLOW_MESSAGE).is_empty());
    }

    #[tokio::test]
    async fn uses_the_vietnamese_inbox_label_when_workspace_language_is_vi() {
        let mut fx = Fixture::new().await;
        fx.on_channel("whatsapp").await;
        fx.exec(r#"UPDATE "Workspace" SET "language" = 'vi' WHERE "id" = 2"#)
            .await;
        flow(&fx, json!({ "replyFormat": "RF08" })).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(chats(&fx)[0]["quickReplies"][0]["label"], "Gửi vị trí");
    }

    #[tokio::test]
    async fn non_whatsapp_location_format_keeps_the_text_prompt_path() {
        let fx = fixture(json!({ "replyFormat": "RF08" })).await;
        fx.run(json!({})).await.unwrap();
        assert!(chats(&fx).is_empty());
        assert_eq!(flow_texts(&fx), vec!["Please enter your email"]);
    }
}

/// `getUserData — Show Hint contact quick reply (phone/email)`.
mod contact_hint {
    use super::*;

    async fn hinted(
        channel: &'static str,
        format: &str,
        phone: Option<&'static str>,
        email: Option<&'static str>,
    ) -> Fixture {
        let mut fx = Fixture::new().await;
        fx.on_channel(channel).await;
        fx.sql(move |tx| {
            tx.execute(
                r#"UPDATE "Contact" SET "phoneNumber" = ?1, "email" = ?2 WHERE "id" = 3"#,
                chatbotx_db::rusqlite::params![phone, email],
            )?;
            Ok(())
        })
        .await;
        flow(&fx, json!({ "replyFormat": format, "showHint": true })).await;
        fx.run(json!({})).await.unwrap();
        fx
    }

    #[tokio::test]
    async fn messenger_offers_the_stored_contact_phone_with_the_full_value_in_the_payload() {
        let fx = hinted("messenger", "RF04", Some("+84901234567"), None).await;
        let replies = &chats(&fx)[0]["quickReplies"];
        assert_eq!(replies[0]["label"], "+84901234567");
        assert_eq!(
            replies[0]["postback"],
            chatbotx_core::flow_config::steps::encode_contact_hint_quick_reply_payload(
                "+84901234567"
            )
        );
        assert_eq!(replies[1]["label"], "Skip");
    }

    #[tokio::test]
    async fn a_value_longer_than_20_characters_gets_a_shortened_title_and_the_full_payload() {
        let fx = hinted(
            "messenger",
            "RF03",
            None,
            Some("a.very.long.name@example.com"),
        )
        .await;
        let reply = &chats(&fx)[0]["quickReplies"][0];
        assert_eq!(reply["label"], "a.very.long.name@ex…");
        assert_eq!(
            reply["postback"],
            chatbotx_core::flow_config::steps::encode_contact_hint_quick_reply_payload(
                "a.very.long.name@example.com"
            )
        );
    }

    #[tokio::test]
    async fn messenger_without_a_stored_value_falls_back_to_the_native_quick_reply() {
        let fx = hinted("messenger", "RF04", None, None).await;
        assert_eq!(
            chats(&fx)[0]["quickReplies"][0]["postback"],
            "messenger:native-quick-reply:user_phone_number"
        );
        let fx = hinted("messenger", "RF03", None, None).await;
        assert_eq!(
            chats(&fx)[0]["quickReplies"][0]["postback"],
            "messenger:native-quick-reply:user_email"
        );
        // An invalid stored phone falls back too.
        let fx = hinted("messenger", "RF04", Some("call me"), None).await;
        assert_eq!(
            chats(&fx)[0]["quickReplies"][0]["postback"],
            "messenger:native-quick-reply:user_phone_number"
        );
    }

    #[tokio::test]
    async fn instagram_offers_the_stored_contact_value_and_otherwise_keeps_the_text_prompt() {
        let fx = hinted("instagram", "RF03", None, Some("  ada@example.com ")).await;
        assert_eq!(chats(&fx)[0]["quickReplies"][0]["label"], "ada@example.com");
        let fx = hinted("instagram", "RF03", None, None).await;
        assert!(chats(&fx).is_empty());
        assert_eq!(flow_texts(&fx).len(), 1);
    }

    #[tokio::test]
    async fn show_hint_off_or_another_format_keeps_the_text_prompt_path() {
        let mut fx = Fixture::new().await;
        fx.on_channel("messenger").await;
        flow(&fx, json!({ "replyFormat": "RF04", "showHint": false })).await;
        fx.run(json!({})).await.unwrap();
        assert!(chats(&fx).is_empty());
        let fx = hinted("messenger", "RF02", Some("+84901234567"), None).await;
        assert!(chats(&fx).is_empty());
    }

    #[tokio::test]
    async fn a_comment_triggered_run_keeps_the_text_prompt_path() {
        let mut fx = Fixture::new().await;
        fx.on_channel("messenger").await;
        flow(&fx, json!({ "replyFormat": "RF04", "showHint": true })).await;
        fx.run(json!({ "commentAnchor": { "postId": "p", "commentId": "c" } }))
            .await
            .unwrap();
        assert!(chats(&fx).is_empty());
    }
}

/// `getUserData — Auto Skip timer`.
mod auto_skip_timer {
    use super::*;

    async fn timer(fx: &Fixture) -> Option<(String, i64, Value)> {
        let row = fx
            .text(
                r#"SELECT json_object('type', "type", 'triggerAt', "triggerAt", 'metadata', json("metadata"),
                     'flowVersionId', "flowVersionId", 'appointmentId', "appointmentId")
                   FROM "ContactOnSmartDelay" WHERE "conversationId" = 6"#,
            )
            .await?;
        let row: Value = serde_json::from_str(&row).unwrap();
        Some((
            row["type"].as_str()?.to_owned(),
            row["triggerAt"].as_i64()?,
            row,
        ))
    }

    #[tokio::test]
    async fn arms_one_timer_from_the_first_prompt_carrying_the_challenge_id_for_the_claim() {
        let fx = fixture(json!({ "autoSkip": true })).await;
        fx.run(json!({})).await.unwrap();
        let (kind, trigger_at, row) = timer(&fx).await.expect("timer");
        assert_eq!(kind, "stepInputTimeout");
        let challenge = stored(&fx).await;
        assert_eq!(
            row["metadata"]["challengeId"],
            challenge["data"]["challengeId"]
        );
        assert_eq!(row["metadata"]["reason"], "timeout");
        let started =
            chatbotx_core::js::parse_date(challenge["data"]["startedAt"].as_str().unwrap())
                .unwrap();
        assert_eq!(trigger_at, started + 3_600_000);
        assert_eq!(
            fx.int(r#"SELECT COUNT(*) FROM "ContactOnSmartDelay""#)
                .await,
            Some(1)
        );
    }

    #[tokio::test]
    async fn honours_the_configured_unit_seconds() {
        let fx = fixture(
            json!({ "autoSkip": true, "autoSkipTimeUnit": "seconds", "autoSkipTimeValue": 30 }),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        let (_, trigger_at, _) = timer(&fx).await.expect("timer");
        let challenge = stored(&fx).await;
        let started =
            chatbotx_core::js::parse_date(challenge["data"]["startedAt"].as_str().unwrap())
                .unwrap();
        assert_eq!(trigger_at, started + 30_000);
    }

    #[tokio::test]
    async fn pins_no_flow_version_when_the_run_follows_the_latest_one() {
        let fx = fixture(json!({ "autoSkip": true })).await;
        fx.run(json!({ "flowVersionId": null })).await.unwrap();
        let (_, _, row) = timer(&fx).await.expect("timer");
        assert_eq!(row["flowVersionId"], Value::Null);
        // ponytail: "carries the appointment" needs an `Appointment` row (FK);
        // the challenge / prompt side is in `first_send`.
    }

    #[tokio::test]
    async fn arms_no_timer_when_auto_skip_is_off() {
        let fx = fixture(json!({})).await;
        fx.run(json!({})).await.unwrap();
        assert!(timer(&fx).await.is_none());
    }

    #[tokio::test]
    async fn a_retry_never_re_arms_the_timer() {
        let fx = fixture(json!({ "autoSkip": true })).await;
        reply(&fx, "bad").await;
        answer(&fx, 1).await.unwrap();
        assert!(timer(&fx).await.is_none());
    }
}

/// `getUserData — input resolution (timer or Skip button)`.
mod input_resolution {
    use super::*;

    async fn resolve(overrides: Value, reason: &str) -> Fixture {
        let fx = fixture(overrides).await;
        reply(&fx, "user@example.com").await;
        set_challenge(&fx, challenge(1, Some(ID), now_ms())).await;
        let vars = json!({ "attempts": 1, "lastAttemptAt": iso(now_ms()), "challengeId": ID, "resolution": reason });
        fx.run(json!({ "startFromStepId": "g", "stepChallenge": vars }))
            .await
            .unwrap();
        fx
    }

    #[tokio::test]
    async fn a_timeout_takes_the_skip_branch_claims_the_challenge_then_records_the_timeout() {
        let fx = resolve(json!({ "autoSkip": true }), "timeout").await;
        assert_eq!(routed(&fx).as_deref(), Some("n3"));
        assert_eq!(last_input_failure(&fx).await.as_deref(), Some("timeout"));
        assert_eq!(stored(&fx).await, Value::Null);
    }

    #[tokio::test]
    async fn a_skip_tap_takes_the_skip_branch_and_records_user_skipped_and_never_saves_the_reply() {
        let fx = resolve(json!({}), "user_skipped").await;
        assert_eq!(routed(&fx).as_deref(), Some("n3"));
        assert_eq!(
            last_input_failure(&fx).await.as_deref(),
            Some("user_skipped")
        );
        assert_eq!(field(&fx).await, None);
    }

    #[tokio::test]
    async fn ignores_a_timeout_when_the_step_no_longer_has_auto_skip_on() {
        let fx = resolve(json!({ "autoSkip": false }), "timeout").await;
        assert_eq!(routed(&fx), None);
        assert_eq!(stored(&fx).await["data"]["challengeId"], ID);
    }

    #[tokio::test]
    async fn ignores_a_skip_tap_when_the_step_no_longer_has_a_skip_button() {
        let fx = resolve(json!({ "skipButtonLabel": "" }), "user_skipped").await;
        assert_eq!(routed(&fx), None);
        assert_eq!(stored(&fx).await["data"]["challengeId"], ID);
    }

    #[tokio::test]
    async fn losing_the_claim_the_contact_answered_first_writes_nothing_and_waits() {
        let fx = fixture(json!({ "autoSkip": true })).await;
        let vars = json!({ "attempts": 1, "lastAttemptAt": iso(now_ms()), "challengeId": ID, "resolution": "timeout" });
        fx.run(json!({ "startFromStepId": "g", "stepChallenge": vars }))
            .await
            .unwrap();
        assert_eq!(routed(&fx), None);
        assert_eq!(last_input_failure(&fx).await, None);
    }
}

/// `getUserData — Skip button`.
mod skip_button {
    use super::*;

    #[tokio::test]
    async fn a_blank_label_sends_no_button() {
        let fx = fixture(json!({ "skipButtonLabel": "  " })).await;
        fx.run(json!({})).await.unwrap();
        assert!(flow_messages(&fx)[0].get("presetQuickReplies").is_none());
    }

    #[tokio::test]
    async fn the_shown_label_is_shortened_to_20_characters_without_splitting_an_emoji() {
        let fx = fixture(json!({ "skipButtonLabel": "Skip this question for now 😀" })).await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(
            flow_messages(&fx)[0]["presetQuickReplies"][0]["label"],
            "Skip this question …"
        );
        assert_eq!(
            super::super::get_user_data::to_quick_reply_title("1234567890123456789😀😀"),
            "1234567890123456789…"
        );
        assert_eq!(
            super::super::get_user_data::to_quick_reply_title("1234567890123456789😀"),
            "1234567890123456789😀"
        );
    }
}

/// `getUserData — the reply judged is the message that triggered the run`.
mod trigger_message {
    use super::*;

    #[tokio::test]
    async fn an_answer_queued_before_a_skip_tap_is_saved_not_the_skip_label_that_is_now_the_latest_message()
     {
        let fx = fixture(json!({})).await;
        let now = now_ms();
        message(&fx, 80, "text", Some("user@example.com"), None, now - 1_000).await;
        message(&fx, 81, "text", Some("Skip"), None, now).await;
        set_challenge(&fx, challenge(1, Some(ID), now)).await;
        run_answer(
            &fx,
            1,
            Some(ID),
            now,
            json!({ "triggerMessageId": "80", "triggerMessageCreatedAt": iso(now - 1_000) }),
        )
        .await
        .unwrap();
        assert_eq!(field(&fx).await.as_deref(), Some("user@example.com"));
    }

    #[tokio::test]
    async fn falls_back_to_the_latest_incoming_message_when_the_trigger_cannot_be_found() {
        let fx = fixture(json!({})).await;
        reply(&fx, "user@example.com").await;
        set_challenge(&fx, challenge(1, Some(ID), now_ms())).await;
        run_answer(
            &fx,
            1,
            Some(ID),
            now_ms(),
            json!({ "triggerMessageId": "999", "triggerMessageCreatedAt": now_ms() }),
        )
        .await
        .unwrap();
        assert_eq!(field(&fx).await.as_deref(), Some("user@example.com"));
    }
}
