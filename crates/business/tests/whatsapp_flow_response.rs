//! Q.P7-V4: ports `packages/business/__tests__/whatsapp-flow-response-service.test.ts`
//! and `resolve-sent-whatsapp-flow.test.ts`. The mocked repositories become
//! real rows: the stored `ContactCustomField` values (after `setValues`'
//! own normalization), the `WhatsappFlow.completedCount`, the
//! `ContactActivity` row, broadcasts and flow versions. The workspace zone is
//! `Asia/Ho_Chi_Minh`; the contact has none.

mod common;

use chatbotx_business::whatsapp_flow_response::{
    ApplyInput, ResolvedButton, SentFlow, apply_response, resolve_sent_whatsapp_flow,
    resolve_template_flow_button, serialize_flow_value,
};
use chatbotx_core::flow_config::steps::{decode_template_flow_token, encode_template_flow_token};
use chatbotx_db::rusqlite::params;
use common::{TestDb, WS, custom_field, custom_field_values, setup, two_workspaces};
use serde_json::{Map, Value, json};

const CONTACT: i64 = 10;
const INBOX: i64 = 50;
const INBOX_B: i64 = 51;
const INTEGRATION: i64 = 60;
const FLOW_ROW: i64 = 70;

async fn fixture() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "timezone" = 'Asia/Ho_Chi_Minh' WHERE "id" = ?1"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'Ada', ?2)"#,
            params![CONTACT, WS],
        )?;
        for id in [INBOX, INBOX_B] {
            common::inbox(tx, id, WS, "whatsapp")?;
        }
        tx.execute(
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
               VALUES (?1, '{}', 'p-1', 'w-1', 'b-1', 'Wa', ?2, ?3)"#,
            params![INTEGRATION, WS, INBOX],
        )?;
        tx.execute(
            r#"INSERT INTO "WhatsappFlow" ("id", "name", "integrationWhatsappId", "sourceId", "status") VALUES (?1, 'Form', ?2, 'meta-flow-1', 'PUBLISHED')"#,
            params![FLOW_ROW, INTEGRATION],
        )?;
        for (id, name, kind) in [
            (31, "name", "shortText"),
            (32, "count", "shortText"),
            (33, "day", "date"),
            (34, "at", "datetime"),
            (35, "docs", "shortText"),
            (36, "amount", "number"),
        ] {
            custom_field(tx, id, WS, name, kind)?;
        }
        Ok(())
    })
    .await;
    db
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn input(mappings: Value, response: Value, files: Option<Value>) -> ApplyInput {
    ApplyInput {
        workspace_id: WS,
        contact_id: CONTACT,
        inbox_id: None,
        integration_whatsapp_id: Some(INTEGRATION),
        flow_source_id: "meta-flow-1".into(),
        field_mappings: mappings.as_array().cloned().unwrap_or_default(),
        flow_response: object(response),
        flow_files: files.map(object),
    }
}

fn values(rows: &[(i64, &str)]) -> Vec<(i64, String)> {
    rows.iter().map(|(id, v)| (*id, (*v).to_owned())).collect()
}

async fn completed(db: &TestDb) -> (i64, i64) {
    db.read(|conn| {
        Ok((
            conn.query_row(
                r#"SELECT "completedCount" FROM "WhatsappFlow" WHERE "id" = ?1"#,
                [FLOW_ROW],
                |r| r.get(0),
            )?,
            conn.query_row(
                r#"SELECT COUNT(*) FROM "ContactActivity" WHERE "activityType" = 'formCompleted'
                   AND "referenceType" = 'whatsappFlow' AND "referenceId" = ?1 AND "contactId" = ?2"#,
                params![FLOW_ROW, CONTACT],
                |r| r.get(0),
            )?,
        ))
    })
    .await
    .unwrap()
}

