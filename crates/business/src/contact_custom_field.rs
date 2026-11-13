//! `contact-custom-field/normalize.ts` `normalizeCustomFieldValueForStorage`
//! and `service.ts` `setValuesInTransaction` (strict parsing, the
//! contact-then-workspace source timezone) — P3.6-D2 contacts core.

use chatbotx_core::utils::custom_field::{canonical_number_literal, coerce_boolean_literal};
use chatbotx_core::utils::datetime::{
    DEFAULT_FILTER_TIMEZONE, TemporalCustomFieldType, has_explicit_offset, resolve_filter_timezone,
};
use chatbotx_core::utils::temporal_input::{
    TemporalStorageNormalizationInput, normalize_temporal_value_for_storage,
};
use chatbotx_core::variables::system_field::normalize_stored_timezone;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};

use crate::contact::update_fields::CustomFieldChange;
use crate::{BusinessError, Result};

const MAX_ERROR_VALUE_LENGTH: usize = 80;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn preview(value: &str) -> String {
    if value.chars().count() > MAX_ERROR_VALUE_LENGTH {
        format!(
            "{}…",
            value
                .chars()
                .take(MAX_ERROR_VALUE_LENGTH)
                .collect::<String>()
        )
    } else {
        value.to_string()
    }
}

/// `createSourceTimezoneResolver` (contact then workspace timezone).
fn source_timezone(conn: &Connection, workspace_id: i64, contact_id: i64) -> Result<String> {
    let contact: Option<String> = conn
        .query_row(
            r#"SELECT "timezone" FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![contact_id, workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(repo_err)?
        .flatten();
    let workspace: Option<String> = conn
        .query_row(
            r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
            [workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(repo_err)?
        .flatten();
    let zone = normalize_stored_timezone(contact.as_deref())
        .or_else(|| normalize_stored_timezone(workspace.as_deref()));
    Ok(resolve_filter_timezone(zone.as_deref()))
}

/// `normalizeCustomFieldValueForStorage` (strict): `None` = an unparseable
/// temporal value (skipped).
pub fn normalize_value_for_storage(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    kind: &str,
    value: &str,
    explicit_timezone: Option<&str>,
    now_ms: i64,
) -> Result<Option<String>> {
    let Some(temporal) = TemporalCustomFieldType::parse(kind) else {
        return match kind {
            "boolean" if value.trim().is_empty() => Ok(Some(String::new())),
            "boolean" => Ok(Some(coerce_boolean_literal(value).to_string())),
            "number" if value.trim().is_empty() => Ok(Some(String::new())),
            "number" => canonical_number_literal(value).map(Some).ok_or_else(|| {
                BusinessError::exception(
                    format!(
                        "\"{}\" is not a valid number value for this field.",
                        preview(value)
                    ),
                    "invalidCustomFieldValue",
                    400,
                )
            }),
            _ => Ok(Some(value.to_string())),
        };
    };
    if value.is_empty() {
        return Ok(Some(String::new()));
    }
    // `resolveTemporalSourceTimezone`.
    let needs_zone = match temporal {
        TemporalCustomFieldType::Date => true,
        TemporalCustomFieldType::Datetime => !has_explicit_offset(value),
    };
    let zone = if !needs_zone {
        DEFAULT_FILTER_TIMEZONE.to_string()
    } else if let (TemporalCustomFieldType::Date, Some(explicit)) =
        (temporal, normalize_stored_timezone(explicit_timezone))
    {
        resolve_filter_timezone(Some(&explicit))
    } else {
        source_timezone(conn, workspace_id, contact_id)?
    };
    Ok(normalize_temporal_value_for_storage(
        &TemporalStorageNormalizationInput {
            kind: temporal,
            value,
            timezone: Some(&zone),
            parsing: None,
        },
        now_ms,
    ))
}

/// `setValuesInTransaction`: the stored changes (unchanged values and
/// unknown fields are skipped).
pub fn set_values_in_tx(
    tx: &Connection,
    workspace_id: i64,
    contact_id: i64,
    fields: &[(i64, String)],
    source_timezone: Option<&str>,
    now_ms: i64,
) -> Result<Vec<CustomFieldChange>> {
    let mut changes = Vec::new();
    for (custom_field_id, value) in fields {
        let field: Option<(String, String)> = tx
            .query_row(
                r#"SELECT "name", "type" FROM "CustomField" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                params![workspace_id, custom_field_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(repo_err)?;
        let Some((name, kind)) = field else { continue };
        let Some(normalized) = normalize_value_for_storage(
            tx,
            workspace_id,
            contact_id,
            &kind,
            value,
            source_timezone,
            now_ms,
        )?
        else {
            continue;
        };
        let existing: Option<(i64, String)> = tx
            .query_row(
                r#"SELECT "id", "value" FROM "ContactCustomField" WHERE "contactId" = ?1 AND "customFieldId" = ?2"#,
                params![contact_id, custom_field_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(repo_err)?;
        match &existing {
            Some((_, old)) if *old == normalized => continue,
            Some((id, _)) => {
                tx.execute(
                    r#"UPDATE "ContactCustomField" SET "value" = ?2, "updatedAt" = ?3 WHERE "id" = ?1"#,
                    params![id, normalized, now_ms],
                )
                .map_err(repo_err)?;
            }
            None => {
                tx.execute(
                    r#"INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value") VALUES (?1, ?2, ?3, ?4)"#,
                    params![crate::contact::create::new_id(), contact_id, custom_field_id, normalized],
                )
                .map_err(repo_err)?;
            }
        }
        changes.push(CustomFieldChange {
            custom_field_id: *custom_field_id,
            custom_field_name: name,
            old_value: existing.map(|(_, old)| old),
            new_value: normalized,
        });
    }
    Ok(changes)
}

// ---- P3.6-D2 slice contact custom-field values ---------------------------

pub use chatbotx_core::utils::custom_field_operation::FieldOperationType;
pub use chatbotx_core::utils::datetime::{SourceTimezoneStrategy, TemporalInputParsing};
use chatbotx_db::Db;
use chatbotx_db::repo::contact::ContactCustomFieldWithDefinition;

/// One stored change (`PendingContactCustomFieldChange` + its contact;
/// the original emits `customFieldChanged` from it). `new_value: None` = the
/// value was deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChange {
    pub contact_id: i64,
    pub custom_field_id: i64,
    pub custom_field_name: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
}

/// The `normalizeCustomFieldValueForStorage` input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizeInput<'a> {
    /// The `CustomFieldType`.
    pub kind: &'a str,
    pub value: &'a str,
    /// The submitter's zone (only `date` honors it).
    pub explicit_timezone: Option<&'a str>,
    pub parsing: TemporalInputParsing,
    pub fill_empty_temporal_with_now: bool,
}

impl<'a> NormalizeInput<'a> {
    /// Strict parsing, no explicit zone, no "now" fill.
    pub fn new(kind: &'a str, value: &'a str) -> NormalizeInput<'a> {
        NormalizeInput {
            kind,
            value,
            explicit_timezone: None,
            parsing: TemporalInputParsing::Strict,
            fill_empty_temporal_with_now: false,
        }
    }
}

/// `normalizeCustomFieldValueForStorage`: `None` = an unparseable temporal
/// value (skipped). `resolve_source_timezone` is called at most once, and
/// only when the value needs a source zone.
pub fn normalize_custom_field_value_for_storage(
    input: &NormalizeInput<'_>,
    resolve_source_timezone: &mut dyn FnMut() -> Result<String>,
    now_ms: i64,
) -> Result<Option<String>> {
    let _ = (input, resolve_source_timezone, now_ms);
    todo!("P3.6-D2")
}

/// `createSourceTimezoneResolver`: memoized contact-then-workspace (or
/// workspace) zone; an explicit source zone wins without a lookup.
#[derive(Debug, Clone)]
pub struct SourceTimezoneResolver {
    workspace_id: i64,
    contact_id: i64,
    strategy: SourceTimezoneStrategy,
    zone: Option<String>,
}

impl SourceTimezoneResolver {
    pub fn new(
        workspace_id: i64,
        contact_id: i64,
        strategy: SourceTimezoneStrategy,
        explicit_source_timezone: Option<&str>,
    ) -> SourceTimezoneResolver {
        let _ = explicit_source_timezone;
        SourceTimezoneResolver {
            workspace_id,
            contact_id,
            strategy,
            zone: None,
        }
    }

    /// The zone (looked up on the first call only).
    pub fn resolve(&mut self, conn: &Connection) -> Result<String> {
        let _ = (conn, self.workspace_id, self.contact_id, self.strategy, &self.zone);
        todo!("P3.6-D2")
    }
}

/// `SetValuesInput` (`None` = the default: contact-then-workspace zone,
/// strict parsing).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetValuesInput {
    pub workspace_id: i64,
    pub contact_id: i64,
    /// `(customFieldId, value)`.
    pub fields: Vec<(i64, String)>,
    pub source_timezone: Option<String>,
    pub source_timezone_override: Option<String>,
    pub source_timezone_strategy: Option<SourceTimezoneStrategy>,
    pub temporal_input_parsing: Option<TemporalInputParsing>,
    pub fill_empty_temporal_with_now: bool,
    pub touch_unchanged: bool,
}

/// `setValuesInTransaction` (`writeValues`): the stored changes.
pub fn set_values_in_transaction(
    tx: &Connection,
    input: &SetValuesInput,
    now_ms: i64,
) -> Result<Vec<FieldChange>> {
    let _ = (tx, input, now_ms);
    todo!("P3.6-D2")
}

/// `setValues`: one write transaction; the stored changes.
pub async fn set_values(db: &Db, input: SetValuesInput) -> Result<Vec<FieldChange>> {
    let _ = (db, input);
    todo!("P3.6-D2")
}

/// `setValueForContact`: the contact and the field must be in the workspace.
pub async fn set_value_for_contact(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    custom_field_id: i64,
    value: &str,
    source_timezone: Option<&str>,
) -> Result<Vec<FieldChange>> {
    let _ = (
        db,
        workspace_id,
        contact_id,
        custom_field_id,
        value,
        source_timezone,
    );
    todo!("P3.6-D2")
}

/// `FieldOperationRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldOperationRequest {
    pub contact_id: i64,
    pub custom_field_id: i64,
    pub operation: FieldOperationType,
    pub operand: String,
}

/// `ApplyFieldOperationsInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyFieldOperationsInput {
    pub workspace_id: i64,
    /// Applied in order; a later operation sees the earlier results.
    pub operations: Vec<FieldOperationRequest>,
    pub source_timezone: Option<String>,
    pub source_timezone_override: Option<String>,
    pub temporal_input_parsing: Option<TemporalInputParsing>,
    pub fill_empty_temporal_with_now: bool,
}

