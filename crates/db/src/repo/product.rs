//! Port of `repositories/product/repository.ts` (P3.3). `FOR UPDATE` (`lockForStock`) is a
//! plain read under the one SQLite writer. jsonb `?|` = `EXISTS (json_each ∩ json_each)`.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{Product, ProductAddon, ProductCategory, ProductVariant, ProductVariantOption};
use super::patch::{Patch, insert_clause, set_clause};
use super::sql::Sql;
use super::utils::{
    PaginationInput, SortItem, like_contains, order_by_sql, parse_order_by, parse_pagination,
};
use super::{Json, Model, Result, id_list, new_id, now_ms, text_list};

/// Default `StoreSource`.
pub const DEFAULT_SOURCE: &str = "chatbotx";

/// `ProductListInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductListInput {
    pub workspace_id: i64,
    pub source: Option<String>,
    pub name: Option<String>,
    pub category_id: Option<i64>,
    pub ids: Option<Vec<i64>>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// One row of [`list`].
#[derive(Debug, Clone, PartialEq)]
pub struct ProductListRow {
    pub product: Product,
    pub category: Option<String>,
    pub subcategory: Option<String>,
    pub vendor: Option<String>,
    pub variant_count: i64,
    pub variant_inventory_total: Option<i64>,
}

/// Result of [`list`].
#[derive(Debug, Clone, PartialEq)]
pub struct ProductPage {
    pub data: Vec<ProductListRow>,
    pub page_count: i64,
}

/// Result of [`find_detail`].
#[derive(Debug, Clone, PartialEq)]
pub struct ProductDetail {
    pub product: Product,
    pub category: Option<String>,
    pub subcategory: Option<String>,
    pub vendor: Option<String>,
    pub variant_options: Vec<ProductVariantOption>,
    pub variants: Vec<ProductVariant>,
    pub addons: Vec<ProductAddon>,
}

/// One row of [`list_for_catalog_sync`].
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogSyncProduct {
    pub product: Product,
    pub category: Option<ProductCategory>,
    pub subcategory: Option<ProductCategory>,
    pub vendor: Option<String>,
    pub variants: Vec<ProductVariant>,
}

/// A shop product row (`listForShop` / `listRelatedForShop`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShopProduct {
    pub product: Product,
    pub vendor: Option<String>,
    pub vendor_discount_percent: i64,
    pub has_variants: bool,
    pub variant_inventory_quantities: Vec<Option<i64>>,
}

/// Result of [`list_for_shop`].
#[derive(Debug, Clone, PartialEq)]
pub struct ShopPage {
    pub data: Vec<ShopProduct>,
    pub page_count: i64,
}

/// Input of [`list_for_shop`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShopListInput {
    pub workspace_id: i64,
    pub source: Option<String>,
    pub keyword: Option<String>,
    pub category_id: Option<i64>,
    pub vendor_id: Option<i64>,
    pub product_ids: Option<Vec<i64>>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// A product of [`find_for_order`] with its variants and vendor.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderProduct {
    pub product: Product,
    pub variants: Vec<ProductVariant>,
    /// `(name, isActive, discountPercent, minimumOrderValue, openHoursId)`.
    pub vendor: Option<(String, bool, i64, i64, Option<i64>)>,
}

/// One row of [`lock_for_stock`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockRow {
    pub id: i64,
    pub inventory_policy: String,
    pub inventory_quantity: i64,
    pub allow_out_of_stock_purchase: bool,
}

/// `ProductImportInsert`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductImportInsert {
    pub name: String,
    pub sku: Option<String>,
    pub price: Option<f64>,
    pub discount: Option<f64>,
    pub currency: Option<String>,
    pub product_url: Option<String>,
    pub short_description: Option<String>,
    pub long_description: Option<String>,
    pub category_id: Option<i64>,
    pub subcategory_id: Option<i64>,
    pub tags: Vec<String>,
    pub vendor_id: Option<i64>,
    pub inventory_quantity: Option<i64>,
    pub inventory_policy: Option<String>,
    pub allow_out_of_stock_purchase: Option<bool>,
    /// `[{ url, type }]`.
    pub images: Option<serde_json::Value>,
    pub status: Option<String>,
}

fn in_category(category_id: Option<i64>, alias: &str) -> Option<Sql> {
    category_id.map(|id| {
        let mut s = Sql::bind(format!("{alias}\"categoryId\" = "), id);
        s.push(&format!(" OR {alias}\"subcategoryId\" = "));
        s.push_bind(id);
        s
    })
}

fn ids_in(col: &str, ids: Option<&[i64]>) -> Option<Sql> {
    ids.filter(|i| !i.is_empty()).map(|i| {
        Sql::bind(
            format!("{col} IN (SELECT value FROM json_each("),
            id_list(i),
        )
        .closed("))")
    })
}

