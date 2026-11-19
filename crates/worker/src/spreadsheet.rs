//! The Google Sheets flow steps (`spreadsheet-handler.ts`,
//! `spreadsheet-lookup-values.ts`, `spreadsheet-write-values.ts`,
//! `operator-handler.ts`) and the trigger action `runGoogleSheet`
//! (`integration-actions.ts`), which builds the same step and runs it.
//!
//! The Sheets calls are `chatbotx_business::google_sheets` (Q.I4) over the
//! given [`Http`]; a failure is logged, recorded as a provider error and
//! ends the step as `error` with the original message.

use chatbotx_business::email_marketing::Http;
use chatbotx_business::{bot_field, contact_custom_field, google_sheets};
use chatbotx_core::flow_config::field_reference::{FieldReference, parse_field_reference};
use chatbotx_core::flow_config::steps::to_spreadsheet_step_version;
use chatbotx_core::utils::datetime::{SourceTimezoneStrategy, TemporalInputParsing};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};

use crate::{Deps, str_of};

/// The Sheets steps (`stepTypes.enum.spreadsheet*`).
pub(crate) const STEP_TYPES: &[&str] = &[
    "spreadsheetGetRow",
    "spreadsheetClearRow",
    "spreadsheetGetRandomRow",
    "spreadsheetSendData",
    "spreadsheetUpdateRow",
];

/// Who the step runs for.
pub(crate) struct SheetRun<'a> {
    pub deps: &'a Deps,
    pub http: &'a dyn Http,
    pub conversation: &'a Conversation,
    pub contact_inbox: &'a ContactInbox,
    pub sent_by_user_id: Option<String>,
}

/// `compare` (`operator-handler.ts`): both sides trimmed; the ordering
/// operators compare as JS `Number(…)` (`""` = 0, not a number = never).
fn compare(cell: &str, value: &str, operator: &str) -> bool {
    let (a, b) = (cell.trim(), value.trim());
    let number = |s: &str| {
        if s.is_empty() {
            Some(0.0)
        } else {
            s.parse::<f64>().ok()
        }
    };
    let ordered = |f: fn(f64, f64) -> bool| match (number(a), number(b)) {
        (Some(x), Some(y)) => f(x, y),
        _ => false,
    };
    match operator {
        "is" => a == b,
        "is_not" => a != b,
        "gte" => ordered(|x, y| x >= y),
        "lte" => ordered(|x, y| x <= y),
        "gt" => ordered(|x, y| x > y),
        "lt" => ordered(|x, y| x < y),
        "contains" => a.contains(b),
        "not_contains" => !a.contains(b),
        "starts_with" => a.starts_with(b),
        "ends_with" => a.ends_with(b),
        // The original throws; the step's catch turns it into an error.
        _ => false,
    }
}

/// `isMatchedRow`: `AND` = every condition, else any (no condition: `AND`
/// matches, `OR` does not).
pub(crate) fn is_matched_row(headers: &[String], row: &[String], lookup: &Value) -> bool {
    let conditions = lookup["conditions"].as_array().cloned().unwrap_or_default();
    let mut results = conditions.iter().map(|c| {
        let column = c["column"].as_str().unwrap_or_default();
        let Some(index) = headers.iter().position(|h| h == column) else {
            return false;
        };
        // An absent cell is JS `undefined`, `String(undefined)`.
        let cell = row.get(index).map_or("undefined", String::as_str);
        compare(
            cell,
            c["value"].as_str().unwrap_or_default(),
            c["operator"].as_str().unwrap_or_default(),
        )
    });
    if lookup["mode"] == "AND" {
        results.all(|m| m)
    } else {
        results.any(|m| m)
    }
}

