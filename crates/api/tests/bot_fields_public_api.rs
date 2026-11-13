//! Ports of `apps/builder/__tests__/bot-fields-public-api.test.ts` and
//! `bot-fields-public-id-schema.test.ts` (P3.6-D2 slice bot fields).
//! Adapted: the captured oRPC handlers become `handlers::dispatch` calls over
//! a real migrated database; "the service was called with" becomes the
//! response and the stored rows; `inputSchema.safeParse` becomes "the call is
//! (not) a 422". An unknown operation name is refused by the input schema
//! (422) before the handler's own `invalidFieldOperation` check. The route
//! tag check is not-applicable (route metadata, not a handler behaviour).

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

/// Folder 70 (`customField`); bot fields (id, name, type, value, folder,
/// createdAt): 1 "first", 123 "plan", 3 "legacy-field", 4 "visits" (number
/// "1"), 5 "note" ("a"), 6 "deadline" (date), 7 "token one" (folder 70),
/// 8 "tokens" (root), 9007199254740993 "big"; `WS` zone `Asia/Ho_Chi_Minh`.
async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "Workspace" SET "timezone" = 'Asia/Ho_Chi_Minh' WHERE "id" = ?1"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Folder" ("id", "workspaceId", "name", "folderType", "paths") VALUES (70, ?1, 'F', 'customField', '[]')"#,
                params![WS],
            )?;
            for (id, name, kind, value, folder, at) in [
                (1_i64, "first", "shortText", None, None, 1_000_i64),
                (123, "plan", "shortText", Some("free"), None, 2_000),
                (3, "legacy-field", "shortText", None, None, 3_000),
                (4, "visits", "number", Some("1"), None, 4_000),
                (5, "note", "shortText", Some("a"), None, 5_000),
                (6, "deadline", "date", Some("2026-03-10T00:00:00+07:00"), None, 6_000),
                (7, "token one", "shortText", None, Some(70_i64), 7_000),
                (8, "tokens", "shortText", None, None, 8_000),
                (9_007_199_254_740_993, "big", "shortText", None, None, 9_000),
            ] {
                tx.execute(
                    r#"INSERT INTO "BotField" ("id", "workspaceId", "name", "type", "value", "folderId", "createdAt", "updatedAt") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)"#,
                    params![id, WS, name, kind, value, folder, at],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
}

async fn value_of(f: &Fixture, id: i64) -> Value {
    f.state
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "value" FROM "BotField" WHERE "id" = ?1"#,
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )?)
        })
        .await
        .map_or(Value::Null, |v| v.map_or(Value::Null, Value::String))
}

async fn set_many(f: &Fixture, fields: Value) -> (u16, Value) {
    call(
        f,
        "botFields.setMany",
        WS,
        &[],
        None,
        Some(json!({ "fields": fields })),
    )
    .await
}

async fn operation(f: &Fixture, key: &str, body: Value) -> (u16, Value) {
    call(
        f,
        "botFields.applyOperation",
        WS,
        &[("idOrName", key)],
        None,
        Some(body),
    )
    .await
}

fn names(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|row| row["name"].as_str().expect("name").to_string())
        .collect()
}

// ---- PATCH /v1/bot-fields/{idOrName} ----

#[tokio::test]
async fn patch_v1_bot_fields_id_or_name_passes_only_the_given_fields_to_update_by_key() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "botFields.update",
        WS,
        &[("idOrName", "token one")],
        None,
        Some(json!({ "name": "lead_score", "folderId": null })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["name"], json!("lead_score"));
    assert_eq!(body["folderId"], Value::Null);
    assert_eq!(body["type"], json!("shortText"));
}

// ---- PATCH /v1/bot-fields/{idOrName} with an empty body ----

#[tokio::test]
async fn patch_v1_bot_fields_id_or_name_with_an_empty_body_returns_the_field_unchanged_instead_of_writing()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "botFields.update",
        WS,
        &[("idOrName", "plan")],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("123"));
    assert_eq!(body["value"], json!("free"));
    assert_eq!(body["updatedAt"], body["createdAt"]);
}