fn list_where(input: &ProductListInput) -> Sql {
    Sql::and([
        Some(Sql::bind("p.\"workspaceId\" = ", input.workspace_id)),
        Some(Sql::bind(
            "p.\"source\" = ",
            input
                .source
                .clone()
                .unwrap_or_else(|| DEFAULT_SOURCE.to_owned()),
        )),
        input
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| Sql::bind("ilike(p.\"name\", ", like_contains(n)).closed(")")),
        ids_in("p.\"id\"", input.ids.as_deref()),
        in_category(input.category_id, "p."),
    ])
}

fn category_name(conn: &Connection, id: Option<i64>) -> Result<Option<String>> {
    let Some(id) = id else { return Ok(None) };
    let row: Option<ProductCategory> = super::find_one(conn, "\"id\" = ?1", &[&id])?;
    Ok(row.map(|c| c.name))
}

fn vendor_name(conn: &Connection, id: Option<i64>) -> Result<Option<String>> {
    let Some(id) = id else { return Ok(None) };
    let row: Option<super::models::Vendor> = super::find_one(conn, "\"id\" = ?1", &[&id])?;
    Ok(row.map(|v| v.name))
}

fn variants_of(conn: &Connection, product_id: i64) -> Result<Vec<ProductVariant>> {
    super::find_many(conn, "WHERE \"productId\" = ?1", &[&product_id])
}

/// `list`: with category / subcategory / vendor names and the enabled-variant stock summary.
pub fn list(conn: &Connection, input: &ProductListInput) -> Result<ProductPage> {
    todo!("P3.3")
}

/// `findDetail`.
pub fn find_detail(conn: &Connection, id: i64, workspace_id: i64) -> Result<Option<ProductDetail>> {
    todo!("P3.3")
}

/// `createFromImport`.
pub fn create_from_import(
    conn: &Connection,
    workspace_id: i64,
    products: &[ProductImportInsert],
) -> Result<Vec<Product>> {
    todo!("P3.3")
}

fn insert_returning(conn: &Connection, patch: &Patch) -> Result<Product> {
    let mut q = Sql::raw("INSERT INTO \"Product\" ");
    q.append(insert_clause(Product::COLUMNS, patch)?);
    q.push(&format!(" RETURNING {}", Product::columns_sql(None)));
    q.model(conn)?
        .ok_or_else(|| super::Error::msg("Failed to insert product"))
}

/// `listFormOptions`: `(products (id, name), vendors (id, name))`.
pub fn list_form_options(
    conn: &Connection,
    workspace_id: i64,
    source: Option<&str>,
) -> Result<(Vec<(i64, String)>, Vec<(i64, String)>)> {
    todo!("P3.3")
}

/// `listForCatalogSync`: by id.
pub fn list_for_catalog_sync(
    conn: &Connection,
    workspace_id: i64,
    category_id: Option<i64>,
    product_ids: Option<&[i64]>,
) -> Result<Vec<CatalogSyncProduct>> {
    todo!("P3.3")
}

/// `create` (new snowflake id).
pub fn create(conn: &Connection, values: &Patch) -> Result<Product> {
    todo!("P3.3")
}

/// `update`: returns the id when the product is in the workspace.
pub fn update(
    conn: &Connection,
    workspace_id: i64,
    product_id: i64,
    values: &Patch,
) -> Result<Option<i64>> {
    todo!("P3.3")
}

/// `deleteByIds`: the deleted ids.
pub fn delete_by_ids(
    conn: &Connection,
    workspace_id: i64,
    product_ids: &[i64],
) -> Result<Vec<i64>> {
    todo!("P3.3")
}

/// `find`.
pub fn find(conn: &Connection, workspace_id: i64, product_id: i64) -> Result<Option<Product>> {
    todo!("P3.3")
}

/// `findByIds`.
pub fn find_by_ids(
    conn: &Connection,
    workspace_id: i64,
    product_ids: &[i64],
) -> Result<Vec<Product>> {
    todo!("P3.3")
}

fn is_product_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 18 && s.bytes().all(|b| b.is_ascii_digit())
}

/// `findByImportIdentifiers`: a retailer id matches our own id (1..18 digits) or the trimmed SKU.
pub fn find_by_import_identifiers(
    conn: &Connection,
    workspace_id: i64,
    retailer_ids: &[&str],
) -> Result<Vec<Product>> {
    todo!("P3.3")
}

