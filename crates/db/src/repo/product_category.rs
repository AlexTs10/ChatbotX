//! Port of `repositories/product-category/repository.ts` (`productCategoryRepository`; no
//! TS tests, smoke test `tests/repo_p33_smoke.rs`). Every UPDATE of a category sets
//! `"updatedAt"` (Drizzle `$onUpdate`). The conflict target
//! `(workspaceId, source, parentId, name)` is the expression index
//! `ProductCategory_workspaceId_parent_name_key` (`ifnull(parentId, 0)`), so the inserts use
//! a bare `ON CONFLICT DO NOTHING` (the table has no other unique key besides the id).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::ProductCategory;
use super::sql::{Sql, bool_val, opt_int, opt_text};
use super::{Model, Result, find_many, find_one, new_id, now_ms, returning, text_list};

/// One row of `listWithProductCount`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryWithCount {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub image_url: Option<String>,
    pub is_visible: bool,
    pub rank: i64,
    pub product_count: i64,
}

/// Input of `create`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateCategoryInput {
    pub workspace_id: i64,
    pub name: String,
    pub rank: Option<i64>,
    pub parent_id: Option<i64>,
    pub image_url: Option<String>,
    pub is_visible: Option<bool>,
}

/// Input of `update` (`None` = leave the column as is; `Some(None)` = set NULL).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateCategoryInput {
    pub workspace_id: i64,
    pub category_id: i64,
    pub name: Option<String>,
    pub parent_id: Option<Option<i64>>,
    pub image_url: Option<Option<String>>,
    pub is_visible: Option<bool>,
    pub rank: Option<i64>,
}

/// `listWithProductCount`: the categories of a source with their product count.
pub fn list_with_product_count(
    conn: &Connection,
    workspace_id: i64,
    source: &str,
) -> Result<Vec<CategoryWithCount>> {
    Sql::new(
        "SELECT c.\"id\", c.\"parentId\", c.\"name\", c.\"imageUrl\", c.\"isVisible\", c.\"rank\", count(p.\"id\") FROM \"ProductCategory\" c LEFT JOIN \"Product\" p ON (p.\"categoryId\" = c.\"id\" OR p.\"subcategoryId\" = c.\"id\") AND p.\"workspaceId\" = ? WHERE c.\"workspaceId\" = ? AND c.\"source\" = ? GROUP BY c.\"id\" ORDER BY c.\"rank\" ASC, c.\"name\" ASC",
        vec![
            Value::Integer(workspace_id),
            Value::Integer(workspace_id),
            Value::Text(source.to_owned()),
        ],
    )
    .query_map(conn, |row| {
        Ok(CategoryWithCount {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            name: row.get(2)?,
            image_url: row.get(3)?,
            is_visible: row.get(4)?,
            rank: row.get(5)?,
            product_count: row.get(6)?,
        })
    })
}

/// `listChildren`.
pub fn list_children(
    conn: &Connection,
    workspace_id: i64,
    parent_id: i64,
) -> Result<Vec<ProductCategory>> {
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"parentId\" = ?2 ORDER BY \"rank\" ASC, \"name\" ASC",
        &[&workspace_id, &parent_id],
    )
}

/// `list`: the categories of a source (default `chatbotx`).
pub fn list(conn: &Connection, workspace_id: i64, source: &str) -> Result<Vec<ProductCategory>> {
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"source\" = ?2 ORDER BY \"rank\" ASC, \"name\" ASC",
        &[&workspace_id, &source],
    )
}

/// `find`.
pub fn find(
    conn: &Connection,
    workspace_id: i64,
    category_id: i64,
) -> Result<Option<ProductCategory>> {
    find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&category_id, &workspace_id],
    )
}

/// `create` (`rank` 10, visible, top level by default).
pub fn create(conn: &Connection, input: &CreateCategoryInput) -> Result<Option<ProductCategory>> {
    Ok(returning::<ProductCategory>(
        conn,
        "INSERT INTO \"ProductCategory\" (\"id\", \"workspaceId\", \"name\", \"rank\", \"parentId\", \"imageUrl\", \"isVisible\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        &[
            &new_id(),
            &input.workspace_id,
            &input.name,
            &input.rank.unwrap_or(10),
            &input.parent_id,
            &input.image_url,
            &input.is_visible.unwrap_or(true),
        ],
    )?
    .into_iter()
    .next())
}

