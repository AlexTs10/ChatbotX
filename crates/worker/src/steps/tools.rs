//! The tool steps of `tool-handler.ts`: `countCharacters`, `formatDate`,
//! `generateCode`, `getDataFromJSON`. A field slot holds a custom field id
//! or a `bot_field:<id>` Account Field reference (`parseFieldReference`).
//! `callApi` is `external_request.rs`.

use chatbotx_business::contact_custom_field::{
    self, SetValueByKeyInput, SetValuesInput, SourceTimezoneResolver,
};
use chatbotx_core::flow_config::field_reference::{FieldReference, parse_field_reference};
use chatbotx_core::utils::datetime::{SourceTimezoneStrategy, is_temporal_custom_field_type};
use chatbotx_core::utils::id::math_random;
use chatbotx_core::utils::json_path::{read_json_path, stringify_json_path_value};
use chatbotx_core::utils::tz::{DateArg, format_in_time_zone};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_db::{Db, DbError};
use chatbotx_jobs::JobError;
use serde_json::Value;

use super::contact::biz_fail;
use super::{StepCtx, step_error};
use crate::db_fail;
use crate::flow::StepStatus;

/// A parsed field slot: `Custom(None)` is a custom field id that is not a
/// number (no row can match it).
#[derive(Clone, Copy)]
enum Field {
    Custom(Option<i64>),
    Bot(i64),
}

fn field(raw: &str) -> Field {
    match parse_field_reference(raw) {
        FieldReference::BotField { id } => id.parse().map_or(Field::Custom(None), Field::Bot),
        FieldReference::CustomField { key } => Field::Custom(key.parse().ok()),
    }
}

/// `resolveReferencedFieldType` (`None` = the field does not exist).
fn field_type(conn: &Connection, ws: i64, field: Field) -> Result<Option<String>, DbError> {
    let (sql, id) = match field {
        Field::Bot(id) => (
            r#"SELECT "type" FROM "BotField" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            id,
        ),
        Field::Custom(Some(id)) => (
            r#"SELECT "type" FROM "CustomField" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            id,
        ),
        Field::Custom(None) => return Ok(None),
    };
    Ok(conn
        .query_row(sql, params![id, ws], |r| r.get(0))
        .optional()?)
}

/// `readReferencedFieldValue` (`None` = no field or no stored value).
fn read_value(
    conn: &Connection,
    ws: i64,
    contact_id: i64,
    field: Field,
) -> Result<Option<String>, DbError> {
    let value: Option<Option<String>> = match field {
        Field::Bot(id) => conn
            .query_row(
                r#"SELECT "value" FROM "BotField" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![id, ws],
                |r| r.get(0),
            )
            .optional()?,
        Field::Custom(Some(id)) => conn
            .query_row(
                r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = ?1 AND "customFieldId" = ?2"#,
                params![contact_id, id],
                |r| r.get(0),
            )
            .optional()?,
        Field::Custom(None) => None,
    };
    Ok(value.flatten())
}

async fn read(db: &Db, ws: i64, contact_id: i64, raw: &str) -> Result<Option<String>, JobError> {
    let field = field(raw);
    db.read(move |conn| read_value(conn, ws, contact_id, field))
        .await
        .map_err(db_fail)
}

/// `writeReferencedFieldValue`: a custom field through `setValues`, an
/// Account Field through `setValueByKey` (a failure is logged and swallowed).
async fn write(ctx: &StepCtx<'_, '_>, raw: &str, value: String) -> Result<(), JobError> {
    let db = &ctx.run.deps.db;
    let (workspace_id, contact_id) = (ctx.workspace_id(), ctx.contact_id());
    match field(raw) {
        Field::Bot(_) => {
            let input = SetValueByKeyInput {
                workspace_id,
                contact_id,
                keyword: raw.to_owned(),
                value,
                allow_bot_fields: true,
                ..SetValueByKeyInput::default()
            };
            if let Err(err) = contact_custom_field::set_value_by_key(db, input).await {
                tracing::error!(err = %err.message(), workspace_id, contact_id, field_id = raw, "Failed to write Account Field value from a tool step; continuing");
            }
        }
        Field::Custom(Some(id)) => {
            set_custom_values(ctx, vec![(id, value)]).await?;
        }
        // `setValues` skips an unknown field.
        Field::Custom(None) => {}
    }
    Ok(())
}

async fn set_custom_values(
    ctx: &StepCtx<'_, '_>,
    fields: Vec<(i64, String)>,
) -> Result<(), JobError> {
    contact_custom_field::set_values(
        &ctx.run.deps.db,
        SetValuesInput {
            workspace_id: ctx.workspace_id(),
            contact_id: ctx.contact_id(),
            fields,
            ..SetValuesInput::default()
        },
    )
    .await
    .map(drop)
    .map_err(biz_fail)
}

/// The tool steps; `None` = not a tool step.
pub(super) async fn run(
    ctx: &StepCtx<'_, '_>,
    step_type: &str,
) -> Result<Option<StepStatus>, JobError> {
    Ok(Some(match step_type {
        "countCharacters" => count_characters(ctx).await?,
        "formatDate" => format_date(ctx).await?,
        "generateCode" => generate_code(ctx).await?,
        "getDataFromJson" => get_data_from_json(ctx).await?,
        _ => return Ok(None),
    }))
}

