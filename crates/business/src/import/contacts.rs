//! The contact import run (P3.6-D2 slice contact imports, part two):
//! `importService.startContactImport` / `markProcessing` / `fail` /
//! `complete`, the worker `runImport` dispatcher, `runImportPipeline`
//! (`base-import.ts`) and the contacts handler + extractor
//! (`handler/contacts/{handler,extractor}.ts`), `insertImportedContactBatch`.
//!
//! No default-queue worker is ported: the API runs [`run_import`] in the
//! request after [`start_contact_import`] (the `bulkTagContacts`
//! precedent). No quota / workspace-usage counters and no audit record (the
//! token path has no `userId`).

use std::collections::{HashMap, HashSet};

use chatbotx_core::js;
use chatbotx_core::utils::custom_field::{canonical_number_literal, coerce_boolean_literal};
use chatbotx_core::utils::datetime::{TemporalCustomFieldType, TemporalInputParsing};
use chatbotx_core::utils::temporal_input::{
    TemporalStorageNormalizationInput, normalize_temporal_value_for_storage,
};
use chatbotx_db::Db;
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};
use chatbotx_files::{FilesError, Uploader};
use phonenumber::PhoneNumber;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{BYTES_PER_MB, ImportType, config, csv, repo_err};
use crate::contact_custom_field::{NewContactValues, insert_normalized_values_for_new_contacts_in};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `GENERIC_IMPORT_FAILURE`: `toPublicErrorMessage` keeps only a
/// `ChatbotXException` / channel error message; every failure of the
/// contacts run is a plain string or `Error`, so the stored message is this.
pub const GENERIC_IMPORT_FAILURE: &str = "The import failed. Please try again or contact support.";

const MAX_FIELD_LENGTH: usize = 1000;
const MIN_PHONE_DIGITS: usize = 5;
const ERROR_SAMPLE_LIMIT: usize = 50;
/// `config.maxRows` of the contacts registry entry.
const CONTACTS_MAX_ROWS: u64 = 100_000;

/// `contactImportColumnMapSchema`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_user_id: Option<String>,
}

/// One `contactImportFieldMappingSchema` entry (`customFieldId` = a custom
/// field id or `bot_field:<id>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMapping {
    pub column: String,
    pub custom_field_id: String,
}

/// `contactImportMetaSchema` (the stored `Import.meta`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactImportMeta {
    pub channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    pub column_map: ColumnMap,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_mapping: Option<Vec<FieldMapping>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_id: Option<String>,
}

/// `startContactImport` input.
#[derive(Debug, Clone)]
pub struct StartContactImport {
    pub workspace_id: i64,
    pub user_id: Option<i64>,
    pub inbox_id: i64,
    pub file_id: i64,
    pub meta: ContactImportMeta,
}

/// `inferImportFormat`: by MIME type, then by extension.
fn infer_import_format(mime_type: &str, file_name: &str) -> Option<&'static str> {
    let by_mime = match mime_type {
        "text/csv" => Some("csv"),
        super::XLSX_MIME => Some("xlsx"),
        "application/vnd.ms-excel" => Some("xls"),
        "application/json" => Some("json"),
        _ => None,
    };
    by_mime.or_else(|| {
        let extension = file_name.rsplit_once('.')?.1.to_lowercase();
        super::IMPORT_FORMATS
            .iter()
            .copied()
            .find(|f| *f == extension)
    })
}

