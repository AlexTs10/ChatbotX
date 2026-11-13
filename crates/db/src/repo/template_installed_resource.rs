//! Port of `repositories/template-installed-resource/kinds.ts` and `repository.ts`
//! (`templateInstalledResourceRepository`).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::TemplateInstalledResource;
use super::sql::Sql;
use super::{Model, Result, id_list, text_list};

/// `templateResourceKinds`.
pub const TEMPLATE_RESOURCE_KINDS: [&str; 23] = [
    "flow",
    "aiAgent",
    "aiFunction",
    "calendar",
    "integrationWebchat",
    "automatedResponse",
    "reflink",
    "trigger",
    "fbCommentAutomation",
    "product",
    "savedReply",
    "botField",
    "customField",
    "tag",
    "folder",
    "pipeline",
    "productCategory",
    "placeCategory",
    "sequence",
    "aiMcpServer",
    "aiFile",
    "questionnaire",
    "couponTopic",
];

/// The permission a lock check is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateResourcePermission {
    AllowEdit,
    AllowDelete,
}

/// One `(kind, id)` key of `findLiveKeys`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceKey {
    pub kind: String,
    pub resource_id: i64,
}

/// One row of `findPriorCopies`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorCopy {
    pub source_resource_id: String,
    pub resource_id: i64,
}

/// One row of `listUnknownResourceKinds`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownResourceKind {
    pub resource_kind: String,
    pub rows: i64,
}

/// Result of `listPage`.
#[derive(Debug, Clone, PartialEq)]
pub struct InstalledResourcePage {
    pub rows: Vec<TemplateInstalledResource>,
    pub next_cursor: Option<i64>,
}

/// Input of `listPage`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListPageInput {
    pub installation_id: i64,
    pub workspace_id: i64,
    pub after_id: Option<i64>,
    pub limit: i64,
    pub category: Option<String>,
}

/// `keepKnownResourceKinds`: keeps the rows of a known kind; logs and skips the rest.
pub fn keep_known_resource_kinds(
    rows: Vec<TemplateInstalledResource>,
) -> Vec<TemplateInstalledResource> {
    rows.into_iter()
        .filter(|row| {
            let known = TEMPLATE_RESOURCE_KINDS.contains(&row.resource_kind.as_str());
            if !known {
                tracing::warn!(
                    row_id = %row.id,
                    resource_kind = %row.resource_kind,
                    "Skipping a template-installed resource of an unknown kind"
                );
            }
            known
        })
        .collect()
}

/// `listUnknownResourceKinds`: unknown `resourceKind` values with their row counts.
pub fn list_unknown_resource_kinds(conn: &Connection) -> Result<Vec<UnknownResourceKind>> {
    Sql::new(
        "SELECT \"resourceKind\", count(*) AS \"rows\" FROM \"TemplateInstalledResource\" WHERE \"resourceKind\" NOT IN (SELECT value FROM json_each(?)) GROUP BY 1 ORDER BY 2 DESC",
        vec![Value::Text(text_list(&TEMPLATE_RESOURCE_KINDS[..]))],
    )
    .query_map(conn, |row| {
        Ok(UnknownResourceKind {
            resource_kind: row.get(0)?,
            rows: row.get(1)?,
        })
    })
}

