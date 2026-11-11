//! Port of `apps/worker/__tests__/connection-send-gate.test.ts`
//! (`deferSendWhileAccountReconnecting`, `deferralDelayMs`, spec 1.4 / S7).
//! `moveToDelayed` + `DelayedError` = `JobError::Delayed { run_at_ms }`.

mod support;

use std::sync::Mutex;

use chatbotx_jobs::deferral::{
    CONNECTION_SEND_DEFERRAL_POLICY, ConnectionState, SendDecision,
    defer_send_while_account_reconnecting, deferral_delay_ms,
};
use chatbotx_jobs::{BoxError, BoxFuture, JobCtx, JobError};
use serde_json::{Value, json};
use support::TestClock;

const NOW: i64 = 1_000_000;

/// Replaces `connectionStateService.resolveSendDecision` (default: "send").
struct FakeConnectionState {
    outcome: Result<SendDecision, String>,
    lookups: Mutex<Vec<String>>,
}

impl FakeConnectionState {
    fn new(outcome: Result<SendDecision, String>) -> FakeConnectionState {
        FakeConnectionState {
            outcome,
            lookups: Mutex::new(Vec::new()),
        }
    }

    fn lookups(&self) -> Vec<String> {
        self.lookups.lock().unwrap().clone()
    }
}

impl ConnectionState for FakeConnectionState {
    fn resolve_send_decision<'a>(
        &'a self,
        inbox_id: &'a str,
    ) -> BoxFuture<'a, Result<SendDecision, BoxError>> {
        Box::pin(async move {
            self.lookups.lock().unwrap().push(inbox_id.to_owned());
            self.outcome.clone().map_err(BoxError::from)
        })
    }
}

fn build_job(
    data: Value,
    attempts_started: u32,
    attempts_made: u32,
    token: Option<&str>,
) -> JobCtx {
    JobCtx {
        job_id: "job-1".to_owned(),
        data,
        attempts_started,
        attempts_made,
        lease_token: token.map(str::to_owned),
        ..JobCtx::default()
    }
}

fn channel_message(inbox_id: &str) -> Value {
    json!({
        "type": "sendChannelMessage",
        "data": { "contactInbox": { "inboxId": inbox_id }, "message": { "senderType": "bot" } },
    })
}

fn random() -> f64 {
    0.37
}

