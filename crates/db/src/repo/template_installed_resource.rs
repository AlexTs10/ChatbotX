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
    todo!("P3.3")
}

/// `listUnknownResourceKinds`: unknown `resourceKind` values with their row counts.
pub fn list_unknown_resource_kinds(conn: &Connection) -> Result<Vec<UnknownResourceKind>> {
    todo!("P3.3")
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
    todo!("P3.3")
}

/// `findLiveKeys`: the keys, among `keys`, with a live (not detached, not reused)
/// provenance row, in input order.
pub fn find_live_keys(
    conn: &Connection,
    workspace_id: i64,
    keys: &[ResourceKey],
) -> Result<Vec<ResourceKey>> {
    todo!("P3.3")
}

/// `findPriorCopies`: copies an earlier install created from these source ids, newest first.
pub fn find_prior_copies(
    conn: &Connection,
    workspace_id: i64,
    resource_kind: &str,
    source_resource_ids: &[String],
) -> Result<Vec<PriorCopy>> {
    todo!("P3.3")
}

/// `listCategories`: an installation's rows of a known kind, by id.
pub fn list_categories(
    conn: &Connection,
    installation_id: i64,
    workspace_id: i64,
) -> Result<Vec<TemplateInstalledResource>> {
    todo!("P3.3")
}

/// `listPage`: one keyset page of an installation's tracked resources, by id. The cursor
/// comes from the raw page, so an unknown kind is dropped without ending pagination.
pub fn list_page(conn: &Connection, input: &ListPageInput) -> Result<InstalledResourcePage> {
    todo!("P3.3")
}
