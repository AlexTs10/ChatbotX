//! `handlers/meta-catalog/import-products.ts`, `apis/products.ts` and
//! `metaCatalogImportService.importPage`.

use std::collections::HashMap;

use chatbotx_business::meta_catalog::{process_encryption, resolve_auth};
use chatbotx_business::product_category::{self, CategoryPath};
use chatbotx_business::{tx, vendor};
use chatbotx_db::Db;
use chatbotx_db::repo::meta_catalog_item::{self, CatalogLinkItem, CatalogScope};
use chatbotx_db::repo::models::IntegrationMetaCatalog;
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::product::{self, ProductImportInsert};
use chatbotx_db::repo::template_installed_resource::{TemplateResourcePermission, find_blocking};
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::params;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use super::items::to_imported_meta_product;
use super::{
    GENERIC_IMPORT_FAILURE, McError, McResult, StoredAuth, claim_run, fail_run, graph, log_failure,
    mark_invalid, outcome, public_text,
};
use crate::{Deps, db_fail, id_of};

const PAGE_SIZE: &str = "100";
const MAX_PAGES: usize = 100;
const MAX_REPORTED_REASONS: usize = 3;
const UNEXPECTED_SHAPE_REASON: &str = "Meta returned a product in an unexpected shape";
const PRODUCT_FIELDS: &str = "id,retailer_id,name,description,short_description,price,sale_price,currency,image_url,additional_image_urls,images,image_cdn_urls,additional_image_cdn_urls,availability,brand,product_type,quantity_to_sell_on_facebook,custom_label_0,custom_label_1,custom_label_2,custom_label_3,custom_label_4,url";

const STRING_FIELDS: &[&str] = &[
    "id",
    "retailer_id",
    "name",
    "description",
    "short_description",
    "currency",
    "image_url",
    "availability",
    "brand",
    "product_type",
    "custom_label_0",
    "custom_label_1",
    "custom_label_2",
    "custom_label_3",
    "custom_label_4",
    "url",
];

/// `metaCatalogRemoteProductSchema.safeParse(product).success`.
fn is_remote_product(product: &Value) -> bool {
    let Some(map) = product.as_object() else {
        return false;
    };
    let ok = |key: &str, test: fn(&Value) -> bool| map.get(key).is_none_or(test);
    STRING_FIELDS.iter().all(|key| ok(key, Value::is_string))
        && ["price", "sale_price", "quantity_to_sell_on_facebook"]
            .iter()
            .all(|key| ok(key, |v| v.is_number() || v.is_string()))
        && ok("additional_image_urls", |v| {
            v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
        })
}

/// `MetaCatalogProductPage`.
struct ProductPage {
    products: Vec<Value>,
    invalid_count: usize,
    next_cursor: Option<String>,
}

