//! `features/contacts/api/public/export.ts` (`contactsExportPublicRouter`,
//! P3.6-D2 slice contact export): `contacts.export / getExportFile /
//! listExportFiles`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chatbotx_business::contact_export::{
    ExportFileCursor, ExportFilter, StartExport, get_file, list_files, run_export, start,
};
use chatbotx_business::contact_filter::{parse_strict_param, to_filter_criteria};
use chatbotx_db::repo::iso;
use serde_json::{Value, json};

use super::contact_inboxes::{invalid_cursor, lenient_base64};
use super::input::{Input, Num, Seg, Str, key};
use super::{ApiState, CREATING, Call, ErrorMap, FINDING, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "contacts.export",
    "contacts.getExportFile",
    "contacts.listExportFiles",
];

/// `possibleErrorsOnListingWithCursor`.
const LISTING_WITH_CURSOR: ErrorMap = &[("businessError", 400), ("invalidCursor", 400)];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "contacts.export" => start_export(state, call).await,
        "contacts.getExportFile" => {
            let mut input = Input::from_query(call.params, call.query);
            let file_id = input.id("fileId");
            input.finish()?;
            let file = get_file(
                &state.db,
                state.files.as_deref(),
                call.workspace_id,
                file_id.unwrap_or_default(),
                None,
            )
            .await
            .map_err(|e| business_error(e, FINDING))?;
            Ok(json(
                200,
                json!({
                    "status": file.status,
                    "fileName": file.file_name,
                    "downloadUrl": file.download_url,
                    "totalRecords": file.total_records,
                }),
            ))
        }
        _ => list_export_files(state, call).await,
    }
}

/// `exportContactsRequest` -> `contactExportService.start` (a token: no
/// user, full PII), then the export runs in the request (no default-queue
/// worker; the `bulkTagContacts` precedent). A run error is logged; the
/// File row carries the result.
async fn start_export(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let fields = string_array(&mut input, "fields", true, |input, path, value| {
        let text = input.string_at(path, Some(value), Str::default(), true)?;
        let ok = ["sys:", "cus:", "tag:"]
            .iter()
            .any(|p| text.strip_prefix(p).is_some_and(|rest| !rest.is_empty()));
        if !ok {
            input.issue(
                path,
                "invalid_format",
                "Use sys:<column>, cus:<customFieldId> or tag:<tagId>",
            );
        }
        ok.then_some(text)
    });
    let contact_ids = string_array(&mut input, "contactIds", false, |input, path, value| {
        input.id_at(path, Some(value), true)
    });
    let export_all = input.opt_bool("exportAll");
    let filter = match input.get("filter").cloned() {
        None => None,
        Some(Value::Object(object)) => {
            let keyword = match object.get("keyword") {
                None => None,
                value => input.string_at(
                    &[Seg::Key("filter".into()), Seg::Key("keyword".into())],
                    value,
                    Str::default(),
                    false,
                ),
            };
            let contact_filter = match parse_strict_param(object.get("contactFilter")) {
                Ok(parsed) => parsed.as_ref().map(to_filter_criteria),
                Err(issues) => {
                    input.extend_issues(issues.into_iter().map(under_filter).collect());
                    None
                }
            };
            Some(ExportFilter {
                keyword,
                contact_filter,
            })
        }
        Some(other) => {
            input.issue(
                &key("filter"),
                "invalid_type",
                &format!(
                    "Invalid input: expected object, received {}",
                    type_name(&other)
                ),
            );
            None
        }
    };
    // `.refine` (only once the object itself parsed).
    if !input.failed() && export_all != Some(true) && contact_ids.as_ref().is_none_or(Vec::is_empty)
    {
        input.issue(
            &key("contactIds"),
            "custom",
            "Either contactIds or exportAll must be provided",
        );
    }
    input.finish()?;
    let job = start(
        &state.db,
        StartExport {
            workspace_id: call.workspace_id,
            requested_user_id: None,
            can_export_email_and_phone: true,
            restrict_to_assigned_user_id: None,
            fields: fields.unwrap_or_default(),
            export_all: export_all == Some(true),
            filter,
            contact_ids: contact_ids.unwrap_or_default(),
        },
    )
    .await
    .map_err(|e| business_error(e, CREATING))?;
    match state.files.as_deref() {
        Some(files) => {
            if let Err(error) = run_export(&state.db, files, &job).await {
                tracing::error!(%error, file_id = job.file_id, "contact export run failed");
            }
        }
        None => tracing::error!(
            file_id = job.file_id,
            "file storage is not configured; export not run"
        ),
    }
    Ok(json(202, json!({ "fileId": job.file_id.to_string() })))
}

