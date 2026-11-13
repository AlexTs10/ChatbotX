//! `product-category/service.ts` (`productCategoryService`): a two-level tree
//! (category -> sub-category), names unique per parent (exact case). The
//! original drops the `product-categories:` cache tag after each write and
//! checks template edit locks; the Rust port has no read cache and no locks
//! (DECISIONS, P3.6-D1).

use std::collections::HashMap;

use chatbotx_db::Db;
use chatbotx_db::repo::models::ProductCategory;
use chatbotx_db::repo::{is_unique_violation_error, product_category as repo};
use chatbotx_db::rusqlite::Connection;

pub use chatbotx_db::repo::product_category::CategoryWithCount;

use crate::errors::{BusinessError, Result};
use crate::store_source::{external_store_managed, is_external_store_source, resolve_store_source};
use crate::{discount_code, tx};

/// The source of rows the importers create.
const OWN_SOURCE: &str = "chatbotx";

fn name_required() -> BusinessError {
    BusinessError::exception(
        "Product category name is required",
        "productCategoryNameRequired",
        400,
    )
}

fn duplicated() -> BusinessError {
    BusinessError::exception(
        "Product category already exists",
        "productCategoryDuplicated",
        400,
    )
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Product category not found")
}

/// A unique violation is a duplicate name; other errors pass through.
fn duplicate_or(error: chatbotx_db::repo::Error) -> BusinessError {
    if is_unique_violation_error(&error, None) {
        duplicated()
    } else {
        BusinessError::Repo(error)
    }
}

fn lower(name: &str) -> String {
    name.trim().to_lowercase()
}

fn child_key(parent_id: Option<i64>, name: &str) -> String {
    format!(
        "{}\u{0}{}",
        parent_id.map_or_else(|| "null".to_string(), |p| p.to_string()),
        lower(name)
    )
}

/// `assertManagedHere`: a row mirrored from the connected store changes there.
fn assert_managed_here(conn: &Connection, workspace_id: i64, category_id: i64) -> Result<()> {
    let source = resolve_store_source(conn, workspace_id)?;
    if !is_external_store_source(&source) {
        return Ok(());
    }
    if repo::find(conn, workspace_id, category_id)?.is_some_and(|row| row.source == source) {
        return Err(external_store_managed(&source));
    }
    Ok(())
}

/// `assertParentCanAdopt`.
fn assert_parent_can_adopt(
    conn: &Connection,
    workspace_id: i64,
    parent_id: Option<i64>,
    category_id: Option<i64>,
) -> Result<()> {
    let Some(parent_id) = parent_id else {
        return Ok(());
    };
    if Some(parent_id) == category_id {
        return Err(BusinessError::exception(
            "A product category cannot be its own parent",
            "productCategoryParentSelf",
            400,
        ));
    }
    let parent = repo::find(conn, workspace_id, parent_id)?
        .ok_or_else(|| BusinessError::not_found("Parent product category not found"))?;
    if parent.parent_id.is_some() {
        return Err(BusinessError::exception(
            "A sub-category cannot contain further sub-categories",
            "productCategoryNestingTooDeep",
            400,
        ));
    }
    Ok(())
}

/// `assertHasNoChildren` (only when it gets a parent).
fn assert_has_no_children(
    conn: &Connection,
    workspace_id: i64,
    category_id: i64,
    parent_id: Option<i64>,
) -> Result<()> {
    if parent_id.is_none() {
        return Ok(());
    }
    if !repo::list_children(conn, workspace_id, category_id)?.is_empty() {
        return Err(BusinessError::exception(
            "A category with sub-categories cannot itself become a sub-category",
            "productCategoryNestingTooDeep",
            400,
        ));
    }
    Ok(())
}

fn listed(conn: &Connection, workspace_id: i64) -> Result<Vec<CategoryWithCount>> {
    let source = resolve_store_source(conn, workspace_id)?;
    Ok(repo::list_with_product_count(conn, workspace_id, &source)?)
}

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
    format!(
        "{}\u{0}{}",
        lower(&path.category_name),
        lower(path.subcategory_name.as_deref().unwrap_or(""))
    )
}

/// `list`: the whole tree (rank, then name) with product counts, filtered.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    filter: ListFilter,
) -> Result<Vec<CategoryWithCount>> {
    tx::read(db, move |conn| {
        let keyword = filter.name.as_deref().map(lower).filter(|k| !k.is_empty());
        Ok(listed(conn, workspace_id)?
            .into_iter()
            .filter(|c| filter.parent_id.is_none_or(|p| c.parent_id == Some(p)))
            .filter(|c| {
                keyword
                    .as_ref()
                    .is_none_or(|k| c.name.to_lowercase().contains(k))
            })
            .collect())
    })
    .await
}