/// `applyFieldOperations`: one transaction; strict (throws
/// `invalidFieldOperation`) when it targets one contact, else a rejected
/// operation is skipped. Contacts outside the workspace are dropped.
pub async fn apply_field_operations(
    db: &Db,
    input: ApplyFieldOperationsInput,
) -> Result<Vec<FieldChange>> {
    let _ = (db, input);
    todo!("P3.6-D2")
}

/// `applyOperationToContacts`.
pub async fn apply_operation_to_contacts(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    custom_field_id: i64,
    operation: FieldOperationType,
    value: &str,
    source_timezone: Option<&str>,
) -> Result<Vec<FieldChange>> {
    let _ = (
        db,
        workspace_id,
        contact_ids,
        custom_field_id,
        operation,
        value,
        source_timezone,
    );
    todo!("P3.6-D2")
}

/// One operation of `applyOperations`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldOperation {
    pub custom_field_id: i64,
    pub operation: FieldOperationType,
    pub value: String,
}

/// `applyOperations`: operations on one contact, all-or-nothing.
pub async fn apply_operations(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    operations: Vec<FieldOperation>,
    source_timezone: Option<&str>,
) -> Result<Vec<FieldChange>> {
    let _ = (db, workspace_id, contact_id, operations, source_timezone);
    todo!("P3.6-D2")
}

