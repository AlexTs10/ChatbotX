//! `place-near-me/category.service.ts` (`placeCategoryService`).
//!
//! The original invalidates the `place-categories:<workspaceId>` cache tag after
//! each write; the Rust port has no read cache, so there is nothing to drop.

use chatbotx_core::js;
use chatbotx_db::Db;
use chatbotx_db::repo::client::is_unique_violation_error;
use chatbotx_db::repo::models::PlaceCategory;
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::place;
use chatbotx_db::rusqlite::types::Value;

use crate::errors::{BusinessError, Result};
use crate::tx;

/// One row of `placeCategoryService.list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceCategoryListItem {
    pub id: i64,
    pub name: String,
    pub rank: i64,
    pub location_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Input of [`create`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePlaceCategory {
    pub workspace_id: i64,
    pub name: String,
    pub rank: Option<i64>,
}

/// Input of [`update`] (`data` keys: `None` = not sent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlaceCategory {
    pub workspace_id: i64,
    pub category_id: i64,
    pub name: Option<String>,
    pub rank: Option<i64>,
}

/// Result of [`delete`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeletedPlaceCategory {
    pub uncategorized_location_count: i64,
}

fn normalize_name(name: &str) -> String {
    js::trim(name).to_string()
}

fn name_required() -> BusinessError {
    BusinessError::exception(
        "Place category name is required",
        "placeCategoryNameRequired",
        400,
    )
}

fn duplicated() -> BusinessError {
    BusinessError::exception(
        "Place category already exists",
        "placeCategoryDuplicated",
        409,
    )
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Place category not found")
}

fn map_unique(error: BusinessError) -> BusinessError {
    match error {
        BusinessError::Repo(repo) if is_unique_violation_error(&repo, None) => duplicated(),
        other => other,
    }
}

/// `placeCategoryService.list`: by rank, name, id, with location counts.
pub async fn list(db: &Db, workspace_id: i64) -> Result<Vec<PlaceCategoryListItem>> {
    todo!("P3.6-D1")
}

/// `placeCategoryService.find`.
pub async fn find(db: &Db, workspace_id: i64, category_id: i64) -> Result<PlaceCategory> {
    todo!("P3.6-D1")
}

/// `placeCategoryService.create`: trims the name; a name taken in any case is
/// `placeCategoryDuplicated` (409).
pub async fn create(db: &Db, input: CreatePlaceCategory) -> Result<PlaceCategory> {
    todo!("P3.6-D1")
}

/// `placeCategoryService.update`: only the sent fields change.
pub async fn update(db: &Db, input: UpdatePlaceCategory) -> Result<PlaceCategory> {
    todo!("P3.6-D1")
}

/// `placeCategoryService.delete`: its locations stay, uncategorized.
pub async fn delete(db: &Db, workspace_id: i64, category_id: i64) -> Result<DeletedPlaceCategory> {
    todo!("P3.6-D1")
}
