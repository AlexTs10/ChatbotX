//! Ports of `challenge-handler.test.ts` and `step-input-resolution.test.ts`.
//! Adapted: the mocked `runStepsAndQuickReplies` becomes the real runner on a
//! migrated SQLite file. Node `n1` has the steps `s0`, `step-1` and `s2`, each
//! an `addContactNotes`: the notes show which step ran, and the `sendFlow`
//! continuation to `s2` shows the carried run fields. The challenge variables
//! are read from `reentry_job`. Not ported: the `message:bot_received`
//! dashboard events (not emitted, DECISIONS) and the injected "db down" error.

use serde_json::{Value, json};

use super::*;
use crate::steps::test_support::*;

const STARTED_AT: &str = "2026-11-09T10:00:00.000Z";

fn data(extra: Value) -> Value {
    let mut data = json!({
        "flowId": "10", "flowVersionId": "11", "nodeId": "n1", "stepId": "step-1",
        "attempts": 2, "lastAttemptAt": "2026-11-09T10:05:00.000Z",
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        data[k] = v;
    }
    data
}

fn step_challenge(extra: Value) -> Value {
    json!({ "type": "step", "data": data(extra) })
}

/// The full challenge of the resolution tests (deadline origin, id, appointment).
fn full_challenge() -> Value {
    step_challenge(
        json!({ "startedAt": STARTED_AT, "challengeId": "challenge-1", "appointmentId": "77" }),
    )
}

async fn fixture(nodes: Option<Value>) -> Fixture {
    let fx = Fixture::new().await;
    let note =
        |id: &str, text: &str| json!({ "id": id, "stepType": "addContactNotes", "content": text });
    match nodes {
        Some(nodes) => fx.flow(nodes, json!([])).await,
        None => {
            fx.node(
                json!([
                    note("s0", "first"),
                    note("step-1", "resumed"),
                    note("s2", "after")
                ]),
                json!([]),
            )
            .await
        }
    }
    fx
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

async fn notes(fx: &Fixture) -> Option<String> {
    fx.text(
        r#"SELECT group_concat("text", ',') FROM (SELECT "text" FROM "ContactNote" ORDER BY "id")"#,
    )
    .await
}

async fn stored_challenge(fx: &Fixture) -> Value {
    fx.text(r#"SELECT json_extract("additionalAttributes", '$.challenge') FROM "Conversation" WHERE "id" = 6"#)
        .await
        .map(|c| serde_json::from_str(&c).unwrap())
        .unwrap_or(Value::Null)
}

fn ids() -> Value {
    json!({ "conversationId": "6", "contactInboxId": "5" })
}

fn with(mut base: Value, extra: Value) -> Value {
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        base[k] = v;
    }
    base
}

/// `reentry_job` for `challenge` on the fixture's conversation.
async fn job_for(
    fx: &Fixture,
    challenge: &Value,
    message_id: Option<&str>,
    started_by_contact_message: bool,
    resolution: Option<&str>,
) -> Value {
    let conversation = crate::load_conversation(&fx.deps.db, CONVERSATION)
        .await
        .unwrap()
        .unwrap();
    let contact_inbox =
        crate::load_contact_inbox(&fx.deps.db, CONTACT_INBOX, conversation.contact_id)
            .await
            .unwrap()
            .unwrap();
    reentry_job(&Reentry {
        conversation: &conversation,
        contact_inbox: &contact_inbox,
        challenge,
        message_id,
        message_created_at: None,
        started_by_contact_message,
        resolution,
        drop_when_unavailable: false,
    })
}

/// `challenge-handler.test.ts`.
mod challenge_handler {
    use super::*;

    fn run_data(challenge: Value, extra: Value) -> Value {
        with(with(ids(), json!({ "challenge": challenge })), extra)
    }

    #[tokio::test]
    async fn resumes_challenge_from_the_stored_step() {
        let fx = fixture(None).await;
        run_challenge(&fx.deps, &run_data(step_challenge(json!({})), json!({})))
            .await
            .unwrap();
        assert_eq!(notes(&fx).await.as_deref(), Some("resumed"));
        assert_eq!(fx.jobs.send_flows()[0]["startFromStepId"], "s2");
    }

    #[tokio::test]
    async fn passes_the_node_id_so_the_remaining_steps_resume_in_the_same_node() {
        let fx = fixture(None).await;
        let job = job_for(&fx, &step_challenge(json!({})), None, true, None).await;
        assert_eq!(
            (&job["nodeId"], &job["startFromStepId"]),
            (&json!("n1"), &json!("step-1"))
        );
        run_challenge(&fx.deps, &run_data(step_challenge(json!({})), json!({})))
            .await
            .unwrap();
        assert_eq!(fx.jobs.send_flows()[0]["nodeId"], "n1");
    }

    #[tokio::test]
    async fn a_typed_reply_is_a_contact_initiated_run_and_carries_no_resolution() {
        let fx = fixture(None).await;
        run_challenge(
            &fx.deps,
            &run_data(step_challenge(json!({})), json!({ "messageId": "msg-1" })),
        )
        .await
        .unwrap();
        let next = &fx.jobs.send_flows()[0];
        assert_eq!(next["startedByContactMessage"], true);
        assert_eq!(next["triggerMessageId"], "msg-1");
        // The challenge variables reach the resumed step only.
        assert!(next.get(STEP_CHALLENGE).is_none());
        let job = job_for(&fx, &step_challenge(json!({})), Some("msg-1"), true, None).await;
        assert!(job[STEP_CHALLENGE].get("resolution").is_none());
    }

    #[tokio::test]
    async fn exposes_the_deadline_origin_and_challenge_id_to_the_step_when_the_challenge_has_them()
    {
        let fx = fixture(None).await;
        let challenge = step_challenge(
            json!({ "startedAt": "2025-12-31T00:00:00.000Z", "challengeId": "challenge-1" }),
        );
        let job = job_for(&fx, &challenge, None, true, None).await;
        assert_eq!(job[STEP_CHALLENGE]["attempts"], 2);
        assert_eq!(job[STEP_CHALLENGE]["startedAt"], "2025-12-31T00:00:00.000Z");
        assert_eq!(job[STEP_CHALLENGE]["challengeId"], "challenge-1");
    }

    #[tokio::test]
    async fn a_legacy_challenge_exposes_neither_so_the_step_falls_back_to_the_last_attempt() {
        let fx = fixture(None).await;
        let job = job_for(&fx, &step_challenge(json!({})), None, true, None).await;
        assert!(job[STEP_CHALLENGE].get("startedAt").is_none());
        assert!(job[STEP_CHALLENGE].get("challengeId").is_none());
        assert_eq!(
            job[STEP_CHALLENGE]["lastAttemptAt"],
            "2026-11-09T10:05:00.000Z"
        );
    }

    #[tokio::test]
    async fn a_missing_flow_still_fails_the_job_so_it_retries() {
        let fx = fixture(None).await;
        let err = run_challenge(
            &fx.deps,
            &run_data(step_challenge(json!({ "flowVersionId": "999" })), json!({})),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("FlowVersion not found"), "{err}");
    }

    #[tokio::test]
    async fn delegates_a_quick_reply_challenge_and_does_not_re_run_the_node() {
        // Adapted: the quick-reply challenge resume is not ported (DECISIONS);
        // the job is a no-op and the node does not run.
        let fx = fixture(None).await;
        let challenge = json!({ "type": "quickReply", "data": { "flowId": "10", "nodeId": "n1", "attempts": 0 } });
        run_challenge(&fx.deps, &run_data(challenge, json!({})))
            .await
            .unwrap();
        assert_eq!(notes(&fx).await, None);
    }

    #[tokio::test]
    async fn ignores_an_unknown_challenge_type_without_resolving_a_flow() {
        let fx = fixture(None).await;
        run_challenge(
            &fx.deps,
            &run_data(json!({ "type": "somethingNew", "data": {} }), json!({})),
        )
        .await
        .unwrap();
        assert_eq!(notes(&fx).await, None);
        assert!(fx.jobs.all().is_empty());
    }
}

/// `step-input-resolution.test.ts`.
mod step_input_resolution {
    use super::*;

    fn user_skip() -> Value {
        with(
            ids(),
            json!({ "challengeId": "challenge-1", "reason": "user_skipped", "messageId": "msg-1",
                     "messageCreatedAt": "2026-11-09T10:06:00.000Z" }),
        )
    }

    fn timeout() -> Value {
        with(
            ids(),
            json!({ "challengeId": "challenge-1", "reason": "timeout" }),
        )
    }

    async fn armed(nodes: Option<Value>) -> Fixture {
        let fx = fixture(nodes).await;
        set_challenge(&fx, full_challenge()).await;
        fx
    }

    #[tokio::test]
    async fn a_user_skip_re_enters_the_step_from_its_challenge_as_a_contact_initiated_run() {
        let fx = armed(None).await;
        resolve_step_input(&fx.deps, &user_skip()).await.unwrap();
        assert_eq!(notes(&fx).await.as_deref(), Some("resumed"));
        let next = &fx.jobs.send_flows()[0];
        assert_eq!(next["startedByContactMessage"], true);
        assert_eq!(next["triggerMessageId"], "msg-1");
        assert_eq!(next["appointmentId"], "77");
        let job = job_for(
            &fx,
            &full_challenge(),
            Some("msg-1"),
            true,
            Some("user_skipped"),
        )
        .await;
        assert_eq!(
            job[STEP_CHALLENGE],
            json!({ "attempts": 2, "lastAttemptAt": "2026-11-09T10:05:00.000Z", "startedAt": STARTED_AT,
                    "challengeId": "challenge-1", "resolution": "user_skipped" })
        );
    }

    #[tokio::test]
    async fn a_timeout_is_not_contact_initiated_and_carries_no_message_identity() {
        let fx = armed(None).await;
        resolve_step_input(&fx.deps, &timeout()).await.unwrap();
        let next = &fx.jobs.send_flows()[0];
        assert_eq!(next["startedByContactMessage"], false);
        assert!(next.get("triggerMessageId").is_none());
    }

    #[tokio::test]
    async fn passes_no_run_metadata_so_a_flow_looping_back_to_the_step_cannot_re_resolve_it() {
        let fx = armed(None).await;
        resolve_step_input(&fx.deps, &user_skip()).await.unwrap();
        assert!(fx.jobs.send_flows()[0].get("metadata").is_none());
    }

    #[tokio::test]
    async fn ignores_a_stale_resolution() {
        let stale = [
            Value::Null,
            step_challenge(json!({ "challengeId": "newer" })),
            json!({ "type": "quickReply", "data": { "flowId": "10", "nodeId": "n1" } }),
        ];
        for challenge in stale {
            let fx = fixture(None).await;
            set_challenge(&fx, challenge.clone()).await;
            resolve_step_input(&fx.deps, &user_skip()).await.unwrap();
            assert_eq!(notes(&fx).await, None, "{challenge}");
            assert_eq!(stored_challenge(&fx).await, challenge);
        }
    }

    #[tokio::test]
    async fn leaves_the_challenge_alone_when_the_workspace_is_no_longer_active() {
        let fx = armed(None).await;
        fx.exec(r#"UPDATE "Workspace" SET "isActive" = 0 WHERE "id" = 2"#)
            .await;
        resolve_step_input(&fx.deps, &user_skip()).await.unwrap();
        assert_eq!(notes(&fx).await, None);
        assert_eq!(stored_challenge(&fx).await, full_challenge());
    }

    #[tokio::test]
    async fn leaves_the_challenge_alone_when_the_bot_is_paused() {
        let fx = armed(None).await;
        fx.exec(r#"UPDATE "Conversation" SET "botEnabled" = 0 WHERE "id" = 6"#)
            .await;
        resolve_step_input(&fx.deps, &timeout()).await.unwrap();
        assert_eq!(notes(&fx).await, None);
    }

    #[tokio::test]
    async fn clears_the_challenge_and_does_not_retry_when_the_flow_was_edited() {
        let cases = [
            ("node gone", json!([])),
            (
                "step gone",
                json!([{ "id": "n1", "data": { "details": { "steps": [] } } }]),
            ),
            (
                "node without steps",
                json!([{ "id": "n1", "data": { "details": { "type": "wait" } } }]),
            ),
        ];
        for (label, nodes) in cases {
            let fx = armed(Some(nodes)).await;
            resolve_step_input(&fx.deps, &user_skip()).await.unwrap();
            assert_eq!(stored_challenge(&fx).await, Value::Null, "{label}");
            assert!(fx.jobs.all().is_empty(), "{label}");
        }
        // Flow version gone.
        let fx = fixture(None).await;
        set_challenge(
            &fx,
            step_challenge(json!({ "challengeId": "challenge-1", "flowVersionId": "999" })),
        )
        .await;
        resolve_step_input(&fx.deps, &user_skip()).await.unwrap();
        assert_eq!(stored_challenge(&fx).await, Value::Null);
        assert_eq!(notes(&fx).await, None);
    }

    #[tokio::test]
    async fn a_failure_while_running_the_step_propagates_so_the_job_retries() {
        let fx = armed(None).await;
        fx.jobs.fail_next();
        assert!(resolve_step_input(&fx.deps, &user_skip()).await.is_err());
    }
}

/// `step-input-resolution.test.ts` (`runStepInputTimeoutResume`).
mod step_input_timeout_resume {
    use super::*;

    const ROW: i64 = 90;

    async fn row(
        fx: &Fixture,
        kind: &'static str,
        status: &'static str,
        trigger_offset_ms: i64,
        metadata: Value,
    ) {
        let (metadata, trigger_at) = (metadata.to_string(), now_ms() + trigger_offset_ms);
        fx.sql(move |tx| {
            tx.execute(
                r#"INSERT INTO "ContactOnSmartDelay" ("id", "workspaceId", "flowId", "flowVersionId", "contactInboxId",
                     "conversationId", "nodeId", "stepId", "metadata", "type", "triggerAt", "status")
                   VALUES (?1, 2, 10, 11, 5, 6, 'n1', 'step-1', ?2, ?3, ?4, ?5)"#,
                chatbotx_db::rusqlite::params![ROW, metadata, kind, trigger_at, status],
            )?;
            Ok(())
        })
        .await;
    }

    fn payload() -> Value {
        json!({ "type": "stepInputResolution", "stepId": "step-1", "challengeId": "challenge-1", "reason": "timeout" })
    }

    async fn status(fx: &Fixture) -> Option<String> {
        fx.text(r#"SELECT "status" FROM "ContactOnSmartDelay" WHERE "id" = 90"#)
            .await
    }

    async fn fire(fx: &Fixture) -> Result<(), JobError> {
        resume_step_input_timeout(&fx.deps, &json!({ "smartDelayId": ROW.to_string() })).await
    }

    #[tokio::test]
    async fn fires_claims_the_row_then_resolves_the_step_with_the_stored_challenge_and_reason() {
        let fx = fixture(None).await;
        set_challenge(&fx, full_challenge()).await;
        row(&fx, "stepInputTimeout", "scheduled", -1_000, payload()).await;
        fire(&fx).await.unwrap();
        assert_eq!(status(&fx).await.as_deref(), Some("completed"));
        assert_eq!(notes(&fx).await.as_deref(), Some("resumed"));
        assert_eq!(fx.jobs.send_flows()[0]["startedByContactMessage"], false);
    }

    #[tokio::test]
    async fn does_nothing_when_the_row_is_gone_or_not_a_scheduled_step_timeout() {
        let fx = fixture(None).await;
        set_challenge(&fx, full_challenge()).await;
        fire(&fx).await.unwrap();
        for (kind, state) in [
            ("followUp", "scheduled"),
            ("stepInputTimeout", "completed"),
            ("stepInputTimeout", "canceled"),
            ("stepInputTimeout", "pending"),
        ] {
            let fx = fixture(None).await;
            set_challenge(&fx, full_challenge()).await;
            row(&fx, kind, state, -1_000, payload()).await;
            fire(&fx).await.unwrap();
            assert_eq!(status(&fx).await.as_deref(), Some(state), "{kind} {state}");
            assert_eq!(notes(&fx).await, None, "{kind} {state}");
        }
    }

    #[tokio::test]
    async fn a_superseded_timer_restarted_for_a_later_deadline_stands_down() {
        let fx = fixture(None).await;
        set_challenge(&fx, full_challenge()).await;
        row(&fx, "stepInputTimeout", "scheduled", 30 * 60_000, payload()).await;
        fire(&fx).await.unwrap();
        assert_eq!(status(&fx).await.as_deref(), Some("scheduled"));
        assert_eq!(notes(&fx).await, None);
    }

    #[tokio::test]
    async fn cancels_a_row_whose_payload_is_unreadable_instead_of_running_it() {
        let fx = fixture(None).await;
        set_challenge(&fx, full_challenge()).await;
        row(
            &fx,
            "stepInputTimeout",
            "scheduled",
            -1_000,
            json!({ "type": "stepInputResolution", "reason": "nonsense" }),
        )
        .await;
        fire(&fx).await.unwrap();
        assert_eq!(status(&fx).await.as_deref(), Some("canceled"));
        assert_eq!(notes(&fx).await, None);
    }

    // "a timer re-armed between the read and the claim" and "a lost claim":
    // the claim compares status and the observed `triggerAt` in one UPDATE
    // (`claim_for_run`, crates/business smart_delay tests); a lost claim runs
    // nothing, as the `completed` case above shows.

    #[tokio::test]
    async fn a_failed_run_restores_the_claimed_row_and_rethrows_so_the_job_retries() {
        let fx = fixture(None).await;
        set_challenge(&fx, full_challenge()).await;
        row(&fx, "stepInputTimeout", "scheduled", -1_000, payload()).await;
        fx.jobs.fail_next();
        assert!(fire(&fx).await.is_err());
        assert_eq!(status(&fx).await.as_deref(), Some("scheduled"));
    }

    #[tokio::test]
    async fn a_stale_challenge_the_contact_already_answered_completes_without_running_the_step() {
        let fx = fixture(None).await;
        row(&fx, "stepInputTimeout", "scheduled", -1_000, payload()).await;
        fire(&fx).await.unwrap();
        assert_eq!(status(&fx).await.as_deref(), Some("completed"));
        assert_eq!(notes(&fx).await, None);
    }
}
