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
    let pagination = parse_pagination(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let order = parse_order_by(Product::COLUMNS, input.sort.as_deref(), None);
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Product\" p WHERE ",
        Product::columns_sql(Some("p"))
    ));
    q.append(list_where(input));
    q.push(&format!(" {}", order_by_sql(&order, Some("p"))));
    if let Some(p) = pagination {
        q.push(" LIMIT ");
        q.push_bind(p.limit);
        q.push(" OFFSET ");
        q.push_bind(p.offset);
    }
    let rows: Vec<Product> = q.models(conn)?;
    let mut c = Sql::raw("SELECT count(*) FROM \"Product\" p WHERE ");
    c.append(list_where(input));
    let total: i64 = c
        .query_map(conn, |r| r.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    let mut data = Vec::with_capacity(rows.len());
    for product in rows {
        let enabled: Vec<ProductVariant> = variants_of(conn, product.id)?
            .into_iter()
            .filter(|v| v.is_enabled)
            .collect();
        let stocked: Vec<i64> = enabled
            .iter()
            .filter_map(|v| v.inventory_quantity)
            .collect();
        data.push(ProductListRow {
            category: category_name(conn, product.category_id)?,
            subcategory: category_name(conn, product.subcategory_id)?,
            vendor: vendor_name(conn, product.vendor_id)?,
            variant_count: enabled.len() as i64,
            variant_inventory_total: (!stocked.is_empty())
                .then(|| stocked.iter().map(|q| (*q).max(0)).sum()),
            product,
        });
    }
    Ok(ProductPage {
        data,
        page_count: pagination.map_or(1, |p| (total + p.limit - 1) / p.limit),
    })
}

/// `findDetail`.
pub fn find_detail(conn: &Connection, id: i64, workspace_id: i64) -> Result<Option<ProductDetail>> {
    let row: Option<Product> = super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )?;
    let Some(product) = row else {
        return Ok(None);
    };
    Ok(Some(ProductDetail {
        category: category_name(conn, product.category_id)?,
        subcategory: category_name(conn, product.subcategory_id)?,
        vendor: vendor_name(conn, product.vendor_id)?,
        variant_options: super::find_many(conn, "WHERE \"productId\" = ?1", &[&product.id])?,
        variants: variants_of(conn, product.id)?,
        addons: super::find_many(conn, "WHERE \"productId\" = ?1", &[&product.id])?,
        product,
    }))
}

/// `createFromImport`.
pub fn create_from_import(
    conn: &Connection,
    workspace_id: i64,
    products: &[ProductImportInsert],
) -> Result<Vec<Product>> {
    let now = now_ms();
    let mut out = Vec::with_capacity(products.len());
    for p in products {
        let mut patch: Patch = vec![
            ("id", Value::Integer(new_id())),
            ("createdAt", Value::Integer(now)),
            ("updatedAt", Value::Integer(now)),
            ("workspaceId", Value::Integer(workspace_id)),
            ("name", Value::Text(p.name.clone())),
            ("sku", super::sql::opt_text(p.sku.as_deref())),
            ("price", Value::Real(p.price.unwrap_or(0.0))),
            ("discount", Value::Real(p.discount.unwrap_or(0.0))),
            ("productUrl", super::sql::opt_text(p.product_url.as_deref())),
            (
                "shortDescription",
                super::sql::opt_text(p.short_description.as_deref()),
            ),
            (
                "longDescription",
                super::sql::opt_text(p.long_description.as_deref()),
            ),
            ("categoryId", super::sql::opt_int(p.category_id)),
            ("subcategoryId", super::sql::opt_int(p.subcategory_id)),
            ("tags", Value::Text(text_list(&p.tags))),
            ("vendorId", super::sql::opt_int(p.vendor_id)),
            (
                "inventoryQuantity",
                Value::Integer(p.inventory_quantity.unwrap_or(0)),
            ),
            (
                "inventoryPolicy",
                Value::Text(
                    p.inventory_policy
                        .clone()
                        .unwrap_or_else(|| "dont_track".to_owned()),
                ),
            ),
            (
                "allowOutOfStockPurchase",
                super::sql::bool_val(p.allow_out_of_stock_purchase.unwrap_or(false)),
            ),
            (
                "images",
                Value::Text(
                    p.images
                        .clone()
                        .unwrap_or_else(|| serde_json::json!([]))
                        .to_string(),
                ),
            ),
            (
                "status",
                Value::Text(p.status.clone().unwrap_or_else(|| "active".to_owned())),
            ),
        ];
        if let Some(c) = p.currency.as_deref().filter(|c| !c.is_empty()) {
            patch.push(("currency", Value::Text(c.to_owned())));
        }
        out.push(insert_returning(conn, &patch)?);
    }
    Ok(out)
}

