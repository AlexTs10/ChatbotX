//! Ports of the contact import run tests (P3.6-D2 slice contact imports,
//! part two): `packages/business/__tests__/import-start-contact-import.test.ts`,
//! `contact-insert-imported-batch.test.ts`,
//! `apps/worker/__tests__/import-contacts-extractor.test.ts`,
//! `run-import.test.ts` and `import-contacts-handler.test.ts`.
//! Adapted: the mocked db / services / uploader become real rows and an
//! in-memory store; "updateSet calls" = the stored `Import` row after the run;
//! "the queue job" = the caller runs [`run_import`] (no queue).
//! One assertion change everywhere a mocked `importService.fail` saw the raw
//! reason: the stored `errorMessage` is `GENERIC_IMPORT_FAILURE`
//! (`toPublicErrorMessage` keeps only a `ChatbotXException` message; DECISIONS
//! 2026-11-16 P3.6-D2 imports).

mod common;

use chatbotx_business::import::contacts::{
    AcceptedContact, ColumnMap, ContactImportMeta, ContactRow, FieldMapping,
    GENERIC_IMPORT_FAILURE, StartContactImport, extract_row_data, insert_imported_contact_batch,
    run_import, start_contact_import,
};
use chatbotx_db::rusqlite::params;
use chatbotx_files::{PutOptions, Uploader};
use common::{
    TestDb, WS, bot_field, bot_field_value, contact, contact_inbox, custom_field,
    custom_field_values, inbox, linked_tag_ids, rows, setup, tag, two_workspaces,
};
use serde_json::{Map, Value, json};

const INBOX: i64 = 501;
const PATH: &str = "workspaces/2001/imports/contacts/test.csv";

fn base_meta() -> ContactImportMeta {
    ContactImportMeta {
        channel: "messenger".into(),
        column_map: ColumnMap {
            contact_id: Some("external_id".into()),
            phone_number: Some("phone".into()),
            email: Some("email".into()),
            ..ColumnMap::default()
        },
        ..ContactImportMeta::default()
    }
}

/// A file row (`subType`, `mimeType`, `fileName`) of `WS`.
async fn file(
    db: &TestDb,
    id: i64,
    context: &'static str,
    sub: &'static str,
    mime: &'static str,
    name: &'static str,
) {
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType", "status") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending')"#,
            params![id, WS, context, sub, PATH, name, mime],
        )?;
        Ok(())
    })
    .await;
}

/// The workspaces, a messenger inbox and an uploaded contacts CSV (file 1).
async fn ready(lines: &[&str], channel: &'static str) -> (TestDb, Uploader) {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        inbox(tx, INBOX, WS, channel)?;
        Ok(())
    })
    .await;
    file(&db, 1, "import", "contacts", "text/csv", "test.csv").await;
    let uploader = Uploader::in_memory();
    uploader
        .put_object(
            PATH,
            lines.join("\n").into_bytes(),
            PutOptions::default(),
            None,
        )
        .await
        .expect("put");
    (db, uploader)
}

/// An `Import` row of `WS` for file 1 (`format`, `meta` JSON).
async fn import_row(db: &TestDb, id: i64, format: &'static str, meta: Value) {
    let meta = meta.to_string();
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Import" ("id", "workspaceId", "inboxId", "userId", "fileId", "type", "format", "status", "meta") VALUES (?1, ?2, ?3, 1001, 1, 'contacts', ?4, 'pending', ?5)"#,
            params![id, WS, INBOX, format, meta],
        )?;
        Ok(())
    })
    .await;
}

async fn run(db: &TestDb, uploader: &Uploader, meta: ContactImportMeta) -> Map<String, Value> {
    import_row(db, 77, "csv", serde_json::to_value(meta).expect("meta")).await;
    run_import(db, uploader, 77).await.expect("run");
    imported(db, 77).await
}

async fn imported(db: &TestDb, id: i64) -> Map<String, Value> {
    rows(
        db,
        r#"SELECT "status", "totalCount", "processedCount", "successCount", "failedCount", "errorMessage", "errorSample" FROM "Import" WHERE "id" = ?1"#,
        id,
    )
    .await
    .pop()
    .expect("import row")
}

fn assert_row(row: &Map<String, Value>, expected: Value) {
    for (k, v) in expected.as_object().expect("object") {
        assert_eq!(row.get(k), Some(v), "{k} in {row:?}");
    }
}

/// `(sourceId, sourceUserId, source)` of the inbox's contact inboxes.
async fn contact_inboxes(db: &TestDb) -> Vec<(String, Option<String>, String)> {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "sourceId", "sourceUserId", "source" FROM "ContactInbox" WHERE "inboxId" = ?1 ORDER BY "sourceId""#,
        )?;
        let out = stmt
            .query_map([INBOX], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    })
    .await
    .expect("contact inboxes")
}

async fn contact_id_of(db: &TestDb, source_id: &'static str) -> i64 {
    db.read(move |conn| {
        Ok(conn.query_row(
            r#"SELECT "contactId" FROM "ContactInbox" WHERE "sourceId" = ?1"#,
            [source_id],
            |r| r.get(0),
        )?)
    })
    .await
    .expect("contact id")
}

async fn count(db: &TestDb, sql: &'static str) -> i64 {
    db.read(move |conn| Ok(conn.query_row(sql, [], |r| r.get(0))?))
        .await
        .expect("count")
}

// --- importService.startContactImport ----------------------------------