#[test]
fn serialize_flow_value_coerces_answers_for_custom_field_storage() {
    let s = |v: Value| serialize_flow_value(Some(&v));
    assert_eq!(s(json!("Alice")).as_deref(), Some("Alice"));
    assert_eq!(s(json!(42)).as_deref(), Some("42"));
    assert_eq!(s(json!(true)).as_deref(), Some("true"));
    assert_eq!(
        s(json!({ "nested": "value" })).as_deref(),
        Some(r#"{"nested":"value"}"#)
    );
    assert_eq!(
        s(json!({ "start-date": "2024-10-21", "end-date": "2024-10-25" })).as_deref(),
        Some("2024-10-21 - 2024-10-25")
    );
    assert_eq!(s(Value::Null), None);
    assert_eq!(serialize_flow_value(None), None);
}

#[tokio::test]
async fn apply_response_counts_the_completion_and_writes_mapped_fields() {
    let db = fixture().await;
    apply_response(
        &db,
        input(
            json!([
                { "paramKey": "name", "paramLabel": "Name", "customFieldId": "31" },
                { "paramKey": "ignored", "customFieldId": null },
                { "paramKey": "count", "customFieldId": "32" },
            ]),
            json!({ "name": "Alice", "count": 3 }),
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        custom_field_values(&db, CONTACT).await,
        values(&[(31, "Alice"), (32, "3")])
    );
    // The activity is idempotent per contact; the count is per submission.
    assert_eq!(completed(&db).await, (1, 1));
    apply_response(&db, input(json!([]), json!({}), None))
        .await
        .unwrap();
    assert_eq!(completed(&db).await, (2, 1));
}

fn file(id: &str) -> Value {
    json!({ "mediaId": id, "fileName": format!("{id}.pdf"), "mimeType": "application/pdf",
            "size": 10, "url": format!("https://cdn.example.com/{id}.pdf") })
}

/// `(mappings, response, files, stored values)`.
type FileCase = (Value, Value, Option<Value>, Vec<(i64, String)>);

#[tokio::test]
async fn file_answers_store_kept_urls_and_never_the_raw_answer() {
    let cases: Vec<FileCase> = vec![
        // Every URL, comma-separated, by default; `firstUrl` keeps one.
        (
            json!([{ "paramKey": "docs", "customFieldId": "35" }]),
            json!({ "docs": [{ "id": "1" }, { "id": "2" }] }),
            Some(json!({ "docs": [file("1"), file("2")] })),
            values(&[(
                35,
                "https://cdn.example.com/1.pdf, https://cdn.example.com/2.pdf",
            )]),
        ),
        (
            json!([{ "paramKey": "docs", "customFieldId": "35", "fileFormat": "firstUrl" }]),
            json!({}),
            Some(json!({ "docs": [file("1"), file("2")] })),
            values(&[(35, "https://cdn.example.com/1.pdf")]),
        ),
        // No file kept, or a file answer left empty: the field stays.
        (
            json!([{ "paramKey": "docs", "customFieldId": "35" }]),
            json!({ "docs": [{ "id": "1", "file_name": "a.pdf" }] }),
            Some(json!({ "docs": [] })),
            vec![],
        ),
        (
            json!([{ "paramKey": "docs", "customFieldId": "35", "fileFormat": "allUrls" }]),
            json!({ "docs": [] }),
            None,
            vec![],
        ),
        // File and text answers together; a key named like an inherited
        // member reads its own answer.
        (
            json!([{ "paramKey": "name", "customFieldId": "31" }, { "paramKey": "docs", "customFieldId": "35" }]),
            json!({ "name": "Alice", "docs": [{ "id": "1" }] }),
            Some(json!({ "docs": [file("1")] })),
            values(&[(31, "Alice"), (35, "https://cdn.example.com/1.pdf")]),
        ),
        (
            json!([{ "paramKey": "constructor", "customFieldId": "31" }]),
            json!({ "constructor": "Alice", "docs": [{ "id": "1" }] }),
            Some(json!({ "docs": [file("1")] })),
            values(&[(31, "Alice")]),
        ),
        // File links mapped to a non-text field are skipped.
        (
            json!([{ "paramKey": "name", "customFieldId": "31" }, { "paramKey": "docs", "customFieldId": "36" }]),
            json!({ "name": "Alice", "docs": [{ "id": "1" }] }),
            Some(json!({ "docs": [file("1")] })),
            values(&[(31, "Alice")]),
        ),
        // Ingested without files: serialized as before.
        (
            json!([{ "paramKey": "docs", "customFieldId": "35" }]),
            json!({ "docs": [{ "id": "1", "file_name": "a.pdf" }] }),
            None,
            // serde_json sorts the keys (no `preserve_order`).
            values(&[(35, r#"[{"file_name":"a.pdf","id":"1"}]"#)]),
        ),
    ];
    for (mappings, response, files, expected) in cases {
        let db = fixture().await;
        apply_response(&db, input(mappings.clone(), response, files))
            .await
            .unwrap();
        assert_eq!(
            custom_field_values(&db, CONTACT).await,
            expected,
            "{mappings}"
        );
    }
}

async fn save_answers(answers: Value) -> Vec<(i64, String)> {
    let db = fixture().await;
    let mappings: Vec<Value> = answers
        .as_object()
        .unwrap()
        .keys()
        .map(|k| json!({ "paramKey": k, "customFieldId": k }))
        .collect();
    apply_response(&db, input(Value::Array(mappings), answers, None))
        .await
        .unwrap();
    custom_field_values(&db, CONTACT).await
}

/// 2024-03-20T18:07:00Z, already 2024-03-21 in Ho Chi Minh City.
const LATE_EVENING_UTC_MS: &str = "1710958020000";

#[tokio::test]
async fn date_answers_take_the_shape_of_a_date_field() {
    // A legacy millisecond timestamp: the workspace's day in a date field
    // (stored by `setValues` as that day's local midnight), the instant in a
    // datetime field.
    assert_eq!(
        save_answers(json!({ "33": LATE_EVENING_UTC_MS, "34": LATE_EVENING_UTC_MS })).await,
        values(&[
            (33, "2024-03-21T00:00:00+07:00"),
            (34, "2024-03-20T18:07:00.000Z")
        ])
    );
    // Older timestamps (before 2001, before 1970) are dates too.
    assert_eq!(
        save_answers(json!({ "33": "631152000000", "34": "-315619200000" })).await,
        values(&[
            (33, "1990-01-01T00:00:00+07:00"),
            (34, "1960-01-01T00:00:00.000Z")
        ])
    );
    for (ms, expected) in [
        ("0", "1970-01-01T00:00:00.000Z"),
        ("86400000", "1970-01-02T00:00:00.000Z"),
        ("-86400000", "1969-12-31T00:00:00.000Z"),
    ] {
        assert_eq!(
            save_answers(json!({ "34": ms })).await,
            values(&[(34, expected)])
        );
    }
    // A calendar date is kept in a date field and local midnight (the
    // workspace zone, +07:00) in a datetime field.
    assert_eq!(
        save_answers(json!({ "34": "2024-10-21", "33": "2024-10-21" })).await,
        values(&[
            (33, "2024-10-21T00:00:00+07:00"),
            (34, "2024-10-20T17:00:00.000Z")
        ])
    );
    // A number in a number field and a date-like text answer are untouched;
    // a CalendarPicker range is `start - end`.
    assert_eq!(
        save_answers(json!({ "36": "5", "31": LATE_EVENING_UTC_MS, "35": { "start-date": "2024-10-21", "end-date": "2024-10-25" } })).await,
        values(&[(31, LATE_EVENING_UTC_MS), (35, "2024-10-21 - 2024-10-25"), (36, "5")])
    );
}

#[tokio::test]
async fn a_number_past_javascripts_date_range_is_left_for_the_field_to_refuse() {
    let db = fixture().await;
    let too_big = "9".repeat(17);
    let result = apply_response(
        &db,
        input(
            json!([{ "paramKey": "d", "customFieldId": "33" }]),
            json!({ "d": too_big }),
            None,
        ),
    )
    .await;
    // `setValues` refuses it for the date field (nothing stored).
    assert!(result.is_err() || custom_field_values(&db, CONTACT).await.is_empty());
}

#[tokio::test]
async fn the_integration_is_looked_up_by_inbox_and_nothing_is_recorded_without_one() {
    let db = fixture().await;
    let by_inbox = ApplyInput {
        integration_whatsapp_id: None,
        inbox_id: Some(INBOX),
        ..input(json!([]), json!({}), None)
    };
    apply_response(&db, by_inbox).await.unwrap();
    assert_eq!(completed(&db).await, (1, 1));
    let unknown = ApplyInput {
        integration_whatsapp_id: None,
        inbox_id: Some(INBOX_B),
        ..input(
            json!([{ "paramKey": "name", "customFieldId": "31" }]),
            json!({ "name": "A" }),
            None,
        )
    };
    apply_response(&db, unknown).await.unwrap();
    assert_eq!(completed(&db).await, (1, 1));
    assert!(custom_field_values(&db, CONTACT).await.is_empty());
}

#[tokio::test]
async fn a_flow_row_that_no_longer_exists_records_nothing_but_saves_fields() {
    let db = fixture().await;
    let gone = ApplyInput {
        flow_source_id: "meta-flow-gone".into(),
        ..input(
            json!([{ "paramKey": "name", "customFieldId": "31" }]),
            json!({ "name": "A" }),
            None,
        )
    };
    apply_response(&db, gone).await.unwrap();
    assert_eq!(completed(&db).await, (0, 0));
    assert_eq!(
        custom_field_values(&db, CONTACT).await,
        values(&[(31, "A")])
    );
}

// ---- resolveTemplateFlowButton / resolveSentWhatsappFlow ------------------

const BROADCAST: i64 = 11_612_473_309_626_368;
const FLOW: i64 = 11_612_473_309_626_368;
const VERSION: i64 = 11_612_473_309_626_369;
const STEP: &str = "11612473309626370";
const BUTTON: &str = "11612473309626371";

fn flow_param(source: &str) -> Value {
    json!({ "sub_type": "flow", "index": 0, "flowSourceId": source,
            "fieldMappings": [{ "paramKey": "email", "customFieldId": "cf-email" }] })
}

async fn broadcast(db: &TestDb, integration: Option<i64>, data: Value, targets: &[(i64, Value)]) {
    let targets: Vec<(i64, String)> = targets.iter().map(|(i, d)| (*i, d.to_string())).collect();
    let mode = if targets.is_empty() {
        "channel"
    } else {
        "targets"
    };
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "integrationWhatsappId", "templateId", "templateData", "targetMode", "status", "schedulesType", "schedulesAt", "subaction", "channel")
               VALUES (?1, 'B', ?2, ?3, 1, ?4, ?5, 'sent', 'now', 0, 'sendTemplate', 'whatsapp')"#,
            params![BROADCAST, WS, integration, data.to_string(), mode],
        )?;
        for (inbox, data) in targets {
            tx.execute(
                r#"INSERT INTO "BroadcastTarget" ("broadcastId", "inboxId", "templateId", "templateData") VALUES (?1, ?2, 1, ?3)"#,
                params![BROADCAST, inbox, data],
            )?;
        }
        Ok(())
    })
    .await;
}

/// Flow `FLOW` (active) with version `VERSION` holding `steps`; `current`
/// sets it as the flow's current version.
async fn flow_version(db: &TestDb, steps: Value, current: bool) {
    let nodes = json!([{ "data": { "details": { "steps": steps } } }]).to_string();
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name", "active") VALUES (?1, ?2, 'F', 1)"#,
            params![FLOW, WS],
        )?;
        tx.execute(
            r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "startNodeId") VALUES (?1, ?2, ?3, ?4, '[]', 0, 1)"#,
            params![VERSION, WS, FLOW, nodes],
        )?;
        if current {
            tx.execute(
                r#"UPDATE "Flow" SET "currentVersionId" = ?1 WHERE "id" = ?2"#,
                params![VERSION, FLOW],
            )?;
        }
        Ok(())
    })
    .await;
}