/// `findBlocking`: the ids, among `resource_ids`, that an active template-installed copy
/// locks against `permission`.
pub fn find_blocking(
    conn: &Connection,
    workspace_id: i64,
    resource_kind: &str,
    resource_ids: &[i64],
    permission: TemplateResourcePermission,
    exclude_installation_ids: &[i64],
) -> Result<Vec<i64>> {
    if resource_ids.is_empty() {
        return Ok(Vec::new());
    }
    let column = match permission {
        TemplateResourcePermission::AllowEdit => "allowEdit",
        TemplateResourcePermission::AllowDelete => "allowDelete",
    };
    let rows: Vec<i64> = Sql::new(
        format!(
            "SELECT r.\"resourceId\" FROM \"TemplateInstalledResource\" r JOIN \"TemplateInstallation\" i ON i.\"id\" = r.\"installationId\" WHERE r.\"workspaceId\" = ? AND r.\"resourceKind\" = ? AND r.\"resourceId\" IN (SELECT value FROM json_each(?)) AND r.\"wasExisting\" = 0 AND r.\"detachedAt\" IS NULL AND r.\"{column}\" = 0 AND r.\"installationId\" NOT IN (SELECT value FROM json_each(?)) AND i.\"sourceDeletedAt\" IS NULL AND i.\"templateId\" IS NOT NULL"
        ),
        vec![
            Value::Integer(workspace_id),
            Value::Text(resource_kind.to_owned()),
            Value::Text(id_list(resource_ids)),
            Value::Text(id_list(exclude_installation_ids)),
        ],
    )
    .query_map(conn, |row| row.get(0))?;
    let mut out: Vec<i64> = Vec::with_capacity(rows.len());
    for id in rows {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

/// `findLiveKeys`: the keys, among `keys`, with a live (not detached, not reused)
/// provenance row, in input order.
pub fn find_live_keys(
    conn: &Connection,
    workspace_id: i64,
    keys: &[ResourceKey],
) -> Result<Vec<ResourceKey>> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let pairs = serde_json::Value::from(
        keys.iter()
            .map(|k| serde_json::json!([k.kind, k.resource_id]))
            .collect::<Vec<_>>(),
    )
    .to_string();
    let live: Vec<(String, i64)> = Sql::new(
        "SELECT \"resourceKind\", \"resourceId\" FROM \"TemplateInstalledResource\" WHERE \"workspaceId\" = ? AND \"wasExisting\" = 0 AND \"detachedAt\" IS NULL AND EXISTS (SELECT 1 FROM json_each(?) k WHERE json_extract(k.value, '$[0]') = \"resourceKind\" AND json_extract(k.value, '$[1]') = \"resourceId\")",
        vec![Value::Integer(workspace_id), Value::Text(pairs)],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(keys
        .iter()
        .filter(|k| {
            live.iter()
                .any(|(kind, id)| *kind == k.kind && *id == k.resource_id)
        })
        .cloned()
        .collect())
}

/// `findPriorCopies`: copies an earlier install created from these source ids, newest first.
pub fn find_prior_copies(
    conn: &Connection,
    workspace_id: i64,
    resource_kind: &str,
    source_resource_ids: &[String],
) -> Result<Vec<PriorCopy>> {
    if source_resource_ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"sourceResourceId\", \"resourceId\" FROM \"TemplateInstalledResource\" WHERE \"workspaceId\" = ? AND \"resourceKind\" = ? AND \"wasExisting\" = 0 AND \"sourceResourceId\" IN (SELECT value FROM json_each(?)) ORDER BY \"id\" DESC",
        vec![
            Value::Integer(workspace_id),
            Value::Text(resource_kind.to_owned()),
            Value::Text(text_list(source_resource_ids)),
        ],
    )
    .query_map(conn, |row| {
        Ok(PriorCopy {
            source_resource_id: row.get(0)?,
            resource_id: row.get(1)?,
        })
    })
}

/// `listCategories`: an installation's rows of a known kind, by id.
pub fn list_categories(
    conn: &Connection,
    installation_id: i64,
    workspace_id: i64,
) -> Result<Vec<TemplateInstalledResource>> {
    let rows = Sql::new(
        format!(
            "{} WHERE \"installationId\" = ? AND \"workspaceId\" = ? ORDER BY \"id\" ASC",
            TemplateInstalledResource::select_sql()
        ),
        vec![
            Value::Integer(installation_id),
            Value::Integer(workspace_id),
        ],
    )
    .models::<TemplateInstalledResource>(conn)?;
    Ok(keep_known_resource_kinds(rows))
}

/// `listPage`: one keyset page of an installation's tracked resources, by id. The cursor
/// comes from the raw page, so an unknown kind is dropped without ending pagination.
pub fn list_page(conn: &Connection, input: &ListPageInput) -> Result<InstalledResourcePage> {
    let rows = Sql::new(
        format!(
            "{} WHERE \"installationId\" = ? AND \"workspaceId\" = ? AND (? IS NULL OR \"category\" = ?) AND (? IS NULL OR \"id\" > ?) ORDER BY \"id\" ASC LIMIT ?",
            TemplateInstalledResource::select_sql()
        ),
        vec![
            Value::Integer(input.installation_id),
            Value::Integer(input.workspace_id),
            super::sql::opt_text(input.category.as_deref()),
            super::sql::opt_text(input.category.as_deref()),
            super::sql::opt_int(input.after_id),
            super::sql::opt_int(input.after_id),
            Value::Integer(input.limit + 1),
        ],
    )
    .models::<TemplateInstalledResource>(conn)?;
    let keep = usize::try_from(input.limit).unwrap_or(0);
    let more = rows.len() > keep;
    let mut page = rows;
    page.truncate(keep);
    let next_cursor = if more {
        page.last().map(|r| r.id)
    } else {
        None
    };
    Ok(InstalledResourcePage {
        rows: keep_known_resource_kinds(page),
        next_cursor,
    })
}