/// `getCatalogProductsPage`.
async fn get_products_page(
    auth: &StoredAuth,
    catalog_id: &str,
    after: Option<&str>,
) -> McResult<ProductPage> {
    let mut query = vec![("fields", PRODUCT_FIELDS), ("limit", PAGE_SIZE)];
    if let Some(after) = after {
        query.push(("after", after));
    }
    let (body, _) = graph(auth, &format!("{catalog_id}/products"), &query, None).await?;
    let (products, invalid): (Vec<Value>, Vec<Value>) = body
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .partition(is_remote_product);
    let paging = body.get("paging");
    let next_cursor = paging
        .filter(|p| {
            p.get("next")
                .and_then(Value::as_str)
                .is_some_and(|n| !n.is_empty())
        })
        .and_then(|p| p.pointer("/cursors/after"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(ProductPage {
        products,
        invalid_count: invalid.len(),
        next_cursor,
    })
}

/// `createImportTally`.
#[derive(Default)]
struct Tally {
    total: i64,
    imported: i64,
    failed: i64,
    /// First-seen order (a `Map`).
    reasons: Vec<(String, i64)>,
}

impl Tally {
    fn reject(&mut self, reason: &str, count: i64) {
        self.failed += count;
        match self.reasons.iter_mut().find(|(r, _)| r == reason) {
            Some(entry) => entry.1 += count,
            None => self.reasons.push((reason.to_owned(), count)),
        }
    }

    fn read_page(&mut self, page: &ProductPage) -> Vec<Value> {
        self.total += (page.products.len() + page.invalid_count) as i64;
        if page.invalid_count > 0 {
            self.reject(UNEXPECTED_SHAPE_REASON, page.invalid_count as i64);
        }
        let mut out = Vec::new();
        for product in &page.products {
            match to_imported_meta_product(product) {
                Ok(product) => out.push(product),
                Err(reason) => self.reject(&reason, 1),
            }
        }
        out
    }

    /// `summarize` (`describeFailures`: stable sort by count, top 3).
    fn summarize(&self) -> Option<String> {
        if self.failed == 0 {
            return None;
        }
        let mut ranked = self.reasons.clone();
        ranked.sort_by_key(|r| std::cmp::Reverse(r.1));
        let shown: Vec<String> = ranked
            .iter()
            .take(MAX_REPORTED_REASONS)
            .map(|(reason, count)| format!("{reason} ({count})"))
            .collect();
        let hidden = ranked.len() - shown.len();
        let described = if hidden > 0 {
            format!("{}; and {hidden} other reason(s)", shown.join("; "))
        } else {
            shown.join("; ")
        };
        Some(format!(
            "{} Meta products could not be imported: {described}",
            self.failed
        ))
    }
}

fn opt_str(product: &Value, key: &str) -> Option<Option<String>> {
    product.get(key).map(|v| v.as_str().map(str::to_owned))
}

/// `toProductValues` as a patch (`include_sku` for a new product).
fn product_values(
    product: &Value,
    paths: &HashMap<String, product_category::ResolvedPath>,
    vendors: &HashMap<String, i64>,
    include_sku: bool,
) -> Patch {
    let text = |v: Option<String>| v.map_or(Sql::Null, Sql::Text);
    let mut patch: Patch = vec![(
        "name",
        Sql::Text(product["name"].as_str().unwrap_or("").to_owned()),
    )];
    if include_sku {
        patch.push(("sku", text(opt_str(product, "sku").flatten())));
    }
    for (key, column) in [("price", "price"), ("discount", "discount")] {
        if let Some(n) = product.get(key).and_then(Value::as_f64) {
            patch.push((column, Sql::Real(n)));
        }
    }
    if let Some(currency) = product
        .get("currency")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
    {
        patch.push(("currency", Sql::Text(currency.to_owned())));
    }
    for (key, column) in [
        ("productUrl", "productUrl"),
        ("shortDescription", "shortDescription"),
        ("longDescription", "longDescription"),
    ] {
        if let Some(value) = opt_str(product, key) {
            patch.push((column, text(value)));
        }
    }
    match product.get("categoryName") {
        Some(Value::Null) => {
            patch.push(("categoryId", Sql::Null));
            patch.push(("subcategoryId", Sql::Null));
        }
        Some(Value::String(name)) if !name.is_empty() => {
            let key = product_category::path_key(&category_path(product).expect("a name"));
            if let Some(path) = paths.get(&key) {
                patch.push(("categoryId", Sql::Integer(path.category_id)));
                patch.push((
                    "subcategoryId",
                    path.subcategory_id.map_or(Sql::Null, Sql::Integer),
                ));
            }
        }
        _ => {}
    }
    if let Some(tags) = product.get("tags") {
        patch.push(("tags", Sql::Text(tags.to_string())));
    }
    if let Some(vendor) = product.get("vendor") {
        let id = vendor
            .as_str()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .and_then(|n| vendors.get(&n.to_lowercase()).copied());
        patch.push(("vendorId", id.map_or(Sql::Null, Sql::Integer)));
    }
    if let Some(n) = product.get("inventoryQuantity").and_then(Value::as_i64) {
        patch.push(("inventoryQuantity", Sql::Integer(n)));
    }
    if let Some(policy) = product.get("inventoryPolicy").and_then(Value::as_str) {
        patch.push(("inventoryPolicy", Sql::Text(policy.to_owned())));
    }
    if let Some(allow) = product
        .get("allowOutOfStockPurchase")
        .and_then(Value::as_bool)
    {
        patch.push(("allowOutOfStockPurchase", Sql::Integer(allow.into())));
    }
    if let Some(images) = product.get("images") {
        patch.push(("images", Sql::Text(images.to_string())));
    }
    if let Some(status) = product.get("status").and_then(Value::as_str) {
        patch.push(("status", Sql::Text(status.to_owned())));
    }
    patch
}

fn category_path(product: &Value) -> Option<CategoryPath> {
    Some(CategoryPath {
        category_name: product.get("categoryName")?.as_str()?.to_owned(),
        subcategory_name: product
            .get("subcategoryName")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// The `createFromImport` row of a missing product.
fn insert_of(product: &Value, patch: &Patch) -> ProductImportInsert {
    let get = |column: &str| patch.iter().find(|(c, _)| *c == column).map(|(_, v)| v);
    let text = |column: &str| match get(column) {
        Some(Sql::Text(t)) => Some(t.clone()),
        _ => None,
    };
    let int = |column: &str| match get(column) {
        Some(Sql::Integer(n)) => Some(*n),
        _ => None,
    };
    ProductImportInsert {
        name: product["name"].as_str().unwrap_or("").to_owned(),
        sku: text("sku"),
        price: product.get("price").and_then(Value::as_f64),
        discount: product.get("discount").and_then(Value::as_f64),
        currency: text("currency"),
        product_url: text("productUrl"),
        short_description: text("shortDescription"),
        long_description: text("longDescription"),
        category_id: int("categoryId"),
        subcategory_id: int("subcategoryId"),
        tags: product
            .get("tags")
            .and_then(Value::as_array)
            .map(|t| {
                t.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        vendor_id: int("vendorId"),
        inventory_quantity: int("inventoryQuantity"),
        inventory_policy: text("inventoryPolicy"),
        allow_out_of_stock_purchase: product
            .get("allowOutOfStockPurchase")
            .and_then(Value::as_bool),
        images: product.get("images").cloned(),
        status: text("status"),
    }
}

fn import_error(message: String) -> chatbotx_business::BusinessError {
    chatbotx_business::BusinessError::Repo(repo::Error::msg(message))
}

/// `metaCatalogImportService.importPage`: `(imported, existing)`.
async fn import_page(
    db: &Db,
    workspace_id: i64,
    scope: CatalogScope,
    products: Vec<Value>,
) -> McResult<(i64, i64)> {
    // `new Map(products by retailerId)`: first position, last value.
    let mut unique: Vec<Value> = Vec::new();
    for product in &products {
        let id = &product["retailerId"];
        match unique.iter_mut().find(|p| &p["retailerId"] == id) {
            Some(existing) => *existing = product.clone(),
            None => unique.push(product.clone()),
        }
    }
    let duplicates = (products.len() - unique.len()) as i64;
    if unique.is_empty() {
        return Ok((0, 0));
    }
    let rid = |p: &Value| p["retailerId"].as_str().unwrap_or("").to_owned();
    Ok(tx::write(db, move |tx| {
        let retailer_ids: Vec<String> = unique.iter().map(rid).collect();
        let refs: Vec<&str> = retailer_ids.iter().map(String::as_str).collect();
        meta_catalog_item::lock_catalog_assignments(tx, &scope)?;
        let links: HashMap<String, i64> = meta_catalog_item::find_by_retailer_ids(tx, &scope, &refs)?
            .into_iter()
            .map(|l| (l.retailer_id, l.product_id))
            .collect();
        let unlinked: Vec<&Value> = unique.iter().filter(|p| !links.contains_key(&rid(p))).collect();
        let unlinked_ids: Vec<String> = unlinked.iter().map(|p| rid(p)).collect();
        let unlinked_refs: Vec<&str> = unlinked_ids.iter().map(String::as_str).collect();
        let matches = product::find_by_import_identifiers(tx, workspace_id, &unlinked_refs)?;
        let mut matched: Vec<(&Value, i64)> = Vec::new();
        for p in &unlinked {
            let retailer_id = rid(p);
            let mut candidates: Vec<i64> = matches
                .iter()
                .filter(|m| {
                    m.id.to_string() == retailer_id
                        || m.sku.as_deref().map(str::trim) == Some(retailer_id.as_str())
                })
                .map(|m| m.id)
                .collect();
            candidates.dedup();
            if candidates.len() > 1 {
                return Err(import_error(format!(
                    "Meta Catalog retailer ID \"{retailer_id}\" matches multiple product identifiers"
                )));
            }
            if let Some(id) = candidates.first() {
                matched.push((p, *id));
            }
        }
        let missing: Vec<&Value> = unlinked
            .iter()
            .filter(|p| !matched.iter().any(|(m, _)| rid(m) == rid(p)))
            .copied()
            .collect();
        let existing: Vec<(&Value, i64)> = unique
            .iter()
            .filter_map(|p| {
                links
                    .get(&rid(p))
                    .map(|id| (p, *id))
                    .or_else(|| matched.iter().find(|(m, _)| rid(m) == rid(p)).copied())
            })
            .collect();
        let mut owner: HashMap<i64, String> = HashMap::new();
        for (p, id) in &existing {
            if let Some(assigned) = owner.get(id)
                && *assigned != rid(p)
            {
                return Err(import_error(format!(
                    "Meta Catalog items \"{assigned}\" and \"{}\" match the same product",
                    rid(p)
                )));
            }
            owner.insert(*id, rid(p));
        }
        let paths: Vec<CategoryPath> = unique.iter().filter_map(category_path).collect();
        let paths = product_category::resolve_paths(tx, workspace_id, &paths, true)?;
        let vendor_names: Vec<String> = unique
            .iter()
            .filter_map(|p| p.get("vendor").and_then(Value::as_str).map(str::to_owned))
            .collect();
        let vendors = vendor::resolve_by_names(tx, workspace_id, &vendor_names, true)?;
        let existing_ids: Vec<i64> = existing.iter().map(|(_, id)| *id).collect();
        let locked = find_blocking(
            tx,
            workspace_id,
            "product",
            &existing_ids,
            TemplateResourcePermission::AllowEdit,
            &[],
        )?;
        for (p, id) in &existing {
            if locked.contains(id) {
                tracing::info!(workspace_id, product_id = id, "Meta Catalog import skipped a template-locked product");
                continue;
            }
            let values = product_values(p, &paths, &vendors, false);
            if product::update(tx, workspace_id, *id, &values)?.is_none() {
                return Err(import_error(format!(
                    "Meta Catalog retailer ID \"{}\" points to a missing product",
                    rid(p)
                )));
            }
        }
        let inserts: Vec<ProductImportInsert> = missing
            .iter()
            .map(|p| insert_of(p, &product_values(p, &paths, &vendors, true)))
            .collect();
        let rows = if inserts.is_empty() {
            Vec::new()
        } else {
            product::create_from_import(tx, workspace_id, &inserts)?
        };
        let mut link_items: Vec<CatalogLinkItem> = matched
            .iter()
            .map(|(p, id)| CatalogLinkItem { product_id: *id, retailer_id: rid(p), fingerprint: None })
            .collect();
        link_items.extend(rows.iter().zip(&missing).map(|(row, p)| CatalogLinkItem {
            product_id: row.id,
            retailer_id: rid(p),
            fingerprint: None,
        }));
        meta_catalog_item::link_imported(tx, &scope, &link_items)?;
        Ok((rows.len() as i64, existing.len() as i64 + duplicates))
    })
    .await?)
}

/// `integrationMetaCatalogService.claimImport`.
fn claim_import(
    conn: &chatbotx_db::rusqlite::Connection,
    connection_id: i64,
    workspace_id: i64,
) -> McResult<Option<IntegrationMetaCatalog>> {
    Ok(repo::returning::<IntegrationMetaCatalog>(
        conn,
        r#"UPDATE "IntegrationMetaCatalog" SET "importStatus" = 'running', "updatedAt" = ?3
           WHERE "id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL AND "importStatus" = 'queued'"#,
        &[&connection_id, &workspace_id, &now_ms()],
    )?
    .into_iter()
    .next())
}

/// `reportProgress` (`updateImportProgress` + `recordImportProgress`).
async fn report_progress(
    db: &Db,
    connection_id: i64,
    run_id: Option<i64>,
    t: (i64, i64, i64),
) -> McResult<()> {
    db.write(move |tx| {
        let now = now_ms();
        tx.execute(
            r#"UPDATE "IntegrationMetaCatalog" SET "importTotalCount" = ?2, "importedCount" = ?3, "importFailedCount" = ?4, "updatedAt" = ?5
               WHERE "id" = ?1 AND "deletedAt" IS NULL AND "importStatus" = 'running'"#,
            params![connection_id, t.0, t.1, t.2, now],
        )?;
        if let Some(run_id) = run_id {
            tx.execute(
                r#"UPDATE "MetaCatalogSyncRun" SET "totalCount" = ?2, "succeededCount" = ?3, "failedCount" = ?4, "updatedAt" = ?5
                   WHERE "id" = ?1 AND "status" = 'running'"#,
                params![run_id, t.0, t.1, t.2, now],
            )?;
        }
        Ok(())
    })
    .await?;
    Ok(())
}

/// `finishImport` (`completeImport` on the connection and the run).
async fn finish_import(
    db: &Db,
    connection_id: i64,
    run_id: Option<i64>,
    t: (i64, i64, i64),
    error: Option<String>,
) -> McResult<()> {
    let status = outcome(t.1, t.2, 0);
    let import_error = error
        .as_deref()
        .map(|e| public_text(e, GENERIC_IMPORT_FAILURE));
    let run_error = error
        .as_deref()
        .map(|e| public_text(e, super::GENERIC_SYNC_FAILURE));
    db.write(move |tx| {
        let now = now_ms();
        tx.execute(
            r#"UPDATE "IntegrationMetaCatalog" SET "importStatus" = ?2, "importTotalCount" = ?3, "importedCount" = ?4,
                 "importFailedCount" = ?5, "importError" = ?6, "lastImportedAt" = ?7, "updatedAt" = ?7
               WHERE "id" = ?1 AND "deletedAt" IS NULL AND "importStatus" = 'running'"#,
            params![connection_id, status, t.0, t.1, t.2, import_error, now],
        )?;
        if let Some(run_id) = run_id {
            tx.execute(
                r#"UPDATE "MetaCatalogSyncRun" SET "status" = ?2, "totalCount" = ?3, "succeededCount" = ?4,
                     "failedCount" = ?5, "error" = ?6, "finishedAt" = ?7, "updatedAt" = ?7
                   WHERE "id" = ?1 AND "status" = 'running'"#,
                params![run_id, status, t.0, t.1, t.2, run_error, now],
            )?;
        }
        Ok(())
    })
    .await?;
    Ok(())
}

/// `integrationMetaCatalogService.failImport(id, error)`.
async fn fail_import(db: &Db, connection_id: i64, error: &McError) -> McResult<()> {
    let message = super::to_public_error_message(error, GENERIC_IMPORT_FAILURE);
    db.write(move |tx| {
        let now = now_ms();
        tx.execute(
            r#"UPDATE "IntegrationMetaCatalog" SET "importStatus" = 'failed', "importError" = ?2, "lastImportedAt" = ?3, "updatedAt" = ?3
               WHERE "id" = ?1 AND "deletedAt" IS NULL AND "importStatus" IN ('queued', 'running')"#,
            params![connection_id, message, now],
        )?;
        Ok(())
    })
    .await?;
    Ok(())
}

/// `importMetaCatalogProducts`.
pub async fn handle(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let db = &deps.db;
    let workspace_id =
        id_of(&data["workspaceId"]).ok_or_else(|| crate::fail("workspaceId is required"))?;
    let connection_id = id_of(&data["integrationMetaCatalogId"])
        .ok_or_else(|| crate::fail("integrationMetaCatalogId is required"))?;
    let run_id = id_of(&data["runId"]);
    let run = match run_id {
        Some(run_id) => {
            let run = db
                .write(move |tx| claim_run(tx, run_id, workspace_id, None).map_err(to_db))
                .await
                .map_err(db_fail)?;
            if run.is_none() {
                return Ok(());
            }
            run
        }
        None => None,
    };
    let connection = db
        .write(move |tx| claim_import(tx, connection_id, workspace_id).map_err(to_db))
        .await
        .map_err(db_fail)?;
    let Some(connection) = connection else {
        if let Some(run_id) = run_id {
            // A plain sentence: `fail` stores the generic text.
            fail_run(
                db,
                run_id,
                &McError::internal("Another Meta Catalog import was already running"),
            )
            .await
            .map_err(job_err)?;
        }
        return Ok(());
    };
    let mut tally = Tally::default();
    let mut page_index = 0usize;
    let mut catalog_id: Option<String> = None;
    let result: McResult<()> = async {
        if run.as_ref().is_some_and(|r| r.integration_meta_catalog_id != connection.id) {
            return Err(McError::internal("Meta Catalog import run does not match its connection"));
        }
        catalog_id = run.as_ref().and_then(|r| r.catalog_id.clone()).or(connection.catalog_id.clone());
        let Some(catalog) = catalog_id.clone() else {
            return Err(McError::internal("Meta Catalog is not selected"));
        };
        let auth = resolve_auth(db, process_encryption(), connection.id).await?;
        let mut after: Option<String> = None;
        while page_index < MAX_PAGES {
            let page = get_products_page(&auth, &catalog, after.as_deref()).await?;
            let products = tally.read_page(&page);
            let scope = CatalogScope { integration_meta_catalog_id: connection.id, catalog_id: catalog.clone() };
            let (imported, existing) = import_page(db, workspace_id, scope, products).await?;
            tally.imported += imported + existing;
            let counts = (tally.total, tally.imported, tally.failed);
            report_progress(db, connection.id, run_id, counts).await?;
            after = page.next_cursor;
            if after.is_none() {
                if tally.failed > 0 {
                    tracing::warn!(connection_id = connection.id, failed = tally.failed, reasons = ?tally.reasons, "Meta Catalog import skipped products");
                }
                let summary = tally.summarize();
                return finish_import(db, connection.id, run_id, counts, summary).await;
            }
            page_index += 1;
        }
        let reason = format!("Meta Catalog exceeds the {MAX_PAGES}-page import limit");
        tally.reject(&reason, 1);
        finish_import(db, connection.id, run_id, (tally.total, tally.imported, tally.failed), Some(reason)).await
    }
    .await;
    let Err(error) = result else {
        return Ok(());
    };
    log_failure(
        &error,
        "Meta Catalog import failed",
        json!({ "connectionId": connection.id, "workspaceId": workspace_id, "runId": run_id, "catalogId": catalog_id, "pageIndex": page_index }),
    );
    if error.is_invalid_token() {
        mark_invalid(db, workspace_id).await.map_err(job_err)?;
    }
    fail_import(db, connection.id, &error)
        .await
        .map_err(job_err)?;
    if let Some(run_id) = run_id {
        fail_run(db, run_id, &error).await.map_err(job_err)?;
    }
    super::log_provider_error(deps, workspace_id, &error).await;
    Ok(())
}

pub(super) fn to_db(error: McError) -> chatbotx_db::DbError {
    chatbotx_db::DbError::other(error.message().to_owned())
}

pub(super) fn job_err(error: McError) -> JobError {
    crate::fail(error.message().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_remote_product_shape() {
        assert!(is_remote_product(
            &json!({ "retailer_id": "r", "price": 1, "images": [{}] })
        ));
        assert!(!is_remote_product(&json!({ "retailer_id": 1 })));
        assert!(!is_remote_product(&json!({ "additional_image_urls": [1] })));
        assert!(!is_remote_product(&json!("x")));
    }

    #[test]
    fn summarizes_the_top_three_failure_reasons() {
        let mut tally = Tally::default();
        for (reason, count) in [("a", 1), ("b", 3), ("c", 2), ("d", 1)] {
            tally.reject(reason, count);
        }
        assert_eq!(
            tally.summarize().unwrap(),
            "7 Meta products could not be imported: b (3); c (2); a (1); and 1 other reason(s)"
        );
        assert_eq!(Tally::default().summarize(), None);
    }
}
