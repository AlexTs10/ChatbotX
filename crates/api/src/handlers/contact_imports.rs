//! `features/import/api/public.ts` (`importPublicRouter`, P3.6-D2 slice
//! contact imports, part one): `contacts.createImportUpload /
//! getImportTemplate / peekImportHeaders / listImports / getImport`.

use chatbotx_business::BusinessError;
use chatbotx_business::connection::providers::CHANNEL_TYPES;
use chatbotx_business::import::contacts::{
    ColumnMap, ContactImportMeta, FieldMapping, StartContactImport, run_import,
    start_contact_import,
};
use chatbotx_business::import::template::{
    CONTACTS_IMPORT_TEMPLATE_FILENAME, build_contacts_import_template_csv,
};
use chatbotx_business::import::{
    CreateImportUpload, ImportRow, ImportType, ListImports, create_import_upload, find, list,
    peek_import_headers, suggest_contact_import_column_map,
};
use chatbotx_db::repo::iso;
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Seg, Str, child, key};
use super::qr_codes::{public_paging, sort_query};
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, Reply, business_error, internal_error, json,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "contacts.createImportUpload",
    "contacts.getImportTemplate",
    "contacts.peekImportHeaders",
    "contacts.listImports",
    "contacts.getImport",
    "contacts.import",
];

