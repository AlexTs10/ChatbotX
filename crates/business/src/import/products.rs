//! Product imports (Q.D1i): `matchProductImportHeaders`,
//! `suggestProductImportColumnMap` / `resolveProductImportColumnMap`
//! (`import/column-map.ts`), `importService.startProductImport`, the
//! product template (`createProductImportTemplate` with the builder labels)
//! and the worker products handler + extractor
//! (`handler/products/{handler,extractor}.ts`) on `runImportPipeline`.
//!
//! As the contact import, the API runs the import in the request after
//! [`start_product_import`] (no default-queue worker).

use std::collections::HashMap;

use chatbotx_core::js;
use chatbotx_db::Db;
use chatbotx_db::repo::product::ProductImportInsert;
use chatbotx_db::repo::{new_id, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_files::{FilesError, Uploader};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::contacts::{
    Counters, ERROR_SAMPLE_LIMIT, ErrorSample, RunRow, complete, fail, mark_processing,
};
use super::xlsx::{self, Cell};
use super::{
    BYTES_PER_MB, ImportType, config, csv, find_import_file, normalize_contact_header,
    peek_import_headers, repo_err, resolve_import_file_format,
};
use crate::errors::{BusinessError, Result};
use crate::{product, product_category, tx};

/// `IMPORT_BATCH_SIZE`.
const BATCH_SIZE: usize = 1000;
/// The products `maxRows`.
const MAX_ROWS: u64 = 10_000;

/// `productImportFields`.
pub const PRODUCT_IMPORT_FIELDS: [&str; 10] = [
    "name",
    "sku",
    "price",
    "discount",
    "shortDescription",
    "category",
    "vendor",
    "inventoryQuantity",
    "imageUrl",
    "productUrl",
];

/// `HEADER_CANDIDATES` (already normalized).
const HEADER_CANDIDATES: [&[&str]; 10] = [
    &["name", "productname", "tensanpham"],
    &["sku", "code", "masanpham"],
    &["price", "sellingprice", "giaban"],
    &["discount", "discountpercentage", "giamgia"],
    &["shortdescription", "description", "motangan"],
    &["category", "productcategory", "danhmuc"],
    &["vendor", "brand", "thuonghieu"],
    &["inventoryquantity", "quantity", "soluong"],
    &["imageurl", "image", "hinhanh"],
    &["producturl", "url", "linksanpham"],
];

/// `ProductImportColumnMap` (`name` required).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductColumnMap {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sku: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_quantity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_url: Option<String>,
}

impl ProductColumnMap {
    /// The map from `(field, header)` pairs (`None` without `name`).
    pub fn from_pairs(pairs: &[(&str, String)]) -> Option<ProductColumnMap> {
        let get = |field: &str| {
            pairs
                .iter()
                .find(|(f, _)| *f == field)
                .map(|(_, h)| h.clone())
        };
        Some(ProductColumnMap {
            name: get("name")?,
            sku: get("sku"),
            price: get("price"),
            discount: get("discount"),
            short_description: get("shortDescription"),
            category: get("category"),
            vendor: get("vendor"),
            inventory_quantity: get("inventoryQuantity"),
            image_url: get("imageUrl"),
            product_url: get("productUrl"),
        })
    }
}

/// `ProductImportMeta`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductImportMeta {
    pub column_map: ProductColumnMap,
    #[serde(default = "yes")]
    pub create_missing_categories: bool,
}

fn yes() -> bool {
    true
}

/// `matchProductImportHeaders` / `suggestProductImportColumnMap`: `(field,
/// header)` in `productImportFields` order; a header is used once.
pub fn suggest_product_import_column_map(headers: &[String]) -> Vec<(&'static str, String)> {
    let mut used: Vec<&String> = Vec::new();
    let mut out = Vec::new();
    for (field, candidates) in PRODUCT_IMPORT_FIELDS.iter().zip(HEADER_CANDIDATES) {
        let found = headers.iter().find(|header| {
            !used.contains(header)
                && candidates.contains(&normalize_contact_header(header).as_str())
        });
        if let Some(header) = found {
            used.push(header);
            out.push((*field, header.clone()));
        }
    }
    out
}

