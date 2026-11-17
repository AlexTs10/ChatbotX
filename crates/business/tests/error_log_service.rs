//! Ported from `packages/business/__tests__/error-log.list.test.ts` and
//! `error-log.service.test.ts` (Q.D11 slice 4). The TS tests read the
//! drizzle `where` and the mocked `emit`; here the search terms are a pure
//! function ([`search_terms`]), the list runs on SQLite, and `emit` is a
//! recording [`EventEmitter`].

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use chatbotx_business::error_log::{
    self, ListInput, ProviderErrorInput, SearchTerm, Thrown, log_provider_error,
    log_provider_error_for_channel, log_provider_errors, search_terms,
};
use chatbotx_db::repo::SortItem;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::BoxFuture;
use chatbotx_jobs::events::bus::EventBusError;
use chatbotx_jobs::events::emit::{EventBuses, EventEmitter};
use common::{OTHER_WS, WS, scalar_i64, setup, two_workspaces};
use serde_json::Value;

// --- listErrorLogs ---

#[test]
fn matches_a_provider_by_the_label_the_table_shows_not_just_the_stored_slug() {
    assert!(search_terms(Some("Email")).contains(&SearchTerm::ActionIn(vec!["smtp"])));
}

#[test]
fn keeps_the_free_text_search_over_action_and_detail() {
    let or = search_terms(Some("timeout"));
    assert!(or.contains(&SearchTerm::ActionIlike("%timeout%".into())));
    assert!(or.contains(&SearchTerm::DetailIlike("%timeout%".into())));
    assert_eq!(or.len(), 2);
}

#[test]
fn never_searches_the_channel_side_contact_id() {
    assert_eq!(
        search_terms(Some("psid-1")),
        vec![
            SearchTerm::ActionIlike("%psid-1%".into()),
            SearchTerm::DetailIlike("%psid-1%".into()),
        ]
    );
}

#[test]
fn applies_no_search_terms_without_a_keyword() {
    assert!(search_terms(None).is_empty());
}

async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        common::contact(tx, 7, WS)?;
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (70, ?1, 7)"#,
            [WS],
        )?;
        for (id, ws, action, detail, created, contact) in [
            (1, WS, "smtp", "mail refused", 1_000, None),
            (2, WS, "messenger", "token timeout", 3_000, Some(7)),
            (3, WS, "whatsapp", "rate limited", 2_000, None),
            (4, OTHER_WS, "smtp", "timeout", 4_000, None),
        ] {
            tx.execute(
                r#"INSERT INTO "ErrorLog" ("id", "workspaceId", "action", "detail", "createdAt", "updatedAt", "contactId", "sourceId", "stackTrace") VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, 'psid-1', '    at x')"#,
                params![id, ws, action, detail, created, contact],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

fn ids(page: &error_log::ErrorLogPage) -> Vec<i64> {
    page.data.iter().map(|r| r.log.id).collect()
}

#[tokio::test]
async fn lists_newest_first_by_default_with_the_contact_and_no_withheld_columns() {
    let db = seeded().await;
    let page = error_log::list(
        &db,
        ListInput {
            workspace_id: WS,
            ..ListInput::default()
        },
    )
    .await
    .expect("list");
    assert_eq!(ids(&page), vec![2, 3, 1]);
    assert_eq!(page.page_count, 1);
    let first = &page.data[0];
    assert_eq!(first.contact.as_ref().map(|c| c.id), Some(7));
    assert_eq!(first.conversation_id, Some(70));
    assert!(
        page.data
            .iter()
            .all(|r| r.log.source_id.is_none() && r.log.stack_trace.is_none())
    );
}

#[tokio::test]
async fn searches_by_keyword_and_by_provider_label() {
    let db = seeded().await;
    let search = |keyword: &str| ListInput {
        workspace_id: WS,
        keyword: Some(keyword.into()),
        ..ListInput::default()
    };
    let page = error_log::list(&db, search("TIMEOUT")).await.expect("list");
    assert_eq!(ids(&page), vec![2]);
    let page = error_log::list(&db, search("Email")).await.expect("list");
    assert_eq!(ids(&page), vec![1]);
    // The channel-side id is never searched.
    let page = error_log::list(&db, search("psid-1")).await.expect("list");
    assert!(page.data.is_empty());
}

#[tokio::test]
async fn sorts_only_by_the_allowed_columns() {
    let db = seeded().await;
    let sorted = |id: &str| ListInput {
        workspace_id: WS,
        sort: Some(vec![SortItem {
            id: id.into(),
            desc: false,
        }]),
        ..ListInput::default()
    };
    let page = error_log::list(&db, sorted("action")).await.expect("list");
    assert_eq!(ids(&page), vec![2, 1, 3]);
    // `sourceId` is withheld: ignored, so the default order applies.
    let page = error_log::list(&db, sorted("sourceId"))
        .await
        .expect("list");
    assert_eq!(ids(&page), vec![2, 3, 1]);
}

// --- deleteErrorLogs ---

#[tokio::test]
async fn deletes_the_given_ids_scoped_to_the_workspace() {
    let db = seeded().await;
    let deleted = error_log::delete(&db, WS, vec![1, 2, 4])
        .await
        .expect("delete");
    assert_eq!(deleted, 2);
    let count = |ws| {
        scalar_i64(
            &db,
            r#"SELECT COUNT(*) FROM "ErrorLog" WHERE "workspaceId" = ?1"#,
            ws,
        )
    };
    assert_eq!(count(WS).await, Some(1));
    assert_eq!(count(OTHER_WS).await, Some(1));
}

#[tokio::test]
async fn skips_the_query_when_no_ids_are_given() {
    let db = seeded().await;
    assert_eq!(error_log::delete(&db, WS, vec![]).await.expect("delete"), 0);
}

// --- logProviderError ---

enum Reply {
    Ok(&'static str),
    Reject,
    Sync,
}

#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<(String, Value)>>,
    replies: Mutex<VecDeque<Reply>>,
}

impl EventEmitter for Recorder {
    fn emit<'a>(
        &'a self,
        event_type: &'a str,
        payload: Value,
    ) -> Result<BoxFuture<'a, Result<String, EventBusError>>, EventBusError> {
        self.calls
            .lock()
            .unwrap()
            .push((event_type.to_string(), payload));
        match self.replies.lock().unwrap().pop_front() {
            None => Ok(Box::pin(async { Ok("stream-id".to_string()) })),
            Some(Reply::Ok(id)) => Ok(Box::pin(async move { Ok(id.to_string()) })),
            Some(Reply::Reject) => Ok(Box::pin(async {
                Err(EventBusError::Store("redis down".into()))
            })),
            Some(Reply::Sync) => Err(EventBusError::Other("routing".into())),
        }
    }
}