/// `listWithDefinitionByContact`.
pub async fn list_with_definition_by_contact(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<ContactCustomFieldWithDefinition>> {
    let _ = (db, workspace_id, contact_id);
    todo!("P3.6-D2")
}

/// `findWithDefinition`.
pub async fn find_with_definition(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    custom_field_id: i64,
) -> Result<Option<ContactCustomFieldWithDefinition>> {
    let _ = (db, workspace_id, contact_id, custom_field_id);
    todo!("P3.6-D2")
}

/// `clearByContactId`.
pub async fn clear_by_contact_id(db: &Db, workspace_id: i64, contact_id: i64) -> Result<()> {
    let _ = (db, workspace_id, contact_id);
    todo!("P3.6-D2")
}

/// `deleteByCustomFieldId` (no change records).
pub async fn delete_by_custom_field_id(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    custom_field_id: i64,
) -> Result<()> {
    let _ = (db, workspace_id, contact_ids, custom_field_id);
    todo!("P3.6-D2")
}

/// `SetValueByKeyInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetValueByKeyInput {
    pub workspace_id: i64,
    pub contact_id: i64,
    /// Field id or name (or `bot_field:<id>` with `allow_bot_fields`).
    pub keyword: String,
    pub value: String,
    pub source_timezone_override: Option<String>,
    pub temporal_input_parsing: Option<TemporalInputParsing>,
    pub fill_empty_temporal_with_now: bool,
    /// `None` = `set`.
    pub operation: Option<FieldOperationType>,
    pub touch_unchanged: bool,
    pub allow_bot_fields: bool,
}

