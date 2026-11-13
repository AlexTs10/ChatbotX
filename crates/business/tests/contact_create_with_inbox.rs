//! Port of `packages/business/__tests__/contact-create-with-inbox.test.ts`
//! (P3.6-D2). Adapted: the mocked inbox / workspace / contact / contact-inbox
//! services and the quota helper become a migrated database; the asserted
//! mock calls become the stored `Contact`, `ContactInbox` and
//! `Conversation` rows. Not applicable: the audit record, the
//! `emitContactCreated` / analytics events and the quota helper call (no
//! audit log, event bus or quota enforcement in the Rust port yet).

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::contact::{
    CONTACT_ALREADY_ON_INBOX_MESSAGE, CreateContactInput, CreateOptions, PhoneUniqueness,
    create_with_inbox,
};
use common::{
    TestDb, WS, contact_inbox, contact_with, inbox, rows, scalar_i64, setup, target_country,
    two_workspaces,
};
use serde_json::{Map, Value};

const WEBCHAT: i64 = 3001;
const MESSENGER: i64 = 3002;
const WHATSAPP: i64 = 3003;
const SMTP: i64 = 3004;
const BSUID: &str = "US.13491208655302741918";

async fn db_with(country: Option<&'static str>) -> TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        inbox(tx, WEBCHAT, WS, "webchat")?;
        inbox(tx, MESSENGER, WS, "messenger")?;
        inbox(tx, WHATSAPP, WS, "whatsapp")?;
        inbox(tx, SMTP, WS, "smtp")?;
        if let Some(country) = country {
            target_country(tx, WS, country)?;
        }
        Ok(())
    })
    .await;
    db
}

fn base_input() -> CreateContactInput {
    CreateContactInput {
        email: "ada@example.com".to_string(),
        first_name: Some("Ada".to_string()),
        gender: "unknown".to_string(),
        inbox_id: WEBCHAT,
        phone_number: Some("+15551234567".to_string()),
        channel: "webchat".to_string(),
        contact_id: Some(String::new()),
        ..CreateContactInput::default()
    }
}

fn on(channel: &str, inbox_id: i64) -> CreateContactInput {
    CreateContactInput {
        channel: channel.to_string(),
        inbox_id,
        email: String::new(),
        ..base_input()
    }
}

async fn inbox_rows(db: &TestDb, inbox_id: i64) -> Vec<Map<String, Value>> {
    rows(
        db,
        r#"SELECT "contactId", "originalContactId", "inboxId", "channel", "source", "sourceId", "sourceUserId"
           FROM "ContactInbox" WHERE "inboxId" = ?1 ORDER BY "id""#,
        inbox_id,
    )
    .await
}