/// `listLevel`: one page of one level, each row with its child count.
pub async fn list_level(db: &Db, input: LevelInput) -> Result<Level> {
    tx::read(db, move |conn| {
        let all = listed(conn, input.workspace_id)?;
        let keyword = input.name.as_deref().map(lower).filter(|k| !k.is_empty());
        let level: Vec<&CategoryWithCount> = all
            .iter()
            .filter(|c| c.parent_id == input.parent_id)
            .filter(|c| {
                keyword
                    .as_ref()
                    .is_none_or(|k| c.name.to_lowercase().contains(k))
            })
            .collect();
        let child_count = |id: i64| all.iter().filter(|c| c.parent_id == Some(id)).count() as i64;
        let len = level.len() as i64;
        let per_page = input.per_page.unwrap_or(len);
        let start = ((input.page.unwrap_or(1) - 1) * per_page).max(0) as usize;
        let take = if per_page > 0 {
            per_page as usize
        } else {
            usize::MAX
        };
        let data = level
            .iter()
            .skip(start)
            .take(take)
            .map(|c| LevelRow {
                category: (*c).clone(),
                child_count: child_count(c.id),
            })
            .collect();
        let page_count = if per_page > 0 {
            ((len + per_page - 1) / per_page).max(1)
        } else {
            1
        };
        let parent = input
            .parent_id
            .and_then(|p| all.iter().find(|c| c.id == p).cloned());
        Ok(Level {
            data,
            page_count,
            parent,
        })
    })
    .await
}

/// `create`.
pub async fn create(db: &Db, input: CreateCategory) -> Result<ProductCategory> {
    tx::write(db, move |conn| {
        let source = resolve_store_source(conn, input.workspace_id)?;
        if is_external_store_source(&source) {
            return Err(external_store_managed(&source));
        }
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(name_required());
        }
        assert_parent_can_adopt(conn, input.workspace_id, input.parent_id, None)?;
        let row = repo::CreateCategoryInput {
            workspace_id: input.workspace_id,
            name,
            rank: input.rank,
            parent_id: input.parent_id,
            image_url: input.image_url,
            is_visible: input.is_visible,
        };
        repo::create(conn, &row)
            .map_err(duplicate_or)?
            .ok_or_else(|| {
                BusinessError::exception("Failed to create product category", "systemError", 400)
            })
    })
    .await
}

/// `update`.
pub async fn update(db: &Db, input: UpdateCategory) -> Result<ProductCategory> {
    tx::write(db, move |conn| {
        let only_visibility = input.name.is_none()
            && input.parent_id.is_none()
            && input.image_url.is_none()
            && input.rank.is_none();
        if !only_visibility {
            assert_managed_here(conn, input.workspace_id, input.category_id)?;
        }
        let name = input.name.as_deref().map(|n| n.trim().to_string());
        if name.as_deref() == Some("") {
            return Err(name_required());
        }
        if let Some(parent_id) = input.parent_id {
            assert_parent_can_adopt(conn, input.workspace_id, parent_id, Some(input.category_id))?;
            assert_has_no_children(conn, input.workspace_id, input.category_id, parent_id)?;
        }
        let before = match input.parent_id {
            Some(_) => repo::find(conn, input.workspace_id, input.category_id)?,
            None => None,
        };
        let updated = repo::update(
            conn,
            &repo::UpdateCategoryInput {
                workspace_id: input.workspace_id,
                category_id: input.category_id,
                name,
                parent_id: input.parent_id,
                image_url: input.image_url.clone(),
                is_visible: input.is_visible,
                rank: input.rank,
            },
        )
        .map_err(duplicate_or)?
        .ok_or_else(not_found)?;
        if let (Some(before), Some(parent_id)) = (before, input.parent_id)
            && before.parent_id != parent_id
        {
            match parent_id {
                Some(parent_id) => {
                    repo::file_under_parent(conn, input.workspace_id, input.category_id, parent_id)?
                }
                None => repo::file_as_top_level(conn, input.workspace_id, input.category_id)?,
            }
        }
        Ok(updated)
    })
    .await
}

/// `delete`: removes the sub-categories too (foreign key) and switches off the
/// category discount codes of the subtree first.
pub async fn delete(db: &Db, workspace_id: i64, category_id: i64) -> Result<()> {
    tx::write(db, move |conn| {
        assert_managed_here(conn, workspace_id, category_id)?;
        let mut guarded = vec![category_id];
        guarded.extend(
            repo::list_children(conn, workspace_id, category_id)?
                .iter()
                .map(|c| c.id),
        );
        discount_code::deactivate_by_category_ids(conn, workspace_id, &guarded)?;
        repo::delete(conn, workspace_id, category_id)?.ok_or_else(not_found)?;
        Ok(())
    })
    .await
}

