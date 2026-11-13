//! Ports of the custom-field part of
//! `apps/builder/__tests__/contacts-tags-and-fields-public-api.test.ts`
//! (P3.6-D2 slice contact custom-field values). Adapted: the captured oRPC
//! handlers become `handlers::dispatch` calls over a real migrated database;
//! "the service was called with" becomes the response and the stored rows
//! (an operation code is checked by its effect on a field of a type that
//! supports it); `schema.safeParse` becomes "the call is (not) a 422".

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

/// Contact 10; fields 31 "Full Name" shortText, 32 "Loyalty Points" number,
/// 33 "when" datetime, 34 "day" date; contact 10 has "Full Name" = "a".
async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (10, 'Ada', ?1)"#,
                params![WS],
            )?;
            for (id, name, kind) in [
                (31_i64, "Full Name", "shortText"),
                (32, "Loyalty Points", "number"),
                (33, "when", "datetime"),
                (34, "day", "date"),
            ] {
                tx.execute(
                    r#"INSERT INTO "CustomField" ("id", "workspaceId", "name", "type") VALUES (?1, ?2, ?3, ?4)"#,
                    params![id, WS, name, kind],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value") VALUES (900, 10, 31, 'a')"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

const CONTACT: (&str, &str) = ("identifier", "id:10");

/// The stored `customFieldId -> value` of contact 10.
async fn stored(f: &Fixture) -> Value {
    f.state
        .db
        .read(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "customFieldId", "value" FROM "ContactCustomField" WHERE "contactId" = 10 ORDER BY "customFieldId""#,
            )?;
            let mut out = serde_json::Map::new();
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (id, value) = row?;
                out.insert(id.to_string(), json!(value));
            }
            Ok(Value::Object(out))
        })
        .await
        .expect("stored")
}

async fn put(f: &Fixture, key: &str, body: Value) -> (u16, Value) {
    call(
        f,
        "contacts.setCustomField",
        WS,
        &[CONTACT, ("idOrName", key)],
        None,
        Some(body),
    )
    .await
}

async fn patch(f: &Fixture, body: Value) -> (u16, Value) {
    call(
        f,
        "contacts.applyCustomFieldOperations",
        WS,
        &[CONTACT],
        None,
        Some(body),
    )
    .await
}

fn ops(list: Value) -> Value {
    json!({ "operations": list })
}

fn issue_paths(body: &Value) -> Vec<String> {
    body["data"]["issues"]
        .as_array()
        .map(|issues| {
            issues
                .iter()
                .map(|issue| {
                    issue["path"]
                        .as_array()
                        .map(|p| {
                            p.iter()
                                .map(|s| s.as_str().map_or_else(|| s.to_string(), str::to_string))
                                .collect::<Vec<_>>()
                                .join(".")
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn full_name(value: &str) -> Value {
    json!({ "id": "31", "name": "Full Name", "type": "shortText", "description": null, "value": value })
}

// ---- GET /v1/contacts/{identifier}/custom-fields ----

#[tokio::test]
async fn get_v1_contacts_identifier_custom_fields_resolves_the_contact_id_via_resolve_id_by_identifier_then_returns_list_contact_custom_fields()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(&f, "contacts.listCustomFields", WS, &[CONTACT], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "data": [full_name("a")] }));
}

// ---- GET /v1/contacts/{identifier}/custom-fields/{idOrName} ----

#[tokio::test]
async fn get_v1_contacts_identifier_custom_fields_id_or_name_resolves_the_contact_id_and_the_field_by_id_then_returns_find_contact_custom_field()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "contacts.getCustomField",
        WS,
        &[CONTACT, ("idOrName", "31")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, full_name("a"));
}

#[tokio::test]
async fn get_v1_contacts_identifier_custom_fields_id_or_name_resolves_the_field_by_name() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "contacts.getCustomField",
        WS,
        &[CONTACT, ("idOrName", "Full Name")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, full_name("a"));
}

// ---- PUT /v1/contacts/{identifier}/custom-fields/{idOrName} ----

#[tokio::test]
async fn put_v1_contacts_identifier_custom_fields_id_or_name_resolves_the_contact_id_and_the_field_by_id_before_setting_the_value()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = put(&f, "31", json!({ "value": "hello" })).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored(&f).await, json!({ "31": "hello" }));
}

#[tokio::test]
async fn put_v1_contacts_identifier_custom_fields_id_or_name_forwards_the_caller_s_client_timezone_to_anchor_a_date_value()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = put(
        &f,
        "34",
        json!({ "value": "2026-10-03", "clientTimezone": "Asia/Ho_Chi_Minh" }),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored(&f).await["34"], json!("2026-10-03T00:00:00+07:00"));
}

