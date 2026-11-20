//! Ported step tests of Q.P6-U4. As `tests.rs`: the handlers run through
//! `runFlowNode` on a migrated SQLite file; a step's route is read from the
//! `sendFlow` job its state edge queues (`ok` → n2, `err` → n3).

use std::sync::{Arc, Mutex};

use chatbotx_business::external_request::{BoxFuture, Hop, HopReply, HopRequest};
use serde_json::{Value, json};

use super::test_support::*;
use super::tests::with_variables;

fn states() -> Value {
    json!([{ "id": "ok", "stateType": "success" }, { "id": "err", "stateType": "error" }])
}

fn edges() -> Value {
    json!([{ "sourceHandle": "ok", "target": "n2" }, { "sourceHandle": "err", "target": "n3" }])
}

/// The node the step routed to (`None` = no route).
fn routed(fx: &Fixture) -> Option<String> {
    fx.jobs
        .send_flows()
        .first()
        .and_then(|j| j["nodeId"].as_str().map(str::to_owned))
}

/// `coupon-step-handler.test.ts` (+ the refusal and success paths).
mod coupon_step_handler {
    use super::*;

    async fn topic(fx: &Fixture) {
        fx.exec(
            r#"INSERT INTO "CouponTopic" ("id", "workspaceId", "name", "status") VALUES (70, 2, 'T', 'active');
               INSERT INTO "Coupon" ("id", "workspaceId", "topicId", "code") VALUES (71, 2, 70, 'CODE-1');"#,
        )
        .await;
    }

    async fn run(fx: &Fixture, step_type: &str, topic: &str) {
        fx.node(
            json!([{ "id": "1", "stepType": step_type, "topicId": topic, "states": states() }]),
            edges(),
        )
        .await;
        fx.run(json!({})).await.unwrap();
    }

    #[tokio::test]
    async fn set_up_coupon_issues_the_next_free_coupon() {
        let fx = Fixture::new().await;
        topic(&fx).await;
        run(&fx, "setUpCoupon", "70").await;
        assert_eq!(
            fx.int(r#"SELECT "issuedContactId" FROM "Coupon" WHERE "id" = 71"#)
                .await,
            Some(3)
        );
        assert_eq!(routed(&fx).as_deref(), Some("n2"));
    }

    #[tokio::test]
    async fn a_refusal_is_skipped_and_the_flow_succeeds() {
        let fx = Fixture::new().await;
        run(&fx, "setUpCoupon", "70").await;
        assert_eq!(routed(&fx).as_deref(), Some("n2"));
        let fx = Fixture::new().await;
        run(&fx, "markCouponUsed", "70").await;
        assert_eq!(routed(&fx).as_deref(), Some("n2"));
    }

    #[tokio::test]
    async fn mark_coupon_used_stamps_the_issued_coupon() {
        let fx = Fixture::new().await;
        topic(&fx).await;
        run(&fx, "setUpCoupon", "70").await;
        run(&fx, "markCouponUsed", "70").await;
        assert!(
            fx.int(r#"SELECT "usedAt" FROM "Coupon" WHERE "id" = 71"#)
                .await
                .is_some()
        );
    }

    /// "returns an error status when coupon issuing / marking throws": a
    /// failing database write (the table is gone).
    #[tokio::test]
    async fn a_thrown_error_routes_to_the_error_state() {
        for step_type in ["setUpCoupon", "markCouponUsed"] {
            let fx = Fixture::new().await;
            fx.exec(
                r#"INSERT INTO "CouponTopic" ("id", "workspaceId", "name", "status") VALUES (70, 2, 'T', 'active');
                   DROP TABLE "Coupon";"#,
            )
            .await;
            run(&fx, step_type, "70").await;
            assert_eq!(routed(&fx).as_deref(), Some("n3"), "{step_type}");
        }
    }
}

/// A scripted transport for the `callApi` step.
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<HopRequest>>,
    reply: Mutex<Option<Result<HopReply, String>>>,
}

impl Hop for Recorder {
    fn send(&self, request: &HopRequest) -> BoxFuture<'_, Result<HopReply, String>> {
        self.seen.lock().unwrap().push(request.clone());
        let reply = self.reply.lock().unwrap().clone().unwrap_or(Ok(HopReply {
            status: 200,
            headers: vec![],
            body: "{}".into(),
        }));
        Box::pin(async move { reply })
    }
}