/// `update`: sets the given columns (and `updatedAt`).
pub fn update(conn: &Connection, input: &UpdateCategoryInput) -> Result<Option<ProductCategory>> {
    let mut sets: Vec<(&str, Value)> = Vec::new();
    if let Some(name) = &input.name {
        sets.push(("name", Value::Text(name.clone())));
    }
    if let Some(image_url) = &input.image_url {
        sets.push(("imageUrl", opt_text(image_url.as_deref())));
    }
    if let Some(is_visible) = input.is_visible {
        sets.push(("isVisible", bool_val(is_visible)));
    }
    if let Some(rank) = input.rank {
        sets.push(("rank", Value::Integer(rank)));
    }
    if let Some(parent_id) = input.parent_id {
        sets.push(("parentId", opt_int(parent_id)));
    }
    sets.push(("updatedAt", Value::Integer(now_ms())));
    let mut sql = Sql::raw("UPDATE \"ProductCategory\" SET ");
    for (i, (col, value)) in sets.into_iter().enumerate() {
        if i > 0 {
            sql.push(", ");
        }
        sql.push(&format!("\"{col}\" = "));
        sql.push_bind(value);
    }
    sql.push(" WHERE \"id\" = ");
    sql.push_bind(input.category_id);
    sql.push(" AND \"workspaceId\" = ");
    sql.push_bind(input.workspace_id);
    sql.push(&format!(
        " RETURNING {}",
        ProductCategory::columns_sql(None)
    ));
    sql.model::<ProductCategory>(conn)
}

/// `fileUnderParent`: products and vendors of `category_id` move under `parent_id` (the
/// category becomes their subcategory).
pub fn file_under_parent(
    conn: &Connection,
    workspace_id: i64,
    category_id: i64,
    parent_id: i64,
) -> Result<()> {
    for table in ["Product", "Vendor"] {
        conn.execute(
            &format!(
                "UPDATE \"{table}\" SET \"categoryId\" = ?1, \"subcategoryId\" = ?2, \"updatedAt\" = ?4 WHERE \"workspaceId\" = ?3 AND (\"categoryId\" = ?2 OR \"subcategoryId\" = ?2)"
            ),
            (parent_id, category_id, workspace_id, now_ms()),
        )?;
    }
    Ok(())
}

/// `fileAsTopLevel`: products and vendors filed under `category_id` as a subcategory now
/// have it as their category.
pub fn file_as_top_level(conn: &Connection, workspace_id: i64, category_id: i64) -> Result<()> {
    for table in ["Product", "Vendor"] {
        conn.execute(
            &format!(
                "UPDATE \"{table}\" SET \"categoryId\" = ?1, \"subcategoryId\" = NULL, \"updatedAt\" = ?3 WHERE \"workspaceId\" = ?2 AND \"subcategoryId\" = ?1"
            ),
            (category_id, workspace_id, now_ms()),
        )?;
    }
    Ok(())
}

/// `delete`: the deleted row, if any.
pub fn delete(
    conn: &Connection,
    workspace_id: i64,
    category_id: i64,
) -> Result<Option<ProductCategory>> {
    Ok(returning::<ProductCategory>(
        conn,
        "DELETE FROM \"ProductCategory\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&category_id, &workspace_id],
    )?
    .into_iter()
    .next())
}

/// `countProducts`: products of the workspace in the category (as category or subcategory).
pub fn count_products(conn: &Connection, workspace_id: i64, category_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM \"Product\" WHERE \"workspaceId\" = ?1 AND (\"categoryId\" = ?2 OR \"subcategoryId\" = ?2)",
        (workspace_id, category_id),
        |row| row.get(0),
    )?)
}

fn unique_trimmed(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        let name = name.trim();
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
    }
    out
}

/// `createMissingByName`: inserts the top-level categories that do not exist yet; returns
/// every top-level category with one of the (trimmed) names.
pub fn create_missing_by_name(
    conn: &Connection,
    workspace_id: i64,
    names: &[String],
    source: Option<&str>,
) -> Result<Vec<ProductCategory>> {
    let names = unique_trimmed(names);
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let source = source.unwrap_or("chatbotx");
    for name in &names {
        conn.execute(
            "INSERT INTO \"ProductCategory\" (\"id\", \"workspaceId\", \"name\", \"source\") VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
            (new_id(), workspace_id, name, source),
        )?;
    }
    find_by_names(conn, workspace_id, &names, Some(source))
}

/// `createMissingChildren`: inserts the `(parent, name)` children that do not exist yet.
pub fn create_missing_children(
    conn: &Connection,
    workspace_id: i64,
    children: &[(i64, String)],
) -> Result<()> {
    let mut seen: Vec<(i64, String)> = Vec::new();
    for (parent_id, name) in children {
        let name = name.trim().to_owned();
        if name.is_empty() {
            continue;
        }
        if let Some(slot) = seen.iter_mut().find(|(p, n)| p == parent_id && *n == name) {
            *slot = (*parent_id, name);
        } else {
            seen.push((*parent_id, name));
        }
    }
    for (parent_id, name) in &seen {
        conn.execute(
            "INSERT INTO \"ProductCategory\" (\"id\", \"workspaceId\", \"parentId\", \"name\") VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
            (new_id(), workspace_id, parent_id, name),
        )?;
    }
    Ok(())
}

/// `findByNames`: top-level categories of a source with one of these names.
pub fn find_by_names(
    conn: &Connection,
    workspace_id: i64,
    names: &[String],
    source: Option<&str>,
) -> Result<Vec<ProductCategory>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"parentId\" IS NULL AND \"name\" IN (SELECT value FROM json_each(?2)) AND \"source\" = ?3",
        &[
            &workspace_id,
            &text_list(names),
            &source.unwrap_or("chatbotx"),
        ],
    )
}