fn start_input() -> StartContactImport {
    StartContactImport {
        workspace_id: WS,
        user_id: Some(1001),
        inbox_id: INBOX,
        file_id: 1,
        meta: base_meta(),
    }
}

fn code_of(error: chatbotx_business::BusinessError) -> (String, u16) {
    match error {
        chatbotx_business::BusinessError::Exception { code, status, .. } => (code, status),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn import_service_start_contact_import_throws_contact_import_file_not_found_404_when_the_file_doesn_t_resolve()
 {
    let db = two_workspaces().await;
    let error = start_contact_import(&db, start_input())
        .await
        .expect_err("missing");
    assert_eq!(code_of(error), ("contactImportFileNotFound".into(), 404));
}

#[tokio::test]
async fn import_service_start_contact_import_throws_contact_import_file_type_invalid_when_the_file_isn_t_a_contacts_import()
 {
    let db = two_workspaces().await;
    file(&db, 1, "import", "products", "text/csv", "a.csv").await;
    let error = start_contact_import(&db, start_input())
        .await
        .expect_err("type");
    assert_eq!(code_of(error).0, "contactImportFileTypeInvalid");
}

#[tokio::test]
async fn import_service_start_contact_import_throws_contact_import_unsupported_format_when_the_format_isn_t_accepted()
 {
    let db = two_workspaces().await;
    file(&db, 1, "import", "contacts", "application/zip", "a.zip").await;
    let error = start_contact_import(&db, start_input())
        .await
        .expect_err("format");
    assert_eq!(code_of(error).0, "contactImportUnsupportedFormat");
}

#[tokio::test]
async fn import_service_start_contact_import_throws_contact_import_inbox_not_found_404_when_the_inbox_doesn_t_resolve()
 {
    let db = two_workspaces().await;
    file(&db, 1, "import", "contacts", "text/csv", "a.csv").await;
    let error = start_contact_import(&db, start_input())
        .await
        .expect_err("inbox");
    assert_eq!(code_of(error), ("contactImportInboxNotFound".into(), 404));
    // "transaction not called" = no import row, the file still pending.
    assert_eq!(count(&db, r#"SELECT count(*) FROM "Import""#).await, 0);
}

#[tokio::test]
async fn import_service_start_contact_import_throws_contact_import_already_running_when_a_pending_processing_import_exists()
 {
    let (db, _) = ready(&["id"], "messenger").await;
    import_row(&db, 9, "csv", json!({})).await;
    let error = start_contact_import(&db, start_input())
        .await
        .expect_err("running");
    assert_eq!(code_of(error), ("contactImportAlreadyRunning".into(), 409));
    assert_eq!(count(&db, r#"SELECT count(*) FROM "Import""#).await, 1);
}

#[tokio::test]
async fn import_service_start_contact_import_happy_path_writes_the_import_row_and_enqueues_run_import()
 {
    let (db, _) = ready(&["id"], "messenger").await;
    let id = start_contact_import(&db, start_input())
        .await
        .expect("start");
    let row = rows(
        &db,
        r#"SELECT i."status", i."type", i."format", i."inboxId", i."meta", f."status" AS "fileStatus" FROM "Import" i JOIN "File" f ON f."id" = i."fileId" WHERE i."id" = ?1"#,
        id,
    )
    .await
    .pop()
    .expect("row");
    assert_row(
        &row,
        json!({ "status": "pending", "type": "contacts", "format": "csv", "inboxId": INBOX, "fileStatus": "uploaded" }),
    );
    let meta: Value = serde_json::from_str(row["meta"].as_str().expect("meta")).expect("json");
    assert_eq!(
        meta,
        json!({ "channel": "messenger", "columnMap": { "contactId": "external_id", "phoneNumber": "phone", "email": "email" } })
    );
}

// --- extractRowData ------------------------------------------------------

fn column_map() -> ColumnMap {
    ColumnMap {
        contact_id: Some("id".into()),
        phone_number: Some("phone".into()),
        email: Some("email".into()),
        first_name: Some("first".into()),
        last_name: Some("last".into()),
        source_user_id: None,
    }
}

fn obj(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn phone_of(phone: &str, country: Option<&str>) -> Option<String> {
    extract_row_data(
        &obj(json!({ "phone": phone })),
        &column_map(),
        &[],
        country,
        None,
    )
    .and_then(|r| r.phone_number)
}

#[test]
fn extract_row_data_phone_normalization_strips_the_trunk_0_and_applies_the_calling_code_vn() {
    assert_eq!(
        phone_of("0912345678", Some("+84")).as_deref(),
        Some("+84912345678")
    );
}

#[test]
fn extract_row_data_phone_normalization_retains_the_leading_0_for_countries_that_keep_it_it_landline()
 {
    assert_eq!(
        phone_of("0612345678", Some("+39")).as_deref(),
        Some("+390612345678")
    );
}

#[test]
fn extract_row_data_phone_normalization_normalizes_the_00_international_prefix() {
    assert_eq!(
        phone_of("0084912345678", Some("+84")).as_deref(),
        Some("+84912345678")
    );
}

#[test]
fn extract_row_data_phone_normalization_passes_through_an_already_prefixed_number_without_a_country_code()
 {
    assert_eq!(
        phone_of("+390612345678", None).as_deref(),
        Some("+390612345678")
    );
}

#[test]
fn extract_row_data_phone_normalization_ignores_the_country_code_when_the_number_is_already_international()
 {
    assert_eq!(
        phone_of("+84912345678", Some("+1")).as_deref(),
        Some("+84912345678")
    );
}

#[test]
fn extract_row_data_phone_normalization_accepts_spaces_and_dashes_in_the_input() {
    assert_eq!(
        phone_of("091-234 5678", Some("+84")).as_deref(),
        Some("+84912345678")
    );
}

#[test]
fn extract_row_data_phone_normalization_returns_undefined_for_an_invalid_number() {
    assert_eq!(phone_of("123", Some("+84")), None);
}

#[test]
fn extract_row_data_phone_normalization_returns_undefined_for_a_local_number_with_no_country_code()
{
    assert_eq!(phone_of("0912345678", None), None);
}

#[test]
fn extract_row_data_phone_normalization_keeps_the_row_when_phone_is_invalid_but_another_field_is_present()
 {
    let row = extract_row_data(
        &obj(json!({ "phone": "123", "email": "a@b.com" })),
        &column_map(),
        &[],
        Some("+84"),
        None,
    )
    .expect("row");
    assert_eq!(row.phone_number, None);
    assert_eq!(row.email.as_deref(), Some("a@b.com"));
}

#[test]
fn extract_row_data_phone_normalization_drops_the_row_when_phone_is_invalid_and_no_other_field_is_present()
 {
    let row = extract_row_data(
        &obj(json!({ "phone": "123" })),
        &column_map(),
        &[],
        Some("+84"),
        None,
    );
    assert_eq!(row, None);
}

#[test]
fn extract_row_data_external_id_resolution_uses_the_contact_id_column_as_external_id_for_non_whatsapp_channels()
 {
    let row = extract_row_data(
        &obj(json!({ "id": "ext-1", "phone": "+84912345678" })),
        &column_map(),
        &[],
        None,
        None,
    )
    .expect("row");
    assert_eq!(row.external_id.as_deref(), Some("ext-1"));
    assert_eq!(row.phone_number.as_deref(), Some("+84912345678"));
}

#[test]
fn extract_row_data_external_id_resolution_derives_external_id_from_the_phone_digits_no_for_whatsapp()
 {
    let row = extract_row_data(
        &obj(json!({ "id": "ext-1", "phone": "+84912345678" })),
        &column_map(),
        &[],
        None,
        Some("whatsapp"),
    )
    .expect("row");
    assert_eq!(row.external_id.as_deref(), Some("84912345678"));
    assert_eq!(row.phone_number.as_deref(), Some("+84912345678"));
}

#[test]
fn extract_row_data_external_id_resolution_derives_the_whatsapp_external_id_from_a_normalized_leading_zero_phone()
 {
    let row = extract_row_data(
        &obj(json!({ "phone": "0912345678" })),
        &column_map(),
        &[],
        Some("+84"),
        Some("whatsapp"),
    )
    .expect("row");
    assert_eq!(row.external_id.as_deref(), Some("84912345678"));
    assert_eq!(row.phone_number.as_deref(), Some("+84912345678"));
}

#[test]
fn extract_row_data_external_id_resolution_drops_a_whatsapp_row_when_the_phone_is_invalid() {
    let row = extract_row_data(
        &obj(json!({ "phone": "123" })),
        &column_map(),
        &[],
        Some("+84"),
        Some("whatsapp"),
    );
    assert_eq!(row, None);
}

#[test]
fn extract_row_data_external_id_resolution_resolves_the_whatsapp_external_id_from_the_phone_when_no_source_user_id_column_is_mapped_regression()
 {
    let row = extract_row_data(
        &obj(json!({ "phone": "+84912345678" })),
        &column_map(),
        &[],
        None,
        Some("whatsapp"),
    )
    .expect("row");
    assert_eq!(row.external_id.as_deref(), Some("84912345678"));
    assert_eq!(row.source_user_id, None);
}

fn with_source_user_id() -> ColumnMap {
    ColumnMap {
        source_user_id: Some("wa_user_id".into()),
        ..column_map()
    }
}

#[test]
fn extract_row_data_source_user_id_bsuid_resolution_whatsapp_phone_bsuid_column_external_id_comes_from_the_phone_source_user_id_is_kept()
 {
    let row = extract_row_data(
        &obj(json!({ "phone": "+84912345678", "wa_user_id": "user.9373928427292738" })),
        &with_source_user_id(),
        &[],
        None,
        Some("whatsapp"),
    )
    .expect("row");
    assert_eq!(row.external_id.as_deref(), Some("84912345678"));
    assert_eq!(row.source_user_id.as_deref(), Some("user.9373928427292738"));
}

#[test]
fn extract_row_data_source_user_id_bsuid_resolution_whatsapp_bsuid_only_no_phone_external_id_falls_back_to_source_user_id()
 {
    let row = extract_row_data(
        &obj(json!({ "wa_user_id": "user.9373928427292738" })),
        &with_source_user_id(),
        &[],
        None,
        Some("whatsapp"),
    )
    .expect("row");
    assert_eq!(row.external_id.as_deref(), Some("user.9373928427292738"));
    assert_eq!(row.source_user_id.as_deref(), Some("user.9373928427292738"));
}

#[test]
fn extract_row_data_source_user_id_bsuid_resolution_whatsapp_with_neither_phone_bsuid_nor_email_is_rejected_regression()
 {
    let row = extract_row_data(
        &Map::new(),
        &with_source_user_id(),
        &[],
        None,
        Some("whatsapp"),
    );
    assert_eq!(row, None);
}

#[test]
fn extract_row_data_source_user_id_bsuid_resolution_does_not_extract_source_user_id_for_a_non_whatsapp_channel_even_when_the_column_is_mapped()
 {
    let row = extract_row_data(
        &obj(json!({ "id": "ext-1", "wa_user_id": "user.9373928427292738" })),
        &with_source_user_id(),
        &[],
        None,
        None,
    )
    .expect("row");
    assert_eq!(row.external_id.as_deref(), Some("ext-1"));
    assert_eq!(row.source_user_id, None);
}

#[test]
fn extract_row_data_field_handling_maps_standard_columns_and_trims_values() {
    let row = extract_row_data(
        &obj(json!({
            "id": "  ext-1 ",
            "phone": " +84912345678 ",
            "email": "  user@example.com ",
            "first": "Ada ",
            "last": " Lovelace",
        })),
        &column_map(),
        &[],
        None,
        None,
    );
    assert_eq!(
        row,
        Some(ContactRow {
            external_id: Some("ext-1".into()),
            phone_number: Some("+84912345678".into()),
            email: Some("user@example.com".into()),
            first_name: Some("Ada".into()),
            last_name: Some("Lovelace".into()),
            source_user_id: None,
            custom_fields: vec![],
        })
    );
}

#[test]
fn extract_row_data_field_handling_returns_null_when_only_blank_strings_are_present() {
    let row = extract_row_data(
        &obj(json!({ "id": "   ", "phone": "", "email": "  " })),
        &column_map(),
        &[],
        None,
        None,
    );
    assert_eq!(row, None);
}

#[test]
fn extract_row_data_field_handling_accepts_a_row_with_only_an_email() {
    let row = extract_row_data(
        &obj(json!({ "email": "lone@example.com" })),
        &column_map(),
        &[],
        None,
        None,
    )
    .expect("row");
    assert_eq!(row.email.as_deref(), Some("lone@example.com"));
    assert_eq!(row.phone_number, None);
}

#[test]
fn extract_row_data_field_handling_collects_field_mapping_into_custom_fields_and_skips_blanks() {
    let mapping = |column: &str, id: &str| FieldMapping {
        column: column.into(),
        custom_field_id: id.into(),
    };
    let row = extract_row_data(
        &obj(json!({ "phone": "+84912345678", "company": "Acme", "role": "  " })),
        &ColumnMap {
            phone_number: Some("phone".into()),
            ..ColumnMap::default()
        },
        &[
            mapping("company", "10"),
            mapping("role", "11"),
            mapping("missing", "12"),
        ],
        None,
        None,
    )
    .expect("row");
    assert_eq!(
        row.custom_fields,
        vec![("10".to_string(), "Acme".to_string())]
    );
}

#[test]
fn extract_row_data_field_handling_stores_values_raw_without_formula_escaping_escaping_happens_at_export()
 {
    let row = extract_row_data(
        &obj(json!({ "id": "=CMD()", "first": "+DANGER", "last": "@evil" })),
        &column_map(),
        &[],
        None,
        None,
    )
    .expect("row");
    assert_eq!(row.external_id.as_deref(), Some("=CMD()"));
    assert_eq!(row.first_name.as_deref(), Some("+DANGER"));
    assert_eq!(row.last_name.as_deref(), Some("@evil"));
}

#[test]
fn extract_row_data_field_handling_ignores_non_string_row_values() {
    let row = extract_row_data(
        &obj(json!({ "phone": 12_345, "email": null })),
        &column_map(),
        &[],
        None,
        None,
    );
    assert_eq!(row, None);
}

// --- insertImportedContactBatch -----------------------------------------

fn accepted(contact_id: i64, external_id: Option<&str>) -> AcceptedContact {
    AcceptedContact {
        contact_id,
        contact_inbox_id: contact_id + 100,
        row: ContactRow {
            external_id: external_id.map(str::to_string),
            ..ContactRow::default()
        },
    }
}

async fn batch(
    db: &TestDb,
    rows: Vec<AcceptedContact>,
    tag_id: Option<i64>,
) -> chatbotx_business::Result<chatbotx_business::import::contacts::InsertedBatch> {
    db.write(move |tx| {
        Ok(insert_imported_contact_batch(
            tx,
            WS,
            INBOX,
            "messenger",
            &rows,
            tag_id,
        ))
    })
    .await
    .expect("write")
}

async fn with_inbox() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        inbox(tx, INBOX, WS, "messenger")?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn insert_imported_contact_batch_returns_inserted_0_orphan_count_0_without_a_transaction_when_accepted_is_empty()
 {
    let db = with_inbox().await;
    let result = batch(&db, vec![], None).await.expect("batch");
    assert_eq!((result.inserted, result.orphan_count), (0, 0));
    assert_eq!(count(&db, r#"SELECT count(*) FROM "Contact""#).await, 0);
}

#[tokio::test]
async fn insert_imported_contact_batch_maps_each_accepted_row_onto_a_contact_row_and_a_contact_inbox_row()
 {
    let db = with_inbox().await;
    let row = AcceptedContact {
        contact_id: 1,
        contact_inbox_id: 11,
        row: ContactRow {
            external_id: Some("user.9373928427292738".into()),
            source_user_id: Some("user.9373928427292738".into()),
            phone_number: Some("+15551234567".into()),
            email: Some("a@example.com".into()),
            first_name: Some("Ada".into()),
            last_name: Some("Lovelace".into()),
            custom_fields: vec![],
        },
    };
    batch(&db, vec![row], None).await.expect("batch");
    let contact = rows(
        &db,
        r#"SELECT "workspaceId", "phoneNumber", "email", "firstName", "lastName" FROM "Contact" WHERE "id" = ?1"#,
        1,
    )
    .await;
    assert_eq!(
        Value::Object(contact[0].clone()),
        json!({ "workspaceId": WS, "phoneNumber": "+15551234567", "email": "a@example.com", "firstName": "Ada", "lastName": "Lovelace" })
    );
    let link = rows(
        &db,
        r#"SELECT "contactId", "originalContactId", "inboxId", "channel", "sourceId", "sourceUserId" FROM "ContactInbox" WHERE "id" = ?1"#,
        11,
    )
    .await;
    assert_eq!(
        Value::Object(link[0].clone()),
        json!({ "contactId": 1, "originalContactId": 1, "inboxId": INBOX, "channel": "messenger", "sourceId": "user.9373928427292738", "sourceUserId": "user.9373928427292738" })
    );
}

#[tokio::test]
async fn insert_imported_contact_batch_throws_the_external_id_invariant_when_a_row_lacks_external_id()
 {
    let db = with_inbox().await;
    let error = batch(&db, vec![accepted(1, None)], None)
        .await
        .expect_err("invariant");
    assert!(
        error
            .to_string()
            .contains("Invariant: externalId must be set before insert"),
        "{error}"
    );
}

#[tokio::test]
async fn insert_imported_contact_batch_orphan_prune_fires_when_a_contact_inbox_insert_conflicts() {
    let db = with_inbox().await;
    // ext-2 is already linked: its contact-inbox insert conflicts.
    setup(&db, |tx| {
        contact(tx, 900, WS)?;
        contact_inbox(tx, 901, 900, INBOX, "messenger", "ext-2")?;
        Ok(())
    })
    .await;
    let result = batch(
        &db,
        vec![accepted(1, Some("ext-1")), accepted(2, Some("ext-2"))],
        None,
    )
    .await
    .expect("batch");
    assert_eq!((result.inserted, result.orphan_count), (1, 1));
    assert_eq!(
        count(&db, r#"SELECT count(*) FROM "Contact" WHERE "id" = 2"#).await,
        0
    );
}

#[tokio::test]
async fn insert_imported_contact_batch_cancel_by_inbox_source_receives_the_shared_tx_handle() {
    let db = with_inbox().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "MessageCleanup" ("id", "workspaceId", "contactId", "contactInboxId", "inboxId", "sourceId", "conversationIds") VALUES (1, ?1, 7, 8, ?2, 'ext-1', '[]')"#,
            params![WS, INBOX],
        )?;
        Ok(())
    })
    .await;
    batch(&db, vec![accepted(1, Some("ext-1"))], None)
        .await
        .expect("batch");
    // "the shared tx": the cleanup row is gone with the same commit.
    assert_eq!(
        count(&db, r#"SELECT count(*) FROM "MessageCleanup""#).await,
        0
    );
}

#[tokio::test]
async fn insert_imported_contact_batch_tag_insert_only_runs_when_tag_id_is_present() {
    let db = with_inbox().await;
    setup(&db, |tx| {
        tag(tx, 5, WS, "t")?;
        Ok(())
    })
    .await;
    batch(&db, vec![accepted(1, Some("ext-1"))], None)
        .await
        .expect("batch");
    assert_eq!(linked_tag_ids(&db, 1).await, Vec::<i64>::new());
    batch(&db, vec![accepted(2, Some("ext-2"))], Some(5))
        .await
        .expect("batch");
    assert_eq!(linked_tag_ids(&db, 2).await, vec![5]);
}

#[tokio::test]
async fn insert_imported_contact_batch_returns_inserted_0_without_conversation_custom_field_inserts_when_every_row_is_an_orphan()
 {
    let db = with_inbox().await;
    setup(&db, |tx| {
        contact(tx, 900, WS)?;
        contact_inbox(tx, 901, 900, INBOX, "messenger", "ext-1")?;
        Ok(())
    })
    .await;
    let result = batch(&db, vec![accepted(1, Some("ext-1"))], None)
        .await
        .expect("batch");
    assert_eq!((result.inserted, result.orphan_count), (0, 1));
    assert_eq!(
        count(&db, r#"SELECT count(*) FROM "Conversation""#).await,
        0
    );
}

// --- runImport dispatcher -------------------------------------------------

#[tokio::test]
async fn run_import_dispatcher_returns_early_when_import_row_is_missing() {
    let db = two_workspaces().await;
    run_import(&db, &Uploader::in_memory(), 404)
        .await
        .expect("run");
    assert_eq!(count(&db, r#"SELECT count(*) FROM "Import""#).await, 0);
}

#[tokio::test]
async fn run_import_dispatcher_dispatches_contacts_handler_when_type_is_contacts() {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email",
            "ext-1,+15551234567,a@example.com",
        ],
        "messenger",
    )
    .await;
    let row = run(&db, &uploader, base_meta()).await;
    assert_row(&row, json!({ "status": "completed", "successCount": 1 }));
}

// --- contacts import pipeline --------------------------------------------

#[tokio::test]
async fn contacts_import_pipeline_marks_row_failed_when_inbox_missing() {
    let (db, uploader) = ready(&["external_id"], "messenger").await;
    setup(&db, |tx| {
        tx.execute(r#"DELETE FROM "Inbox""#, [])?;
        Ok(())
    })
    .await;
    // The inbox is gone; `Import.inboxId` cascades, so the row keeps no inbox.
    import_row_without_inbox(&db).await;
    run_import(&db, &uploader, 78).await.expect("run");
    assert_row(
        &imported(&db, 78).await,
        json!({ "status": "failed", "errorMessage": GENERIC_IMPORT_FAILURE }),
    );
}

async fn import_row_without_inbox(db: &TestDb) {
    let meta = serde_json::to_string(&base_meta()).expect("meta");
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Import" ("id", "workspaceId", "fileId", "type", "format", "status", "meta") VALUES (78, ?1, 1, 'contacts', 'csv', 'pending', ?2)"#,
            params![WS, meta],
        )?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn contacts_import_pipeline_inserts_a_batch_and_marks_completed_with_counts() {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email",
            "ext-1,+15551234567,first@example.com",
            "ext-2,+15557654321,second@example.com",
        ],
        "messenger",
    )
    .await;
    let row = run(&db, &uploader, base_meta()).await;
    assert_row(
        &row,
        json!({ "status": "completed", "totalCount": 2, "processedCount": 2, "successCount": 2, "failedCount": 0 }),
    );
    let sources: Vec<String> = contact_inboxes(&db)
        .await
        .into_iter()
        .map(|r| r.0)
        .collect();
    assert_eq!(sources, vec!["ext-1", "ext-2"]);
}

fn bot_meta(mapping: &[(&str, &str)]) -> ContactImportMeta {
    ContactImportMeta {
        field_mapping: Some(
            mapping
                .iter()
                .map(|(column, id)| FieldMapping {
                    column: (*column).into(),
                    custom_field_id: (*id).into(),
                })
                .collect(),
        ),
        ..base_meta()
    }
}

async fn with_bot_field(db: &TestDb, kind: &'static str) {
    setup(db, move |tx| {
        bot_field(tx, 11, WS, "score", kind, None, 1)?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn contacts_import_pipeline_a_bot_field_mapping_is_applied_once_after_completion_with_the_last_row_s_value()
 {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email,score",
            "ext-1,+15551234567,first@example.com,10",
            "ext-2,+15557654321,second@example.com,25",
        ],
        "messenger",
    )
    .await;
    with_bot_field(&db, "number").await;
    let row = run(&db, &uploader, bot_meta(&[("score", "bot_field:11")])).await;
    assert_row(&row, json!({ "status": "completed" }));
    assert_eq!(bot_field_value(&db, 11).await, Some(Some("25".into())));
    // Not a contact custom field.
    assert_eq!(
        count(&db, r#"SELECT count(*) FROM "ContactCustomField""#).await,
        0
    );
}

#[tokio::test]
async fn contacts_import_pipeline_mixed_mapping_custom_field_entries_stay_per_row_while_the_bot_field_entry_is_finalized_once()
 {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email,note,score",
            "ext-1,+15551234567,first@example.com,hello,10",
        ],
        "messenger",
    )
    .await;
    with_bot_field(&db, "number").await;
    setup(&db, |tx| {
        custom_field(tx, 7, WS, "note", "shortText")?;
        Ok(())
    })
    .await;
    let row = run(
        &db,
        &uploader,
        bot_meta(&[("note", "7"), ("score", "bot_field:11")]),
    )
    .await;
    assert_row(&row, json!({ "status": "completed" }));
    let contact = contact_id_of(&db, "ext-1").await;
    assert_eq!(
        custom_field_values(&db, contact).await,
        vec![(7, "hello".into())]
    );
    assert_eq!(bot_field_value(&db, 11).await, Some(Some("10".into())));
}

#[tokio::test]
async fn contacts_import_pipeline_a_lenient_csv_date_normalizes_into_a_date_bot_field() {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email,birthday",
            "ext-1,+15551234567,a@example.com,28/08/2026",
        ],
        "messenger",
    )
    .await;
    with_bot_field(&db, "date").await;
    // The real `updateByKey` re-normalizes in the workspace zone (the
    // original mocks it): the workspace uses the import zone.
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "timezone" = 'Asia/Ho_Chi_Minh'"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let meta = ContactImportMeta {
        timezone: Some("Asia/Ho_Chi_Minh".into()),
        ..bot_meta(&[("birthday", "bot_field:11")])
    };
    let row = run(&db, &uploader, meta).await;
    assert_row(&row, json!({ "status": "completed" }));
    assert_eq!(
        bot_field_value(&db, 11).await,
        Some(Some("2026-08-28T00:00:00+07:00".into()))
    );
}

#[tokio::test]
async fn contacts_import_pipeline_an_invalid_bot_field_value_is_skipped_the_last_valid_row_still_wins_and_the_import_completes()
 {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email,score",
            "ext-1,+15551234567,a@example.com,10",
            "ext-2,+15557654321,b@example.com,not-a-number",
        ],
        "messenger",
    )
    .await;
    with_bot_field(&db, "number").await;
    let row = run(&db, &uploader, bot_meta(&[("score", "bot_field:11")])).await;
    assert_row(
        &row,
        json!({ "status": "completed", "successCount": 2, "failedCount": 0 }),
    );
    assert_eq!(bot_field_value(&db, 11).await, Some(Some("10".into())));
}

#[tokio::test]
async fn contacts_import_pipeline_a_blank_bot_field_cell_is_skipped_and_never_overwrites_the_tracked_value()
 {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email,score",
            "ext-1,+15551234567,a@example.com,10",
            "ext-2,+15557654321,b@example.com,",
        ],
        "messenger",
    )
    .await;
    with_bot_field(&db, "number").await;
    let row = run(&db, &uploader, bot_meta(&[("score", "bot_field:11")])).await;
    assert_row(&row, json!({ "status": "completed" }));
    assert_eq!(bot_field_value(&db, 11).await, Some(Some("10".into())));
}

#[tokio::test]
async fn contacts_import_pipeline_no_bot_field_write_happens_when_every_row_s_value_is_invalid() {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email,score",
            "ext-1,+15551234567,a@example.com,not-a-number",
        ],
        "messenger",
    )
    .await;
    with_bot_field(&db, "number").await;
    let row = run(&db, &uploader, bot_meta(&[("score", "bot_field:11")])).await;
    assert_row(&row, json!({ "status": "completed" }));
    assert_eq!(bot_field_value(&db, 11).await, Some(None));
}