fn insert_returning(conn: &Connection, patch: &Patch) -> Result<Product> {
    let mut q = Sql::raw("INSERT INTO \"Product\" ");
    q.append(insert_clause(Product::COLUMNS, patch)?);
    q.push(&format!(" RETURNING {}", Product::columns_sql(None)));
    q.model(conn)?
        .ok_or_else(|| super::Error::msg("Failed to insert product"))
}

/// `(id, name)` options of products and vendors.
pub type FormOptions = (Vec<(i64, String)>, Vec<(i64, String)>);

/// `listFormOptions`: `(products (id, name), vendors (id, name))`.
pub fn list_form_options(
    conn: &Connection,
    workspace_id: i64,
    source: Option<&str>,
) -> Result<FormOptions> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"name\" FROM \"Product\" WHERE \"workspaceId\" = ?1 AND \"source\" = ?2 ORDER BY \"name\" ASC, \"id\" ASC",
    )?;
    let products = stmt
        .query_map(
            rusqlite::params![workspace_id, source.unwrap_or(DEFAULT_SOURCE)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let vendors = super::vendor::list_options(conn, workspace_id, source)?;
    Ok((products, vendors))
}

/// `listForCatalogSync`: by id.
pub fn list_for_catalog_sync(
    conn: &Connection,
    workspace_id: i64,
    category_id: Option<i64>,
    product_ids: Option<&[i64]>,
) -> Result<Vec<CatalogSyncProduct>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Product\" p WHERE ",
        Product::columns_sql(Some("p"))
    ));
    q.append(Sql::and([
        Some(Sql::bind("p.\"workspaceId\" = ", workspace_id)),
        in_category(category_id, "p."),
        ids_in("p.\"id\"", product_ids),
    ]));
    q.push(" ORDER BY p.\"id\" ASC");
    let rows: Vec<Product> = q.models(conn)?;
    rows.into_iter()
        .map(|product| {
            let cat = |id: Option<i64>| -> Result<Option<ProductCategory>> {
                match id {
                    Some(id) => super::find_one(conn, "\"id\" = ?1", &[&id]),
                    None => Ok(None),
                }
            };
            Ok(CatalogSyncProduct {
                category: cat(product.category_id)?,
                subcategory: cat(product.subcategory_id)?,
                vendor: vendor_name(conn, product.vendor_id)?,
                variants: variants_of(conn, product.id)?,
                product,
            })
        })
        .collect()
}

/// `create` (new snowflake id).
pub fn create(conn: &Connection, values: &Patch) -> Result<Product> {
    let now = now_ms();
    let mut all: Patch = vec![
        ("id", Value::Integer(new_id())),
        ("createdAt", Value::Integer(now)),
        ("updatedAt", Value::Integer(now)),
    ];
    all.extend(values.iter().cloned());
    insert_returning(conn, &all)
}

