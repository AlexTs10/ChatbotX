//! Port of the route-scope parts of `apps/builder/__tests__/*-public-scope.test.ts`.
//!
//! The original tests import each public router module with a mocked
//! `workspaceTokenAuthAPIForScope` and record its scope argument, or call a
//! real procedure with a stubbed token. Here the facts live in the route table
//! (`chatbotx_api::routes`), so the tests check the table and send requests
//! through `handle_public_request` with stubbed services. "Passes the real
//! route" means "passes auth" (the handlers are ported with their domains).

mod support;

use chatbotx_api::routes::{PUBLIC_ROUTES, scopes_of_module};
use support::{MockDeps, auth_result, call_op, op_scopes};

fn token(scopes: Option<&[&str]>, permission: &str) -> MockDeps {
    MockDeps::with_lookup(Some(auth_result(scopes, permission)))
}

fn scope_message(scope: &str) -> String {
    format!("Token is not authorized for the '{scope}' scope")
}

/// A token with `scopes` is refused operation `id` with FORBIDDEN for `scope`.
async fn assert_denied(id: &str, scopes: &[&str], scope: &str) {
    let answer = call_op(&token(Some(scopes), "full"), id).await;
    assert_eq!(answer.status, 403, "{id}: {}", answer.body);
    assert_eq!(answer.code(), "FORBIDDEN", "{id}");
    assert_eq!(answer.message(), scope_message(scope), "{id}");
}

/// A token with `scopes` (`None` = unrestricted) passes the auth of `id`.
async fn assert_passes(id: &str, scopes: Option<&[&str]>, permission: &str) {
    let answer = call_op(&token(scopes, permission), id).await;
    assert!(
        answer.passed_auth(),
        "{id}: {} {}",
        answer.status,
        answer.body
    );
}

/// A read-only token is refused operation `id`.
async fn assert_read_only_denied(id: &str, scopes: Option<&[&str]>) {
    let answer = call_op(&token(scopes, "read_only"), id).await;
    assert_eq!(answer.status, 403, "{id}: {}", answer.body);
    assert_eq!(answer.code(), "FORBIDDEN", "{id}");
    assert_eq!(
        answer.message(),
        "Read-only token cannot perform this operation",
        "{id}"
    );
}

/// The operations of router key `prefix` (e.g. `broadcasts` -> `broadcasts.*`).
fn ops_of(prefix: &str) -> Vec<&'static str> {
    let dotted = format!("{prefix}.");
    PUBLIC_ROUTES
        .iter()
        .filter(|route| route.id.starts_with(&dotted))
        .map(|route| route.id)
        .collect()
}

/// Every operation of router key `prefix` is refused to a contacts-only token.
async fn assert_router_denied_to_contacts(prefix: &str, scope: &str) {
    let ops = ops_of(prefix);
    assert!(!ops.is_empty(), "{prefix} has operations");
    for id in ops {
        assert_denied(id, &["contacts"], scope).await;
    }
}

fn module_scopes(module: &str) -> Vec<&'static str> {
    scopes_of_module(module)
}