async fn contact_count(db: &TestDb) -> i64 {
    scalar_i64(
        db,
        r#"SELECT COUNT(*) FROM "Contact" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .unwrap_or_default()
}

fn field_error(error: BusinessError) -> (Option<String>, String) {
    match error {
        BusinessError::Exception { field, message, .. } => (field, message),
        other => panic!("expected a ChatbotXException, got {other:?}"),
    }
}

async fn create(db: &TestDb, input: CreateContactInput) -> chatbotx_business::Result<i64> {
    create_with_inbox(db, WS, input, CreateOptions::default())
        .await
        .map(|created| created.contact.id)
}

async fn rejects(db: &TestDb, input: CreateContactInput) -> (Option<String>, String) {
    let error = create(db, input).await.expect_err("validation error");
    field_error(error)
}

#[tokio::test]
async fn creates_manual_contacts_through_the_no_mac_quota_helper() {
    let db = db_with(None).await;
    let created = create_with_inbox(&db, WS, base_input(), CreateOptions::default())
        .await
        .expect("created");
    assert_eq!(created.contact.workspace_id, WS);
    assert_eq!(created.contact.first_name.as_deref(), Some("Ada"));
    assert_eq!(
        created.contact.phone_number.as_deref(),
        Some("+15551234567")
    );
    assert_eq!(created.contact.email.as_deref(), Some("ada@example.com"));
    assert_eq!(created.contact_inbox.contact_id, created.contact.id);
    // One contact, its contact inbox and its open conversation.
    assert_eq!(contact_count(&db).await, 1);
    let conversations = scalar_i64(
        &db,
        r#"SELECT COUNT(*) FROM "Conversation" WHERE "contactId" = ?1"#,
        created.contact.id,
    )
    .await;
    assert_eq!(conversations, Some(1));
}

#[tokio::test]
async fn attaches_manual_contacts_to_the_selected_workspace_inbox() {
    let db = db_with(None).await;
    let id = create(
        &db,
        CreateContactInput {
            inbox_id: MESSENGER,
            channel: "messenger".to_string(),
            contact_id: Some("psid-123".to_string()),
            ..base_input()
        },
    )
    .await
    .expect("created");
    let contact = rows(
        &db,
        r#"SELECT "email", "firstName", "gender", "phoneNumber" FROM "Contact" WHERE "id" = ?1"#,
        id,
    )
    .await;
    assert_eq!(contact[0]["email"], "ada@example.com");
    assert_eq!(contact[0]["firstName"], "Ada");
    assert_eq!(contact[0]["gender"], "unknown");
    assert_eq!(contact[0]["phoneNumber"], "+15551234567");
    let inboxes = inbox_rows(&db, MESSENGER).await;
    assert_eq!(inboxes.len(), 1);
    assert_eq!(inboxes[0]["channel"], "messenger");
    assert_eq!(inboxes[0]["inboxId"], MESSENGER);
    assert_eq!(inboxes[0]["source"], "direct");
    assert_eq!(inboxes[0]["sourceId"], "psid-123");
    assert_eq!(inboxes[0]["contactId"], id);
    assert_eq!(inboxes[0]["originalContactId"], id);
}

#[tokio::test]
async fn uses_whatsapp_phone_digits_as_the_source_id() {
    let db = db_with(None).await;
    create(&db, on("whatsapp", WHATSAPP))
        .await
        .expect("created");
    let inboxes = inbox_rows(&db, WHATSAPP).await;
    assert_eq!(inboxes[0]["sourceId"], "15551234567");
}

#[tokio::test]
async fn normalizes_local_whatsapp_phone_numbers_with_the_workspace_target_country() {
    let db = db_with(Some("VN")).await;
    let id = create(
        &db,
        CreateContactInput {
            phone_number: Some("0901234567".to_string()),
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await
    .expect("created");
    let phone = rows(
        &db,
        r#"SELECT "phoneNumber" FROM "Contact" WHERE "id" = ?1"#,
        id,
    )
    .await;
    assert_eq!(phone[0]["phoneNumber"], "+84901234567");
    assert_eq!(
        inbox_rows(&db, WHATSAPP).await[0]["sourceId"],
        "84901234567"
    );
}

#[tokio::test]
async fn an_explicit_country_code_in_the_phone_beats_the_workspace_target_country() {
    let db = db_with(Some("VN")).await;
    let id = create(&db, on("whatsapp", WHATSAPP))
        .await
        .expect("created");
    let phone = rows(
        &db,
        r#"SELECT "phoneNumber" FROM "Contact" WHERE "id" = ?1"#,
        id,
    )
    .await;
    assert_eq!(phone[0]["phoneNumber"], "+15551234567");
    assert_eq!(
        inbox_rows(&db, WHATSAPP).await[0]["sourceId"],
        "15551234567"
    );
}

#[tokio::test]
async fn strips_separators_before_normalization() {
    let db = db_with(Some("US")).await;
    let id = create(
        &db,
        CreateContactInput {
            phone_number: Some("(555) 123-4567".to_string()),
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await
    .expect("created");
    let phone = rows(
        &db,
        r#"SELECT "phoneNumber" FROM "Contact" WHERE "id" = ?1"#,
        id,
    )
    .await;
    assert_eq!(phone[0]["phoneNumber"], "+15551234567");
    assert_eq!(
        inbox_rows(&db, WHATSAPP).await[0]["sourceId"],
        "15551234567"
    );
}

#[tokio::test]
async fn keeps_an_already_international_format_number_as_is() {
    let db = db_with(Some("VN")).await;
    let id = create(
        &db,
        CreateContactInput {
            phone_number: Some("+84901234567".to_string()),
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await
    .expect("created");
    let phone = rows(
        &db,
        r#"SELECT "phoneNumber" FROM "Contact" WHERE "id" = ?1"#,
        id,
    )
    .await;
    assert_eq!(phone[0]["phoneNumber"], "+84901234567");
    assert_eq!(
        inbox_rows(&db, WHATSAPP).await[0]["sourceId"],
        "84901234567"
    );
}

#[tokio::test]
async fn normalizes_a_non_whatsapp_channel_phone_number_against_the_resolved_country() {
    let db = db_with(Some("VN")).await;
    let id = create(
        &db,
        CreateContactInput {
            phone_number: Some("0901234567".to_string()),
            contact_id: Some("psid-123".to_string()),
            ..on("messenger", MESSENGER)
        },
    )
    .await
    .expect("created");
    let phone = rows(
        &db,
        r#"SELECT "phoneNumber" FROM "Contact" WHERE "id" = ?1"#,
        id,
    )
    .await;
    assert_eq!(phone[0]["phoneNumber"], "+84901234567");
    assert_eq!(inbox_rows(&db, MESSENGER).await[0]["sourceId"], "psid-123");
}

#[tokio::test]
async fn an_unresolvable_non_whatsapp_phone_number_produces_a_validation_error() {
    let db = db_with(None).await;
    let (field, message) = rejects(
        &db,
        CreateContactInput {
            phone_number: Some("0901234567".to_string()),
            contact_id: Some("psid-123".to_string()),
            ..on("messenger", MESSENGER)
        },
    )
    .await;
    assert_eq!(field.as_deref(), Some("phoneNumber"));
    assert_eq!(message, "Please include the country code (e.g. +84)");
    assert_eq!(contact_count(&db).await, 0);
}

#[tokio::test]
async fn requires_a_country_code_for_local_whatsapp_phone_numbers_when_target_country_is_unset_or_unknown()
 {
    for country in [None, Some("unknown")] {
        let db = db_with(country).await;
        let (field, message) = rejects(
            &db,
            CreateContactInput {
                phone_number: Some("0901234567".to_string()),
                ..on("whatsapp", WHATSAPP)
            },
        )
        .await;
        assert_eq!(field.as_deref(), Some("phoneNumber"), "{country:?}");
        assert_eq!(message, "Please include the country code (e.g. +84)");
        assert_eq!(contact_count(&db).await, 0);
    }
}

#[tokio::test]
async fn uses_email_as_the_source_id_for_smtp_contacts() {
    let db = db_with(None).await;
    create(
        &db,
        CreateContactInput {
            email: "ada@example.com".to_string(),
            phone_number: Some(String::new()),
            ..on("smtp", SMTP)
        },
    )
    .await
    .expect("created");
    assert_eq!(
        inbox_rows(&db, SMTP).await[0]["sourceId"],
        "ada@example.com"
    );
}

#[tokio::test]
async fn uses_a_synthetic_source_id_for_webchat_contacts() {
    let db = db_with(None).await;
    create(
        &db,
        CreateContactInput {
            phone_number: Some(String::new()),
            ..on("webchat", WEBCHAT)
        },
    )
    .await
    .expect("created");
    create(
        &db,
        CreateContactInput {
            phone_number: Some(String::new()),
            ..on("webchat", WEBCHAT)
        },
    )
    .await
    .expect("a second webchat contact is not a duplicate");
    let inboxes = inbox_rows(&db, WEBCHAT).await;
    let first = inboxes[0]["sourceId"].as_str().expect("sourceId");
    let second = inboxes[1]["sourceId"].as_str().expect("sourceId");
    // `randomString(10)` + `createId()`.
    assert!(first.len() > 10, "{first}");
    assert!(first[10..].chars().all(|c| c.is_ascii_digit()), "{first}");
    assert_ne!(first, second);
}

#[tokio::test]
async fn rejects_when_the_selected_inbox_channel_does_not_match_the_requested_source() {
    let db = db_with(None).await;
    let (field, message) = rejects(
        &db,
        CreateContactInput {
            inbox_id: MESSENGER,
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await;
    assert_eq!(field.as_deref(), Some("inboxId"));
    assert_eq!(message, "Selected inbox does not match the selected source");
    assert_eq!(contact_count(&db).await, 0);
}

#[tokio::test]
async fn returns_a_field_validation_error_when_the_selected_inbox_identity_already_exists() {
    let db = db_with(None).await;
    setup(&db, |tx| {
        contact_with(tx, 9001, WS, Some("Old"), None, None)?;
        contact_inbox(tx, 9101, 9001, MESSENGER, "messenger", "psid-123")?;
        Ok(())
    })
    .await;
    let (field, message) = rejects(
        &db,
        CreateContactInput {
            phone_number: Some(String::new()),
            contact_id: Some("psid-123".to_string()),
            ..on("messenger", MESSENGER)
        },
    )
    .await;
    assert_eq!(field.as_deref(), Some("contactId"));
    assert_eq!(message, CONTACT_ALREADY_ON_INBOX_MESSAGE);
    assert_eq!(contact_count(&db).await, 1);
}

#[tokio::test]
async fn dedups_whatsapp_by_the_normalized_phone_number() {
    let db = db_with(Some("VN")).await;
    setup(&db, |tx| {
        contact_with(tx, 9001, WS, Some("Old"), None, Some("+84901234567"))?;
        Ok(())
    })
    .await;
    let (field, message) = rejects(
        &db,
        CreateContactInput {
            phone_number: Some("0901234567".to_string()),
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await;
    assert_eq!(field.as_deref(), Some("phoneNumber"));
    assert_eq!(message, "Phone number is exists");
    assert_eq!(contact_count(&db).await, 1);
}

#[tokio::test]
async fn maps_a_duplicate_whatsapp_wa_id_to_the_phone_number_field() {
    let db = db_with(Some("VN")).await;
    setup(&db, |tx| {
        // Another phone on the contact, so only the inbox identity collides.
        contact_with(tx, 9001, WS, Some("Old"), None, None)?;
        contact_inbox(tx, 9101, 9001, WHATSAPP, "whatsapp", "84901234567")?;
        Ok(())
    })
    .await;
    let (field, message) = rejects(
        &db,
        CreateContactInput {
            phone_number: Some("0901234567".to_string()),
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await;
    assert_eq!(field.as_deref(), Some("phoneNumber"));
    assert_eq!(message, CONTACT_ALREADY_ON_INBOX_MESSAGE);
}

#[tokio::test]
async fn maps_a_duplicate_smtp_identity_to_the_email_field() {
    let db = db_with(None).await;
    setup(&db, |tx| {
        contact_with(tx, 9001, WS, Some("Old"), None, None)?;
        contact_inbox(tx, 9101, 9001, SMTP, "smtp", "ada@example.com")?;
        Ok(())
    })
    .await;
    let (field, message) = rejects(
        &db,
        CreateContactInput {
            email: "ada@example.com".to_string(),
            phone_number: Some(String::new()),
            ..on("smtp", SMTP)
        },
    )
    .await;
    assert_eq!(field.as_deref(), Some("email"));
    assert_eq!(message, CONTACT_ALREADY_ON_INBOX_MESSAGE);
}

#[tokio::test]
async fn skips_the_phone_dedup_when_no_phone_is_provided() {
    let db = db_with(None).await;
    setup(&db, |tx| {
        // A contact without a phone must not count as a phone match.
        contact_with(tx, 9001, WS, Some("Old"), None, None)?;
        Ok(())
    })
    .await;
    create(
        &db,
        CreateContactInput {
            email: "ada@example.com".to_string(),
            phone_number: Some(String::new()),
            ..on("smtp", SMTP)
        },
    )
    .await
    .expect("created");
    assert_eq!(contact_count(&db).await, 2);
}

#[tokio::test]
async fn records_the_direct_source_unless_the_caller_names_another() {
    let db = db_with(None).await;
    create(&db, base_input()).await.expect("created");
    create_with_inbox(
        &db,
        WS,
        CreateContactInput {
            phone_number: Some("+15551234568".to_string()),
            ..base_input()
        },
        CreateOptions {
            source: "inboundWebhook".to_string(),
            ..CreateOptions::default()
        },
    )
    .await
    .expect("created");
    let inboxes = inbox_rows(&db, WEBCHAT).await;
    assert_eq!(inboxes[0]["source"], "direct");
    assert_eq!(inboxes[1]["source"], "inboundWebhook");
}

#[tokio::test]
async fn keeps_the_workspace_wide_phone_check_by_default() {
    let db = db_with(None).await;
    setup(&db, |tx| {
        contact_with(tx, 9001, WS, Some("Old"), None, Some("+15551234567"))?;
        Ok(())
    })
    .await;
    let (field, message) = rejects(&db, base_input()).await;
    assert_eq!(field.as_deref(), Some("phoneNumber"));
    assert_eq!(message, "Phone number is exists");
}

#[tokio::test]
async fn scopes_phone_identity_to_the_inbox_when_asked_so_another_channels_contact_does_not_block_it()
 {
    let db = db_with(None).await;
    setup(&db, |tx| {
        contact_with(tx, 9001, WS, Some("Old"), None, Some("+15551234567"))?;
        contact_inbox(tx, 9101, 9001, WEBCHAT, "webchat", "web-1")?;
        Ok(())
    })
    .await;
    create_with_inbox(
        &db,
        WS,
        on("whatsapp", WHATSAPP),
        CreateOptions {
            phone_uniqueness: PhoneUniqueness::Inbox,
            ..CreateOptions::default()
        },
    )
    .await
    .expect("created");
    assert_eq!(contact_count(&db).await, 2);
}

#[tokio::test]
async fn still_rejects_a_contact_already_on_the_same_inbox_when_phone_identity_is_inbox_scoped() {
    let db = db_with(None).await;
    setup(&db, |tx| {
        contact_with(tx, 9001, WS, Some("Old"), None, Some("+15551234567"))?;
        contact_inbox(tx, 9101, 9001, WHATSAPP, "whatsapp", "15551234567")?;
        Ok(())
    })
    .await;
    let error = create_with_inbox(
        &db,
        WS,
        on("whatsapp", WHATSAPP),
        CreateOptions {
            phone_uniqueness: PhoneUniqueness::Inbox,
            ..CreateOptions::default()
        },
    )
    .await
    .expect_err("duplicate");
    assert_eq!(field_error(error).1, CONTACT_ALREADY_ON_INBOX_MESSAGE);
}

#[tokio::test]
async fn emits_contact_created_and_analytics_events_on_success() {
    // Adapted: no event bus; the event inputs (contact id, first name, phone,
    // email, contact inbox id) are the returned rows.
    let db = db_with(None).await;
    let created = create_with_inbox(
        &db,
        WS,
        CreateContactInput {
            phone_number: Some(String::new()),
            ..base_input()
        },
        CreateOptions::default(),
    )
    .await
    .expect("created");
    assert_eq!(created.contact.first_name.as_deref(), Some("Ada"));
    assert_eq!(created.contact.email.as_deref(), Some("ada@example.com"));
    assert_eq!(created.contact_inbox.channel, "webchat");
    assert_eq!(created.contact_inbox.inbox_id, WEBCHAT);
    assert!(!created.contact_inbox.source_id.is_empty());
}

#[tokio::test]
async fn bsuid_keys_a_bsuid_only_contact_by_the_bsuid_so_sends_route_via_recipient() {
    let db = db_with(None).await;
    setup(&db, |tx| {
        // A contact without a phone must not count as a phone match.
        contact_with(tx, 9001, WS, Some("Old"), None, None)?;
        Ok(())
    })
    .await;
    let id = create(
        &db,
        CreateContactInput {
            phone_number: Some(String::new()),
            source_user_id: Some(BSUID.to_string()),
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await
    .expect("created");
    let inboxes = inbox_rows(&db, WHATSAPP).await;
    assert_eq!(inboxes[0]["sourceId"], BSUID);
    assert_eq!(inboxes[0]["sourceUserId"], BSUID);
    let contact = rows(
        &db,
        r#"SELECT "phoneNumber" FROM "Contact" WHERE "id" = ?1"#,
        id,
    )
    .await;
    assert_eq!(contact[0]["phoneNumber"], "");
}

#[tokio::test]
async fn bsuid_keys_by_phone_digits_and_stores_the_bsuid_alongside_when_both_are_given() {
    let db = db_with(None).await;
    create(
        &db,
        CreateContactInput {
            source_user_id: Some(BSUID.to_string()),
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await
    .expect("created");
    let inboxes = inbox_rows(&db, WHATSAPP).await;
    assert_eq!(inboxes[0]["sourceId"], "15551234567");
    assert_eq!(inboxes[0]["sourceUserId"], BSUID);
}

#[tokio::test]
async fn bsuid_rejects_a_bsuid_already_on_the_inbox_on_the_source_user_id_field() {
    let db = db_with(None).await;
    setup(&db, |tx| {
        contact_with(tx, 9001, WS, Some("Old"), None, None)?;
        contact_inbox(tx, 9101, 9001, WHATSAPP, "whatsapp", "4400000000")?;
        tx.execute(
            r#"UPDATE "ContactInbox" SET "sourceUserId" = ?1 WHERE "id" = 9101"#,
            [BSUID],
        )?;
        Ok(())
    })
    .await;
    let (field, _) = rejects(
        &db,
        CreateContactInput {
            source_user_id: Some(BSUID.to_string()),
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await;
    assert_eq!(field.as_deref(), Some("sourceUserId"));
    assert_eq!(contact_count(&db).await, 1);
}

#[tokio::test]
async fn bsuid_reports_a_bsuid_keyed_duplicate_source_id_on_the_source_user_id_field() {
    let db = db_with(None).await;
    setup(&db, |tx| {
        contact_with(tx, 9001, WS, Some("Old"), None, None)?;
        contact_inbox(tx, 9101, 9001, WHATSAPP, "whatsapp", BSUID)?;
        Ok(())
    })
    .await;
    let (field, _) = rejects(
        &db,
        CreateContactInput {
            phone_number: Some(String::new()),
            source_user_id: Some(BSUID.to_string()),
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await;
    assert_eq!(field.as_deref(), Some("sourceUserId"));
}

#[tokio::test]
async fn bsuid_still_requires_a_phone_when_no_bsuid_is_given() {
    let db = db_with(None).await;
    let (field, _) = rejects(
        &db,
        CreateContactInput {
            phone_number: Some(String::new()),
            ..on("whatsapp", WHATSAPP)
        },
    )
    .await;
    assert_eq!(field.as_deref(), Some("phoneNumber"));
}

#[tokio::test]
async fn bsuid_ignores_a_bsuid_on_a_non_whatsapp_channel() {
    let db = db_with(None).await;
    create(
        &db,
        CreateContactInput {
            contact_id: Some("psid-1".to_string()),
            source_user_id: Some(BSUID.to_string()),
            ..on("messenger", MESSENGER)
        },
    )
    .await
    .expect("created");
    let inboxes = inbox_rows(&db, MESSENGER).await;
    assert_eq!(inboxes[0]["sourceId"], "psid-1");
    assert_eq!(inboxes[0]["sourceUserId"], Value::Null);
}