fn buses(recorder: &Arc<Recorder>) -> EventBuses {
    let emitter: Arc<dyn EventEmitter> = recorder.clone();
    EventBuses {
        message: emitter.clone(),
        flow: emitter.clone(),
        dashboard: emitter.clone(),
        error_log: emitter,
    }
}

fn error(message: &str) -> Thrown {
    Thrown::Error {
        message: message.into(),
        stack: None,
        status: None,
    }
}

fn with_stack(message: &str, stack: &str) -> Thrown {
    Thrown::Error {
        message: message.into(),
        stack: Some(stack.into()),
        status: None,
    }
}

/// The first emitted payload.
async fn payload_of(input: ProviderErrorInput) -> Value {
    let recorder = Arc::new(Recorder::default());
    log_provider_error(Some(&buses(&recorder)), input).await;
    let calls = recorder.calls.lock().unwrap();
    assert_eq!(calls[0].0, "error-log:recorded");
    calls[0].1.clone()
}

#[tokio::test]
async fn enqueues_the_provider_as_is_with_no_operation_suffix() {
    let p = payload_of(ProviderErrorInput::new("mailchimp", "ws-1", error("boom"))).await;
    assert_eq!(p["provider"], "mailchimp");
    assert_eq!(p["workspaceId"], "ws-1");
}

#[tokio::test]
async fn passes_contact_id_through_when_given() {
    let mut input = ProviderErrorInput::new("messenger", "ws-1", error("boom"));
    input.contact_id = Some("c-7".into());
    assert_eq!(payload_of(input).await["contactId"], "c-7");
}

#[tokio::test]
async fn omits_contact_id_when_it_is_null() {
    let p = payload_of(ProviderErrorInput::new("messenger", "ws-1", error("boom"))).await;
    assert!(p.get("contactId").is_none());
}

#[tokio::test]
async fn passes_the_contacts_channel_side_source_id_through_when_given() {
    let mut input = ProviderErrorInput::new("messenger", "ws-1", error("boom"));
    input.source_id = Some("psid-7".into());
    assert_eq!(payload_of(input).await["sourceId"], "psid-7");
}

