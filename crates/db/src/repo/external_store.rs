//! Port of `repositories/external-store/repository.ts` (`externalStoreRepository`).
//! `upsertProduct` has a ported test (P3.3 slice shopify-upsert); the other functions have
//! no TS tests (smoke test `tests/repo_p33_smoke.rs`).
//!
//! The TS opens its own `tx.transaction` so a sync and a webhook writing one product wait
//! for each other; here the caller runs [`upsert_product`] in one write job, which the one
//! SQLite writer serialises.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{DiscountCode, Product};
use super::patch::{self, Patch};
use super::sql::Sql;
use super::{Error, Model, Result, id_list, new_id, now_ms, text_list};

/// `ExternalVariantValues`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalVariantValues {
    pub external_id: String,
    pub external_inventory_item_id: Option<String>,
    /// Option name -> value.
    pub combination: serde_json::Value,
    pub price: f64,
    pub sku: Option<String>,
    pub inventory_quantity: Option<i64>,
    pub is_enabled: bool,
}

/// One product option (`{ name, values, position }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalProductOption {
    pub name: String,
    pub values: Vec<String>,
    pub position: i64,
}

/// Input of `upsertProduct`.
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertProductInput {
    pub workspace_id: i64,
    /// `shopify` / `woocommerce`.
    pub source: String,
    pub external_id: String,
    /// `ExternalProductValues`: product columns the caller sets.
    pub values: Patch,
    pub options: Vec<ExternalProductOption>,
    pub variants: Vec<ExternalVariantValues>,
}

/// `upsertProduct`: writes a product mirrored from an external store with its options and
/// variants (variants keep their ids: matched by the store's id, not replaced). Run it in
/// one write job.
pub fn upsert_product(conn: &Connection, input: &UpsertProductInput) -> Result<Product> {
    let mut values: Patch = vec![
        ("id", Value::Integer(new_id())),
        ("workspaceId", Value::Integer(input.workspace_id)),
        ("source", Value::Text(input.source.clone())),
        ("externalId", Value::Text(input.external_id.clone())),
    ];
    values.extend(input.values.iter().cloned());
    let mut sql = Sql::raw("INSERT INTO \"Product\" ");
    sql.append(patch::insert_clause(Product::COLUMNS, &values)?);
    let mut set = patch::excluded_clause(&input.values);
    if !set.is_empty() {
        set.push_str(", ");
    }
    set.push_str("\"updatedAt\" = ");
    sql.push(&format!(
        " ON CONFLICT (\"workspaceId\", \"source\", \"externalId\") WHERE \"externalId\" IS NOT NULL DO UPDATE SET {set}"
    ));
    sql.push_bind(now_ms());
    sql.push(&format!(" RETURNING {}", Product::columns_sql(None)));
    let product = sql
        .model::<Product>(conn)?
        .ok_or_else(|| Error::msg("Failed to save the mirrored product"))?;

    conn.execute(
        "DELETE FROM \"ProductVariantOption\" WHERE \"productId\" = ?1",
        [product.id],
    )?;
    for option in &input.options {
        conn.execute(
            "INSERT INTO \"ProductVariantOption\" (\"id\", \"productId\", \"name\", \"values\", \"position\") VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                new_id(),
                product.id,
                &option.name,
                serde_json::Value::from(option.values.clone()).to_string(),
                option.position,
            ),
        )?;
    }

    let existing: Vec<(i64, Option<String>)> = Sql::new(
        "SELECT \"id\", \"externalId\" FROM \"ProductVariant\" WHERE \"productId\" = ?",
        vec![Value::Integer(product.id)],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut keep = Vec::with_capacity(input.variants.len());
    for variant in &input.variants {
        let found = existing
            .iter()
            .find(|(_, ext)| ext.as_deref() == Some(variant.external_id.as_str()))
            .map(|(id, _)| *id);
        let combination = variant.combination.to_string();
        match found {
            Some(id) => {
                keep.push(id);
                conn.execute(
                    "UPDATE \"ProductVariant\" SET \"externalId\" = ?1, \"externalInventoryItemId\" = ?2, \"combination\" = ?3, \"price\" = ?4, \"sku\" = ?5, \"inventoryQuantity\" = ?6, \"isEnabled\" = ?7, \"updatedAt\" = ?8 WHERE \"id\" = ?9",
                    rusqlite::params![
                        variant.external_id,
                        variant.external_inventory_item_id,
                        combination,
                        variant.price,
                        variant.sku,
                        variant.inventory_quantity,
                        variant.is_enabled,
                        now_ms(),
                        id
                    ],
                )?;
            }
            None => {
                let id = new_id();
                keep.push(id);
                conn.execute(
                    "INSERT INTO \"ProductVariant\" (\"id\", \"productId\", \"externalId\", \"externalInventoryItemId\", \"combination\", \"price\", \"sku\", \"inventoryQuantity\", \"isEnabled\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        id,
                        product.id,
                        variant.external_id,
                        variant.external_inventory_item_id,
                        combination,
                        variant.price,
                        variant.sku,
                        variant.inventory_quantity,
                        variant.is_enabled
                    ],
                )?;
            }
        }
    }
    conn.execute(
        "DELETE FROM \"ProductVariant\" WHERE \"productId\" = ?1 AND \"id\" NOT IN (SELECT value FROM json_each(?2))",
        (product.id, id_list(&keep)),
    )?;
    Ok(product)
}