#[tokio::test]
async fn put_v1_contacts_identifier_custom_fields_id_or_name_resolves_the_field_by_name_before_setting_the_value()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = put(&f, "Full Name", json!({ "value": "hello" })).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored(&f).await, json!({ "31": "hello" }));
}

// ---- DELETE /v1/contacts/{identifier}/custom-fields/{idOrName} ----

#[tokio::test]
async fn delete_v1_contacts_identifier_custom_fields_id_or_name_resolves_the_contact_id_via_resolve_id_by_identifier_before_deleting_by_key()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "contacts.clearCustomField",
        WS,
        &[CONTACT, ("idOrName", "31")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored(&f).await, json!({}));
}

// ---- DELETE /v1/contacts/{identifier}/custom-fields ----

#[tokio::test]
async fn delete_v1_contacts_identifier_custom_fields_resolves_the_contact_id_via_resolve_id_by_identifier_before_clearing_all_custom_fields()
 {
    let f = fixture().await;
    seed(&f).await;
    put(&f, "32", json!({ "value": "3" })).await;
    let (status, body) = call(&f, "contacts.clearCustomFields", WS, &[CONTACT], None, None).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored(&f).await, json!({}));
}

// ---- PATCH /v1/contacts/{identifier}/custom-fields ----

/// `(operation, field, operand, stored before, stored after)`.
type Case = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
);

#[tokio::test]
async fn patch_v1_contacts_identifier_custom_fields_maps_friendly_operation_names_to_internal_field_operation_type_codes()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = patch(
        &f,
        ops(json!([{ "customFieldId": "32", "operation": "increase", "value": "1" }])),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored(&f).await["32"], json!("1"));
}

#[tokio::test]
async fn patch_v1_contacts_identifier_custom_fields_maps_the_s_operation_to_s() {
    let f = fixture().await;
    seed(&f).await;
    // (operation, field, operand, stored before, stored after; None = cleared / any).
    let cases: [Case; 7] = [
        ("clear", "31", "1", Some("a"), None),
        ("setNow", "33", "", None, Some("")),
        ("formula", "32", "1", None, Some("1")),
        ("dateExpression", "33", "now", None, Some("")),
        (
            "addMinutes",
            "33",
            "1",
            Some("2025-01-01T00:00:00.000Z"),
            Some("2025-01-01T00:01:00.000Z"),
        ),
        (
            "addHours",
            "33",
            "1",
            Some("2025-01-01T00:00:00.000Z"),
            Some("2025-01-01T01:00:00.000Z"),
        ),
        (
            "addDays",
            "33",
            "1",
            Some("2025-01-01T00:00:00.000Z"),
            Some("2025-01-02T00:00:00.000Z"),
        ),
    ];
    for (operation, field, operand, before, after) in cases {
        let clear = ops(json!([{ "customFieldId": field, "operation": "clear" }]));
        assert_eq!(patch(&f, clear).await.0, 204);
        if let Some(before) = before {
            let set = ops(json!([{ "customFieldId": field, "operation": "set", "value": before }]));
            assert_eq!(patch(&f, set).await.0, 204, "{operation} setup");
        }
        let (status, body) = patch(
            &f,
            ops(json!([{ "customFieldId": field, "operation": operation, "value": operand }])),
        )
        .await;
        assert_eq!(status, 204, "{operation}: {body}");
        let now = stored(&f).await[field].clone();
        match after {
            None => assert_eq!(now, Value::Null, "{operation}"),
            Some("") => assert!(
                now.as_str().is_some_and(|v| v.ends_with('Z')),
                "{operation}: {now}"
            ),
            Some(value) => assert_eq!(now, json!(value), "{operation}"),
        }
    }
}

#[tokio::test]
async fn patch_v1_contacts_identifier_custom_fields_resolves_every_field_by_id_or_name_in_one_lookup_before_applying()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = patch(
        &f,
        ops(json!([
            { "customFieldId": "Loyalty Points", "operation": "increase", "value": "1" },
            { "customFieldId": "31", "operation": "set", "value": "x" },
        ])),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored(&f).await, json!({ "31": "x", "32": "1" }));
}

#[tokio::test]
async fn patch_v1_contacts_identifier_custom_fields_an_unknown_field_fails_the_request_before_anything_is_applied()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = patch(
        &f,
        ops(json!([
            { "customFieldId": "31", "operation": "set", "value": "changed" },
            { "customFieldId": "missing", "operation": "set", "value": "b" },
        ])),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Custom field not found"));
    assert_eq!(stored(&f).await, json!({ "31": "a" }));
}