/// `countProducts`.
pub async fn count_products(db: &Db, workspace_id: i64, category_id: i64) -> Result<i64> {
    tx::read(db, move |conn| {
        Ok(repo::count_products(conn, workspace_id, category_id)?)
    })
    .await
}

/// `resolveByNames`: top-level category ids keyed by lower-cased name.
pub fn resolve_by_names(
    conn: &Connection,
    workspace_id: i64,
    names: &[String],
    create_missing: bool,
) -> Result<HashMap<String, i64>> {
    // Last spelling of a lower-cased name wins (`new Map(...)`), first-seen order.
    let mut unique: Vec<(String, String)> = Vec::new();
    for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        match unique
            .iter_mut()
            .find(|(key, _)| *key == name.to_lowercase())
        {
            Some(entry) => entry.1 = name.to_string(),
            None => unique.push((name.to_lowercase(), name.to_string())),
        }
    }
    let top_level = |conn: &Connection| -> Result<HashMap<String, i64>> {
        Ok(repo::list(conn, workspace_id, OWN_SOURCE)?
            .into_iter()
            .filter(|row| row.parent_id.is_none())
            .map(|row| (lower(&row.name), row.id))
            .collect())
    };
    let mut resolved = top_level(conn)?;
    if create_missing {
        let missing: Vec<String> = unique
            .iter()
            .filter(|(key, _)| !resolved.contains_key(key))
            .map(|(_, name)| name.clone())
            .collect();
        if !missing.is_empty() {
            repo::create_missing_by_name(conn, workspace_id, &missing, None)?;
            resolved = top_level(conn)?;
        }
    }
    Ok(unique
        .into_iter()
        .filter_map(|(key, _)| resolved.get(&key).map(|id| (key, *id)))
        .collect())
}

/// `resolvePaths`: ids keyed by [`path_key`].
pub fn resolve_paths(
    conn: &Connection,
    workspace_id: i64,
    paths: &[CategoryPath],
    create_missing: bool,
) -> Result<HashMap<String, ResolvedPath>> {
    let mut unique: Vec<(String, CategoryPath)> = Vec::new();
    for path in paths {
        let normalized = CategoryPath {
            category_name: path.category_name.trim().to_string(),
            subcategory_name: path
                .subcategory_name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string),
        };
        if normalized.category_name.is_empty() {
            continue;
        }
        let key = path_key(&normalized);
        match unique.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = normalized,
            None => unique.push((key, normalized)),
        }
    }
    let rows = |conn: &Connection| -> Result<Vec<ProductCategory>> {
        Ok(repo::list(conn, workspace_id, OWN_SOURCE)?)
    };
    let top_level = |rows: &[ProductCategory]| -> HashMap<String, i64> {
        rows.iter()
            .filter(|row| row.parent_id.is_none())
            .map(|row| (lower(&row.name), row.id))
            .collect()
    };
    let mut current = rows(conn)?;
    if create_missing {
        let existing = top_level(&current);
        let mut missing: Vec<String> = Vec::new();
        for (_, path) in &unique {
            if !existing.contains_key(&lower(&path.category_name)) {
                missing.push(path.category_name.clone());
            }
        }
        if !missing.is_empty() {
            repo::create_missing_by_name(conn, workspace_id, &missing, None)?;
            current = rows(conn)?;
        }
        let refreshed = top_level(&current);
        let existing_children: Vec<String> = current
            .iter()
            .filter(|row| row.parent_id.is_some())
            .map(|row| child_key(row.parent_id, &row.name))
            .collect();
        let mut children: Vec<(i64, String)> = Vec::new();
        for (_, path) in &unique {
            let (Some(sub), Some(parent)) = (
                &path.subcategory_name,
                refreshed.get(&lower(&path.category_name)),
            ) else {
                continue;
            };
            if !existing_children.contains(&child_key(Some(*parent), sub)) {
                children.push((*parent, sub.clone()));
            }
        }
        if !children.is_empty() {
            repo::create_missing_children(conn, workspace_id, &children)?;
            current = rows(conn)?;
        }
    }
    let resolved = top_level(&current);
    let child_by_key: HashMap<String, i64> = current
        .iter()
        .filter(|row| row.parent_id.is_some())
        .map(|row| (child_key(row.parent_id, &row.name), row.id))
        .collect();
    Ok(unique
        .into_iter()
        .filter_map(|(key, path)| {
            let category_id = *resolved.get(&lower(&path.category_name))?;
            let subcategory_id = path.subcategory_name.as_deref().and_then(|sub| {
                child_by_key
                    .get(&child_key(Some(category_id), sub))
                    .copied()
            });
            Some((
                key,
                ResolvedPath {
                    category_id,
                    subcategory_id,
                },
            ))
        })
        .collect())
}