#[tokio::test]
async fn omits_source_id_when_it_is_null() {
    let p = payload_of(ProviderErrorInput::new("messenger", "ws-1", error("boom"))).await;
    assert!(p.get("sourceId").is_none());
}

#[tokio::test]
async fn carries_source_id_with_no_contact_id_the_contact_creation_path() {
    let mut input = ProviderErrorInput::new("messenger", "ws-1", error("getProfile failed"));
    input.source_id = Some("psid-7".into());
    let p = payload_of(input).await;
    assert_eq!(p["sourceId"], "psid-7");
    assert!(p.get("contactId").is_none());
}

#[tokio::test]
async fn scrubs_an_access_token_from_the_stored_message_and_stack() {
    let url = "https://graph.facebook.com/v25.0/1?fields=name&access_token=SECRETTOKEN123";
    let mut input = ProviderErrorInput::new(
        "instagram",
        "ws-1",
        error(&format!("Request to {url} timed out")),
    );
    input.source_id = Some("igsid-7".into());
    let p = payload_of(input).await;
    assert!(!p.to_string().contains("SECRETTOKEN123"));
    assert!(
        p["error"]["message"]
            .as_str()
            .unwrap()
            .contains("access_token=[redacted]")
    );
}

#[tokio::test]
async fn survives_a_json_round_trip_unchanged_keys_and_all() {
    let mut input = ProviderErrorInput::new("messenger", "ws-1", error("boom"));
    input.source_id = Some("psid-7".into());
    let p = payload_of(input).await;
    let back: Value = serde_json::from_str(&p.to_string()).unwrap();
    assert_eq!(back, p);
}

#[tokio::test]
async fn truncates_detail_without_touching_source_id() {
    let mut input = ProviderErrorInput::new("messenger", "ws-1", error(&"x".repeat(9000)));
    input.source_id = Some("psid-7".into());
    let p = payload_of(input).await;
    assert_eq!(p["error"]["message"].as_str().unwrap().len(), 8192);
    assert_eq!(p["sourceId"], "psid-7");
}

#[tokio::test]
async fn takes_http_code_from_an_sdk_exception_status() {
    let thrown = Thrown::Error {
        message: "rate limited".into(),
        stack: None,
        status: Some(429),
    };
    let p = payload_of(ProviderErrorInput::new("whatsapp", "ws-1", thrown)).await;
    assert_eq!(p["error"]["httpCode"], "429");
}

#[tokio::test]
async fn maps_the_minus_one_unknown_sentinel_to_null_never_to_minus_one() {
    let thrown = Thrown::Object {
        message: Some("unknown".into()),
        status: Some(-1),
    };
    let p = payload_of(ProviderErrorInput::new("openai", "ws-1", thrown)).await;
    assert!(p["error"]["httpCode"].is_null());
}

#[tokio::test]
async fn prefers_an_explicit_http_code_override() {
    let thrown = Thrown::Error {
        message: "boom".into(),
        stack: None,
        status: Some(500),
    };
    let mut input = ProviderErrorInput::new("meta-conversions", "ws-1", thrown);
    input.http_code = Some(Some("400".into()));
    assert_eq!(payload_of(input).await["error"]["httpCode"], "400");
}

#[tokio::test]
async fn writes_null_http_code_for_a_plain_non_http_error() {
    let input = ProviderErrorInput::new(
        "google-sheets",
        "ws-1",
        error("cannot read property of undefined"),
    );
    assert!(payload_of(input).await["error"]["httpCode"].is_null());
}

#[tokio::test]
async fn carries_the_stack_frames_without_the_message_prefix() {
    let thrown = with_stack(
        "boom",
        "Error: boom\n    at /srv/app/packages/business/src/x.ts:1:1",
    );
    let p = payload_of(ProviderErrorInput::new("drip", "ws-1", thrown)).await;
    assert_eq!(p["error"]["message"], "boom");
    assert_eq!(
        p["error"]["stackTrace"],
        "    at /srv/app/packages/business/src/x.ts:1:1"
    );
    assert!(!p["error"]["stackTrace"].as_str().unwrap().contains("boom"));
}

