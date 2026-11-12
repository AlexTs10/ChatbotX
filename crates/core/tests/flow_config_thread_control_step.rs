//! Port of `packages/flow-config/__tests__/thread-control-step.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn thread_control_flow_step_contract_the_default_step_releases_the_thread_and_has_success_and_error_states()
 {
    let parsed = fc::thread_control_step_schema()
        .parse(&fc::thread_control_step_default_fn(&json!({})))
        .unwrap();

    assert_match_object(
        &parsed,
        &json!({ "stepType": "threadControl", "action": "release" }),
    );
    let states: Vec<&Value> = parsed["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| &s["stateType"])
        .collect();
    assert_eq!(states, vec!["success", "error"]);
}

#[test]
fn thread_control_flow_step_contract_accepts_the_s_action() {
    for action in ["release", "pass"] {
        let parsed = fc::thread_control_step_schema()
            .parse(&spread(
                &fc::thread_control_step_default_fn(&json!({})),
                json!({ "action": action }),
            ))
            .unwrap();
        assert_eq!(parsed["action"], action);
    }
}

#[test]
fn thread_control_flow_step_contract_rejects_an_action_the_step_cannot_perform_take_is_a_human_action()
 {
    assert!(
        fc::thread_control_step_schema()
            .parse(&spread(
                &fc::thread_control_step_default_fn(&json!({})),
                json!({ "action": "take" })
            ))
            .is_err()
    );
}

#[test]
fn thread_control_flow_step_contract_rejects_the_wrong_step_type() {
    assert!(
        fc::thread_control_step_schema()
            .parse(&spread(
                &fc::thread_control_step_default_fn(&json!({})),
                json!({ "stepType": "sendText" })
            ))
            .is_err()
    );
}

#[test]
fn thread_control_flow_step_contract_is_registered_as_a_step_type_and_in_the_action_steps_not_as_a_node_type()
 {
    assert!(
        fc::step_types()
            .enum_values()
            .contains(&"threadControl".to_owned())
    );
    let schema = fc::thread_control_step_schema();
    assert!(fc::action_steps().iter().any(|s| s.same(&schema)));
}