/// `importService.startContactImport`: checks the file (404
/// `contactImportFileNotFound`, 400 `contactImportFileTypeInvalid` /
/// `contactImportUnsupportedFormat`), the inbox (404
/// `contactImportInboxNotFound`) and that no contact import is running (409
/// `contactImportAlreadyRunning`), then marks the file uploaded and writes
/// the pending `Import` row. The id of the new import.
pub async fn start_contact_import(db: &Db, input: StartContactImport) -> Result<i64> {
    let meta = serde_json::to_string(&input.meta)
        .map_err(|e| BusinessError::Repo(chatbotx_db::repo::Error::msg(e.to_string())))?;
    tx::write(db, move |conn| {
        let ws = input.workspace_id;
        let file: Option<(String, Option<String>, String, String)> = conn
            .query_row(
                r#"SELECT "contextType", "subType", "mimeType", "fileName" FROM "File" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![input.file_id, ws],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(repo_err)?;
        let Some((context_type, sub_type, mime_type, file_name)) = file else {
            return Err(BusinessError::exception(
                "File not found",
                "contactImportFileNotFound",
                404,
            ));
        };
        if context_type != "import" || sub_type.as_deref() != Some("contacts") {
            return Err(BusinessError::exception(
                "File is not a contacts import",
                "contactImportFileTypeInvalid",
                400,
            ));
        }
        let format = infer_import_format(&mime_type, &file_name)
            .filter(|f| config(ImportType::Contacts).accepted_formats.contains(f));
        let Some(format) = format else {
            return Err(BusinessError::exception(
                "Unsupported file format",
                "contactImportUnsupportedFormat",
                400,
            ));
        };
        let inbox: Option<i64> = conn
            .query_row(
                r#"SELECT "id" FROM "Inbox" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![input.inbox_id, ws],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?;
        if inbox.is_none() {
            return Err(BusinessError::exception(
                "Inbox not found",
                "contactImportInboxNotFound",
                404,
            ));
        }
        let active: Option<i64> = conn
            .query_row(
                r#"SELECT "id" FROM "Import" WHERE "workspaceId" = ?1 AND "type" = 'contacts' AND "status" IN ('pending', 'processing') LIMIT 1"#,
                [ws],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?;
        if active.is_some() {
            return Err(BusinessError::exception(
                "An import is already in progress for this workspace. Please wait for it to complete.",
                "contactImportAlreadyRunning",
                409,
            ));
        }
        let now = now_ms();
        conn.execute(
            r#"UPDATE "File" SET "status" = 'uploaded', "uploadedAt" = ?3, "updatedAt" = ?3 WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![input.file_id, ws, now],
        )
        .map_err(repo_err)?;
        let import_id = new_id();
        conn.execute(
            r#"INSERT INTO "Import" ("id", "workspaceId", "userId", "inboxId", "fileId", "type", "format", "status", "meta", "createdAt", "updatedAt")
               VALUES (?1, ?2, ?3, ?4, ?5, 'contacts', ?6, 'pending', ?7, ?8, ?8)"#,
            params![import_id, ws, input.user_id, input.inbox_id, input.file_id, format, meta, now],
        )
        .map_err(repo_err)?;
        Ok(import_id)
    })
    .await
}

/// `ImportCounters`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub processed: u64,
    pub success: u64,
    pub failed: u64,
}

/// One `ImportErrorSample` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorSample {
    pub row: u64,
    pub reason: String,
}

fn sample_json(sample: &[ErrorSample]) -> String {
    Value::Array(
        sample
            .iter()
            .map(|e| json!({ "row": e.row, "reason": e.reason }))
            .collect(),
    )
    .to_string()
}

fn as_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

async fn mark_processing(db: &Db, import_id: i64) -> Result<()> {
    tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "Import" SET "status" = 'processing', "updatedAt" = ?2 WHERE "id" = ?1"#,
            params![import_id, now_ms()],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}

/// `importService.fail` (the public message is [`GENERIC_IMPORT_FAILURE`];
/// `reason` goes to the log). Without counters the counts stay as stored.
async fn fail(
    db: &Db,
    import_id: i64,
    reason: &str,
    counters: Option<Counters>,
    sample: Option<Vec<ErrorSample>>,
) -> Result<()> {
    tracing::warn!(import_id, reason, "contact import failed");
    tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"UPDATE "Import" SET "status" = 'failed', "errorMessage" = ?2, "completedAt" = ?3, "updatedAt" = ?3 WHERE "id" = ?1"#,
            params![import_id, GENERIC_IMPORT_FAILURE, now],
        )
        .map_err(repo_err)?;
        if let Some(c) = counters {
            conn.execute(
                r#"UPDATE "Import" SET "totalCount" = ?2, "processedCount" = ?2, "successCount" = ?3, "failedCount" = ?4 WHERE "id" = ?1"#,
                params![import_id, as_i64(c.processed), as_i64(c.success), as_i64(c.failed)],
            )
            .map_err(repo_err)?;
        }
        if let Some(sample) = sample {
            conn.execute(
                r#"UPDATE "Import" SET "errorSample" = ?2 WHERE "id" = ?1"#,
                params![import_id, sample_json(&sample)],
            )
            .map_err(repo_err)?;
        }
        Ok(())
    })
    .await
}

