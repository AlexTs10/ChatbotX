//! The WhatsApp routing pipeline end to end on a migrated database: a standby copy
//! and its owner promotion (`received-message.ts` thread-control cases), the
//! hand-back resume flow (`handover-response.test.ts`) and `isOwnRetry`.
use serde_json::{Value, json};

use super::*;
use crate::inbound::tests::setup;

const PHONE: &str = "phone-1";
const USER: &str = "84900000001";

async fn whatsapp_rows(deps: &Deps) {
    deps.db
        .write(|tx| {
            for sql in [
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (7, 2, 'Wa', 'whatsapp', 'phone-1')"#,
                r#"INSERT INTO "Flow" ("id", "name", "workspaceId", "currentVersionId") VALUES (10, 'Resume', 2, 11)"#,
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId", "handoverResumeFlowId") VALUES (8, '{}', 'phone-1', 'w', 'b', 'Wa', 2, 7, 10)"#,
            ] {
                tx.execute(sql, [])?;
            }
            Ok(())
        })
        .await
        .unwrap();
}

fn message(ts: &str) -> Value {
    json!({ "id": "wamid.1", "from": USER, "type": "text", "text": { "body": "hi" }, "timestamp": ts })
}

fn delivery(field: &str, ts: &str) -> Value {
    let value = if field == "standby" {
        json!({ "metadata": { "phone_number_id": PHONE }, "standby": { "messages": [message(ts)] } })
    } else {
        json!({ "metadata": { "phone_number_id": PHONE }, "messages": [message(ts)] })
    };
    json!({
        "phoneID": PHONE, "from": USER, "message": message(ts), "name": "Ann",
        "raw": { "object": "whatsapp_business_account", "entry": [{ "id": "waba", "changes": [{ "field": field, "value": value }] }] },
    })
}

async fn thread(deps: &Deps) -> (Option<String>, Option<String>) {
    deps.db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT ci."threadControlState", m."contentAttributes" FROM "ContactInbox" ci
                   JOIN "Message" m ON m."contactInboxId" = ci."id" WHERE m."sourceId" = 'wamid.1'"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_standby_copy_is_stored_silently_then_promoted_once_by_its_owner_delivery() {
    let (deps, jobs) = setup().await;
    whatsapp_rows(&deps).await;
    let standby = json!({ "integrationType": "whatsapp", "integrationIdentifier": PHONE,
        "payload": { "kind": "standbyMessage", "body": delivery("standby", "1755700100") } });
    receive_event(&deps, &standby, &EventJob::default())
        .await
        .unwrap();
    let (state, attrs) = thread(&deps).await;
    assert_eq!(state.as_deref(), Some("standby"));
    let attrs: Value = serde_json::from_str(&attrs.unwrap()).unwrap();
    assert_eq!(attrs[tc::DELIVERY_KEY], tc::STANDBY_DELIVERY);
    assert!(attrs.get(tc::PROMOTED_KEY).is_none());

    let owner = json!({ "integrationType": "whatsapp", "integrationIdentifier": PHONE,
        "payload": delivery("messages", "1755700101") });
    let stored = crate::inbound::receive_message(&deps, &owner)
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored.message.is_some(),
        "the promoted copy owns the automation"
    );
    let (state, attrs) = thread(&deps).await;
    assert_eq!(state.as_deref(), Some("owned"));
    let attrs: Value = serde_json::from_str(&attrs.unwrap()).unwrap();
    assert_eq!(attrs[tc::PROMOTED_KEY], true);

    // A second owner redelivery is a plain duplicate.
    let again = crate::inbound::receive_message(&deps, &owner)
        .await
        .unwrap()
        .unwrap();
    assert!(again.message.is_none());
    assert!(
        jobs.0
            .lock()
            .unwrap()
            .iter()
            .all(|(_, name, _, _)| name != TAKE_BACK_JOB),
        "WhatsApp has no AI take-back"
    );
}

fn handover(ts: &str) -> Value {
    json!({ "integrationType": "whatsapp", "integrationIdentifier": PHONE, "payload": { "kind": "handover", "body": {
        "messaging_product": "whatsapp",
        "sender": { "phone_number": USER },
        "recipient": { "phone_number_id": PHONE },
        "type": "control_passed",
        "timestamp": ts,
        "control_passed": { "previous_owner_role": "META_AI", "new_owner_role": "PRIMARY" },
    }}})
}

fn resume_jobs(jobs: &crate::inbound::tests::Recording) -> Vec<String> {
    jobs.0
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, name, _, _)| name == crate::flow::SEND_FLOW)
        .map(|(_, _, data, opts)| {
            assert_eq!(data["data"]["flowId"], "10");
            assert_eq!(data["data"]["origin"], "channel");
            opts.job_id.clone().unwrap_or_default()
        })
        .collect()
}

