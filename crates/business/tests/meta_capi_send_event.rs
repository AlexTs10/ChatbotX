//! Port (partial) of `apps/worker/__tests__/send-meta-capi-event.test.ts`
//! over a migrated database, a fake queue and a scripted Graph:
//! `enqueueEvent` then `handleSendMetaCapiEvent`.

mod common;

use std::sync::Mutex;

use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::email_marketing::seal_auth;
use chatbotx_business::meta_capi_send::{
    EnqueueError, EnqueueInput, SendDeps, enqueue_event, send_meta_capi_event,
};
use chatbotx_business::meta_catalog::process_encryption;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{AddResult, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use common::{WS, contact_inbox, contact_with, inbox, setup, two_workspaces};
use serde_json::{Value, json};

#[derive(Default)]
struct Queue {
    added: Mutex<Vec<(String, Value, JobOptions)>>,
}

impl Enqueuer for Queue {
    fn add(
        &self,
        _: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let job_id = opts.job_id.clone().unwrap_or_default();
        self.added
            .lock()
            .unwrap()
            .push((name.to_owned(), data, opts));
        Box::pin(async move {
            Ok(AddResult {
                job_id,
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _: QueueName,
        _: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        Box::pin(async { Err(JobsError::Other("not used".into())) })
    }
}

/// Answers every call with `reply`; records the requests.
struct Graph {
    reply: UpstreamReply,
    seen: Mutex<Vec<UpstreamRequest>>,
}

impl Graph {
    fn ok() -> Self {
        Graph::with(Ok((200, json!({ "events_received": 1 }).to_string())))
    }

    fn with(reply: UpstreamReply) -> Self {
        Graph {
            reply,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn bodies(&self) -> Vec<(String, Value)> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r.url.clone(),
                    r.body
                        .as_deref()
                        .map(|b| serde_json::from_str(b).unwrap())
                        .unwrap_or(Value::Null),
                )
            })
            .collect()
    }
}

impl Upstream for Graph {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        self.seen.lock().unwrap().push(request);
        self.reply.clone()
    }
}

fn manual_token() -> String {
    let sealed = process_encryption()
        .encrypt_object(&json!({ "accessToken": "manual-token" }), None)
        .unwrap();
    serde_json::to_string(&sealed).unwrap()
}

/// Messenger integration 6001 on inbox 4001 with a manual token and a
/// dataset; contact 3001 / contact inbox 5001.
async fn seed(db: &chatbotx_db::Db, channel: &'static str) {
    let auth = seal_auth(&json!({ "tokens": { "accessToken": "oauth-token" } })).unwrap();
    let token = manual_token();
    setup(db, move |tx| {
        contact_with(tx, 3001, WS, Some("Mary"), Some("mary@example.com"), None)?;
        inbox(tx, 4001, WS, channel)?;
        contact_inbox(tx, 5001, 3001, 4001, channel, "psid-1")?;
        match channel {
            "messenger" => tx.execute(
                r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId", "datasetId", "capiAccessToken")
                   VALUES (6001, ?1, 'page-1', 'Page', '[]', '[]', '[]', ?2, 4001, 'dataset-1', ?3)"#,
                params![auth, WS, token],
            )?,
            _ => tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId", "datasetId", "capiAccessToken")
                   VALUES (6001, ?1, 'pn-1', 'waba-1', 'b', 'WA', ?2, 4001, 'dataset-1', ?3)"#,
                params![auth, WS, token],
            )?,
        };
        Ok(())
    })
    .await;
}

fn input(channel: &str) -> EnqueueInput {
    EnqueueInput {
        workspace_id: WS,
        channel: channel.into(),
        contact_inbox_id: 5001,
        inbox_id: 4001,
        source: "flowStep".into(),
        source_key: "flow:1:5001:abc".into(),
        ..EnqueueInput::default()
    }
}

async fn enqueue(db: &chatbotx_db::Db, input: EnqueueInput) -> i64 {
    let queue = Queue::default();
    let row = enqueue_event(db, &queue, input).await.unwrap().unwrap();
    let added = queue.added.lock().unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].0, "sendMetaCapiEvent");
    assert_eq!(
        added[0].2.job_id.as_deref(),
        Some(format!("meta-capi-send-{}", row.id).as_str())
    );
    row.id
}