#[tokio::test]
async fn keeps_frames_when_a_pathological_message_would_fill_the_budget() {
    let message = "x".repeat(9000);
    let thrown = with_stack(
        &message,
        &format!("Error: {message}\n    at /srv/app/a.ts:1:1"),
    );
    let p = payload_of(ProviderErrorInput::new("drip", "ws-1", thrown)).await;
    assert_eq!(p["error"]["stackTrace"], "    at /srv/app/a.ts:1:1");
}

#[tokio::test]
async fn caps_the_stored_frames_at_2048_characters() {
    let stack = format!("Error: boom{}", "\n    at /srv/app/deep.ts:1:1".repeat(500));
    let p = payload_of(ProviderErrorInput::new(
        "drip",
        "ws-1",
        with_stack("boom", &stack),
    ))
    .await;
    let trace = p["error"]["stackTrace"].as_str().unwrap();
    assert_eq!(trace.len(), 2048);
    assert!(trace.starts_with("    at /srv/app/deep.ts"));
}

#[tokio::test]
async fn skips_frame_shaped_lines_that_belong_to_the_message() {
    let inner = "Error: upstream 502\n    at /srv/app/remote.ts:1:1";
    let message = format!("Provider failed: {inner}");
    let stack = format!("Error: {message}\n    at /srv/app/throw-site.ts:2:2");
    let p = payload_of(ProviderErrorInput::new(
        "drip",
        "ws-1",
        with_stack(&message, &stack),
    ))
    .await;
    assert_eq!(
        p["error"]["stackTrace"],
        "    at /srv/app/throw-site.ts:2:2"
    );
    assert!(
        !p["error"]["stackTrace"]
            .as_str()
            .unwrap()
            .contains("remote.ts")
    );
    assert!(
        p["error"]["message"]
            .as_str()
            .unwrap()
            .contains("remote.ts")
    );
}

#[tokio::test]
async fn omits_the_stack_for_a_non_error_throwable() {
    let thrown = Thrown::Object {
        message: Some("remote failure".into()),
        status: Some(400),
    };
    let p = payload_of(ProviderErrorInput::new("klaviyo", "ws-1", thrown)).await;
    assert!(p["error"].get("stackTrace").is_none());
}

#[tokio::test]
async fn omits_the_stack_when_the_error_carries_no_frames() {
    let p = payload_of(ProviderErrorInput::new(
        "klaviyo",
        "ws-1",
        with_stack("boom", "Error: boom"),
    ))
    .await;
    assert!(p["error"].get("stackTrace").is_none());
}

#[tokio::test]
async fn prefers_a_caller_supplied_stack_over_deriving_one_from_the_error() {
    let thrown = Thrown::Object {
        message: Some("remote failure".into()),
        status: Some(401),
    };
    let mut input = ProviderErrorInput::new("messenger", "ws-1", thrown);
    input.stack_trace = Some(Some(
        "    at /srv/app/apps/worker/src/chat/handlers/x.ts:1:1".into(),
    ));
    assert_eq!(
        payload_of(input).await["error"]["stackTrace"],
        "    at /srv/app/apps/worker/src/chat/handlers/x.ts:1:1"
    );
}

#[tokio::test]
async fn caps_a_caller_supplied_stack_at_2048_characters() {
    let thrown = Thrown::Object {
        message: Some("remote failure".into()),
        status: Some(401),
    };
    let mut input = ProviderErrorInput::new("messenger", "ws-1", thrown);
    input.stack_trace = Some(Some("    at /srv/app/deep.ts:1:1\n".repeat(500)));
    let p = payload_of(input).await;
    assert_eq!(p["error"]["stackTrace"].as_str().unwrap().len(), 2048);
}

#[tokio::test]
async fn omits_the_stack_when_the_error_has_no_stack_at_all() {
    let p = payload_of(ProviderErrorInput::new("klaviyo", "ws-1", error("boom"))).await;
    assert!(p["error"].get("stackTrace").is_none());
}

#[tokio::test]
async fn stringifies_a_non_error_throwable_rather_than_losing_it() {
    let thrown = Thrown::Text("plain string failure".into());
    let p = payload_of(ProviderErrorInput::new("klaviyo", "ws-1", thrown)).await;
    assert_eq!(p["error"]["message"], "plain string failure");
}

#[tokio::test]
async fn does_not_write_object_object_for_a_message_less_object_throw() {
    let thrown = Thrown::Object {
        message: None,
        status: None,
    };
    let p = payload_of(ProviderErrorInput::new("klaviyo", "ws-1", thrown)).await;
    assert_eq!(p["error"]["message"], "An unknown error occurred");
}

