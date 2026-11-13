//! Port of `packages/business/__tests__/contact-update-fields.test.ts`
//! (P3.6-D2). Adapted: the mocked contact / contact-inbox / custom-field
//! services become stored rows; the asserted writes are the rows and the
//! returned `UpdateOutcome` (its `has_real_change` is the audit decision).
//! The emit ordering ("emits only after commit") is not applicable (no
//! event bus); the stored value is asserted instead.

mod common;

use chatbotx_business::contact::update_fields_and_custom_fields;
use common::{TestDb, WS, contact_inbox, custom_field, inbox, rows, setup, two_workspaces};

const CONTACT: i64 = 9001;
const CONTACT_INBOX: i64 = 9101;

async fn db(first_name: Option<&'static str>, email: Option<&'static str>) -> TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "email", "workspaceId") VALUES (?1, ?2, ?3, ?4)"#,
            chatbotx_db::rusqlite::params![CONTACT, first_name, email, WS],
        )?;
        inbox(tx, 3002, WS, "messenger")?;
        contact_inbox(tx, CONTACT_INBOX, CONTACT, 3002, "messenger", "psid-1")?;
        custom_field(tx, 1, WS, "plan", "shortText")?;
        Ok(())
    })
    .await;
    db
}

fn fields(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

async fn values(db: &TestDb) -> Vec<(i64, String)> {
    rows(
        db,
        r#"SELECT "customFieldId", "value" FROM "ContactCustomField" WHERE "contactId" = ?1 ORDER BY "customFieldId""#,
        CONTACT,
    )
    .await
    .into_iter()
    .map(|row| {
        (
            row["customFieldId"].as_i64().expect("id"),
            row["value"].as_str().expect("value").to_string(),
        )
    })
    .collect()
}

#[tokio::test]
async fn writes_inside_the_transaction_and_emits_custom_field_changes_only_after_commit() {
    let db = db(None, None).await;
    let outcome = update_fields_and_custom_fields(
        &db,
        WS,
        CONTACT,
        fields(&[("1", "pro"), ("clientTimezone", "Asia/Ho_Chi_Minh")]),
    )
    .await
    .expect("updated");
    assert_eq!(values(&db).await, vec![(1, "pro".to_string())]);
    assert_eq!(outcome.custom_field_changes.len(), 1);
    let change = &outcome.custom_field_changes[0];
    assert_eq!(change.custom_field_id, 1);
    assert_eq!(change.custom_field_name, "plan");
    assert_eq!(change.old_value, None);
    assert_eq!(change.new_value, "pro");
    assert!(outcome.contact_fields.is_empty());
    assert!(outcome.has_real_change());
}

#[tokio::test]
async fn does_not_touch_the_custom_field_funnel_when_no_custom_fields_are_submitted() {
    let db = db(None, None).await;
    let outcome =
        update_fields_and_custom_fields(&db, WS, CONTACT, fields(&[("firstName", "Ada")]))
            .await
            .expect("updated");
    assert!(outcome.custom_field_changes.is_empty());
    assert_eq!(outcome.contact_fields, vec!["firstName".to_string()]);
    assert!(values(&db).await.is_empty());
}

#[tokio::test]
async fn skips_contact_writes_emits_and_audit_for_an_unchanged_payload() {
    let db = db(Some("Ada"), Some("ada@example.com")).await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Contact" SET "gender" = 'female', "phoneNumber" = '84901234567' WHERE "id" = ?1"#,
            [CONTACT],
        )?;
        tx.execute(
            r#"UPDATE "ContactInbox" SET "language" = 'en' WHERE "id" = ?1"#,
            [CONTACT_INBOX],
        )?;
        tx.execute(
            r#"INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value") VALUES (1, ?1, 1, 'pro')"#,
            [CONTACT],
        )?;
        Ok(())
    })
    .await;
    let before = rows(
        &db,
        r#"SELECT "updatedAt" FROM "Contact" WHERE "id" = ?1"#,
        CONTACT,
    )
    .await;
    let outcome = update_fields_and_custom_fields(
        &db,
        WS,
        CONTACT,
        fields(&[
            ("firstName", "Ada"),
            ("gender", "female"),
            ("phoneNumber", "84901234567"),
            ("email", "ada@example.com"),
            ("contactInboxId", &CONTACT_INBOX.to_string()),
            ("language", "en_US"),
            ("1", "pro"),
        ]),
    )
    .await
    .expect("updated");
    assert_eq!(outcome, Default::default());
    assert!(!outcome.has_real_change());
    let after = rows(
        &db,
        r#"SELECT "updatedAt" FROM "Contact" WHERE "id" = ?1"#,
        CONTACT,
    )
    .await;
    assert_eq!(before, after);
}

