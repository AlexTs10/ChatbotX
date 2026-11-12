//! `packages/flow-config/src/nodes`.

mod add_notes;
mod base;
mod condition;
mod follow_up;
mod landing_page;
mod perform_action;
mod quick_reply_carrier;
mod send_mail;
mod send_message;
mod split_traffic;
mod start_flow;
mod wait;

#[allow(unused_imports)]
use crate::flow_config::prelude::*;
pub use add_notes::*;
pub use base::*;
pub use condition::*;
pub use follow_up::*;
pub use landing_page::*;
pub use perform_action::*;
pub use quick_reply_carrier::*;
pub use send_mail::*;
pub use send_message::*;
pub use split_traffic::*;
pub use start_flow::*;
pub use wait::*;

schema!(pub fn flow_version_schema = z::discriminated_union("type", vec![send_message_node_schema(),
start_flow_node_schema(),
perform_action_node_schema(),
condition_node_schema(),
split_traffic_node_schema(),
wait_node_schema(),
follow_up_node_schema(),
add_notes_node_schema(),
landing_page_node_schema(),
send_mail_node_schema()]));

schema!(pub fn edge_schema = z::object(vec![("id", z::string().describe("Unique edge id.")),
("source", z::string().describe("Id of the node the edge leaves.")),
("sourceHandle", z::string().describe("Output handle on the source node. For the node-level Continue connection use the source node's own id; for a button, condition case, or other step output use that button/case/state id.")),
("target", z::string().describe("Id of the node the edge enters.")),
("targetHandle", z::string().describe("Input handle on the target node: the target node's id."))]));

pub const DISABLED_CONTINUE_NODE_TYPES: &[&str] = &["splitTraffic", "condition"];

pub const DISABLED_CONTINUE_STEP_TYPES: &[&str] = &["appointmentScheduling"];

/// `shouldShowDefaultContinue(type, data)`.
pub fn should_show_default_continue(node_type: &str, data: &Value) -> bool {
    if DISABLED_CONTINUE_NODE_TYPES.contains(&node_type) {
        return false;
    }
    let Some(steps) = data["details"].get("steps") else {
        return true;
    };
    let last = steps.as_array().and_then(|s| s.last());
    !last.is_some_and(|step| {
        step["stepType"]
            .as_str()
            .is_some_and(|t| DISABLED_CONTINUE_STEP_TYPES.contains(&t))
    })
}
