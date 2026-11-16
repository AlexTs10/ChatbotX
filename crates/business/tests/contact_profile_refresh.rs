//! Port of `packages/business/__tests__/contact-profile-refresh.test.ts`
//! (rules, capability table, cooldown, `contactProfileRefreshService.refresh`;
//! source lines 113–660) and of
//! `apps/builder/src/features/contacts/lib/__tests__/refresh-contact-profile.test.ts`
//! (P3.6-D2, slice inboxes / ads / refresh).
//!
//! Adapted: the mocked `contactService` / Redis / uploader become a real
//! database, a `MemKv` and an in-memory `Uploader`; "was (not) called"
//! becomes the stored row, the cooldown key and the stored objects. A DB
//! write failure is a temp trigger that aborts `UPDATE "Contact"`.
//!
//! Not applicable: the Redis / uploader / Error-Log failure cases (the
//! in-process stores cannot fail; the Error-Log write is not ported, a
//! failure is a `tracing` warning), "fetch returns undefined" (Rust has one
//! `None`), and the member `accessScope` cases (no member context).
//! Source lines 661–1182 (locale normalization, `ContactInbox.language`,
//! `applyContactProfile*`) are moved (spec P3.6-D2 Decisions).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chatbotx_business::contact::profile_refresh as pr;
use chatbotx_business::contact::profile_refresh::{
    ContactProfileUpdate, IncomingProfile, ProfileNameSource, RefreshOutcome, SkipReason,
};
use chatbotx_db::Db;
use chatbotx_files::Uploader;
use chatbotx_kv::clock::TestClock;
use chatbotx_kv::mem::MemKv;
use common::{
    OTHER_WS, WS, contact, contact_inbox, contact_name, inbox, scalar_text, setup, two_workspaces,
};

const CONTACT: i64 = 10;
const CONTACT_INBOX: i64 = 11;
const COOLDOWN_KEY: &str = "contact-profile-refresh:cooldown:11";
const NEW_AVATAR: &str = "public/space/ws-1/avatars/new";

/// Contact 10 (WS, no name) with messenger contact inbox 11 (`psid-10`, no
/// language); contact 20 of OTHER_WS; webchat contact inbox 12 of contact 10.
async fn fixture() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        inbox(tx, 1, WS, "messenger")?;
        inbox(tx, 2, WS, "webchat")?;
        contact(tx, CONTACT, WS)?;
        contact(tx, 20, OTHER_WS)?;
        contact_inbox(tx, CONTACT_INBOX, CONTACT, 1, "messenger", "psid-10")?;
        contact_inbox(tx, 12, CONTACT, 2, "webchat", "web-10")?;
        Ok(())
    })
    .await;
    db
}

fn kv() -> MemKv {
    MemKv::new(Arc::new(TestClock::new(1_800_000_000_000)))
}

async fn put_object(uploader: &Uploader, path: &str) {
    uploader
        .put_object(path, b"img".to_vec(), Default::default(), None)
        .await
        .expect("put object");
}

async fn object_exists(uploader: &Uploader, path: &str) -> bool {
    uploader.head_object(path).await.is_ok()
}

fn refresh_input(source: ProfileNameSource) -> pr::RefreshInput {
    pr::RefreshInput {
        workspace_id: WS,
        contact_id: CONTACT,
        contact_inbox: pr::RefreshContactInbox {
            id: CONTACT_INBOX,
            channel: "messenger".to_string(),
            contact_id: CONTACT,
            language: None,
            source_id: "psid-10".to_string(),
        },
        source,
    }
}

fn named(first: &str) -> IncomingProfile {
    IncomingProfile {
        source_id: "psid-10".to_string(),
        first_name: Some(first.to_string()),
        ..IncomingProfile::default()
    }
}

async fn first_name(db: &Db) -> Option<String> {
    scalar_text(
        db,
        r#"SELECT "firstName" FROM "Contact" WHERE "id" = ?1"#,
        CONTACT,
    )
    .await
}

