//! `vendor/service.ts` (`vendorService`): names unique per workspace ignoring
//! case. Money fields are plain numbers here; the repository stores them
//! ×10,000. No cache invalidation (no read cache in the Rust port).

use std::collections::HashMap;

use chatbotx_db::Db;
use chatbotx_db::repo::models::Vendor;
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::utils::PaginationInput;
use chatbotx_db::repo::{is_unique_violation_error, product_category, vendor as repo};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension};

pub use chatbotx_db::repo::vendor::{VendorPage, VendorWithCount};

use crate::errors::{BusinessError, Result};
use crate::store_source::{external_store_managed, is_external_store_source, resolve_store_source};
use crate::tx;

const MONEY_SCALE: f64 = 10_000.0;

fn name_required() -> BusinessError {
    BusinessError::exception("Vendor name is required", "productVendorNameRequired", 400)
}

fn duplicated() -> BusinessError {
    BusinessError::exception("Vendor already exists", "productVendorDuplicated", 400)
}

fn duplicate_or(error: chatbotx_db::repo::Error) -> BusinessError {
    if is_unique_violation_error(&error, None) {
        duplicated()
    } else {
        BusinessError::Repo(error)
    }
}

fn text(value: &Option<String>) -> Sql {
    value.clone().map_or(Sql::Null, Sql::Text)
}

fn int(value: Option<i64>) -> Sql {
    value.map_or(Sql::Null, Sql::Integer)
}

/// The columns a write sets (`name` already trimmed).
fn patch_of(data: &VendorData, name: Option<String>) -> Patch {
    let mut out: Patch = Vec::new();
    if let Some(name) = name {
        out.push(("name", Sql::Text(name)));
    }
    let texts = [
        ("shortDescription", &data.short_description),
        ("phone", &data.phone),
        ("storeAddress", &data.store_address),
        ("imageUrl", &data.image_url),
    ];
    for (column, value) in texts {
        if let Some(value) = value {
            out.push((column, text(value)));
        }
    }
    if let Some(is_active) = data.is_active {
        out.push(("isActive", Sql::Integer(i64::from(is_active))));
    }
    for (column, value) in [
        ("discountPercent", data.discount_percent),
        ("minimumOrderValue", data.minimum_order_value),
    ] {
        if let Some(value) = value {
            out.push((column, Sql::Integer((value * MONEY_SCALE).round() as i64)));
        }
    }
    let ids = [
        ("openHoursId", data.open_hours_id),
        ("categoryId", data.category_id),
        ("subcategoryId", data.subcategory_id),
    ];
    for (column, value) in ids {
        if let Some(value) = value {
            out.push((column, int(value)));
        }
    }
    if let Some(rank) = data.rank {
        out.push(("rank", Sql::Integer(rank)));
    }
    out
}

fn assert_managed_here(conn: &Connection, workspace_id: i64, vendor_id: i64) -> Result<()> {
    let source = resolve_store_source(conn, workspace_id)?;
    if is_external_store_source(&source)
        && repo::find(conn, workspace_id, vendor_id)?.is_some_and(|row| row.source == source)
    {
        return Err(external_store_managed(&source));
    }
    Ok(())
}