#[tokio::test]
async fn updates_emits_and_audits_only_the_changed_contact_fields() {
    let db = db(Some("Ada"), Some("ada@example.com")).await;
    let outcome = update_fields_and_custom_fields(
        &db,
        WS,
        CONTACT,
        fields(&[("firstName", "Grace"), ("email", "ada@example.com")]),
    )
    .await
    .expect("updated");
    assert_eq!(outcome.contact_fields, vec!["firstName".to_string()]);
    assert!(!outcome.language_updated);
    assert!(outcome.has_real_change());
    let contact = rows(
        &db,
        r#"SELECT "firstName", "email", "phoneNumber" FROM "Contact" WHERE "id" = ?1"#,
        CONTACT,
    )
    .await;
    assert_eq!(contact[0]["firstName"], "Grace");
    assert_eq!(contact[0]["email"], "ada@example.com");
    assert_eq!(contact[0]["phoneNumber"], serde_json::Value::Null);
}

#[tokio::test]
async fn updates_language_and_audits_when_the_contact_inbox_language_changed() {
    let db = db(None, None).await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "ContactInbox" SET "language" = 'vi' WHERE "id" = ?1"#,
            [CONTACT_INBOX],
        )?;
        Ok(())
    })
    .await;
    let outcome = update_fields_and_custom_fields(
        &db,
        WS,
        CONTACT,
        fields(&[
            ("contactInboxId", &CONTACT_INBOX.to_string()),
            ("language", "en_US"),
        ]),
    )
    .await
    .expect("updated");
    assert!(outcome.language_updated);
    assert!(outcome.contact_fields.is_empty());
    assert!(outcome.has_real_change());
    let inbox = rows(
        &db,
        r#"SELECT "language" FROM "ContactInbox" WHERE "id" = ?1"#,
        CONTACT_INBOX,
    )
    .await;
    assert_eq!(inbox[0]["language"], "en");
}

#[tokio::test]
async fn returns_without_writes_emits_or_audit_for_an_empty_payload() {
    let db = db(Some("Ada"), None).await;
    let outcome = update_fields_and_custom_fields(&db, WS, CONTACT, Vec::new())
        .await
        .expect("updated");
    assert_eq!(outcome, Default::default());
    assert!(values(&db).await.is_empty());
}

#[tokio::test]
async fn writes_a_custom_field_beyond_the_old_50_row_clamp() {
    let db = db(None, None).await;
    setup(&db, |tx| {
        for id in 2..=60 {
            custom_field(tx, id, WS, &format!("field-{id}"), "shortText")?;
        }
        Ok(())
    })
    .await;
    let outcome = update_fields_and_custom_fields(&db, WS, CONTACT, fields(&[("55", "value-55")]))
        .await
        .expect("updated");
    assert_eq!(outcome.custom_field_changes.len(), 1);
    assert_eq!(
        outcome.custom_field_changes[0].custom_field_name,
        "field-55"
    );
    assert_eq!(values(&db).await, vec![(55, "value-55".to_string())]);
}
