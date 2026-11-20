//! All integration tests of this crate in one binary (see the guard test).
// Each test file keeps its own copy of its helper modules, as before.
#![allow(clippy::duplicate_mod)]

#[path = "ai_agent_service.rs"]
mod ai_agent_service;
#[path = "ai_file_service.rs"]
mod ai_file_service;
#[path = "ai_function_service.rs"]
mod ai_function_service;
#[path = "ai_handover_service.rs"]
mod ai_handover_service;
#[path = "ai_knowledge_search.rs"]
mod ai_knowledge_search;
#[path = "ai_mcp_server_service.rs"]
mod ai_mcp_server_service;
#[path = "ai_model_catalog.rs"]
mod ai_model_catalog;
#[path = "analytics_campaigns.rs"]
mod analytics_campaigns;
#[path = "analytics_contacts.rs"]
mod analytics_contacts;
#[path = "analytics_events.rs"]
mod analytics_events;
#[path = "analytics_flows.rs"]
mod analytics_flows;
#[path = "analytics_links.rs"]
mod analytics_links;
#[path = "analytics_mac.rs"]
mod analytics_mac;
#[path = "audit_service.rs"]
mod audit_service;
#[path = "automated_response_service.rs"]
mod automated_response_service;
#[path = "billing_service.rs"]
mod billing_service;
#[path = "bot_field_find_many_by_ids.rs"]
mod bot_field_find_many_by_ids;
#[path = "bot_field_unique_violation.rs"]
mod bot_field_unique_violation;
#[path = "bot_field_value_operation.rs"]
mod bot_field_value_operation;
#[path = "broadcast_history.rs"]
mod broadcast_history;
#[path = "broadcast_list_contacts.rs"]
mod broadcast_list_contacts;
#[path = "broadcast_materialize.rs"]
mod broadcast_materialize;
#[path = "broadcast_service.rs"]
mod broadcast_service;
#[path = "capabilities_service.rs"]
mod capabilities_service;
#[path = "channel_integration_settings.rs"]
mod channel_integration_settings;
#[path = "comment_automation_post_lists.rs"]
mod comment_automation_post_lists;
#[path = "connection_credentials.rs"]
mod connection_credentials;
#[path = "connection_oauth_channels.rs"]
mod connection_oauth_channels;
#[path = "connection_oauth_facebook_ads.rs"]
mod connection_oauth_facebook_ads;
#[path = "connection_oauth_google_ads.rs"]
mod connection_oauth_google_ads;
#[path = "connection_oauth_instagram.rs"]
mod connection_oauth_instagram;
#[path = "contact_bulk_tag_stats.rs"]
mod contact_bulk_tag_stats;
#[path = "contact_create_with_inbox.rs"]
mod contact_create_with_inbox;
#[path = "contact_custom_field_apply_operation.rs"]
mod contact_custom_field_apply_operation;
#[path = "contact_custom_field_bot_routing.rs"]
mod contact_custom_field_bot_routing;
#[path = "contact_custom_field_delete.rs"]
mod contact_custom_field_delete;
#[path = "contact_custom_field_list_values_by_keys.rs"]
mod contact_custom_field_list_values_by_keys;
#[path = "contact_custom_field_normalize.rs"]
mod contact_custom_field_normalize;
#[path = "contact_custom_field_set_values.rs"]
mod contact_custom_field_set_values;
#[path = "contact_custom_field_set_values_runtime_coerce.rs"]
mod contact_custom_field_set_values_runtime_coerce;
#[path = "contact_export.rs"]
mod contact_export;
#[path = "contact_filter_ctwa_retarget.rs"]
mod contact_filter_ctwa_retarget;
#[path = "contact_filter_list_filter_fields.rs"]
mod contact_filter_list_filter_fields;
#[path = "contact_filter_pipeline_item_schema.rs"]
mod contact_filter_pipeline_item_schema;
#[path = "contact_filter_query_schema.rs"]
mod contact_filter_query_schema;
#[path = "contact_filter_schema.rs"]
mod contact_filter_schema;
#[path = "contact_filter_value_labels.rs"]
mod contact_filter_value_labels;
#[path = "contact_list.rs"]
mod contact_list;
#[path = "contact_list_by_custom_field_value.rs"]
mod contact_list_by_custom_field_value;
#[path = "contact_note_service.rs"]
mod contact_note_service;
#[path = "contact_phone.rs"]
mod contact_phone;
#[path = "contact_profile_refresh.rs"]
mod contact_profile_refresh;
#[path = "contact_referral_event_history.rs"]
mod contact_referral_event_history;
#[path = "contact_resolve_id_by_identifier.rs"]
mod contact_resolve_id_by_identifier;
#[path = "contact_sequence_service.rs"]
mod contact_sequence_service;
#[path = "contact_sequence_subscribe.rs"]
mod contact_sequence_subscribe;
#[path = "contact_sequence_unsubscribe.rs"]
mod contact_sequence_unsubscribe;
#[path = "contact_service_delete.rs"]
mod contact_service_delete;
#[path = "contact_service_upsert.rs"]
mod contact_service_upsert;
#[path = "contact_update_fields.rs"]
mod contact_update_fields;
#[path = "conversation_contact_inboxes.rs"]
mod conversation_contact_inboxes;
#[path = "copy_name.rs"]
mod copy_name;
#[path = "count_mode.rs"]
mod count_mode;
#[path = "coupon_template_guards.rs"]
mod coupon_template_guards;
#[path = "custom_field_service_update.rs"]
mod custom_field_service_update;
#[path = "decision_connection.rs"]
mod decision_connection;
#[path = "decision_transport_redirect.rs"]
mod decision_transport_redirect;
#[path = "ecommerce_open_hours_is_open.rs"]
mod ecommerce_open_hours_is_open;
#[path = "ecommerce_open_hours_service.rs"]
mod ecommerce_open_hours_service;
#[path = "ecommerce_settings_service.rs"]
mod ecommerce_settings_service;
#[path = "error_log_service.rs"]
mod error_log_service;
#[path = "events_commerce_events.rs"]
mod events_commerce_events;
#[path = "events_commerce_webhook_context.rs"]
mod events_commerce_webhook_context;
#[path = "events_contact_inbox_context.rs"]
mod events_contact_inbox_context;
#[path = "events_evaluator_filtered_cache.rs"]
mod events_evaluator_filtered_cache;
#[path = "events_trigger_emitter_contact_inbox_roundtrip.rs"]
mod events_trigger_emitter_contact_inbox_roundtrip;
#[path = "events_webhook_context.rs"]
mod events_webhook_context;
#[path = "facebook_lead_ads_forms.rs"]
mod facebook_lead_ads_forms;
#[path = "flow_duplicate.rs"]
mod flow_duplicate;
#[path = "flow_filters.rs"]
mod flow_filters;
#[path = "flow_import.rs"]
mod flow_import;
#[path = "flow_service.rs"]
mod flow_service;
#[path = "flow_version_after_publish_if_live.rs"]
mod flow_version_after_publish_if_live;
#[path = "flow_version_service.rs"]
mod flow_version_service;
#[path = "flow_version_service_publish.rs"]
mod flow_version_service_publish;
#[path = "folder_service_create.rs"]
mod folder_service_create;
#[path = "google_ads_consent.rs"]
mod google_ads_consent;
#[path = "google_ads_conversion.rs"]
mod google_ads_conversion;
#[path = "google_ads_hash_user_data.rs"]
mod google_ads_hash_user_data;
#[path = "google_ads_ids.rs"]
mod google_ads_ids;
#[path = "google_ads_processing_status.rs"]
mod google_ads_processing_status;
#[path = "google_ads_sanitize.rs"]
mod google_ads_sanitize;
#[path = "google_ads_scopes.rs"]
mod google_ads_scopes;
#[path = "google_ads_timing.rs"]
mod google_ads_timing;
#[path = "google_oauth_connect.rs"]
mod google_oauth_connect;
#[path = "import_column_map_and_template.rs"]
mod import_column_map_and_template;
#[path = "import_contacts_run.rs"]
mod import_contacts_run;
#[path = "import_create_upload.rs"]
mod import_create_upload;
#[path = "import_peek_headers.rs"]
mod import_peek_headers;
#[path = "import_product_column_map.rs"]
mod import_product_column_map;
#[path = "inbound_webhook_apply_payload.rs"]
mod inbound_webhook_apply_payload;
#[path = "integration_ai_provider_verify.rs"]
mod integration_ai_provider_verify;
#[path = "integration_openai_compatible.rs"]
mod integration_openai_compatible;
#[path = "leaderboard_service.rs"]
mod leaderboard_service;
#[path = "license_refresh.rs"]
mod license_refresh;
#[path = "license_service.rs"]
mod license_service;
#[path = "magic_link_update_empty.rs"]
mod magic_link_update_empty;
#[path = "mcp_oauth_service.rs"]
mod mcp_oauth_service;
#[path = "media_library_service.rs"]
mod media_library_service;
#[path = "message_outgoing_and_list.rs"]
mod message_outgoing_and_list;
#[path = "message_outgoing_media.rs"]
mod message_outgoing_media;
#[path = "meta_catalog_connect.rs"]
mod meta_catalog_connect;
#[path = "meta_catalog_service.rs"]
mod meta_catalog_service;
#[path = "mini_app_definition.rs"]
mod mini_app_definition;
#[path = "mini_app_service.rs"]
mod mini_app_service;
#[path = "mini_app_upload_service.rs"]
mod mini_app_upload_service;
#[path = "minigame_play.rs"]
mod minigame_play;
#[path = "minigame_service.rs"]
mod minigame_service;
#[path = "net_ssrf_guard.rs"]
mod net_ssrf_guard;
#[path = "pipeline_access.rs"]
mod pipeline_access;
#[path = "pipeline_assignees.rs"]
mod pipeline_assignees;
#[path = "pipeline_can_view.rs"]
mod pipeline_can_view;
#[path = "pipeline_currency.rs"]
mod pipeline_currency;
#[path = "pipeline_currency_refresh.rs"]
mod pipeline_currency_refresh;
#[path = "pipeline_flow_actions.rs"]
mod pipeline_flow_actions;
#[path = "pipeline_flow_publish_check.rs"]
mod pipeline_flow_publish_check;
#[path = "pipeline_item_assignees.rs"]
mod pipeline_item_assignees;
#[path = "pipeline_mutation_engine.rs"]
mod pipeline_mutation_engine;
#[path = "pipeline_naming.rs"]
mod pipeline_naming;
#[path = "pipeline_template.rs"]
mod pipeline_template;
#[path = "pipeline_update_lock.rs"]
mod pipeline_update_lock;
#[path = "place_near_me_category_service.rs"]
mod place_near_me_category_service;
#[path = "place_near_me_contact_location.rs"]
mod place_near_me_contact_location;
#[path = "place_near_me_location_service.rs"]
mod place_near_me_location_service;
#[path = "place_near_me_nearest.rs"]
mod place_near_me_nearest;
#[path = "place_near_me_preview_url.rs"]
mod place_near_me_preview_url;
#[path = "platform_credential_service.rs"]
mod platform_credential_service;
#[path = "poll_analytics.rs"]
mod poll_analytics;
#[path = "poll_chat_messages.rs"]
mod poll_chat_messages;
#[path = "poll_service.rs"]
mod poll_service;
#[path = "poll_state.rs"]
mod poll_state;
#[path = "poll_vote.rs"]
mod poll_vote;
#[path = "product_category_service.rs"]
mod product_category_service;
#[path = "product_message_resolve.rs"]
mod product_message_resolve;
#[path = "product_message_service.rs"]
mod product_message_service;
#[path = "product_service.rs"]
mod product_service;
#[path = "qr_code_service.rs"]
mod qr_code_service;
#[path = "questionnaire_service.rs"]
mod questionnaire_service;
#[path = "quota_enforcement_service.rs"]
mod quota_enforcement_service;
#[path = "reflink_service.rs"]
mod reflink_service;
#[path = "response_sla_window.rs"]
mod response_sla_window;
#[path = "sequence_service.rs"]
mod sequence_service;
#[path = "smart_delay_service.rs"]
mod smart_delay_service;
#[path = "tag_service_attach_detach_replace.rs"]
mod tag_service_attach_detach_replace;
#[path = "tag_service_bulk_attach.rs"]
mod tag_service_bulk_attach;
#[path = "tag_service_create.rs"]
mod tag_service_create;
#[path = "tag_service_detach_from_contact.rs"]
mod tag_service_detach_from_contact;
#[path = "tag_service_soft_delete.rs"]
mod tag_service_soft_delete;
#[path = "template_install_intent.rs"]
mod template_install_intent;
#[path = "template_service.rs"]
mod template_service;
#[path = "tenant_service.rs"]
mod tenant_service;
#[path = "thread_control_service.rs"]
mod thread_control_service;
#[path = "trigger_executions.rs"]
mod trigger_executions;
#[path = "trigger_service.rs"]
mod trigger_service;
#[path = "user_quota_plan.rs"]
mod user_quota_plan;
#[path = "user_quota_reconcile.rs"]
mod user_quota_reconcile;
#[path = "vendor_service.rs"]
mod vendor_service;
#[path = "whatsapp_call_announcement_options.rs"]
mod whatsapp_call_announcement_options;
#[path = "whatsapp_call_hours.rs"]
mod whatsapp_call_hours;
#[path = "whatsapp_call_ring_targets.rs"]
mod whatsapp_call_ring_targets;
#[path = "whatsapp_call_voip_call_service.rs"]
mod whatsapp_call_voip_call_service;
#[path = "whatsapp_meta_call_permission.rs"]
mod whatsapp_meta_call_permission;
#[path = "whatsapp_voip_sdp.rs"]
mod whatsapp_voip_sdp;
#[path = "whatsapp_voip_turn_credentials.rs"]
mod whatsapp_voip_turn_credentials;
#[path = "workspace_api_token_credentials.rs"]
mod workspace_api_token_credentials;
#[path = "workspace_deletion_schedule.rs"]
mod workspace_deletion_schedule;
#[path = "workspace_lifecycle.rs"]
mod workspace_lifecycle;
#[path = "workspace_member_service.rs"]
mod workspace_member_service;
#[path = "workspace_purge.rs"]
mod workspace_purge;
#[path = "workspace_service.rs"]
mod workspace_service;
#[path = "x1_services.rs"]
mod x1_services;

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
