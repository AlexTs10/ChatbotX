//! Port of `repositories/tag-channel/repository.ts` (P3.3).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

use rusqlite::Connection;

use super::models::TagChannel;
use super::{Result, id_list, new_id, now_ms};

/// A channel label: `(tagId, channelType, integrationId)` plus the external label id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagChannelKey {
    pub workspace_id: i64,
    pub tag_id: i64,
    pub channel_type: String,
    pub integration_id: i64,
    pub external_label_id: String,
}

/// Input of [`upsert_label_mapping`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelMappingInput {
    pub workspace_id: i64,
    pub channel_type: String,
    pub integration_id: i64,
    pub external_label_id: String,
    pub label_name: String,
    pub contact_inbox_id: i64,
    pub contact_id: i64,
}

/// `ContactTagChannelRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactTagChannelRow {
    pub tag_channel_id: i64,
    pub contact_inbox_id: i64,
    pub channel_type: String,
    pub integration_id: i64,
    pub external_label_id: String,
    pub source_id: String,
}

/// The columns `listByTag` reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagChannelSummary {
    pub id: i64,
    pub channel_type: String,
    pub integration_id: i64,
    pub external_label_id: String,
}

const INSERT_TAG_CHANNEL: &str = "INSERT INTO \"TagChannel\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"tagId\", \
     \"channelType\", \"integrationId\", \"externalLabelId\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7) \
     ON CONFLICT (\"tagId\", \"channelType\", \"integrationId\") ";

/// `upsertLabelMapping`: finds-or-creates the live tag by name, upserts its channel
/// label, and links the contact (tag) and the contact inbox (tag channel). One write job.
pub fn upsert_label_mapping(conn: &Connection, input: &LabelMappingInput) -> Result<()> {
    todo!("P3.3")
}

/// `insertIfAbsent`.
pub fn insert_if_absent(conn: &Connection, key: &TagChannelKey) -> Result<()> {
    todo!("P3.3")
}

/// `findByTagAndIntegration`.
pub fn find_by_tag_and_integration(
    conn: &Connection,
    workspace_id: i64,
    tag_id: i64,
    channel_type: &str,
    integration_id: i64,
) -> Result<Option<TagChannel>> {
    todo!("P3.3")
}

/// `updateExternalLabelId`.
pub fn update_external_label_id(conn: &Connection, id: i64, external_label_id: &str) -> Result<()> {
    todo!("P3.3")
}

fn tag_channel_rows(conn: &Connection, sql: &str, key: &TagChannelKey) -> Result<Vec<TagChannel>> {
    super::returning::<TagChannel>(
        conn,
        sql,
        &[
            &new_id(),
            &now_ms(),
            &key.workspace_id,
            &key.tag_id,
            &key.channel_type,
            &key.integration_id,
            &key.external_label_id,
        ],
    )
}

/// `insertOrFetch`: the inserted row, or the existing row of the same key.
pub fn insert_or_fetch(conn: &Connection, key: &TagChannelKey) -> Result<Option<TagChannel>> {
    todo!("P3.3")
}

/// `upsertByTagAndIntegration`: sets the external label id of an existing key.
pub fn upsert_by_tag_and_integration(
    conn: &Connection,
    key: &TagChannelKey,
) -> Result<Option<TagChannel>> {
    todo!("P3.3")
}

/// `linkContactInbox` (a repeat is ignored).
pub fn link_contact_inbox(
    conn: &Connection,
    tag_id: i64,
    tag_channel_id: i64,
    contact_inbox_id: i64,
) -> Result<()> {
    todo!("P3.3")
}

/// `unlinkContactInbox`.
pub fn unlink_contact_inbox(
    conn: &Connection,
    tag_channel_id: i64,
    contact_inbox_id: i64,
) -> Result<()> {
    todo!("P3.3")
}

/// `listContactTagChannelRows`: the contact's channel links of one tag.
pub fn list_contact_tag_channel_rows(
    conn: &Connection,
    tag_id: i64,
    contact_id: i64,
) -> Result<Vec<ContactTagChannelRow>> {
    todo!("P3.3")
}

/// `listByTag` (optional channel type / integration filters).
pub fn list_by_tag(
    conn: &Connection,
    workspace_id: i64,
    tag_id: i64,
    channel_type: Option<&str>,
    integration_id: Option<i64>,
) -> Result<Vec<TagChannelSummary>> {
    todo!("P3.3")
}

/// `deleteById`.
pub fn delete_by_id(conn: &Connection, id: i64) -> Result<()> {
    todo!("P3.3")
}

/// `listContactInboxIdsForChannelPage`: ascending, after the cursor.
pub fn list_contact_inbox_ids_for_channel_page(
    conn: &Connection,
    tag_channel_id: i64,
    after_contact_inbox_id: Option<i64>,
    limit: i64,
) -> Result<Vec<i64>> {
    todo!("P3.3")
}

/// `deleteLinksForChannel` (no statement for an empty list).
pub fn delete_links_for_channel(
    conn: &Connection,
    tag_channel_id: i64,
    contact_inbox_ids: &[i64],
) -> Result<()> {
    todo!("P3.3")
}

/// `deleteContactTagsForContacts` (no statement for an empty list).
pub fn delete_contact_tags_for_contacts(
    conn: &Connection,
    tag_id: i64,
    contact_ids: &[i64],
) -> Result<()> {
    todo!("P3.3")
}

/// `listTaggedContactIdsPage`: ascending, after the cursor.
pub fn list_tagged_contact_ids_page(
    conn: &Connection,
    tag_id: i64,
    after_contact_id: Option<i64>,
    limit: i64,
) -> Result<Vec<i64>> {
    todo!("P3.3")
}
