//! Port of `packages/flow-config/__tests__/sendgrid-add-contact.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn sendgridaddcontactschema_provides_safe_defaults_and_registers_in_the_shared_union() {
    let value = fc::send_grid_add_contact_default_fn();
    assert_match_object(&value, &json!({ "emailField": "email", "mergeFields": [] }));
    assert!(value.get("listId").is_none());
    assert!(value.get("phoneField").is_none());
    assert!(
        fc::send_grid_add_contact_schema()
            .safe_parse(&value)
            .success
    );
    assert!(any_ok(&fc::action_steps(), &value));
}

#[test]
fn sendgridaddcontactschema_normalizes_empty_optional_values() {
    let result = fc::send_grid_add_contact_schema()
        .parse(&spread(
            &fc::send_grid_add_contact_default_fn(),
            json!({ "listId": " ", "phoneField": "" }),
        ))
        .unwrap();
    assert!(result.get("listId").is_none());
    assert!(result.get("phoneField").is_none());
}

#[test]
fn sendgridaddcontactschema_rejects_duplicate_target_fields() {
    let value = spread(
        &fc::send_grid_add_contact_default_fn(),
        json!({
            "mergeFields": [
                { "contactFieldId": "field-1", "sendGridField": "target" },
                { "contactFieldId": "field-2", "sendGridField": "target" },
            ],
        }),
    );
    assert!(
        !fc::send_grid_add_contact_schema()
            .safe_parse(&value)
            .success
    );
}
