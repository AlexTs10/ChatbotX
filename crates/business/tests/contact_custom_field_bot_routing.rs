//! Port of `packages/business/__tests__/contact-custom-field-bot-routing.test.ts`
//! (P3.6-D2 slice bot fields). Adapted: the mocked `botFieldService` /
//! `customFieldModel` / `setValues` / `deleteByCustomFieldId` become real
//! `BotField`, `CustomField` and `ContactCustomField` rows ("routes to
//! botFieldService" = the bot field row changed and no contact value was
//! written).

mod common;

use chatbotx_business::contact_custom_field::{
    FieldOperationType as Op, SetValueByKeyInput, TemporalInputParsing, delete_by_key,
    set_value_by_key,
};
use common::{
    TestDb, WS, bot_field, bot_field_value, contact, custom_field, custom_field_value,
    custom_field_values, setup, two_workspaces,
};

/// Contact 10; custom field 42 "Field" (shortText, value "x"); bot fields 5
/// (number, "1"), 7 (shortText), 8 (date), 9 (shortText, "keep").
async fn fixture() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact(tx, 10, WS)?;
        custom_field(tx, 42, WS, "Field", "shortText")?;
        custom_field_value(tx, 900, 10, 42, "x")?;
        bot_field(tx, 5, WS, "visits", "number", Some("1"), 1_000)?;
        bot_field(tx, 7, WS, "greeting", "shortText", None, 1_000)?;
        bot_field(tx, 8, WS, "day", "date", None, 1_000)?;
        bot_field(tx, 9, WS, "keep", "shortText", Some("keep"), 1_000)?;
        Ok(())
    })
    .await;
    db
}

fn by_key(keyword: &str, value: &str, allow_bot_fields: bool) -> SetValueByKeyInput {
    SetValueByKeyInput {
        workspace_id: WS,
        contact_id: 10,
        keyword: keyword.to_string(),
        value: value.to_string(),
        allow_bot_fields,
        ..SetValueByKeyInput::default()
    }
}

fn x_only() -> Vec<(i64, String)> {
    vec![(42, "x".to_string())]
}

// ---- contactCustomFieldService.setValueByKey — bot-field routing ----

#[tokio::test]
async fn contact_custom_field_service_set_value_by_key_bot_field_routing_allow_bot_fields_true_bot_token_routes_to_bot_field_service_with_the_operation_never_touching_the_contact_lookup()
 {
    let db = fixture().await;
    let input = SetValueByKeyInput {
        operation: Some(Op::Increase),
        ..by_key("bot_field:5", "42", true)
    };
    let changes = set_value_by_key(&db, input).await.expect("set");
    assert!(changes.is_empty());
    assert_eq!(bot_field_value(&db, 5).await, Some(Some("43".to_string())));
    assert_eq!(custom_field_values(&db, 10).await, x_only());
}

#[tokio::test]
async fn contact_custom_field_service_set_value_by_key_bot_field_routing_allow_bot_fields_true_bot_token_defaults_the_operation_to_set_when_omitted()
 {
    let db = fixture().await;
    set_value_by_key(&db, by_key("bot_field:7", "hello", true))
        .await
        .expect("set");
    assert_eq!(
        bot_field_value(&db, 7).await,
        Some(Some("hello".to_string()))
    );
}

#[tokio::test]
async fn contact_custom_field_service_set_value_by_key_bot_field_routing_forwards_temporal_input_parsing_and_fill_empty_temporal_with_now_to_bot_field_service_apply_value_operation()
 {
    let db = fixture().await;
    let input = SetValueByKeyInput {
        temporal_input_parsing: Some(TemporalInputParsing::Lenient),
        fill_empty_temporal_with_now: true,
        source_timezone_override: Some("Asia/Ho_Chi_Minh".to_string()),
        ..by_key("bot_field:8", "", true)
    };
    set_value_by_key(&db, input).await.expect("set");
    let value = bot_field_value(&db, 8).await.flatten().expect("stamped");
    assert!(value.ends_with("T00:00:00+07:00"), "{value}");
}

#[tokio::test]
async fn contact_custom_field_service_set_value_by_key_bot_field_routing_allow_bot_fields_true_plain_id_falls_through_to_the_legacy_contact_path_never_touching_bot_field_service()
 {
    let db = fixture().await;
    set_value_by_key(&db, by_key("42", "hi", true))
        .await
        .expect("set");
    assert_eq!(
        custom_field_values(&db, 10).await,
        vec![(42, "hi".to_string())]
    );
    assert_eq!(bot_field_value(&db, 5).await, Some(Some("1".to_string())));
}

#[tokio::test]
async fn contact_custom_field_service_set_value_by_key_bot_field_routing_allow_bot_fields_false_default_bot_token_falls_through_to_legacy_name_lookup_and_throws_the_existing_not_found_never_touching_bot_field_service()
 {
    let db = fixture().await;
    let error = set_value_by_key(&db, by_key("bot_field:5", "42", false))
        .await
        .expect_err("not found");
    assert_eq!(error.message(), "Custom field not found");
    assert_eq!(bot_field_value(&db, 5).await, Some(Some("1".to_string())));
}

// ---- contactCustomFieldService.deleteByKey — bot-field routing ----

#[tokio::test]
async fn contact_custom_field_service_delete_by_key_bot_field_routing_allow_bot_fields_true_bot_token_routes_to_bot_field_service_clear_value_by_key_nulls_doesn_t_delete_a_row_never_touching_the_contact_lookup()
 {
    let db = fixture().await;
    let change = delete_by_key(&db, WS, 10, "bot_field:9", true)
        .await
        .expect("delete");
    assert_eq!(change, None);
    assert_eq!(bot_field_value(&db, 9).await, Some(None));
    assert_eq!(custom_field_values(&db, 10).await, x_only());
}

#[tokio::test]
async fn contact_custom_field_service_delete_by_key_bot_field_routing_allow_bot_fields_true_plain_id_name_falls_through_to_the_legacy_contact_delete_path_never_touching_bot_field_service()
 {
    let db = fixture().await;
    delete_by_key(&db, WS, 10, "Field", true)
        .await
        .expect("delete");
    assert!(custom_field_values(&db, 10).await.is_empty());
    assert_eq!(
        bot_field_value(&db, 9).await,
        Some(Some("keep".to_string()))
    );
}

#[tokio::test]
async fn contact_custom_field_service_delete_by_key_bot_field_routing_allow_bot_fields_false_default_bot_token_falls_through_to_legacy_name_lookup_and_throws_the_existing_not_found_never_touching_bot_field_service()
 {
    let db = fixture().await;
    delete_by_key(&db, WS, 10, "bot_field:9", false)
        .await
        .expect_err("not found");
    assert_eq!(
        bot_field_value(&db, 9).await,
        Some(Some("keep".to_string()))
    );
}

#[tokio::test]
async fn contact_custom_field_service_delete_by_key_bot_field_routing_public_api_guarantee_the_workspace_token_contact_custom_field_delete_handler_s_exact_call_shape_no_allow_bot_fields_never_clears_a_bot_field_for_a_bot_field_id_token()
 {
    let db = fixture().await;
    let error = delete_by_key(&db, WS, 10, "bot_field:9", false)
        .await
        .expect_err("not found");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(
        bot_field_value(&db, 9).await,
        Some(Some("keep".to_string()))
    );
}
