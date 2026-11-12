//! Port of `repositories/vendor/repository.ts` (P3.3). `lower()` is the Unicode
//! `lower_u` app function (the unique index uses it too).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::Vendor;
use super::patch::{Patch, insert_clause, set_clause};
use super::sql::Sql;
use super::utils::{PaginationInput, like_contains, parse_pagination};
use super::{Model, Result, new_id, now_ms, text_list};

/// Default `StoreSource`.
pub const DEFAULT_SOURCE: &str = "chatbotx";

/// A vendor with its product count.
#[derive(Debug, Clone, PartialEq)]
pub struct VendorWithCount {
    pub vendor: Vendor,
    pub product_count: i64,
}

/// Result of [`list_with_product_count`].
#[derive(Debug, Clone, PartialEq)]
pub struct VendorPage {
    pub data: Vec<VendorWithCount>,
    pub page_count: i64,
}

fn list_where(workspace_id: i64, source: Option<&str>, name: Option<&str>) -> Sql {
    Sql::and([
        Some(Sql::bind("v.\"workspaceId\" = ", workspace_id)),
        Some(Sql::bind(
            "v.\"source\" = ",
            source.unwrap_or(DEFAULT_SOURCE).to_owned(),
        )),
        name.filter(|n| !n.is_empty())
            .map(|n| Sql::bind("ilike(v.\"name\", ", like_contains(n)).closed(")")),
    ])
}

/// `listWithProductCount`: by rank, name, id; every row when no pagination is given.
pub fn list_with_product_count(
    conn: &Connection,
    workspace_id: i64,
    source: Option<&str>,
    name: Option<&str>,
    pagination: PaginationInput,
) -> Result<VendorPage> {
    let p = parse_pagination(pagination);
    let mut q = Sql::bind(
        format!(
            "SELECT {}, count(p.\"id\") FROM \"Vendor\" v LEFT JOIN \"Product\" p ON p.\"vendorId\" = v.\"id\" \
             AND p.\"workspaceId\" = ",
            Vendor::columns_sql(Some("v"))
        ),
        workspace_id,
    );
    q.push(" WHERE ");
    q.append(list_where(workspace_id, source, name));
    q.push(" GROUP BY v.\"id\" ORDER BY v.\"rank\" ASC, v.\"name\" ASC, v.\"id\" ASC");
    if let Some(p) = p {
        q.push(" LIMIT ");
        q.push_bind(p.limit);
        q.push(" OFFSET ");
        q.push_bind(p.offset);
    }
    let n = Vendor::COLUMNS.len();
    let data = q.query_map(conn, |r| {
        Ok(VendorWithCount {
            vendor: Vendor::from_row(r)?,
            product_count: r.get(n)?,
        })
    })?;
    let mut c = Sql::raw("SELECT count(*) FROM \"Vendor\" v WHERE ");
    c.append(list_where(workspace_id, source, name));
    let total = c
        .query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    let page_count = match p {
        Some(p) if p.limit > 0 => (total + p.limit - 1) / p.limit,
        _ => 1,
    };
    Ok(VendorPage { data, page_count })
}

/// `listOptions`: `(id, name)` by rank, name.
pub fn list_options(
    conn: &Connection,
    workspace_id: i64,
    source: Option<&str>,
) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"name\" FROM \"Vendor\" WHERE \"workspaceId\" = ?1 AND \"source\" = ?2 ORDER BY \"rank\" ASC, \"name\" ASC",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_id, source.unwrap_or(DEFAULT_SOURCE)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `find`.
pub fn find(conn: &Connection, workspace_id: i64, vendor_id: i64) -> Result<Option<Vendor>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&vendor_id, &workspace_id],
    )
}

fn lowered_names(names: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        let l = crate::functions::lower_u(n.trim());
        if !l.is_empty() && !out.contains(&l) {
            out.push(l);
        }
    }
    out
}

/// `findByNames`: `(id, name)` matched case-insensitively on the trimmed names.
pub fn find_by_names(
    conn: &Connection,
    workspace_id: i64,
    names: &[&str],
    source: Option<&str>,
) -> Result<Vec<(i64, String)>> {
    let lowered = lowered_names(names);
    if lowered.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"name\" FROM \"Vendor\" WHERE \"workspaceId\" = ?1 AND \"source\" = ?2 \
         AND lower_u(\"name\") IN (SELECT value FROM json_each(?3))",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                workspace_id,
                source.unwrap_or(DEFAULT_SOURCE),
                text_list(&lowered)
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `createMissing`: inserts the trimmed names (first spelling per lower-cased name);
/// existing vendors are kept.
pub fn create_missing(
    conn: &Connection,
    workspace_id: i64,
    names: &[&str],
    source: Option<&str>,
) -> Result<()> {
    let mut by_lower: Vec<(String, String)> = Vec::new();
    for n in names {
        let trimmed = n.trim();
        let lower = crate::functions::lower_u(trimmed);
        if !trimmed.is_empty() && !by_lower.iter().any(|(l, _)| *l == lower) {
            by_lower.push((lower, trimmed.to_owned()));
        }
    }
    let now = now_ms();
    let mut stmt = conn.prepare_cached(
        "INSERT INTO \"Vendor\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"name\", \"source\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING",
    )?;
    for (_, name) in by_lower {
        stmt.execute(rusqlite::params![
            new_id(),
            now,
            workspace_id,
            name,
            source.unwrap_or(DEFAULT_SOURCE)
        ])?;
    }
    Ok(())
}

/// `create` (`values` = the vendor columns besides id, workspace and timestamps).
pub fn create(conn: &Connection, workspace_id: i64, values: &Patch) -> Result<Option<Vendor>> {
    let now = now_ms();
    let mut all: Patch = vec![
        ("id", Value::Integer(new_id())),
        ("createdAt", Value::Integer(now)),
        ("updatedAt", Value::Integer(now)),
        ("workspaceId", Value::Integer(workspace_id)),
    ];
    all.extend(values.iter().cloned());
    let mut q = Sql::raw("INSERT INTO \"Vendor\" ");
    q.append(insert_clause(Vendor::COLUMNS, &all)?);
    q.push(&format!(" RETURNING {}", Vendor::columns_sql(None)));
    q.model(conn)
}

/// `update`.
pub fn update(
    conn: &Connection,
    workspace_id: i64,
    vendor_id: i64,
    values: &Patch,
) -> Result<Option<Vendor>> {
    let mut all = values.clone();
    all.push(("updatedAt", Value::Integer(now_ms())));
    let mut q = Sql::raw("UPDATE \"Vendor\" SET ");
    q.append(set_clause(Vendor::COLUMNS, &all)?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(vendor_id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(&format!(" RETURNING {}", Vendor::columns_sql(None)));
    q.model(conn)
}

/// `delete`: the deleted row.
pub fn delete(conn: &Connection, workspace_id: i64, vendor_id: i64) -> Result<Option<Vendor>> {
    Ok(super::returning::<Vendor>(
        conn,
        "DELETE FROM \"Vendor\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&vendor_id, &workspace_id],
    )?
    .into_iter()
    .next())
}