async fn last_name(db: &Db) -> Option<String> {
    scalar_text(
        db,
        r#"SELECT "lastName" FROM "Contact" WHERE "id" = ?1"#,
        CONTACT,
    )
    .await
}

async fn abort_contact_updates(db: &Db) {
    setup(db, |tx| {
        tx.execute_batch(
            r#"CREATE TRIGGER "test_fail_contact_update" BEFORE UPDATE ON "Contact"
               BEGIN SELECT RAISE(ABORT, 'db down'); END;"#,
        )?;
        Ok(())
    })
    .await;
}

// ─── PROFILE_NAME_BLANK_CHARACTERS ──────────────────────────────────────

#[test]
fn every_character_in_the_set_is_blank_per_string_prototype_trim_agrees_with_hasemptyprofilename_s_definition_of_blank()
 {
    assert!(!pr::PROFILE_NAME_BLANK_CHARACTERS.is_empty());
    for ch in pr::PROFILE_NAME_BLANK_CHARACTERS.chars() {
        let alone = ch.to_string();
        assert_eq!(pr::js_trim(&alone), "", "{ch:?}");
        assert_eq!(pr::js_trim(&format!("x{ch}")), "x", "{ch:?}");
    }
}

#[test]
fn a_non_whitespace_character_is_not_blank_and_is_not_in_the_set() {
    assert_eq!(pr::js_trim("x"), "x");
    assert!(!pr::PROFILE_NAME_BLANK_CHARACTERS.contains('x'));
}

#[test]
fn contains_the_ecmascript_whitespace_lineterminator_code_points_the_finding_named_tab_newline_nbsp_ideographic_space_bom()
 {
    let set = pr::PROFILE_NAME_BLANK_CHARACTERS;
    assert_eq!(set.chars().count(), 25);
    for ch in ['\t', '\n', '\u{00A0}', '\u{3000}', '\u{FEFF}'] {
        assert!(set.contains(ch), "{ch:?}");
    }
}

// ─── hasEmptyProfileName ────────────────────────────────────────────────

#[test]
fn has_empty_profile_name_both_blank_true() {
    assert!(pr::has_empty_profile_name(None, None));
}

#[test]
fn has_empty_profile_name_both_whitespace_only_true() {
    assert!(pr::has_empty_profile_name(Some("   "), Some("  ")));
}

#[test]
fn has_empty_profile_name_tab_only_firstname_newline_only_lastname_true() {
    assert!(pr::has_empty_profile_name(Some("\t"), Some("\n")));
}

#[test]
fn has_empty_profile_name_nbsp_only_firstname_ideographic_space_only_lastname_true() {
    assert!(pr::has_empty_profile_name(
        Some("\u{00A0}"),
        Some("\u{3000}")
    ));
}

#[test]
fn has_empty_profile_name_firstname_only_false() {
    assert!(!pr::has_empty_profile_name(Some("Jane"), None));
}

#[test]
fn has_empty_profile_name_lastname_only_false() {
    assert!(!pr::has_empty_profile_name(None, Some("Doe")));
}

// ─── hasProfileName ─────────────────────────────────────────────────────

fn update_named(first: Option<&str>, last: Option<&str>) -> ContactProfileUpdate {
    ContactProfileUpdate {
        first_name: first.map(str::to_string),
        last_name: last.map(str::to_string),
        ..ContactProfileUpdate::default()
    }
}

#[test]
fn has_profile_name_both_blank_false() {
    assert!(!pr::has_profile_name(&ContactProfileUpdate::default()));
}

#[test]
fn has_profile_name_both_whitespace_only_false() {
    assert!(!pr::has_profile_name(&update_named(
        Some("   "),
        Some("  ")
    )));
}

#[test]
fn has_profile_name_tab_only_firstname_and_nbsp_only_lastname_false() {
    assert!(!pr::has_profile_name(&update_named(
        Some("\t"),
        Some("\u{00A0}")
    )));
}

