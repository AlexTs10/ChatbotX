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
