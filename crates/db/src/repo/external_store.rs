//! Port of `repositories/external-store/repository.ts` (`externalStoreRepository`), first
//! part: `upsertProduct` (P3.3 slice shopify-upsert; the rest of the file is open).
//!
//! The TS opens its own `tx.transaction` so a sync and a webhook writing one product wait
//! for each other; here the caller runs [`upsert_product`] in one write job, which the one
//! SQLite writer serialises.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::Product;
use super::patch::{self, Patch};
use super::sql::Sql;
use super::{Error, Model, Result, id_list, new_id, now_ms};

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
