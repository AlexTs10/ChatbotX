//! Port of the `createItem` cases of
//! `apps/builder/__tests__/pipelines-public-api.test.ts` (the other cases are
//! in `pipelines_public_api.rs` and the later slices' files). Adapted: the
//! zod schema is checked through `handlers::dispatch` over a real pipeline and
//! contact; a refused request is `422 invalidRequestData` at the issue path,
//! an accepted one reaches the service (201, or the service's own
//! `Unknown field` validation when the field ids do not exist).

mod handlers_common;

use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

const CONTACT: i64 = 3001;

async fn board(f: &Fixture) -> String {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'Ada', ?2)"#,
                [CONTACT, WS],
            )?;
            Ok(())
        })
        .await
        .expect("contact");
    let (status, body) = call(
        f,
        "pipelines.create",
        WS,
        &[],
        None,
        Some(json!({ "type": "opportunity", "name": "Sales" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("id").to_string()
}

async fn create_item(f: &Fixture, pipeline_id: &str, patch: Value) -> (u16, Value) {
    let mut body = json!({ "contactId": CONTACT.to_string(), "title": "Deal" });
    if let (Value::Object(base), Value::Object(extra)) = (&mut body, patch) {
        base.extend(extra);
    }
    call(
        f,
        "pipelines.createItem",
        WS,
        &[("id", pipeline_id)],
        None,
        Some(body),
    )
    .await
}

fn is_schema_refusal(status: u16, body: &Value) -> bool {
    status == 422 && body["code"] == "invalidRequestData"
}

#[tokio::test]
async fn create_item_rejects_out_of_range_or_malformed_fields() {
    let f = fixture().await;
    let id = board(&f).await;
    for patch in [
        json!({ "value": 1e16 }),
        json!({ "value": -1 }),
        json!({ "contactId": "abc" }),
        json!({ "title": "  " }),
    ] {
        let (status, body) = create_item(&f, &id, patch.clone()).await;
        assert!(is_schema_refusal(status, &body), "{patch}: {status} {body}");
    }
}

#[tokio::test]
async fn create_item_accepts_the_documented_fields() {
    let f = fixture().await;
    let id = board(&f).await;
    let (status, body) = create_item(
        &f,
        &id,
        json!({
            "value": 1500.5,
            "fieldValues": { "10": "gold", "11": null },
            "idempotencyKey": "flow-1:step-2",
        }),
    )
    .await;
    assert!(!is_schema_refusal(status, &body), "{body}");
    // The schema kept both keys; the service then names the first unknown one.
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["message"], "Unknown field 10", "{body}");
}

#[tokio::test]
async fn create_item_caps_the_number_of_custom_field_values() {
    let f = fixture().await;
    let id = board(&f).await;
    let field_values: serde_json::Map<String, Value> = (0..101)
        .map(|index| (index.to_string(), json!("x")))
        .collect();
    let (status, body) = create_item(&f, &id, json!({ "fieldValues": field_values })).await;
    assert!(is_schema_refusal(status, &body), "{status} {body}");
}

#[tokio::test]
async fn create_item_rejects_nul_in_text_fields() {
    let f = fixture().await;
    let id = board(&f).await;
    let (status, body) = create_item(&f, &id, json!({ "title": "ok" })).await;
    assert_eq!(status, 201, "{body}");
    for patch in [
        json!({ "title": "a\u{0}" }),
        json!({ "description": "a\u{0}" }),
    ] {
        let (status, body) = create_item(&f, &id, patch.clone()).await;
        assert!(is_schema_refusal(status, &body), "{patch}: {status} {body}");
    }
}

#[tokio::test]
async fn nul_is_rejected_in_idempotency_keys() {
    let f = fixture().await;
    let id = board(&f).await;
    let (status, body) = create_item(
        &f,
        &id,
        json!({ "title": "ok", "idempotencyKey": "k\u{0}" }),
    )
    .await;
    assert!(is_schema_refusal(status, &body), "{status} {body}");
    let (status, body) =
        create_item(&f, &id, json!({ "title": "ok", "idempotencyKey": "k" })).await;
    assert_eq!(status, 201, "{body}");
}

#[tokio::test]
async fn field_value_keys_are_bigint_field_ids() {
    let f = fixture().await;
    let id = board(&f).await;
    for key in ["a\u{0}b".to_string(), "name".to_string(), "9".repeat(25)] {
        let (status, body) =
            create_item(&f, &id, json!({ "fieldValues": { key.clone(): "x" } })).await;
        assert!(is_schema_refusal(status, &body), "{key:?}: {status} {body}");
    }
    let (status, body) = create_item(&f, &id, json!({ "fieldValues": { "12": "x" } })).await;
    assert!(!is_schema_refusal(status, &body), "{body}");
    assert_eq!(body["message"], "Unknown field 12", "{body}");
}