fn recorder(reply: Option<Result<HopReply, String>>) -> Arc<Recorder> {
    let hop = Arc::new(Recorder {
        reply: Mutex::new(reply),
        ..Recorder::default()
    });
    let shared: Arc<dyn Hop> = hop.clone();
    super::external_request::TEST_HOP.with(|h| *h.borrow_mut() = Some(shared));
    hop
}

/// `external-request-handler.test.ts`. Adapted: `executeAndMap` runs for
/// real on a public IP literal (no DNS lookup) over a scripted transport;
/// the provider error log has no bus here, so the logged arguments are not
/// observable.
mod external_request_handler {
    use super::*;

    const URL: &str = "http://93.184.216.34/data";

    fn step(body: Value) -> Value {
        let mut step = json!({ "id": "1", "stepType": "callApi", "method": "POST", "url": URL,
            "headers": [], "mapping": [{ "jsonPath": "id", "outputFieldId": "80" }], "states": states() });
        if !body.is_null() {
            step["body"] = body;
        }
        step
    }

    async fn fixture() -> Fixture {
        let mut fx = Fixture::new().await;
        with_variables(&mut fx);
        fx.exec(
            r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (80, 'remote_id', 'shortText', 2), (81, 'nick', 'shortText', 2);
               INSERT INTO "ContactCustomField" ("id", "value", "contactId", "customFieldId") VALUES (82, 'O"Brien', 3, 81);"#,
        )
        .await;
        fx
    }

    async fn run(fx: &Fixture, step: Value) {
        fx.node(json!([step]), edges()).await;
        fx.run(json!({})).await.unwrap();
    }