/// `importService.complete`.
async fn complete(db: &Db, import_id: i64, c: Counters, sample: Vec<ErrorSample>) -> Result<()> {
    let hidden = c.failed.saturating_sub(sample.len() as u64);
    let message = (hidden > 0).then(|| {
        format!(
            "{} rows failed; showing the first {}",
            c.failed,
            sample.len()
        )
    });
    tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"UPDATE "Import" SET "status" = 'completed', "completedAt" = ?2, "updatedAt" = ?2,
               "totalCount" = ?3, "processedCount" = ?3, "successCount" = ?4, "failedCount" = ?5,
               "errorSample" = ?6, "errorMessage" = ?7 WHERE "id" = ?1"#,
            params![
                import_id,
                now,
                as_i64(c.processed),
                as_i64(c.success),
                as_i64(c.failed),
                sample_json(&sample),
                message
            ],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}

// --- extractor ---------------------------------------------------------

/// `cleanCell`: a trimmed string cell of at most `max` characters
/// (UTF-16 units in the original), `None` when blank or not a string.
fn clean_cell(raw: Option<&Value>, max: usize) -> Option<String> {
    let text: String = js::trim(raw?.as_str()?).chars().take(max).collect();
    (!text.is_empty()).then_some(text)
}

/// `EMAIL_RE`: `^[^\s@]+@[^\s@]+\.[^\s@]+$`.
pub fn is_email(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    let ok = |s: &str| !s.is_empty() && !s.chars().any(|c| c.is_whitespace() || c == '@');
    if !ok(local) || domain.contains('@') {
        return false;
    }
    // `[^\s@]+\.[^\s@]+` over the domain: some dot with text on both sides.
    domain
        .char_indices()
        .any(|(i, c)| c == '.' && ok(&domain[..i]) && ok(&domain[i + 1..]))
}

/// `PHONE_RE`: `^\+?[0-9 ()\-.]{5,32}$`.
pub fn is_phone(text: &str) -> bool {
    let body = text.strip_prefix('+').unwrap_or(text);
    let len = body.chars().count();
    (5..=32).contains(&len)
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ' ' | '(' | ')' | '-' | '.'))
}

fn clean_email(raw: Option<&Value>) -> Option<String> {
    let lower = clean_cell(raw, MAX_FIELD_LENGTH)?.to_lowercase();
    is_email(&lower).then_some(lower)
}

/// `cleanPhone`: the digits (and `+`) of a phone-shaped cell.
fn clean_phone(raw: Option<&Value>) -> Option<String> {
    let text = clean_cell(raw, MAX_FIELD_LENGTH)?;
    if !is_phone(&text) {
        return None;
    }
    let digits: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    (digits.chars().count() >= MIN_PHONE_DIGITS).then_some(digits)
}

fn e164(number: &PhoneNumber) -> String {
    number.format().mode(phonenumber::Mode::E164).to_string()
}

/// `normalizePhone` (`parsePhoneNumberFromString(phone, { defaultCallingCode })`
/// and `isValid()`): a national number is read in each region of the calling
/// code; the first valid reading wins.
pub fn normalize_import_phone(phone: Option<&str>, country_code: Option<&str>) -> Option<String> {
    let phone = phone?;
    let normalized = match phone.strip_prefix("00") {
        Some(rest) => format!("+{rest}"),
        None => phone.to_string(),
    };
    if normalized.starts_with('+') {
        return phonenumber::parse(None, &normalized)
            .ok()
            .filter(PhoneNumber::is_valid)
            .map(|n| e164(&n));
    }
    let code: u16 = country_code
        .map(|c| c.strip_prefix('+').unwrap_or(c))?
        .parse()
        .ok()?;
    let regions = phonenumber::metadata::DATABASE.by_code(&code)?;
    regions.iter().find_map(|meta| {
        let region = meta.id().parse::<phonenumber::country::Id>().ok()?;
        phonenumber::parse(Some(region), &normalized)
            .ok()
            .filter(PhoneNumber::is_valid)
            .map(|n| e164(&n))
    })
}