/// A variant row of `findVariantsByExternalIds` / `findVariantsForVariations`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalVariantRow {
    pub external_id: Option<String>,
    pub variant_id: i64,
    pub combination: super::Json,
    pub product_id: i64,
    /// Only set by `findVariantsForVariations`.
    pub product_external_id: Option<String>,
    pub category_id: Option<i64>,
    pub vendor_id: Option<i64>,
}

/// A product row of `findProductsByExternalIds`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalProductRow {
    pub id: i64,
    pub external_id: Option<String>,
    pub external_variant_id: Option<String>,
    pub category_id: Option<i64>,
    pub vendor_id: Option<i64>,
}

/// Result of `findCheckoutVariantIds` (maps as `(id, value)` lists).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckoutVariantIds {
    pub by_product: Vec<(i64, Option<String>)>,
    pub by_variant: Vec<(i64, Option<String>)>,
    pub product_external_ids: Vec<(i64, Option<String>)>,
}

/// `deleteProductByExternalId`.
pub fn delete_product_by_external_id(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
    external_id: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM \"Product\" WHERE \"workspaceId\" = ?1 AND \"source\" = ?2 AND \"externalId\" = ?3",
        (workspace_id, source, external_id),
    )?;
    Ok(())
}

fn delete_not_in(
    conn: &Connection,
    table: &str,
    workspace_id: i64,
    source: &str,
    keep_external_ids: &[String],
) -> Result<()> {
    let keep = (!keep_external_ids.is_empty()).then(|| text_list(keep_external_ids));
    conn.execute(
        &format!(
            "DELETE FROM \"{table}\" WHERE \"workspaceId\" = ?1 AND \"source\" = ?2 AND (?3 IS NULL OR \"externalId\" NOT IN (SELECT value FROM json_each(?3)))"
        ),
        (workspace_id, source, keep),
    )?;
    Ok(())
}

/// `deleteProductsNotIn`: every mirrored product of the source except the kept ones (all
/// of them for an empty list).
pub fn delete_products_not_in(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
    keep_external_ids: &[String],
) -> Result<()> {
    delete_not_in(conn, "Product", workspace_id, source, keep_external_ids)
}

/// `countProducts`.
pub fn count_products(conn: &Connection, workspace_id: i64, source: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM \"Product\" WHERE \"workspaceId\" = ?1 AND \"source\" = ?2",
        (workspace_id, source),
        |row| row.get(0),
    )?)
}

fn variant_rows(
    conn: &Connection,
    extra_where: &str,
    params: Vec<Value>,
) -> Result<Vec<ExternalVariantRow>> {
    Sql::new(
        format!(
            "SELECT v.\"externalId\", v.\"id\", v.\"combination\", p.\"id\", p.\"externalId\", p.\"categoryId\", p.\"vendorId\" FROM \"ProductVariant\" v JOIN \"Product\" p ON p.\"id\" = v.\"productId\" WHERE p.\"workspaceId\" = ? AND p.\"source\" = ? AND {extra_where}"
        ),
        params,
    )
    .query_map(conn, |row| {
        Ok(ExternalVariantRow {
            external_id: row.get(0)?,
            variant_id: row.get(1)?,
            combination: row.get(2)?,
            product_id: row.get(3)?,
            product_external_id: row.get(4)?,
            category_id: row.get(5)?,
            vendor_id: row.get(6)?,
        })
    })
}

/// `findVariantsByExternalIds` (`product_external_id` is not part of this result: `None`).
pub fn find_variants_by_external_ids(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
    external_ids: &[String],
) -> Result<Vec<ExternalVariantRow>> {
    if external_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows = variant_rows(
        conn,
        "v.\"externalId\" IN (SELECT value FROM json_each(?))",
        vec![
            Value::Integer(workspace_id),
            Value::Text(source.to_owned()),
            Value::Text(text_list(external_ids)),
        ],
    )?;
    for row in &mut rows {
        row.product_external_id = None;
    }
    Ok(rows)
}