/// `alignWriteValuesToHeaders`: each value goes to its mapping header's
/// column; a header no longer in the sheet is skipped.
pub(crate) fn align_write_values(
    map: &[Value],
    values: &[String],
    headers: &[String],
    base_row: &[String],
    skip_empty: bool,
) -> Vec<String> {
    let width = headers.len().max(base_row.len());
    let mut row: Vec<String> = (0..width)
        .map(|i| base_row.get(i).cloned().unwrap_or_default())
        .collect();
    for (index, item) in map.iter().enumerate() {
        let header = item["header"].as_str().unwrap_or_default();
        let Some(column) = headers.iter().position(|h| h == header) else {
            continue;
        };
        let value = values.get(index).cloned().unwrap_or_default();
        if skip_empty && value.is_empty() {
            continue;
        }
        row[column] = value;
    }
    row
}

/// Why a Sheets step failed: the step's `errorMessage`, and the provider
/// failure when one was thrown (logged as a provider error).
pub(crate) struct Failure {
    pub message: &'static str,
    pub cause: Option<String>,
}

fn failed(message: &'static str) -> impl Fn(String) -> Failure {
    move |cause| Failure {
        message,
        cause: Some(cause),
    }
}

/// The spreadsheet row, the integration auth and the Sheets id.
async fn sheet_target(deps: &Deps, ws: i64, step: &Value) -> Result<(Value, String), String> {
    let id = str_of(step, "spreadsheetId")
        .and_then(|id| id.parse::<i64>().ok())
        .unwrap_or(-1);
    let row = google_sheets::find_by_workspace_or_fail(&deps.db, ws)
        .await
        .map_err(|e| e.message())?;
    let spreadsheet = chatbotx_business::spreadsheet::find_or_fail(&deps.db, ws, id)
        .await
        .map_err(|e| e.message())?;
    Ok((row.auth, spreadsheet.spreadsheet_id))
}

/// `getSheetData`: the header row and every row.
async fn sheet_data(
    r: &SheetRun<'_>,
    step: &Value,
) -> Result<(Value, String, Vec<String>, Vec<Vec<String>>), String> {
    let ws = r.conversation.workspace_id;
    let (auth, sheet_id) = sheet_target(r.deps, ws, step).await?;
    let sheet = str_of(step, "sheetName").unwrap_or_default();
    let headers = google_sheets::list_sheet_headers(r.http, &auth, &sheet_id, sheet)
        .await
        .map_err(|e| e.message)?;
    let rows = google_sheets::get_sheet_values(r.http, &auth, &sheet_id, sheet)
        .await
        .map_err(|e| e.message)?;
    Ok((auth, sheet_id, headers, rows))
}

/// `resolveSpreadsheetLookup`: the condition values through the contact's
/// variables.
async fn resolve_lookup(r: &SheetRun<'_>, step: &Value) -> Result<Value, String> {
    let mut lookup = step["lookup"].clone();
    let values: Vec<Value> = lookup["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| c["value"].as_str().map_or(json!(""), |v| json!(v)))
        .collect();
    if values.is_empty() {
        return Ok(lookup);
    }
    let resolved = resolve(r, &Value::Array(values)).await?;
    if let Some(conditions) = lookup["conditions"].as_array_mut() {
        for (condition, value) in conditions.iter_mut().zip(resolved) {
            condition["value"] = Value::String(value);
        }
    }
    Ok(lookup)
}

/// The strings of `value` through the contact's variables (none = as is).
async fn resolve(r: &SheetRun<'_>, value: &Value) -> Result<Vec<String>, String> {
    let resolved = match &r.deps.variables {
        Some(variables) => {
            variables
                .resolve(
                    r.conversation,
                    r.contact_inbox,
                    value,
                    None,
                    r.sent_by_user_id.clone(),
                    false,
                )
                .await?
        }
        None => value.clone(),
    };
    Ok(resolved
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| v.as_str().unwrap_or_default().to_owned())
        .collect())
}