// ---- PUT /v1/bot-fields ----

#[tokio::test]
async fn put_v1_bot_fields_resolves_each_entry_addressed_by_id() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = set_many(&f, json!([{ "id": 123, "value": "pro" }])).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(value_of(&f, 123).await, json!("pro"));
}

#[tokio::test]
async fn put_v1_bot_fields_resolves_each_entry_addressed_by_name() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = set_many(&f, json!([{ "name": "plan", "value": "pro" }])).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(value_of(&f, 123).await, json!("pro"));
}

#[tokio::test]
async fn put_v1_bot_fields_accepts_the_legacy_key_shape_for_backward_compatibility() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = set_many(&f, json!([{ "key": "plan", "value": "pro" }])).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(value_of(&f, 123).await, json!("pro"));
}

#[tokio::test]
async fn put_v1_bot_fields_resolves_a_mixed_batch_of_id_name_and_legacy_key_entries_in_order() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = set_many(
        &f,
        json!([
            { "id": 1, "value": "a" },
            { "name": "plan", "value": "b" },
            { "key": "legacy-field", "value": "c" },
        ]),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(value_of(&f, 1).await, json!("a"));
    assert_eq!(value_of(&f, 123).await, json!("b"));
    assert_eq!(value_of(&f, 3).await, json!("c"));
}

// ---- POST /v1/bot-fields/{idOrName}/reset ----

#[tokio::test]
async fn post_v1_bot_fields_id_or_name_reset_clears_the_value_of_one_field_addressed_by_id_or_name()
{
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "botFields.reset",
        WS,
        &[("idOrName", "plan")],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("123"));
    assert_eq!(body["value"], Value::Null);
    assert_eq!(value_of(&f, 123).await, Value::Null);
}

// ---- POST /v1/bot-fields/{idOrName}/operations ----

#[tokio::test]
async fn post_v1_bot_fields_id_or_name_operations_applies_the_named_operation_through_the_shared_service_method()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = operation(
        &f,
        "visits",
        json!({ "operation": "increase", "value": "2" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("4"));
    assert_eq!(body["value"], json!("3"));
}

#[tokio::test]
async fn post_v1_bot_fields_id_or_name_operations_forwards_the_caller_s_timezone_for_date_operations()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = operation(
        &f,
        "deadline",
        json!({ "operation": "addDays", "value": "3", "clientTimezone": "Asia/Ho_Chi_Minh" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["value"], json!("2026-03-13T00:00:00+07:00"));
}

#[tokio::test]
async fn post_v1_bot_fields_id_or_name_operations_rejects_an_unknown_operation_name_without_calling_the_service()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = operation(
        &f,
        "visits",
        json!({ "operation": "multiply", "value": "2" }),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(value_of(&f, 4).await, json!("1"));
}

#[tokio::test]
async fn post_v1_bot_fields_id_or_name_operations_the_input_schema_trims_the_value_and_rejects_an_unknown_operation()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) =
        operation(&f, "note", json!({ "operation": "append", "value": " x " })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["value"], json!("ax"));
    let (status, _) = operation(&f, "note", json!({ "operation": "multiply", "value": "1" })).await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn post_v1_bot_fields_id_or_name_operations_the_input_schema_requires_an_operand_for_formula_and_none_for_clear()
 {
    let f = fixture().await;
    seed(&f).await;
    let status = |body: Value| {
        let f = &f;
        async move { operation(f, "visits", body).await.0 }
    };
    assert_eq!(
        status(json!({ "operation": "formula", "value": "  " })).await,
        422
    );
    assert_eq!(status(json!({ "operation": "addDays" })).await, 422);
    assert_ne!(status(json!({ "operation": "clear" })).await, 422);
    assert_ne!(status(json!({ "operation": "setNow" })).await, 422);
    assert_ne!(
        status(json!({ "operation": "increase", "value": "" })).await,
        422
    );
}

#[tokio::test]
async fn post_v1_bot_fields_id_or_name_operations_a_type_the_operation_does_not_fit_surfaces_the_service_s_invalid_field_operation()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) =
        operation(&f, "note", json!({ "operation": "increase", "value": "1" })).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["code"], json!("invalidFieldOperation"));
}