/// `update`: returns the id when the product is in the workspace.
pub fn update(
    conn: &Connection,
    workspace_id: i64,
    product_id: i64,
    values: &Patch,
) -> Result<Option<i64>> {
    let mut all = values.clone();
    all.push(("updatedAt", Value::Integer(now_ms())));
    let mut q = Sql::raw("UPDATE \"Product\" SET ");
    q.append(set_clause(Product::COLUMNS, &all)?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(product_id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" RETURNING \"id\"");
    Ok(q.query_map(conn, |r| r.get(0))?.into_iter().next())
}

/// `deleteByIds`: the deleted ids.
pub fn delete_by_ids(
    conn: &Connection,
    workspace_id: i64,
    product_ids: &[i64],
) -> Result<Vec<i64>> {
    if product_ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "DELETE FROM \"Product\" WHERE \"workspaceId\" = ? AND \"id\" IN (SELECT value FROM json_each(?)) RETURNING \"id\"",
        vec![Value::Integer(workspace_id), Value::Text(id_list(product_ids))],
    )
    .query_map(conn, |r| r.get(0))
}

/// `find`.
pub fn find(conn: &Connection, workspace_id: i64, product_id: i64) -> Result<Option<Product>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&product_id, &workspace_id],
    )
}

/// `findByIds`.
pub fn find_by_ids(
    conn: &Connection,
    workspace_id: i64,
    product_ids: &[i64],
) -> Result<Vec<Product>> {
    if product_ids.is_empty() {
        return Ok(Vec::new());
    }
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2))",
        &[&workspace_id, &id_list(product_ids)],
    )
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
    if retailer_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = retailer_ids
        .iter()
        .filter(|r| is_product_id(r))
        .filter_map(|r| r.parse().ok())
        .collect();
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND (\"id\" IN (SELECT value FROM json_each(?2)) \
         OR trim(\"sku\") IN (SELECT value FROM json_each(?3)))",
        &[&workspace_id, &id_list(&ids), &text_list(retailer_ids)],
    )
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
    let category_ids: Vec<i64> = [product.category_id, product.subcategory_id]
        .into_iter()
        .flatten()
        .collect();
    let tags: Vec<&str> = product
        .tags
        .iter()
        .map(String::as_str)
        .filter(|t| !t.trim().is_empty())
        .collect();
    if category_ids.is_empty() && tags.is_empty() {
        return Ok(Vec::new());
    }
    let same_sub = product
        .subcategory_id
        .map(|id| Sql::bind("p.\"subcategoryId\" = ", id));
    let same_cat = (!category_ids.is_empty()).then(|| {
        let mut s = Sql::bind(
            "p.\"categoryId\" IN (SELECT value FROM json_each(",
            id_list(&category_ids),
        );
        s.push(")) OR p.\"subcategoryId\" IN (SELECT value FROM json_each(");
        s.push_bind(id_list(&category_ids));
        s.push("))");
        s
    });
    let shares_tag = (!tags.is_empty()).then(|| {
        Sql::bind(
            "EXISTS (SELECT 1 FROM json_each(p.\"tags\") t WHERE t.value IN (SELECT value FROM json_each(",
            text_list(&tags),
        )
        .closed(")))")
    });
    let mut q = Sql::raw(format!("{}{SHOP_FROM}", shop_select()));
    q.append(shop_visible(workspace_id, source));
    q.push(" AND p.\"id\" <> ");
    q.push_bind(product.id);
    q.push(" AND ");
    q.append(Sql::or([same_cat.clone(), shares_tag]));
    q.push(" ORDER BY CASE WHEN ");
    q.append(same_sub.unwrap_or_else(|| Sql::raw("0")));
    q.push(" THEN 0 WHEN ");
    q.append(same_cat.unwrap_or_else(|| Sql::raw("0")));
    q.push(" THEN 1 ELSE 2 END, p.\"rank\" ASC, p.\"name\" ASC LIMIT ");
    q.push_bind(limit);
    q.query_map(conn, shop_row)
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
    let pagination = parse_pagination(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let mut q = Sql::raw(format!("{}{SHOP_FROM}", shop_select()));
    q.append(shop_where(input));
    q.push(" ORDER BY p.\"rank\" ASC, p.\"name\" ASC");
    if let Some(p) = pagination {
        q.push(" LIMIT ");
        q.push_bind(p.limit);
        q.push(" OFFSET ");
        q.push_bind(p.offset);
    }
    let data = q.query_map(conn, shop_row)?;
    let mut c = Sql::raw(format!("SELECT count(*) {SHOP_FROM}"));
    c.append(shop_where(input));
    let total: i64 = c
        .query_map(conn, |r| r.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    Ok(ShopPage {
        data,
        page_count: pagination.map_or(1, |p| (total + p.limit - 1) / p.limit),
    })
}

/// `listSearchableNames`: `(id, name)`.
pub fn list_searchable_names(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
    source: Option<&str>,
) -> Result<Vec<(i64, String)>> {
    let mut q = Sql::raw(format!("SELECT p.\"id\", p.\"name\" {SHOP_FROM}"));
    q.append(shop_visible(workspace_id, source));
    q.push(" AND p.\"isSearchable\" = 1 ORDER BY p.\"rank\" ASC, p.\"name\" ASC LIMIT ");
    q.push_bind(limit);
    q.query_map(conn, |r| Ok((r.get(0)?, r.get(1)?)))
}

/// `findForOrder` (`source` filters only when given).
pub fn find_for_order(
    conn: &Connection,
    workspace_id: i64,
    product_ids: &[i64],
    source: Option<&str>,
) -> Result<Vec<OrderProduct>> {
    if product_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut q = Sql::raw(format!("{} WHERE ", Product::select_sql()));
    q.append(Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", workspace_id)),
        ids_in("\"id\"", Some(product_ids)),
        source.map(|s| Sql::bind("\"source\" = ", s.to_owned())),
    ]));
    let rows: Vec<Product> = q.models(conn)?;
    rows.into_iter()
        .map(|product| {
            let vendor = match product.vendor_id {
                Some(id) => {
                    let v: Option<super::models::Vendor> =
                        super::find_one(conn, "\"id\" = ?1", &[&id])?;
                    v.map(|v| {
                        (
                            v.name,
                            v.is_active,
                            v.discount_percent,
                            v.minimum_order_value,
                            v.open_hours_id,
                        )
                    })
                }
                None => None,
            };
            Ok(OrderProduct {
                variants: variants_of(conn, product.id)?,
                vendor,
                product,
            })
        })
        .collect()
}