/// `buildSpreadsheetWriteData`: v1 reads the mapped custom / bot fields, v2
/// resolves each mapping's value template.
async fn write_values(r: &SheetRun<'_>, step: &Value) -> Result<Vec<String>, String> {
    let map = step["map"].as_array().cloned().unwrap_or_default();
    if to_spreadsheet_step_version(step["version"].as_str()) == "v2" {
        let templates: Vec<Value> = map
            .iter()
            .map(|m| json!(m["value"].as_str().unwrap_or_default()))
            .collect();
        return resolve(r, &Value::Array(templates)).await;
    }
    let (ws, contact) = (r.conversation.workspace_id, r.conversation.contact_id);
    let stored: Vec<(String, Option<String>)> = r
        .deps
        .db
        .read(move |c| {
            let mut stmt = c.prepare(
                r#"SELECT CAST("customFieldId" AS TEXT), "value" FROM "ContactCustomField" WHERE "contactId" = ?1"#,
            )?;
            let rows = stmt
                .query_map(params![contact], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(|e| e.to_string())?;
    let bot_ids: Vec<i64> = map
        .iter()
        .filter_map(
            |m| match parse_field_reference(m["customFieldId"].as_str()?) {
                FieldReference::BotField { id } => id.parse().ok(),
                FieldReference::CustomField { .. } => None,
            },
        )
        .collect();
    let bot_values: Vec<(String, String)> = if bot_ids.is_empty() {
        Vec::new()
    } else {
        bot_field::find_many_by_ids(&r.deps.db, ws, &bot_ids)
            .await
            .map_err(|e| e.message())?
            .into_iter()
            .map(|f| (f.id.to_string(), f.value.unwrap_or_default()))
            .collect()
    };
    Ok(map
        .iter()
        .map(|m| {
            let key = m["customFieldId"].as_str().unwrap_or_default();
            if key.is_empty() {
                return String::new();
            }
            let found = match parse_field_reference(key) {
                FieldReference::BotField { id } => bot_values
                    .iter()
                    .find(|(k, _)| *k == id)
                    .map(|(_, v)| v.clone()),
                FieldReference::CustomField { key } => stored
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.clone().unwrap_or_default()),
            };
            found.unwrap_or_default()
        })
        .collect())
}

/// `updateContactCustomFields`: the mapped cells into the contact's custom
/// fields; a bot field reference goes to `updateByKey` (a failure skips it).
async fn store_row(
    r: &SheetRun<'_>,
    step: &Value,
    headers: &[String],
    row: &[String],
) -> Result<(), String> {
    let ws = r.conversation.workspace_id;
    let mut custom: Vec<(i64, String)> = Vec::new();
    for item in step["map"].as_array().into_iter().flatten() {
        let header = item["header"].as_str().unwrap_or_default();
        let key = item["customFieldId"].as_str().unwrap_or_default();
        let Some(index) = headers.iter().position(|h| h == header) else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        let value = row.get(index).cloned().unwrap_or_default();
        match parse_field_reference(key) {
            FieldReference::BotField { id } => {
                let data = bot_field::UpdateBotField {
                    value: Some(Some(value)),
                    ..bot_field::UpdateBotField::default()
                };
                if let Err(err) = bot_field::update_by_key(&r.deps.db, ws, &id, data).await {
                    tracing::warn!(err = %err.message(), workspace_id = ws, bot_field_id = %id, "Failed to write spreadsheet value to bot field; skipping entry");
                }
            }
            FieldReference::CustomField { key } => {
                // `setValues` skips a field it does not know.
                if let Ok(id) = key.parse::<i64>() {
                    custom.push((id, value));
                }
            }
        }
    }
    if custom.is_empty() {
        return Ok(());
    }
    contact_custom_field::set_values(
        &r.deps.db,
        contact_custom_field::SetValuesInput {
            workspace_id: ws,
            contact_id: r.conversation.contact_id,
            fields: custom,
            temporal_input_parsing: Some(TemporalInputParsing::Lenient),
            source_timezone_strategy: Some(SourceTimezoneStrategy::Workspace),
            ..contact_custom_field::SetValuesInput::default()
        },
    )
    .await
    .map(drop)
    .map_err(|e| e.message())
}

/// The indices of the matching rows (`findRows`; `first` = stop at one).
fn matching(headers: &[String], rows: &[Vec<String>], lookup: &Value, first: bool) -> Vec<usize> {
    let mut out = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if is_matched_row(headers, row, lookup) {
            out.push(index);
            if first {
                break;
            }
        }
    }
    out
}

fn row_index(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// Runs one Sheets step.
pub(crate) async fn execute(
    r: &SheetRun<'_>,
    step_type: &str,
    step: &Value,
) -> Result<(), Failure> {
    let sheet = str_of(step, "sheetName").unwrap_or_default();
    match step_type {
        "spreadsheetGetRow" | "spreadsheetGetRandomRow" => {
            let (message, not_found) = if step_type == "spreadsheetGetRow" {
                ("Failed to get spreadsheet row", "Row not found")
            } else {
                ("Failed to get random spreadsheet row", "No rows found")
            };
            let (_, _, headers, rows) = sheet_data(r, step).await.map_err(failed(message))?;
            let lookup = resolve_lookup(r, step).await.map_err(failed(message))?;
            let found = matching(&headers, &rows, &lookup, step_type == "spreadsheetGetRow");
            let pick = if step_type == "spreadsheetGetRow" {
                found.first().copied()
            } else {
                // `getRandomRow`: `Math.random()` over the matches.
                use std::hash::BuildHasher as _;
                let x = std::collections::hash_map::RandomState::new()
                    .hash_one(chatbotx_db::repo::now_ms());
                (!found.is_empty()).then(|| found[(x % found.len() as u64) as usize])
            };
            let Some(index) = pick else {
                return Err(Failure {
                    message: not_found,
                    cause: None,
                });
            };
            store_row(r, step, &headers, &rows[index])
                .await
                .map_err(failed(message))
        }
        "spreadsheetSendData" => {
            let message = "Failed to send spreadsheet data";
            let ws = r.conversation.workspace_id;
            let (auth, sheet_id) = sheet_target(r.deps, ws, step)
                .await
                .map_err(failed(message))?;
            let headers = google_sheets::list_sheet_headers(r.http, &auth, &sheet_id, sheet)
                .await
                .map_err(|e| failed(message)(e.message))?;
            let values = write_values(r, step).await.map_err(failed(message))?;
            let map = step["map"].as_array().cloned().unwrap_or_default();
            let data = align_write_values(&map, &values, &headers, &[], false);
            google_sheets::insert_row(r.http, &auth, &sheet_id, sheet, &data)
                .await
                .map_err(|e| failed(message)(e.message))
        }
        "spreadsheetUpdateRow" | "spreadsheetClearRow" => {
            let message = if step_type == "spreadsheetUpdateRow" {
                "Failed to update spreadsheet row"
            } else {
                "Failed to clear spreadsheet row"
            };
            let (auth, sheet_id, headers, rows) =
                sheet_data(r, step).await.map_err(failed(message))?;
            let lookup = resolve_lookup(r, step).await.map_err(failed(message))?;
            let found = matching(&headers, &rows, &lookup, false);
            if found.is_empty() {
                return Err(Failure {
                    message: "No rows found",
                    cause: None,
                });
            }
            if step_type == "spreadsheetClearRow" {
                for index in found {
                    google_sheets::clear_row(r.http, &auth, &sheet_id, sheet, row_index(index))
                        .await
                        .map_err(|e| failed(message)(e.message))?;
                }
                return Ok(());
            }
            let values = write_values(r, step).await.map_err(failed(message))?;
            let map = step["map"].as_array().cloned().unwrap_or_default();
            for index in found {
                // Unmapped columns keep their value; an empty value leaves the cell.
                let data = align_write_values(&map, &values, &headers, &rows[index], true);
                google_sheets::update_row(r.http, &auth, &sheet_id, sheet, row_index(index), &data)
                    .await
                    .map_err(|e| failed(message)(e.message))?;
            }
            Ok(())
        }
        _ => Err(Failure {
            message: "Unknown spreadsheet step",
            cause: None,
        }),
    }
}

/// Logs a failed Sheets step and records the provider error
/// (`logStepProviderError("google-sheets", …)`).
pub(crate) async fn report(r: &SheetRun<'_>, step_type: &str, failure: &Failure) {
    let Some(cause) = &failure.cause else {
        return;
    };
    tracing::error!(
        workspace_id = r.conversation.workspace_id,
        conversation_id = r.conversation.id,
        step_type,
        err = %cause,
        "{}",
        failure.message
    );
    let input = crate::events::ProviderError {
        provider: "google-sheets".to_owned(),
        workspace_id: r.conversation.workspace_id.to_string(),
        contact_id: Some(r.conversation.contact_id.to_string()),
        source_id: Some(r.contact_inbox.source_id.clone()),
        error: json!({ "message": cause }),
        ..Default::default()
    };
    crate::events::log_provider_errors(r.deps.events.as_ref(), &[input]).await;
}

/// `runGoogleSheet`'s step: the action's `action` (the step type), sheet,
/// lookup and map.
pub(crate) fn step_of_action(action: &Value) -> Option<(String, Value)> {
    let step_type = str_of(action, "action")?;
    if !STEP_TYPES.contains(&step_type) {
        return None;
    }
    let step = json!({
        "stepType": step_type,
        "spreadsheetId": action.get("spreadsheetId").cloned().unwrap_or(Value::Null),
        "sheetName": action.get("sheetName").cloned().unwrap_or(Value::Null),
        "lookup": action.get("lookup").cloned().unwrap_or(json!({ "mode": "AND", "conditions": [] })),
        "map": action.get("map").cloned().unwrap_or(json!([])),
    });
    Some((step_type.to_owned(), step))
}

#[cfg(test)]
#[path = "spreadsheet_tests.rs"]
mod handler_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|v| (*v).to_owned()).collect()
    }

    /// `operator-handler.test.ts`.
    #[test]
    fn rows_match_by_mode_and_trimmed_operators() {
        let headers = s(&["Email", "Age"]);
        let row = s(&["ada@example.com ", "36"]);
        let and = json!({ "mode": "AND", "conditions": [
            { "column": "Email", "operator": "is", "value": " ada@example.com" },
            { "column": "Age", "operator": "gte", "value": "30" },
        ]});
        assert!(is_matched_row(&headers, &row, &and));
        let or = json!({ "mode": "OR", "conditions": [
            { "column": "Missing", "operator": "is", "value": "x" },
            { "column": "Age", "operator": "lt", "value": "40" },
        ]});
        assert!(is_matched_row(&headers, &row, &or));
        let nan = json!({ "mode": "AND", "conditions": [
            { "column": "Email", "operator": "gt", "value": "1" },
        ]});
        assert!(!is_matched_row(&headers, &row, &nan));
        assert!(compare("", "0", "gte"));
        assert!(compare("Hello world", "world", "ends_with"));
        assert!(!compare("Hello", "x", "contains"));
    }

    /// `spreadsheet-write-values.test.ts`: header-aligned writes.
    #[test]
    fn write_values_align_to_headers() {
        let map = vec![
            json!({ "header": "B" }),
            json!({ "header": "Gone" }),
            json!({ "header": "A" }),
        ];
        let headers = s(&["A", "B", "C"]);
        let values = s(&["b", "x", "a"]);
        assert_eq!(
            align_write_values(&map, &values, &headers, &[], false),
            s(&["a", "b", ""])
        );
        let base = s(&["1", "2", "3", "4"]);
        let values = s(&["", "x", "a"]);
        assert_eq!(
            align_write_values(&map, &values, &headers, &base, true),
            s(&["a", "2", "3", "4"])
        );
    }

    #[test]
    fn run_google_sheet_builds_the_step() {
        let action = json!({ "type": "runGoogleSheet", "action": "spreadsheetSendData", "spreadsheetId": "5", "sheetName": "S" });
        let (kind, step) = step_of_action(&action).expect("step");
        assert_eq!(kind, "spreadsheetSendData");
        assert_eq!(step["map"], json!([]));
        assert!(step_of_action(&json!({ "action": "other" })).is_none());
    }
}