/// `resolveProductImportColumnMap`: the caller's map, or the one suggested
/// from the file's headers (422 `validation` on `columnMap` without a
/// recognised name column).
pub async fn resolve_product_import_column_map(
    db: &Db,
    uploader: &Uploader,
    workspace_id: i64,
    file_id: i64,
    column_map: Option<ProductColumnMap>,
) -> Result<ProductColumnMap> {
    if let Some(map) = column_map {
        return Ok(map);
    }
    let headers = peek_import_headers(
        db,
        uploader,
        workspace_id,
        file_id,
        Some(ImportType::Products),
    )
    .await?;
    resolve_from_headers(&headers)
}

/// The suggested map of `headers`, or the 422 listing them.
pub fn resolve_from_headers(headers: &[String]) -> Result<ProductColumnMap> {
    ProductColumnMap::from_pairs(&suggest_product_import_column_map(headers)).ok_or_else(|| {
        BusinessError::validation(
            "columnMap",
            format!(
                "No column was recognised as the product name (file headers: {}). Send columnMap.",
                headers.join(", ")
            ),
        )
    })
}

/// `startProductImport` input.
#[derive(Debug, Clone)]
pub struct StartProductImport {
    pub workspace_id: i64,
    pub user_id: Option<i64>,
    pub file_id: i64,
    /// `csv` / `xlsx`; `None` = taken from the file.
    pub format: Option<String>,
    pub meta: ProductImportMeta,
}