fn broadcast_token(card: Option<u64>) -> String {
    let mut token =
        json!({ "origin": "b", "broadcastId": BROADCAST.to_string(), "buttonIndex": 0 });
    if let Some(card) = card {
        token["cardIndex"] = json!(card);
    }
    encode_template_flow_token(&token)
}

fn step_token() -> String {
    encode_template_flow_token(&json!({ "origin": "s", "flowId": FLOW.to_string(),
        "flowVersionId": VERSION.to_string(), "stepId": STEP, "buttonIndex": 0 }))
}

async fn resolve(db: &TestDb, token: String) -> Option<ResolvedButton> {
    let token = decode_template_flow_token(Some(&token)).expect("a template token");
    chatbotx_business::tx::read(db, move |conn| {
        resolve_template_flow_button(conn, &token, WS, INBOX)
    })
    .await
    .unwrap()
}

async fn sent(db: &TestDb, token: Option<&str>) -> Option<SentFlow> {
    let token = token.map(str::to_owned);
    chatbotx_business::tx::read(db, move |conn| {
        resolve_sent_whatsapp_flow(conn, WS, INBOX, token.as_deref())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn resolves_a_broadcasts_top_level_and_carousel_flow_buttons() {
    let db = fixture().await;
    let param = flow_param("wa-flow-1");
    broadcast(
        &db,
        Some(INTEGRATION),
        json!({ "button": [param], "carousel": [
            { "card_index": 0, "button": [flow_param("wrong-flow")] },
            { "card_index": 2, "button": [flow_param("wa-flow-carousel")] },
        ] }),
        &[],
    )
    .await;
    assert_eq!(
        resolve(&db, broadcast_token(None)).await,
        Some(ResolvedButton {
            integration_whatsapp_id: Some(INTEGRATION),
            flow_source_id: "wa-flow-1".into(),
            param,
        })
    );
    assert_eq!(
        resolve(&db, broadcast_token(Some(2)))
            .await
            .unwrap()
            .flow_source_id,
        "wa-flow-carousel"
    );
    assert_eq!(
        sent(&db, Some(&broadcast_token(None))).await,
        Some(SentFlow {
            flow_source_id: "wa-flow-1".into(),
            integration_whatsapp_id: Some(INTEGRATION),
        })
    );
}

#[tokio::test]
async fn keeps_a_multi_page_broadcasts_null_integration_and_its_page_params() {
    let db = fixture().await;
    broadcast(
        &db,
        None,
        Value::Null,
        &[
            (INBOX, json!({ "button": [flow_param("wa-flow-page-b")] })),
            (INBOX_B, json!({ "button": [flow_param("other-page")] })),
        ],
    )
    .await;
    let resolved = resolve(&db, broadcast_token(None)).await.unwrap();
    assert_eq!(resolved.integration_whatsapp_id, None);
    assert_eq!(resolved.flow_source_id, "wa-flow-page-b");
}

#[tokio::test]
async fn returns_none_for_a_missing_broadcast_or_a_non_flow_button() {
    let db = fixture().await;
    assert_eq!(resolve(&db, broadcast_token(None)).await, None);
    broadcast(
        &db,
        Some(INTEGRATION),
        json!({ "button": [{ "sub_type": "quick_reply", "index": 0, "payload": "payload" }] }),
        &[],
    )
    .await;
    assert_eq!(resolve(&db, broadcast_token(None)).await, None);
}

#[tokio::test]
async fn resolves_a_flow_steps_template_flow_button_from_the_pinned_version() {
    let db = fixture().await;
    assert_eq!(resolve(&db, step_token()).await, None, "no version yet");
    let param = flow_param("wa-flow-step");
    flow_version(
        &db,
        json!([{ "id": STEP, "stepType": "sendWaTemplateMessage", "template": { "params": { "button": [param] } } }]),
        false,
    )
    .await;
    assert_eq!(
        resolve(&db, step_token()).await,
        Some(ResolvedButton {
            integration_whatsapp_id: None,
            flow_source_id: "wa-flow-step".into(),
            param,
        })
    );
}

fn whatsapp_flow_step(flow_id: Value, source: &str) -> Value {
    json!({ "id": "step-1", "stepType": "whatsappFlow", "text": "body",
            "buttons": [{ "id": BUTTON, "label": "Go", "steps": [] }], "inboxId": null,
            "flow": { "id": flow_id, "sourceId": source, "startScreenId": "WELCOME", "fieldMappings": [] } })
}

#[tokio::test]
async fn resolves_a_send_whatsapp_flow_step_by_its_button_preferring_flow_id() {
    let db = fixture().await;
    let token = format!("{FLOW}:{VERSION}:{BUTTON}");
    flow_version(
        &db,
        json!([whatsapp_flow_step(
            json!(FLOW_ROW.to_string()),
            "stale-source"
        )]),
        false,
    )
    .await;
    assert_eq!(
        sent(&db, Some(&token)).await,
        Some(SentFlow {
            flow_source_id: "meta-flow-1".into(),
            integration_whatsapp_id: None
        })
    );
    // The Flow row is gone: the step's stored sourceId.
    setup(&db, |tx| {
        tx.execute(r#"DELETE FROM "WhatsappFlow""#, [])?;
        Ok(())
    })
    .await;
    assert_eq!(
        sent(&db, Some(&token)).await.unwrap().flow_source_id,
        "stale-source"
    );
}

#[tokio::test]
async fn uses_the_flows_current_version_when_the_token_pins_none() {
    let db = fixture().await;
    flow_version(&db, json!([whatsapp_flow_step(Value::Null, "src-1")]), true).await;
    assert_eq!(
        sent(&db, Some(&format!("{FLOW}::{BUTTON}")))
            .await
            .unwrap()
            .flow_source_id,
        "src-1"
    );
    // No token, an empty one, a foreign one, a button no step carries.
    for token in [None, Some(""), Some("a-random-uuid-from-elsewhere")] {
        assert_eq!(sent(&db, token).await, None);
    }
    assert_eq!(
        sent(&db, Some(&format!("{FLOW}::{}", BUTTON.replace('1', "2")))).await,
        None
    );
}