mod ads {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_ads_conversion_rules_route_with_forbidden()
     {
        assert_denied("ads.listRules", &["contacts"], "ads").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_ads_conversion_rules_route() {
        assert_passes("ads.listRules", None, "full").await;
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_post_v1_ads_campaigns_route_with_forbidden()
    {
        assert_denied("ads.createCampaign", &["contacts"], "ads").await;
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_post_v1_ads_retarget_audiences_route_with_forbidden()
     {
        assert_denied("ads.startRetargetAudienceSync", &["contacts"], "ads").await;
    }

    #[test]
    fn the_router_was_built_entirely_under_workspacetokenauthapiforscope_ads() {
        let ops = ops_of("ads");
        for key in [
            "listRules",
            "getRule",
            "createRule",
            "updateRule",
            "deleteRule",
            "getFunnel",
            "getFunnelTimeseries",
            "getCapiDelivery",
            "listConversionExportRows",
            "listChannelAdAccounts",
            "getAnalyticsOverview",
            "getAnalyticsTimeseries",
            "getConversion",
            "listCustomAudiences",
            "startRetargetAudienceSync",
            "createCampaign",
            "retryCampaign",
            "publishCampaign",
            "pauseCampaign",
            "deleteCampaign",
            "listCampaigns",
            "getCampaignsInsights",
            "listCampaignAdAccounts",
            "getCampaignAdAccountDetails",
            "uploadCampaignVideo",
            "getCampaignVideoStatus",
            "listCampaignMessengerPages",
            "checkCampaignPrerequisites",
            "listConnections",
            "disconnectConnection",
        ] {
            assert!(ops.contains(&format!("ads.{key}").as_str()), "ads.{key}");
        }
        for id in ops {
            assert_eq!(op_scopes(id), ["ads"], "{id}");
        }
    }
}

mod analytics {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_analytics_contacts_count_route_with_forbidden()
     {
        assert_denied("analytics.contactsCount", &["contacts"], "analytics").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_analytics_contacts_count_route() {
        assert_passes("analytics.contactsCount", None, "full").await;
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_analytics_agent_response_series_route_with_forbidden()
     {
        assert_denied("analytics.agentResponseSeries", &["contacts"], "analytics").await;
    }

    #[tokio::test]
    async fn an_analytics_scoped_token_passes_the_real_get_v1_analytics_agent_response_series_route()
     {
        assert_passes(
            "analytics.agentResponseSeries",
            Some(&["analytics"]),
            "full",
        )
        .await;
    }
}

mod appointments {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_appointment_calendars_route_with_forbidden()
     {
        assert_denied("appointmentCalendars.list", &["contacts"], "appointments").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_appointment_calendars_route() {
        assert_passes("appointmentCalendars.list", None, "full").await;
    }

    #[tokio::test]
    async fn an_appointments_scoped_token_passes_the_real_get_v1_appointment_calendars_route() {
        assert_passes("appointmentCalendars.list", Some(&["appointments"]), "full").await;
    }

    #[tokio::test]
    async fn an_inbox_only_token_is_denied_the_appointments_route() {
        assert_denied("appointments.createBookingLink", &["inbox"], "appointments").await;
    }
}

mod automation {
    use super::*;

    const ROUTERS: &[&str] = &[
        "fbComments",
        "igComments",
        "igStories",
        "qrCodes",
        "questionnaires",
        "spreadsheets",
        "facebookLeadAds",
        "threadsComments",
        "tiktokComments",
        "magicLinks",
        "commentAutomations",
    ];

    #[test]
    fn exports_at_least_one_procedure() {
        for router in ROUTERS {
            assert!(!ops_of(router).is_empty(), "{router}");
        }
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_s_with_forbidden() {
        for router in ROUTERS {
            assert_router_denied_to_contacts(router, "automation").await;
        }
    }

    #[tokio::test]
    async fn an_automation_scoped_token_passes_get_v1_qr_codes() {
        assert_passes("qrCodes.list", Some(&["automation"]), "full").await;
    }
}

mod broadcasts {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_broadcasts_route_with_forbidden() {
        assert_denied("broadcasts.list", &["contacts"], "broadcasts").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_broadcasts_route() {
        assert_passes("broadcasts.list", None, "full").await;
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_s_with_forbidden() {
        assert_router_denied_to_contacts("broadcasts", "broadcasts").await;
    }

    #[tokio::test]
    async fn a_read_only_token_is_denied_post_v1_broadcasts_before_any_service_call() {
        assert_read_only_denied("broadcasts.create", None).await;
    }
}

mod channels_and_integrations {
    use super::*;

    #[tokio::test]
    async fn denies_the_integrations_route_to_a_token_without_integrations() {
        assert_denied("integrations.list", &["channels"], "integrations").await;
    }

    #[tokio::test]
    async fn allows_the_integrations_route_to_an_integrations_scoped_token() {
        assert_passes("integrations.list", Some(&["integrations"]), "full").await;
    }

    #[tokio::test]
    async fn denies_a_channel_route_to_a_token_without_channels() {
        assert_denied("webchats.list", &["integrations"], "channels").await;
    }

    #[tokio::test]
    async fn allows_a_channel_route_with_a_channels_scoped_token() {
        assert_passes("webchats.list", Some(&["channels"]), "full").await;
    }

    #[tokio::test]
    async fn keeps_unrestricted_tokens_authorized() {
        assert_passes("integrations.list", None, "full").await;
        assert_passes("webchats.list", None, "full").await;
    }
}

mod channels {
    use super::*;

    fn channels_module(feature: &str) -> Vec<&'static str> {
        module_scopes(&format!("features/{feature}/api/public"))
    }

    #[test]
    fn user_persistent_menus_api_public_ts_registers_under_the_channels_scope() {
        assert_eq!(channels_module("user-persistent-menus"), ["channels"]);
    }

    #[test]
    fn integration_webchat_api_public_ts_registers_under_the_channels_scope() {
        assert_eq!(channels_module("integration-webchat"), ["channels"]);
    }

    #[test]
    fn integration_smtp_api_public_ts_registers_under_the_channels_scope() {
        assert_eq!(channels_module("integration-smtp"), ["channels"]);
    }

    #[test]
    fn personas_api_public_ts_registers_under_the_channels_scope() {
        assert_eq!(channels_module("personas"), ["channels"]);
    }

    #[test]
    fn channel_integrations_api_public_ts_registers_under_the_channels_scope() {
        assert_eq!(channels_module("channel-integrations"), ["channels"]);
    }

    #[test]
    fn integration_messenger_api_public_ts_registers_under_the_channels_scope() {
        assert_eq!(channels_module("integration-messenger"), ["channels"]);
    }

    #[test]
    fn integration_zalo_api_public_ts_registers_under_the_channels_scope() {
        assert_eq!(channels_module("integration-zalo"), ["channels"]);
    }

    #[test]
    fn integration_instagram_api_public_ts_registers_under_the_channels_scope() {
        assert_eq!(channels_module("integration-instagram"), ["channels"]);
    }

    #[test]
    fn integration_tiktok_api_public_ts_registers_under_the_channels_scope() {
        assert_eq!(channels_module("integration-tiktok"), ["channels"]);
    }

    #[test]
    fn bot_simulator_api_public_ts_registers_under_the_channels_scope() {
        assert_eq!(channels_module("bot-simulator"), ["channels"]);
    }
}

mod contacts {
    use super::*;

    #[test]
    fn crud_ts_registers_under_the_contacts_scope() {
        assert_eq!(
            module_scopes("features/contacts/api/public/crud"),
            ["contacts"]
        );
    }

    #[test]
    fn tags_and_fields_ts_registers_under_the_contacts_scope() {
        assert_eq!(
            module_scopes("features/contacts/api/public/tags"),
            ["contacts"]
        );
        assert_eq!(
            module_scopes("features/contacts/api/public/custom-fields"),
            ["contacts"]
        );
    }

    #[test]
    fn notes_ts_registers_under_the_contacts_scope() {
        assert_eq!(
            module_scopes("features/contact-notes/api/public"),
            ["contacts"]
        );
    }

    #[test]
    fn sequences_ts_registers_under_the_contacts_scope() {
        assert_eq!(
            module_scopes("features/contact-sequences/api/public"),
            ["contacts"]
        );
    }

    #[test]
    fn inboxes_ts_registers_under_the_contacts_scope() {
        assert_eq!(
            module_scopes("features/contact-inboxes/api/public"),
            ["contacts"]
        );
    }

    #[test]
    fn export_files_ts_registers_under_the_contacts_scope() {
        assert_eq!(
            module_scopes("features/contacts/api/public/export"),
            ["contacts"]
        );
    }

    #[test]
    fn filter_fields_ts_registers_under_the_contacts_scope() {
        assert_eq!(
            module_scopes("features/contact-filter/api/public"),
            ["contacts"]
        );
    }

    #[test]
    fn bulk_ts_registers_under_the_contacts_scope() {
        assert_eq!(
            module_scopes("features/contacts/api/public/bulk"),
            ["contacts"]
        );
    }

    #[test]
    fn export_ts_registers_under_the_contacts_scope() {
        // The original sees no new scope call (the module was imported
        // before); its routes are the export routes, under `contacts`.
        assert_eq!(
            module_scopes("features/contacts/api/public/export"),
            ["contacts"]
        );
    }

    #[test]
    fn refresh_profile_ts_registers_under_the_contacts_scope() {
        assert_eq!(
            module_scopes("features/contacts/api/public/refresh-profile"),
            ["contacts"]
        );
    }

    #[test]
    fn messages_ts_registers_under_both_the_inbox_and_automation_scopes_never_contacts() {
        let scopes = module_scopes("features/contacts/api/public/messages");
        assert!(scopes.contains(&"inbox"), "{scopes:?}");
        assert!(scopes.contains(&"automation"), "{scopes:?}");
        assert!(!scopes.contains(&"contacts"), "{scopes:?}");
    }

    #[test]
    fn import_public_router_registers_under_the_contacts_scope() {
        assert_eq!(module_scopes("features/import/api/public"), ["contacts"]);
    }

    #[test]
    fn contact_scan_public_router_registers_under_the_contacts_scope() {
        assert_eq!(
            module_scopes("features/contact-scan/api/public"),
            ["contacts"]
        );
    }

    #[test]
    fn folders_public_router_registers_under_the_contacts_scope() {
        assert_eq!(module_scopes("features/folders/api/public"), ["contacts"]);
    }
}

mod conversations {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_conversations_id_route_with_forbidden()
     {
        assert_denied("conversations.get", &["contacts"], "inbox").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_conversations_id_route() {
        assert_passes("conversations.get", None, "full").await;
    }

    #[tokio::test]
    async fn an_inbox_scoped_token_passes_the_real_get_v1_conversations_id_route() {
        assert_passes("conversations.get", Some(&["inbox"]), "full").await;
    }
}

mod coupons {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_coupon_topics_route_with_forbidden()
    {
        assert_denied("coupons.listTopics", &["contacts"], "ecommerce").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_coupon_topics_route() {
        assert_passes("coupons.listTopics", None, "full").await;
    }
}

mod ecommerce {
    use super::*;

    #[tokio::test]
    async fn s_denies_a_contacts_scoped_token() {
        for id in [
            "orders.list",
            "productVendors.list",
            "ecommerceSettings.get",
            "openHours.list",
            "webshop.getLink",
            "ecommerceAnalytics.get",
            "discountCodes.list",
            "carts.get",
        ] {
            assert_denied(id, &["contacts"], "ecommerce").await;
        }
    }

    #[tokio::test]
    async fn an_ecommerce_scoped_token_lists_orders_of_its_own_workspace() {
        assert_passes("orders.list", Some(&["ecommerce"]), "full").await;
    }
}

mod email_topics {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_email_topics_route_with_forbidden() {
        assert_denied("emailTopics.list", &["contacts"], "broadcasts").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_email_topics_route() {
        assert_passes("emailTopics.list", None, "full").await;
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_s_with_forbidden() {
        assert_router_denied_to_contacts("emailTopics", "broadcasts").await;
    }

    #[tokio::test]
    async fn a_read_only_token_is_denied_post_v1_email_topics_before_any_service_call() {
        assert_read_only_denied("emailTopics.create", None).await;
    }
}

mod flows {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_get_v1_flows_id_nodes_with_forbidden() {
        assert_denied("flows.listNodes", &["contacts"], "automation").await;
    }

    #[tokio::test]
    async fn an_automation_scoped_token_can_list_the_nodes() {
        assert_passes("flows.listNodes", Some(&["automation"]), "full").await;
    }

    #[tokio::test]
    async fn an_unrestricted_token_null_scopes_can_list_the_nodes() {
        assert_passes("flows.listNodes", None, "full").await;
    }
}

mod integrations {
    use super::*;

    #[test]
    fn crud_ts_registers_under_the_integrations_scope() {
        assert_eq!(
            module_scopes("features/integrations/api/public/crud"),
            ["integrations"]
        );
    }

    #[test]
    fn ai_ts_registers_under_the_integrations_scope() {
        assert_eq!(
            module_scopes("features/integrations/api/public/ai"),
            ["integrations"]
        );
    }

    #[test]
    fn ai_hand_over_routes_register_under_the_integrations_scope() {
        assert_eq!(
            module_scopes("features/integration-ai-handover/api/public"),
            ["integrations"]
        );
    }
}

mod media {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_media_library_files_route_with_forbidden()
     {
        assert_denied("mediaLibrary.listFiles", &["contacts"], "media").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_media_library_files_route() {
        assert_passes("mediaLibrary.listFiles", None, "full").await;
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_post_v1_media_library_files_upload_url_route_with_forbidden()
     {
        assert_denied("mediaLibrary.createUploadUrl", &["contacts"], "media").await;
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_dynamic_images_route_with_forbidden()
    {
        assert_denied("dynamicImages.list", &["contacts"], "media").await;
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_post_v1_dynamic_images_route_with_forbidden()
     {
        assert_denied("dynamicImages.create", &["contacts"], "media").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_dynamic_images_route() {
        assert_passes("dynamicImages.list", None, "full").await;
    }

    #[test]
    fn the_media_library_router_was_built_entirely_under_workspacetokenauthapiforscope_media() {
        let ops = ops_of("mediaLibrary");
        assert!(!ops.is_empty());
        for id in ops {
            assert_eq!(op_scopes(id), ["media"], "{id}");
        }
    }

    #[test]
    fn the_dynamic_images_router_was_built_entirely_under_workspacetokenauthapiforscope_media() {
        let ops = ops_of("dynamicImages");
        assert!(!ops.is_empty());
        for id in ops {
            assert_eq!(op_scopes(id), ["media"], "{id}");
        }
    }
}

mod minigames {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_minigames_route_with_forbidden() {
        assert_denied("minigames.list", &["contacts"], "minigames").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_minigames_route() {
        assert_passes("minigames.list", None, "full").await;
    }

    #[tokio::test]
    async fn a_read_only_token_is_denied_s_before_any_service_call() {
        for id in [
            "minigames.create",
            "minigames.update",
            "minigames.updatePartial",
            "minigames.delete",
            "minigames.deleteMany",
        ] {
            assert_read_only_denied(id, None).await;
        }
    }
}

mod pipelines {
    use super::*;

    #[tokio::test]
    async fn every_pipelines_procedure_is_denied_to_a_token_without_the_pipelines_scope() {
        assert_router_denied_to_contacts("pipelines", "pipelines").await;
    }

    #[tokio::test]
    async fn a_pipelines_scoped_token_reaches_the_service_with_the_workspace_token_scope() {
        assert_passes("pipelines.list", Some(&["pipelines"]), "full").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_pass() {
        assert_passes("pipelines.list", None, "full").await;
    }

    #[tokio::test]
    async fn a_read_only_token_cannot_move_an_item() {
        assert_read_only_denied("pipelines.moveItem", Some(&["pipelines"])).await;
    }

    #[tokio::test]
    async fn a_read_only_token_can_still_read_the_board_s_items() {
        assert_passes("pipelines.listItems", Some(&["pipelines"]), "read_only").await;
    }
}

mod product_categories {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_product_categories_route_with_forbidden()
     {
        assert_denied("productCategories.list", &["contacts"], "ecommerce").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_product_categories_route() {
        assert_passes("productCategories.list", None, "full").await;
    }
}

mod products {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_products_route_with_forbidden() {
        assert_denied("products.list", &["contacts"], "ecommerce").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_products_route() {
        assert_passes("products.list", None, "full").await;
    }
}

mod sequences {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_the_real_get_v1_sequences_route_with_forbidden() {
        assert_denied("sequences.list", &["contacts"], "broadcasts").await;
    }

    #[tokio::test]
    async fn null_scopes_unrestricted_passes_the_real_get_v1_sequences_route() {
        assert_passes("sequences.list", None, "full").await;
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_s_with_forbidden() {
        assert_router_denied_to_contacts("sequences", "broadcasts").await;
    }

    #[tokio::test]
    async fn a_read_only_token_is_denied_post_v1_sequences_before_any_service_call() {
        assert_read_only_denied("sequences.create", None).await;
    }
}

mod templates {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_get_v1_templates_with_forbidden() {
        assert_denied("templates.list", &["contacts"], "templates").await;
    }

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_post_v1_template_installations() {
        assert_denied("templateInstallations.install", &["contacts"], "templates").await;
    }

    #[tokio::test]
    async fn a_read_only_token_cannot_save_a_template() {
        assert_read_only_denied("templates.create", None).await;
    }
}

mod working_hours {
    use super::*;

    #[tokio::test]
    async fn a_contacts_scoped_token_is_denied_get_v1_working_hours_with_forbidden() {
        assert_denied("workingHours.list", &["contacts"], "inbox").await;
    }

    #[tokio::test]
    async fn an_inbox_scoped_token_passes_the_real_list_route() {
        assert_passes("workingHours.list", Some(&["inbox"]), "full").await;
    }

    #[tokio::test]
    async fn a_read_only_token_cannot_delete_a_schedule() {
        assert_read_only_denied("workingHours.delete", Some(&["inbox"])).await;
    }
}