/// `ContactRow`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactRow {
    pub external_id: Option<String>,
    pub phone_number: Option<String>,
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub source_user_id: Option<String>,
    /// `(customFieldId, value)`.
    pub custom_fields: Vec<(String, String)>,
}

fn pick<'a>(row: &'a Map<String, Value>, column: Option<&String>) -> Option<&'a Value> {
    column.and_then(|c| row.get(c))
}

/// `extractRowData`: the mapped row, `None` without a phone, an email or an
/// external id.
pub fn extract_row_data(
    row: &Map<String, Value>,
    column_map: &ColumnMap,
    field_mapping: &[FieldMapping],
    country_code: Option<&str>,
    channel: Option<&str>,
) -> Option<ContactRow> {
    let email = clean_email(pick(row, column_map.email.as_ref()));
    let first_name = clean_cell(pick(row, column_map.first_name.as_ref()), MAX_FIELD_LENGTH);
    let last_name = clean_cell(pick(row, column_map.last_name.as_ref()), MAX_FIELD_LENGTH);
    let phone_number = normalize_import_phone(
        clean_phone(pick(row, column_map.phone_number.as_ref())).as_deref(),
        country_code,
    );
    let (external_id, source_user_id) = if channel == Some("whatsapp") {
        let source_user_id = clean_cell(
            pick(row, column_map.source_user_id.as_ref()),
            MAX_FIELD_LENGTH,
        );
        let from_phone = phone_number
            .as_deref()
            .map(|p| p.strip_prefix('+').unwrap_or(p).to_string());
        (
            from_phone.or_else(|| source_user_id.clone()),
            source_user_id,
        )
    } else {
        (
            clean_cell(pick(row, column_map.contact_id.as_ref()), MAX_FIELD_LENGTH),
            None,
        )
    };
    if phone_number.is_none() && email.is_none() && external_id.is_none() {
        return None;
    }
    let custom_fields = field_mapping
        .iter()
        .filter_map(|m| {
            clean_cell(row.get(&m.column), MAX_FIELD_LENGTH)
                .map(|value| (m.custom_field_id.clone(), value))
        })
        .collect();
    Some(ContactRow {
        external_id,
        phone_number,
        email,
        first_name,
        last_name,
        source_user_id,
        custom_fields,
    })
}

/// `validateCustomFieldValue` (`normalizeCustomFieldValueByType`, lenient
/// temporal parsing): the canonical value, `None` = skip.
pub fn validate_custom_field_value(
    kind: &str,
    raw: &str,
    timezone: Option<&str>,
) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    match kind {
        "boolean" => Some(coerce_boolean_literal(raw).to_string()),
        "number" => canonical_number_literal(raw),
        "email" => {
            let lower = raw.to_lowercase();
            is_email(&lower).then_some(lower)
        }
        "phoneNumber" => {
            if !is_phone(raw) {
                return None;
            }
            let digits: String = raw
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '+')
                .collect();
            (!digits.is_empty()).then_some(digits)
        }
        "date" | "datetime" => normalize_temporal_value_for_storage(
            &TemporalStorageNormalizationInput {
                kind: TemporalCustomFieldType::parse(kind)?,
                value: raw,
                timezone,
                parsing: Some(TemporalInputParsing::Lenient),
            },
            now_ms(),
        ),
        _ => Some(raw.to_string()),
    }
}

// --- batch insert --------------------------------------------------------

/// One accepted row of `insertImportedContactBatch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedContact {
    pub contact_id: i64,
    pub contact_inbox_id: i64,
    pub row: ContactRow,
}

/// `insertImportedContactBatch` result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InsertedBatch {
    pub inserted: u64,
    pub orphan_count: u64,
}