#[tokio::test]
async fn still_prefers_a_message_carried_on_a_non_error_object() {
    let thrown = Thrown::Object {
        message: Some("remote refused".into()),
        status: None,
    };
    let p = payload_of(ProviderErrorInput::new("klaviyo", "ws-1", thrown)).await;
    assert_eq!(p["error"]["message"], "remote refused");
}

#[tokio::test]
async fn never_throws_when_the_event_bus_is_unavailable() {
    let recorder = Arc::new(Recorder::default());
    recorder.replies.lock().unwrap().push_back(Reply::Reject);
    // `log_provider_error` returns `()`: the failure is only logged.
    log_provider_error(
        Some(&buses(&recorder)),
        ProviderErrorInput::new("sendgrid", "ws-1", error("boom")),
    )
    .await;
    assert_eq!(recorder.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn writes_no_row_for_an_unknown_channel() {
    let recorder = Arc::new(Recorder::default());
    let input = ProviderErrorInput::new("", "ws-1", error("boom"));
    log_provider_error_for_channel(Some(&buses(&recorder)), "omnichannel", input.clone()).await;
    assert!(recorder.calls.lock().unwrap().is_empty());
    log_provider_error_for_channel(Some(&buses(&recorder)), "telegram", input).await;
    assert_eq!(recorder.calls.lock().unwrap()[0].1["provider"], "telegram");
}

#[tokio::test]
async fn emits_nothing_without_an_event_bus() {
    // `isNoRedisEnv()`: no buses, nothing emitted, nothing failed.
    let inputs = [ProviderErrorInput::new("messenger", "ws-1", error("boom"))];
    assert!(log_provider_errors(None, &inputs).await.is_empty());
}

// --- logProviderErrors ---

fn batch(n: usize, provider: &str) -> Vec<ProviderErrorInput> {
    (0..n)
        .map(|i| ProviderErrorInput::new(provider, &format!("ws-{i}"), error("boom")))
        .collect()
}

#[tokio::test]
async fn emits_one_event_per_failure() {
    let recorder = Arc::new(Recorder::default());
    log_provider_errors(Some(&buses(&recorder)), &batch(40, "messenger")).await;
    let calls = recorder.calls.lock().unwrap();
    assert_eq!(calls.len(), 40);
    assert!(
        calls
            .iter()
            .any(|(t, p)| t == "error-log:recorded" && p["workspaceId"] == "ws-0")
    );
}

#[tokio::test]
async fn gives_every_event_its_own_row_id_so_a_redelivery_cannot_duplicate() {
    let recorder = Arc::new(Recorder::default());
    log_provider_errors(Some(&buses(&recorder)), &batch(3, "whatsapp")).await;
    let ids: Vec<String> = recorder
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|(_, p)| p["id"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(ids.iter().all(|id| !id.is_empty()));
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        3
    );
}

#[tokio::test]
async fn reports_the_inputs_it_could_not_emit_instead_of_throwing() {
    let recorder = Arc::new(Recorder::default());
    recorder
        .replies
        .lock()
        .unwrap()
        .extend([Reply::Ok("stream-id"), Reply::Reject]);
    let failed = log_provider_errors(Some(&buses(&recorder)), &batch(2, "telegram")).await;
    assert_eq!(failed, vec![1]);
}

#[tokio::test]
async fn reports_a_synchronous_routing_failure_which_emit_signals_with_undefined() {
    let recorder = Arc::new(Recorder::default());
    recorder.replies.lock().unwrap().push_back(Reply::Sync);
    let failed = log_provider_errors(Some(&buses(&recorder)), &batch(1, "zalo")).await;
    assert_eq!(failed, vec![0]);
}

#[tokio::test]
async fn drops_a_schema_rejection_rather_than_reporting_it() {
    let recorder = Arc::new(Recorder::default());
    recorder.replies.lock().unwrap().push_back(Reply::Ok(""));
    let failed = log_provider_errors(Some(&buses(&recorder)), &batch(1, "instagram")).await;
    assert!(failed.is_empty());
}

#[tokio::test]
async fn emits_nothing_for_an_empty_batch() {
    let recorder = Arc::new(Recorder::default());
    assert!(
        log_provider_errors(Some(&buses(&recorder)), &[])
            .await
            .is_empty()
    );
    assert!(recorder.calls.lock().unwrap().is_empty());
}
