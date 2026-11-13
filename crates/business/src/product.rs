//! `product/service.ts` (`productService` + the variant-option, variant and
//! addon services). Template edit locks are not checked (DECISIONS, P3.6-D1).

use chatbotx_db::Db;
use chatbotx_db::repo::models::Product;
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::{new_id, product as repo, product_category};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

pub use chatbotx_db::repo::product::{ProductDetail, ProductListInput, ProductPage};

use crate::errors::{BusinessError, Result};
use crate::store_source::{external_store_managed, is_external_store_source, resolve_store_source};
use crate::{discount_code, tx, vendor};

fn not_found() -> BusinessError {
    BusinessError::not_found("Product does not exist.")
}

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn opt_text(value: &Option<String>) -> Sql {
    value.clone().map_or(Sql::Null, Sql::Text)
}

/// The product columns a write sets (without the vendor, resolved apart).
fn patch_of(data: &ProductData) -> Patch {
    let mut out: Patch = Vec::new();
    if let Some(name) = &data.name {
        out.push(("name", Sql::Text(name.clone())));
    }
    let texts = [
        ("shortDescription", &data.short_description),
        ("longDescription", &data.long_description),
        ("productUrl", &data.product_url),
        ("sku", &data.sku),
    ];
    for (column, value) in texts {
        if let Some(value) = value {
            out.push((column, opt_text(value)));
        }
    }
    for (column, value) in [
        ("price", data.price),
        ("taxes", data.taxes),
        ("discount", data.discount),
    ] {
        if let Some(value) = value {
            out.push((column, Sql::Real(value)));
        }
    }
    for (column, value) in [
        ("currency", &data.currency),
        ("inventoryPolicy", &data.inventory_policy),
        ("status", &data.status),
    ] {
        if let Some(value) = value {
            out.push((column, Sql::Text(value.clone())));
        }
    }
    if let Some(quantity) = data.inventory_quantity {
        out.push(("inventoryQuantity", Sql::Integer(quantity)));
    }
    if let Some(rank) = data.rank {
        out.push(("rank", Sql::Integer(rank)));
    }
    for (column, value) in [
        ("allowOutOfStockPurchase", data.allow_out_of_stock_purchase),
        ("isSearchable", data.is_searchable),
        ("allowSpecialRequest", data.allow_special_request),
        ("isAddonOnly", data.is_addon_only),
    ] {
        if let Some(value) = value {
            out.push((column, Sql::Integer(i64::from(value))));
        }
    }
    if let Some(images) = &data.images {
        let rows: Vec<Value> = images
            .iter()
            .map(|i| json!({ "type": i.mode, "url": i.url }))
            .collect();
        out.push(("images", Sql::Text(Value::Array(rows).to_string())));
    }
    if let Some(tags) = &data.tags {
        out.push(("tags", Sql::Text(json!(tags).to_string())));
    }
    for (column, value) in [
        ("categoryId", data.category_id),
        ("subcategoryId", data.subcategory_id),
    ] {
        if let Some(value) = value {
            out.push((column, value.map_or(Sql::Null, Sql::Integer)));
        }
    }
    out
}

/// `assertManagedHere`: no new product while the store mirrors Shopify /
/// WooCommerce; no change to a mirrored product.
fn assert_managed_here(
    conn: &Connection,
    workspace_id: i64,
    product_ids: Option<&[i64]>,
) -> Result<()> {
    match product_ids {
        None => {
            let source = resolve_store_source(conn, workspace_id)?;
            if is_external_store_source(&source) {
                return Err(external_store_managed(&source));
            }
        }
        Some(ids) => {
            if let Some(row) = repo::find_by_ids(conn, workspace_id, ids)?
                .into_iter()
                .find(|row| is_external_store_source(&row.source))
            {
                return Err(external_store_managed(&row.source));
            }
        }
    }
    Ok(())
}

/// `assertReferencesBelongToWorkspace`.
fn assert_references(
    conn: &Connection,
    workspace_id: i64,
    data: &ProductData,
    expected_parent_id: Option<Option<i64>>,
    addons: &[AddonInput],
) -> Result<()> {
    if let Some(Some(category_id)) = data.category_id
        && product_category::find(conn, workspace_id, category_id)?.is_none()
    {
        return Err(BusinessError::not_found("Product category does not exist."));
    }
    if let Some(Some(subcategory_id)) = data.subcategory_id {
        let sub = product_category::find(conn, workspace_id, subcategory_id)?
            .ok_or_else(|| BusinessError::not_found("Product sub-category does not exist."))?;
        let expected = expected_parent_id.unwrap_or(data.category_id.flatten());
        if sub.parent_id != expected {
            return Err(BusinessError::not_found(
                "Product sub-category does not belong to the selected category.",
            ));
        }
    }
    let mut wanted: Vec<&str> = Vec::new();
    for id in addons.iter().flat_map(|a| a.addon_product_ids.iter()) {
        if !wanted.contains(&id.as_str()) {
            wanted.push(id);
        }
    }
    if wanted.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = wanted.iter().filter_map(|id| id.parse().ok()).collect();
    if ids.len() != wanted.len()
        || repo::find_by_ids(conn, workspace_id, &ids)?.len() != wanted.len()
    {
        return Err(BusinessError::not_found(
            "One or more addon products do not exist.",
        ));
    }
    Ok(())
}