/// `possibleErrorsOnCreatingImportUpload`.
const CREATING_UPLOAD: ErrorMap = &[
    ("businessError", 400),
    ("importUnsupportedFileType", 400),
    ("importFileTooLarge", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `possibleErrorsOnPeekingImportHeaders`.
pub(super) const PEEKING: ErrorMap = &[
    ("businessError", 400),
    ("importUnableToReadHeaders", 400),
    ("importUnsupportedFileType", 400),
    ("importFileTooLarge", 400),
];

/// `possibleErrorsOnStartingContactImport`.
const STARTING_IMPORT: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("contactImportAlreadyRunning", 409),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `importStatuses`.
const IMPORT_STATUSES: &[&str] = &["pending", "processing", "completed", "failed"];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    match route_id {
        "contacts.createImportUpload" => create_upload(state, call, ImportType::Contacts).await,
        "contacts.getImportTemplate" => {
            let mut input = Input::from_query(call.params, call.query);
            let value = input.get("language").cloned();
            let language = input.enum_at(&key("language"), value.as_ref(), &["en", "vi"], false);
            input.finish()?;
            let language = match language {
                Some(language) => language,
                None => workspace_language(state, ws).await?,
            };
            Ok(json(
                200,
                json!({
                    "fileName": CONTACTS_IMPORT_TEMPLATE_FILENAME,
                    "mimeType": "text/csv",
                    "content": build_contacts_import_template_csv(&language),
                }),
            ))
        }
        "contacts.peekImportHeaders" => {
            let mut input = Input::from_query(call.params, call.query);
            let file_id = input.id("fileId");
            input.finish()?;
            let file_id = file_id.unwrap_or_default();
            let files = state
                .files
                .as_deref()
                .ok_or_else(|| internal_error("file storage is not configured"))?;
            let headers =
                peek_import_headers(&state.db, files, ws, file_id, Some(ImportType::Contacts))
                    .await
                    .map_err(|e| business_error(e, PEEKING))?;
            // `contactImportHeadersPublicResponse` key order.
            let suggested = suggest_contact_import_column_map(&headers);
            let mut map = Map::new();
            for field in [
                "phoneNumber",
                "contactId",
                "email",
                "firstName",
                "lastName",
                "sourceUserId",
            ] {
                if let Some((_, header)) = suggested.iter().find(|(f, _)| *f == field) {
                    map.insert(field.into(), Value::String(header.clone()));
                }
            }
            Ok(json(
                200,
                json!({ "headers": headers, "suggestedColumnMap": Value::Object(map) }),
            ))
        }
        "contacts.listImports" => list_imports(state, call, ImportType::Contacts).await,
        "contacts.getImport" => get_import(state, call, ImportType::Contacts).await,
        "contacts.import" => start_import(state, call).await,
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `createImportUpload` of an import type (`contacts` / `products`).
pub(super) async fn create_upload(
    state: &ApiState,
    call: Call<'_>,
    import_type: ImportType,
) -> Reply {
    let ws = call.workspace_id;
    let mut input = Input::from_body(call.params, call.body);
    let file_name = input.string(
        "fileName",
        Str {
            min: Some(1),
            max: Some(255),
            ..Str::default()
        },
    );
    let mime_type = input.string(
        "mimeType",
        Str {
            min: Some(1),
            max: Some(255),
            ..Str::default()
        },
    );
    let file_size = input.number(
        "fileSize",
        Num {
            int: true,
            min: Some(1.0),
            max: None,
        },
    );
    input.finish()?;
    let (Some(file_name), Some(mime_type), Some(file_size)) = (file_name, mime_type, file_size)
    else {
        return Err(crate::routes::not_ported_error());
    };
    let files = state
        .files
        .as_deref()
        .ok_or_else(|| internal_error("file storage is not configured"))?;
    let storage_url = super::contacts::tenant_urls(state).storage_url;
    let upload = create_import_upload(
        &state.db,
        files,
        &storage_url,
        CreateImportUpload {
            workspace_id: ws,
            user_id: None,
            import_type,
            file_name,
            mime_type,
            // A positive JSON integer (checked above).
            file_size: file_size as i64,
        },
    )
    .await
    .map_err(|e| business_error(e, CREATING_UPLOAD))?;
    Ok(json(
        201,
        json!({
            "fileId": upload.file_id.to_string(),
            "presignedPostUrl": upload.presigned_post_url,
            "publicUrl": upload.public_url,
            "path": upload.path,
        }),
    ))
}

/// `listImports` of an import type.
pub(super) async fn list_imports(
    state: &ApiState,
    call: Call<'_>,
    import_type: ImportType,
) -> Reply {
    let ws = call.workspace_id;
    let mut input = Input::from_query(call.params, call.query);
    let (page, per_page) = public_paging(&mut input);
    let value = input.get("status").cloned();
    let status = input.enum_at(&key("status"), value.as_ref(), IMPORT_STATUSES, false);
    let keyword = input.nullish_string("keyword", Str::default()).flatten();
    let sort = sort_query(&mut input, None);
    input.finish()?;
    let (rows, page_count) = list(
        &state.db,
        ws,
        ListImports {
            import_type: Some(import_type),
            status,
            keyword,
            page: Some(page),
            per_page: Some(per_page),
            sort,
        },
    )
    .await
    .map_err(|e| business_error(e, LISTING))?;
    let data: Vec<Value> = rows
        .iter()
        .map(|row| import_json(row, import_type))
        .collect();
    Ok(json(200, json!({ "data": data, "pageCount": page_count })))
}

/// `getImport` of an import type (404 `Import not found`).
pub(super) async fn get_import(state: &ApiState, call: Call<'_>, import_type: ImportType) -> Reply {
    let ws = call.workspace_id;
    let mut input = Input::from_query(call.params, call.query);
    let id = input.id("id");
    input.finish()?;
    let row = find(&state.db, ws, id.unwrap_or_default(), import_type)
        .await
        .map_err(|e| business_error(e, FINDING))?
        .ok_or_else(|| business_error(BusinessError::not_found("Import not found"), FINDING))?;
    Ok(json(200, import_json(&row, import_type)))
}

/// `contacts.import` (`importContactsRequest` + `buildContactImportMeta`):
/// starts the import and runs it in the request (no default-queue worker;
/// the `bulkTagContacts` precedent). A run error is logged, the job row
/// carries the result.
async fn start_import(state: &ApiState, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    let mut input = Input::from_body(call.params, call.body);
    let file_id = input.id("fileId");
    let value = input.get("channel").cloned();
    let channel = input.enum_at(&key("channel"), value.as_ref(), CHANNEL_TYPES, true);
    let inbox_id = input.id("inboxId");
    let timezone = input.opt_string("timezone", Str::trimmed(1, 255));
    let country_code = match input.get("countryCode").cloned() {
        None => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(value) => {
            let text = input.string_at(&key("countryCode"), Some(&value), Str::default(), true);
            match text {
                Some(text) if is_country_code(&text) => Some(text),
                Some(_) => {
                    input.issue(
                        &key("countryCode"),
                        "invalid_format",
                        "Country code must be in E.164 format (e.g. +1, +84)",
                    );
                    None
                }
                None => None,
            }
        }
    };
    let column = |input: &mut Input, name: &str| input.opt_string(name, Str::max(255));
    let column_map = ColumnMap {
        phone_number: column(&mut input, "phoneNumber"),
        contact_id: column(&mut input, "contactId"),
        source_user_id: column(&mut input, "sourceUserId"),
        email: column(&mut input, "email"),
        first_name: column(&mut input, "firstName"),
        last_name: column(&mut input, "lastName"),
    };
    let tag_id = input
        .get("tagId")
        .cloned()
        .and_then(|value| input.id_at(&key("tagId"), Some(&value), true));
    let field_mapping = field_mapping(&mut input);
    // `superRefine` (only once the object itself parsed).
    if !input.failed() {
        if channel.as_deref() == Some("whatsapp") {
            if column_map.phone_number.as_deref().is_none_or(str::is_empty)
                && column_map
                    .source_user_id
                    .as_deref()
                    .is_none_or(str::is_empty)
            {
                input.issue(
                    &key("phoneNumber"),
                    "custom",
                    "Phone number or WhatsApp User ID is required for WhatsApp imports",
                );
            }
        } else if column_map.contact_id.as_deref().is_none_or(str::is_empty) {
            input.issue(&key("contactId"), "custom", "Contact ID is required");
        }
    }
    input.finish()?;
    let (Some(file_id), Some(channel), Some(inbox_id)) = (file_id, channel, inbox_id) else {
        return Err(crate::routes::not_ported_error());
    };
    let meta = ContactImportMeta {
        channel,
        country_code,
        timezone,
        column_map,
        field_mapping,
        tag_id: tag_id.map(|id| id.to_string()),
    };
    let import_id = start_contact_import(
        &state.db,
        StartContactImport {
            workspace_id: ws,
            user_id: None,
            inbox_id,
            file_id,
            meta,
        },
    )
    .await
    .map_err(|e| business_error(e, STARTING_IMPORT))?;
    match state.files.as_deref() {
        Some(files) => {
            if let Err(error) = run_import(&state.db, files, import_id).await {
                tracing::error!(%error, import_id, "contact import run failed");
            }
        }
        None => tracing::error!(import_id, "file storage is not configured; import not run"),
    }
    Ok(json(201, json!({ "importId": import_id.to_string() })))
}

/// `countryCodeSchema`: `^\+\d{1,4}$`.
fn is_country_code(text: &str) -> bool {
    text.strip_prefix('+')
        .is_some_and(|d| (1..=4).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()))
}

/// The `fieldMapping` key: rows without a truthy `column` and
/// `customFieldId` are dropped first; at most 10 `{ column, customFieldId }`
/// (an id or `bot_field:<id>`).
fn field_mapping(input: &mut Input) -> Option<Vec<FieldMapping>> {
    let value = input.get("fieldMapping").cloned()?;
    let path = key("fieldMapping");
    let Value::Array(rows) = value else {
        input.issue(&path, "invalid_type", "Invalid input: expected array");
        return None;
    };
    let truthy = |v: Option<&Value>| match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(_) => true,
    };
    let rows: Vec<Value> = rows
        .into_iter()
        .filter(|row| truthy(row.get("column")) && truthy(row.get("customFieldId")))
        .collect();
    if rows.len() > 10 {
        input.issue(
            &path,
            "too_big",
            "Too big: expected array to have <=10 items",
        );
        return None;
    }
    let mut out = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let mut at = path.clone();
        at.push(Seg::Index(index));
        let column = input.string_at(
            &child(&at, "column"),
            row.get("column"),
            Str {
                min: Some(1),
                max: Some(255),
                ..Str::default()
            },
            true,
        );
        let target = match row.get("customFieldId") {
            Some(Value::String(text)) if is_bot_field_ref(text) => Some(text.clone()),
            Some(Value::String(text))
                if !text.is_empty()
                    && text.len() <= 18
                    && text.bytes().all(|b| b.is_ascii_digit()) =>
            {
                Some(text.clone())
            }
            _ => {
                input.issue(
                    &child(&at, "customFieldId"),
                    "invalid_union",
                    "Invalid input",
                );
                None
            }
        };
        if let (Some(column), Some(custom_field_id)) = (column, target) {
            out.push(FieldMapping {
                column,
                custom_field_id,
            });
        }
    }
    Some(out)
}

/// `^bot_field:\d+$`.
fn is_bot_field_ref(text: &str) -> bool {
    text.strip_prefix("bot_field:")
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// `context.workspace.language`.
pub(super) async fn workspace_language(
    state: &ApiState,
    workspace_id: i64,
) -> Result<String, crate::orpc::OrpcError> {
    state
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "language" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |row| row.get::<_, String>(0),
            )?)
        })
        .await
        .map_err(|e| internal_error(&e.to_string()))
}

/// `contactImportPublicResource` / `productImportPublicResource`.
pub(super) fn import_json(row: &ImportRow, import_type: ImportType) -> Value {
    json!({
        "id": row.id.to_string(),
        "userId": row.user_id.map(|id| id.to_string()),
        "fileId": row.file_id.to_string(),
        "fileName": row.file_name,
        "type": import_type.as_str(),
        "status": row.status,
        "totalCount": row.total_count,
        "processedCount": row.processed_count,
        "successCount": row.success_count,
        "failedCount": row.failed_count,
        "errorMessage": row.error_message,
        "errorSample": row.error_sample,
        "completedAt": row.completed_at.map(iso),
        "createdAt": iso(row.created_at),
        "updatedAt": iso(row.updated_at),
    })
}
