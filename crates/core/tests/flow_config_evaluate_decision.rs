//! Port of `packages/flow-config/__tests__/evaluate-decision.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn valid_step() -> Value {
    spread(
        &fc::evaluate_decision_step_default_fn(),
        json!({ "contextCustomFieldIds": ["1"], "profileId": "2" }),
    )
}

#[test]
fn evaluatedecisionstepschema_accepts_a_required_allowed_contact_context() {
    assert!(
        fc::evaluate_decision_step_schema()
            .safe_parse(&valid_step())
            .success
    );
}

#[test]
fn evaluatedecisionstepschema_rejects_missing_duplicate_and_over_limit_context_fields() {
    let schema = fc::evaluate_decision_step_schema();
    assert!(
        !schema
            .safe_parse(&spread(
                &valid_step(),
                json!({ "contextCustomFieldIds": [] })
            ))
            .success
    );
    assert!(
        !schema
            .safe_parse(&spread(
                &valid_step(),
                json!({ "contextCustomFieldIds": ["1", "1"] })
            ))
            .success
    );
    let nine: Vec<String> = (0..9).map(|index| (index + 1).to_string()).collect();
    assert!(
        !schema
            .safe_parse(&spread(
                &valid_step(),
                json!({ "contextCustomFieldIds": nine })
            ))
            .success
    );
}

#[test]
fn evaluatedecisionstepschema_rejects_retired_input_source_configuration() {
    assert!(
        !fc::evaluate_decision_step_schema()
            .safe_parse(&spread(
                &valid_step(),
                json!({ "input": { "type": "currentMessage" } })
            ))
            .success
    );
}