async fn count_characters(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let (ws, contact_id) = (ctx.workspace_id(), ctx.contact_id());
    let (input, output) = (
        ctx.str("inputFieldId").unwrap_or("").to_owned(),
        ctx.str("outputFieldId").unwrap_or("").to_owned(),
    );
    let (input_field, output_field) = (field(&input), field(&output));
    let value = ctx
        .run
        .deps
        .db
        .read(move |conn| {
            if field_type(conn, ws, input_field)?.is_none()
                || field_type(conn, ws, output_field)?.is_none()
            {
                return Ok(None);
            }
            read_value(conn, ws, contact_id, input_field)
        })
        .await
        .map_err(db_fail)?;
    if let Some(value) = value {
        // JavaScript `String.length` (UTF-16 code units).
        write(ctx, &output, value.encode_utf16().count().to_string()).await?;
    }
    Ok(StepStatus::Success)
}

async fn format_date(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let (ws, contact_id) = (ctx.workspace_id(), ctx.contact_id());
    let db = &ctx.run.deps.db;
    let Some(input) = read(db, ws, contact_id, ctx.str("inputFieldId").unwrap_or("")).await? else {
        return Ok(StepStatus::Success);
    };
    let output = ctx.str("outputFieldId").unwrap_or("").to_owned();
    let output_field = field(&output);
    let strategy = match ctx.str("timezone") {
        Some("workspace") => SourceTimezoneStrategy::Workspace,
        _ => SourceTimezoneStrategy::ContactThenWorkspace,
    };
    let zone = db
        .read(move |conn| {
            // A display string in a temporal field would be re-parsed and
            // corrupt the stored value.
            match field_type(conn, ws, output_field)? {
                Some(kind) if !is_temporal_custom_field_type(&kind) => {}
                _ => return Ok(None),
            }
            SourceTimezoneResolver::new(ws, contact_id, strategy, None)
                .resolve(conn)
                .map(Some)
                .map_err(|e| DbError::other(e.message()))
        })
        .await
        .map_err(db_fail)?;
    let Some(zone) = zone else {
        return Ok(StepStatus::Success);
    };
    // date-fns-tz throws a `RangeError` on an invalid date: the job fails.
    let formatted = format_in_time_zone(
        DateArg::Text(&input),
        &zone,
        ctx.str("format").unwrap_or(""),
    )
    .map_err(JobError::retryable)?;
    write(ctx, &output, formatted).await?;
    Ok(StepStatus::Success)
}

/// `faker.number.int({ min, max })`: an integer in `[ceil(min), floor(max)]`.
fn random_int(min: f64, max: f64) -> Result<i64, JobError> {
    let (lo, hi) = (min.ceil(), max.floor());
    if hi < lo {
        return Err(JobError::retryable(format!(
            "Max {max} should be greater than min {min}."
        )));
    }
    Ok((lo + (math_random() * (hi - lo + 1.0)).floor()) as i64)
}

async fn generate_code(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let num = |key: &str| ctx.step.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let (min, max) = (num("min"), num("max"));
    let value = match ctx.str("type") {
        Some("NUMERIC_LENGTH") => {
            random_int(10f64.powf(min - 1.0), 10f64.powf(max) - 1.0)?.to_string()
        }
        Some("NUMERIC_VALUE") => random_int(min, max)?.to_string(),
        Some("ALPHANUMERIC_LENGTH") => {
            // `faker.string.alpha`: mixed-case letters only.
            const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
            let length = random_int(min, max)?.max(0);
            (0..length)
                .map(|_| LETTERS[(math_random() * LETTERS.len() as f64) as usize] as char)
                .collect()
        }
        _ => String::new(),
    };
    if !value.is_empty() {
        write(ctx, ctx.str("outputFieldId").unwrap_or(""), value).await?;
    }
    Ok(StepStatus::Success)
}

async fn get_data_from_json(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let (ws, contact_id) = (ctx.workspace_id(), ctx.contact_id());
    let db = &ctx.run.deps.db;
    let Some(input) = read(db, ws, contact_id, ctx.str("inputFieldId").unwrap_or("")).await? else {
        return Ok(step_error(ctx, "Input custom field not found"));
    };
    let Ok(data) = serde_json::from_str::<Value>(&input) else {
        return Ok(step_error(
            ctx,
            "Input custom field value is not valid JSON",
        ));
    };
    let mut custom = Vec::new();
    let mut bot = Vec::new();
    for entry in ctx
        .step
        .get("mapping")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let output = entry
            .get("outputFieldId")
            .and_then(Value::as_str)
            .unwrap_or("");
        let path = entry.get("jsonPath").and_then(Value::as_str).unwrap_or("");
        let value = stringify_json_path_value(read_json_path(&data, path));
        match field(output) {
            Field::Bot(_) => bot.push((output.to_owned(), value)),
            Field::Custom(Some(id)) => custom.push((id, value)),
            Field::Custom(None) => {}
        }
    }
    // Custom-field outputs: the existing fields, one batched `setValues`.
    if !custom.is_empty() {
        let ids: Vec<i64> = custom.iter().map(|(id, _)| *id).collect();
        let valid: Vec<i64> = db
            .read(move |conn| {
                let mut out = Vec::new();
                for id in ids {
                    if field_type(conn, ws, Field::Custom(Some(id)))?.is_some() {
                        out.push(id);
                    }
                }
                Ok(out)
            })
            .await
            .map_err(db_fail)?;
        let fields: Vec<(i64, String)> = custom
            .into_iter()
            .filter(|(id, _)| valid.contains(id))
            .filter_map(|(id, value)| Some((id, value?)))
            .collect();
        if !fields.is_empty() {
            set_custom_values(ctx, fields).await?;
        }
    }
    for (output, value) in bot {
        if let Some(value) = value {
            write(ctx, &output, value).await?;
        }
    }
    Ok(StepStatus::Success)
}