#[tokio::test]
async fn a_hand_back_starts_the_resume_flow_once_and_again_on_our_own_retry() {
    let (deps, jobs) = setup().await;
    whatsapp_rows(&deps).await;
    let first = EventJob {
        is_retry: false,
        job_id: Some("wa-tc-phone-1-h".into()),
    };
    receive_event(&deps, &handover("1755700200"), &first)
        .await
        .unwrap();
    let ids = resume_jobs(&jobs);
    assert_eq!(ids.len(), 1);
    assert!(ids[0].starts_with("thread-resume-") && ids[0].ends_with("-wa-tc-phone-1-h"));

    // A Meta redelivery is applied idempotently but answers nothing.
    receive_event(&deps, &handover("1755700200"), &first)
        .await
        .unwrap();
    assert_eq!(resume_jobs(&jobs).len(), 1);
    // Our own retry (the earlier attempt may have died before the enqueue) does.
    let retry = EventJob {
        is_retry: true,
        ..first
    };
    receive_event(&deps, &handover("1755700200"), &retry)
        .await
        .unwrap();
    let ids = resume_jobs(&jobs);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1], "the same job id dedupes it in the queue");
}

#[test]
fn own_retry_matches_only_the_event_that_moved_the_row() {
    let event = HandoverEvent {
        event: "controlPassed",
        occurred_at: 5000,
        only_if_owned_by_app_id: Some("ai".into()),
        ..HandoverEvent::default()
    };
    let retry = EventJob {
        is_retry: true,
        job_id: None,
    };
    let own = |job: &EventJob, last, prev, at| own_retry_of(job, &event, last, prev, at);
    assert!(own(&retry, Some("controlPassed"), Some("ai"), Some(5000)));
    assert!(!own(
        &EventJob::default(),
        Some("controlPassed"),
        Some("ai"),
        Some(5000)
    ));
    assert!(!own(&retry, Some("controlPassed"), Some("ai"), Some(6000)));
    assert!(!own(
        &retry,
        Some("controlPassed"),
        Some("partner"),
        Some(5000)
    ));
    assert!(!own(&retry, Some("taken"), Some("ai"), Some(5000)));
}

#[test]
fn take_back_stage_takes_from_the_ai_and_replays_after_our_take() {
    assert_eq!(
        stage_of("standby", Some("ai_agent"), None, None, "ai"),
        Some("take")
    );
    assert_eq!(
        stage_of("owned", None, Some("taken"), Some("ai"), "ai"),
        Some("replay")
    );
    assert_eq!(
        stage_of("owned", None, Some("taken"), Some("partner"), "ai"),
        None
    );
    assert_eq!(
        stage_of("standby", Some("secondary"), None, None, "ai"),
        None
    );
    assert_eq!(stage_of("idle", None, None, None, "ai"), None);
}

#[test]
fn template_echoes_render_the_definition_with_the_sent_parameters() {
    let definition = json!({ "components": [
        { "type": "HEADER", "format": "TEXT", "text": "Hi {{1}}" },
        { "type": "BODY", "text": "Order {{order}} ships {{2}}." },
    ]});
    let sent = json!({ "name": "ship", "components": [
        { "type": "header", "parameters": [{ "type": "text", "text": "Ann" }] },
        { "type": "body", "parameters": [
            { "type": "text", "text": "x", "parameter_name": "order" },
            { "type": "text", "text": "today" },
        ] },
    ]});
    assert_eq!(
        render_template_echo(Some(&definition), Some(&sent)),
        "Hi Ann\nOrder x ships today."
    );
    let media = json!({ "components": [{ "type": "HEADER", "format": "IMAGE" }, { "type": "BODY", "text": "Look" }] });
    assert_eq!(render_template_echo(Some(&media), None), "[image]\nLook");
    assert_eq!(render_template_echo(None, Some(&sent)), "[template] ship");

    let echo = json!({ "id": "wamid.e", "message": { "to": USER, "type": "template", "template": sent }, "template": definition });
    let received = parse_standby_echo(&echo, json!({ "delivery": "standby" }));
    let message = received.message.unwrap();
    assert_eq!(message.message_type, "outgoing");
    assert_eq!(
        message.text.as_deref(),
        Some("Hi Ann\nOrder x ships today.")
    );
    assert_eq!(received.contact.source_id, USER);
    assert_eq!(received.echo_origin.as_deref(), Some("thirdParty"));
    assert_eq!(
        received.thread_control.unwrap()["signalsOwnership"],
        false,
        "a template echo says nothing about ownership"
    );
}
