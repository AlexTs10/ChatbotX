//! Port of `packages/flow-config/src/nodes/send-message.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_message_node_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Node id (numeric string) that `edges[].source`/`target` refer to. Use a unique numeric string per node in the graph.")),
("position", z::object(vec![("x", z::number().describe("Horizontal canvas position in pixels.")),
("y", z::number().describe("Vertical canvas position in pixels."))]).describe("Top-left position of the node on the builder canvas.")),
("measured", z::object(vec![("width", z::number().describe("Rendered node width in pixels.")),
("height", z::number().describe("Rendered node height in pixels."))]).describe("Rendered node size on the canvas, in pixels. Layout only; the builder default is 288x100.")),
("type", z::literal(json!("sendMessage")).describe("Node type \"sendMessage\": sends messages. `data.details.beforeStep` selects the channel, `steps` are the messages/actions in order, `quickReplies` are quick-reply buttons shown after the last message.")),
("data", z::object(vec![("name", z::string().trim().min(1).max(255).describe("Node display name shown on the canvas (1-255 characters).")),
("isStartNode", z::boolean().describe("True for the node where the flow begins. Exactly one node should be the start node.")),
("details", z::object(vec![("beforeStep", choose_channel_step_schema()),
("steps", z::array(z::discriminated_union("stepType", vec![send_audio_step_schema(),
send_file_step_schema(),
send_image_step_schema(),
send_multiple_images_step_schema(),
send_text_step_schema(),
send_video_step_schema(),
send_card_step_schema(),
send_carousel_step_schema(),
get_user_data_step_schema(),
send_gif_step_schema(),
typing_step_schema(),
send_wa_template_message_step_schema(),
send_messenger_template_message_step_schema(),
whatsapp_option_list_step_schema(),
send_view_catalog_step_schema(),
whatsapp_call_button_step_schema(),
whatsapp_flow_step_schema(),
send_products_step_schema(),
send_collection_step_schema(),
whatsapp_location_step_schema(),
enable_bot_step_schema(),
disable_bot_step_schema(),
assign_conversation_step_schema(),
auto_assign_conversation_step_schema(),
unassign_conversation_step_schema(),
mark_conversation_as_unread_step_schema(),
mark_conversation_as_read_step_schema(),
follow_conversation_step_schema(),
unfollow_conversation_step_schema(),
archive_conversation_step_schema(),
unarchive_conversation_step_schema(),
thread_control_step_schema(),
add_contact_notes_step_schema(),
block_contact_step_schema(),
add_contact_tag_step_schema(),
remove_contact_tag_step_schema(),
set_custom_field_step_schema(),
clear_custom_field_step_schema(),
delete_contact_step_schema(),
appointment_scheduling_step_schema(),
questionnaires_step_schema(),
set_up_coupon_step_schema(),
mark_coupon_used_step_schema(),
change_gamification_score_step_schema(),
pipeline_step_schema(),
subscribe_broadcast_step_schema(),
unsubscribe_broadcast_step_schema(),
subscribe_sequence_step_schema(),
unsubscribe_sequence_step_schema(),
get_data_from_json_step_schema(),
format_date_step_schema(),
generate_code_step_schema(),
count_characters_step_schema(),
external_request_step_schema(),
execute_javascript_step_schema(),
evaluate_decision_step_schema(),
mark_email_verified_step_schema(),
opt_in_email_step_schema(),
opt_out_email_step_schema(),
active_campaign_sync_contact_schema(),
get_response_add_contact_schema(),
mailchimp_add_member_schema(),
mailer_lite_add_subscriber_schema(),
moosend_create_contact_schema(),
drip_subscribe_subscriber_schema(),
send_grid_add_contact_schema(),
klaviyo_sync_profile_schema(),
start_another_node_step_schema(),
start_external_flow_step_schema(),
start_external_node_step_schema(),
ai_generate_text_schema(),
ai_generate_image_schema(),
ai_delete_message_history_schema(),
ai_edit_image_schema(),
ai_segment_image_schema(),
ai_analyze_image_schema(),
ai_generate_text_agent_schema(),
ai_extract_data_schema(),
ai_speech_to_text_schema(),
ai_text_to_speech_schema(),
spreadsheet_get_row_schema(),
spreadsheet_clear_row_schema(),
spreadsheet_get_random_row_schema(),
spreadsheet_send_data_schema(),
spreadsheet_update_row_schema(),
facebook_custom_audience_schema(),
send_meta_capi_event_schema(),
set_messenger_user_persistent_menu_step_schema(),
enable_messenger_composer_step_schema(),
disable_messenger_composer_step_schema(),
set_messenger_persona_step_schema(),
update_messenger_contact_data_step_schema(),
send_google_ads_conversion_schema(),
make_step_schema(),
trigger_n8n_step_schema(),
find_places_near_me_step_schema()]))),
("quickReplies", z::array(button_step_schema()).super_refine(refine_reply_only_buttons).max(10).describe("Quick-reply buttons shown after the last message (max 10); use [] for none.")),
("quickReplySettings", quick_reply_settings_schema().optional())]).super_refine(refine_quick_reply_settings))]))]));

// HAND-PORT getSendMessageChannel:
// node=>{const parsed=channelTypes.safeParse(node.data.details.beforeStep.channel);return parsed.success?parsed.data:channelTypes.enum.omnichannel}