#[tokio::test]
async fn contacts_import_pipeline_skips_bot_field_updates_when_the_import_fails_before_completion()
{
    let (db, uploader) = ready(&["external_id"], "messenger").await;
    with_bot_field(&db, "number").await;
    // The import fails at `prepare` (a tag that is not in the workspace).
    let meta = ContactImportMeta {
        tag_id: Some("999".into()),
        ..bot_meta(&[("score", "bot_field:11")])
    };
    let row = run(&db, &uploader, meta).await;
    assert_row(&row, json!({ "status": "failed" }));
    assert_eq!(bot_field_value(&db, 11).await, Some(None));
}

#[tokio::test]
async fn contacts_import_pipeline_verifies_object_size_server_side_even_when_a_stored_file_size_exists()
 {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email",
            "ext-1,+15551234567,first@example.com",
        ],
        "messenger",
    )
    .await;
    setup(&db, |tx| {
        tx.execute(r#"UPDATE "File" SET "fileSize" = 1024"#, [])?;
        Ok(())
    })
    .await;
    let row = run(&db, &uploader, base_meta()).await;
    assert_row(
        &row,
        json!({ "status": "completed", "totalCount": 1, "processedCount": 1, "successCount": 1, "failedCount": 0 }),
    );
}

#[tokio::test]
async fn contacts_import_pipeline_marks_row_failed_and_does_not_stream_when_object_size_exceeds_the_import_limit()
 {
    let (db, uploader) = ready(&["x"], "messenger").await;
    let big = vec![b'a'; 21 * 1024 * 1024];
    uploader
        .put_object(PATH, big, PutOptions::default(), None)
        .await
        .expect("put");
    let row = run(&db, &uploader, base_meta()).await;
    // Original: errorMessage "File exceeds 20MB limit" (mocked fail); stored = generic.
    assert_row(
        &row,
        json!({ "status": "failed", "errorMessage": GENERIC_IMPORT_FAILURE, "processedCount": 0 }),
    );
}

