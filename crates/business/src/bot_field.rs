//! `packages/business/src/bot-field/service.ts`: workspace-level bot fields
//! (P3.6-D2 slice bot fields). Private queries on `"BotField"` /
//! `"Folder"`. No read cache, no template edit locks / delete guards
//! (P3.6-D1 rule), no advisory name lock (one SQLite writer).
//! `resolveByNameAndType` / `planResolution` (template install) are not
//! ported here.

use chatbotx_core::utils::custom_field::{CustomFieldType, canonical_number_literal};
pub use chatbotx_core::utils::custom_field_operation::FieldOperationType;
use chatbotx_core::utils::custom_field_operation::is_field_operation_allowed;
pub use chatbotx_core::utils::datetime::TemporalInputParsing;
use chatbotx_core::utils::datetime::{
    DEFAULT_FILTER_TIMEZONE, SourceTimezoneStrategy, is_temporal_custom_field_type,
};
use chatbotx_core::utils::field_value_calculator::date_expression::DateClock;
use chatbotx_core::utils::field_value_calculator::{
    FieldValueCalculationInput, FieldValueCalculationResult, calculate_field_value,
};
use chatbotx_db::Db;
use chatbotx_db::repo::ads_conversion_event::decimal_sum;
pub use chatbotx_db::repo::models::BotField;
use chatbotx_db::repo::utils::{order_by_sql, parse_order_by, with_id_tie_break};
use chatbotx_db::repo::{
    Model, PaginationInput, SortItem, get_pagination_with_defaults, is_unique_violation_error,
    like_contains, new_id, now_ms,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};
use serde_json::json;

use crate::contact_custom_field::{
    NormalizeInput, SourceTimezoneResolver, normalize_custom_field_value_for_storage,
};
use crate::{BusinessError, Result, tx};

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Bot field not found")
}

/// `new ChatbotXException(message, "invalidFieldOperation")` (400, no data).
fn invalid_operation(message: String) -> BusinessError {
    BusinessError::exception(message, "invalidFieldOperation", 400)
}

/// A unique `(workspaceId, type, name)` violation is the name check.
fn name_taken(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    let error = chatbotx_db::repo::Error::from(error);
    if is_unique_violation_error(&error, None) {
        BusinessError::validation("name", "Name is already taken")
    } else {
        BusinessError::Repo(error)
    }
}

/// `ListBotFieldsInput` (`folder_id: Some(0)` = the root).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListBotFields {
    pub folder_id: Option<i64>,
    pub name: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// `PaginatedResult<BotFieldModel>`.
#[derive(Debug, Clone, PartialEq)]
pub struct BotFieldPage {
    pub data: Vec<BotField>,
    pub page_count: i64,
}

/// `list`: one page, `name` an `ilike` contains filter.
pub async fn list(db: &Db, workspace_id: i64, input: ListBotFields) -> Result<BotFieldPage> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        // No `sort`: no order in the original (`{}`); rows come back in id order.
        let order_sql = match input.sort.as_deref() {
            None => r#"ORDER BY "id""#.to_string(),
            Some(sort) => order_by_sql(
                &with_id_tie_break(&parse_order_by(BotField::COLUMNS, Some(sort), None)),
                None,
            ),
        };
        let mut filter = String::from(r#""workspaceId" = ?"#);
        let mut binds: Vec<Sql> = vec![Sql::Integer(workspace_id)];
        match input.folder_id {
            Some(0) => filter.push_str(r#" AND "folderId" IS NULL"#),
            Some(folder) => {
                filter.push_str(r#" AND "folderId" = ?"#);
                binds.push(Sql::Integer(folder));
            }
            None => {}
        }
        if let Some(name) = input.name.as_deref().filter(|n| !n.is_empty()) {
            filter.push_str(r#" AND ilike("name", ?)"#);
            binds.push(Sql::Text(like_contains(name)));
        }
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT count(*) FROM "BotField" WHERE {filter}"#),
                params_from_iter(binds.iter()),
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let sql = format!(
            "{} WHERE {filter} {} LIMIT ? OFFSET ?",
            BotField::select_sql(),
            order_sql
        );
        binds.push(Sql::Integer(pagination.limit));
        binds.push(Sql::Integer(pagination.offset));
        let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
        let data = stmt
            .query_map(params_from_iter(binds.iter()), BotField::from_row)
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        let page_count = if pagination.limit > 0 {
            (total + pagination.limit - 1) / pagination.limit
        } else {
            1
        };
        Ok(BotFieldPage { data, page_count })
    })
    .await
}

fn find_by_id_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<BotField>> {
    conn.query_row(
        &format!(
            r#"{} WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            BotField::select_sql()
        ),
        params![id, workspace_id],
        BotField::from_row,
    )
    .optional()
    .map_err(repo_err)
}

/// `findByKey` on a connection: a digits key is the id (beyond `i64` = no
/// row), anything else the name.
pub fn find_by_key_in(conn: &Connection, workspace_id: i64, key: &str) -> Result<Option<BotField>> {
    if !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()) {
        return match key.parse::<i64>() {
            Ok(id) => find_by_id_in(conn, workspace_id, id),
            Err(_) => Ok(None),
        };
    }
    conn.query_row(
        &format!(
            r#"{} WHERE "name" = ?1 AND "workspaceId" = ?2 ORDER BY "id" LIMIT 1"#,
            BotField::select_sql()
        ),
        params![key, workspace_id],
        BotField::from_row,
    )
    .optional()
    .map_err(repo_err)
}