#[test]
fn has_profile_name_firstname_only_true() {
    assert!(pr::has_profile_name(&update_named(Some("Jane"), None)));
}

#[test]
fn has_profile_name_lastname_only_true() {
    assert!(pr::has_profile_name(&update_named(None, Some("Doe"))));
}

// ─── buildContactProfileUpdate ──────────────────────────────────────────

#[test]
fn build_contact_profile_update_drops_undefined_fields() {
    let update = pr::build_contact_profile_update(&named("Jane"));
    assert_eq!(update, update_named(Some("Jane"), None));
}

#[test]
fn build_contact_profile_update_keeps_every_field_the_profile_returned() {
    let update = pr::build_contact_profile_update(&IncomingProfile {
        source_id: "psid-1".to_string(),
        first_name: Some("Jane".to_string()),
        last_name: Some("Doe".to_string()),
        avatar: Some("public/space/ws-1/avatars/abc".to_string()),
        locale: Some("en_US".to_string()),
        timezone: Some("+07:00".to_string()),
        gender: Some("female".to_string()),
        language: None,
    });
    assert_eq!(
        update,
        ContactProfileUpdate {
            first_name: Some("Jane".to_string()),
            last_name: Some("Doe".to_string()),
            avatar: Some("public/space/ws-1/avatars/abc".to_string()),
            locale: Some("en_US".to_string()),
            timezone: Some("+07:00".to_string()),
            gender: Some("female".to_string()),
        }
    );
}

#[test]
fn build_contact_profile_update_invalid_gender_string_is_dropped() {
    let update = pr::build_contact_profile_update(&IncomingProfile {
        gender: Some("not-a-real-gender".to_string()),
        ..named("Jane")
    });
    assert_eq!(update, update_named(Some("Jane"), None));
    assert_eq!(update.gender, None);
}

/// Adapted: `ContactProfileUpdate` has exactly the six writable columns (no
/// `fullName` field exists), so a full profile maps to those six only.
#[test]
fn build_contact_profile_update_never_emits_fullname_generated_column() {
    let update = pr::build_contact_profile_update(&IncomingProfile {
        last_name: Some("Doe".to_string()),
        ..named("Jane")
    });
    assert_eq!(update, update_named(Some("Jane"), Some("Doe")));
}

// ─── capability table ───────────────────────────────────────────────────

#[test]
fn resolve_inbound_profile_name_source_per_channel() {
    let expected = [
        ("messenger", Some(ProfileNameSource::ChannelApi)),
        ("instagram", Some(ProfileNameSource::ChannelApi)),
        ("zalo", Some(ProfileNameSource::ChannelApi)),
        ("telegram", Some(ProfileNameSource::ChannelApi)),
        ("whatsapp", Some(ProfileNameSource::Payload)),
        ("api", Some(ProfileNameSource::Payload)),
        ("tiktok", None),
        ("threads", None),
        ("webchat", None),
        ("smtp", None),
        ("omnichannel", None),
    ];
    for (channel, source) in expected {
        assert_eq!(
            pr::resolve_inbound_profile_name_source(channel),
            source,
            "{channel}"
        );
    }
}

#[test]
fn has_on_demand_profile_api_is_true_for_exactly_messenger_instagram_zalo_telegram() {
    let on_demand = ["messenger", "instagram", "zalo", "telegram"];
    for channel in pr::CHANNEL_TYPES {
        assert_eq!(
            pr::has_on_demand_profile_api(channel),
            on_demand.contains(channel),
            "{channel}"
        );
    }
}

#[test]
fn every_channel_types_options_value_has_a_capability_row() {
    assert_eq!(pr::CHANNEL_TYPES.len(), 11);
    for channel in pr::CHANNEL_TYPES {
        assert!(pr::capability(channel).is_some(), "{channel}");
    }
}