/// `withConsistentSubcategory`: a category change alone clears a stored
/// sub-category that does not sit under the new category.
fn with_consistent_subcategory(
    conn: &Connection,
    workspace_id: i64,
    product_id: i64,
    mut data: ProductData,
) -> Result<ProductData> {
    let (Some(category_id), None) = (data.category_id, data.subcategory_id) else {
        return Ok(data);
    };
    let Some(current) = repo::find(conn, workspace_id, product_id)? else {
        return Ok(data);
    };
    let Some(stored_sub) = current.subcategory_id else {
        return Ok(data);
    };
    if current.category_id == category_id {
        return Ok(data);
    }
    if let Some(category_id) = category_id
        && product_category::find(conn, workspace_id, stored_sub)?
            .is_some_and(|s| s.parent_id == Some(category_id))
    {
        return Ok(data);
    }
    data.subcategory_id = Some(None);
    Ok(data)
}

/// `resolveExpectedParentId`: a sub-category sent without its category is
/// checked against the stored category.
fn expected_parent_id(
    conn: &Connection,
    workspace_id: i64,
    product_id: i64,
    data: &ProductData,
) -> Result<Option<Option<i64>>> {
    if data.category_id.is_some() || !matches!(data.subcategory_id, Some(Some(_))) {
        return Ok(None);
    }
    Ok(Some(
        repo::find(conn, workspace_id, product_id)?.and_then(|p| p.category_id),
    ))
}

/// `resolveVendorId`: `Some(id)` sets the column; `None` leaves it.
fn resolve_vendor_id(
    conn: &Connection,
    workspace_id: i64,
    data: &ProductData,
) -> Result<Option<Option<i64>>> {
    if let Some(vendor_id) = data.vendor_id {
        if let Some(id) = vendor_id {
            vendor::assert_belongs_to_workspace(conn, workspace_id, id)?;
        }
        return Ok(Some(vendor_id));
    }
    if let Some(name) = &data.vendor {
        return Ok(Some(vendor::resolve_name(
            conn,
            workspace_id,
            name.as_deref(),
        )?));
    }
    Ok(None)
}

fn insert(conn: &Connection, workspace_id: i64, data: &ProductData) -> Result<Product> {
    let vendor_id = resolve_vendor_id(conn, workspace_id, data)?.flatten();
    let mut values = patch_of(data);
    values.push(("workspaceId", Sql::Integer(workspace_id)));
    values.push(("vendorId", vendor_id.map_or(Sql::Null, Sql::Integer)));
    Ok(repo::create(conn, &values)?)
}

fn apply_patch(
    conn: &Connection,
    workspace_id: i64,
    product_id: i64,
    data: &ProductData,
) -> Result<()> {
    let mut values = patch_of(data);
    if let Some(vendor_id) = resolve_vendor_id(conn, workspace_id, data)? {
        values.push(("vendorId", vendor_id.map_or(Sql::Null, Sql::Integer)));
    }
    repo::update(conn, workspace_id, product_id, &values)?.ok_or_else(not_found)?;
    Ok(())
}

/// The variant-option, variant and addon `createBulk`s.
fn insert_children(conn: &Connection, product_id: i64, data: &FullData) -> Result<()> {
    for option in &data.variant_options {
        conn.execute(
            r#"INSERT INTO "ProductVariantOption" ("id", "productId", "name", "values", "position") VALUES (?1, ?2, ?3, ?4, ?5)"#,
            params![new_id(), product_id, option.name, json!(option.values).to_string(), option.position.round() as i64],
        )
        .map_err(db_error)?;
    }
    for variant in &data.variants {
        conn.execute(
            r#"INSERT INTO "ProductVariant" ("id", "productId", "combination", "price", "isEnabled") VALUES (?1, ?2, ?3, ?4, ?5)"#,
            params![new_id(), product_id, Value::Object(variant.combination.clone()).to_string(), variant.price, variant.is_enabled],
        )
        .map_err(db_error)?;
    }
    for addon in &data.addons {
        conn.execute(
            r#"INSERT INTO "ProductAddon" ("id", "productId", "name", "maxSelections", "addonProductIds") VALUES (?1, ?2, ?3, ?4, ?5)"#,
            params![new_id(), product_id, addon.name, addon.max_selections, json!(addon.addon_product_ids).to_string()],
        )
        .map_err(db_error)?;
    }
    Ok(())
}

/// `ProductImageInput` (`mode` is stored as the image `type`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInput {
    pub mode: String,
    pub url: String,
}