/// `lockForStock`: by id (no row lock needed under the one writer).
pub fn lock_for_stock(
    conn: &Connection,
    workspace_id: i64,
    product_ids: &[i64],
) -> Result<Vec<StockRow>> {
    if product_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"inventoryPolicy\", \"inventoryQuantity\", \"allowOutOfStockPurchase\" FROM \"Product\" \
         WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2)) ORDER BY \"id\" ASC",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, id_list(product_ids)], |r| {
            Ok(StockRow {
                id: r.get(0)?,
                inventory_policy: r.get(1)?,
                inventory_quantity: r.get(2)?,
                allow_out_of_stock_purchase: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `takeInventory`: takes up to `quantity` of a tracked product; returns the taken amount.
pub fn take_inventory(
    conn: &Connection,
    workspace_id: i64,
    product_id: i64,
    quantity: i64,
) -> Result<i64> {
    let row: Option<Product> = super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2 AND \"inventoryPolicy\" = 'track'",
        &[&product_id, &workspace_id],
    )?;
    let Some(row) = row else { return Ok(0) };
    let taken = quantity.min(row.inventory_quantity).max(0);
    if taken > 0 {
        conn.execute(
            "UPDATE \"Product\" SET \"inventoryQuantity\" = \"inventoryQuantity\" - ?3, \"updatedAt\" = ?4 \
             WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
            rusqlite::params![product_id, workspace_id, taken, now_ms()],
        )?;
    }
    Ok(taken)
}

/// `restoreInventory`.
pub fn restore_inventory(
    conn: &Connection,
    workspace_id: i64,
    product_id: i64,
    quantity: i64,
) -> Result<()> {
    if quantity <= 0 {
        return Ok(());
    }
    conn.execute(
        "UPDATE \"Product\" SET \"inventoryQuantity\" = \"inventoryQuantity\" + ?3, \"updatedAt\" = ?4 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        rusqlite::params![product_id, workspace_id, quantity, now_ms()],
    )?;
    Ok(())
}
