//! Port of `packages/events/__tests__/contact-inbox-context.test.ts`
//! (P3.6-X1): `withContactInboxMetadata` and `extractContactInboxId`.

use chatbotx_business::events::contact_inbox::{
    CONTACT_INBOX_METADATA_KEY, extract_contact_inbox_id, with_contact_inbox_metadata,
};
use serde_json::{Map, Value, json};

fn map(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

// withContactInboxMetadata > adds the contactInboxId key when provided
#[test]
fn adds_the_contact_inbox_id_key_when_provided() {
    let metadata = map(json!({ "tagId": "tag-1" }));
    let result = with_contact_inbox_metadata(Some(metadata), Some("ci-1"));
    assert_eq!(
        result,
        Some(map(json!({ "tagId": "tag-1", "contactInboxId": "ci-1" })))
    );
}

// withContactInboxMetadata > returns the metadata object byte-identical (same reference) when contactInboxId is omitted
#[test]
fn returns_the_metadata_unchanged_when_contact_inbox_id_is_omitted() {
    let metadata = map(json!({ "tagId": "tag-1" }));
    let result = with_contact_inbox_metadata(Some(metadata.clone()), None);
    assert_eq!(result, Some(metadata));
}

// withContactInboxMetadata > returns undefined when both metadata and contactInboxId are omitted
#[test]
fn returns_none_when_both_metadata_and_contact_inbox_id_are_omitted() {
    assert_eq!(with_contact_inbox_metadata(None, None), None);
}

// withContactInboxMetadata > builds a fresh metadata object from undefined when only contactInboxId is provided
#[test]
fn builds_fresh_metadata_when_only_contact_inbox_id_is_provided() {
    assert_eq!(
        with_contact_inbox_metadata(None, Some("ci-1")),
        Some(map(json!({ "contactInboxId": "ci-1" })))
    );
}

// withContactInboxMetadata > does not mutate the original metadata object
#[test]
fn does_not_mutate_the_original_metadata_object() {
    let metadata = map(json!({ "tagId": "tag-1" }));
    let _ = with_contact_inbox_metadata(Some(metadata.clone()), Some("ci-1"));
    assert_eq!(metadata, map(json!({ "tagId": "tag-1" })));
}

fn extract(value: Value) -> Option<String> {
    extract_contact_inbox_id(Some(&map(json!({ (CONTACT_INBOX_METADATA_KEY): value }))))
}

// extractContactInboxId > returns the value for a real (numeric snowflake) id
#[test]
fn returns_the_value_for_a_real_numeric_snowflake_id() {
    assert_eq!(
        extract(json!("11669088263749632")),
        Some("11669088263749632".to_string())
    );
}

// extractContactInboxId > returns undefined for a non-numeric string (malformed/replayed job)
#[test]
fn returns_none_for_a_non_numeric_string() {
    for bad in ["not-an-id", "123abc", " 123", "12.3", "-5"] {
        assert_eq!(extract(json!(bad)), None, "{bad:?}");
    }
}

// extractContactInboxId > returns undefined when eventData is undefined
#[test]
fn returns_none_when_event_data_is_none() {
    assert_eq!(extract_contact_inbox_id(None), None);
}

// extractContactInboxId > returns undefined when the key is missing
#[test]
fn returns_none_when_the_key_is_missing() {
    assert_eq!(
        extract_contact_inbox_id(Some(&map(json!({ "sourceId": "tag-1" })))),
        None
    );
}

// extractContactInboxId > returns undefined when the value is null
#[test]
fn returns_none_when_the_value_is_null() {
    assert_eq!(extract(Value::Null), None);
}

// extractContactInboxId > returns undefined when the value is not a string (malformed)
#[test]
fn returns_none_when_the_value_is_not_a_string() {
    assert_eq!(extract(json!(12_345)), None);
}

// extractContactInboxId > returns undefined when the value is an empty string
#[test]
fn returns_none_when_the_value_is_an_empty_string() {
    assert_eq!(extract(json!("")), None);
}

// extractContactInboxId > returns undefined when the value is an object (malformed)
#[test]
fn returns_none_when_the_value_is_an_object() {
    assert_eq!(extract(json!({ "nested": true })), None);
}
