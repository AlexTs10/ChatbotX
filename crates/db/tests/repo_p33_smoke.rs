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

#[tokio::test]
async fn every_untested_p3_3_function_runs_against_the_schema() {
    let db = fresh_db().await;
    seed(&db).await;
    repo_common::ok(repo::write(&db, |tx| all(tx)).await);
    // A second fresh file: `all` deletes rows of the seed graph.
    let db = fresh_db().await;
    seed(&db).await;
    repo_common::ok(repo::write(&db, |tx| more(tx)).await);
}