/// `ProductWriteData` (all optional: the patch form). `Some(None)` = null.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductData {
    pub name: Option<String>,
    pub short_description: Option<Option<String>>,
    pub long_description: Option<Option<String>>,
    pub price: Option<f64>,
    pub taxes: Option<f64>,
    pub discount: Option<f64>,
    pub currency: Option<String>,
    pub product_url: Option<Option<String>>,
    pub sku: Option<Option<String>>,
    pub inventory_policy: Option<String>,
    pub inventory_quantity: Option<i64>,
    pub allow_out_of_stock_purchase: Option<bool>,
    pub images: Option<Vec<ImageInput>>,
    pub tags: Option<Vec<String>>,
    pub vendor_id: Option<Option<i64>>,
    pub vendor: Option<Option<String>>,
    pub rank: Option<i64>,
    pub category_id: Option<Option<i64>>,
    pub subcategory_id: Option<Option<i64>>,
    pub status: Option<String>,
    pub is_searchable: Option<bool>,
    pub allow_special_request: Option<bool>,
    pub is_addon_only: Option<bool>,
}

/// `ProductVariantOptionInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantOptionInput {
    pub name: String,
    pub values: Vec<String>,
    pub position: f64,
}

/// `ProductVariantInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantInput {
    pub combination: Map<String, Value>,
    pub price: f64,
    pub is_enabled: bool,
}

/// `ProductAddonInput` (ids as sent: strings).
#[derive(Debug, Clone, PartialEq)]
pub struct AddonInput {
    pub name: String,
    pub max_selections: i64,
    pub addon_product_ids: Vec<String>,
}

/// `ProductFullWriteData`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FullData {
    pub product: ProductData,
    pub variant_options: Vec<VariantOptionInput>,
    pub variants: Vec<VariantInput>,
    pub addons: Vec<AddonInput>,
}

/// `create`: checks the category references, then inserts.
pub async fn create(db: &Db, workspace_id: i64, data: ProductData) -> Result<Product> {
    tx::write(db, move |conn| {
        assert_references(conn, workspace_id, &data, None, &[])?;
        insert(conn, workspace_id, &data)
    })
    .await
}

/// `createFull`: product, variant options, variants and addons in one transaction.
pub async fn create_full(db: &Db, workspace_id: i64, data: FullData) -> Result<Product> {
    tx::write(db, move |conn| {
        assert_managed_here(conn, workspace_id, None)?;
        assert_references(conn, workspace_id, &data.product, None, &data.addons)?;
        let product = insert(conn, workspace_id, &data.product)?;
        insert_children(conn, product.id, &data)?;
        Ok(product)
    })
    .await
}

/// `update`: a patch (only the sent fields).
pub async fn update(db: &Db, workspace_id: i64, product_id: i64, data: ProductData) -> Result<()> {
    tx::write(db, move |conn| {
        assert_managed_here(conn, workspace_id, Some(&[product_id]))?;
        let data = with_consistent_subcategory(conn, workspace_id, product_id, data)?;
        let expected = expected_parent_id(conn, workspace_id, product_id, &data)?;
        assert_references(conn, workspace_id, &data, expected, &[])?;
        apply_patch(conn, workspace_id, product_id, &data)
    })
    .await
}

/// `updateFull`: the patch, then the child rows replaced.
pub async fn update_full(
    db: &Db,
    workspace_id: i64,
    product_id: i64,
    data: FullData,
) -> Result<()> {
    tx::write(db, move |conn| {
        assert_managed_here(conn, workspace_id, Some(&[product_id]))?;
        let product =
            with_consistent_subcategory(conn, workspace_id, product_id, data.product.clone())?;
        let expected = expected_parent_id(conn, workspace_id, product_id, &product)?;
        assert_references(conn, workspace_id, &product, expected, &data.addons)?;
        apply_patch(conn, workspace_id, product_id, &product)?;
        for table in ["ProductVariantOption", "ProductVariant", "ProductAddon"] {
            conn.execute(
                &format!("DELETE FROM \"{table}\" WHERE \"productId\" = ?1"),
                [product_id],
            )
            .map_err(db_error)?;
        }
        insert_children(conn, product_id, &data)
    })
    .await
}

/// `delete`: drops the ids from discount codes, then deletes (other
/// workspaces' ids are ignored).
pub async fn delete(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    tx::write(db, move |conn| {
        assert_managed_here(conn, workspace_id, Some(&ids))?;
        discount_code::remove_product_ids(conn, workspace_id, &ids)?;
        repo::delete_by_ids(conn, workspace_id, &ids)?;
        Ok(())
    })
    .await
}

/// `findById` (`Product does not exist.`).
pub async fn find_by_id(db: &Db, workspace_id: i64, id: i64) -> Result<ProductDetail> {
    tx::read(db, move |conn| {
        repo::find_detail(conn, id, workspace_id)?.ok_or_else(not_found)
    })
    .await
}

/// `list`: the store source of the workspace unless given.
pub async fn list(db: &Db, input: ProductListInput) -> Result<ProductPage> {
    tx::read(db, move |conn| {
        let mut input = input;
        if input.source.is_none() {
            input.source = Some(resolve_store_source(conn, input.workspace_id)?);
        }
        Ok(repo::list(conn, &input)?)
    })
    .await
}