#[test]
fn resolve_inbound_profile_name_source_returns_null_for_an_unknown_channel_string_never_throws() {
    assert_eq!(pr::resolve_inbound_profile_name_source("legacy"), None);
}

#[test]
fn has_on_demand_profile_api_returns_false_for_an_unknown_channel_string_never_throws() {
    assert!(!pr::has_on_demand_profile_api("legacy"));
}

#[test]
fn cooldown_by_profile_source_payload_is_not_rate_limited_channelapi_is() {
    assert!(!pr::cooldown_by_profile_source(ProfileNameSource::Payload));
    assert!(pr::cooldown_by_profile_source(
        ProfileNameSource::ChannelApi
    ));
}

// ─── cooldown ───────────────────────────────────────────────────────────

#[test]
fn is_contact_profile_refresh_cooling_down_true_while_the_key_exists() {
    let kv = kv();
    kv.set_number(COOLDOWN_KEY, 1.0, 300);
    assert!(pr::is_cooling_down(Some(&kv), CONTACT_INBOX));
}

#[test]
fn is_contact_profile_refresh_cooling_down_false_while_the_key_is_absent() {
    let kv = kv();
    assert!(!pr::is_cooling_down(Some(&kv), CONTACT_INBOX));
    assert!(!pr::is_cooling_down(None, CONTACT_INBOX));
}

#[test]
fn start_contact_profile_refresh_cooldown_writes_the_key_with_a_300s_ttl() {
    let kv = kv();
    pr::start_cooldown(Some(&kv), CONTACT_INBOX);
    assert!(kv.exists(COOLDOWN_KEY));
    assert_eq!(kv.ttl_secs(COOLDOWN_KEY), Some(300));
    assert_eq!(pr::PROFILE_REFRESH_COOLDOWN_SECONDS, 300);
}

// ─── contactProfileRefreshService.refresh ───────────────────────────────

#[tokio::test]
async fn name_present_skipped_profilecomplete_no_cooldown_check_no_fetch() {
    let db = fixture().await;
    setup(&db, |tx| {
        contact_name(tx, CONTACT, Some("Jane"), None, None)
    })
    .await;
    let kv = kv();
    kv.set_number(COOLDOWN_KEY, 1.0, 300);
    let fetched = AtomicBool::new(false);
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: None,
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::ChannelApi),
        || async {
            fetched.store(true, Ordering::SeqCst);
            Ok(None)
        },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Skipped(SkipReason::ProfileComplete));
    assert!(!fetched.load(Ordering::SeqCst));
}

#[tokio::test]
async fn payload_source_cooldown_never_checked_nor_started_unavailable_outcome() {
    let db = fixture().await;
    let kv = kv();
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: None,
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::Payload),
        || async { Ok(None) },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Unavailable);
    assert!(!kv.exists(COOLDOWN_KEY));
}

#[tokio::test]
async fn payload_source_cooldown_never_checked_nor_started_failed_outcome() {
    let db = fixture().await;
    let kv = kv();
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: None,
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::Payload),
        || async { Err("resolution failed".to_string()) },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Failed);
    assert!(!kv.exists(COOLDOWN_KEY));
}

#[tokio::test]
async fn cooling_down_skipped_coolingdown_no_fetch() {
    let db = fixture().await;
    let kv = kv();
    kv.set_number(COOLDOWN_KEY, 1.0, 300);
    let fetched = AtomicBool::new(false);
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: None,
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::ChannelApi),
        || async {
            fetched.store(true, Ordering::SeqCst);
            Ok(Some(named("Jane")))
        },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Skipped(SkipReason::CoolingDown));
    assert!(!fetched.load(Ordering::SeqCst));
    assert_eq!(first_name(&db).await, None);
}

#[tokio::test]
async fn fetch_throws_failed_error_recorded_cooldown_started_contact_untouched() {
    let db = fixture().await;
    let kv = kv();
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: None,
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::ChannelApi),
        || async { Err("graph error".to_string()) },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Failed);
    assert_eq!(kv.ttl_secs(COOLDOWN_KEY), Some(300));
    assert_eq!(first_name(&db).await, None);
}