/// `dropStaleSubcategory`.
fn drop_stale_subcategory(
    conn: &Connection,
    workspace_id: i64,
    mut data: VendorData,
    stored: Option<&Vendor>,
) -> Result<VendorData> {
    let (Some(category_id), None, Some(stored)) = (data.category_id, data.subcategory_id, stored)
    else {
        return Ok(data);
    };
    let Some(stored_sub) = stored.subcategory_id else {
        return Ok(data);
    };
    if stored.category_id == category_id {
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

/// `assertReferences`.
fn assert_references(
    conn: &Connection,
    workspace_id: i64,
    data: &VendorData,
    stored_category_id: Option<i64>,
) -> Result<()> {
    if let Some(Some(open_hours_id)) = data.open_hours_id {
        let found: Option<i64> = conn
            .query_row(
                r#"SELECT "id" FROM "EcommerceOpenHours" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                [open_hours_id, workspace_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| BusinessError::Repo(e.into()))?;
        if found.is_none() {
            return Err(BusinessError::not_found(
                "Open-hours schedule does not exist.",
            ));
        }
    }
    for id in [data.category_id.flatten(), data.subcategory_id.flatten()]
        .into_iter()
        .flatten()
    {
        if product_category::find(conn, workspace_id, id)?.is_none() {
            return Err(BusinessError::not_found("Product category does not exist."));
        }
    }
    if let Some(Some(sub)) = data.subcategory_id {
        let effective = match data.category_id {
            Some(category_id) => category_id,
            None => stored_category_id,
        };
        if product_category::find(conn, workspace_id, sub)?.and_then(|s| s.parent_id) != effective {
            return Err(BusinessError::not_found(
                "Product sub-category does not belong to the selected category.",
            ));
        }
    }
    Ok(())
}

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
    scaled as f64 / MONEY_SCALE
}

/// `list`: rank, then name, with product counts.
pub async fn list(db: &Db, input: ListVendors) -> Result<VendorPage> {
    tx::read(db, move |conn| {
        let source = resolve_store_source(conn, input.workspace_id)?;
        Ok(repo::list_with_product_count(
            conn,
            input.workspace_id,
            Some(&source),
            input.name.as_deref(),
            PaginationInput {
                page: input.page,
                per_page: input.per_page,
            },
        )?)
    })
    .await
}

/// `create` (`name` is required).
pub async fn create(db: &Db, workspace_id: i64, data: VendorData) -> Result<Vendor> {
    tx::write(db, move |conn| {
        let source = resolve_store_source(conn, workspace_id)?;
        if is_external_store_source(&source) {
            return Err(external_store_managed(&source));
        }
        let name = data.name.as_deref().unwrap_or("").trim().to_string();
        if name.is_empty() {
            return Err(name_required());
        }
        assert_references(conn, workspace_id, &data, None)?;
        repo::create(conn, workspace_id, &patch_of(&data, Some(name)))
            .map_err(duplicate_or)?
            .ok_or_else(|| BusinessError::exception("Failed to create vendor", "systemError", 400))
    })
    .await
}

/// `update`: only the sent fields; a stale sub-category is cleared when the
/// category changes alone.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    vendor_id: i64,
    data: VendorData,
) -> Result<Vendor> {
    tx::write(db, move |conn| {
        if patch_of(&data, data.name.clone())
            .iter()
            .any(|(column, _)| *column != "isActive")
        {
            assert_managed_here(conn, workspace_id, vendor_id)?;
        }
        let name = data.name.as_deref().map(|n| n.trim().to_string());
        if name.as_deref() == Some("") {
            return Err(name_required());
        }
        let stored = if data.category_id.is_some() || data.subcategory_id.is_some() {
            repo::find(conn, workspace_id, vendor_id)?
        } else {
            None
        };
        let data = drop_stale_subcategory(conn, workspace_id, data, stored.as_ref())?;
        assert_references(
            conn,
            workspace_id,
            &data,
            stored.as_ref().and_then(|s| s.category_id),
        )?;
        repo::update(conn, workspace_id, vendor_id, &patch_of(&data, name))
            .map_err(duplicate_or)?
            .ok_or_else(|| BusinessError::not_found("Vendor not found"))
    })
    .await
}

/// `delete`: its products keep no vendor (foreign key).
pub async fn delete(db: &Db, workspace_id: i64, vendor_id: i64) -> Result<()> {
    tx::write(db, move |conn| {
        assert_managed_here(conn, workspace_id, vendor_id)?;
        repo::delete(conn, workspace_id, vendor_id)?
            .map(|_| ())
            .ok_or_else(|| BusinessError::not_found("Vendor not found"))
    })
    .await
}

/// `assertBelongsToWorkspace`.
pub fn assert_belongs_to_workspace(
    conn: &Connection,
    workspace_id: i64,
    vendor_id: i64,
) -> Result<Vendor> {
    repo::find(conn, workspace_id, vendor_id)?
        .ok_or_else(|| BusinessError::not_found("Vendor does not exist."))
}

/// `resolveByNames`: ids keyed by lower-cased name.
pub fn resolve_by_names(
    conn: &Connection,
    workspace_id: i64,
    names: &[String],
    create_missing: bool,
) -> Result<HashMap<String, i64>> {
    let mut unique: Vec<String> = Vec::new();
    for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        if !unique.iter().any(|u| u == name) {
            unique.push(name.to_string());
        }
    }
    if unique.is_empty() {
        return Ok(HashMap::new());
    }
    let refs: Vec<&str> = unique.iter().map(String::as_str).collect();
    if create_missing {
        repo::create_missing(conn, workspace_id, &refs, None)?;
    }
    Ok(repo::find_by_names(conn, workspace_id, &refs, None)?
        .into_iter()
        .map(|(id, name)| (name.to_lowercase(), id))
        .collect())
}

/// `resolveName`: a blank name is no vendor; a new name creates the vendor.
pub fn resolve_name(
    conn: &Connection,
    workspace_id: i64,
    name: Option<&str>,
) -> Result<Option<i64>> {
    let name = name.map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Ok(None);
    }
    let ids = resolve_by_names(conn, workspace_id, &[name.to_string()], true)?;
    Ok(ids.get(&name.to_lowercase()).copied())
}