/// `insertImportedContactBatch`: the contacts, their `imported` contact
/// inboxes (a conflicting link drops its contact), the message-cleanup
/// cancel, the conversations, the custom-field values and the tag, in the
/// caller's transaction.
pub fn insert_imported_contact_batch(
    tx: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    channel: &str,
    accepted: &[AcceptedContact],
    tag_id: Option<i64>,
) -> Result<InsertedBatch> {
    if accepted.is_empty() {
        return Ok(InsertedBatch::default());
    }
    for a in accepted {
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "workspaceId", "phoneNumber", "email", "firstName", "lastName") VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![
                a.contact_id,
                workspace_id,
                a.row.phone_number,
                a.row.email,
                a.row.first_name,
                a.row.last_name
            ],
        )
        .map_err(repo_err)?;
    }
    let mut survivors: Vec<&AcceptedContact> = Vec::new();
    let mut orphans: Vec<i64> = Vec::new();
    for a in accepted {
        let Some(source_id) = a.row.external_id.as_deref() else {
            return Err(BusinessError::Repo(chatbotx_db::repo::Error::msg(
                "Invariant: externalId must be set before insert",
            )));
        };
        let inserted = tx
            .execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId", "sourceUserId")
                   VALUES (?1, ?2, ?2, ?3, ?4, 'imported', ?5, ?6) ON CONFLICT DO NOTHING"#,
                params![a.contact_inbox_id, a.contact_id, inbox_id, channel, source_id, a.row.source_user_id],
            )
            .map_err(repo_err)?;
        if inserted == 1 {
            survivors.push(a);
        } else {
            orphans.push(a.contact_id);
        }
    }
    for a in &survivors {
        tx.execute(
            r#"DELETE FROM "MessageCleanup" WHERE "inboxId" = ?1 AND "sourceId" = ?2"#,
            params![inbox_id, a.row.external_id],
        )
        .map_err(repo_err)?;
    }
    if !orphans.is_empty() {
        let marks = vec!["?"; orphans.len()].join(", ");
        tx.execute(
            &format!(r#"DELETE FROM "Contact" WHERE "id" IN ({marks})"#),
            params_from_iter(orphans.iter()),
        )
        .map_err(repo_err)?;
    }
    let orphan_count = orphans.len() as u64;
    if survivors.is_empty() {
        return Ok(InsertedBatch {
            inserted: 0,
            orphan_count,
        });
    }
    for a in &survivors {
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (?1, ?2, ?3)"#,
            params![new_id(), workspace_id, a.contact_id],
        )
        .map_err(repo_err)?;
    }
    let entries: Vec<NewContactValues> = survivors
        .iter()
        .map(|a| NewContactValues {
            contact_id: a.contact_id,
            fields: a
                .row
                .custom_fields
                .iter()
                .filter_map(|(id, value)| Some((id.parse().ok()?, value.clone())))
                .collect(),
        })
        .collect();
    insert_normalized_values_for_new_contacts_in(tx, workspace_id, &entries)?;
    if let Some(tag_id) = tag_id {
        for a in &survivors {
            tx.execute(
                r#"INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES (?1, ?2) ON CONFLICT DO NOTHING"#,
                params![a.contact_id, tag_id],
            )
            .map_err(repo_err)?;
        }
    }
    Ok(InsertedBatch {
        inserted: survivors.len() as u64,
        orphan_count,
    })
}

// --- run -----------------------------------------------------------------

/// The `prepare` dependencies of the contacts handler.
struct Deps {
    inbox_channel: String,
    custom_field_types: HashMap<String, String>,
    custom_mappings: Vec<FieldMapping>,
    /// `(column, botFieldId)`.
    bot_mappings: Vec<(String, i64)>,
    bot_field_types: HashMap<i64, String>,
    bot_field_latest: Vec<(i64, String)>,
}

/// The stored row the run reads (`findForWorker`).
struct RunRow {
    workspace_id: i64,
    inbox_id: Option<i64>,
    import_type: String,
    format: String,
    meta: String,
    path: Option<String>,
}

