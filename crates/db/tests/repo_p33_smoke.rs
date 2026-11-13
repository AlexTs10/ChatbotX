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

/// Slices message / whatsapp-call / pipeline / template-installed-resource / order /
/// product-category: the functions without a ported test.
fn more(conn: &chatbotx_db::rusqlite::Connection) -> Result<()> {
    use chatbotx_db::repo::*;
    let seed = common::Seed {
        user_id: USER,
        workspace_id: WS,
        inbox_id: 3001,
        contact_id: 4001,
    };
    common::insert_contact_inbox(conn, seed, 3101, "wa-1", None, None)?;
    common::insert_conversation(conn, seed, 3201, None)?;
    // whatsapp-call permission
    whatsapp_call_permission::upsert_for_contact_inbox(
        conn,
        &whatsapp_call_permission::UpsertPermissionInput {
            workspace_id: WS,
            contact_inbox_id: 3101,
            response: "accept".into(),
            is_permanent: false,
            expires_at: Some(NOW),
            responded_at: NOW,
        },
    )?;
    whatsapp_call_permission::find_by_contact_inbox_id(conn, 3101)?;
    // message (no TS tests)
    message::update_content_attributes(conn, 1, WS, &json!({}), NOW)?;
    message::update_message_attributes(conn, 1, WS, true, false, NOW)?;
    message::update_message_text(conn, 1, WS, "t", NOW)?;
    message::update_text_by_source_id(conn, "s", WS, "t")?;
    message::update_content_by_source_id(conn, "s", WS, &message::ContentBySourcePatch::default())?;
    message::claim_content_attributes(
        conn,
        &message::ClaimContentAttributesParams {
            message_id: 1,
            workspace_id: WS,
            created_at: NOW,
            guard_key: "k".into(),
            overlay: serde_json::Map::new(),
        },
    )?;
    message::delete_attachments_by_message_id(conn, 1, WS, NOW)?;
    message::update_send_error(conn, 1, Some("e"), WS, NOW)?;
    message::has_attachment_of_type(
        conn,
        &message::HasAttachmentOfTypeParams {
            workspace_id: WS,
            message_id: 1,
            message_created_at: NOW,
            file_type: "image".into(),
        },
    )?;
    message::find_trigger_message(
        conn,
        &message::FindTriggerMessageOptions {
            conversation_id: 3201,
            id: 1,
            require_complete_results: false,
            since_time: NOW,
            workspace_id: WS,
        },
    )?;
    message::find_by_source_id(conn, "s", 3201, WS, Some(NOW))?;
    message::find_last_by_conversation(
        conn,
        3201,
        &message::FindLastByConversationOptions {
            attachment_count_only: false,
            limit: Some(5),
            message_types: vec!["incoming".into()],
            require_complete_results: false,
            since_time: Some(NOW),
            with_attachments: true,
            workspace_id: Some(WS),
        },
    )?;
    message::find_many_by_ids(conn, &[1], 3101, Some(NOW), Some(WS))?;
    // pipeline (functions without a ported test)
    let p = pipeline::insert(
        conn,
        &pipeline::PipelineInsert {
            workspace_id: WS,
            name: "P".into(),
            r#type: "opportunity".into(),
            reference_name: "p".into(),
            settings: Json(json!({})),
            ..Default::default()
        },
    )?;
    pipeline::acquire_pipeline_advisory_lock(conn, "k")?;
    pipeline::find_by_id(conn, WS, p.id)?;
    pipeline::find_by_reference_name(conn, WS, "p")?;
    pipeline::find_by_reference_names(conn, WS, &["p".to_owned()])?;
    pipeline::find_by_name(conn, WS, "P")?;
    pipeline::list_by_workspace(conn, WS)?;
    pipeline::count_items_by_pipeline(conn, WS)?;
    pipeline::list_names_starting_with(conn, WS, pipeline::PipelineNameColumn::ReferenceName, "p")?;
    pipeline::update(conn, WS, p.id, &vec![("name", Value::Text("P2".into()))])?;
    pipeline::update(conn, WS, p.id, &vec![])?;
    let field_values = pipeline::PipelineFieldInsert {
        workspace_id: WS,
        pipeline_id: p.id,
        name: "F".into(),
        reference_name: "f".into(),
        r#type: "shortText".into(),
        required: false,
        position: 0,
    };
    let f = pipeline::insert_field(conn, &field_values)?;
    pipeline::insert_fields(
        conn,
        &[pipeline::PipelineFieldInsert {
            reference_name: "g".into(),
            ..field_values
        }],
    )?;
    pipeline::list_fields(conn, WS, p.id)?;
    pipeline::list_fields_by_pipeline_ids(conn, WS, &[p.id])?;
    pipeline::update_field(conn, WS, p.id, f.id, &vec![("required", Value::Integer(1))])?;
    pipeline::lock_fields_shared(conn, WS, p.id, &[f.id])?;
    pipeline::delete_field(conn, WS, p.id, f.id)?;
    conn.execute(
        "INSERT INTO \"WorkspaceMember\" (\"id\", \"workspaceId\", \"userId\", \"role\") VALUES (9901, ?1, ?2, 'agent')",
        (WS, USER),
    )?;
    let values = pipeline::PipelineMemberValues {
        can_view: true,
        ..Default::default()
    };
    let member = pipeline::PipelineMemberInsert {
        workspace_id: WS,
        pipeline_id: p.id,
        workspace_member_id: 9901,
        values,
    };
    pipeline::upsert_member(conn, &member)?;
    pipeline::upsert_member(conn, &member)?;
    pipeline::list_members(conn, WS, p.id)?;
    pipeline::find_member(conn, WS, p.id, 9901)?;
    pipeline::list_member_access(conn, WS, 9901)?;
    pipeline::delete_member(conn, WS, p.id, 9901)?;
    pipeline::insert_members(conn, &[member])?;
    let stage = pipeline_stage::insert(
        conn,
        &pipeline_stage::PipelineStageInsert {
            workspace_id: WS,
            pipeline_id: p.id,
            name: "S".into(),
            bulk_job_id: Some("job".into()),
            ..Default::default()
        },
    )?;
    pipeline_stage::list_by_pipeline_ids(conn, WS, &[p.id])?;
    pipeline_stage::list_names_by_ids(conn, WS, &[stage.id])?;
    pipeline_stage::find_by_id(conn, WS, Some(p.id), stage.id)?;
    pipeline_stage::lock_shared(conn, WS, stage.id)?;
    pipeline_stage::lock_exclusive(conn, WS, stage.id)?;
    pipeline_stage::update(
        conn,
        WS,
        p.id,
        stage.id,
        &vec![("name", Value::Text("S2".into()))],
    )?;
    pipeline_stage::release_bulk_claim(conn, WS, p.id, stage.id, "job")?;
    pipeline_stage::next_position(conn, WS, p.id)?;
    let number = pipeline::next_item_number(conn, WS)?;
    let item = pipeline_item::insert_idempotent(
        conn,
        &pipeline_item::PipelineItemInsert {
            workspace_id: WS,
            pipeline_id: p.id,
            stage_id: stage.id,
            contact_id: 4001,
            number,
            title: "I".into(),
            status: "open".into(),
            priority: "low".into(),
            field_values: Json(json!({})),
            stage_entered_at: NOW,
            ..Default::default()
        },
    )?
    .row;
    pipeline_item::find_by_idempotency_key(conn, WS, p.id, "k")?;
    pipeline_item::lock_for_update(conn, WS, item.id)?;
    pipeline_item::update(
        conn,
        WS,
        item.id,
        &vec![("title", Value::Text("I2".into()))],
    )?;
    pipeline_item::touch(conn, WS, item.id)?;
    pipeline_item::list_for_contact(conn, WS, 4001, Some(&[p.id]), 10)?;
    pipeline_item::list_for_contact(conn, WS, 4001, None, 10)?;
    conn.execute(
        "INSERT INTO \"InboxTeam\" (\"id\", \"workspaceId\", \"name\") VALUES (9902, ?1, 'T')",
        [WS],
    )?;
    pipeline_item::replace_assignees(conn, WS, item.id, &[9901], &[9902])?;
    pipeline_item::replace_assignees(conn, WS, item.id, &[], &[9902])?;
    pipeline_item::list_assignees_for_items(conn, WS, &[item.id])?;
    pipeline_variable_targets::list_for_contact(conn, WS, 4001)?;
    let comment = pipeline_item_content::insert_comment_idempotent(
        conn,
        &pipeline_item_content::PipelineCommentInsert {
            workspace_id: WS,
            item_id: item.id,
            text: "c".into(),
            created_by_id: Some(USER),
            idempotency_key: Some("k".into()),
        },
    )?;
    pipeline_item_content::insert_comment_idempotent(
        conn,
        &pipeline_item_content::PipelineCommentInsert {
            workspace_id: WS,
            item_id: item.id,
            text: "c".into(),
            created_by_id: Some(USER),
            idempotency_key: Some("k".into()),
        },
    )?;
    pipeline_item_content::list_comments(conn, WS, item.id, 10, None)?;
    pipeline_item_content::count_comments_for_items(conn, WS, &[item.id])?;
    pipeline_item_content::latest_comment_at(conn, WS, item.id)?;
    pipeline_item_content::delete_comment(conn, WS, item.id, comment.row.id)?;
    pipeline_item_content::insert_attachments(conn, WS, item.id, &[])?;
    pipeline_item_content::list_attachments(conn, WS, item.id)?;
    pipeline_item_content::count_attachments(conn, WS, item.id)?;
    pipeline_item_content::delete_attachment(conn, WS, item.id, 1)?;
    pipeline_reference::lock_user_if_exists(conn, USER)?;
    pipeline_reference::list_workspace_members(conn, WS, &[9901])?;
    pipeline_reference::list_inbox_team_ids(conn, WS, &[9902])?;
    pipeline_reference::list_media_library_file_ids(conn, WS, &[1])?;
    pipeline_reference::list_contact_summaries(conn, WS, &[4001])?;
    pipeline_reference::contact_exists(conn, WS, 4001)?;
    pipeline_stage::delete(conn, WS, p.id, stage.id)?;
    // template-installed-resource (functions without a ported test)
    conn.execute(
        "INSERT INTO \"TemplateInstallation\" (\"id\", \"workspaceId\", \"templateName\", \"sourceWorkspaceId\", \"formatVersion\", \"permissions\") VALUES (9903, ?1, 'T', ?1, 1, '{}')",
        [WS],
    )?;
    template_installed_resource::find_blocking(
        conn,
        WS,
        "flow",
        &[1],
        template_installed_resource::TemplateResourcePermission::AllowEdit,
        &[],
    )?;
    template_installed_resource::find_live_keys(
        conn,
        WS,
        &[template_installed_resource::ResourceKey {
            kind: "flow".into(),
            resource_id: 1,
        }],
    )?;
    template_installed_resource::find_prior_copies(conn, WS, "flow", &["s".to_owned()])?;
    template_installed_resource::list_categories(conn, 9903, WS)?;
    template_installation_sync::list_installed_resources_by_installation_ids(conn, &[9903])?;
    template_installation_sync::mark_synced(conn, 1, Some("h"), 1)?;
    template_installation_sync::detach_resources(conn, &[1])?;
    template_installed_resource_meta::spread_created_at(
        conn,
        &[
            ("tag".to_owned(), 1),
            ("tag".to_owned(), 2),
            ("flow".to_owned(), 3),
        ],
    )?;
    template_installed_resource_meta::load_resource_labels(
        conn,
        WS,
        &[
            ("automatedResponse".to_owned(), 1),
            ("savedReply".to_owned(), 1),
            ("calendar".to_owned(), 1),
            ("aiAgent".to_owned(), 1),
            ("aiFunction".to_owned(), 1),
            ("aiMcpServer".to_owned(), 1),
            ("aiFile".to_owned(), 1),
            ("questionnaire".to_owned(), 1),
            ("couponTopic".to_owned(), 1),
            ("integrationWebchat".to_owned(), 1),
            ("fbCommentAutomation".to_owned(), 1),
            ("unknownKind".to_owned(), 1),
        ],
    )?;
    for kind in template_installed_resource::TEMPLATE_RESOURCE_KINDS {
        template_installed_resource_meta::load_resource_labels(conn, WS, &[(kind.to_owned(), 1)])?;
        template_installed_resource_meta::spread_created_at(
            conn,
            &[(kind.to_owned(), 1), (kind.to_owned(), 2)],
        )?;
    }
    template_installed_resource_uninstall::lock_keywords(conn, WS, &[1])?;
    template_installed_resource_uninstall::find_referenced_keyword_ids(conn, WS, &[1])?;
    template_installed_resource_uninstall::find_tombstoned(conn, WS, "tag", &[1])?;
    template_installed_resource_uninstall::delete_keywords(conn, WS, &[1], "inbound")?;
    template_installed_resource_uninstall::find_delete_blocked_by_others(
        conn,
        WS,
        "tag",
        &[1],
        &[9903],
    )?;
    // product-category
    let cat = product_category::create(
        conn,
        &product_category::CreateCategoryInput {
            workspace_id: WS,
            name: "C".into(),
            ..Default::default()
        },
    )?
    .map(|c| c.id)
    .unwrap_or_default();
    product_category::create_missing_by_name(conn, WS, &[" C ".to_owned(), "D".to_owned()], None)?;
    product_category::create_missing_children(
        conn,
        WS,
        &[(cat, "E".to_owned()), (cat, "E ".to_owned())],
    )?;
    product_category::list_with_product_count(conn, WS, "chatbotx")?;
    product_category::list_children(conn, WS, cat)?;
    product_category::list(conn, WS, "chatbotx")?;
    product_category::find(conn, WS, cat)?;
    product_category::update(
        conn,
        &product_category::UpdateCategoryInput {
            workspace_id: WS,
            category_id: cat,
            name: Some("C2".into()),
            parent_id: Some(None),
            image_url: Some(None),
            is_visible: Some(false),
            rank: Some(1),
        },
    )?;
    product_category::file_under_parent(conn, WS, cat, cat)?;
    product_category::file_as_top_level(conn, WS, cat)?;
    product_category::count_products(conn, WS, cat)?;
    product_category::find_by_names(conn, WS, &["D".to_owned()], Some("chatbotx"))?;
    product_category::delete(conn, WS, cat)?;
    // order + analytics
    let o = order::insert(
        conn,
        &vec![
            ("workspaceId", Value::Integer(WS)),
            ("number", Value::Integer(1)),
            ("source", Value::Text("manual".into())),
            ("externalId", Value::Text("x".into())),
            ("currency", Value::Text("USD".into())),
            ("subtotal", Value::Integer(0)),
            ("discountTotal", Value::Integer(0)),
            ("shippingTotal", Value::Integer(0)),
            ("serviceFeeTotal", Value::Integer(0)),
            ("taxTotal", Value::Integer(0)),
            ("total", Value::Integer(0)),
        ],
    )?
    .map(|o| o.id)
    .unwrap_or_default();
    order::insert_items(
        conn,
        WS,
        o,
        &[vec![
            ("name", Value::Text("n".into())),
            ("unitPrice", Value::Integer(1)),
            ("quantity", Value::Integer(2)),
            ("discountPercent", Value::Integer(0)),
            ("taxPercent", Value::Integer(0)),
            ("discountTotal", Value::Integer(0)),
            ("taxTotal", Value::Integer(0)),
            ("lineTotal", Value::Integer(2)),
            ("vendorName", Value::Text("v".into())),
        ]],
    )?;
    order::insert_history(
        conn,
        &vec![
            ("orderId", Value::Integer(o)),
            ("workspaceId", Value::Integer(WS)),
            ("toStatus", Value::Text("pending".into())),
            ("actorType", Value::Text("user".into())),
            ("actorUserId", Value::Integer(USER)),
        ],
    )?;
    order::list(
        conn,
        &order::OrderListInput {
            workspace_id: WS,
            status: vec!["pending".into()],
            keyword: Some("#1".into()),
            page: Some(1),
            per_page: Some(10),
            ..Default::default()
        },
    )?;
    order::find_detail(conn, WS, o)?;
    order::mark_paid(conn, WS, o, NOW)?;
    order::find_for_update(conn, WS, o)?;
    order::find_by_external_id(conn, WS, "x")?;
    order::find_by_external_id_for_update(conn, WS, "x")?;
    order::update(conn, WS, o, &vec![("note", Value::Text("n".into()))])?;
    order::transition_status(conn, WS, o, &["pending".to_owned()], "accepted", &vec![])?;
    order::set_rating(conn, WS, o, 5, Some("ok"))?;
    order::list_items(conn, WS, o)?;
    let scope = order_analytics::SalesScope {
        workspace_id: WS,
        currency: "USD".into(),
        since: Some(0),
        until: Some(NOW * 2),
    };
    order_analytics::count_other_currency_orders(conn, &scope)?;
    order_analytics::summary(conn, &scope)?;
    order_analytics::series(
        conn,
        &scope,
        "UTC",
        order_analytics::SeriesGranularity::Hour,
    )?;
    order_analytics::series(conn, &scope, "UTC", order_analytics::SeriesGranularity::Day)?;
    order_analytics::top_products(conn, &scope, 5)?;
    order_analytics::top_vendors(conn, &scope, 5)?;
    // external-store (functions without a ported test)
    let ids = ["gid://1".to_owned()];
    external_store::delete_product_by_external_id(conn, WS, "shopify", "gid://0")?;
    external_store::delete_products_not_in(conn, WS, "shopify", &ids)?;
    external_store::delete_products_not_in(conn, WS, "shopify", &[])?;
    external_store::count_products(conn, WS, "shopify")?;
    external_store::find_variants_by_external_ids(conn, WS, "shopify", &ids)?;
    external_store::find_variants_for_variations(conn, WS, "woocommerce", &ids)?;
    external_store::find_products_by_external_ids(conn, WS, "shopify", &ids)?;
    external_store::find_checkout_variant_ids(conn, WS, "shopify", &[1], &[1])?;
    let discount = vec![
        ("code", Value::Text("SAVE".into())),
        ("discountType", Value::Text("percentage".into())),
        ("productIds", Value::Text("[]".into())),
    ];
    external_store::upsert_discount(conn, WS, "shopify", "d1", &discount)?;
    external_store::upsert_discount(conn, WS, "shopify", "d1", &discount)?;
    external_store::delete_discounts_not_in(conn, WS, "shopify", &["d2".to_owned()])?;
    external_store::delete_discount_by_external_id(conn, WS, "shopify", "d1")?;
    order::delete_items(conn, WS, o)?;
    order::delete(conn, WS, o)?;
    Ok(())
}