    #[tokio::test]
    async fn returns_success_and_writes_the_mapped_field() {
        let fx = fixture().await;
        let hop = recorder(Some(Ok(HopReply {
            status: 200,
            headers: vec![],
            body: r#"{"id":"abc"}"#.into(),
        })));
        run(&fx, step(Value::Null)).await;
        assert_eq!(routed(&fx).as_deref(), Some("n2"));
        assert_eq!(hop.seen.lock().unwrap()[0].url, URL);
        assert_eq!(
            fx.text(r#"SELECT "value" FROM "ContactCustomField" WHERE "customFieldId" = 80"#)
                .await
                .as_deref(),
            Some("abc")
        );
    }

    #[tokio::test]
    async fn a_non_2xx_status_routes_to_the_error_state() {
        let fx = fixture().await;
        recorder(Some(Ok(HopReply {
            status: 500,
            headers: vec![],
            body: String::new(),
        })));
        run(&fx, step(Value::Null)).await;
        assert_eq!(routed(&fx).as_deref(), Some("n3"));
    }

    #[tokio::test]
    async fn a_thrown_request_routes_to_the_error_state() {
        let fx = fixture().await;
        recorder(Some(Err("network failure".into())));
        run(&fx, step(Value::Null)).await;
        assert_eq!(routed(&fx).as_deref(), Some("n3"));
    }

    #[tokio::test]
    async fn an_ssrf_rejection_routes_to_the_error_state_without_a_request() {
        let fx = fixture().await;
        let hop = recorder(None);
        let mut step = step(Value::Null);
        step["url"] = json!("http://169.254.169.254/");
        run(&fx, step).await;
        assert_eq!(routed(&fx).as_deref(), Some("n3"));
        assert!(hop.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn json_escapes_a_contact_variable_substituted_into_json_body() {
        let fx = fixture().await;
        let hop = recorder(None);
        run(
            &fx,
            step(json!({ "bodyType": "json", "jsonBody": "{\"name\":\"{{nick}}\"}" })),
        )
        .await;
        let body = hop.seen.lock().unwrap()[0].body.clone().unwrap();
        assert_eq!(body, r#"{"name":"O\"Brien"}"#);
        assert!(serde_json::from_str::<Value>(&body).is_ok());
    }

    #[tokio::test]
    async fn substitutes_a_system_field_that_resolves_to_an_empty_string() {
        let fx = fixture().await;
        let hop = recorder(None);
        run(
            &fx,
            step(json!({ "bodyType": "json", "jsonBody": "{\"note\":\"{{empty}}\"}" })),
        )
        .await;
        assert_eq!(
            hop.seen.lock().unwrap()[0].body.as_deref(),
            Some(r#"{"note":""}"#)
        );
    }
}

/// `external-request-step-registration.test.ts`: `callApi` reaches the
/// external request handler (a request is sent).
mod external_request_step_registration {
    use super::*;

    #[tokio::test]
    async fn dispatches_call_api_steps_to_external_request() {
        let fx = Fixture::new().await;
        let hop = recorder(None);
        fx.node(
            json!([{ "id": "1", "stepType": "callApi", "method": "GET", "url": "http://93.184.216.34/", "headers": [], "mapping": [] }]),
            json!([]),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(hop.seen.lock().unwrap().len(), 1);
    }
}

/// `facebook-custom-audience-handler.test.ts` +
/// `facebook-custom-audience-step-registration.test.ts`. Adapted: the Graph
/// calls are not mocked here, so only "logs and continues without throwing
/// when the action fails" (no Facebook Ads connection) runs; the payloads
/// are covered by `crates/business/tests/ads_audience_users.rs`.
mod facebook_custom_audience_handler {
    use super::*;

    #[tokio::test]
    async fn logs_and_continues_without_throwing_when_the_action_fails() {
        let fx = Fixture::new().await;
        fx.node(
            json!([{ "id": "1", "stepType": "facebookCustomAudience", "customAudienceId": "aud-1",
                     "operation": "add", "states": states() }]),
            edges(),
        )
        .await;
        fx.run(json!({})).await.unwrap();
        assert_eq!(routed(&fx).as_deref(), Some("n2"));
    }
}

/// `process-outbound-message.test.ts`: an agent's message against the
/// outbound (Page) rules. Adapted: the real dispatch runs; the queued
/// `sendFlow` shows the trigger type and no message-driven flag.
mod process_outbound_message {
    use super::*;

    #[tokio::test]
    async fn an_outbound_rule_never_marks_the_flow_run_message_driven() {
        let fx = Fixture::new().await;
        fx.node(json!([]), json!([])).await;
        fx.exec(
            r#"INSERT INTO "AutomatedResponse" ("id", "workspaceId", "keywords", "status", "flowId", "type") VALUES (60, 2, '["hello"]', 1, 10, 'outbound'),
                 (61, 2, '["hello"]', 1, 10, 'inbound');"#,
        )
        .await;
        let data = json!({ "conversation": { "id": "6" }, "contactInbox": { "id": "5" },
                           "message": { "id": "900", "text": "hello" } });
        crate::ai_agent::check_outbound_automated_response(&fx.deps, &data)
            .await
            .unwrap();
        let flows = fx.jobs.send_flows();
        assert_eq!(flows.len(), 1);
        assert_eq!(
            flows[0]["trackingContext"]["triggerType"],
            "agent_message_out"
        );
        assert!(flows[0].get("startedByContactMessage").is_none());
        // An outbound rule is not something the contact said: no activity.
        assert_eq!(
            fx.int(r#"SELECT COUNT(*) FROM "ContactActivity""#).await,
            Some(0)
        );
    }

    #[tokio::test]
    async fn an_empty_text_dispatches_nothing() {
        let fx = Fixture::new().await;
        let data = json!({ "conversation": { "id": "6" }, "contactInbox": { "id": "5" },
                           "message": { "id": "900", "text": "" } });
        crate::ai_agent::check_outbound_automated_response(&fx.deps, &data)
            .await
            .unwrap();
        assert!(fx.jobs.all().is_empty());
    }
}