/// `findVariantsForVariations`: variants whose store id is one of `variation_ids` or
/// starts with `<id>~`.
pub fn find_variants_for_variations(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
    variation_ids: &[String],
) -> Result<Vec<ExternalVariantRow>> {
    if variation_ids.is_empty() {
        return Ok(Vec::new());
    }
    let prefixes: Vec<String> = variation_ids
        .iter()
        .map(|id| format!("{}~%", super::utils::escape_like_pattern(id)))
        .collect();
    variant_rows(
        conn,
        "(v.\"externalId\" IN (SELECT value FROM json_each(?)) OR EXISTS (SELECT 1 FROM json_each(?) x WHERE like_cs(v.\"externalId\", x.value)))",
        vec![
            Value::Integer(workspace_id),
            Value::Text(source.to_owned()),
            Value::Text(text_list(variation_ids)),
            Value::Text(text_list(&prefixes)),
        ],
    )
}

/// `findProductsByExternalIds`.
pub fn find_products_by_external_ids(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
    external_ids: &[String],
) -> Result<Vec<ExternalProductRow>> {
    if external_ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"id\", \"externalId\", \"externalVariantId\", \"categoryId\", \"vendorId\" FROM \"Product\" WHERE \"workspaceId\" = ? AND \"source\" = ? AND \"externalId\" IN (SELECT value FROM json_each(?))",
        vec![
            Value::Integer(workspace_id),
            Value::Text(source.to_owned()),
            Value::Text(text_list(external_ids)),
        ],
    )
    .query_map(conn, |row| {
        Ok(ExternalProductRow {
            id: row.get(0)?,
            external_id: row.get(1)?,
            external_variant_id: row.get(2)?,
            category_id: row.get(3)?,
            vendor_id: row.get(4)?,
        })
    })
}

/// `findCheckoutVariantIds`: the store ids behind the cart's products and variants.
pub fn find_checkout_variant_ids(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
    product_ids: &[i64],
    variant_ids: &[i64],
) -> Result<CheckoutVariantIds> {
    let mut out = CheckoutVariantIds::default();
    if !product_ids.is_empty() {
        let rows: Vec<(i64, Option<String>, Option<String>)> = Sql::new(
            "SELECT \"id\", \"externalId\", \"externalVariantId\" FROM \"Product\" WHERE \"workspaceId\" = ? AND \"source\" = ? AND \"id\" IN (SELECT value FROM json_each(?))",
            vec![
                Value::Integer(workspace_id),
                Value::Text(source.to_owned()),
                Value::Text(id_list(product_ids)),
            ],
        )
        .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        for (id, external_id, external_variant_id) in rows {
            out.by_product.push((id, external_variant_id));
            out.product_external_ids.push((id, external_id));
        }
    }
    if !variant_ids.is_empty() {
        out.by_variant = Sql::new(
            "SELECT v.\"id\", v.\"externalId\" FROM \"ProductVariant\" v JOIN \"Product\" p ON p.\"id\" = v.\"productId\" WHERE p.\"workspaceId\" = ? AND p.\"source\" = ? AND v.\"id\" IN (SELECT value FROM json_each(?))",
            vec![
                Value::Integer(workspace_id),
                Value::Text(source.to_owned()),
                Value::Text(id_list(variant_ids)),
            ],
        )
        .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?;
    }
    Ok(out)
}

/// `upsertDiscount`: a discount code mirrored from the store (`values` = the discount
/// columns the caller sets).
pub fn upsert_discount(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
    external_id: &str,
    values: &Patch,
) -> Result<Option<DiscountCode>> {
    let mut all: Patch = vec![
        ("id", Value::Integer(new_id())),
        ("workspaceId", Value::Integer(workspace_id)),
        ("source", Value::Text(source.to_owned())),
        ("externalId", Value::Text(external_id.to_owned())),
    ];
    all.extend(values.iter().cloned());
    let mut sql = Sql::raw("INSERT INTO \"DiscountCode\" ");
    sql.append(patch::insert_clause(DiscountCode::COLUMNS, &all)?);
    let mut set = patch::excluded_clause(values);
    if !set.is_empty() {
        set.push_str(", ");
    }
    sql.push(&format!(
        " ON CONFLICT (\"workspaceId\", \"source\", \"externalId\") WHERE \"externalId\" IS NOT NULL DO UPDATE SET {set}\"updatedAt\" = "
    ));
    sql.push_bind(now_ms());
    sql.push(&format!(" RETURNING {}", DiscountCode::columns_sql(None)));
    sql.model::<DiscountCode>(conn)
}

/// `deleteDiscountByExternalId`.
pub fn delete_discount_by_external_id(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
    external_id: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM \"DiscountCode\" WHERE \"workspaceId\" = ?1 AND \"source\" = ?2 AND \"externalId\" = ?3",
        (workspace_id, source, external_id),
    )?;
    Ok(())
}

/// `deleteDiscountsNotIn`.
pub fn delete_discounts_not_in(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
    keep_external_ids: &[String],
) -> Result<()> {
    delete_not_in(
        conn,
        "DiscountCode",
        workspace_id,
        source,
        keep_external_ids,
    )
}