#[tokio::test]
async fn fetch_returns_null_unavailable_cooldown_started_no_write() {
    let db = fixture().await;
    let kv = kv();
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: None,
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::ChannelApi),
        || async { Ok(None) },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Unavailable);
    assert!(kv.exists(COOLDOWN_KEY));
    assert_eq!(first_name(&db).await, None);
}

#[tokio::test]
async fn applycontactprofile_throws_failed_cooldown_started_new_upload_deleted_best_effort() {
    let db = fixture().await;
    abort_contact_updates(&db).await;
    let kv = kv();
    let uploader = Uploader::in_memory();
    put_object(&uploader, NEW_AVATAR).await;
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: Some(&uploader),
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::ChannelApi),
        || async {
            Ok(Some(IncomingProfile {
                avatar: Some(NEW_AVATAR.to_string()),
                ..named("Jane")
            }))
        },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Failed);
    assert!(!object_exists(&uploader, NEW_AVATAR).await);
    assert!(kv.exists(COOLDOWN_KEY));
    assert_eq!(first_name(&db).await, None);
}

#[tokio::test]
async fn fetch_returns_avatar_locale_but_no_name_unavailable_no_update_managed_avatar_deleted_cooldown_started()
 {
    let db = fixture().await;
    let kv = kv();
    let uploader = Uploader::in_memory();
    let orphan = "public/space/ws-1/avatars/orphan";
    put_object(&uploader, orphan).await;
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: Some(&uploader),
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::ChannelApi),
        || async {
            Ok(Some(IncomingProfile {
                source_id: "psid-10".to_string(),
                avatar: Some(orphan.to_string()),
                locale: Some("en_US".to_string()),
                ..IncomingProfile::default()
            }))
        },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Unavailable);
    assert!(!object_exists(&uploader, orphan).await);
    assert!(kv.exists(COOLDOWN_KEY));
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "locale" FROM "Contact" WHERE "id" = ?1"#,
            CONTACT
        )
        .await,
        None
    );
}

#[tokio::test]
async fn no_usable_name_and_an_external_unmanaged_avatar_left_untouched() {
    let db = fixture().await;
    let kv = kv();
    let uploader = Uploader::in_memory();
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: Some(&uploader),
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::ChannelApi),
        || async {
            Ok(Some(IncomingProfile {
                source_id: "psid-10".to_string(),
                avatar: Some("https://cdn.example.com/pic.jpg".to_string()),
                ..IncomingProfile::default()
            }))
        },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Unavailable);
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "avatar" FROM "Contact" WHERE "id" = ?1"#,
            CONTACT
        )
        .await,
        None
    );
}

#[tokio::test]
async fn no_usable_name_and_no_avatar_at_all_nothing_to_delete() {
    let db = fixture().await;
    let kv = kv();
    let uploader = Uploader::in_memory();
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: Some(&uploader),
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::ChannelApi),
        || async {
            Ok(Some(IncomingProfile {
                source_id: "psid-10".to_string(),
                locale: Some("en_US".to_string()),
                ..IncomingProfile::default()
            }))
        },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Unavailable);
}

#[tokio::test]
async fn happy_path_updated_contactservice_updateifprofilenameempty_called_once_with_mapped_data_no_cooldown_started()
 {
    let db = fixture().await;
    let kv = kv();
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: None,
    };
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::ChannelApi),
        || async {
            Ok(Some(IncomingProfile {
                last_name: Some("Doe".to_string()),
                ..named("Jane")
            }))
        },
    )
    .await
    .expect("refresh");
    let RefreshOutcome::Updated(contact) = result else {
        panic!("expected updated, got {result:?}");
    };
    assert_eq!(contact.id, CONTACT);
    assert_eq!(contact.first_name.as_deref(), Some("Jane"));
    assert_eq!(contact.last_name.as_deref(), Some("Doe"));
    assert_eq!(first_name(&db).await.as_deref(), Some("Jane"));
    assert_eq!(last_name(&db).await.as_deref(), Some("Doe"));
    assert!(!kv.exists(COOLDOWN_KEY));
}

