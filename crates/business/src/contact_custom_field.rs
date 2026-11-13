//! `packages/business/src/contact-custom-field` (`normalize.ts`,
//! `service.ts`): normalization, value writes, field operations, deletes
//! (P3.6-D2 contacts core + slice contact custom-field values). The changes
//! are returned (the original emits `customFieldChanged`; no event bus, no
//! read cache).

use chatbotx_core::utils::custom_field::{canonical_number_literal, coerce_boolean_literal};
use chatbotx_core::utils::datetime::{
    DEFAULT_FILTER_TIMEZONE, has_explicit_offset, resolve_filter_timezone,
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

/// `normalizeCustomFieldValueForStorage` (strict, contact-then-workspace
/// zone): `None` = an unparseable temporal value (skipped).
pub fn normalize_value_for_storage(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    kind: &str,
    value: &str,
    explicit_timezone: Option<&str>,
    now_ms: i64,
) -> Result<Option<String>> {
    let mut resolver = SourceTimezoneResolver::new(
        workspace_id,
        contact_id,
        SourceTimezoneStrategy::ContactThenWorkspace,
        None,
    );
    let mut resolve = || resolver.resolve(conn);
    normalize_custom_field_value_for_storage(
        &NormalizeInput {
            explicit_timezone,
            ..NormalizeInput::new(kind, value)
        },
        &mut resolve,
        now_ms,
    )
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
    let input = SetValuesInput {
        workspace_id,
        contact_id,
        fields: fields.to_vec(),
        source_timezone: source_timezone.map(str::to_string),
        ..SetValuesInput::default()
    };
    Ok(set_values_in_transaction(tx, &input, now_ms)?
        .into_iter()
        .map(|change| CustomFieldChange {
            custom_field_id: change.custom_field_id,
            custom_field_name: change.custom_field_name,
            old_value: change.old_value,
            new_value: change.new_value.unwrap_or_default(),
        })
        .collect())
}

// ---- P3.6-D2 slice contact custom-field values ---------------------------

use chatbotx_core::flow_config::field_reference::{FieldReference, parse_field_reference};
use chatbotx_core::utils::custom_field::{CustomFieldType, canonical_boolean_literal};
pub use chatbotx_core::utils::custom_field_operation::FieldOperationType;
pub use chatbotx_core::utils::datetime::{SourceTimezoneStrategy, TemporalInputParsing};
use chatbotx_core::utils::datetime::{
    TemporalCustomFieldType as Temporal, current_temporal_literal,
};
use chatbotx_core::utils::field_value_calculator::date_expression::DateClock;
use chatbotx_core::utils::field_value_calculator::{
    FieldValueCalculationInput, FieldValueCalculationResult, calculate_field_value,
};
use chatbotx_db::Db;
use chatbotx_db::repo::contact::ContactCustomFieldWithDefinition;
use chatbotx_db::repo::contact_custom_field as repo;
use chatbotx_db::repo::now_ms;
use serde_json::json;

use crate::tx;

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

/// `RUNTIME_COERCE_HANDLERS` (an unknown type passes through).
fn runtime_coerce(kind: &str, value: &str) -> Result<String> {
    match kind {
        "boolean" if value.trim().is_empty() => Ok(String::new()),
        "boolean" => Ok(coerce_boolean_literal(value).to_string()),
        "number" if value.trim().is_empty() => Ok(String::new()),
        "number" => canonical_number_literal(value).ok_or_else(|| {
            BusinessError::exception(
                format!(
                    "\"{}\" is not a valid number value for this field.",
                    preview(value)
                ),
                "invalidCustomFieldValue",
                400,
            )
        }),
        _ => Ok(value.to_string()),
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
    let Some(temporal) = Temporal::parse(input.kind) else {
        return runtime_coerce(input.kind, input.value).map(Some);
    };
    let is_empty = input.value.is_empty();
    if is_empty && !input.fill_empty_temporal_with_now {
        return Ok(Some(String::new()));
    }
    // `resolveTemporalSourceTimezone`.
    let needs_zone = match temporal {
        Temporal::Date => true,
        Temporal::Datetime => !has_explicit_offset(input.value),
    };
    let explicit = (temporal == Temporal::Date)
        .then(|| normalize_stored_timezone(input.explicit_timezone))
        .flatten();
    let zone = if !needs_zone {
        DEFAULT_FILTER_TIMEZONE.to_string()
    } else if let Some(explicit) = explicit {
        resolve_filter_timezone(Some(&explicit))
    } else {
        resolve_source_timezone()?
    };
    let literal;
    let (value, parsing) = if is_empty {
        literal = current_temporal_literal(temporal, Some(&zone), now_ms);
        (literal.as_str(), TemporalInputParsing::Strict)
    } else {
        (input.value, input.parsing)
    };
    Ok(normalize_temporal_value_for_storage(
        &TemporalStorageNormalizationInput {
            kind: temporal,
            value,
            timezone: Some(&zone),
            parsing: Some(parsing),
        },
        now_ms,
    ))
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
        SourceTimezoneResolver {
            workspace_id,
            contact_id,
            strategy,
            zone: normalize_stored_timezone(explicit_source_timezone)
                .map(|zone| resolve_filter_timezone(Some(&zone))),
        }
    }

    /// The zone (looked up on the first call only).
    pub fn resolve(&mut self, conn: &Connection) -> Result<String> {
        if let Some(zone) = &self.zone {
            return Ok(zone.clone());
        }
        let zone = match self.strategy {
            SourceTimezoneStrategy::ContactThenWorkspace => {
                source_timezone(conn, self.workspace_id, self.contact_id)?
            }
            SourceTimezoneStrategy::Workspace => {
                let workspace: Option<String> = conn
                    .query_row(
                        r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                        [self.workspace_id],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(repo_err)?
                    .flatten();
                resolve_filter_timezone(normalize_stored_timezone(workspace.as_deref()).as_deref())
            }
        };
        self.zone = Some(zone.clone());
        Ok(zone)
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

fn field_definition(
    conn: &Connection,
    workspace_id: i64,
    custom_field_id: i64,
) -> Result<Option<(String, String)>> {
    conn.query_row(
        r#"SELECT "name", "type" FROM "CustomField" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
        params![workspace_id, custom_field_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(repo_err)
}

fn stored_value(
    conn: &Connection,
    contact_id: i64,
    custom_field_id: i64,
) -> Result<Option<(i64, String)>> {
    conn.query_row(
        r#"SELECT "id", "value" FROM "ContactCustomField" WHERE "contactId" = ?1 AND "customFieldId" = ?2"#,
        params![contact_id, custom_field_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(repo_err)
}

/// `setValuesInTransaction` (`writeValues`): the stored changes, in input
/// order (unknown fields, unparseable temporal values and unchanged values
/// are skipped).
pub fn set_values_in_transaction(
    tx: &Connection,
    input: &SetValuesInput,
    now_ms: i64,
) -> Result<Vec<FieldChange>> {
    let (workspace_id, contact_id) = (input.workspace_id, input.contact_id);
    let parsing = input
        .temporal_input_parsing
        .unwrap_or(TemporalInputParsing::Strict);
    let mut resolver = SourceTimezoneResolver::new(
        workspace_id,
        contact_id,
        input
            .source_timezone_strategy
            .unwrap_or(SourceTimezoneStrategy::ContactThenWorkspace),
        input.source_timezone_override.as_deref(),
    );
    let mut changes = Vec::new();
    for (custom_field_id, value) in &input.fields {
        let Some((name, kind)) = field_definition(tx, workspace_id, *custom_field_id)? else {
            continue;
        };
        let mut resolve = || resolver.resolve(tx);
        let normalized = normalize_custom_field_value_for_storage(
            &NormalizeInput {
                kind: &kind,
                value,
                explicit_timezone: input.source_timezone.as_deref(),
                parsing,
                fill_empty_temporal_with_now: input.fill_empty_temporal_with_now,
            },
            &mut resolve,
            now_ms,
        )?;
        let Some(normalized) = normalized else {
            if parsing == TemporalInputParsing::Lenient {
                tracing::warn!(
                    workspace_id,
                    contact_id,
                    custom_field_id,
                    r#type = %kind,
                    "Skipped unparseable temporal custom-field value from a lenient source"
                );
            }
            continue;
        };
        let existing = stored_value(tx, contact_id, *custom_field_id)?;
        match &existing {
            Some((id, old)) if *old == normalized => {
                if input.touch_unchanged {
                    tx.execute(
                        r#"UPDATE "ContactCustomField" SET "updatedAt" = ?2 WHERE "id" = ?1"#,
                        params![id, now_ms],
                    )
                    .map_err(repo_err)?;
                }
                continue;
            }
            Some((id, _)) => {
                tx.execute(
                    r#"UPDATE "ContactCustomField" SET "value" = ?2, "updatedAt" = ?3 WHERE "id" = ?1"#,
                    params![id, normalized, now_ms],
                )
                .map_err(repo_err)?;
            }
            None => {
                tx.execute(
                    r#"INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value", "createdAt", "updatedAt")
                       VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                       ON CONFLICT ("contactId", "customFieldId") DO UPDATE SET "value" = excluded."value", "updatedAt" = excluded."updatedAt""#,
                    params![
                        crate::contact::create::new_id(),
                        contact_id,
                        custom_field_id,
                        normalized,
                        now_ms
                    ],
                )
                .map_err(repo_err)?;
            }
        }
        changes.push(FieldChange {
            contact_id,
            custom_field_id: *custom_field_id,
            custom_field_name: name,
            old_value: existing.map(|(_, old)| old),
            new_value: Some(normalized),
        });
    }
    Ok(changes)
}

/// `setValues`: one write transaction; the stored changes.
pub async fn set_values(db: &Db, input: SetValuesInput) -> Result<Vec<FieldChange>> {
    tx::write(db, move |tx| {
        set_values_in_transaction(tx, &input, now_ms())
    })
    .await
}

fn contact_in_workspace(conn: &Connection, workspace_id: i64, contact_id: i64) -> Result<()> {
    conn.query_row(
        r#"SELECT 1 FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![contact_id, workspace_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Contact not found"))
}

fn field_not_found() -> BusinessError {
    BusinessError::not_found("Custom field not found")
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
    let input = SetValuesInput {
        workspace_id,
        contact_id,
        fields: vec![(custom_field_id, value.to_string())],
        source_timezone: source_timezone.map(str::to_string),
        ..SetValuesInput::default()
    };
    tx::write(db, move |tx| {
        contact_in_workspace(tx, workspace_id, contact_id)?;
        if field_definition(tx, workspace_id, custom_field_id)?.is_none() {
            return Err(field_not_found());
        }
        set_values_in_transaction(tx, &input, now_ms())
    })
    .await
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

/// `invalidFieldOperationException(reason)`.
fn invalid_field_operation(reason: &str) -> BusinessError {
    BusinessError::Exception {
        message: format!("This operation cannot be applied to the field ({reason})."),
        code: "invalidFieldOperation".to_string(),
        status: 400,
        field: None,
        data: Some(json!({ "reason": reason })),
    }
}

fn apply_in_tx(
    tx: &Connection,
    input: &ApplyFieldOperationsInput,
    now: i64,
) -> Result<Vec<FieldChange>> {
    let workspace_id = input.workspace_id;
    // `contactService.findManyByIds`: contacts outside the workspace are dropped.
    let mut accessible: Vec<i64> = Vec::new();
    for request in &input.operations {
        if !accessible.contains(&request.contact_id)
            && contact_in_workspace(tx, workspace_id, request.contact_id).is_ok()
        {
            accessible.push(request.contact_id);
        }
    }
    let operations: Vec<&FieldOperationRequest> = input
        .operations
        .iter()
        .filter(|request| accessible.contains(&request.contact_id))
        .collect();
    if operations.is_empty() {
        return Ok(Vec::new());
    }
    let mut fields: Vec<(i64, String, String)> = Vec::new();
    for request in &operations {
        if fields.iter().any(|(id, ..)| *id == request.custom_field_id) {
            continue;
        }
        let (name, kind) = field_definition(tx, workspace_id, request.custom_field_id)?
            .ok_or_else(field_not_found)?;
        fields.push((request.custom_field_id, name, kind));
    }
    let pairs: Vec<(i64, i64)> = operations
        .iter()
        .map(|r| (r.contact_id, r.custom_field_id))
        .collect();
    repo::lock_pairs(tx, &pairs)?;
    let field_ids: Vec<i64> = fields.iter().map(|(id, ..)| *id).collect();
    let mut current: std::collections::HashMap<(i64, i64), String> =
        repo::list_values_for_pairs(tx, &accessible, &field_ids)?
            .into_iter()
            .map(|(contact, field, value)| ((contact, field), value))
            .collect();
    let strict = accessible.len() == 1;
    let explicit = input
        .source_timezone_override
        .as_deref()
        .or(input.source_timezone.as_deref());
    let mut resolvers: std::collections::HashMap<i64, SourceTimezoneResolver> =
        std::collections::HashMap::new();
    let mut changes = Vec::new();
    for request in operations {
        let (_, name, kind) = fields
            .iter()
            .find(|(id, ..)| *id == request.custom_field_id)
            .ok_or_else(field_not_found)?;
        let key = (request.contact_id, request.custom_field_id);
        let field_type = CustomFieldType::parse(kind).unwrap_or(CustomFieldType::ShortText);
        let time_zone = if Temporal::parse(kind).is_some() {
            resolvers
                .entry(request.contact_id)
                .or_insert_with(|| {
                    SourceTimezoneResolver::new(
                        workspace_id,
                        request.contact_id,
                        SourceTimezoneStrategy::ContactThenWorkspace,
                        explicit,
                    )
                })
                .resolve(tx)?
        } else {
            DEFAULT_FILTER_TIMEZONE.to_string()
        };
        let result = calculate_field_value(&FieldValueCalculationInput {
            field_type,
            operation: request.operation,
            current_value: current.get(&key).map(String::as_str),
            operand: &request.operand,
            clock: DateClock {
                now_ms: now,
                time_zone: time_zone.clone(),
            },
        });
        let applied = match result {
            FieldValueCalculationResult::Set(value) => {
                let plain = request.operation == FieldOperationType::Set;
                let set = SetValuesInput {
                    workspace_id,
                    contact_id: request.contact_id,
                    fields: vec![(request.custom_field_id, value)],
                    source_timezone: input.source_timezone.clone(),
                    source_timezone_override: if plain {
                        input.source_timezone_override.clone()
                    } else {
                        Some(time_zone.clone())
                    },
                    source_timezone_strategy: None,
                    temporal_input_parsing: if plain {
                        input.temporal_input_parsing
                    } else {
                        Some(TemporalInputParsing::Strict)
                    },
                    fill_empty_temporal_with_now: plain && input.fill_empty_temporal_with_now,
                    touch_unchanged: false,
                };
                set_values_in_transaction(tx, &set, now)?
            }
            FieldValueCalculationResult::Clear => {
                match repo::delete_value(tx, request.contact_id, request.custom_field_id)? {
                    Some(old) => vec![FieldChange {
                        contact_id: request.contact_id,
                        custom_field_id: request.custom_field_id,
                        custom_field_name: name.clone(),
                        old_value: Some(old),
                        new_value: None,
                    }],
                    None => Vec::new(),
                }
            }
            FieldValueCalculationResult::Unchanged => Vec::new(),
            FieldValueCalculationResult::Rejected(reason) => {
                if strict {
                    return Err(invalid_field_operation(reason.as_str()));
                }
                tracing::warn!(
                    workspace_id,
                    contact_id = request.contact_id,
                    custom_field_id = request.custom_field_id,
                    reason = reason.as_str(),
                    "Skipped a rejected custom-field operation"
                );
                Vec::new()
            }
        };
        for change in &applied {
            match &change.new_value {
                Some(value) => current.insert(key, value.clone()),
                None => current.remove(&key),
            };
        }
        changes.extend(applied);
    }
    Ok(changes)
}

/// `applyFieldOperations`: one transaction; strict (throws
/// `invalidFieldOperation`) when it targets one contact, else a rejected
/// operation is skipped. Contacts outside the workspace are dropped.
pub async fn apply_field_operations(
    db: &Db,
    input: ApplyFieldOperationsInput,
) -> Result<Vec<FieldChange>> {
    tx::write(db, move |tx| apply_in_tx(tx, &input, now_ms())).await
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
    let operations = contact_ids
        .iter()
        .map(|contact_id| FieldOperationRequest {
            contact_id: *contact_id,
            custom_field_id,
            operation,
            operand: value.to_string(),
        })
        .collect();
    apply_field_operations(
        db,
        ApplyFieldOperationsInput {
            workspace_id,
            operations,
            source_timezone: source_timezone.map(str::to_string),
            ..ApplyFieldOperationsInput::default()
        },
    )
    .await
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
    let operations = operations
        .into_iter()
        .map(|op| FieldOperationRequest {
            contact_id,
            custom_field_id: op.custom_field_id,
            operation: op.operation,
            operand: op.value,
        })
        .collect();
    apply_field_operations(
        db,
        ApplyFieldOperationsInput {
            workspace_id,
            operations,
            source_timezone: source_timezone.map(str::to_string),
            ..ApplyFieldOperationsInput::default()
        },
    )
    .await
}

/// `listWithDefinitionByContact`.
pub async fn list_with_definition_by_contact(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<ContactCustomFieldWithDefinition>> {
    tx::read(db, move |conn| {
        Ok(repo::list_with_definition_by_contact(
            conn,
            workspace_id,
            contact_id,
        )?)
    })
    .await
}

/// `findWithDefinition`.
pub async fn find_with_definition(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    custom_field_id: i64,
) -> Result<Option<ContactCustomFieldWithDefinition>> {
    tx::read(db, move |conn| {
        Ok(repo::find_with_definition(
            conn,
            workspace_id,
            contact_id,
            custom_field_id,
        )?)
    })
    .await
}

/// `clearByContactId`.
pub async fn clear_by_contact_id(db: &Db, workspace_id: i64, contact_id: i64) -> Result<()> {
    let _ = workspace_id;
    tx::write(db, move |tx| {
        tx.execute(
            r#"DELETE FROM "ContactCustomField" WHERE "contactId" = ?1"#,
            [contact_id],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}

/// `deleteByCustomFieldId` (no change records).
pub async fn delete_by_custom_field_id(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    custom_field_id: i64,
) -> Result<()> {
    let _ = workspace_id;
    if contact_ids.is_empty() {
        return Ok(());
    }
    let ids = serde_json::to_string(contact_ids).unwrap_or_else(|_| "[]".to_string());
    tx::write(db, move |tx| {
        tx.execute(
            r#"DELETE FROM "ContactCustomField" WHERE "customFieldId" = ?1 AND "contactId" IN (SELECT value FROM json_each(?2))"#,
            params![custom_field_id, ids],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
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

/// The `{ id, name }` of a field by numeric id, then by name.
fn field_by_keyword(conn: &Connection, workspace_id: i64, keyword: &str) -> Result<(i64, String)> {
    if chatbotx_core::utils::id::is_numeric_id(keyword)
        && let Ok(id) = keyword.parse::<i64>()
        && let Some((name, _)) = field_definition(conn, workspace_id, id)?
    {
        return Ok((id, name));
    }
    conn.query_row(
        r#"SELECT "id", "name" FROM "CustomField" WHERE "name" = ?1 AND "workspaceId" = ?2 ORDER BY "id" LIMIT 1"#,
        params![keyword, workspace_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(field_not_found)
}

/// `resolveBotFieldKey`.
fn bot_field_key(keyword: &str, allow_bot_fields: bool) -> Option<String> {
    if !allow_bot_fields {
        return None;
    }
    match parse_field_reference(keyword) {
        FieldReference::BotField { id } => Some(id),
        FieldReference::CustomField { .. } => None,
    }
}

/// `setValueByKey`: the stored contact changes (empty for a bot field).
pub async fn set_value_by_key(db: &Db, input: SetValueByKeyInput) -> Result<Vec<FieldChange>> {
    if let Some(key) = bot_field_key(&input.keyword, input.allow_bot_fields) {
        crate::bot_field::apply_value_operation(
            db,
            input.workspace_id,
            crate::bot_field::ValueOperation {
                key,
                operation: input.operation.unwrap_or(FieldOperationType::Set),
                value: input.value,
                source_timezone_override: input.source_timezone_override,
                temporal_input_parsing: input.temporal_input_parsing,
                fill_empty_temporal_with_now: input.fill_empty_temporal_with_now,
            },
        )
        .await?;
        return Ok(Vec::new());
    }
    let (workspace_id, keyword) = (input.workspace_id, input.keyword.clone());
    let (custom_field_id, _) = tx::read(db, move |conn| {
        field_by_keyword(conn, workspace_id, &keyword)
    })
    .await?;
    if let Some(operation) = input.operation.filter(|op| *op != FieldOperationType::Set) {
        return apply_field_operations(
            db,
            ApplyFieldOperationsInput {
                workspace_id,
                operations: vec![FieldOperationRequest {
                    contact_id: input.contact_id,
                    custom_field_id,
                    operation,
                    operand: input.value,
                }],
                source_timezone: None,
                source_timezone_override: input.source_timezone_override,
                temporal_input_parsing: input.temporal_input_parsing,
                fill_empty_temporal_with_now: input.fill_empty_temporal_with_now,
            },
        )
        .await;
    }
    set_values(
        db,
        SetValuesInput {
            workspace_id,
            contact_id: input.contact_id,
            fields: vec![(custom_field_id, input.value)],
            source_timezone: None,
            source_timezone_override: input.source_timezone_override,
            source_timezone_strategy: None,
            temporal_input_parsing: input.temporal_input_parsing,
            fill_empty_temporal_with_now: input.fill_empty_temporal_with_now,
            touch_unchanged: input.touch_unchanged,
        },
    )
    .await
}

/// `deleteByKey`: the deleted value (`None` = no value, or a bot field).
pub async fn delete_by_key(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    keyword: &str,
    allow_bot_fields: bool,
) -> Result<Option<FieldChange>> {
    if let Some(key) = bot_field_key(keyword, allow_bot_fields) {
        crate::bot_field::clear_value_by_key(db, workspace_id, &key).await?;
        return Ok(None);
    }
    let keyword = keyword.to_string();
    tx::write(db, move |tx| {
        let (custom_field_id, name) = field_by_keyword(tx, workspace_id, &keyword)?;
        Ok(
            repo::delete_value(tx, contact_id, custom_field_id)?.map(|old| FieldChange {
                contact_id,
                custom_field_id,
                custom_field_name: name,
                old_value: Some(old),
                new_value: None,
            }),
        )
    })
    .await
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
    // `assertValuesAreCanonical`.
    for entry in entries {
        for (custom_field_id, value) in &entry.fields {
            if value.is_empty() {
                continue;
            }
            let kind = field_definition(tx, workspace_id, *custom_field_id)?.map(|(_, kind)| kind);
            let canonical = match kind.as_deref() {
                Some("boolean") => canonical_boolean_literal(value) == Some(value),
                Some("number") => {
                    canonical_number_literal(value).as_deref() == Some(value.as_str())
                }
                _ => true,
            };
            if !canonical {
                return Err(BusinessError::exception(
                    format!(
                        "Non-canonical {} value for custom field {custom_field_id}; normalize before calling insertNormalizedValuesForNewContacts.",
                        kind.unwrap_or_default()
                    ),
                    "invalidCustomFieldValue",
                    400,
                ));
            }
        }
    }
    let now = now_ms();
    for entry in entries {
        for (custom_field_id, value) in &entry.fields {
            tx.execute(
                r#"INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value", "createdAt", "updatedAt") VALUES (?1, ?2, ?3, ?4, ?5, ?5)"#,
                params![
                    crate::contact::create::new_id(),
                    entry.contact_id,
                    custom_field_id,
                    value,
                    now
                ],
            )
            .map_err(repo_err)?;
        }
    }
    Ok(())
}

/// [`insert_normalized_values_for_new_contacts_in`] in its own transaction.
pub async fn insert_normalized_values_for_new_contacts(
    db: &Db,
    workspace_id: i64,
    entries: Vec<NewContactValues>,
) -> Result<()> {
    tx::write(db, move |tx| {
        insert_normalized_values_for_new_contacts_in(tx, workspace_id, &entries)
    })
    .await
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
    crate::place_near_me::contact_location::list_values_by_keys(
        db,
        crate::place_near_me::contact_location::ContactLocationInput {
            workspace_id,
            contact_id,
            keys,
        },
    )
    .await
}
