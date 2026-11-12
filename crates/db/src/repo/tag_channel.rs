//! Port of `repositories/tag-channel/repository.ts` (P3.3).

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
    let now = now_ms();
    let tag_id: i64 = conn.query_row(
        "INSERT INTO \"Tag\" (\"id\", \"createdAt\", \"updatedAt\", \"name\", \"workspaceId\") VALUES (?1, ?2, ?2, ?3, ?4) \
         ON CONFLICT (\"workspaceId\", \"name\") WHERE \"deletedAt\" IS NULL DO UPDATE SET \"name\" = excluded.\"name\" \
         RETURNING \"id\"",
        rusqlite::params![new_id(), now, input.label_name, input.workspace_id],
        |r| r.get(0),
    )?;
    let tag_channel_id: i64 = conn.query_row(
        &format!("{INSERT_TAG_CHANNEL}DO UPDATE SET \"externalLabelId\" = excluded.\"externalLabelId\", \"updatedAt\" = excluded.\"updatedAt\" RETURNING \"id\""),
        rusqlite::params![
            new_id(),
            now,
            input.workspace_id,
            tag_id,
            input.channel_type,
            input.integration_id,
            input.external_label_id
        ],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO \"ContactToTag\" (\"contactId\", \"tagId\") VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        rusqlite::params![input.contact_id, tag_id],
    )?;
    link_contact_inbox(conn, tag_id, tag_channel_id, input.contact_inbox_id)
}

/// `insertIfAbsent`.
pub fn insert_if_absent(conn: &Connection, key: &TagChannelKey) -> Result<()> {
    conn.execute(
        &format!("{INSERT_TAG_CHANNEL}DO NOTHING"),
        rusqlite::params![
            new_id(),
            now_ms(),
            key.workspace_id,
            key.tag_id,
            key.channel_type,
            key.integration_id,
            key.external_label_id
        ],
    )?;
    Ok(())
}

/// `findByTagAndIntegration`.
pub fn find_by_tag_and_integration(
    conn: &Connection,
    workspace_id: i64,
    tag_id: i64,
    channel_type: &str,
    integration_id: i64,
) -> Result<Option<TagChannel>> {
    super::find_one(
        conn,
        "\"tagId\" = ?1 AND \"workspaceId\" = ?2 AND \"channelType\" = ?3 AND \"integrationId\" = ?4",
        &[&tag_id, &workspace_id, &channel_type, &integration_id],
    )
}

/// `updateExternalLabelId`.
pub fn update_external_label_id(conn: &Connection, id: i64, external_label_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE \"TagChannel\" SET \"externalLabelId\" = ?2, \"updatedAt\" = ?3 WHERE \"id\" = ?1",
        rusqlite::params![id, external_label_id, now_ms()],
    )?;
    Ok(())
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
    if let Some(row) = tag_channel_rows(conn, &format!("{INSERT_TAG_CHANNEL}DO NOTHING"), key)?
        .into_iter()
        .next()
    {
        return Ok(Some(row));
    }
    find_by_tag_and_integration(
        conn,
        key.workspace_id,
        key.tag_id,
        &key.channel_type,
        key.integration_id,
    )
}

/// `upsertByTagAndIntegration`: sets the external label id of an existing key.
pub fn upsert_by_tag_and_integration(
    conn: &Connection,
    key: &TagChannelKey,
) -> Result<Option<TagChannel>> {
    Ok(tag_channel_rows(
        conn,
        &format!("{INSERT_TAG_CHANNEL}DO UPDATE SET \"externalLabelId\" = excluded.\"externalLabelId\", \"updatedAt\" = excluded.\"updatedAt\""),
        key,
    )?
    .into_iter()
    .next())
}

/// `linkContactInbox` (a repeat is ignored).
pub fn link_contact_inbox(
    conn: &Connection,
    tag_id: i64,
    tag_channel_id: i64,
    contact_inbox_id: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO \"ContactToTagChannel\" (\"tagId\", \"tagChannelId\", \"contactInboxId\") VALUES (?1, ?2, ?3) \
         ON CONFLICT DO NOTHING",
        rusqlite::params![tag_id, tag_channel_id, contact_inbox_id],
    )?;
    Ok(())
}

/// `unlinkContactInbox`.
pub fn unlink_contact_inbox(
    conn: &Connection,
    tag_channel_id: i64,
    contact_inbox_id: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM \"ContactToTagChannel\" WHERE \"tagChannelId\" = ?1 AND \"contactInboxId\" = ?2",
        rusqlite::params![tag_channel_id, contact_inbox_id],
    )?;
    Ok(())
}