/// `importService.startProductImport`: 404 `productImportFileNotFound`, 422
/// `productImportFileTypeInvalid` / `productImportFormatMismatch`, 409
/// `productImportAlreadyRunning`; then marks the file uploaded and writes
/// the pending `Import` row. The id of the new import.
pub async fn start_product_import(db: &Db, input: StartProductImport) -> Result<i64> {
    let meta = serde_json::to_string(&input.meta)
        .map_err(|e| BusinessError::Repo(chatbotx_db::repo::Error::msg(e.to_string())))?;
    tx::write(db, move |conn| {
        let ws = input.workspace_id;
        let Some((sub_type, file_name, mime_type, _, _)) = find_import_file(conn, ws, input.file_id)?
        else {
            return Err(BusinessError::exception(
                "Product import file not found",
                "productImportFileNotFound",
                404,
            ));
        };
        let format = resolve_import_file_format(config(ImportType::Products), &file_name, &mime_type);
        let (Some(format), Some("products")) = (format, sub_type.as_deref()) else {
            return Err(BusinessError::exception(
                "Product import file type is invalid",
                "productImportFileTypeInvalid",
                422,
            ));
        };
        if input.format.as_deref().is_some_and(|f| f != format) {
            return Err(BusinessError::exception(
                "Product import format does not match the uploaded file",
                "productImportFormatMismatch",
                422,
            ));
        }
        let active: Option<i64> = conn
            .query_row(
                r#"SELECT "id" FROM "Import" WHERE "workspaceId" = ?1 AND "type" = 'products' AND "status" IN ('pending', 'processing') LIMIT 1"#,
                [ws],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?;
        if active.is_some() {
            return Err(BusinessError::exception(
                "A product import is already running",
                "productImportAlreadyRunning",
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
            r#"INSERT INTO "Import" ("id", "workspaceId", "userId", "fileId", "type", "format", "status", "meta", "createdAt", "updatedAt")
               VALUES (?1, ?2, ?3, ?4, 'products', ?5, 'pending', ?6, ?7, ?7)"#,
            params![import_id, ws, input.user_id, input.file_id, format, meta, now],
        )
        .map_err(repo_err)?;
        Ok(import_id)
    })
    .await
}

// --- template --------------------------------------------------------------

/// `productImportTemplateFileName`.
pub fn product_import_template_file_name(locale: &str) -> String {
    format!("product-import-{locale}.xlsx")
}

/// `resolveProductImportTemplateLocale`.
pub fn resolve_product_import_template_locale(locale: &str) -> &'static str {
    if locale == "vi" { "vi" } else { "en" }
}

/// `buildProductImportTemplate(locale)`: the header row (`FIELDS`, no
/// product URL) and two example rows, bold header, columns 24 wide. The
/// labels are the `productImport.template` messages.
pub fn build_product_import_template(locale: &str) -> Vec<u8> {
    let vi = locale == "vi";
    let pick = |en: &str, vi_text: &str| Cell::Text(if vi { vi_text } else { en }.to_string());
    let labels = vec![
        pick("Product name", "Tên sản phẩm"),
        pick("SKU", "Mã sản phẩm"),
        pick("Price", "Giá bán"),
        pick("Discount", "Giảm giá"),
        pick("Short description", "Mô tả ngắn"),
        pick("Category", "Danh mục"),
        pick("Vendor", "Thương hiệu"),
        pick("Inventory quantity", "Số lượng"),
        pick("Image URL", "Hình ảnh"),
    ];
    let example = |name: Cell,
                   sku: &str,
                   price: f64,
                   discount: f64,
                   description: Cell,
                   category: Cell,
                   quantity: f64,
                   image: &str| {
        vec![
            name,
            Cell::Text(sku.into()),
            Cell::Number(price),
            Cell::Number(discount),
            description,
            category,
            pick("Example Brand", "Thương hiệu mẫu"),
            Cell::Number(quantity),
            Cell::Text(image.into()),
        ]
    };
    let rows = vec![
        labels,
        example(
            pick("Classic T-shirt", "Áo thun cổ điển"),
            "SKU-001",
            199_000.0,
            10.0,
            pick("Soft cotton T-shirt", "Áo thun cotton mềm mại"),
            pick("Clothing", "Quần áo"),
            25.0,
            "https://example.com/product-one.jpg",
        ),
        example(
            pick("Canvas tote", "Túi vải canvas"),
            "SKU-002",
            99_000.0,
            0.0,
            pick("Reusable canvas tote bag", "Túi vải canvas tái sử dụng"),
            pick("Accessories", "Phụ kiện"),
            50.0,
            "https://example.com/product-two.jpg",
        ),
    ];
    xlsx::write_workbook(if vi { "Sản phẩm" } else { "Products" }, &rows, 24)
}

// --- extractor -------------------------------------------------------------

/// `ProductImportDraft`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductDraft {
    pub source_row: u64,
    pub name: String,
    pub sku: Option<String>,
    pub price: Option<f64>,
    pub discount: Option<f64>,
    pub short_description: Option<String>,
    pub category_name: Option<String>,
    pub vendor: Option<String>,
    pub inventory_quantity: Option<f64>,
    pub image_url: Option<String>,
    pub product_url: Option<String>,
}

/// `cleanText`: a trimmed string of at most `max` UTF-16 units, `None`
/// when blank or not a string.
fn clean_text(raw: Option<&Value>, max: usize) -> Option<String> {
    let trimmed = js::trim(raw?.as_str()?);
    let text = js::slice_utf16(trimmed, 0, max.min(js::utf16_len(trimmed)));
    (!text.is_empty()).then(|| text.to_string())
}

/// `parseOptionalNumber`.
fn optional_number(raw: Option<&Value>, label: &str) -> std::result::Result<Option<f64>, String> {
    let number = match raw {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(text)) if text.is_empty() => return Ok(None),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(text)) => js::to_number(&js::trim(text).replace(',', "")),
        Some(_) => f64::NAN,
    };
    if number.is_finite() {
        Ok(Some(number))
    } else {
        Err(format!("{label} must be a number"))
    }
}

/// `parseOptionalHttpUrl`: the normalized URL (`new URL(text).toString()`).
fn optional_http_url(
    raw: Option<&Value>,
    label: &str,
) -> std::result::Result<Option<String>, String> {
    let number_text;
    let raw = match raw {
        Some(Value::Number(n)) => {
            number_text = Value::String(js::number_to_string(n.as_f64().unwrap_or(f64::NAN)));
            Some(&number_text)
        }
        other => other,
    };
    let Some(text) = clean_text(raw, 2048) else {
        return Ok(None);
    };
    match url::Url::parse(&text) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => Ok(Some(url.to_string())),
        Ok(_) => Err(format!("{label} must use HTTP or HTTPS")),
        Err(_) => Err(format!("{label} is invalid")),
    }
}