// HAND-PORT sendMessageNodeDefaultFn:
// props=>({...defaultNodeData(),type:nodeTypeSchema.enum.sendMessage,...props.nodeProps,data:{name:"Send Message",isStartNode:false,...props.dataProps,details:{beforeStep:chooseChannelStepDefaultFn(),steps:[],quickReplies:[],quickReplySettings:quickReplySettingsDefaultFn(),...props.detailProps}}})

pub const BROADCAST_PAYLOAD_TYPE: &str = "broadcast";

pub const SEQUENCE_SCHEDULE_PAYLOAD_TYPE: &str = "sequenceSchedule";

// HAND-PORT isBulkOutboundMetadata:
// (metadata,isBulkBroadcast=false)=>typeof metadata==="object"&&metadata!==null&&"type"in metadata&&(metadata.type===SEQUENCE_SCHEDULE_PAYLOAD_TYPE||metadata.type===BROADCAST_PAYLOAD_TYPE&&isBulkBroadcast)

// HAND-PORT isProactiveOutboundMetadata:
// metadata=>isBulkOutboundMetadata(metadata,true)

pub const COMMENT_AUTOMATION_PAYLOAD_TYPE: &str = "commentAutomation";

pub const UPDATE_STATUS_PAYLOAD_TYPE: &str = "updateStatus";

pub const FLOW_NODE_PAYLOAD_TYPE: &str = "flowNode";

pub const APPOINTMENT_WEBVIEW_SELECTION_PAYLOAD_TYPE: &str = "appointmentWebviewSelection";

pub const APPOINTMENT_AVAILABILITY_RANGE_SELECTION_PAYLOAD_TYPE: &str =
    "appointmentAvailabilityRangeSelection";

pub const APPOINTMENT_AVAILABILITY_RANGE_SKIPPED_PAYLOAD_TYPE: &str =
    "appointmentAvailabilityRangeSkipped";

pub const GET_USER_DATA_WEBVIEW_SELECTION_PAYLOAD_TYPE: &str = "getUserDataWebviewSelection";

schema!(pub fn base_metadata_payload = z::object(vec![("stepId", z::string().optional()),
("contactInboxId", z::string().optional())]));

schema!(pub fn broadcast_metadata_payload = z::object(vec![("stepId", z::string().optional()),
("contactInboxId", z::string()),
("type", z::literal(json!("broadcast"))),
("broadcastId", z::string())]));

schema!(pub fn sequence_schedule_metadata_payload = z::object(vec![("stepId", z::string().optional()),
("contactInboxId", z::string()),
("type", z::literal(json!("sequenceSchedule"))),
("sequenceStepId", z::string()),
("sequenceId", z::string()),
("dispatchId", z::string())]));

schema!(pub fn comment_automation_metadata_payload = z::object(vec![("stepId", z::string().optional()),
("contactInboxId", z::string().optional()),
("type", z::literal(json!("commentAutomation"))),
("commentAutomationId", z::string()),
("commentId", z::string()),
("replyChannel", z::enum_(&["public", "private"]))]));

schema!(pub fn update_status_payload = z::object(vec![("stepId", z::string().optional()),
("contactInboxId", z::string().optional()),
("type", z::literal(json!("updateStatus")))]));

schema!(pub fn appointment_webview_selection_payload = z::object(vec![("stepId", z::string()),
("contactInboxId", z::string().optional()),
("type", z::literal(json!("appointmentWebviewSelection"))),
("selectedStartAt", z::string().iso_datetime(false, false, None)),
("appointmentId", z::string().optional())]));

schema!(pub fn appointment_availability_range_selection_payload = z::object(vec![("stepId", z::string()),
("contactInboxId", z::string().optional()),
("type", z::literal(json!("appointmentAvailabilityRangeSelection"))),
("startDate", z::string().iso_datetime(false, true, None)),
("endDate", z::string().iso_datetime(false, true, None))]));

schema!(pub fn appointment_availability_range_skipped_payload = z::object(vec![("stepId", z::string()),
("contactInboxId", z::string().optional()),
("type", z::literal(json!("appointmentAvailabilityRangeSkipped")))]));

schema!(pub fn get_user_data_webview_selection_payload = z::object(vec![("stepId", z::string()),
("contactInboxId", z::string().optional()),
("type", z::literal(json!("getUserDataWebviewSelection"))),
("challengeId", z::string()),
("selectedValue", z::string().iso_datetime(false, false, None))]));

schema!(pub fn step_input_resolution_payload = z::object(vec![("stepId", z::string()),
("contactInboxId", z::string().optional()),
("type", z::literal(json!("stepInputResolution"))),
("challengeId", z::string()),
("reason", step_input_resolution_reasons())]));

schema!(pub fn metadata_schema = z::discriminated_union("type", vec![broadcast_metadata_payload(),
sequence_schedule_metadata_payload(),
comment_automation_metadata_payload(),
update_status_payload(),
appointment_webview_selection_payload(),
appointment_availability_range_selection_payload(),
appointment_availability_range_skipped_payload(),
get_user_data_webview_selection_payload(),
step_input_resolution_payload()]));

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn send_message_node_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}