const SHOP_FROM: &str = "FROM \"Product\" p LEFT JOIN \"ProductCategory\" cat ON cat.\"id\" = p.\"categoryId\" \
     LEFT JOIN \"ProductCategory\" sub ON sub.\"id\" = p.\"subcategoryId\" \
     LEFT JOIN \"Vendor\" v ON v.\"id\" = p.\"vendorId\" WHERE ";

fn shop_visible(workspace_id: i64, source: Option<&str>) -> Sql {
    let mut s = Sql::bind("p.\"workspaceId\" = ", workspace_id);
    s.push(" AND p.\"source\" = ");
    s.push_bind(source.unwrap_or(DEFAULT_SOURCE).to_owned());
    s.push(
        " AND p.\"status\" = 'active' AND p.\"isAddonOnly\" = 0 AND (p.\"categoryId\" IS NULL OR cat.\"isVisible\" = 1) \
         AND (p.\"subcategoryId\" IS NULL OR sub.\"isVisible\" = 1) AND (p.\"vendorId\" IS NULL OR v.\"isActive\" = 1)",
    );
    s
}

fn shop_select() -> String {
    format!(
        "SELECT {}, v.\"name\", v.\"discountPercent\", \
         EXISTS (SELECT 1 FROM \"ProductVariant\" pv WHERE pv.\"productId\" = p.\"id\" AND pv.\"isEnabled\"), \
         COALESCE((SELECT json_group_array(pv.\"inventoryQuantity\") FROM \"ProductVariant\" pv \
         WHERE pv.\"productId\" = p.\"id\" AND pv.\"isEnabled\"), '[]') ",
        Product::columns_sql(Some("p"))
    )
}

fn shop_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ShopProduct> {
    let n = Product::COLUMNS.len();
    let quantities: Json = r.get(n + 3)?;
    Ok(ShopProduct {
        product: Product::from_row(r)?,
        vendor: r.get(n)?,
        vendor_discount_percent: r.get::<_, Option<i64>>(n + 1)?.unwrap_or(0),
        has_variants: r.get(n + 2)?,
        variant_inventory_quantities: quantities
            .0
            .as_array()
            .map(|a| a.iter().map(serde_json::Value::as_i64).collect())
            .unwrap_or_default(),
    })
}

/// The product fields that [`list_related_for_shop`] reads.
#[derive(Debug, Clone, PartialEq)]
pub struct RelatedTo {
    pub id: i64,
    pub category_id: Option<i64>,
    pub subcategory_id: Option<i64>,
    pub tags: Vec<String>,
}

/// `listRelatedForShop`: same subcategory first, then same category, then shared tags.
pub fn list_related_for_shop(
    conn: &Connection,
    workspace_id: i64,
    source: Option<&str>,
    product: &RelatedTo,
    limit: i64,
) -> Result<Vec<ShopProduct>> {
    todo!("P3.3")
}

fn shop_where(input: &ShopListInput) -> Sql {
    let keyword = input
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty());
    Sql::and([
        Some(shop_visible(input.workspace_id, input.source.as_deref())),
        in_category(input.category_id, "p."),
        input.vendor_id.map(|v| Sql::bind("p.\"vendorId\" = ", v)),
        ids_in("p.\"id\"", input.product_ids.as_deref()),
        keyword.map(|k| {
            Sql::bind(
                "p.\"isSearchable\" = 1 AND ilike(p.\"name\", ",
                like_contains(k),
            )
            .closed(")")
        }),
    ])
}

/// `listForShop`: by rank, name; every row when no pagination is given.
pub fn list_for_shop(conn: &Connection, input: &ShopListInput) -> Result<ShopPage> {
    todo!("P3.3")
}

/// `listSearchableNames`: `(id, name)`.
pub fn list_searchable_names(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
    source: Option<&str>,
) -> Result<Vec<(i64, String)>> {
    todo!("P3.3")
}

/// `findForOrder` (`source` filters only when given).
pub fn find_for_order(
    conn: &Connection,
    workspace_id: i64,
    product_ids: &[i64],
    source: Option<&str>,
) -> Result<Vec<OrderProduct>> {
    todo!("P3.3")
}

/// `lockForStock`: by id (no row lock needed under the one writer).
pub fn lock_for_stock(
    conn: &Connection,
    workspace_id: i64,
    product_ids: &[i64],
) -> Result<Vec<StockRow>> {
    todo!("P3.3")
}

/// `takeInventory`: takes up to `quantity` of a tracked product; returns the taken amount.
pub fn take_inventory(
    conn: &Connection,
    workspace_id: i64,
    product_id: i64,
    quantity: i64,
) -> Result<i64> {
    todo!("P3.3")
}

/// `restoreInventory`.
pub fn restore_inventory(
    conn: &Connection,
    workspace_id: i64,
    product_id: i64,
    quantity: i64,
) -> Result<()> {
    todo!("P3.3")
}