// ---- POST /v1/bot-fields/bulk-reset ----

#[tokio::test]
async fn post_v1_bot_fields_bulk_reset_clears_the_values_of_several_fields_in_this_workspace() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "botFields.resetMany",
        WS,
        &[],
        None,
        Some(json!({ "ids": ["4", "5"] })),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(value_of(&f, 4).await, Value::Null);
    assert_eq!(value_of(&f, 5).await, Value::Null);
    assert_eq!(value_of(&f, 123).await, json!("free"));
}

#[tokio::test]
async fn post_v1_bot_fields_bulk_reset_is_a_204_route_like_the_other_body_less_mutations() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "botFields.resetMany",
        WS,
        &[],
        None,
        Some(json!({ "ids": ["1"] })),
    )
    .await;
    assert_eq!((status, body), (204, Value::Null));
}

// ---- GET /v1/bot-fields ----

#[tokio::test]
async fn get_v1_bot_fields_passes_the_name_and_folder_filters_through_and_defaults_to_newest_first()
{
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "botFields.list",
        WS,
        &[],
        Some("page=1&perPage=20&name=tok&folderId=0"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(names(&body), ["tokens"]);
    let (_, all) = call(
        &f,
        "botFields.list",
        WS,
        &[],
        Some("page=1&perPage=20"),
        None,
    )
    .await;
    assert_eq!(names(&all)[..2], ["big", "tokens"]);
    assert_eq!(all["pageCount"], json!(1));
}

#[tokio::test]
async fn get_v1_bot_fields_honours_an_explicit_sort() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "botFields.list",
        WS,
        &[],
        Some("page=1&perPage=3&sort[0][id]=name&sort[0][desc]=false"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(names(&body), ["big", "deadline", "first"]);
    assert_eq!(body["pageCount"], json!(3));
}

// ---- bot-fields-public-id-schema.test.ts: publicBotFieldIdSchema ----

#[tokio::test]
async fn public_bot_field_id_schema_keeps_a_snowflake_id_beyond_2_53_intact() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = set_many(&f, json!([{ "id": "9007199254740993", "value": "v" }])).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(value_of(&f, 9_007_199_254_740_993).await, json!("v"));
}

#[tokio::test]
async fn public_bot_field_id_schema_still_accepts_a_plain_integer_for_older_callers() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = set_many(&f, json!([{ "id": 4, "value": 42 }])).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(value_of(&f, 4).await, json!("42"));
}

#[tokio::test]
async fn public_bot_field_id_schema_rejects_non_numeric_strings_and_non_positive_numbers() {
    let f = fixture().await;
    seed(&f).await;
    for id in [json!("plan"), json!(0)] {
        let (status, body) = set_many(&f, json!([{ "id": id, "value": "x" }])).await;
        assert_eq!(status, 422, "{id}: {body}");
    }
}

// ---- bot-fields-public-id-schema.test.ts: resetBotFieldsRequest ----

#[tokio::test]
async fn reset_bot_fields_request_requires_at_least_one_id_and_caps_the_batch() {
    let f = fixture().await;
    seed(&f).await;
    let many: Vec<String> = (1..=101).map(|i| i.to_string()).collect();
    for ids in [json!([]), json!(many)] {
        let (status, body) = call(
            &f,
            "botFields.resetMany",
            WS,
            &[],
            None,
            Some(json!({ "ids": ids })),
        )
        .await;
        assert_eq!(status, 422, "{body}");
    }
}
