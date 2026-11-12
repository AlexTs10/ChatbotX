//! Port of `packages/flow-config/__tests__/moosend-create-contact.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn moosendcreatecontactschema_provides_state_defaults_and_registers_in_the_shared_union() {
    let value = fc::moosend_create_contact_default_fn();
    assert_match_object(&value, &json!({ "emailField": "email", "listId": "" }));
    assert_eq!(value["states"].as_array().unwrap().len(), 2);
    assert!(!any_ok(&fc::action_steps(), &value));

    let valid = spread(&value, json!({ "listId": "list-1" }));
    assert!(
        fc::moosend_create_contact_schema()
            .safe_parse(&valid)
            .success
    );
    assert!(any_ok(&fc::action_steps(), &valid));
}

#[test]
fn moosendcreatecontactschema_requires_a_mailing_list_and_email_field() {
    let value = fc::moosend_create_contact_default_fn();
    assert!(
        !fc::moosend_create_contact_schema()
            .safe_parse(&value)
            .success
    );
    assert!(
        !fc::moosend_create_contact_schema()
            .safe_parse(&spread(
                &value,
                json!({ "listId": "list-1", "emailField": " " })
            ))
            .success
    );
}
