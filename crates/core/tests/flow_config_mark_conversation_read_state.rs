//! Port of `packages/flow-config/__tests__/mark-conversation-read-state.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn mark_conversation_read_unread_step_contracts_defaults_carry_the_right_steptype_and_an_id() {
    let read = fc::mark_conversation_as_read_step_default_fn();
    let unread = fc::mark_conversation_as_unread_step_default_fn();

    assert_eq!(read["stepType"], "markConversationAsRead");
    assert_eq!(unread["stepType"], "markConversationAsUnread");
    assert!(truthy(&read["id"]));
    assert!(truthy(&unread["id"]));
}

#[test]
fn mark_conversation_read_unread_step_contracts_schemas_round_trip_their_defaults_and_reject_the_other_type()
 {
    let read = fc::mark_conversation_as_read_step_default_fn();
    let unread = fc::mark_conversation_as_unread_step_default_fn();

    assert_eq!(
        fc::mark_conversation_as_read_step_schema()
            .parse(&read)
            .unwrap(),
        read
    );
    assert_eq!(
        fc::mark_conversation_as_unread_step_schema()
            .parse(&unread)
            .unwrap(),
        unread
    );
    assert!(
        fc::mark_conversation_as_read_step_schema()
            .parse(&unread)
            .is_err()
    );
}

#[test]
fn mark_conversation_read_unread_step_contracts_both_are_registered_as_action_steps() {
    for step_type in ["markConversationAsRead", "markConversationAsUnread"] {
        assert!(any_ok(
            &fc::action_steps(),
            &json!({ "id": "1", "stepType": step_type })
        ));
    }
}