/// `filter.contactFilter` issue paths start at `filter`.
fn under_filter(mut issue: Value) -> Value {
    if let Some(Value::Array(path)) = issue.get_mut("path") {
        path.insert(0, json!("filter"));
    }
    issue
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// An array key; `fields` has `.min(1)`.
fn string_array<T>(
    input: &mut Input,
    name: &str,
    required: bool,
    mut item: impl FnMut(&mut Input, &[Seg], &Value) -> Option<T>,
) -> Option<Vec<T>> {
    let Some(value) = input.get(name).cloned() else {
        if required {
            input.issue(
                &key(name),
                "invalid_type",
                "Invalid input: expected array, received undefined",
            );
        }
        return None;
    };
    let Value::Array(items) = value else {
        input.issue(
            &key(name),
            "invalid_type",
            &format!(
                "Invalid input: expected array, received {}",
                type_name(&value)
            ),
        );
        return None;
    };
    let mut out = Vec::new();
    for (index, value) in items.iter().enumerate() {
        let path = vec![Seg::Key(name.into()), Seg::Index(index)];
        if let Some(parsed) = item(input, &path, value) {
            out.push(parsed);
        }
    }
    if required && items.is_empty() {
        input.issue(
            &key(name),
            "too_small",
            "Too small: expected array to have >=1 items",
        );
    }
    Some(out)
}

/// `decodeCursor(cursor, contactExportFileCursor)`.
fn decode_cursor(text: &str) -> Option<ExportFileCursor> {
    let value: Value = serde_json::from_slice(&lenient_base64(text)?).ok()?;
    ExportFileCursor::parse(
        value.get("createdAt")?.as_str()?,
        value.get("id")?.as_str()?,
    )
}

/// `encodeCursor({ createdAt, id })`.
fn encode_cursor(cursor: &ExportFileCursor) -> String {
    STANDARD
        .encode(json!({ "createdAt": cursor.created_at, "id": cursor.id.to_string() }).to_string())
}

/// `listContactExportFiles` (a token: every export of the workspace).
async fn list_export_files(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 100.0));
    let cursor = input.opt_string("cursor", Str::default());
    input.finish()?;
    let cursor = match cursor.as_deref().filter(|c| !c.is_empty()) {
        Some(text) => Some(decode_cursor(text).ok_or_else(invalid_cursor)?),
        None => None,
    };
    let limit = usize::try_from(per_page.unwrap_or(20)).unwrap_or(20);
    let (rows, next) = list_files(&state.db, call.workspace_id, None, limit, cursor)
        .await
        .map_err(|e| business_error(e, LISTING_WITH_CURSOR))?;
    let data: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.id.to_string(),
                "status": row.status,
                "fileName": row.file_name,
                "totalRecords": row.total_records,
                "createdAt": iso(row.created_at),
                "expiresAt": iso(row.expires_at),
                "requestedBy": row.requested_by.as_ref().map(|(id, name)| json!({ "id": id.to_string(), "name": name })),
            })
        })
        .collect();
    Ok(json(
        200,
        json!({ "data": data, "nextCursor": next.as_ref().map(encode_cursor) }),
    ))
}