/// `runImport`: runs the import `import_id` (a missing row is a no-op). Only
/// contact imports are ported; another type fails the row.
pub async fn run_import(db: &Db, uploader: &Uploader, import_id: i64) -> Result<()> {
    let row = tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT i."workspaceId", i."inboxId", i."type", i."format", i."meta", f."path"
               FROM "Import" i LEFT JOIN "File" f ON f."id" = i."fileId" WHERE i."id" = ?1"#,
            [import_id],
            |r| {
                Ok(RunRow {
                    workspace_id: r.get(0)?,
                    inbox_id: r.get(1)?,
                    import_type: r.get(2)?,
                    format: r.get(3)?,
                    meta: r.get(4)?,
                    path: r.get(5)?,
                })
            },
        )
        .optional()
        .map_err(repo_err)
    })
    .await?;
    let Some(row) = row else {
        tracing::warn!(import_id, "Import row not found");
        return Ok(());
    };
    if row.import_type == "flow" {
        return crate::flow::import::run_flow_import(db, uploader, import_id).await;
    }
    let Some(path) = row.path.clone() else {
        return fail(db, import_id, "Associated file not found", None, None).await;
    };
    if row.import_type != "contacts" {
        return fail(
            db,
            import_id,
            &format!("Unsupported import type: {}", row.import_type),
            None,
            None,
        )
        .await;
    }
    if let Err(error) = run_contacts(db, uploader, import_id, &row, &path).await {
        fail(db, import_id, &error.to_string(), None, None).await?;
    }
    Ok(())
}

/// `runImportPipeline` with the contacts handler.
async fn run_contacts(
    db: &Db,
    uploader: &Uploader,
    import_id: i64,
    row: &RunRow,
    path: &str,
) -> Result<()> {
    let meta: ContactImportMeta = match serde_json::from_str(&row.meta) {
        Ok(meta) => meta,
        Err(error) => return fail(db, import_id, &error.to_string(), None, None).await,
    };
    let config = config(ImportType::Contacts);
    if !config.accepted_formats.contains(&row.format.as_str()) {
        let reason = format!("{} is not supported for contacts imports", row.format);
        return fail(db, import_id, &reason, None, None).await;
    }
    mark_processing(db, import_id).await?;
    let mut deps = match prepare(db, row, &meta).await? {
        Ok(deps) => deps,
        Err(reason) => return fail(db, import_id, reason, None, None).await,
    };
    let max_bytes = config.max_file_size_mb * BYTES_PER_MB;
    let too_large = format!("File exceeds {}MB limit", config.max_file_size_mb);
    // `loadImportObject`: the stored size first (a failed head falls back
    // to the read), then the bytes read.
    if let Ok(head) = uploader.head_object(path).await
        && i64::try_from(head.content_length).unwrap_or(i64::MAX) > max_bytes
    {
        return fail(db, import_id, &too_large, None, None).await;
    }
    let bytes = match uploader.get_object(path).await {
        Ok(bytes) => bytes,
        Err(FilesError::NotFound(_)) => {
            return fail(db, import_id, "The file was not found", None, None).await;
        }
        Err(error) => return fail(db, import_id, &error.to_string(), None, None).await,
    };
    if i64::try_from(bytes.len()).unwrap_or(i64::MAX) > max_bytes {
        return fail(db, import_id, &too_large, None, None).await;
    }
    let mut counters = Counters::default();
    let mut sample: Vec<ErrorSample> = Vec::new();
    let records = match csv::parse_records(&String::from_utf8_lossy(&bytes)) {
        Ok(records) => records,
        Err(error) => {
            return fail(db, import_id, &error.0, Some(counters), Some(sample)).await;
        }
    };
    let mut records = records.into_iter();
    let headers = records.next().unwrap_or_default();
    let mut buffer: Vec<ContactRow> = Vec::new();
    for record in records {
        if counters.processed >= CONTACTS_MAX_ROWS {
            let reason = format!("Row limit exceeded ({CONTACTS_MAX_ROWS})");
            return fail(db, import_id, &reason, Some(counters), Some(sample)).await;
        }
        // `columns: true`: a record of another length is a parse error.
        if record.len() != headers.len() {
            let reason = format!(
                "Invalid Record Length: columns length is {}, got {}",
                headers.len(),
                record.len()
            );
            return fail(db, import_id, &reason, Some(counters), Some(sample)).await;
        }
        counters.processed += 1;
        let mut raw = Map::new();
        for (header, value) in headers.iter().zip(record) {
            raw.insert(header.clone(), Value::String(value));
        }
        match process_row(&mut deps, &raw, &meta) {
            Some(mapped) => buffer.push(mapped),
            None => {
                counters.failed += 1;
                if sample.len() < ERROR_SAMPLE_LIMIT {
                    sample.push(ErrorSample {
                        row: counters.processed + 1,
                        reason: "Invalid row".into(),
                    });
                }
            }
        }
    }
    // ponytail: one batch (the original flushes every 1000 rows; the file
    // is at most 20 MB and already in memory).
    let (success, failed) = process_batch(db, row, &meta, &deps, buffer).await;
    counters.success += success;
    counters.failed += failed;
    complete(db, import_id, counters, sample).await?;
    // `finalizeContacts`: the latest valid bot-field values, once.
    for (bot_field_id, value) in &deps.bot_field_latest {
        let data = crate::bot_field::UpdateBotField {
            value: Some(Some(value.clone())),
            ..Default::default()
        };
        if let Err(error) =
            crate::bot_field::update_by_key(db, row.workspace_id, &bot_field_id.to_string(), data)
                .await
        {
            tracing::error!(%error, bot_field_id, "Import bot field value update failed; skipping");
        }
    }
    Ok(())
}