fn find_by_key_or_fail_in(conn: &Connection, workspace_id: i64, key: &str) -> Result<BotField> {
    find_by_key_in(conn, workspace_id, key)?.ok_or_else(not_found)
}

/// `findByKey`: a digits key is the id, anything else the name.
pub async fn find_by_key(db: &Db, workspace_id: i64, key: &str) -> Result<Option<BotField>> {
    let key = key.to_string();
    tx::read(db, move |conn| find_by_key_in(conn, workspace_id, &key)).await
}

/// `findByKeyOrFail`: 404 "Bot field not found".
pub async fn find_by_key_or_fail(db: &Db, workspace_id: i64, key: &str) -> Result<BotField> {
    find_by_key(db, workspace_id, key)
        .await?
        .ok_or_else(not_found)
}

/// `findManyByIds` (workspace-scoped).
pub async fn find_many_by_ids(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<Vec<BotField>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids = serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_string());
    tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                r#"{} WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2)) ORDER BY "id""#,
                BotField::select_sql()
            ))
            .map_err(repo_err)?;
        let rows = stmt
            .query_map(params![workspace_id, ids], BotField::from_row)
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(rows)
    })
    .await
}

/// `folderService.ensureExists` (a `customField` folder of the workspace).
fn ensure_folder(conn: &Connection, workspace_id: i64, folder_id: i64) -> Result<()> {
    conn.query_row(
        r#"SELECT 1 FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "folderType" = 'customField'"#,
        params![folder_id, workspace_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Folder not found"))
}

/// `normalizeCustomFieldValueForStorage` against the workspace zone (or an
/// explicit override).
fn normalize(
    conn: &Connection,
    workspace_id: i64,
    input: &NormalizeInput<'_>,
    source_timezone_override: Option<&str>,
    now: i64,
) -> Result<Option<String>> {
    let mut resolver = SourceTimezoneResolver::new(
        workspace_id,
        0,
        SourceTimezoneStrategy::Workspace,
        source_timezone_override,
    );
    let mut resolve = || resolver.resolve(conn);
    normalize_custom_field_value_for_storage(input, &mut resolve, now)
}

/// `prepareValuePatch`: a value is normalized against `kind` (422-free: an
/// unparseable temporal value is 400 `invalidFieldOperation`).
fn prepare_value(conn: &Connection, workspace_id: i64, kind: &str, value: &str) -> Result<String> {
    normalize(
        conn,
        workspace_id,
        &NormalizeInput::new(kind, value),
        None,
        now_ms(),
    )?
    .ok_or_else(|| invalid_operation(format!("Invalid {kind} value for bot field")))
}

/// `CreateBotFieldData` (`folder_id: Some(0)` = the root).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateBotField {
    pub name: String,
    pub r#type: String,
    pub value: Option<String>,
    pub description: Option<String>,
    pub folder_id: Option<i64>,
}

/// `create`: the value is normalized against the workspace zone; a taken
/// `(type, name)` is 422 `name` "Name is already taken".
pub async fn create(db: &Db, workspace_id: i64, data: CreateBotField) -> Result<BotField> {
    tx::write(db, move |conn| {
        let folder_id = data.folder_id.filter(|id| *id != 0);
        if let Some(folder) = folder_id {
            ensure_folder(conn, workspace_id, folder)?;
        }
        let value = match data.value.as_deref() {
            Some(value) => Some(prepare_value(conn, workspace_id, &data.r#type, value)?),
            None => None,
        };
        let id = new_id();
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "BotField" ("id", "createdAt", "updatedAt", "name", "type", "value", "description", "folderId", "workspaceId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![id, now, data.name, data.r#type, value, data.description, folder_id, workspace_id],
        )
        .map_err(name_taken)?;
        find_by_id_in(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `UpdateBotFieldData`: `None` = keep; `Some(None)` = set `null`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateBotField {
    pub name: Option<String>,
    pub r#type: Option<String>,
    pub value: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub folder_id: Option<Option<i64>>,
}

impl UpdateBotField {
    /// No key given (the PATCH "nothing to change" case).
    pub fn is_empty(&self) -> bool {
        self == &UpdateBotField::default()
    }
}

/// `writeUpdate`: the patched row.
fn write_update(
    conn: &Connection,
    workspace_id: i64,
    existing: &BotField,
    data: &UpdateBotField,
) -> Result<BotField> {
    let mut sets: Vec<String> = Vec::new();
    let mut binds: Vec<Sql> = Vec::new();
    let mut push = |column: &str, value: Sql| {
        binds.push(value);
        sets.push(format!(r#""{column}" = ?{}"#, binds.len()));
    };
    if let Some(name) = &data.name {
        push("name", Sql::Text(name.clone()));
    }
    if let Some(kind) = &data.r#type {
        push("type", Sql::Text(kind.clone()));
    }
    if let Some(value) = &data.value {
        push("value", value.clone().map_or(Sql::Null, Sql::Text));
    }
    if let Some(description) = &data.description {
        push(
            "description",
            description.clone().map_or(Sql::Null, Sql::Text),
        );
    }
    if let Some(folder) = &data.folder_id {
        let folder = folder.filter(|id| *id != 0);
        if let Some(id) = folder
            && Some(id) != existing.folder_id
        {
            ensure_folder(conn, workspace_id, id)?;
        }
        push("folderId", folder.map_or(Sql::Null, Sql::Integer));
    }
    push("updatedAt", Sql::Integer(now_ms()));
    binds.push(Sql::Integer(existing.id));
    let sql = format!(
        r#"UPDATE "BotField" SET {} WHERE "id" = ?{}"#,
        sets.join(", "),
        binds.len()
    );
    conn.execute(&sql, params_from_iter(binds.iter()))
        .map_err(name_taken)?;
    find_by_id_in(conn, workspace_id, existing.id)?.ok_or_else(not_found)
}

fn update_by_key_in(
    conn: &Connection,
    workspace_id: i64,
    key: &str,
    mut data: UpdateBotField,
) -> Result<BotField> {
    let existing = find_by_key_or_fail_in(conn, workspace_id, key)?;
    if let Some(Some(value)) = &data.value {
        let kind = data
            .r#type
            .clone()
            .unwrap_or_else(|| existing.r#type.clone());
        data.value = Some(Some(prepare_value(conn, workspace_id, &kind, value)?));
    }
    write_update(conn, workspace_id, &existing, &data)
}

/// `updateByKey`: the value is normalized against the new (or stored) type.
pub async fn update_by_key(
    db: &Db,
    workspace_id: i64,
    key: &str,
    data: UpdateBotField,
) -> Result<BotField> {
    let key = key.to_string();
    tx::write(db, move |conn| {
        update_by_key_in(conn, workspace_id, &key, data)
    })
    .await
}

/// The `applyValueOperation` input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueOperation {
    pub key: String,
    pub operation: FieldOperationType,
    pub value: String,
    pub source_timezone_override: Option<String>,
    pub temporal_input_parsing: Option<TemporalInputParsing>,
    pub fill_empty_temporal_with_now: bool,
}

impl ValueOperation {
    /// No zone override, strict parsing, no "now" fill.
    pub fn new(key: &str, operation: FieldOperationType, value: &str) -> ValueOperation {
        ValueOperation {
            key: key.to_string(),
            operation,
            value: value.to_string(),
            source_timezone_override: None,
            temporal_input_parsing: None,
            fill_empty_temporal_with_now: false,
        }
    }
}

fn set_value(conn: &Connection, existing: &BotField, value: Option<String>) -> Result<BotField> {
    write_update(
        conn,
        existing.workspace_id,
        existing,
        &UpdateBotField {
            value: Some(value),
            ..UpdateBotField::default()
        },
    )
}

/// `ATOMIC_VALUE_EXPRESSIONS`: append / prepend concatenate; increase /
/// decrease are exact decimal sums (Postgres `numeric`).
fn atomic(
    conn: &Connection,
    existing: &BotField,
    operation: FieldOperationType,
    value: &str,
) -> Result<BotField> {
    let current = existing.value.clone().unwrap_or_default();
    let next = match operation {
        FieldOperationType::Append => format!("{current}{value}"),
        FieldOperationType::Prepend => format!("{value}{current}"),
        _ => {
            let operand = canonical_number_literal(value).ok_or_else(|| {
                invalid_operation(format!(
                    "\"{value}\" is not a valid number value for this field."
                ))
            })?;
            let operand = if operation == FieldOperationType::Decrease {
                operand
                    .strip_prefix('-')
                    .map_or_else(|| format!("-{operand}"), str::to_string)
            } else {
                operand
            };
            let base = if current.is_empty() {
                "0".to_string()
            } else {
                current.trim().to_string()
            };
            let not_numeric = || {
                invalid_operation(
                    "Bot field value is not numeric and cannot be increased or decreased"
                        .to_string(),
                )
            };
            if decimal_sum(std::iter::once(base.as_str())).is_none() {
                return Err(not_numeric());
            }
            decimal_sum([base.as_str(), operand.as_str()].into_iter()).ok_or_else(not_numeric)?
        }
    };
    set_value(conn, existing, Some(next))
}

/// `applyCalculatedOperation`.
fn calculated(conn: &Connection, existing: &BotField, input: &ValueOperation) -> Result<BotField> {
    let workspace_id = existing.workspace_id;
    let mut resolver = SourceTimezoneResolver::new(
        workspace_id,
        0,
        SourceTimezoneStrategy::Workspace,
        input.source_timezone_override.as_deref(),
    );
    let time_zone = if is_temporal_custom_field_type(&existing.r#type) {
        resolver.resolve(conn)?
    } else {
        DEFAULT_FILTER_TIMEZONE.to_string()
    };
    let now = now_ms();
    let result = calculate_field_value(&FieldValueCalculationInput {
        field_type: CustomFieldType::parse(&existing.r#type).unwrap_or(CustomFieldType::ShortText),
        operation: input.operation,
        current_value: existing.value.as_deref(),
        operand: &input.value,
        clock: DateClock {
            now_ms: now,
            time_zone: time_zone.clone(),
        },
    });
    let value = match result {
        FieldValueCalculationResult::Rejected(reason) => {
            return Err(BusinessError::Exception {
                message: format!(
                    "This operation cannot be applied to the field ({}).",
                    reason.as_str()
                ),
                code: "invalidFieldOperation".to_string(),
                status: 400,
                field: None,
                data: Some(json!({ "reason": reason.as_str() })),
            });
        }
        FieldValueCalculationResult::Unchanged => return Ok(existing.clone()),
        FieldValueCalculationResult::Clear => None,
        FieldValueCalculationResult::Set(value) => {
            let mut resolve = || resolver.resolve(conn);
            let normalized = normalize_custom_field_value_for_storage(
                &NormalizeInput {
                    explicit_timezone: Some(&time_zone),
                    ..NormalizeInput::new(&existing.r#type, &value)
                },
                &mut resolve,
                now,
            )?;
            Some(normalized.ok_or_else(|| {
                invalid_operation(format!("Invalid {} value for bot field", existing.r#type))
            })?)
        }
    };
    set_value(conn, existing, value)
}

fn apply_in(conn: &Connection, workspace_id: i64, input: &ValueOperation) -> Result<BotField> {
    let existing = find_by_key_or_fail_in(conn, workspace_id, &input.key)?;
    let kind = CustomFieldType::parse(&existing.r#type).unwrap_or(CustomFieldType::ShortText);
    if !is_field_operation_allowed(input.operation, kind) {
        return Err(invalid_operation(format!(
            "Operation \"{}\" is not supported for field type \"{}\"",
            input.operation, existing.r#type
        )));
    }
    match input.operation {
        FieldOperationType::Set => {
            let normalized = normalize(
                conn,
                workspace_id,
                &NormalizeInput {
                    explicit_timezone: input.source_timezone_override.as_deref(),
                    parsing: input
                        .temporal_input_parsing
                        .unwrap_or(TemporalInputParsing::Strict),
                    fill_empty_temporal_with_now: input.fill_empty_temporal_with_now,
                    ..NormalizeInput::new(&existing.r#type, &input.value)
                },
                input.source_timezone_override.as_deref(),
                now_ms(),
            )?
            .ok_or_else(|| {
                invalid_operation(format!("Invalid {} value for bot field", existing.r#type))
            })?;
            set_value(conn, &existing, Some(normalized))
        }
        FieldOperationType::Append
        | FieldOperationType::Prepend
        | FieldOperationType::Increase
        | FieldOperationType::Decrease => atomic(conn, &existing, input.operation, &input.value),
        _ => calculated(conn, &existing, input),
    }
}

/// `applyValueOperation`: `set`, the atomic text / number operations, or a
/// calculated operation; the updated row.
pub async fn apply_value_operation(
    db: &Db,
    workspace_id: i64,
    input: ValueOperation,
) -> Result<BotField> {
    tx::write(db, move |conn| apply_in(conn, workspace_id, &input)).await
}

/// `applyValueOperation` inside a caller's transaction.
pub fn apply_value_operation_in(
    conn: &Connection,
    workspace_id: i64,
    input: &ValueOperation,
) -> Result<BotField> {
    apply_in(conn, workspace_id, input)
}

/// `clearValueByKey` inside a caller's transaction.
pub fn clear_value_by_key_in(conn: &Connection, workspace_id: i64, key: &str) -> Result<BotField> {
    let existing = find_by_key_or_fail_in(conn, workspace_id, key)?;
    set_value(conn, &existing, None)
}

/// `clearValueByKey`: the value becomes `null` (the row stays).
pub async fn clear_value_by_key(db: &Db, workspace_id: i64, key: &str) -> Result<BotField> {
    let key = key.to_string();
    tx::write(db, move |conn| {
        clear_value_by_key_in(conn, workspace_id, &key)
    })
    .await
}

fn id_list(ids: &[i64]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_string())
}

/// `bulkClearValues`.
pub async fn bulk_clear_values(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let ids = id_list(ids);
    tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "BotField" SET "value" = NULL, "updatedAt" = ?3 WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
            params![workspace_id, ids, now_ms()],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}

/// `bulkUpdateByKeys`: `(key, value)` through [`update_by_key`] (one
/// transaction; the original runs them in parallel without one).
pub async fn bulk_update_by_keys(
    db: &Db,
    workspace_id: i64,
    updates: Vec<(String, String)>,
) -> Result<()> {
    tx::write(db, move |conn| {
        for (key, value) in &updates {
            update_by_key_in(
                conn,
                workspace_id,
                key,
                UpdateBotField {
                    value: Some(Some(value.clone())),
                    ..UpdateBotField::default()
                },
            )?;
        }
        Ok(())
    })
    .await
}

/// `deleteByKey`.
pub async fn delete_by_key(db: &Db, workspace_id: i64, key: &str) -> Result<()> {
    let field = find_by_key_or_fail(db, workspace_id, key).await?;
    bulk_delete(db, workspace_id, &[field.id]).await
}

/// `bulkDelete` (workspace-scoped).
pub async fn bulk_delete(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<()> {
    let ids = id_list(ids);
    tx::write(db, move |conn| {
        conn.execute(
            r#"DELETE FROM "BotField" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
            params![workspace_id, ids],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}