#[tokio::test]
async fn name_filled_between_fetch_and_write_skipped_profilecomplete_no_update_uploaded_avatar_discarded_no_cooldown()
 {
    let db = fixture().await;
    let kv = kv();
    let uploader = Uploader::in_memory();
    put_object(&uploader, NEW_AVATAR).await;
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: Some(&uploader),
    };
    let writer = &db;
    let result = pr::refresh(
        &db,
        services,
        refresh_input(ProfileNameSource::ChannelApi),
        || async move {
            setup(writer, |tx| {
                contact_name(tx, CONTACT, Some("Raced"), None, None)
            })
            .await;
            Ok(Some(IncomingProfile {
                avatar: Some(NEW_AVATAR.to_string()),
                ..named("Jane")
            }))
        },
    )
    .await
    .expect("refresh");
    assert_eq!(result, RefreshOutcome::Skipped(SkipReason::ProfileComplete));
    assert_eq!(first_name(&db).await.as_deref(), Some("Raced"));
    assert!(!object_exists(&uploader, NEW_AVATAR).await);
    assert!(!kv.exists(COOLDOWN_KEY));
}

// ─── refreshContactProfile (builder lib) ────────────────────────────────

#[tokio::test]
async fn refresh_contact_profile_throws_before_any_inbox_channel_work_when_the_contact_is_out_of_scope()
 {
    let db = fixture().await;
    let kv = kv();
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: None,
    };
    let error = pr::refresh_contact_profile(&db, services, WS, 20, CONTACT_INBOX)
        .await
        .expect_err("contact of another workspace");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.to_string(), "Contact not found");
    assert!(!kv.exists(COOLDOWN_KEY));
}

#[tokio::test]
async fn refresh_contact_profile_throws_when_the_contact_inbox_does_not_exist() {
    let db = fixture().await;
    let services = pr::RefreshServices {
        kv: None,
        uploader: None,
    };
    let error = pr::refresh_contact_profile(&db, services, WS, CONTACT, 999)
        .await
        .expect_err("unknown contact inbox");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.to_string(), "Contact inbox not found");
}

#[tokio::test]
async fn refresh_contact_profile_forwards_the_service_s_channelnotcapable_skip_for_a_non_capable_channel_without_ever_invoking_a_fetcher_factory()
 {
    let db = fixture().await;
    let kv = kv();
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: None,
    };
    let result = pr::refresh_contact_profile(&db, services, WS, CONTACT, 12)
        .await
        .expect("refresh");
    assert_eq!(
        result,
        RefreshOutcome::Skipped(SkipReason::ChannelNotCapable)
    );
    assert!(!kv.exists("contact-profile-refresh:cooldown:12"));
}

/// Adapted: the messenger fetcher resolves the inbox's integration; with no
/// integration it fails, so the service result is `failed` and the cooldown
/// starts (the TS case mocks the factory and the service).
#[tokio::test]
async fn refresh_contact_profile_refreshes_via_the_resolved_channel_fetcher_and_forwards_the_service_result()
 {
    let db = fixture().await;
    let kv = kv();
    let services = pr::RefreshServices {
        kv: Some(&kv),
        uploader: None,
    };
    let result = pr::refresh_contact_profile(&db, services, WS, CONTACT, CONTACT_INBOX)
        .await
        .expect("refresh");
    assert_eq!(result, RefreshOutcome::Failed);
    assert!(kv.exists(COOLDOWN_KEY));
    let again = pr::refresh_contact_profile(&db, services, WS, CONTACT, CONTACT_INBOX)
        .await
        .expect("refresh");
    assert_eq!(again, RefreshOutcome::Skipped(SkipReason::CoolingDown));
}