/// `extractProductDraft`: the draft of a row, or the reason it fails.
pub fn extract_product_draft(
    row: &Map<String, Value>,
    map: &ProductColumnMap,
    source_row: u64,
) -> std::result::Result<ProductDraft, String> {
    let read = |column: Option<&String>| column.and_then(|c| row.get(c));
    let Some(name) = clean_text(read(Some(&map.name)), 255) else {
        return Err("Product name is required".into());
    };
    let price = optional_number(read(map.price.as_ref()), "Price");
    let discount = optional_number(read(map.discount.as_ref()), "Discount");
    let quantity = optional_number(read(map.inventory_quantity.as_ref()), "Inventory quantity");
    let image_url = optional_http_url(read(map.image_url.as_ref()), "Image URL");
    let product_url = optional_http_url(read(map.product_url.as_ref()), "Product URL");
    let (price, discount, quantity) = (price?, discount?, quantity?);
    let (image_url, product_url) = (image_url?, product_url?);
    if price.unwrap_or(0.0) < 0.0 {
        return Err("Price cannot be negative".into());
    }
    if !(0.0..=100.0).contains(&discount.unwrap_or(0.0)) {
        return Err("Discount must be between 0 and 100".into());
    }
    if quantity.is_some_and(|q| q.fract() != 0.0 || q < 0.0) {
        return Err("Inventory quantity must be a non-negative integer".into());
    }
    Ok(ProductDraft {
        source_row,
        name,
        sku: clean_text(read(map.sku.as_ref()), 255),
        price,
        discount,
        short_description: clean_text(read(map.short_description.as_ref()), 5000),
        category_name: clean_text(read(map.category.as_ref()), 255),
        vendor: clean_text(read(map.vendor.as_ref()), 255),
        inventory_quantity: quantity,
        image_url,
        product_url,
    })
}

// --- run -------------------------------------------------------------------

fn category_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The rows of the file: CSV records as strings, XLSX cells as values.
pub(super) fn read_rows(
    format: &str,
    bytes: &[u8],
) -> std::result::Result<Vec<Map<String, Value>>, String> {
    if format == "xlsx" {
        return xlsx::read_records(bytes);
    }
    let mut records = csv::parse_records(&String::from_utf8_lossy(bytes))
        .map_err(|e| e.0)?
        .into_iter();
    let headers = records.next().unwrap_or_default();
    records
        .map(|record| {
            // `columns: true`: a record of another length is a parse error.
            if record.len() != headers.len() {
                return Err(format!(
                    "Invalid Record Length: columns length is {}, got {}",
                    headers.len(),
                    record.len()
                ));
            }
            Ok(headers
                .iter()
                .cloned()
                .zip(record.into_iter().map(Value::String))
                .collect())
        })
        .collect()
}

/// `runImportPipeline` with the products handler.
pub(super) async fn run_products(
    db: &Db,
    uploader: &Uploader,
    import_id: i64,
    row: &RunRow,
    path: &str,
) -> Result<()> {
    let meta: ProductImportMeta = match serde_json::from_str(&row.meta) {
        Ok(meta) => meta,
        Err(error) => return fail(db, import_id, &error.to_string(), None, None).await,
    };
    let config = config(ImportType::Products);
    if !config.accepted_formats.contains(&row.format.as_str()) {
        let reason = format!("{} is not supported for products imports", row.format);
        return fail(db, import_id, &reason, None, None).await;
    }
    mark_processing(db, import_id).await?;
    let ws = row.workspace_id;
    // `prepareProducts`: the categories of the store source by lower-cased name.
    let mut categories: HashMap<String, i64> = tx::read(db, move |conn| {
        let source = crate::store_source::resolve_store_source(conn, ws)?;
        Ok(
            chatbotx_db::repo::product_category::list(conn, ws, &source)?
                .into_iter()
                .map(|c| (category_key(&c.name), c.id))
                .collect(),
        )
    })
    .await?;
    let max_bytes = config.max_file_size_mb * BYTES_PER_MB;
    let too_large = format!("File exceeds {}MB limit", config.max_file_size_mb);
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
    let capture = |sample: &mut Vec<ErrorSample>, errors: Vec<ErrorSample>| {
        let room = ERROR_SAMPLE_LIMIT.saturating_sub(sample.len());
        sample.extend(errors.into_iter().take(room));
    };
    let rows = match read_rows(&row.format, &bytes) {
        Ok(rows) => rows,
        Err(error) => return fail(db, import_id, &error, Some(counters), Some(sample)).await,
    };
    let mut buffer: Vec<ProductDraft> = Vec::new();
    for raw in rows {
        if counters.processed >= MAX_ROWS {
            let reason = format!("Row limit exceeded ({MAX_ROWS})");
            return fail(db, import_id, &reason, Some(counters), Some(sample)).await;
        }
        counters.processed += 1;
        let row_number = counters.processed + 1;
        match extract_product_draft(&raw, &meta.column_map, row_number) {
            Ok(draft) => {
                buffer.push(draft);
                if buffer.len() >= BATCH_SIZE {
                    let batch = std::mem::take(&mut buffer);
                    let (success, failed, errors) =
                        process_batch(db, ws, &meta, &mut categories, batch).await?;
                    counters.success += success;
                    counters.failed += failed;
                    capture(&mut sample, errors);
                }
            }
            Err(reason) => {
                counters.failed += 1;
                capture(
                    &mut sample,
                    vec![ErrorSample {
                        row: row_number,
                        reason,
                    }],
                );
            }
        }
    }
    if !buffer.is_empty() {
        let (success, failed, errors) =
            process_batch(db, ws, &meta, &mut categories, buffer).await?;
        counters.success += success;
        counters.failed += failed;
        capture(&mut sample, errors);
    }
    complete(db, import_id, counters, sample).await
}