async fn status(db: &chatbotx_db::Db, id: i64) -> (String, Option<String>) {
    db.read(move |c| {
        Ok(c.query_row(
            r#"SELECT "capiStatus", "capiError" FROM "MetaCapiEvent" WHERE "id" = ?1"#,
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })
    .await
    .unwrap()
}

async fn send(db: &chatbotx_db::Db, up: &Graph, id: i64) -> Result<(), String> {
    let deps = SendDeps {
        db,
        up,
        events: None,
    };
    send_meta_capi_event(&deps, id, WS).await
}

/// Port of: "sends a manual-token event with a stored dataset without refreshing scope"
/// (+ "sends a pending event and marks it sent", "forwards value and currency to the conversion event payload")
#[tokio::test]
async fn sends_a_manual_token_event_and_marks_it_sent() {
    let t = two_workspaces().await;
    seed(&t, "messenger").await;
    let id = enqueue(
        &t,
        EnqueueInput {
            event_name: Some("Purchase".into()),
            value: Some("19.90".into()),
            currency: Some("usd".into()),
            ..input("messenger")
        },
    )
    .await;
    let up = Graph::ok();
    send(&t, &up, id).await.unwrap();
    assert_eq!(status(&t, id).await.0, "sent");
    let calls = up.bodies();
    assert_eq!(calls.len(), 1, "no scope refresh, no dataset create");
    assert_eq!(
        calls[0].0,
        "https://graph.facebook.com/v24.0/dataset-1/events"
    );
    let data = &calls[0].1["data"][0];
    assert_eq!(data["event_name"], "Purchase");
    assert_eq!(data["event_id"], "flow:1:5001:abc");
    assert_eq!(data["messaging_channel"], "messenger");
    assert_eq!(data["user_data"]["page_id"], "page-1");
    assert_eq!(data["user_data"]["page_scoped_user_id"], "psid-1");
    assert!(data["user_data"]["em"].is_array());
    assert_eq!(
        data["custom_data"],
        json!({ "currency": "USD", "value": 19.9 })
    );
    let auth = up.seen.lock().unwrap()[0].headers.clone();
    assert!(auth.contains(&("authorization".into(), "Bearer manual-token".into())));
}

/// Port of: "forwards contentType and contentIds to the conversion event payload"
#[tokio::test]
async fn forwards_content_type_and_ids() {
    let t = two_workspaces().await;
    seed(&t, "messenger").await;
    let id = enqueue(
        &t,
        EnqueueInput {
            event_name: Some("ViewContent".into()),
            action_source: Some("chat".into()),
            content_type: Some("product".into()),
            content_ids: Some(" sku-1, ,sku-2 ".into()),
            ..input("messenger")
        },
    )
    .await;
    let up = Graph::ok();
    send(&t, &up, id).await.unwrap();
    let data = &up.bodies()[0].1["data"][0];
    assert_eq!(
        data["custom_data"],
        json!({ "content_type": "product", "content_ids": ["sku-1", "sku-2"] })
    );
    // "a non-business_messaging action source identifies the person by hashed data only"
    assert_eq!(data["action_source"], "chat");
    assert!(data.get("messaging_channel").is_none());
    assert!(data["user_data"].get("page_id").is_none());
}

/// Port of: "skips a manual-token event without a stored dataset"
#[tokio::test]
async fn skips_a_manual_token_event_without_a_dataset() {
    let t = two_workspaces().await;
    seed(&t, "messenger").await;
    t.write(|c| {
        c.execute(
            r#"UPDATE "IntegrationMessenger" SET "datasetId" = NULL"#,
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let id = enqueue(&t, input("messenger")).await;
    let up = Graph::ok();
    send(&t, &up, id).await.unwrap();
    assert_eq!(status(&t, id).await.0, "skipped_no_scope");
    assert!(up.bodies().is_empty());
}

/// Port of: "marks terminal failures failed and logs them"
#[tokio::test]
async fn marks_terminal_failures_failed() {
    let t = two_workspaces().await;
    seed(&t, "messenger").await;
    let id = enqueue(&t, input("messenger")).await;
    let up = Graph::with(Ok((
        400,
        json!({ "error": { "message": "Invalid parameter", "code": 100 } }).to_string(),
    )));
    send(&t, &up, id).await.unwrap();
    let (status, error) = status(&t, id).await;
    assert_eq!(status, "failed");
    assert!(error.unwrap().contains("Invalid parameter"));
}

/// Port of: "throws retryable failures so BullMQ retries"
#[tokio::test]
async fn throws_retryable_failures() {
    let t = two_workspaces().await;
    seed(&t, "messenger").await;
    let id = enqueue(&t, input("messenger")).await;
    let up = Graph::with(Ok((503, "{}".into())));
    assert!(send(&t, &up, id).await.is_err());
    assert_eq!(status(&t, id).await.0, "pending");
}

/// Port of: "non-pending event is a no-op"
#[tokio::test]
async fn non_pending_event_is_a_no_op() {
    let t = two_workspaces().await;
    seed(&t, "messenger").await;
    let id = enqueue(&t, input("messenger")).await;
    let up = Graph::ok();
    send(&t, &up, id).await.unwrap();
    send(&t, &up, id).await.unwrap();
    assert_eq!(up.bodies().len(), 1);
}

/// Port of: "user-disconnected integration skips the event without sending"
#[tokio::test]
async fn disconnected_integration_skips_the_event() {
    let t = two_workspaces().await;
    seed(&t, "messenger").await;
    let id = enqueue(&t, input("messenger")).await;
    t.write(|c| {
        c.execute(
            r#"UPDATE "IntegrationMessenger" SET "capiDisconnectedAt" = 1"#,
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let up = Graph::ok();
    send(&t, &up, id).await.unwrap();
    assert_eq!(status(&t, id).await.0, "skipped_disconnected");
    assert!(up.bodies().is_empty());
}

/// Port of: "marks failed when the contact inbox belongs to a different inbox than the resolved integration (Phase 0 guard)"
#[tokio::test]
async fn foreign_contact_inbox_marks_failed() {
    let t = two_workspaces().await;
    seed(&t, "messenger").await;
    let id = enqueue(&t, input("messenger")).await;
    t.write(|c| {
        inbox(c, 4002, WS, "messenger")?;
        c.execute(r#"UPDATE "ContactInbox" SET "inboxId" = 4002"#, [])?;
        Ok(())
    })
    .await
    .unwrap();
    let up = Graph::ok();
    send(&t, &up, id).await.unwrap();
    assert_eq!(
        status(&t, id).await,
        (
            "failed".into(),
            Some("contactInboxWorkspaceMismatch".into())
        )
    );
    assert!(up.bodies().is_empty());
}

/// Port of: "skips a whatsapp event whose contact has no ctwa_clid"
/// (+ "sends a manual-token whatsapp event with a stored dataset without refreshing scope")
#[tokio::test]
async fn whatsapp_needs_a_ctwa_clid() {
    let t = two_workspaces().await;
    seed(&t, "whatsapp").await;
    let id = enqueue(&t, input("whatsapp")).await;
    let up = Graph::ok();
    send(&t, &up, id).await.unwrap();
    assert_eq!(status(&t, id).await.0, "skipped_no_identity");
    t.write(|c| {
        c.execute(
            r#"UPDATE "ContactInbox" SET "referral" = '{"ctwaClid":"clid-1"}'"#,
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let id = enqueue(
        &t,
        EnqueueInput {
            source_key: "flow:2:5001:2026-11-22".into(),
            ..input("whatsapp")
        },
    )
    .await;
    send(&t, &up, id).await.unwrap();
    assert_eq!(status(&t, id).await.0, "sent");
    let data = &up.bodies()[0].1["data"][0];
    assert_eq!(data["messaging_channel"], "whatsapp");
    assert_eq!(data["user_data"]["whatsapp_business_account_id"], "waba-1");
    assert_eq!(data["user_data"]["ctwa_clid"], "clid-1");
}

/// `enqueueEventInput`: a Purchase without value / currency is refused with
/// the prettified issues; no row, no job.
#[tokio::test]
async fn enqueue_refuses_a_purchase_without_value() {
    let t = two_workspaces().await;
    seed(&t, "messenger").await;
    let queue = Queue::default();
    let refused = enqueue_event(
        &t,
        &queue,
        EnqueueInput {
            event_name: Some("Purchase".into()),
            value: Some("250Hung".into()),
            ..input("messenger")
        },
    )
    .await
    .unwrap_err();
    let EnqueueError::Invalid(text) = refused else {
        panic!("{refused:?}")
    };
    assert!(
        text.contains("✖ Value must be a plain number such as 19.99\n  → at value"),
        "{text}"
    );
    assert!(queue.added.lock().unwrap().is_empty());
    // a duplicate source key inserts nothing and re-enqueues the pending row
    enqueue(&t, input("messenger")).await;
    let queue = Queue::default();
    assert!(
        enqueue_event(&t, &queue, input("messenger"))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(queue.added.lock().unwrap().len(), 1);
}
