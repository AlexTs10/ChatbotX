//! Port of `packages/flow-config/__tests__/drip-subscribe-subscriber.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn dripsubscribesubscriberschema_requires_an_account_before_the_default_can_be_saved() {
    let value = fc::drip_subscribe_subscriber_default_fn();

    assert_match_object(
        &value,
        &json!({ "accountId": "", "emailField": "email", "tags": [], "mergeFields": [] }),
    );
    assert!(value.get("phoneField").is_none());
    assert!(
        !fc::drip_subscribe_subscriber_schema()
            .safe_parse(&value)
            .success
    );
    let with_account = spread(&value, json!({ "accountId": "123" }));
    assert!(
        fc::drip_subscribe_subscriber_schema()
            .safe_parse(&with_account)
            .success
    );
    assert!(any_ok(&fc::action_steps(), &with_account));
}

#[test]
fn dripsubscribesubscriberschema_trims_tags_and_rejects_duplicates_after_normalization() {
    let value = spread(
        &fc::drip_subscribe_subscriber_default_fn(),
        json!({ "tags": ["vip", " vip "] }),
    );

    assert!(
        !fc::drip_subscribe_subscriber_schema()
            .safe_parse(&value)
            .success
    );
}

#[test]
fn dripsubscribesubscriberschema_rejects_duplicate_drip_field_mappings() {
    let value = spread(
        &fc::drip_subscribe_subscriber_default_fn(),
        json!({
            "mergeFields": [
                { "contactFieldId": "field-1", "dripField": "company" },
                { "contactFieldId": "field-2", "dripField": "company" },
            ],
        }),
    );

    assert!(
        !fc::drip_subscribe_subscriber_schema()
            .safe_parse(&value)
            .success
    );
}