/// `processProductBatch`: the missing categories resolved (created when the
/// import asks), rows naming an unknown one fail, the rest inserted. An
/// insert error fails every row of the batch with its message.
async fn process_batch(
    db: &Db,
    ws: i64,
    meta: &ProductImportMeta,
    categories: &mut HashMap<String, i64>,
    rows: Vec<ProductDraft>,
) -> Result<(u64, u64, Vec<ErrorSample>)> {
    let mut missing: Vec<String> = Vec::new();
    for name in rows.iter().filter_map(|r| r.category_name.as_ref()) {
        if !categories.contains_key(&category_key(name)) && !missing.contains(name) {
            missing.push(name.clone());
        }
    }
    let create_missing = meta.create_missing_categories;
    let resolved = tx::write(db, move |conn| {
        product_category::resolve_by_names(conn, ws, &missing, create_missing)
    })
    .await?;
    categories.extend(resolved);
    let total = rows.len() as u64;
    let every_row: Vec<u64> = rows.iter().map(|r| r.source_row).collect();
    let mut errors = Vec::new();
    let mut valid = Vec::new();
    for row in rows {
        let category_id = match &row.category_name {
            Some(name) => match categories.get(&category_key(name)) {
                Some(id) => Some(*id),
                None => {
                    errors.push(ErrorSample {
                        row: row.source_row,
                        reason: format!("Unknown category: {name}"),
                    });
                    continue;
                }
            },
            None => None,
        };
        valid.push(row_insert(row, category_id));
    }
    let inserts: Vec<(ProductImportInsert, Option<String>)> = valid;
    match tx::write(db, move |conn| {
        product::create_from_import(conn, ws, inserts)
    })
    .await
    {
        Ok(inserted) => {
            let success = inserted.len() as u64;
            Ok((success, total - success, errors))
        }
        Err(error) => {
            let reason = error.message();
            tracing::warn!(%reason, "product batch insert failed");
            let all = every_row
                .into_iter()
                .map(|row| ErrorSample {
                    row,
                    reason: reason.clone(),
                })
                .collect();
            Ok((0, total, all))
        }
    }
}

/// The insert of one draft and its vendor name.
fn row_insert(
    row: ProductDraft,
    category_id: Option<i64>,
) -> (ProductImportInsert, Option<String>) {
    let insert = ProductImportInsert {
        name: row.name,
        sku: row.sku,
        price: row.price,
        discount: row.discount,
        short_description: row.short_description,
        category_id,
        // A non-negative integer (checked by the extractor).
        inventory_quantity: row.inventory_quantity.map(|q| q as i64),
        images: Some(match row.image_url {
            Some(url) => json!([{ "url": url, "type": "link" }]),
            None => json!([]),
        }),
        product_url: row.product_url,
        ..ProductImportInsert::default()
    };
    (insert, row.vendor)
}