/// `prepareContacts`: `Err(reason)` fails the import.
async fn prepare(
    db: &Db,
    row: &RunRow,
    meta: &ContactImportMeta,
) -> Result<std::result::Result<Deps, &'static str>> {
    let mut custom_mappings = Vec::new();
    let mut bot_mappings = Vec::new();
    for mapping in meta.field_mapping.iter().flatten() {
        match mapping
            .custom_field_id
            .strip_prefix("bot_field:")
            .and_then(|id| id.parse::<i64>().ok())
        {
            Some(id) => bot_mappings.push((mapping.column.clone(), id)),
            None => custom_mappings.push(mapping.clone()),
        }
    }
    let ws = row.workspace_id;
    let inbox_id = row.inbox_id;
    let tag_id = meta.tag_id.clone();
    let custom_ids: Vec<String> = custom_mappings
        .iter()
        .map(|m| m.custom_field_id.clone())
        .collect();
    let bot_ids: Vec<i64> = bot_mappings.iter().map(|(_, id)| *id).collect();
    let lookups = tx::read(db, move |conn| {
        let inbox: Option<String> = match inbox_id {
            Some(id) => conn
                .query_row(
                    r#"SELECT "channel" FROM "Inbox" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, ws],
                    |r| r.get(0),
                )
                .optional()
                .map_err(repo_err)?,
            None => None,
        };
        let tag_found = match tag_id.as_deref() {
            Some(id) => conn
                .query_row(
                    r#"SELECT 1 FROM "Tag" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id.parse::<i64>().unwrap_or(-1), ws],
                    |_| Ok(()),
                )
                .optional()
                .map_err(repo_err)?
                .is_some(),
            None => true,
        };
        let mut custom = HashMap::new();
        for id in &custom_ids {
            let Ok(numeric) = id.parse::<i64>() else {
                continue;
            };
            let kind: Option<String> = conn
                .query_row(
                    r#"SELECT "type" FROM "CustomField" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![numeric, ws],
                    |r| r.get(0),
                )
                .optional()
                .map_err(repo_err)?;
            if let Some(kind) = kind {
                custom.insert(id.clone(), kind);
            }
        }
        let mut bot = HashMap::new();
        for id in &bot_ids {
            let kind: Option<String> = conn
                .query_row(
                    r#"SELECT "type" FROM "BotField" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, ws],
                    |r| r.get(0),
                )
                .optional()
                .map_err(repo_err)?;
            if let Some(kind) = kind {
                bot.insert(*id, kind);
            }
        }
        Ok((inbox, tag_found, custom, bot))
    })
    .await?;
    let (inbox, tag_found, custom_field_types, bot_field_types) = lookups;
    let Some(inbox_channel) = inbox else {
        return Ok(Err("Inbox not found"));
    };
    if !tag_found {
        return Ok(Err("Tag not found in workspace"));
    }
    Ok(Ok(Deps {
        inbox_channel,
        custom_field_types,
        custom_mappings,
        bot_mappings,
        bot_field_types,
        bot_field_latest: Vec::new(),
    }))
}

