//! The product import routes of `features/products/api/public.ts` (Q.D1i):
//! `products.getImportTemplate / peekImportHeaders / createImportUpload /
//! startImport / listImports / getImport`. Upload, list and get share the
//! contact import code.

use base64::Engine as _;
use chatbotx_business::import::products::{
    PRODUCT_IMPORT_FIELDS, ProductColumnMap, ProductImportMeta, StartProductImport,
    build_product_import_template, product_import_template_file_name,
    resolve_product_import_column_map, resolve_product_import_template_locale,
    start_product_import, suggest_product_import_column_map,
};
use chatbotx_business::import::{ImportType, contacts::run_import, peek_import_headers};
use serde_json::{Map, Value, json};

use super::contact_imports::{
    PEEKING, create_upload, get_import, list_imports, workspace_language,
};
use super::input::{Input, Str, child, key};
use super::{ApiState, Call, ErrorMap, Reply, business_error, internal_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "products.getImportTemplate",
    "products.peekImportHeaders",
    "products.createImportUpload",
    "products.startImport",
    "products.listImports",
    "products.getImport",
];

/// `possibleErrorsOnStartingProductImport`.
const STARTING_IMPORT: ErrorMap = &[
    ("externalStoreManaged", 409),
    ("notFound", 404),
    ("businessError", 400),
    ("importUnableToReadHeaders", 400),
    ("importUnsupportedFileType", 400),
    ("importFileTooLarge", 400),
    ("productImportFileNotFound", 404),
    ("productImportFileTypeInvalid", 422),
    ("productImportFormatMismatch", 422),
    ("productImportAlreadyRunning", 409),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `PRODUCT_IMPORT_TEMPLATE_MIME_TYPE`.
const XLSX_MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    match route_id {
        "products.getImportTemplate" => {
            let mut input = Input::from_query(call.params, call.query);
            let value = input.get("language").cloned();
            let language = input.enum_at(&key("language"), value.as_ref(), &["en", "vi"], false);
            input.finish()?;
            let language = match language {
                Some(language) => language,
                None => workspace_language(state, ws).await?,
            };
            let locale = resolve_product_import_template_locale(&language);
            let template = build_product_import_template(locale);
            Ok(json(
                200,
                json!({
                    "fileName": product_import_template_file_name(locale),
                    "mimeType": XLSX_MIME,
                    "contentBase64": base64::engine::general_purpose::STANDARD.encode(template),
                }),
            ))
        }
        "products.peekImportHeaders" => {
            let mut input = Input::from_query(call.params, call.query);
            let file_id = input.id("fileId");
            input.finish()?;
            let files = state
                .files
                .as_deref()
                .ok_or_else(|| internal_error("file storage is not configured"))?;
            let headers = peek_import_headers(
                &state.db,
                files,
                ws,
                file_id.unwrap_or_default(),
                Some(ImportType::Products),
            )
            .await
            .map_err(|e| business_error(e, PEEKING))?;
            let suggested = suggest_product_import_column_map(&headers);
            let map: Map<String, Value> = suggested
                .into_iter()
                .map(|(field, header)| (field.to_string(), Value::String(header)))
                .collect();
            Ok(json(
                200,
                json!({ "headers": headers, "suggestedColumnMap": Value::Object(map) }),
            ))
        }
        "products.createImportUpload" => create_upload(state, call, ImportType::Products).await,
        "products.startImport" => start_import(state, call).await,
        "products.listImports" => list_imports(state, call, ImportType::Products).await,
        "products.getImport" => get_import(state, call, ImportType::Products).await,
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `startProductImportPublicRequest`'s `columnMap`: `name` required
/// (non-empty), the other fields optional strings.
fn column_map(input: &mut Input) -> Option<ProductColumnMap> {
    let value = input.get("columnMap").cloned()?;
    let path = key("columnMap");
    let Value::Object(object) = value else {
        let received = match value {
            Value::Null => "null",
            Value::Array(_) => "array",
            Value::String(_) => "string",
            Value::Number(_) => "number",
            Value::Bool(_) => "boolean",
            Value::Object(_) => "object",
        };
        input.issue(
            &path,
            "invalid_type",
            &format!("Invalid input: expected object, received {received}"),
        );
        return None;
    };
    let mut pairs: Vec<(&str, String)> = Vec::new();
    for field in PRODUCT_IMPORT_FIELDS {
        let required = field == "name";
        let spec = if required {
            Str {
                min: Some(1),
                ..Str::default()
            }
        } else {
            Str::default()
        };
        if let Some(text) = input.string_at(&child(&path, field), object.get(field), spec, required)
        {
            pairs.push((field, text));
        }
    }
    ProductColumnMap::from_pairs(&pairs)
}

/// `products.startImport`: the column map (sent or recognised from the
/// headers), the import row, then the run in the request.
async fn start_import(state: &ApiState, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    let mut input = Input::from_body(call.params, call.body);
    let file_id = input.id("fileId");
    let value = input.get("format").cloned();
    let format = input.enum_at(&key("format"), value.as_ref(), &["csv", "xlsx"], false);
    let sent_map = column_map(&mut input);
    let create_missing = input.opt_bool("createMissingCategories").unwrap_or(true);
    input.finish()?;
    let file_id = file_id.unwrap_or_default();
    let files = state
        .files
        .as_deref()
        .ok_or_else(|| internal_error("file storage is not configured"))?;
    let column_map = resolve_product_import_column_map(&state.db, files, ws, file_id, sent_map)
        .await
        .map_err(|e| business_error(e, STARTING_IMPORT))?;
    let import_id = start_product_import(
        &state.db,
        StartProductImport {
            workspace_id: ws,
            user_id: None,
            file_id,
            format,
            meta: ProductImportMeta {
                column_map,
                create_missing_categories: create_missing,
            },
        },
    )
    .await
    .map_err(|e| business_error(e, STARTING_IMPORT))?;
    if let Err(error) = run_import(&state.db, files, import_id).await {
        tracing::error!(%error, import_id, "product import run failed");
    }
    Ok(json(201, json!({ "importId": import_id.to_string() })))
}