#[tokio::test]
async fn contacts_import_pipeline_counts_blank_row_as_failed_but_continues() {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email",
            ",,",
            "ext-1,+15551234567,ok@example.com",
        ],
        "messenger",
    )
    .await;
    let row = run(&db, &uploader, base_meta()).await;
    assert_row(
        &row,
        json!({ "status": "completed", "successCount": 1, "failedCount": 1 }),
    );
}

#[tokio::test]
async fn contacts_import_pipeline_skips_a_row_that_already_exists_in_the_inbox() {
    let (db, uploader) = ready(
        &[
            "external_id,phone,email",
            "ext-1,+15551234567,ok@example.com",
        ],
        "messenger",
    )
    .await;
    setup(&db, |tx| {
        contact(tx, 900, WS)?;
        contact_inbox(tx, 901, 900, INBOX, "messenger", "ext-1")?;
        Ok(())
    })
    .await;
    let row = run(&db, &uploader, base_meta()).await;
    assert_row(
        &row,
        json!({ "status": "completed", "successCount": 0, "failedCount": 1 }),
    );
    assert_eq!(count(&db, r#"SELECT count(*) FROM "Contact""#).await, 1);
}

#[tokio::test]
async fn contacts_import_pipeline_marks_row_failed_when_csv_is_malformed() {
    let (db, uploader) = ready(&["external_id,phone", "\"unterminated,quote"], "messenger").await;
    let row = run(&db, &uploader, base_meta()).await;
    assert_row(&row, json!({ "status": "failed" }));
}

#[tokio::test]
async fn contacts_import_pipeline_empty_csv_finishes_as_completed_with_zero_counts() {
    let (db, uploader) = ready(&["external_id,phone,email"], "messenger").await;
    let row = run(&db, &uploader, base_meta()).await;
    assert_row(
        &row,
        json!({ "status": "completed", "totalCount": 0, "successCount": 0, "failedCount": 0 }),
    );
}

fn field_meta(mapping: &[(&str, &str)], timezone: Option<&str>) -> ContactImportMeta {
    ContactImportMeta {
        timezone: timezone.map(str::to_string),
        column_map: ColumnMap {
            contact_id: Some("external_id".into()),
            phone_number: Some("phone".into()),
            ..ColumnMap::default()
        },
        ..bot_meta(mapping)
    }
}

async fn with_custom_field(db: &TestDb, id: i64, kind: &'static str) {
    setup(db, move |tx| {
        custom_field(tx, id, WS, &format!("f{id}"), kind)?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn contacts_import_pipeline_drops_invalid_custom_field_value_keeps_contact() {
    let (db, uploader) = ready(
        &["external_id,phone,score", "ext-1,+15551234567,abc"],
        "messenger",
    )
    .await;
    with_custom_field(&db, 1, "number").await;
    let row = run(&db, &uploader, field_meta(&[("score", "1")], None)).await;
    assert_row(
        &row,
        json!({ "status": "completed", "successCount": 1, "failedCount": 0 }),
    );
    let contact = contact_id_of(&db, "ext-1").await;
    assert_eq!(custom_field_values(&db, contact).await, vec![]);
}

#[tokio::test]
async fn contacts_import_pipeline_keeps_valid_custom_field_value() {
    let (db, uploader) = ready(
        &["external_id,phone,score", "ext-1,+15551234567,42"],
        "messenger",
    )
    .await;
    with_custom_field(&db, 1, "number").await;
    run(&db, &uploader, field_meta(&[("score", "1")], None)).await;
    let contact = contact_id_of(&db, "ext-1").await;
    assert_eq!(
        custom_field_values(&db, contact).await,
        vec![(1, "42".into())]
    );
}

#[tokio::test]
async fn contacts_import_pipeline_imports_a_widened_boolean_literal_instead_of_dropping_it() {
    let (db, uploader) = ready(
        &["external_id,phone,subscribed", "ext-1,+15551234567,yes"],
        "messenger",
    )
    .await;
    with_custom_field(&db, 1, "boolean").await;
    run(&db, &uploader, field_meta(&[("subscribed", "1")], None)).await;
    let contact = contact_id_of(&db, "ext-1").await;
    assert_eq!(
        custom_field_values(&db, contact).await,
        vec![(1, "true".into())]
    );
}

#[tokio::test]
async fn contacts_import_pipeline_normalizes_imported_date_and_datetime_custom_fields_from_loose_formats()
 {
    let (db, uploader) = ready(
        &[
            "external_id,phone,birthday,appointment",
            "ext-1,+15551234567,23 tháng 7 năm 2026,23/07/2026 09:30",
        ],
        "messenger",
    )
    .await;
    with_custom_field(&db, 1, "date").await;
    with_custom_field(&db, 2, "datetime").await;
    run(
        &db,
        &uploader,
        field_meta(
            &[("birthday", "1"), ("appointment", "2")],
            Some("Asia/Ho_Chi_Minh"),
        ),
    )
    .await;
    let contact = contact_id_of(&db, "ext-1").await;
    assert_eq!(
        custom_field_values(&db, contact).await,
        vec![
            (1, "2026-07-23T00:00:00+07:00".into()),
            (2, "2026-07-23T02:30:00.000Z".into())
        ]
    );
}

#[tokio::test]
async fn contacts_import_pipeline_fails_when_format_is_unsupported() {
    let (db, uploader) = ready(&["external_id,phone"], "messenger").await;
    import_row(
        &db,
        77,
        "xlsx",
        serde_json::to_value(base_meta()).expect("meta"),
    )
    .await;
    run_import(&db, &uploader, 77).await.expect("run");
    // Original: errorMessage contains "xlsx" (mocked fail); stored = generic.
    assert_row(
        &imported(&db, 77).await,
        json!({ "status": "failed", "errorMessage": GENERIC_IMPORT_FAILURE }),
    );
}

#[tokio::test]
async fn contacts_import_pipeline_fails_the_row_when_meta_is_malformed() {
    let (db, uploader) = ready(&["external_id,phone", "ext-1,+15551234567"], "messenger").await;
    import_row(&db, 77, "csv", json!({})).await;
    run_import(&db, &uploader, 77).await.expect("run");
    assert_row(
        &imported(&db, 77).await,
        json!({ "status": "failed", "processedCount": 0 }),
    );
    assert_eq!(count(&db, r#"SELECT count(*) FROM "Contact""#).await, 0);
}

// --- contacts import: whatsapp sourceUserId (BSUID) ------------------------

fn whatsapp_meta() -> ContactImportMeta {
    ContactImportMeta {
        channel: "whatsapp".into(),
        column_map: ColumnMap {
            source_user_id: Some("wa_user_id".into()),
            ..ColumnMap::default()
        },
        ..ContactImportMeta::default()
    }
}

#[tokio::test]
async fn contacts_import_whatsapp_source_user_id_bsuid_bsuid_only_row_creates_a_bsuid_keyed_contact_inbox_source_id_equals_source_user_id()
 {
    let (db, uploader) = ready(&["wa_user_id", "user.9373928427292738"], "whatsapp").await;
    let row = run(&db, &uploader, whatsapp_meta()).await;
    assert_row(
        &row,
        json!({ "status": "completed", "successCount": 1, "failedCount": 0 }),
    );
    assert_eq!(
        contact_inboxes(&db).await,
        vec![(
            "user.9373928427292738".into(),
            Some("user.9373928427292738".into()),
            "imported".into()
        )]
    );
}

#[tokio::test]
async fn contacts_import_whatsapp_source_user_id_bsuid_skips_an_import_row_whose_source_user_id_matches_an_existing_phone_keyed_row_s_backfilled_bsuid()
 {
    let (db, uploader) = ready(&["wa_user_id", "user.9373928427292738"], "whatsapp").await;
    setup(&db, |tx| {
        contact(tx, 900, WS)?;
        contact_inbox(tx, 901, 900, INBOX, "whatsapp", "84912345678")?;
        tx.execute(
            r#"UPDATE "ContactInbox" SET "sourceUserId" = 'user.9373928427292738' WHERE "id" = 901"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let row = run(&db, &uploader, whatsapp_meta()).await;
    assert_row(
        &row,
        json!({ "status": "completed", "successCount": 0, "failedCount": 1 }),
    );
}