#[tokio::test]
async fn patch_v1_contacts_identifier_custom_fields_makes_a_single_apply_operations_call_carrying_every_operation_in_order()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = patch(
        &f,
        ops(json!([
            { "customFieldId": "31", "operation": "set", "value": "a" },
            { "customFieldId": "31", "operation": "append", "value": "b" },
        ])),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored(&f).await, json!({ "31": "ab" }));
}

#[tokio::test]
async fn patch_v1_contacts_identifier_custom_fields_forwards_client_timezone_for_the_whole_batch() {
    let f = fixture().await;
    seed(&f).await;
    let mut body =
        ops(json!([{ "customFieldId": "34", "operation": "set", "value": "2026-10-03" }]));
    body["clientTimezone"] = json!("America/New_York");
    let (status, body) = patch(&f, body).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(stored(&f).await["34"], json!("2026-10-03T00:00:00-04:00"));
}

// ---- custom-field operation request schema ----

#[tokio::test]
async fn custom_field_operation_request_schema_s_needs_no_value() {
    let f = fixture().await;
    seed(&f).await;
    for (operation, field) in [("clear", "31"), ("setNow", "33")] {
        let (status, body) = patch(
            &f,
            ops(json!([{ "customFieldId": field, "operation": operation }])),
        )
        .await;
        assert_eq!(status, 204, "{operation}: {body}");
    }
}

#[tokio::test]
async fn custom_field_operation_request_schema_s_requires_a_value() {
    let f = fixture().await;
    seed(&f).await;
    for operation in [
        "formula",
        "dateExpression",
        "addDays",
        "addHours",
        "addMinutes",
    ] {
        let (status, body) = patch(
            &f,
            ops(json!([{ "customFieldId": "33", "operation": operation }])),
        )
        .await;
        assert_eq!(status, 422, "{operation}: {body}");
        assert_eq!(issue_paths(&body), ["operations.0.value"], "{operation}");
    }
}

#[tokio::test]
async fn custom_field_operation_request_schema_legacy_operations_keep_accepting_a_blank_value() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = patch(
        &f,
        ops(json!([{ "customFieldId": "32", "operation": "increase", "value": "" }])),
    )
    .await;
    assert_ne!(status, 422, "{body}");
}

#[tokio::test]
async fn custom_field_operation_request_schema_rejects_the_unknown_operation_s() {
    let f = fixture().await;
    seed(&f).await;
    for operation in ["constructor", "O04", "multiply"] {
        let (status, body) = patch(
            &f,
            ops(json!([{ "customFieldId": "32", "operation": operation, "value": "1" }])),
        )
        .await;
        assert_eq!(status, 422, "{operation}: {body}");
        assert_eq!(
            issue_paths(&body),
            ["operations.0.operation"],
            "{operation}"
        );
    }
}

// ---- PATCH custom-fields: schema output feeds the handler ----

#[tokio::test]
async fn patch_custom_fields_schema_output_feeds_the_handler_append_value_j_reaches_the_service_as_j()
 {
    let f = fixture().await;
    seed(&f).await;
    // " x " reaches the service as "x"; "   " as "" (an empty append: unchanged).
    for (raw, expected) in [(" x ", "ax"), ("   ", "ax")] {
        let (status, body) = patch(
            &f,
            ops(json!([{ "customFieldId": "31", "operation": "append", "value": raw }])),
        )
        .await;
        assert_eq!(status, 204, "{raw:?}: {body}");
        assert_eq!(stored(&f).await["31"], json!(expected), "{raw:?}");
    }
}

#[tokio::test]
async fn patch_custom_fields_schema_output_feeds_the_handler_an_unknown_custom_field_is_a_404_custom_field_not_found()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = patch(
        &f,
        ops(json!([{ "customFieldId": "nope", "operation": "set", "value": "" }])),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["code"], json!("notFound"));
    assert_eq!(body["message"], json!("Custom field not found"));
}

// ---- clientTimezone validation ----

#[tokio::test]
async fn client_timezone_validation_an_unknown_iana_zone_is_rejected_instead_of_silently_becoming_utc()
 {
    let f = fixture().await;
    seed(&f).await;
    for (zone, ok) in [("Asia/Ho_Chi_Minh", true), ("Asia/Hanoi", false)] {
        let mut body = ops(json!([{ "customFieldId": "31", "operation": "set", "value": "x" }]));
        body["clientTimezone"] = json!(zone);
        let (status, body) = patch(&f, body).await;
        if ok {
            assert_eq!(status, 204, "{zone}: {body}");
        } else {
            assert_eq!(status, 422, "{zone}: {body}");
            assert_eq!(issue_paths(&body), ["clientTimezone"], "{zone}");
        }
    }
}
