//! Port of `repositories/place-location/repository.ts` and
//! `repositories/place-category/repository.ts` (P3.3).

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{PlaceCategory, PlaceLocation};
use super::patch::{Patch, insert_clause, set_clause};
use super::sql::Sql;
use super::utils::{PaginationInput, like_contains, parse_pagination};
use super::{Model, Result, new_id, now_ms, text_list};

/// A location with its category's name.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceLocationWithCategory {
    pub location: PlaceLocation,
    pub category_name: Option<String>,
}

/// Result of [`list_locations`].
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceLocationPage {
    pub data: Vec<PlaceLocationWithCategory>,
    pub page_count: i64,
}

/// `(id, name, latitude, longitude)` of [`list_location_coordinates`].
pub type Coordinates = (i64, String, f64, f64);

fn location_where(workspace_id: i64, name: Option<&str>) -> Sql {
    Sql::and([
        Some(Sql::bind("l.\"workspaceId\" = ", workspace_id)),
        name.filter(|n| !n.is_empty())
            .map(|n| Sql::bind("ilike(l.\"name\", ", like_contains(n)).closed(")")),
    ])
}

fn location_select(workspace_id: i64) -> Sql {
    Sql::bind(
        format!(
            "SELECT {}, c.\"name\" FROM \"PlaceLocation\" l LEFT JOIN \"PlaceCategory\" c \
             ON c.\"id\" = l.\"categoryId\" AND c.\"workspaceId\" = ",
            PlaceLocation::columns_sql(Some("l"))
        ),
        workspace_id,
    )
}

fn map_location(r: &rusqlite::Row<'_>) -> rusqlite::Result<PlaceLocationWithCategory> {
    Ok(PlaceLocationWithCategory {
        location: PlaceLocation::from_row(r)?,
        category_name: r.get(PlaceLocation::COLUMNS.len())?,
    })
}

/// `placeLocationRepository.list`: newest first; every row when no pagination is given.
pub fn list_locations(
    conn: &Connection,
    workspace_id: i64,
    name: Option<&str>,
    pagination: PaginationInput,
) -> Result<PlaceLocationPage> {
    let p = parse_pagination(pagination);
    let mut q = location_select(workspace_id);
    q.push(" WHERE ");
    q.append(location_where(workspace_id, name));
    q.push(" ORDER BY l.\"createdAt\" DESC, l.\"id\" DESC");
    if let Some(p) = p {
        q.push(" LIMIT ");
        q.push_bind(p.limit);
        q.push(" OFFSET ");
        q.push_bind(p.offset);
    }
    let data = q.query_map(conn, map_location)?;
    let mut c = Sql::raw("SELECT count(*) FROM \"PlaceLocation\" l WHERE ");
    c.append(location_where(workspace_id, name));
    let total = c
        .query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    let page_count = match p {
        Some(p) if p.limit > 0 => (total + p.limit - 1) / p.limit,
        _ => 1,
    };
    Ok(PlaceLocationPage { data, page_count })
}