/// `setValueByKey`: the stored contact changes (empty for a bot field).
pub async fn set_value_by_key(db: &Db, input: SetValueByKeyInput) -> Result<Vec<FieldChange>> {
    let _ = (db, input);
    todo!("P3.6-D2")
}

/// `deleteByKey`: the deleted value (`None` = no value, or a bot field).
pub async fn delete_by_key(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    keyword: &str,
    allow_bot_fields: bool,
) -> Result<Option<FieldChange>> {
    let _ = (db, workspace_id, contact_id, keyword, allow_bot_fields);
    todo!("P3.6-D2")
}

/// One entry of `insertNormalizedValuesForNewContacts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewContactValues {
    pub contact_id: i64,
    /// `(customFieldId, value)`, already normalized.
    pub fields: Vec<(i64, String)>,
}

/// `insertNormalizedValuesForNewContacts`: refuses a non-canonical boolean /
/// number before any insert.
pub fn insert_normalized_values_for_new_contacts_in(
    tx: &Connection,
    workspace_id: i64,
    entries: &[NewContactValues],
) -> Result<()> {
    let _ = (tx, workspace_id, entries);
    todo!("P3.6-D2")
}

/// [`insert_normalized_values_for_new_contacts_in`] in its own transaction.
pub async fn insert_normalized_values_for_new_contacts(
    db: &Db,
    workspace_id: i64,
    entries: Vec<NewContactValues>,
) -> Result<()> {
    let _ = (db, workspace_id, entries);
    todo!("P3.6-D2")
}

/// One `listValuesByKeys` entry.
pub use crate::place_near_me::contact_location::ContactFieldValue;

/// `listValuesByKeys`: one value per key (id match first, then name),
/// newest first; bot-field keys are skipped.
pub async fn list_values_by_keys(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    keys: Vec<String>,
) -> Result<Vec<ContactFieldValue>> {
    let _ = (db, workspace_id, contact_id, keys);
    todo!("P3.6-D2")
}
