//! Port of `repositories/mini-app/repository.ts` (P3.3).

use rusqlite::Connection;

use super::models::{MiniApp, MiniAppPublication, MiniAppSubmission};
use super::sql::Sql;
use super::utils::{
    PaginationInput, SortItem, escape_like_pattern, get_pagination_with_defaults, like_contains,
    order_by_sql, parse_order_by,
};
use super::{Model, Result};

/// `MiniAppListInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MiniAppListInput {
    pub workspace_id: i64,
    pub keyword: Option<String>,
    pub page: i64,
    pub per_page: i64,
    pub sort: Option<Vec<SortItem>>,
}

/// A mini app without `definition` / `flowJson`, with its publications' summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiniAppListRow {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub workspace_id: i64,
    pub name: String,
    pub enabled: bool,
    pub submissions_count: i64,
    pub publications: Vec<PublicationSummary>,
}

/// The publication columns `listPaginated` reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationSummary {
    pub id: i64,
    pub integration_whatsapp_id: i64,
    pub source_id: String,
    pub status: String,
    pub published_at: Option<i64>,
}

/// A mini app with every publication.
#[derive(Debug, Clone, PartialEq)]
pub struct MiniAppWithPublications {
    pub mini_app: MiniApp,
    pub publications: Vec<MiniAppPublication>,
}

/// A submission with `contact { id, fullName, avatar }`.
#[derive(Debug, Clone, PartialEq)]
pub struct SubmissionWithContact {
    pub submission: MiniAppSubmission,
    pub contact: Option<(i64, Option<String>, Option<String>)>,
}

fn build_where(workspace_id: i64, keyword: Option<&str>) -> Sql {
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", workspace_id)),
        keyword
            .filter(|k| !k.is_empty())
            .map(|k| Sql::bind("ilike(\"name\", ", like_contains(k)).closed(")")),
    ])
}

/// `miniAppRepository.listPaginated`.
pub fn list_paginated(conn: &Connection, input: &MiniAppListInput) -> Result<Vec<MiniAppListRow>> {
    let p = get_pagination_with_defaults(PaginationInput {
        page: Some(input.page),
        per_page: Some(input.per_page),
    });
    let order = parse_order_by(MiniApp::COLUMNS, input.sort.as_deref(), None);
    let mut q = Sql::raw(
        "SELECT \"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"name\", \"enabled\", \"submissionsCount\" \
         FROM \"MiniApp\" WHERE ",
    );
    q.append(build_where(input.workspace_id, input.keyword.as_deref()));
    q.push(" ");
    q.push(&order_by_sql(&order, None));
    q.push(" LIMIT ");
    q.push_bind(p.limit);
    q.push(" OFFSET ");
    q.push_bind(p.offset);
    let mut rows = q.query_map(conn, |r| {
        Ok(MiniAppListRow {
            id: r.get(0)?,
            created_at: r.get(1)?,
            updated_at: r.get(2)?,
            workspace_id: r.get(3)?,
            name: r.get(4)?,
            enabled: r.get(5)?,
            submissions_count: r.get(6)?,
            publications: Vec::new(),
        })
    })?;
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"integrationWhatsappId\", \"sourceId\", \"status\", \"publishedAt\" \
         FROM \"MiniAppPublication\" WHERE \"miniAppId\" = ?1 ORDER BY \"id\"",
    )?;
    for row in &mut rows {
        row.publications = stmt
            .query_map([row.id], |r| {
                Ok(PublicationSummary {
                    id: r.get(0)?,
                    integration_whatsapp_id: r.get(1)?,
                    source_id: r.get(2)?,
                    status: r.get(3)?,
                    published_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
    }
    Ok(rows)
}

/// `miniAppRepository.count`.
pub fn count(conn: &Connection, workspace_id: i64, keyword: Option<&str>) -> Result<i64> {
    let mut q = Sql::raw("SELECT count(*) FROM \"MiniApp\" WHERE ");
    q.append(build_where(workspace_id, keyword));
    Ok(q.query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}

/// `miniAppRepository.findByIdAndWorkspace`.
pub fn find_by_id_and_workspace(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<MiniAppWithPublications>> {
    let Some(mini_app) = super::find_one::<MiniApp>(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )?
    else {
        return Ok(None);
    };
    let publications = super::find_many(conn, "WHERE \"miniAppId\" = ?1 ORDER BY \"id\"", &[&id])?;
    Ok(Some(MiniAppWithPublications {
        mini_app,
        publications,
    }))
}

/// `miniAppRepository.listNamesWithPrefix` (case-sensitive `LIKE`).
pub fn list_names_with_prefix(
    conn: &Connection,
    workspace_id: i64,
    prefix: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"name\" FROM \"MiniApp\" WHERE \"workspaceId\" = ?1 AND like_cs(\"name\", ?2)",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_id, format!("{}%", escape_like_pattern(prefix))],
            |r| r.get(0),
        )?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

/// `miniAppRepository.findById` (unscoped).
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<MiniApp>> {
    super::find_one(conn, "\"id\" = ?1", &[&id])
}

/// `miniAppSubmissionRepository.listPaginated`: newest first, with the contact.
pub fn list_submissions(
    conn: &Connection,
    workspace_id: i64,
    mini_app_id: i64,
    page: i64,
    per_page: i64,
) -> Result<Vec<SubmissionWithContact>> {
    let p = get_pagination_with_defaults(PaginationInput {
        page: Some(page),
        per_page: Some(per_page),
    });
    let sql = format!(
        "SELECT {}, c.\"id\", c.\"fullName\", c.\"avatar\" FROM \"MiniAppSubmission\" s \
         LEFT JOIN \"Contact\" c ON c.\"id\" = s.\"contactId\" \
         WHERE s.\"workspaceId\" = ?1 AND s.\"miniAppId\" = ?2 ORDER BY s.\"createdAt\" DESC LIMIT ?3 OFFSET ?4",
        MiniAppSubmission::columns_sql(Some("s"))
    );
    let n = MiniAppSubmission::COLUMNS.len();
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_id, mini_app_id, p.limit, p.offset],
            |r| {
                let contact = match r.get::<_, Option<i64>>(n)? {
                    Some(id) => Some((id, r.get(n + 1)?, r.get(n + 2)?)),
                    None => None,
                };
                Ok(SubmissionWithContact {
                    submission: MiniAppSubmission::from_row(r)?,
                    contact,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `miniAppSubmissionRepository.count`.
pub fn count_submissions(conn: &Connection, workspace_id: i64, mini_app_id: i64) -> Result<i64> {
    super::client::count_where(
        conn,
        "MiniAppSubmission",
        Some("\"workspaceId\" = ?1 AND \"miniAppId\" = ?2"),
        &[&workspace_id, &mini_app_id],
    )
}