/// `placeLocationRepository.find`.
pub fn find_location(
    conn: &Connection,
    workspace_id: i64,
    location_id: i64,
) -> Result<Option<PlaceLocationWithCategory>> {
    let mut q = location_select(workspace_id);
    q.push(" WHERE l.\"id\" = ");
    q.push_bind(location_id);
    q.push(" AND l.\"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(" LIMIT 1");
    Ok(q.query_map(conn, map_location)?.into_iter().next())
}

/// `placeLocationRepository.listCoordinates` (`category_id` `None` = every category).
pub fn list_location_coordinates(
    conn: &Connection,
    workspace_id: i64,
    category_id: Option<i64>,
) -> Result<Vec<Coordinates>> {
    let mut q = Sql::bind(
        "SELECT \"id\", \"name\", \"latitude\", \"longitude\" FROM \"PlaceLocation\" WHERE \"workspaceId\" = ",
        workspace_id,
    );
    if let Some(c) = category_id {
        q.push(" AND \"categoryId\" = ");
        q.push_bind(c);
    }
    q.query_map(conn, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
}

fn insert_row<M: Model>(conn: &Connection, workspace_id: i64, values: &Patch) -> Result<Option<M>> {
    let now = now_ms();
    let mut all: Patch = vec![
        ("id", Value::Integer(new_id())),
        ("createdAt", Value::Integer(now)),
        ("updatedAt", Value::Integer(now)),
        ("workspaceId", Value::Integer(workspace_id)),
    ];
    all.extend(values.iter().cloned());
    let mut q = Sql::raw(format!("INSERT INTO \"{}\" ", M::TABLE));
    q.append(insert_clause(M::COLUMNS, &all)?);
    q.push(&format!(" RETURNING {}", M::columns_sql(None)));
    q.model(conn)
}

fn update_row<M: Model>(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    values: &Patch,
) -> Result<Option<M>> {
    let mut all = values.clone();
    all.push(("updatedAt", Value::Integer(now_ms())));
    let mut q = Sql::raw(format!("UPDATE \"{}\" SET ", M::TABLE));
    q.append(set_clause(M::COLUMNS, &all)?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(&format!(" RETURNING {}", M::columns_sql(None)));
    q.model(conn)
}

fn delete_row<M: Model>(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<M>> {
    let sql = format!(
        "DELETE FROM \"{}\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        M::TABLE
    );
    Ok(super::returning::<M>(conn, &sql, &[&id, &workspace_id])?
        .into_iter()
        .next())
}

/// `placeLocationRepository.create` (`values` = name, latitude, longitude, categoryId).
pub fn create_location(
    conn: &Connection,
    workspace_id: i64,
    values: &Patch,
) -> Result<Option<PlaceLocation>> {
    insert_row(conn, workspace_id, values)
}

/// `placeLocationRepository.update`.
pub fn update_location(
    conn: &Connection,
    workspace_id: i64,
    location_id: i64,
    values: &Patch,
) -> Result<Option<PlaceLocation>> {
    update_row(conn, workspace_id, location_id, values)
}

/// `placeLocationRepository.delete`: the deleted row.
pub fn delete_location(
    conn: &Connection,
    workspace_id: i64,
    location_id: i64,
) -> Result<Option<PlaceLocation>> {
    delete_row(conn, workspace_id, location_id)
}

/// A category with its location count.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceCategoryWithCount {
    pub category: PlaceCategory,
    pub location_count: i64,
}

/// `placeCategoryRepository.listWithLocationCount`: by rank, name, id.
pub fn list_categories_with_location_count(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<PlaceCategoryWithCount>> {
    let sql = format!(
        "SELECT {}, count(l.\"id\") FROM \"PlaceCategory\" c LEFT JOIN \"PlaceLocation\" l \
         ON l.\"categoryId\" = c.\"id\" AND l.\"workspaceId\" = ?1 WHERE c.\"workspaceId\" = ?1 \
         GROUP BY c.\"id\" ORDER BY c.\"rank\" ASC, c.\"name\" ASC, c.\"id\" ASC",
        PlaceCategory::columns_sql(Some("c"))
    );
    let n = PlaceCategory::COLUMNS.len();
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map([workspace_id], |r| {
            Ok(PlaceCategoryWithCount {
                category: PlaceCategory::from_row(r)?,
                location_count: r.get(n)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `placeCategoryRepository.find`.
pub fn find_category(
    conn: &Connection,
    workspace_id: i64,
    category_id: i64,
) -> Result<Option<PlaceCategory>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&category_id, &workspace_id],
    )
}

/// `placeCategoryRepository.create` (`values` = name, rank).
pub fn create_category(
    conn: &Connection,
    workspace_id: i64,
    values: &Patch,
) -> Result<Option<PlaceCategory>> {
    insert_row(conn, workspace_id, values)
}

/// Result of [`find_or_create_categories_by_names`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CategoriesByName {
    /// Requested (trimmed) name -> id, matched case-insensitively.
    pub id_by_name: BTreeMap<String, i64>,
    pub created_ids: BTreeSet<i64>,
}

/// `placeCategoryRepository.findOrCreateByNames`.
pub fn find_or_create_categories_by_names(
    conn: &Connection,
    workspace_id: i64,
    names: &[&str],
) -> Result<CategoriesByName> {
    let mut unique: Vec<String> = Vec::new();
    for n in names {
        let t = n.trim().to_owned();
        if !unique.contains(&t) {
            unique.push(t);
        }
    }
    let mut out = CategoriesByName::default();
    if unique.is_empty() {
        return Ok(out);
    }
    let now = now_ms();
    let mut insert = conn.prepare_cached(
        "INSERT INTO \"PlaceCategory\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"name\") \
         VALUES (?1, ?2, ?2, ?3, ?4) ON CONFLICT DO NOTHING RETURNING \"id\"",
    )?;
    for name in &unique {
        let mut rows = insert.query(rusqlite::params![new_id(), now, workspace_id, name])?;
        if let Some(row) = rows.next()? {
            out.created_ids.insert(row.get(0)?);
        }
    }
    let mut stmt = conn.prepare_cached(
        "SELECT requested.value, c.\"id\" FROM json_each(?2) AS requested \
         JOIN \"PlaceCategory\" c ON c.\"workspaceId\" = ?1 AND lower_u(c.\"name\") = lower_u(requested.value)",
    )?;
    let matched = stmt
        .query_map(rusqlite::params![workspace_id, text_list(&unique)], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    out.id_by_name.extend(matched);
    Ok(out)
}

/// `placeCategoryRepository.update`.
pub fn update_category(
    conn: &Connection,
    workspace_id: i64,
    category_id: i64,
    values: &Patch,
) -> Result<Option<PlaceCategory>> {
    update_row(conn, workspace_id, category_id, values)
}

/// `placeCategoryRepository.countLocations`.
pub fn count_category_locations(
    conn: &Connection,
    workspace_id: i64,
    category_id: i64,
) -> Result<i64> {
    super::client::count_where(
        conn,
        "PlaceLocation",
        Some("\"categoryId\" = ?1 AND \"workspaceId\" = ?2"),
        &[&category_id, &workspace_id],
    )
}

/// `placeCategoryRepository.delete`: the deleted row.
pub fn delete_category(
    conn: &Connection,
    workspace_id: i64,
    category_id: i64,
) -> Result<Option<PlaceCategory>> {
    delete_row(conn, workspace_id, category_id)
}
