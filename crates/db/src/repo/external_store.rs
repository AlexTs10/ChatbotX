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
    todo!("P3.3")
}