async fn run(job: &JobCtx, state: &FakeConnectionState) -> Result<(), JobError> {
    let clock = TestClock::new(NOW);
    defer_send_while_account_reconnecting(job, state, &*clock, &random).await
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:45 "deferSendWhileAccountReconnecting > lets a send through when the account is not reconnecting"
#[tokio::test]
async fn lets_a_send_through_when_the_account_is_not_reconnecting() {
    let state = FakeConnectionState::new(Ok(SendDecision::Send));
    let job = build_job(channel_message("inbox-1"), 1, 0, Some("token"));

    let result = run(&job, &state).await;

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(state.lookups(), vec!["inbox-1".to_owned()]);
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:58 "deferSendWhileAccountReconnecting > parks the job and throws DelayedError while the account is reconnecting"
#[tokio::test]
async fn parks_the_job_and_throws_delayederror_while_the_account_is() {
    let state = FakeConnectionState::new(Ok(SendDecision::Defer));
    let job = build_job(channel_message("inbox-1"), 1, 0, Some("token"));

    let result = run(&job, &state).await;

    match result {
        Err(JobError::Delayed { run_at_ms }) => assert!(run_at_ms > NOW, "run_at {run_at_ms}"),
        other => panic!("expected Delayed, got {other:?}"),
    }
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:72 "deferSendWhileAccountReconnecting > sends anyway once the deferral budget is spent"
#[tokio::test]
async fn sends_anyway_once_the_deferral_budget_is_spent() {
    let state = FakeConnectionState::new(Ok(SendDecision::Defer));
    let max = CONNECTION_SEND_DEFERRAL_POLICY.max_deferrals;
    let job = build_job(channel_message("inbox-1"), max + 1, 0, Some("token"));

    let result = run(&job, &state).await;

    assert!(result.is_ok(), "{result:?}");
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:87 "deferSendWhileAccountReconnecting > still defers on the last deferral inside the budget"
#[tokio::test]
async fn still_defers_on_the_last_deferral_inside_the_budget() {
    let state = FakeConnectionState::new(Ok(SendDecision::Defer));
    let max = CONNECTION_SEND_DEFERRAL_POLICY.max_deferrals;
    let job = build_job(channel_message("inbox-1"), max, 0, Some("token"));

    let result = run(&job, &state).await;

    assert!(
        matches!(result, Err(JobError::Delayed { .. })),
        "{result:?}"
    );
}

async fn assert_gated_on_i2(action: &str) {
    let state = FakeConnectionState::new(Ok(SendDecision::Send));
    let data = json!({ "conversation": {}, "contactInbox": { "inboxId": "i-2" } });
    let job = build_job(json!({ "type": action, "data": data }), 1, 0, Some("token"));

    let _ = run(&job, &state).await;

    assert_eq!(state.lookups(), vec!["i-2".to_owned()]);
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:99 "deferSendWhileAccountReconnecting > sendWhatsappTemplateMessage is gated on its contact inbox's inbox"
#[tokio::test]
async fn is_gated_on_its_contact_inbox_s_inbox_send_whatsapp_template_message() {
    assert_gated_on_i2("sendWhatsappTemplateMessage").await;
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:99 "deferSendWhileAccountReconnecting > sendWhatsappTemplateToConversation is gated on its contact inbox's inbox"
#[tokio::test]
async fn is_gated_on_its_contact_inbox_s_inbox_send_whatsapp_template_to_conversation() {
    assert_gated_on_i2("sendWhatsappTemplateToConversation").await;
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:99 "deferSendWhileAccountReconnecting > sendMessengerTemplateMessage is gated on its contact inbox's inbox"
#[tokio::test]
async fn is_gated_on_its_contact_inbox_s_inbox_send_messenger_template_message() {
    assert_gated_on_i2("sendMessengerTemplateMessage").await;
}

async fn assert_not_gated(data: Value) {
    let state = FakeConnectionState::new(Ok(SendDecision::Send));
    let job = build_job(data, 1, 0, Some("token"));

    let result = run(&job, &state).await;

    assert!(result.is_ok(), "{result:?}");
    assert!(state.lookups().is_empty());
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:118 "deferSendWhileAccountReconnecting > a flow step with no contact inbox is not gated and costs no query"
#[tokio::test]
async fn is_not_gated_and_costs_no_query_flow_step() {
    assert_not_gated(json!({ "type": "sendFlowMessage", "data": { "conversationId": "c" } })).await;
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:118 "deferSendWhileAccountReconnecting > a chat message addressed by conversation only is not gated and costs no query"
#[tokio::test]
async fn is_not_gated_and_costs_no_query_chat_message() {
    assert_not_gated(json!({ "type": "sendChatMessage", "data": { "conversation": {} } })).await;
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:118 "deferSendWhileAccountReconnecting > a typing indicator is not gated and costs no query"
#[tokio::test]
async fn is_not_gated_and_costs_no_query_typing_indicator() {
    assert_not_gated(
        json!({ "type": "sendTyping", "data": { "contactInbox": { "inboxId": "i" } } }),
    )
    .await;
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:118 "deferSendWhileAccountReconnecting > a delete is not gated and costs no query"
#[tokio::test]
async fn is_not_gated_and_costs_no_query_delete() {
    assert_not_gated(json!({ "type": "deleteChannelMessage", "data": {} })).await;
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:118 "deferSendWhileAccountReconnecting > an edit is not gated and costs no query"
#[tokio::test]
async fn is_not_gated_and_costs_no_query_edit() {
    assert_not_gated(json!({ "type": "editChannelMessage", "data": {} })).await;
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:140 "deferSendWhileAccountReconnecting > without a lock token the job cannot be parked, so it sends"
#[tokio::test]
async fn without_a_lock_token_the_job_cannot_be_parked_so_it_sends() {
    let state = FakeConnectionState::new(Ok(SendDecision::Defer));
    let job = build_job(channel_message("inbox-1"), 1, 0, None);

    let result = run(&job, &state).await;

    assert!(result.is_ok(), "{result:?}");
    assert!(state.lookups().is_empty());
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:151 "deferSendWhileAccountReconnecting > a failing lookup sends, so the handler keeps its own retry and failure path"
#[tokio::test]
async fn a_failing_lookup_sends_so_the_handler_keeps_its_own_retry() {
    let state = FakeConnectionState::new(Err("db down".to_owned()));
    let job = build_job(channel_message("inbox-1"), 1, 0, Some("token"));

    let result = run(&job, &state).await;

    assert!(result.is_ok(), "{result:?}");
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:165 "deferralDelayMs > backs off exponentially and never exceeds the cap"
#[test]
fn backs_off_exponentially_and_never_exceeds_the_cap() {
    let base = CONNECTION_SEND_DEFERRAL_POLICY.base_delay_ms;
    let max = CONNECTION_SEND_DEFERRAL_POLICY.max_delay_ms;
    assert_eq!(base, 30_000);
    assert_eq!(max, 300_000);
    assert_eq!(deferral_delay_ms(0, &|| 1.0), base);
    assert_eq!(deferral_delay_ms(1, &|| 1.0), base * 2);
    assert_eq!(deferral_delay_ms(10, &|| 1.0), max);
    assert_eq!(deferral_delay_ms(10, &|| 0.0), max / 2);
}

/// Port of: apps/worker/__tests__/connection-send-gate.test.ts:177 "deferralDelayMs > keeps the whole wait around ten minutes, not a poll every few seconds"
#[test]
fn keeps_the_whole_wait_around_ten_minutes_not_a_poll_every() {
    let total: u64 = (0..CONNECTION_SEND_DEFERRAL_POLICY.max_deferrals)
        .map(|deferral| deferral_delay_ms(deferral, &|| 1.0))
        .sum();
    assert!(total >= 10 * 60_000, "total {total}");
    assert!(total <= 15 * 60_000, "total {total}");
}