/// `listContactTagChannelRows`: the contact's channel links of one tag.
pub fn list_contact_tag_channel_rows(
    conn: &Connection,
    tag_id: i64,
    contact_id: i64,
) -> Result<Vec<ContactTagChannelRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT l.\"tagChannelId\", l.\"contactInboxId\", t.\"channelType\", t.\"integrationId\", t.\"externalLabelId\", \
         ci.\"sourceId\" FROM \"ContactToTagChannel\" l \
         INNER JOIN \"TagChannel\" t ON l.\"tagChannelId\" = t.\"id\" \
         INNER JOIN \"ContactInbox\" ci ON l.\"contactInboxId\" = ci.\"id\" \
         WHERE l.\"tagId\" = ?1 AND ci.\"contactId\" = ?2",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![tag_id, contact_id], |r| {
            Ok(ContactTagChannelRow {
                tag_channel_id: r.get(0)?,
                contact_inbox_id: r.get(1)?,
                channel_type: r.get(2)?,
                integration_id: r.get(3)?,
                external_label_id: r.get(4)?,
                source_id: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listByTag` (optional channel type / integration filters).
pub fn list_by_tag(
    conn: &Connection,
    workspace_id: i64,
    tag_id: i64,
    channel_type: Option<&str>,
    integration_id: Option<i64>,
) -> Result<Vec<TagChannelSummary>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"channelType\", \"integrationId\", \"externalLabelId\" FROM \"TagChannel\" \
         WHERE \"tagId\" = ?1 AND \"workspaceId\" = ?2 AND (?3 IS NULL OR \"channelType\" = ?3) \
         AND (?4 IS NULL OR \"integrationId\" = ?4)",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                tag_id,
                workspace_id,
                channel_type.filter(|c| !c.is_empty()),
                integration_id
            ],
            |r| {
                Ok(TagChannelSummary {
                    id: r.get(0)?,
                    channel_type: r.get(1)?,
                    integration_id: r.get(2)?,
                    external_label_id: r.get(3)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `deleteById`.
pub fn delete_by_id(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM \"TagChannel\" WHERE \"id\" = ?1", [id])?;
    Ok(())
}

/// `listContactInboxIdsForChannelPage`: ascending, after the cursor.
pub fn list_contact_inbox_ids_for_channel_page(
    conn: &Connection,
    tag_channel_id: i64,
    after_contact_inbox_id: Option<i64>,
    limit: i64,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"contactInboxId\" FROM \"ContactToTagChannel\" WHERE \"tagChannelId\" = ?1 \
         AND (?2 IS NULL OR \"contactInboxId\" > ?2) ORDER BY \"contactInboxId\" ASC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![tag_channel_id, after_contact_inbox_id, limit],
            |r| r.get(0),
        )?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

/// `deleteLinksForChannel` (no statement for an empty list).
pub fn delete_links_for_channel(
    conn: &Connection,
    tag_channel_id: i64,
    contact_inbox_ids: &[i64],
) -> Result<()> {
    if contact_inbox_ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM \"ContactToTagChannel\" WHERE \"tagChannelId\" = ?1 AND \"contactInboxId\" IN (SELECT value FROM json_each(?2))",
        rusqlite::params![tag_channel_id, id_list(contact_inbox_ids)],
    )?;
    Ok(())
}

/// `deleteContactTagsForContacts` (no statement for an empty list).
pub fn delete_contact_tags_for_contacts(
    conn: &Connection,
    tag_id: i64,
    contact_ids: &[i64],
) -> Result<()> {
    if contact_ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM \"ContactToTag\" WHERE \"tagId\" = ?1 AND \"contactId\" IN (SELECT value FROM json_each(?2))",
        rusqlite::params![tag_id, id_list(contact_ids)],
    )?;
    Ok(())
}

/// `listTaggedContactIdsPage`: ascending, after the cursor.
pub fn list_tagged_contact_ids_page(
    conn: &Connection,
    tag_id: i64,
    after_contact_id: Option<i64>,
    limit: i64,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"contactId\" FROM \"ContactToTag\" WHERE \"tagId\" = ?1 AND (?2 IS NULL OR \"contactId\" > ?2) \
         ORDER BY \"contactId\" ASC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![tag_id, after_contact_id, limit], |r| {
            r.get(0)
        })?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}
