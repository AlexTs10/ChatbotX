//! Port of `packages/database/__tests__/ai-agent-actions.test.ts` (P3.2b, ported:
//! `safeParse(..).success` is `parse_..(..).is_ok()` on a JSON value).

use chatbotx_db::repo::partials::ai::{parse_ai_agent_action, parse_ai_agent_action_rules};
use serde_json::{Value, json};

const ACTION_IDS: &[&str] = &[
    "send_flow",
    "assign_conversation",
    "remove_assignment",
    "transfer_to_human",
    "add_tag",
    "remove_tag",
    "set_custom_field",
    "clear_custom_field",
    "mark_follow_up",
    "remove_follow_up",
    "transfer_to_bot",
    "archive",
    "block_contact",
];

fn action_by_type(kind: &str) -> Value {
    match kind {
        "send_flow" => json!({ "id": kind, "type": kind, "flowId": "flow-1" }),
        "assign_conversation" => json!({ "id": kind, "type": kind, "assignedId": "u_user-1" }),
        "add_tag" | "remove_tag" => json!({ "id": kind, "type": kind, "tagId": "tag-1" }),
        "set_custom_field" | "clear_custom_field" => {
            json!({ "id": kind, "type": kind, "customFieldId": "field-1" })
        }
        _ => json!({ "id": kind, "type": kind }),
    }
}

#[test]
fn ai_agent_action_contract_accepts_every_configured_action_variant() {
    for kind in ACTION_IDS {
        assert!(
            parse_ai_agent_action(&action_by_type(kind)).is_ok(),
            "{kind}"
        );
    }
}

#[test]
fn ai_agent_action_contract_rejects_duplicate_stable_ids_and_empty_rules() {
    assert!(
        parse_ai_agent_action_rules(&json!([
            { "id": "rule", "when": "", "actions": [action_by_type("archive")] },
        ]))
        .is_err()
    );
    assert!(
        parse_ai_agent_action_rules(&json!([
            { "id": "rule", "when": "matches", "actions": [action_by_type("archive")] },
            { "id": "rule", "when": "other", "actions": [action_by_type("block_contact")] },
        ]))
        .is_err()
    );
    assert!(
        parse_ai_agent_action_rules(&json!([
            {
                "id": "price",
                "when": "asks for prices",
                "actions": [{ "id": "send-flow", "type": "send_flow", "flowId": "price-flow" }],
            },
            {
                "id": "promotion",
                "when": "asks for promotions",
                "actions": [{ "id": "send-flow", "type": "send_flow", "flowId": "promotion-flow" }],
            },
        ]))
        .is_err()
    );
}

#[test]
fn ai_agent_action_contract_rejects_unknown_discriminator_and_excess_rules() {
    assert!(parse_ai_agent_action(&json!({ "id": "x", "type": "delete_everything" })).is_err());
    let rules: Vec<Value> = (0..21)
        .map(|index| {
            json!({
                "id": format!("rule-{index}"),
                "when": "matches",
                "actions": [action_by_type("archive")],
            })
        })
        .collect();
    assert!(parse_ai_agent_action_rules(&Value::Array(rules)).is_err());
}

#[test]
fn ai_agent_action_contract_accepts_inbox_team_and_legacy_administrator_assignment_targets() {
    assert!(
        parse_ai_agent_action(&json!({
            "id": "team-assignment",
            "type": "assign_conversation",
            "assignedId": "t_team-1",
        }))
        .is_ok()
    );
    assert!(
        parse_ai_agent_action(&json!({
            "id": "legacy-admin-assignment",
            "type": "assign_conversation",
            "adminId": "user-1",
        }))
        .is_ok()
    );
}
