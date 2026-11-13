//! `product-category/service.ts` (`productCategoryService`): a two-level tree
//! (category -> sub-category), names unique per parent (exact case). The
//! original drops the `product-categories:` cache tag after each write and
//! checks template edit locks; the Rust port has no read cache and no locks
//! (DECISIONS, P3.6-D1).

use std::collections::HashMap;

use chatbotx_db::Db;
use chatbotx_db::repo::models::ProductCategory;
use chatbotx_db::rusqlite::Connection;

pub use chatbotx_db::repo::product_category::CategoryWithCount;

use crate::errors::Result;

/// `list` filter: one category's children and / or a name part (any case).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFilter {
    pub parent_id: Option<i64>,
    pub name: Option<String>,
}

/// `listLevel` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelInput {
    pub workspace_id: i64,
    pub parent_id: Option<i64>,
    pub name: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// One row of a level, with its number of sub-categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelRow {
    pub category: CategoryWithCount,
    pub child_count: i64,
}

/// `listLevel` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    pub data: Vec<LevelRow>,
    pub page_count: i64,
    pub parent: Option<CategoryWithCount>,
}

/// `create` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateCategory {
    pub workspace_id: i64,
    pub name: String,
    pub rank: Option<i64>,
    pub parent_id: Option<i64>,
    pub image_url: Option<String>,
    pub is_visible: Option<bool>,
}

/// `update` input: `None` leaves a field as it is; `parent_id: Some(None)`
/// promotes to the top level.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateCategory {
    pub workspace_id: i64,
    pub category_id: i64,
    pub name: Option<String>,
    pub parent_id: Option<Option<i64>>,
    pub image_url: Option<Option<String>>,
    pub is_visible: Option<bool>,
    pub rank: Option<i64>,
}

/// `ProductCategoryPath`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CategoryPath {
    pub category_name: String,
    pub subcategory_name: Option<String>,
}

/// One resolved path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPath {
    pub category_id: i64,
    pub subcategory_id: Option<i64>,
}

/// `getProductCategoryPathKey`: lower-cased trimmed names joined by `\0`.
pub fn path_key(path: &CategoryPath) -> String {
    let _ = path;
    todo!("P3.6-D1")
}

/// `list`: the whole tree (rank, then name) with product counts, filtered.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    filter: ListFilter,
) -> Result<Vec<CategoryWithCount>> {
    let _ = (db, workspace_id, filter);
    todo!("P3.6-D1")
}

/// `listLevel`: one page of one level, each row with its child count.
pub async fn list_level(db: &Db, input: LevelInput) -> Result<Level> {
    let _ = (db, input);
    todo!("P3.6-D1")
}

/// `create`.
pub async fn create(db: &Db, input: CreateCategory) -> Result<ProductCategory> {
    let _ = (db, input);
    todo!("P3.6-D1")
}

/// `update`.
pub async fn update(db: &Db, input: UpdateCategory) -> Result<ProductCategory> {
    let _ = (db, input);
    todo!("P3.6-D1")
}

/// `delete`: removes the sub-categories too (foreign key) and switches off the
/// category discount codes of the subtree first.
pub async fn delete(db: &Db, workspace_id: i64, category_id: i64) -> Result<()> {
    let _ = (db, workspace_id, category_id);
    todo!("P3.6-D1")
}

/// `countProducts`.
pub async fn count_products(db: &Db, workspace_id: i64, category_id: i64) -> Result<i64> {
    let _ = (db, workspace_id, category_id);
    todo!("P3.6-D1")
}

/// `resolveByNames`: top-level category ids keyed by lower-cased name.
pub fn resolve_by_names(
    conn: &Connection,
    workspace_id: i64,
    names: &[String],
    create_missing: bool,
) -> Result<HashMap<String, i64>> {
    let _ = (conn, workspace_id, names, create_missing);
    todo!("P3.6-D1")
}

/// `resolvePaths`: ids keyed by [`path_key`].
pub fn resolve_paths(
    conn: &Connection,
    workspace_id: i64,
    paths: &[CategoryPath],
    create_missing: bool,
) -> Result<HashMap<String, ResolvedPath>> {
    let _ = (conn, workspace_id, paths, create_missing);
    todo!("P3.6-D1")
}
