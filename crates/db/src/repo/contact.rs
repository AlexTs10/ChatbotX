//! Port of `repositories/contact/repository.ts` (part: `backdateCreatedAt`,
//! `blockManyIfNotBlocked`, `listForExportPage`; the filter-based functions wait for the
//! contact-filter port, see `conversion/specs/P3.2.md`).

use rusqlite::Connection;

use super::models::Contact;
use super::sql::Sql;
use super::{Model, Result, id_list};

/// `backdateCreatedAt`: moves `createdAt` back (never forward) per `(contactId,
/// workspaceId, createdAt)` row; returns the ids that changed. Workspace-scoped.
pub fn backdate_created_at(conn: &Connection, rows: &[(i64, i64, i64)]) -> Result<Vec<i64>> {
    let mut changed = Vec::new();
    let mut stmt = conn.prepare_cached(
        "UPDATE \"Contact\" SET \"createdAt\" = min(\"createdAt\", ?3) \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 AND \"createdAt\" > ?3 RETURNING \"id\"",
    )?;
    for (contact_id, workspace_id, created_at) in rows {
        let ids = stmt
            .query_map(
                rusqlite::params![contact_id, workspace_id, created_at],
                |r| r.get::<_, i64>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        changed.extend(ids);
    }
    Ok(changed)
}

/// `blockManyIfNotBlocked`: sets `blockedAt` on the not-yet-blocked contacts of the
/// workspace; returns the ids that really transitioned.
pub fn block_many_if_not_blocked(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
    now: i64,
) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "UPDATE \"Contact\" SET \"blockedAt\" = ?3 WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2)) \
         AND \"blockedAt\" IS NULL RETURNING \"id\"",
    )?;
    let out = stmt
        .query_map(rusqlite::params![workspace_id, id_list(ids), now], |r| {
            r.get(0)
        })?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(out)
}

/// Which `ContactInbox` rows the export loads (`ContactExportRelations.contactInboxes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportInboxes {
    None,
    Earliest,
    All,
}

/// `ContactExportRelations`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRelations {
    pub custom_field_ids: Vec<i64>,
    pub tag_ids: Vec<i64>,
    pub contact_inboxes: ExportInboxes,
}

/// `ContactExportRow`: the contact plus only the relations a selected column needs
/// (`None` = not loaded).
#[derive(Debug, Clone, PartialEq)]
pub struct ExportRow {
    pub contact: Contact,
    /// `(customFieldId, value)` of the selected custom fields.
    pub contact_custom_fields: Option<Vec<(i64, Option<String>)>>,
    /// Ids of the selected tags the contact has.
    pub tags: Option<Vec<i64>>,
    /// `(sourceId, sourceUserId)` by id (one row for `Earliest`).
    pub contact_inboxes: Option<Vec<(String, Option<String>)>>,
}

/// `listForExportPage`: one keyset page (`filter` over `c`, by id), with the relations.
pub fn list_for_export_page(
    conn: &Connection,
    filter: &Sql,
    limit: i64,
    relations: &ExportRelations,
) -> Result<Vec<ExportRow>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Contact\" AS c WHERE ",
        Contact::columns_sql(Some("c"))
    ));
    q.append(filter.clone());
    q.push(" ORDER BY c.\"id\" ASC LIMIT ");
    q.push_bind(limit);
    let contacts: Vec<Contact> = q.models(conn)?;
    let mut out = Vec::with_capacity(contacts.len());
    for contact in contacts {
        let contact_custom_fields = if relations.custom_field_ids.is_empty() {
            None
        } else {
            let mut stmt = conn.prepare_cached(
                "SELECT \"customFieldId\", \"value\" FROM \"ContactCustomField\" WHERE \"contactId\" = ?1 \
                 AND \"customFieldId\" IN (SELECT value FROM json_each(?2)) ORDER BY \"customFieldId\"",
            )?;
            let rows = stmt
                .query_map(
                    rusqlite::params![contact.id, id_list(&relations.custom_field_ids)],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Some(rows)
        };
        let tags = if relations.tag_ids.is_empty() {
            None
        } else {
            let mut stmt = conn.prepare_cached(
                "SELECT \"tagId\" FROM \"ContactToTag\" WHERE \"contactId\" = ?1 AND \"tagId\" IN (SELECT value FROM json_each(?2)) ORDER BY \"tagId\"",
            )?;
            let rows = stmt
                .query_map(
                    rusqlite::params![contact.id, id_list(&relations.tag_ids)],
                    |r| r.get(0),
                )?
                .collect::<rusqlite::Result<Vec<i64>>>()?;
            Some(rows)
        };
        let contact_inboxes = match relations.contact_inboxes {
            ExportInboxes::None => None,
            mode => {
                let limit = if mode == ExportInboxes::Earliest {
                    1
                } else {
                    -1
                };
                let mut stmt = conn.prepare_cached(
                    "SELECT \"sourceId\", \"sourceUserId\" FROM \"ContactInbox\" WHERE \"contactId\" = ?1 ORDER BY \"id\" ASC LIMIT ?2",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![contact.id, limit], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Some(rows)
            }
        };
        out.push(ExportRow {
            contact,
            contact_custom_fields,
            tags,
            contact_inboxes,
        });
    }
    Ok(out)
}

/// `findIdsMatchingFilter`: the ids of `contact_ids` (one broadcast audience scan page) of
/// the workspace that match the filter (`buildContactFilterMatchSQL`); `[]` for an empty page.
pub fn find_ids_matching_filter(
    _conn: &Connection,
    _workspace_id: i64,
    _contact_ids: &[i64],
    _criteria: &crate::repo::queries::contact_filter::FilterCriteria,
    _now: i64,
) -> Result<Vec<i64>> {
    todo!("P3.2b")
}
