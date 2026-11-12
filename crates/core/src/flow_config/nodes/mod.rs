//! `packages/flow-config/src/nodes`.

pub mod add_notes;
pub mod base;
pub mod condition;
pub mod follow_up;
pub mod landing_page;
pub mod perform_action;
pub mod quick_reply_carrier;
pub mod send_mail;
pub mod send_message;
pub mod split_traffic;
pub mod start_flow;
pub mod wait;

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

// HAND-PORT shouldShowDefaultContinue:
// (type,data)=>{if(disabledContinueNodeTypes.includes(type)){return false}if(!("steps"in data.details)){return true}const lastStep=data.details.steps?.at(-1);return!(lastStep&&disabledContinueStepTypes.includes(lastStep.stepType))}
