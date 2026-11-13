//! `vendor/service.ts` (`vendorService`): names unique per workspace ignoring
//! case. Money fields are plain numbers here; the repository stores them
//! ×10,000. No cache invalidation (no read cache in the Rust port).

use std::collections::HashMap;

use chatbotx_db::Db;
use chatbotx_db::repo::models::Vendor;
use chatbotx_db::rusqlite::Connection;

pub use chatbotx_db::repo::vendor::{VendorPage, VendorWithCount};

use crate::errors::Result;

/// `VendorWriteData` (all optional: the update form). `Some(None)` = null.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VendorData {
    pub name: Option<String>,
    pub short_description: Option<Option<String>>,
    pub phone: Option<Option<String>>,
    pub store_address: Option<Option<String>>,
    pub image_url: Option<Option<String>>,
    pub is_active: Option<bool>,
    pub discount_percent: Option<f64>,
    pub minimum_order_value: Option<f64>,
    pub open_hours_id: Option<Option<i64>>,
    pub category_id: Option<Option<i64>>,
    pub subcategory_id: Option<Option<i64>>,
    pub rank: Option<i64>,
}

/// `VendorListInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListVendors {
    pub workspace_id: i64,
    pub name: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// A stored money value (×10,000) as a number.
pub fn money(scaled: i64) -> f64 {
    let _ = scaled;
    todo!("P3.6-D1")
}

/// `list`: rank, then name, with product counts.
pub async fn list(db: &Db, input: ListVendors) -> Result<VendorPage> {
    let _ = (db, input);
    todo!("P3.6-D1")
}

/// `create` (`name` is required).
pub async fn create(db: &Db, workspace_id: i64, data: VendorData) -> Result<Vendor> {
    let _ = (db, workspace_id, data);
    todo!("P3.6-D1")
}

/// `update`: only the sent fields; a stale sub-category is cleared when the
/// category changes alone.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    vendor_id: i64,
    data: VendorData,
) -> Result<Vendor> {
    let _ = (db, workspace_id, vendor_id, data);
    todo!("P3.6-D1")
}

/// `delete`: its products keep no vendor (foreign key).
pub async fn delete(db: &Db, workspace_id: i64, vendor_id: i64) -> Result<()> {
    let _ = (db, workspace_id, vendor_id);
    todo!("P3.6-D1")
}

/// `assertBelongsToWorkspace`.
pub fn assert_belongs_to_workspace(
    conn: &Connection,
    workspace_id: i64,
    vendor_id: i64,
) -> Result<Vendor> {
    let _ = (conn, workspace_id, vendor_id);
    todo!("P3.6-D1")
}

/// `resolveByNames`: ids keyed by lower-cased name.
pub fn resolve_by_names(
    conn: &Connection,
    workspace_id: i64,
    names: &[String],
    create_missing: bool,
) -> Result<HashMap<String, i64>> {
    let _ = (conn, workspace_id, names, create_missing);
    todo!("P3.6-D1")
}

/// `resolveName`: a blank name is no vendor; a new name creates the vendor.
pub fn resolve_name(
    conn: &Connection,
    workspace_id: i64,
    name: Option<&str>,
) -> Result<Option<i64>> {
    let _ = (conn, workspace_id, name);
    todo!("P3.6-D1")
}
