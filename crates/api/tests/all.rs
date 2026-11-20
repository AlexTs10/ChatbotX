//! All integration tests of this crate in one binary (see the guard test).
// Each test file keeps its own copy of its helper modules, as before.
#![allow(clippy::duplicate_mod)]

#[path = "admin_workspaces.rs"]
mod admin_workspaces;
#[path = "ai_agents_public_api.rs"]
mod ai_agents_public_api;
#[path = "ai_files_public_api.rs"]
mod ai_files_public_api;
#[path = "ai_functions_mcp_public_api.rs"]
mod ai_functions_mcp_public_api;
#[path = "ai_handover_public_api.rs"]
mod ai_handover_public_api;
#[path = "ai_handover_schema.rs"]
mod ai_handover_schema;
#[path = "ai_mcp_server_schema.rs"]
mod ai_mcp_server_schema;
#[path = "analytics_comment_automation_public_api.rs"]
mod analytics_comment_automation_public_api;
#[path = "analytics_contacts_public_api.rs"]
mod analytics_contacts_public_api;
#[path = "analytics_events_public_api.rs"]
mod analytics_events_public_api;
#[path = "appointments_public_api.rs"]
mod appointments_public_api;
#[path = "auth_authorize_workspace_access.rs"]
mod auth_authorize_workspace_access;
#[path = "auth_token_scope.rs"]
mod auth_token_scope;
#[path = "auth_workspace_token_auth_middleware.rs"]
mod auth_workspace_token_auth_middleware;
#[path = "auth_workspace_token_scope_enforcement.rs"]
mod auth_workspace_token_scope_enforcement;
#[path = "auth_workspace_token_scope_registry.rs"]
mod auth_workspace_token_scope_registry;
#[path = "avatar_proxy_route.rs"]
mod avatar_proxy_route;
#[path = "bot_fields_public_api.rs"]
mod bot_fields_public_api;
#[path = "bot_simulator_public_api.rs"]
mod bot_simulator_public_api;
#[path = "broadcasts_public_api.rs"]
mod broadcasts_public_api;
#[path = "broadcasts_recurrence_public_api.rs"]
mod broadcasts_recurrence_public_api;
#[path = "business_error_defined.rs"]
mod business_error_defined;
#[path = "capabilities_public_api.rs"]
mod capabilities_public_api;
#[path = "channel_integrations_public_api.rs"]
mod channel_integrations_public_api;
#[path = "channel_posts_public_api.rs"]
mod channel_posts_public_api;
#[path = "connections_public_api.rs"]
mod connections_public_api;
#[path = "contact_notes_public_api.rs"]
mod contact_notes_public_api;
#[path = "contacts_ad_referrals_public_api.rs"]
mod contacts_ad_referrals_public_api;
#[path = "contacts_bulk_public_api.rs"]
mod contacts_bulk_public_api;
#[path = "contacts_crud_public_api.rs"]
mod contacts_crud_public_api;
#[path = "contacts_custom_fields_public_api.rs"]
mod contacts_custom_fields_public_api;
#[path = "contacts_export_public_api.rs"]
mod contacts_export_public_api;
#[path = "contacts_filter_fields_public_api.rs"]
mod contacts_filter_fields_public_api;
#[path = "contacts_import_public_api.rs"]
mod contacts_import_public_api;
#[path = "contacts_list_public_api.rs"]
mod contacts_list_public_api;
#[path = "contacts_messages_public_api.rs"]
mod contacts_messages_public_api;
#[path = "contacts_refresh_profile_public_api.rs"]
mod contacts_refresh_profile_public_api;
#[path = "conversations_public_api.rs"]
mod conversations_public_api;
#[path = "conversations_thread_control_public_api.rs"]
mod conversations_thread_control_public_api;
#[path = "coupons_discount_codes_long_ids.rs"]
mod coupons_discount_codes_long_ids;
#[path = "custom_fields_public_api.rs"]
mod custom_fields_public_api;
#[path = "decisions_public_api.rs"]
mod decisions_public_api;
#[path = "dynamic_images_public_api.rs"]
mod dynamic_images_public_api;
#[path = "ecommerce_open_hours_public_api.rs"]
mod ecommerce_open_hours_public_api;
#[path = "email_topics_public_api.rs"]
mod email_topics_public_api;
#[path = "error_logs_public_api.rs"]
mod error_logs_public_api;
#[path = "external_webhooks_public_api.rs"]
mod external_webhooks_public_api;
#[path = "files_route.rs"]
mod files_route;
#[path = "flows_crud_public_api.rs"]
mod flows_crud_public_api;
#[path = "flows_import_public_api.rs"]
mod flows_import_public_api;
#[path = "flows_public_api.rs"]
mod flows_public_api;
#[path = "flows_versions_public_api.rs"]
mod flows_versions_public_api;
#[path = "folders_public_api.rs"]
mod folders_public_api;
#[path = "health.rs"]
mod health;
#[path = "idempotency.rs"]
mod idempotency;
#[path = "import_upload_url_public_api.rs"]
mod import_upload_url_public_api;
#[path = "inbound_webhook_intake_drain.rs"]
mod inbound_webhook_intake_drain;
#[path = "inbound_webhooks_public_api.rs"]
mod inbound_webhooks_public_api;
#[path = "inbox_teams_public_api.rs"]
mod inbox_teams_public_api;
#[path = "inboxes_public_api.rs"]
mod inboxes_public_api;
#[path = "integrations_ai_public_api.rs"]
mod integrations_ai_public_api;
#[path = "integrations_public_api.rs"]
mod integrations_public_api;
#[path = "jobs_admin_page.rs"]
mod jobs_admin_page;
#[path = "keywords_public_api.rs"]
mod keywords_public_api;
#[path = "magic_link_redirect.rs"]
mod magic_link_redirect;
#[path = "magic_links_public_api.rs"]
mod magic_links_public_api;
#[path = "media_library_public_api.rs"]
mod media_library_public_api;
#[path = "messages_public_api.rs"]
mod messages_public_api;
#[path = "messenger_menus_public_api.rs"]
mod messenger_menus_public_api;
#[path = "messenger_templates_public_api.rs"]
mod messenger_templates_public_api;
#[path = "meta_catalog_public_api.rs"]
mod meta_catalog_public_api;
#[path = "mini_apps_public_api.rs"]
mod mini_apps_public_api;
#[path = "mini_apps_whatsapp_public_api.rs"]
mod mini_apps_whatsapp_public_api;
#[path = "minigames_public_api.rs"]
mod minigames_public_api;
#[path = "mobile_member_scope.rs"]
mod mobile_member_scope;
#[path = "orpc_default_error_code.rs"]
mod orpc_default_error_code;
#[path = "p5_fixd_public_api.rs"]
mod p5_fixd_public_api;
#[path = "pipelines_activity_public_api.rs"]
mod pipelines_activity_public_api;
#[path = "pipelines_items_public_api.rs"]
mod pipelines_items_public_api;
#[path = "pipelines_media_scope.rs"]
mod pipelines_media_scope;
#[path = "pipelines_members_public_api.rs"]
mod pipelines_members_public_api;
#[path = "pipelines_public_api.rs"]
mod pipelines_public_api;
#[path = "pipelines_query_public_api.rs"]
mod pipelines_query_public_api;
#[path = "pipelines_side_effects.rs"]
mod pipelines_side_effects;
#[path = "pipelines_stats_public_api.rs"]
mod pipelines_stats_public_api;
#[path = "places_near_me_public_api.rs"]
mod places_near_me_public_api;
#[path = "platform_admins.rs"]
mod platform_admins;
#[path = "platform_settings.rs"]
mod platform_settings;
#[path = "polls_public_api.rs"]
mod polls_public_api;
#[path = "presigned_upload_route.rs"]
mod presigned_upload_route;
#[path = "product_catalog_public_api.rs"]
mod product_catalog_public_api;
#[path = "product_imports_public_api.rs"]
mod product_imports_public_api;
#[path = "qr_reflinks_public_api.rs"]
mod qr_reflinks_public_api;
#[path = "questionnaires_public_api.rs"]
mod questionnaires_public_api;
#[path = "reflink_chat_widget.rs"]
mod reflink_chat_widget;
#[path = "route_coverage.rs"]
mod route_coverage;
#[path = "routes_channel_and_mobile_auth.rs"]
mod routes_channel_and_mobile_auth;
#[path = "routes_public_scope.rs"]
mod routes_public_scope;
#[path = "rpc_automation_pages.rs"]
mod rpc_automation_pages;
#[path = "rpc_channel_connect.rs"]
mod rpc_channel_connect;
#[path = "rpc_channel_token_profile_actions.rs"]
mod rpc_channel_token_profile_actions;
#[path = "rpc_channels.rs"]
mod rpc_channels;
#[path = "rpc_google_ads_settings.rs"]
mod rpc_google_ads_settings;
#[path = "rpc_help_items.rs"]
mod rpc_help_items;
#[path = "rpc_inbox_teams.rs"]
mod rpc_inbox_teams;
#[path = "rpc_leaderboard_actions.rs"]
mod rpc_leaderboard_actions;
#[path = "rpc_platform_credentials.rs"]
mod rpc_platform_credentials;
#[path = "rpc_procedures.rs"]
mod rpc_procedures;
#[path = "rpc_screen_actions.rs"]
mod rpc_screen_actions;
#[path = "rpc_session_auth.rs"]
mod rpc_session_auth;
#[path = "rpc_settings_integrations.rs"]
mod rpc_settings_integrations;
#[path = "rpc_spa_procedures.rs"]
mod rpc_spa_procedures;
#[path = "rpc_template_actions.rs"]
mod rpc_template_actions;
#[path = "rpc_template_pages.rs"]
mod rpc_template_pages;
#[path = "rpc_test_external_request_action.rs"]
mod rpc_test_external_request_action;
#[path = "rpc_workspace.rs"]
mod rpc_workspace;
#[path = "rpc_workspace_members.rs"]
mod rpc_workspace_members;
#[path = "saved_replies_sla_public_api.rs"]
mod saved_replies_sla_public_api;
#[path = "sequences_create_public_api.rs"]
mod sequences_create_public_api;
#[path = "sequences_public_api.rs"]
mod sequences_public_api;
#[path = "session_account.rs"]
mod session_account;
#[path = "session_auth_rate_limit.rs"]
mod session_auth_rate_limit;
#[path = "session_bearer_plugin.rs"]
mod session_bearer_plugin;
#[path = "session_claim_unverified_account.rs"]
mod session_claim_unverified_account;
#[path = "session_generic_oauth_config.rs"]
mod session_generic_oauth_config;
#[path = "session_generic_oauth_path.rs"]
mod session_generic_oauth_path;
#[path = "session_invitation_start.rs"]
mod session_invitation_start;
#[path = "session_magic_link_and_reset.rs"]
mod session_magic_link_and_reset;
#[path = "session_mcp_default_resource.rs"]
mod session_mcp_default_resource;
#[path = "session_mcp_oauth.rs"]
mod session_mcp_oauth;
#[path = "session_mcp_resource.rs"]
mod session_mcp_resource;
#[path = "session_mobile_auth.rs"]
mod session_mobile_auth;
#[path = "session_mobile_resource.rs"]
mod session_mobile_resource;
#[path = "session_oauth_sign_in.rs"]
mod session_oauth_sign_in;
#[path = "session_schema_parity.rs"]
mod session_schema_parity;
#[path = "session_send_magic_link.rs"]
mod session_send_magic_link;
#[path = "session_signup_gate.rs"]
mod session_signup_gate;
#[path = "session_social_redirect_origin.rs"]
mod session_social_redirect_origin;
#[path = "session_trusted_origins_build_phase.rs"]
mod session_trusted_origins_build_phase;
#[path = "spa_default_template_dispatch.rs"]
mod spa_default_template_dispatch;
#[path = "spa_pages.rs"]
mod spa_pages;
#[path = "spa_public_pages.rs"]
mod spa_public_pages;
#[path = "spreadsheets_public_api.rs"]
mod spreadsheets_public_api;
#[path = "tags_public_api.rs"]
mod tags_public_api;
#[path = "templates_install_api.rs"]
mod templates_install_api;
#[path = "templates_public_api.rs"]
mod templates_public_api;
#[path = "templates_save_api.rs"]
mod templates_save_api;
#[path = "triggers_public_api.rs"]
mod triggers_public_api;
#[path = "webchats_public_api.rs"]
mod webchats_public_api;
#[path = "webhooks_public_api.rs"]
mod webhooks_public_api;
#[path = "whatsapp_calling_actions.rs"]
mod whatsapp_calling_actions;
#[path = "whatsapp_calling_public_api.rs"]
mod whatsapp_calling_public_api;
#[path = "whatsapp_flows_public_api.rs"]
mod whatsapp_flows_public_api;
#[path = "whatsapp_templates_public_api.rs"]
mod whatsapp_templates_public_api;
#[path = "working_hours_public_api.rs"]
mod working_hours_public_api;
#[path = "workspace_settings_public_api.rs"]
mod workspace_settings_public_api;
#[path = "x1_wiring.rs"]
mod x1_wiring;

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
