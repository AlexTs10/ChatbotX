//! The trigger worker on the fixture DB (workspace 2, contact 3, contact
//! inbox 5, conversation 6, flow 10). Ports the DB-observable cases of
//! `trigger-action-executor-*.test.ts`, `trigger-action-scope*.test.ts`,
//! `trigger-executor-*.test.ts`, `action-executor-contact-inbox-attribution`
//! and `trigger-worker-contact-inbox-propagation`; call-count cases on
//! mocks are covered by the memoized `Scope` and listed n-a in the spec.

use serde_json::{Value, json};

use super::*;
use crate::steps::test_support::{Fixture, job};

async fn fixture() -> Fixture {
    let f = Fixture::new().await;
    f.exec(
        r#"INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (40, 'VIP', 2), (41, 'Hot', 2);
           INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (50, 'Welcome', 2);
           INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (20, 'plan', 'shortText', 2);"#,
    )
    .await;
    f
}

/// Trigger 70 on `tagApplied` of tag 40 with `actions`.
async fn trigger(f: &Fixture, actions: Value) {
    let actions = actions.to_string();
    f.sql(move |tx| {
        tx.execute(
            r#"INSERT INTO "Trigger" ("id", "name", "actions", "workspaceId") VALUES (70, 'T', ?1, 2)"#,
            [actions],
        )?;
        tx.execute(r#"INSERT INTO "Condition" ("id", "triggerId", "type", "sourceId") VALUES (71, 70, 'tagApplied', '40')"#, [])?;
        Ok(())
    })
    .await;
}

fn event(tag: &str, extra: Value) -> Value {
    let mut data = json!({ "workspaceId": "2", "contactId": "3", "eventType": "tagApplied", "eventData": { "tagId": tag, "sourceId": tag }, "timestamp": "2026-11-17T00:00:00.000Z" });
    for (k, v) in extra.as_object().unwrap() {
        data[k] = v.clone();
    }
    data
}

async fn act(
    f: &Fixture,
    action: Value,
    contact_inbox_id: Option<i64>,
    decision: Option<&str>,
) -> Result<Outcome, JobError> {
    let input = RunInput {
        contact_inbox_id,
        decision_flow_id: decision,
        occurrence: Some("1:1"),
        channel_originated: false,
    };
    let ctx = ActionCtx {
        deps: &f.deps,
        action: &action,
        trigger_id: 70,
        workspace_id: 2,
        contact_id: 3,
        input: &input,
        occurrence_key: None,
    };
    execute_action(&ctx, &mut Scope::default()).await
}

#[tokio::test]
async fn a_matching_event_runs_the_actions_and_records_history_and_stats() {
    let f = fixture().await;
    trigger(&f, json!([{ "type": "addTag", "tagIds": ["41"] }, { "type": "addSequence", "sequenceIds": ["50"] }])).await;
    let ran = evaluate(&f.deps, &event("40", json!({})), Some("9:9"))
        .await
        .unwrap();
    assert_eq!(ran, vec![70]);
    assert_eq!(
        f.int(r#"SELECT COUNT(*) FROM "ContactToTag" WHERE "contactId" = 3 AND "tagId" = 41"#)
            .await,
        Some(1)
    );
    assert_eq!(
        f.int(
            r#"SELECT COUNT(*) FROM "ContactOnSequence" WHERE "contactId" = 3 AND "sequenceId" = 50"#
        )
        .await,
        Some(1)
    );
    assert_eq!(f.int(r#"SELECT COUNT(*) FROM "TriggerContactHistory" WHERE "triggerId" = 70 AND "contactId" = 3"#).await, Some(1));
    assert_eq!(
        f.text(r#"SELECT "totalContacts" || '|' || "totalExecutions" || '|' || "successCount" || '|' || "failureCount" FROM "TriggerStat" WHERE "triggerId" = 70"#).await.as_deref(),
        Some("1|1|1|0")
    );
    // another tag, an inactive trigger, a worker-emitted event: nothing runs
    assert!(
        evaluate(&f.deps, &event("41", json!({})), None)
            .await
            .unwrap()
            .is_empty()
    );
    let worker = job(
        json!({ "type": "evaluateTriggers", "data": event("40", json!({ "source": "worker" })) }),
    );
    handle(f.deps.clone(), worker).await.unwrap();
    f.exec(r#"UPDATE "Trigger" SET "active" = 0"#).await;
    assert!(
        evaluate(&f.deps, &event("40", json!({})), None)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.int(r#"SELECT "totalExecutions" FROM "TriggerStat""#)
            .await,
        Some(1)
    );
}

#[tokio::test]
async fn the_worker_skips_blocked_workspaces_and_rejects_unknown_job_types() {
    let f = fixture().await;
    trigger(&f, json!([{ "type": "addTag", "tagIds": ["41"] }])).await;
    let unknown = handle(
        f.deps.clone(),
        job(json!({ "type": "other", "data": event("40", json!({})) })),
    )
    .await;
    assert!(unknown.is_err());
    f.exec(r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = 2"#)
        .await;
    handle(
        f.deps.clone(),
        job(json!({ "type": "evaluateTriggers", "data": event("40", json!({})) })),
    )
    .await
    .unwrap();
    assert_eq!(
        f.int(r#"SELECT COUNT(*) FROM "ContactToTag" WHERE "tagId" = 41"#)
            .await,
        Some(0)
    );
}

#[test]
fn skips_the_whole_run_once_the_per_contact_limit_is_exceeded() {
    let now = 1_000_000;
    for _ in 0..RUN_LIMIT {
        assert!(try_acquire_contact_run(-1, -1, now));
    }
    assert!(!try_acquire_contact_run(-1, -1, now));
    // another trigger, and the next window, run again
    assert!(try_acquire_contact_run(-2, -1, now));
    assert!(try_acquire_contact_run(-1, -1, now + RUN_WINDOW_MS));
}

#[tokio::test]
async fn a_contact_without_a_conversation_is_skipped_and_unknown_actions_are_skipped() {
    let f = fixture().await;
    assert_eq!(
        act(&f, json!({ "type": "nope" }), None, None)
            .await
            .unwrap(),
        Outcome::Skipped("unknownAction".into())
    );
    assert_eq!(
        act(&f, json!({ "type": "sendMetaCapiEvent" }), None, None)
            .await
            .unwrap(),
        Outcome::Skipped("unsupportedChannel".into())
    );
    f.exec(r#"DELETE FROM "Conversation""#).await;
    assert_eq!(
        act(
            &f,
            json!({ "type": "addTag", "tagIds": ["41"] }),
            None,
            None
        )
        .await
        .unwrap(),
        Outcome::Skipped("noConversation".into())
    );
}

#[tokio::test]
async fn sequence_actions_use_only_existing_sequences() {
    let f = fixture().await;
    assert_eq!(
        act(
            &f,
            json!({ "type": "addSequence", "sequenceIds": ["999"] }),
            None,
            None
        )
        .await
        .unwrap(),
        Outcome::Skipped("sequenceNotFound".into())
    );
    assert_eq!(
        act(
            &f,
            json!({ "type": "addSequence", "sequenceIds": [] }),
            None,
            None
        )
        .await
        .unwrap(),
        Outcome::Skipped("invalidAction".into())
    );
    act(
        &f,
        json!({ "type": "addSequence", "sequenceIds": ["50", "999"] }),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        f.int(r#"SELECT COUNT(*) FROM "ContactOnSequence" WHERE "sequenceId" = 50"#)
            .await,
        Some(1)
    );
    act(
        &f,
        json!({ "type": "removeSequence", "sequenceIds": ["50"] }),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        f.int(r#"SELECT COUNT(*) FROM "ContactOnSequence" WHERE "sequenceId" = 50"#)
            .await,
        Some(0)
    );
}

#[tokio::test]
async fn custom_field_actions_set_and_clear_the_value() {
    let f = fixture().await;
    let set = act(&f, json!({ "type": "setCustomField", "customFieldId": "20", "operation": "set", "value": "Pro" }), None, None).await.unwrap();
    assert_eq!(set, Outcome::Succeeded);
    assert_eq!(f.text(r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = 3 AND "customFieldId" = 20"#).await.as_deref(), Some("Pro"));
    act(
        &f,
        json!({ "type": "clearCustomField", "customFieldId": "20" }),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        f.int(r#"SELECT COUNT(*) FROM "ContactCustomField" WHERE "customFieldId" = 20"#)
            .await,
        Some(0)
    );
    assert_eq!(
        act(&f, json!({ "type": "setCustomField" }), None, None)
            .await
            .unwrap(),
        Outcome::Skipped("invalidAction".into())
    );
}

#[tokio::test]
async fn conversation_actions_change_the_latest_conversation() {
    let f = fixture().await;
    f.exec(
        r#"INSERT INTO "WorkspaceMember" ("workspaceId", "userId", "role") VALUES (2, 1, 'owner')"#,
    )
    .await;
    act(&f, json!({ "type": "archiveConversation" }), None, None)
        .await
        .unwrap();
    assert_eq!(
        f.int(r#"SELECT "archivedAt" IS NOT NULL FROM "Conversation" WHERE "id" = 6"#)
            .await,
        Some(1)
    );
    act(
        &f,
        json!({ "type": "assignConversation", "assignedId": "u_1" }),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        f.int(r#"SELECT "assignedUserId" FROM "Conversation" WHERE "id" = 6"#)
            .await,
        Some(1)
    );
    assert_eq!(f.jobs.named("notifyConversationAssigned").len(), 1);
    act(&f, json!({ "type": "unassignConversation" }), None, None)
        .await
        .unwrap();
    assert_eq!(
        f.int(r#"SELECT "assignedUserId" IS NULL FROM "Conversation" WHERE "id" = 6"#)
            .await,
        Some(1)
    );
    act(
        &f,
        json!({ "type": "transferConversationToHuman", "notifyAdmins": true }),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        f.int(r#"SELECT "botEnabled" FROM "Conversation" WHERE "id" = 6"#)
            .await,
        Some(0)
    );
    act(&f, json!({ "type": "enableBot" }), None, None)
        .await
        .unwrap();
    assert_eq!(
        f.int(r#"SELECT "botEnabled" FROM "Conversation" WHERE "id" = 6"#)
            .await,
        Some(1)
    );
}

#[tokio::test]
async fn flow_actions_enqueue_send_flow_on_the_threaded_inbox() {
    let f = fixture().await;
    f.node(json!([]), json!([])).await;
    f.exec(
        r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId", "lastMessageAt")
           VALUES (8, 3, 3, 4, 'webchat', 'webchat', 's2', 9999999999999)"#,
    )
    .await;
    // the threaded inbox wins over the most recent one
    act(
        &f,
        json!({ "type": "startAnotherFlow", "flowId": "10" }),
        Some(5),
        None,
    )
    .await
    .unwrap();
    // without a thread: the most recently active inbox
    act(
        &f,
        json!({ "type": "startExternalStep", "flowId": "10", "nodeId": "n2" }),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        f.jobs.send_flows(),
        vec![
            json!({ "conversationId": "6", "contactInboxId": "5", "flowId": "10" }),
            json!({ "conversationId": "6", "contactInboxId": "8", "flowId": "10", "nodeId": "n2" }),
        ]
    );
    assert!(
        f.jobs
            .named("sendFlow")
            .iter()
            .all(|j| j.opts.job_id.is_none())
    );
    // skips: invalid ids, missing node, inactive flow, Decision self-loop
    let skip = |r: Result<Outcome, JobError>| match r.unwrap() {
        Outcome::Skipped(reason) => reason,
        Outcome::Succeeded => "succeeded".into(),
    };
    assert_eq!(
        skip(
            act(
                &f,
                json!({ "type": "startExternalStep", "flowId": "abc", "nodeId": "n2" }),
                None,
                None
            )
            .await
        ),
        "invalidAction"
    );
    assert_eq!(
        skip(
            act(
                &f,
                json!({ "type": "startExternalStep", "flowId": "10", "nodeId": "" }),
                None,
                None
            )
            .await
        ),
        "invalidAction"
    );
    assert_eq!(
        skip(
            act(
                &f,
                json!({ "type": "startExternalStep", "flowId": "10", "nodeId": "n9" }),
                None,
                None
            )
            .await
        ),
        "nodeNotFound"
    );
    assert_eq!(
        skip(
            act(
                &f,
                json!({ "type": "startAnotherFlow", "flowId": "10" }),
                None,
                Some("10")
            )
            .await
        ),
        "succeeded"
    );
    f.exec(r#"UPDATE "Flow" SET "active" = 0"#).await;
    assert_eq!(
        skip(
            act(
                &f,
                json!({ "type": "startAnotherFlow", "flowId": "10" }),
                None,
                None
            )
            .await
        ),
        "flowInactive"
    );
    assert_eq!(
        skip(
            act(
                &f,
                json!({ "type": "startExternalStep", "flowId": "10", "nodeId": "n2" }),
                None,
                None
            )
            .await
        ),
        "flowInactive"
    );
    assert_eq!(f.jobs.send_flows().len(), 2);
    // no inbox at all
    f.exec(r#"DELETE FROM "ContactInbox""#).await;
    assert_eq!(
        skip(
            act(
                &f,
                json!({ "type": "startAnotherFlow", "flowId": "10" }),
                None,
                None
            )
            .await
        ),
        "noContactInbox"
    );
}

#[tokio::test]
async fn the_threaded_contact_inbox_id_is_read_from_the_event() {
    let f = fixture().await;
    f.node(json!([]), json!([])).await;
    trigger(&f, json!([{ "type": "startAnotherFlow", "flowId": "10" }])).await;
    let mut data = event("40", json!({ "channelOriginated": true }));
    data["eventData"]["contactInboxId"] = json!("5");
    evaluate(&f.deps, &data, None).await.unwrap();
    assert_eq!(f.jobs.send_flows()[0]["contactInboxId"], json!("5"));
    assert_eq!(f.jobs.send_flows()[0]["origin"], json!("channel"));
}

#[tokio::test]
async fn a_pipeline_action_without_a_pipeline_is_reported_not_raised() {
    let f = fixture().await;
    let outcome = act(
        &f,
        json!({ "type": "pipeline", "action": "addToPipeline" }),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(matches!(outcome, Outcome::Skipped(_)), "{outcome:?}");
}

#[tokio::test]
async fn send_meta_capi_event_enqueues_one_event_on_a_meta_inbox() {
    let mut f = fixture().await;
    f.on_channel("messenger").await;
    // Purchase without value / currency is an invalid action: Error Log, skip.
    assert_eq!(
        act(
            &f,
            json!({ "type": "sendMetaCapiEvent", "eventName": "Purchase" }),
            Some(5),
            None
        )
        .await
        .unwrap(),
        Outcome::Skipped("invalidAction".into())
    );
    assert!(f.jobs.named("sendMetaCapiEvent").is_empty());
    assert_eq!(
        act(
            &f,
            json!({ "type": "sendMetaCapiEvent", "eventName": "Purchase", "value": "35", "currency": "usd" }),
            Some(5),
            None
        )
        .await
        .unwrap(),
        Outcome::Succeeded
    );
    let jobs = f.jobs.named("sendMetaCapiEvent");
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        f.text(r#"SELECT "source" || ':' || "eventName" || ':' || "currency" || ':' || "value" FROM "MetaCapiEvent""#)
            .await
            .as_deref(),
        Some("triggerAction:Purchase:USD:35")
    );
    assert!(
        f.text(r#"SELECT "sourceKey" FROM "MetaCapiEvent""#)
            .await
            .unwrap()
            .starts_with("trigger:70:5:")
    );
}
