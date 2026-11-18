//! All integration tests of this crate in one binary (see the guard test).
// Each test file keeps its own copy of its helper modules, as before.
#![allow(clippy::duplicate_mod)]

// Each test file keeps its own `mod common;` / helper lookups, so a helper file
// is loaded once per test module.
#![allow(clippy::duplicate_mod)]

#[path = "decision_config_profile.rs"]
mod decision_config_profile;
#[path = "encryption_appointment_tokens.rs"]
mod encryption_appointment_tokens;
#[path = "encryption_encryption.rs"]
mod encryption_encryption;
#[path = "encryption_keys.rs"]
mod encryption_keys;
#[path = "encryption_leaderboard_view_token.rs"]
mod encryption_leaderboard_view_token;
#[path = "encryption_link_signature.rs"]
mod encryption_link_signature;
#[path = "encryption_media_token.rs"]
mod encryption_media_token;
#[path = "encryption_minigame_play_token.rs"]
mod encryption_minigame_play_token;
#[path = "encryption_open_link_token.rs"]
mod encryption_open_link_token;
#[path = "encryption_shop_token.rs"]
mod encryption_shop_token;
#[path = "encryption_user_data_webview_token.rs"]
mod encryption_user_data_webview_token;
#[path = "encryption_user_hash.rs"]
mod encryption_user_hash;
#[path = "flow_config_active_campaign_sync_contact.rs"]
mod flow_config_active_campaign_sync_contact;
#[path = "flow_config_ai_model_catalog.rs"]
mod flow_config_ai_model_catalog;
#[path = "flow_config_ai_muse_steps.rs"]
mod flow_config_ai_muse_steps;
#[path = "flow_config_ai_segment_image.rs"]
mod flow_config_ai_segment_image;
#[path = "flow_config_ai_speech_to_text.rs"]
mod flow_config_ai_speech_to_text;
#[path = "flow_config_ai_text_to_speech.rs"]
mod flow_config_ai_text_to_speech;
#[path = "flow_config_appointment_scheduling_schema.rs"]
mod flow_config_appointment_scheduling_schema;
#[path = "flow_config_authoring_compile.rs"]
mod flow_config_authoring_compile;
#[path = "flow_config_authoring_errors.rs"]
mod flow_config_authoring_errors;
#[path = "flow_config_authoring_layout.rs"]
mod flow_config_authoring_layout;
#[path = "flow_config_authoring_normalize_graph.rs"]
mod flow_config_authoring_normalize_graph;
#[path = "flow_config_authoring_pipeline_spec.rs"]
mod flow_config_authoring_pipeline_spec;
#[path = "flow_config_authoring_set_custom_field_operations.rs"]
mod flow_config_authoring_set_custom_field_operations;
#[path = "flow_config_authoring_spec_schema.rs"]
mod flow_config_authoring_spec_schema;
#[path = "flow_config_button_payload.rs"]
mod flow_config_button_payload;
#[path = "flow_config_call_phone_number_button.rs"]
mod flow_config_call_phone_number_button;
#[path = "flow_config_channel_policies.rs"]
mod flow_config_channel_policies;
#[path = "flow_config_channel_validator.rs"]
mod flow_config_channel_validator;
#[path = "flow_config_condition_schema.rs"]
mod flow_config_condition_schema;
#[path = "flow_config_copy_text_button.rs"]
mod flow_config_copy_text_button;
#[path = "flow_config_deep_link.rs"]
mod flow_config_deep_link;
#[path = "flow_config_drip_subscribe_subscriber.rs"]
mod flow_config_drip_subscribe_subscriber;
#[path = "flow_config_evaluate_decision.rs"]
mod flow_config_evaluate_decision;
#[path = "flow_config_external_request.rs"]
mod flow_config_external_request;
#[path = "flow_config_field_reference.rs"]
mod flow_config_field_reference;
#[path = "flow_config_find_places_near_me.rs"]
mod flow_config_find_places_near_me;
#[path = "flow_config_flow_action_target.rs"]
mod flow_config_flow_action_target;
#[path = "flow_config_follow_up.rs"]
mod flow_config_follow_up;
#[path = "flow_config_get_response_add_contact.rs"]
mod flow_config_get_response_add_contact;
#[path = "flow_config_get_user_data_step.rs"]
mod flow_config_get_user_data_step;
#[path = "flow_config_import_export.rs"]
mod flow_config_import_export;
#[path = "flow_config_klaviyo_sync_profile.rs"]
mod flow_config_klaviyo_sync_profile;
#[path = "flow_config_mailer_lite_add_subscriber.rs"]
mod flow_config_mailer_lite_add_subscriber;
#[path = "flow_config_mark_conversation_read_state.rs"]
mod flow_config_mark_conversation_read_state;
#[path = "flow_config_media_step_rules.rs"]
mod flow_config_media_step_rules;
#[path = "flow_config_messenger_template.rs"]
mod flow_config_messenger_template;
#[path = "flow_config_moosend_create_contact.rs"]
mod flow_config_moosend_create_contact;
#[path = "flow_config_open_website.rs"]
mod flow_config_open_website;
#[path = "flow_config_openai_compatible_ai_steps.rs"]
mod flow_config_openai_compatible_ai_steps;
#[path = "flow_config_payment_request.rs"]
mod flow_config_payment_request;
#[path = "flow_config_pipeline_action.rs"]
mod flow_config_pipeline_action;
#[path = "flow_config_pipeline_required_fields.rs"]
mod flow_config_pipeline_required_fields;
#[path = "flow_config_pipeline_step.rs"]
mod flow_config_pipeline_step;
#[path = "flow_config_quick_replies_carrier.rs"]
mod flow_config_quick_replies_carrier;
#[path = "flow_config_quick_reply_settings.rs"]
mod flow_config_quick_reply_settings;
#[path = "flow_config_reference_registry_completeness.rs"]
mod flow_config_reference_registry_completeness;
#[path = "flow_config_remap.rs"]
mod flow_config_remap;
#[path = "flow_config_routable_handle.rs"]
mod flow_config_routable_handle;
#[path = "flow_config_send_google_ads_conversion.rs"]
mod flow_config_send_google_ads_conversion;
#[path = "flow_config_send_meta_capi_event.rs"]
mod flow_config_send_meta_capi_event;
#[path = "flow_config_send_multiple_images.rs"]
mod flow_config_send_multiple_images;
#[path = "flow_config_send_products.rs"]
mod flow_config_send_products;
#[path = "flow_config_send_text_length_rules.rs"]
mod flow_config_send_text_length_rules;
#[path = "flow_config_send_view_catalog_step.rs"]
mod flow_config_send_view_catalog_step;
#[path = "flow_config_sendgrid_add_contact.rs"]
mod flow_config_sendgrid_add_contact;
#[path = "flow_config_set_custom_field_operation.rs"]
mod flow_config_set_custom_field_operation;
#[path = "flow_config_spreadsheet_write_mapping.rs"]
mod flow_config_spreadsheet_write_mapping;
#[path = "flow_config_step_input_resolution.rs"]
mod flow_config_step_input_resolution;
#[path = "flow_config_template_button_open_mini_app.rs"]
mod flow_config_template_button_open_mini_app;
#[path = "flow_config_template_export_schema.rs"]
mod flow_config_template_export_schema;
#[path = "flow_config_template_flow_node_schema.rs"]
mod flow_config_template_flow_node_schema;
#[path = "flow_config_template_parameter_values.rs"]
mod flow_config_template_parameter_values;
#[path = "flow_config_template_structure.rs"]
mod flow_config_template_structure;
#[path = "flow_config_thread_control_step.rs"]
mod flow_config_thread_control_step;
#[path = "flow_config_tiktok_text_rules.rs"]
mod flow_config_tiktok_text_rules;
#[path = "flow_config_unresolved_references.rs"]
mod flow_config_unresolved_references;
#[path = "flow_config_wa_template.rs"]
mod flow_config_wa_template;
#[path = "flow_config_wa_template_flow_token.rs"]
mod flow_config_wa_template_flow_token;
#[path = "flow_config_wa_template_limited_time_offer.rs"]
mod flow_config_wa_template_limited_time_offer;
#[path = "flow_config_wa_template_mpm_rules.rs"]
mod flow_config_wa_template_mpm_rules;
#[path = "flow_config_wa_template_payment_request.rs"]
mod flow_config_wa_template_payment_request;
#[path = "flow_config_wa_template_quick_reply_buttons.rs"]
mod flow_config_wa_template_quick_reply_buttons;
#[path = "flow_config_whatsapp_flow_step.rs"]
mod flow_config_whatsapp_flow_step;
#[path = "flow_config_whatsapp_location.rs"]
mod flow_config_whatsapp_location;
#[path = "logger_diagnostic.rs"]
mod logger_diagnostic;
#[path = "logger_err_serializer.rs"]
mod logger_err_serializer;
#[path = "logger_redact.rs"]
mod logger_redact;
#[path = "partials_contact_filter_reference_coverage.rs"]
mod partials_contact_filter_reference_coverage;
#[path = "partials_thread_control_sdk_parity.rs"]
mod partials_thread_control_sdk_parity;
#[path = "utils_call_capable_channels.rs"]
mod utils_call_capable_channels;
#[path = "utils_coexist_channels.rs"]
mod utils_coexist_channels;
#[path = "utils_concurrency.rs"]
mod utils_concurrency;
#[path = "utils_connection.rs"]
mod utils_connection;
#[path = "utils_coordinates.rs"]
mod utils_coordinates;
#[path = "utils_country_currency.rs"]
mod utils_country_currency;
#[path = "utils_created_at_from_id.rs"]
mod utils_created_at_from_id;
#[path = "utils_crypto.rs"]
mod utils_crypto;
#[path = "utils_csv.rs"]
mod utils_csv;
#[path = "utils_custom_field.rs"]
mod utils_custom_field;
#[path = "utils_custom_field_resolution_plan.rs"]
mod utils_custom_field_resolution_plan;
#[path = "utils_date_expression.rs"]
mod utils_date_expression;
#[path = "utils_datetime.rs"]
mod utils_datetime;
#[path = "utils_env.rs"]
mod utils_env;
#[path = "utils_error_log.rs"]
mod utils_error_log;
#[path = "utils_field_value_calculator.rs"]
mod utils_field_value_calculator;
#[path = "utils_form_answer_format.rs"]
mod utils_form_answer_format;
#[path = "utils_google_click.rs"]
mod utils_google_click;
#[path = "utils_graph_error.rs"]
mod utils_graph_error;
#[path = "utils_graph_pagination.rs"]
mod utils_graph_pagination;
#[path = "utils_id.rs"]
mod utils_id;
#[path = "utils_image_optimize.rs"]
mod utils_image_optimize;
#[path = "utils_image_optimize_settings.rs"]
mod utils_image_optimize_settings;
#[path = "utils_inbound_webhook_channel.rs"]
mod utils_inbound_webhook_channel;
#[path = "utils_inbox_image_support.rs"]
mod utils_inbox_image_support;
#[path = "utils_json_path.rs"]
mod utils_json_path;
#[path = "utils_live_media.rs"]
mod utils_live_media;
#[path = "utils_math_expression.rs"]
mod utils_math_expression;
#[path = "utils_media_download.rs"]
mod utils_media_download;
#[path = "utils_meta_capi.rs"]
mod utils_meta_capi;
#[path = "utils_money.rs"]
mod utils_money;
#[path = "utils_normalize_for_match.rs"]
mod utils_normalize_for_match;
#[path = "utils_placeholder_email.rs"]
mod utils_placeholder_email;
#[path = "utils_recurrence.rs"]
mod utils_recurrence;
#[path = "utils_recurrence_oracle.rs"]
mod utils_recurrence_oracle;
#[path = "utils_request.rs"]
mod utils_request;
#[path = "utils_sanitize_file_name.rs"]
mod utils_sanitize_file_name;
#[path = "utils_spintax.rs"]
mod utils_spintax;
#[path = "utils_stack_discount_percents.rs"]
mod utils_stack_discount_percents;
#[path = "utils_storage.rs"]
mod utils_storage;
#[path = "utils_store_incoming_image.rs"]
mod utils_store_incoming_image;
#[path = "utils_temporal_input.rs"]
mod utils_temporal_input;
#[path = "utils_temporal_storage.rs"]
mod utils_temporal_storage;
#[path = "utils_thread_control_channels.rs"]
mod utils_thread_control_channels;
#[path = "utils_web_url.rs"]
mod utils_web_url;
#[path = "utils_whatsapp_call.rs"]
mod utils_whatsapp_call;
#[path = "utils_whatsapp_call_hours.rs"]
mod utils_whatsapp_call_hours;
#[path = "utils_zod.rs"]
mod utils_zod;
#[path = "variables_assigned_helpers.rs"]
mod variables_assigned_helpers;
#[path = "variables_bot_field_variable.rs"]
mod variables_bot_field_variable;
#[path = "variables_contact_ad_ids.rs"]
mod variables_contact_ad_ids;
#[path = "variables_contact_variable.rs"]
mod variables_contact_variable;
#[path = "variables_deep_replace_contact_variables.rs"]
mod variables_deep_replace_contact_variables;
#[path = "variables_integration_fields_channel_coverage.rs"]
mod variables_integration_fields_channel_coverage;
#[path = "variables_last_call.rs"]
mod variables_last_call;
#[path = "variables_last_input.rs"]
mod variables_last_input;
#[path = "variables_pipeline_variable.rs"]
mod variables_pipeline_variable;
#[path = "variables_queued_messages.rs"]
mod variables_queued_messages;
#[path = "variables_refresh_profile_snapshot.rs"]
mod variables_refresh_profile_snapshot;
#[path = "variables_render_custom_field_value.rs"]
mod variables_render_custom_field_value;
#[path = "variables_replace_custom_field_timezone.rs"]
mod variables_replace_custom_field_timezone;
#[path = "variables_system_field_timezone.rs"]
mod variables_system_field_timezone;
#[path = "variables_system_fields.rs"]
mod variables_system_fields;
#[path = "variables_system_fields_exhaustive.rs"]
mod variables_system_fields_exhaustive;
#[path = "variables_system_fields_helpers.rs"]
mod variables_system_fields_helpers;

// ponytail: one test binary per crate (was one per file: ~200 MB link each,
// the Dell gate bottleneck). Every tests/*.rs file must be listed above.
#[test]
fn every_test_file_is_listed_in_all_rs() {
    let listed = include_str!("all.rs");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    let mut missing: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".rs") && n != "all.rs")
        .filter(|n| !listed.contains(&format!("#[path = \"{n}\"]")))
        .collect();
    missing.sort();
    assert!(missing.is_empty(), "add these to tests/all.rs: {missing:?}");
}
