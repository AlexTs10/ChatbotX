//! P3.3 implementer smoke test (not a ported test): calls every repository function
//! that has no ported test once on a migrated SQLite file, so a SQL typo (wrong column,
//! wrong table) fails here and not first in production.

mod common;
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::{self, Result};
use chatbotx_db::rusqlite::types::Value;
use common::{fresh_db, seed};
use serde_json::json;

const WS: i64 = 2001;
const USER: i64 = 1001;
const NOW: i64 = 1_780_272_000_000;

fn all(conn: &chatbotx_db::rusqlite::Connection) -> Result<()> {
    use chatbotx_db::repo::*;
    // first slice extras
    auth_account::find_by_user_and_provider(conn, USER, "google")?;
    invitation::create(
        conn,
        &invitation::NewInvitation {
            code: "c".into(),
            permissions: Json(json!({})),
            expires_at: NOW,
            workspace_id: Some(WS),
            invited_by: USER,
        },
    )?;
    invitation::find_by_code(conn, "c")?;
    license::upsert(conn, "lic", "tok", NOW)?;
    license::upsert(conn, "lic", "tok2", NOW)?;
    license::find_token_by_license_id(conn, "lic")?;
    mobile_refresh_token::revoke_all_for_user(conn, USER, NOW)?;
    mobile_refresh_token::delete_expired_for_user(conn, USER, NOW)?;
    mobile_refresh_token::delete_expired(conn, NOW)?;
    oauth_access_token::delete_by_user_and_client(conn, USER, "client-1")?;
    oauth_access_token::delete_expired(conn, NOW)?;
    oauth_refresh_token::delete_by_user_and_client(conn, USER, "client-1")?;
    oauth_refresh_token::delete_expired(conn, NOW)?;
    oauth_client::find_by_client_id(conn, "client-1")?;
    oauth_consent::find_latest_given_by_user_and_client(conn, USER, "client-1")?;
    oauth_consent::set_workspace_by_user_and_client(conn, USER, "client-1", WS)?;
    oauth_consent::delete_by_user_and_client(conn, USER, "client-1")?;
    platform_admin::list_with_user(conn, PaginationInput::default())?;
    tenant::find_by_id(conn, 1)?;
    tenant::find_by_owner(conn, USER)?;
    tenant::find_with_template(conn, 1)?;
    tenant::delete_default_template(conn, 1)?;
    workspace::lock_first_workspace(conn, USER)?;
    workspace::find_by_id_for_update(conn, WS)?;
    workspace::find_active_by_owner(conn, USER)?;
    workspace_api_token::lock_workspace_tokens(conn, WS)?;
    // slice 2 extras
    reflink::find_widget_by_id_and_workspace(conn, WS, 1)?;
    reflink::list_ids_by_flow_ids(conn, WS, &[1])?;
    reflink::find_by_id(conn, 1)?;
    whatsapp_business_account::mark_provisioned(conn, WS, "w", 1, Some("cl"))?;
    whatsapp_business_account::delete_if_orphaned(conn, WS, "w")?;
    messaging_ad_operation::find_by_id_for_workspace(conn, 1, WS)?;
    messaging_ad_operation::list_by_workspace_id(
        conn,
        &messaging_ad_operation::ListOperationsInput {
            workspace_id: WS,
            channel: Some("whatsapp".into()),
            ..Default::default()
        },
    )?;
    messaging_ad_operation::claim_for_retry(conn, 1, WS)?;
    messaging_ad_operation::update_publish_state(conn, 1, WS, "paused", None)?;
    messaging_ad_operation::set_cleanup_error(conn, 1, WS, Some("x"))?;
    messenger_welcome_message_flow::set_flow_version(conn, 1, 2)?;
    media_library_file::find_by_ids(conn, WS, &[1])?;
    media_library_file::count_by_folder(conn, WS)?;
    media_library_file::list_by_folder(conn, WS, 1)?;
    let f = media_library_file::create(
        conn,
        &media_library_file::NewMediaLibraryFile {
            name: "n".into(),
            path: "p".into(),
            mime_type: "image/png".into(),
            size: 1,
            workspace_id: WS,
            ..Default::default()
        },
    )?;
    media_library_file::move_to_folder(conn, WS, &[f.id], None)?;
    media_library_file::touch_last_accessed_at(conn, WS, f.id)?;
    media_library_file::list(
        conn,
        &media_library_file::ListFilesInput {
            workspace_id: WS,
            filter: Some("recent".into()),
            search: Some("n".into()),
            file_type: Some("image".into()),
            per_page: 10,
            ..Default::default()
        },
    )?;
    media_library_file::delete_by_folder(conn, WS, 1)?;
    sequence_dispatch::cancel_pending_batch_for_workspace(conn, WS, 10)?;
    for kind in template_selectable_resource::CATEGORIES {
        template_selectable_resource::list(
            conn,
            kind,
            &template_selectable_resource::CategoryInput {
                workspace_id: WS,
                keyword: Some("a".into()),
                limit: 10,
                after: Some(template_selectable_resource::KeysetCursor {
                    key: "a".into(),
                    id: "1".into(),
                }),
                ..Default::default()
            },
        )?;
    }
    template_selectable_resource::list_keywords(
        conn,
        &template_selectable_resource::CategoryInput {
            workspace_id: WS,
            keyword: Some("a".into()),
            limit: 10,
            ..Default::default()
        },
    )?;
    template_selectable_resource::list_keywords(
        conn,
        &template_selectable_resource::CategoryInput {
            workspace_id: WS,
            limit: 10,
            after: Some(template_selectable_resource::KeysetCursor {
                key: "5".into(),
                id: "1".into(),
            }),
            ..Default::default()
        },
    )?;
    template_selectable_resource::list_settings(
        conn,
        &template_selectable_resource::CategoryInput {
            workspace_id: WS,
            keyword: Some("a".into()),
            limit: 10,
            after: Some(template_selectable_resource::KeysetCursor {
                key: "a".into(),
                id: "1".into(),
            }),
            ..Default::default()
        },
    )?;
    template_version::list_changes_between(conn, 1, 0, 5)?;
    template_version::list_history(conn, 1)?;
    template_version::find(conn, 1, 1)?;
    template_version::find_latest(conn, 1)?;
    template_version::prune(conn, 1, 30)?;
    for t in workspace_purge::HEAVY_WORKSPACE_TABLES {
        workspace_purge::delete_workspace_rows_batch(conn, t, WS, 10)?;
        workspace_purge::has_workspace_rows(conn, t, WS)?;
    }
    for t in ["CoexistSyncRun", "FlowAnalyticsSession", "MessageCleanup"] {
        workspace_purge::delete_workspace_rows_batch(conn, t, WS, 10)?;
    }
    for spec in workspace_purge::PARTITIONED_WORKSPACE_TABLES {
        workspace_purge::delete_partitioned_batch(conn, spec, WS, 10)?;
        workspace_purge::has_partitioned_workspace_rows(conn, spec, WS)?;
    }
    for t in workspace_purge::WORKSPACE_ANALYTICS_HYPERTABLES {
        workspace_purge::delete_analytics_slice(conn, t, WS, 0, NOW)?;
        workspace_purge::find_next_analytics_row_at(conn, t, WS)?;
    }
    workspace_purge::list_message_conversation_page(conn, "Message", WS, 10)?;
    workspace_purge::list_message_conversation_page(conn, "Attachment", WS, 10)?;
    workspace_purge::delete_messages_and_attachments(conn, WS, &[1])?;
    mac_partitions::delete_leaderboard_score_event_month_batch(conn, NOW, 10)?;
    // untested repositories
    store_integration::find_shopify(conn, WS)?;
    let shop: repo::patch::Patch = vec![
        ("shopDomain", Value::Text("s.myshopify.com".into())),
        ("currency", Value::Text("USD".into())),
        ("authKind", Value::Text("token".into())),
        ("encryptedAuth", Value::Text("{}".into())),
        ("webhookSubscriptionIds", Value::Text("[]".into())),
    ];
    store_integration::upsert_shopify(conn, WS, &shop)?;
    store_integration::upsert_shopify(conn, WS, &shop)?;
    store_integration::update_shopify(conn, WS, &vec![("status", Value::Text("invalid".into()))])?;
    store_integration::delete_shopify(conn, WS)?;
    let woo: repo::patch::Patch = vec![
        ("siteUrl", Value::Text("https://w".into())),
        ("currency", Value::Text("USD".into())),
        ("encryptedAuth", Value::Text("{}".into())),
        ("webhookIds", Value::Text("[]".into())),
    ];
    store_integration::upsert_woocommerce(conn, WS, &woo)?;
    store_integration::update_woocommerce(conn, WS, &vec![("productCount", Value::Integer(3))])?;
    store_integration::find_woocommerce(conn, WS)?;
    store_integration::delete_woocommerce(conn, WS)?;
    small_repos::questionnaire_list_names_with_prefix(conn, WS, "a_%")?;
    small_repos::whatsapp_message_template_list_ids_by_integration(conn, 1)?;
    let inc = small_repos::BotMessageUsageIncrement {
        count: 2,
        owner_id: USER,
        period_end: NOW + 1,
        period_start: NOW,
        tenant_id: 1,
        workspace_id: WS,
    };
    small_repos::bot_message_usage_increment(conn, &inc)?;
    small_repos::bot_message_usage_increment(conn, &inc)?;
    small_repos::whatsapp_flow_increment_completed_count(conn, 1, "s")?;
    small_repos::list_executed_webhook_pairs(conn, &[1], &[1])?;
    small_repos::cleanup_old_webhook_executions(conn, NOW)?;
    let chat = small_repos::TelegramChatUpsert {
        workspace_id: WS,
        inbox_id: 3001,
        chat_id: "c".into(),
        title: None,
        r#type: "group".into(),
        bot_status: "member".into(),
    };
    small_repos::telegram_chat_upsert(conn, &chat)?;
    small_repos::telegram_chat_upsert(conn, &chat)?;
    small_repos::telegram_chat_list_by_inbox(conn, WS, 3001, &["member"])?;
    small_repos::mac_sum_for_tenant(conn, 1, NOW)?;
    small_repos::mac_sum_for_owner(conn, USER, Some(NOW), false)?;
    small_repos::mac_sum_for_owner(conn, USER, None, true)?;
    let menu =
        small_repos::create_user_persistent_menu(conn, WS, "m", &Json(json!([])))?.expect("menu");
    small_repos::update_user_persistent_menu(conn, WS, menu.id, "m2", &Json(json!([])))?;
    small_repos::list_user_persistent_menus_by_workspace(conn, WS)?;
    small_repos::find_user_persistent_menu_by_id(conn, WS, menu.id)?;
    small_repos::delete_user_persistent_menus(conn, WS, &[menu.id])?;
    small_repos::saved_reply_image_count_references(conn, f.id)?;
    small_repos::saved_reply_image_list_by_saved_reply_ids(conn, WS, &[1])?;
    small_repos::saved_reply_image_count_by_saved_reply_id(conn, WS, 1)?;
    small_repos::saved_reply_image_replace_for_saved_reply(conn, WS, 1, &[])?;
    small_repos::template_ai_file_list_queued_for_installation(conn, 1)?;
    small_repos::template_ai_file_list_queued_before(conn, NOW, 10)?;
    template_resource_backup::find_by_id_and_installation(conn, 1, 1, WS)?;
    template_resource_backup::list_by_sync(conn, 1)?;
    template_resource_backup::prune_for_resource(conn, 1)?;
    template_resource_backup::delete_by_ids(conn, &[1])?;
    meta_capi_event::find_workspace_event(conn, 1, WS)?;
    meta_capi_event::find_pending_by_source_key(conn, WS, "messenger", "k")?;
    meta_capi_event::delete_by_integration(conn, WS, "messenger", 1)?;
    meta_capi_event::update_capi_status(conn, 1, WS, "pending", "sent", Some(NOW), Some(None))?;
    mini_app::list_paginated(
        conn,
        &mini_app::MiniAppListInput {
            workspace_id: WS,
            keyword: Some("a".into()),
            page: 1,
            per_page: 10,
            sort: None,
        },
    )?;
    mini_app::count(conn, WS, Some("a"))?;
    mini_app::find_by_id_and_workspace(conn, WS, 1)?;
    mini_app::list_names_with_prefix(conn, WS, "a")?;
    mini_app::find_by_id(conn, 1)?;
    mini_app::list_submissions(conn, WS, 1, 1, 10)?;
    mini_app::count_submissions(conn, WS, 1)?;
    let seq = sequence::SequenceListInput {
        workspace_id: WS,
        name: Some("a".into()),
        folder_id: Some("0".into()),
        active: Some(true),
        ..Default::default()
    };
    sequence::list_with_counts(conn, &seq)?;
    sequence::count(conn, &seq)?;
    sequence::find_with_steps(conn, WS, 1)?;
    sequence::complete_active_enrollment_batch(conn, WS, 10)?;
    webhook::list_active_date_time_webhooks(conn, None, 10)?;
    webhook::list_active_date_time_webhooks(conn, Some(1), 10)?;
    webhook::list_webhooks_paginated(
        conn,
        &webhook::ListWebhooksInput {
            workspace_id: WS,
            folder_id: Some(None),
            name: Some("a".into()),
            limit: 10,
            offset: 0,
        },
    )?;
    webhook::find_webhook_with_conditions(conn, Some(1), Some(WS))?;
    whatsapp_coexist_staging::stage_payload(conn, 1, "p", &Json(json!({})), "h")?;
    whatsapp_coexist_staging::list_pending(conn, "p", 10)?;
    whatsapp_coexist_staging::mark_processed(conn, &[1])?;
    whatsapp_coexist_staging::mark_parse_failed(conn, &[1])?;
    let cat = place::create_category(conn, WS, &vec![("name", Value::Text("Cafe".into()))])?
        .expect("category");
    place::find_or_create_categories_by_names(conn, WS, &[" cafe ", "Bar"])?;
    place::update_category(conn, WS, cat.id, &vec![("rank", Value::Integer(2))])?;
    place::list_categories_with_location_count(conn, WS)?;
    place::find_category(conn, WS, cat.id)?;
    let loc = place::create_location(
        conn,
        WS,
        &vec![
            ("name", Value::Text("L".into())),
            ("latitude", Value::Real(1.0)),
            ("longitude", Value::Real(2.0)),
            ("categoryId", Value::Integer(cat.id)),
        ],
    )?
    .expect("location");
    place::list_locations(
        conn,
        WS,
        Some("l"),
        PaginationInput {
            page: Some(1),
            per_page: Some(10),
        },
    )?;
    place::list_locations(conn, WS, None, PaginationInput::default())?;
    place::find_location(conn, WS, loc.id)?;
    place::list_location_coordinates(conn, WS, Some(cat.id))?;
    place::update_location(conn, WS, loc.id, &vec![("name", Value::Text("L2".into()))])?;
    place::count_category_locations(conn, WS, cat.id)?;
    place::delete_location(conn, WS, loc.id)?;
    place::delete_category(conn, WS, cat.id)?;
    vendor::create_missing(conn, WS, &["Acme", " acme", "Beta"], None)?;
    vendor::find_by_names(conn, WS, &["ACME"], None)?;
    vendor::list_with_product_count(
        conn,
        WS,
        None,
        Some("a"),
        PaginationInput {
            page: Some(1),
            per_page: Some(10),
        },
    )?;
    vendor::list_options(conn, WS, None)?;
    let v =
        vendor::create(conn, WS, &vec![("name", Value::Text("Gamma".into()))])?.expect("vendor");
    vendor::update(conn, WS, v.id, &vec![("rank", Value::Integer(1))])?;
    vendor::find(conn, WS, v.id)?;
    vendor::delete(conn, WS, v.id)?;
    workspace_deletion_record::read_snapshot_sources(conn, USER, WS)?;
    workspace_deletion_record::insert_if_absent(
        conn,
        &workspace_deletion_record::NewDeletionRecord {
            deletion_requested_at: None,
            deletion_requested_by_email: None,
            deletion_requested_by_id: None,
            details: Json(json!({})),
            owner_email: None,
            owner_id: USER,
            scheduled_deletion_at: None,
            tenant_id: 1,
            usage_snapshot: Json(json!({})),
            workspace_id: WS,
            workspace_name: "W".into(),
        },
    )?;
    workspace_deletion_record::mark_purged(conn, WS)?;
    workspace_deletion_record::list_for_owner(conn, USER, 10, 0)?;
    workspace_deletion_record::list_for_admin(conn, 10, 0, Some("w"), Some(1))?;
    response_sla::upsert_policy(
        conn,
        WS,
        response_sla::ResponseSlaPolicyInput {
            enabled: true,
            first_response_target_seconds: 60,
            next_response_target_seconds: None,
            breaching_threshold_percent: 80,
        },
    )?;
    response_sla::find_policy(conn, WS)?;
    response_sla::read_window_states(conn, WS, &[1], true)?;
    response_sla::are_windows_already_open(conn, WS, &[1])?;
    response_sla::read_assignee_ids(conn, WS, &[1])?;
    response_sla::set_snapshot_if_window_unchanged(
        conn,
        WS,
        1,
        NOW,
        response_sla::SlaSnapshot {
            response_due_at: NOW,
            response_sla_breaching_at: NOW,
            response_sla_target_seconds: 60,
        },
    )?;
    response_sla::clear_snapshot_if_window_unchanged(conn, WS, 1, NOW)?;
    response_sla::clear_workspace_snapshots(conn, WS)?;
    response_sla::delete_policy(conn, WS)?;
    Ok(())
}

#[tokio::test]
async fn every_untested_p3_3_function_runs_against_the_schema() {
    let db = fresh_db().await;
    seed(&db).await;
    repo_common::ok(repo::write(&db, |tx| all(tx)).await);
}