/// `processContactRow`.
fn process_row(
    deps: &mut Deps,
    raw: &Map<String, Value>,
    meta: &ContactImportMeta,
) -> Option<ContactRow> {
    let mut mapped = extract_row_data(
        raw,
        &meta.column_map,
        &deps.custom_mappings,
        meta.country_code.as_deref(),
        Some(meta.channel.as_str()),
    )?;
    let timezone = meta.timezone.as_deref();
    for (column, bot_field_id) in &deps.bot_mappings {
        let Some(kind) = deps.bot_field_types.get(bot_field_id) else {
            continue;
        };
        let Some(value) = clean_cell(raw.get(column), MAX_FIELD_LENGTH) else {
            continue;
        };
        if let Some(normalized) = validate_custom_field_value(kind, &value, timezone) {
            deps.bot_field_latest.retain(|(id, _)| id != bot_field_id);
            deps.bot_field_latest.push((*bot_field_id, normalized));
        }
    }
    mapped.custom_fields = mapped
        .custom_fields
        .into_iter()
        .filter_map(|(id, value)| {
            let kind = deps.custom_field_types.get(&id)?;
            validate_custom_field_value(kind, &value, timezone).map(|v| (id, v))
        })
        .collect();
    Some(mapped)
}

/// `processContactBatch`: `(success, failed)`; a write error fails the
/// whole batch (logged).
async fn process_batch(
    db: &Db,
    row: &RunRow,
    meta: &ContactImportMeta,
    deps: &Deps,
    rows: Vec<ContactRow>,
) -> (u64, u64) {
    let total = rows.len() as u64;
    let mut seen = HashSet::new();
    let contacts: Vec<ContactRow> = rows
        .into_iter()
        .filter(|r| {
            r.external_id
                .as_ref()
                .is_some_and(|id| seen.insert(id.clone()))
        })
        .collect();
    if contacts.is_empty() {
        return (0, total);
    }
    let ws = row.workspace_id;
    let Some(inbox_id) = row.inbox_id else {
        return (0, total);
    };
    let channel = deps.inbox_channel.clone();
    let tag_id = meta.tag_id.as_deref().and_then(|t| t.parse::<i64>().ok());
    let result = tx::write(db, move |conn| {
        let eligible = filter_eligible_rows(conn, inbox_id, contacts)?;
        let accepted: Vec<AcceptedContact> = eligible
            .into_iter()
            .map(|row| AcceptedContact {
                contact_id: new_id(),
                contact_inbox_id: new_id(),
                row,
            })
            .collect();
        let batch = insert_imported_contact_batch(conn, ws, inbox_id, &channel, &accepted, tag_id)?;
        if batch.orphan_count > 0 {
            tracing::warn!(
                inbox_id,
                conflicts = batch.orphan_count,
                "Import contact source conflict: skipped already-linked contacts"
            );
        }
        Ok(batch.inserted)
    })
    .await;
    match result {
        Ok(inserted) => (inserted, total - inserted),
        Err(error) => {
            tracing::error!(%error, "Import batch failed");
            (0, total)
        }
    }
}

/// `filterEligibleRows` (`findExistingSourceIdentities`): drops the rows
/// whose external id is a `sourceId`, or whose `sourceUserId` is a
/// `sourceUserId`, of the inbox.
fn filter_eligible_rows(
    conn: &Connection,
    inbox_id: i64,
    rows: Vec<ContactRow>,
) -> Result<Vec<ContactRow>> {
    let mut out = Vec::new();
    for row in rows {
        let taken: Option<i64> = conn
            .query_row(
                r#"SELECT 1 FROM "ContactInbox" WHERE "inboxId" = ?1 AND ("sourceId" = ?2 OR ("sourceUserId" IS NOT NULL AND "sourceUserId" = ?3)) LIMIT 1"#,
                params![inbox_id, row.external_id, row.source_user_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?;
        if taken.is_none() {
            out.push(row);
        }
    }
    Ok(out)
}