fn templates(conn: &chatbotx_db::rusqlite::Connection) -> Result<()> {
    use chatbotx_db::repo::*;
    let t = |s: &str| Value::Text(s.to_owned());
    // template (snapshot, preview, repository)
    let lookup = template::ManifestLookup {
        workspace_id: WS,
        ids: Some(vec![1]),
        names: Some(vec!["a".into()]),
        limit: Some(5),
    };
    template::snapshot_find_tags(conn, &lookup)?;
    template::snapshot_find_custom_fields(conn, &lookup)?;
    template::snapshot_find_tags(
        conn,
        &template::ManifestLookup {
            workspace_id: WS,
            ..Default::default()
        },
    )?;
    template::preview_find_tags_by_names(conn, WS, &["a".to_owned()])?;
    template::preview_list_place_categories(conn, WS)?;
    template::preview_list_custom_fields(conn, WS)?;
    template::preview_list_bot_fields(conn, WS)?;
    template::preview_find_folders(conn, WS, &["tag".to_owned()], &["a".to_owned()])?;
    let snapshot = template::TemplateVersionSnapshot {
        payload: Json(json!({"manifests": {"inboxes": [{"id": "1"}]}})),
        payload_hash: "h1".into(),
        format_version: 1,
        changes: None,
    };
    let values = vec![
        ("tenantId", Value::Integer(1)),
        ("workspaceId", Value::Integer(WS)),
        ("name", t("T")),
        ("selection", t("{}")),
        ("payload", t(r#"{"manifests":{"inboxes":[{"id":"1"}]}}"#)),
        ("categoryCounts", t(r#"{"flows":1}"#)),
        ("formatVersion", Value::Integer(1)),
        ("shareToken", t("share-1")),
        ("defaultPermissions", t("{}")),
    ];
    let tpl = template::create_with_first_version(conn, &values, 77001, &snapshot)?;
    let id = tpl.id;
    template::find_live(conn, id, WS)?.expect("live");
    template::find_live_detail(conn, id, WS)?;
    template::find_live_by_id(conn, id)?;
    template::find_live_by_share_token(conn, "share-1")?;
    template::find_install_source(conn, id)?;
    template::find_live_payload(conn, id)?;
    template::find_category_counts(conn, id)?;
    template::find_sync_source(conn, id)?;
    template::find_live_tenant(conn, id, WS)?;
    template::find_by_id_for_tenant(conn, id, 1)?;
    template::find_for_staging_by_share_token(conn, "share-1")?;
    assert_eq!(template::find_live_version(conn, id)?, Some(1));
    template::rotate_share_token(conn, id, WS, "share-2")?;
    template::update_share_settings(conn, id, WS, true, Some(Some(NOW)))?;
    template::update_share_settings(conn, id, WS, false, None)?;
    let saved = template::save_with_version(
        conn,
        &template::SaveWithVersion {
            id,
            workspace_id: WS,
            version_id: 77002,
            set: vec![("name", t("T2"))],
            snapshot: template::TemplateVersionSnapshot {
                payload_hash: "h2".into(),
                ..snapshot.clone()
            },
            legacy_payload_hash: None,
            based_on_version: 1,
        },
    )?
    .expect("saved");
    assert_eq!(saved.version, 2);
    let inbox = template::find_inbox_manifest_by_share_token(conn, "share-2")?.expect("manifest");
    assert_eq!(inbox.inboxes, Some(json!([{"id": "1"}])));
    // template-installation
    let inst = template_installation::create(
        conn,
        &vec![
            ("workspaceId", Value::Integer(WS)),
            ("templateId", Value::Integer(id)),
            ("sourceTemplateId", Value::Integer(id)),
            ("templateName", t("T")),
            ("sourceWorkspaceId", Value::Integer(WS)),
            ("formatVersion", Value::Integer(1)),
            ("permissions", t("{}")),
            ("publisherRef", t("pub-1")),
        ],
    )?;
    let iid = inst.id;
    template_installation::lock_install_request(conn, WS, id)?;
    template_installation::find_publisher_ref(conn, WS, id)?;
    template_installation::list_installed_by_share_token(conn, "share-2", &[WS])?;
    template_installation::find_publisher_uninstall_grant(conn, WS, id)?;
    template_installation::list_pending_ids(conn, WS)?;
    template_installation::find_copy_tenants(conn, id, WS)?;
    template_installation::find_by_id_and_workspace(conn, iid, WS)?;
    template_installation::list_by_ids_and_workspace(conn, &[iid], WS)?;
    template_installation::find_latest_by_source_template(conn, WS, id)?;
    template_installation::find_active_by_source_template(conn, WS, id)?;
    template_installation::find_installed_by_source_template(conn, WS, id)?;
    let claimed = template_installation::claim(conn, iid, WS, "tok", NOW)?.expect("claim");
    assert_eq!(claimed.status, "installing");
    template_installation::release(conn, iid, "tok")?;
    template_installation::claim(conn, iid, WS, "tok", NOW)?;
    let effect = template_installation::PublishEffect {
        flow_id: 1,
        version_id: 2,
    };
    template_installation::add_pending_publish_effects(conn, iid, std::slice::from_ref(&effect))?;
    assert_eq!(
        template_installation::find_pending_publish_effects(conn, iid)?,
        vec![effect.clone()]
    );
    template_installation::defer_pending_publish_effects(conn, iid)?;
    template_installation::list_with_pending_publish_effects(conn, NOW, 5)?;
    template_installation::clear_pending_publish_effects(conn, iid, &[effect])?;
    assert!(template_installation::complete(
        conn,
        iid,
        "tok",
        &template_installation::Completion {
            status: "completed".into(),
            warnings: Some(json!([{"entityKind": "quota"}])),
            warning_count: Some(1),
            resource_count: Some(0),
            install_folder_id: Some(None),
            pending_publish_effects: Some(None),
        },
    )?);
    template_installation::fail(conn, iid, "tok", "e", "m")?;
    template_installation::fail_unclaimed(conn, 0, "e", "m", NOW)?;
    template_installation::list_stale(conn, NOW, NOW, 0, 5)?;
    template_installation::fail_abandoned(conn, 0, NOW, NOW, "e", "m")?;
    template_installation::record_install_event(conn, iid)?;
    template_installation::list_active_by_publisher_ref(conn, id, "pub-1")?;
    template_installation::find_latest_by_publisher_ref(conn, id, "pub-1")?;
    template_installation::set_publisher_uninstall_grant(conn, WS, id, true)?;
    let page = template_installation::list_page(
        conn,
        &template_installation::ListPage {
            workspace_id: WS,
            status: Some("completed".into()),
            active_only: false,
            before_id: Some(i64::MAX),
            limit: 5,
            visible_categories: Some(vec!["flows".into()]),
        },
    )?;
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].installer_warning_count, 1);
    template_installation::list_page(
        conn,
        &template_installation::ListPage {
            workspace_id: WS,
            active_only: true,
            limit: 5,
            visible_categories: Some(vec![]),
            ..Default::default()
        },
    )?;
    template_installation::list_with_staged_objects_to_clean(conn, NOW, NOW, 5)?;
    template_installation::claim_staged_objects_cleanup(conn, iid, NOW, NOW)?;
    template_installation::advance_staged_cleaned_generation(conn, iid, 1, 1)?;
    template_installation::release_staged_objects_cleanup(conn, iid, NOW)?;
    // template list / analytics (after the install exists)
    let listed = template::list_with_install_counts(conn, WS)?;
    assert_eq!(listed[0].install_count, 2);
    let first = template::list_with_install_counts_page(conn, WS, 1, None)?;
    template::list_with_install_counts_page(
        conn,
        WS,
        1,
        Some((
            first
                .last_key
                .as_deref()
                .unwrap_or("2026-01-01T00:00:00.000Z"),
            id,
        )),
    )?;
    template::install_analytics(conn, id)?;
    let publishers = template::list_publisher_rows(
        conn,
        &template::ListPublisherRows {
            template_id: id,
            search: Some("Work_%".into()),
            after: Some((i64::MAX, "z".into())),
            limit: 5,
        },
    )?;
    assert!(publishers.is_empty());
    let publishers = template::list_publisher_rows(
        conn,
        &template::ListPublisherRows {
            template_id: id,
            limit: 5,
            ..Default::default()
        },
    )?;
    assert_eq!(publishers.len(), 1);
    // template-installation-sync (rest)
    let new_sync = template_installation_sync::NewTemplateSync {
        id: 78001,
        workspace_id: WS,
        installation_id: iid,
        trigger: "manual".into(),
        requested_version: 2,
        ..Default::default()
    };
    template_installation_sync::create_if_absent(conn, &new_sync)?.expect("sync");
    template_installation_sync::create_many_if_absent(conn, &[new_sync])?;
    template_installation_sync::find_by_id(conn, 78001, WS)?;
    template_installation_sync::adopt_idempotency_key(conn, 78001, "k", None)?;
    template_installation_sync::find_by_idempotency_key(conn, iid, "k")?;
    template_installation_sync::find_active_by_installation(conn, iid)?;
    template_installation_sync::list_by_installation(conn, iid, WS, 5)?;
    template_installation_sync::claim(conn, 78001, WS, "s", NOW)?.expect("sync claim");
    template_installation_sync::has_running_sync(conn, iid)?;
    template_installation_sync::release(conn, 78001, "s")?;
    template_installation_sync::claim(conn, 78001, WS, "s", NOW)?;
    template_installation_sync::finish(
        conn,
        78001,
        "s",
        &template_installation_sync::TemplateSyncOutcome {
            status: "completed".into(),
            ..Default::default()
        },
    )?;
    template_installation_sync::find_status_finished_by_claim(conn, 78001, "s")?;
    template_installation_sync::close_unclaimed(conn, 78001, "skipped", "e", Some(NOW))?;
    template_installation_sync::cancel_pending_syncs(conn, iid)?;
    template_installation_sync::list_unfinished(conn, NOW, NOW, 0, 5)?;
    let codes = vec!["permanent".to_owned()];
    template_installation_sync::list_auto_update_candidates(conn, id, 3, Some(0), 5, &codes)?;
    template_installation_sync::list_templates_behind(conn, 5, &codes)?;
    template_installation_sync::list_templates_behind(conn, 5, &[])?;
    template_installation_sync::complete_installation_sync(conn, iid, 2)?;
    template_installation_sync::fail_abandoned(conn, 0, "e")?;
    template_installation_sync::list_installed_categories_by_pair(conn, WS, id)?;
    // template-uninstall
    let op =
        template_uninstall::create_operation(conn, WS, Some(id), "pub-1", "installer", Some(USER))?;
    template_uninstall::request(conn, iid, op.id, "installer", Some(USER))?.expect("request");
    template_uninstall::operation_id_of(conn, iid)?;
    template_uninstall::operation_has_active_members(conn, op.id)?;
    template_uninstall::claim(conn, iid, WS, "u")?.expect("uninstall claim");
    template_uninstall::has_live_uninstall_touching_workspace(conn, WS)?;
    template_uninstall::renew_lease(conn, iid, "u")?;
    template_uninstall::release(conn, iid, "u")?;
    template_uninstall::claim(conn, iid, WS, "u")?;
    template_uninstall::list_stale(conn, 5)?;
    template_uninstall::list_abandoned(conn, NOW, 5)?;
    template_uninstall::is_in_progress(conn, iid)?;
    let counts = template_uninstall::count_outcomes(conn, iid)?;
    assert!(template_uninstall::finalize(
        conn,
        iid,
        "u",
        "partial",
        &counts,
        Some("e")
    )?);
    template_uninstall::mark_cancelled(conn, iid)?;
    template_uninstall::mark_skipped(conn, iid, "e")?;
    template_uninstall::skip_pending(conn, iid, "e")?;
    template_uninstall::mark_abandoned(conn, iid, NOW, &counts, "e")?;
    template_uninstall::list_pending_publisher_uninstalls(conn, WS, id)?;
    template_uninstall::finalize_operation_if_complete(conn, op.id)?;
    template_uninstall::list_operations_to_finalize(conn, 5)?;
    template_uninstall::list_undelivered_notices(conn, 5)?;
    template_uninstall::mark_notice_delivered(conn, op.id)?;
    template_uninstall::find_operation(conn, op.id, WS)?;
    conn.execute(
        "INSERT INTO \"TemplateInstalledResource\" (\"id\", \"installationId\", \"workspaceId\", \"category\", \"resourceKind\", \"resourceId\", \"sourceResourceId\", \"uninstallOutcome\") VALUES (78101, ?1, ?2, 'tags', 'folder', 1, '__installRoot:1', 'failed')",
        (iid, WS),
    )?;
    template_uninstall::set_effects(conn, &[(78101, json!([{"kind": "x"}]))])?;
    template_uninstall::add_installation_warning(conn, iid, &json!({"code": "w"}))?;
    template_uninstall::list_pending_effects(conn, iid)?;
    template_uninstall::list_installations_with_pending_effects(conn, 5)?;
    template_uninstall::bump_effect_attempts(conn, &[78101])?;
    template_uninstall::clear_effects(conn, &[78101])?;
    template_uninstall::list_tombstones(conn, iid, 5)?;
    template_uninstall::reset_failed_to_deleting(conn, iid)?;
    template_uninstall::set_outcomes(conn, &[78101], "deleting", Some(None), true)?;
    template_uninstall::clear_frozen_from(conn, &[78101])?;
    template_uninstall::set_frozen_from(conn, &[(78101, json!({"a": true}))])?;
    template_uninstall::mark_delete_committed(conn, &[78101])?;
    template_uninstall::untombstone(conn, iid)?;
    template_uninstall::list_doomed(conn, iid)?;
    template_uninstall::find_install_roots(conn, iid)?;
    template_uninstall::delete_folder_if_empty(conn, WS, 999_999)?;
    conn.execute(
        "INSERT INTO \"Folder\" (\"id\", \"workspaceId\", \"name\", \"folderType\", \"paths\") VALUES (78201, ?1, 'F', 'tag', '[]')",
        [WS],
    )?;
    assert_eq!(
        template_uninstall::delete_folder_if_empty(conn, WS, 78201)?,
        "deleted"
    );
    let slots = template_uninstall::ReferenceSlots {
        scalars: vec!["flowId".into()],
        arrays: vec!["tagIds".into()],
        prefixes: vec!["flow".into()],
        condition_types: vec!["tag".into()],
    };
    let edges = template_uninstall::find_inbound_edges(
        conn,
        WS,
        &[id.to_string()],
        &[
            template_uninstall::ReferenceColumnProbe {
                holder_kind: Some("installation".into()),
                table: "TemplateInstallation".into(),
                column: "templateId".into(),
                holder_id_column: "id".into(),
                workspace_column: Some("workspaceId".into()),
                text_column: false,
            },
            template_uninstall::ReferenceColumnProbe {
                table: "Condition".into(),
                column: "sourceId".into(),
                holder_id_column: "id".into(),
                ..Default::default()
            },
        ],
        &[
            template_uninstall::ReferenceContainerProbe {
                holder_kind: Some("template".into()),
                table: "Template".into(),
                holder_id_column: "id".into(),
                json_columns: vec!["payload".into(), "selection".into()],
                workspace_column: Some("workspaceId".into()),
                ..Default::default()
            },
            template_uninstall::ReferenceContainerProbe {
                table: "Condition".into(),
                holder_id_column: "id".into(),
                json_columns: vec!["value".into()],
                condition_type_column: Some("type".into()),
                ..Default::default()
            },
            template_uninstall::ReferenceContainerProbe {
                holder_kind: Some("template".into()),
                table: "Template".into(),
                holder_id_column: "id".into(),
                workspace_column: Some("workspaceId".into()),
                contact_filter_column: Some("selection".into()),
                ..Default::default()
            },
        ],
        &slots,
    )?;
    assert_eq!(edges.len(), 1, "{edges:?}");
    assert!(
        template_uninstall::find_inbound_edges(conn, WS, &["x".into()], &[], &[], &slots).is_err()
    );
    template::soft_delete(conn, id, WS)?;
    template_installation::mark_source_deleted(conn, id)?;
    template_installation::reset_failed(
        conn,
        &template_installation::ResetFailed {
            id: iid,
            workspace_id: WS,
            installed_by: Some(USER),
            installed_by_kind: "user".into(),
            source_version: Some(1),
        },
    )?;
    template_installation::mark_uninstalled(
        conn,
        iid,
        "installer",
        &template_uninstall::empty_counts(),
    )?;
    Ok(())
}

#[tokio::test]
async fn every_untested_p3_3_function_runs_against_the_schema() {
    let db = fresh_db().await;
    seed(&db).await;
    repo_common::ok(repo::write(&db, |tx| all(tx)).await);
    // A second fresh file: `all` deletes rows of the seed graph.
    let db = fresh_db().await;
    seed(&db).await;
    repo_common::ok(repo::write(&db, |tx| more(tx)).await);
    let db = fresh_db().await;
    seed(&db).await;
    repo_common::ok(repo::write(&db, |tx| templates(tx)).await);
}
