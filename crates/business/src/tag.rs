//! `packages/business/src/tag/service.ts`: the tags of a workspace and the
//! contact links (`"ContactToTag"`). Private queries on `"Tag"`,
//! `"ContactToTag"`, `"Contact"`, `"Folder"`. No tag-sync jobs, no
//! `tagApplied` / `tagRemoved` events, no ads-conversion enqueue, no read
//! cache, no template edit locks / delete guards (P3.6-D1 rule).

use std::collections::HashSet;

use chatbotx_db::Db;
use chatbotx_db::repo::models::Tag;
use chatbotx_db::repo::utils::{order_by_sql, parse_order_by, with_id_tie_break};
use chatbotx_db::repo::{
    Model, PaginationInput, SortItem, get_pagination_with_defaults, is_bigint_id, like_contains,
    new_id, now_ms,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};

use crate::errors::{BusinessError, Result};
use crate::tx;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `toStoredFolderId`: `0` (the root) is `NULL`.
fn stored_folder_id(folder_id: Option<i64>) -> Option<i64> {
    folder_id.filter(|id| *id != 0)
}

/// `folderService.ensureExists` (a `tag` folder of the workspace).
fn ensure_folder(conn: &Connection, workspace_id: i64, folder_id: i64) -> Result<()> {
    conn.query_row(
        r#"SELECT 1 FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "folderType" = 'tag'"#,
        params![folder_id, workspace_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Folder not found"))
}

fn tags_where(conn: &Connection, filter: &str, binds: &[Sql]) -> Result<Vec<Tag>> {
    let mut stmt = conn
        .prepare(&format!("{} WHERE {filter}", Tag::select_sql()))
        .map_err(repo_err)?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), Tag::from_row)
        .map_err(repo_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(repo_err)?;
    Ok(rows)
}

fn live_tag(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Tag>> {
    Ok(tags_where(
        conn,
        r#""id" = ? AND "workspaceId" = ? AND "deletedAt" IS NULL"#,
        &[Sql::Integer(id), Sql::Integer(workspace_id)],
    )?
    .pop())
}

fn find_by_key_in(conn: &Connection, workspace_id: i64, key: &str) -> Result<Option<Tag>> {
    if is_bigint_id(key)
        && let Some(tag) = live_tag(conn, workspace_id, key.parse().unwrap_or_default())?
    {
        return Ok(Some(tag));
    }
    Ok(tags_where(
        conn,
        r#""name" = ? AND "workspaceId" = ? AND "deletedAt" IS NULL ORDER BY "id" LIMIT 1"#,
        &[Sql::Text(key.to_string()), Sql::Integer(workspace_id)],
    )?
    .pop())
}

/// `findOrFail(contactModel, { id, workspaceId })`.
fn contact_in_workspace(conn: &Connection, workspace_id: i64, contact_id: i64) -> Result<()> {
    conn.query_row(
        r#"SELECT 1 FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![contact_id, workspace_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Contact not found"))
}

/// `contactService.findManyByIds`: the ids of `ids` that are contacts of the
/// workspace, in input order.
fn contacts_of(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    let mut stmt = conn
        .prepare(r#"SELECT 1 FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#)
        .map_err(repo_err)?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if seen.insert(*id) && stmt.exists(params![id, workspace_id]).map_err(repo_err)? {
            out.push(*id);
        }
    }
    Ok(out)
}

/// Inserts a link unless it exists; `true` when new.
fn link(conn: &Connection, contact_id: i64, tag_id: i64) -> Result<bool> {
    let changed = conn
        .execute(
            r#"INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES (?1, ?2)
               ON CONFLICT ("contactId", "tagId") DO NOTHING"#,
            params![contact_id, tag_id],
        )
        .map_err(repo_err)?;
    Ok(changed > 0)
}

/// Deletes a link; `true` when it existed.
fn unlink(conn: &Connection, contact_id: i64, tag_id: i64) -> Result<bool> {
    let changed = conn
        .execute(
            r#"DELETE FROM "ContactToTag" WHERE "contactId" = ?1 AND "tagId" = ?2"#,
            params![contact_id, tag_id],
        )
        .map_err(repo_err)?;
    Ok(changed > 0)
}

/// Trimmed, unique (first-seen order), non-empty names.
fn clean_names(names: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    names
        .iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty() && seen.insert(name.clone()))
        .collect()
}

/// The live workspace tags named `names` (exact match).
fn tags_named(conn: &Connection, workspace_id: i64, names: &[String]) -> Result<Vec<Tag>> {
    let mut out = Vec::new();
    for name in names {
        out.extend(tags_where(
            conn,
            r#""name" = ? AND "workspaceId" = ? AND "deletedAt" IS NULL"#,
            &[Sql::Text(name.clone()), Sql::Integer(workspace_id)],
        )?);
    }
    Ok(out)
}

fn upsert_in(conn: &Connection, workspace_id: i64, names: &[String]) -> Result<Vec<Tag>> {
    let names = clean_names(names);
    let now = now_ms();
    for name in &names {
        conn.execute(
            r#"INSERT INTO "Tag" ("id", "createdAt", "updatedAt", "name", "workspaceId")
               VALUES (?1, ?2, ?2, ?3, ?4)
               ON CONFLICT ("workspaceId", "name") WHERE "deletedAt" IS NULL DO NOTHING"#,
            params![new_id(), now, name, workspace_id],
        )
        .map_err(repo_err)?;
    }
    tags_named(conn, workspace_id, &names)
}

/// `CONTACT_CHUNK_SIZE`.
pub const CONTACT_CHUNK_SIZE: usize = 200;

/// `list` input (`folderId: Some(0)` = the root).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListTags {
    pub name: Option<String>,
    pub folder_id: Option<i64>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// One listed tag and its `contactsCount`.
#[derive(Debug, Clone, PartialEq)]
pub struct ListedTag {
    pub tag: Tag,
    pub contacts_count: i64,
}

/// `list` result.
#[derive(Debug, Clone, PartialEq)]
pub struct TagPage {
    pub data: Vec<ListedTag>,
    pub page_count: i64,
}

/// `attachByNamesToContacts` / `detachByNamesFromContacts` result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactsOutcome {
    pub processed_contact_ids: Vec<i64>,
    pub skipped_contact_ids: Vec<i64>,
}

/// `list`: live tags, `sort` columns (then `id`), `name` an ILIKE substring.
pub async fn list(db: &Db, workspace_id: i64, input: ListTags) -> Result<TagPage> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        let order = with_id_tie_break(&parse_order_by(Tag::COLUMNS, input.sort.as_deref(), None));
        let mut filter = String::from(r#""workspaceId" = ? AND "deletedAt" IS NULL"#);
        let mut binds: Vec<Sql> = vec![Sql::Integer(workspace_id)];
        if let Some(name) = input.name.as_deref().filter(|n| !n.is_empty()) {
            filter.push_str(r#" AND ilike("name", ?)"#);
            binds.push(Sql::Text(like_contains(name)));
        }
        match input.folder_id {
            Some(0) => filter.push_str(r#" AND "folderId" IS NULL"#),
            Some(folder) => {
                filter.push_str(r#" AND "folderId" = ?"#);
                binds.push(Sql::Integer(folder));
            }
            None => {}
        }
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT count(*) FROM "Tag" WHERE {filter}"#),
                params_from_iter(binds.iter()),
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let sql = format!(
            r#"SELECT {}, (SELECT count(*) FROM "ContactToTag" c WHERE c."tagId" = "Tag"."id")
               FROM "Tag" WHERE {filter} {} LIMIT ? OFFSET ?"#,
            Tag::columns_sql(None),
            order_by_sql(&order, None)
        );
        binds.push(Sql::Integer(pagination.limit));
        binds.push(Sql::Integer(pagination.offset));
        let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
        let data = stmt
            .query_map(params_from_iter(binds.iter()), |row| {
                Ok(ListedTag {
                    tag: Tag::from_row(row)?,
                    contacts_count: row.get(Tag::COLUMNS.len())?,
                })
            })
            .map_err(repo_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        let page_count = if pagination.limit > 0 {
            (total + pagination.limit - 1) / pagination.limit
        } else {
            1
        };
        Ok(TagPage { data, page_count })
    })
    .await
}

/// `create`: a live tag with the same name is
/// `validation("name", "Name is already taken.")`; `folder_id: Some(0)` = root.
pub async fn create(db: &Db, workspace_id: i64, name: &str, folder_id: Option<i64>) -> Result<Tag> {
    let name = name.to_string();
    tx::write(db, move |conn| {
        if !tags_named(conn, workspace_id, std::slice::from_ref(&name))?.is_empty() {
            return Err(BusinessError::validation("name", "Name is already taken."));
        }
        let folder_id = stored_folder_id(folder_id);
        if let Some(folder) = folder_id {
            ensure_folder(conn, workspace_id, folder)?;
        }
        let id = new_id();
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "Tag" ("id", "createdAt", "updatedAt", "name", "folderId", "workspaceId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5)"#,
            params![id, now, name, folder_id, workspace_id],
        )
        .map_err(repo_err)?;
        live_tag(conn, workspace_id, id)?.ok_or_else(|| BusinessError::not_found("Tag not found"))
    })
    .await
}

/// `update`: rename; `folder_id: None` keeps the folder, `Some(None)` /
/// `Some(Some(0))` = the root.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: &str,
    folder_id: Option<Option<i64>>,
) -> Result<Tag> {
    let name = name.to_string();
    tx::write(db, move |conn| {
        let taken = tags_where(
            conn,
            r#""name" = ? AND "workspaceId" = ? AND "deletedAt" IS NULL AND "id" <> ?"#,
            &[Sql::Text(name.clone()), Sql::Integer(workspace_id), Sql::Integer(id)],
        )?;
        if !taken.is_empty() {
            return Err(BusinessError::validation("name", "Name is already taken."));
        }
        let folder_id = folder_id.map(stored_folder_id);
        if let Some(Some(folder)) = folder_id {
            ensure_folder(conn, workspace_id, folder)?;
        }
        let tag = live_tag(conn, workspace_id, id)?.ok_or_else(|| BusinessError::not_found("Tag not found"))?;
        match folder_id {
            Some(folder) => conn.execute(
                r#"UPDATE "Tag" SET "name" = ?1, "folderId" = ?2, "updatedAt" = ?3 WHERE "id" = ?4"#,
                params![name, folder, now_ms(), tag.id],
            ),
            None => conn.execute(
                r#"UPDATE "Tag" SET "name" = ?1, "updatedAt" = ?2 WHERE "id" = ?3"#,
                params![name, now_ms(), tag.id],
            ),
        }
        .map_err(repo_err)?;
        live_tag(conn, workspace_id, tag.id)?.ok_or_else(|| BusinessError::not_found("Tag not found"))
    })
    .await
}

/// `softDelete`: sets `deletedAt` on the live tags of `ids`; returns the ids
/// deleted.
pub async fn soft_delete(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    let ids = ids.to_vec();
    tx::write(db, move |conn| {
        let now = now_ms();
        let mut deleted = Vec::new();
        for chunk in ids.chunks(CONTACT_CHUNK_SIZE) {
            for id in chunk {
                let changed = conn
                    .execute(
                        r#"UPDATE "Tag" SET "deletedAt" = ?1, "updatedAt" = ?1
                           WHERE "workspaceId" = ?2 AND "id" = ?3 AND "deletedAt" IS NULL"#,
                        params![now, workspace_id, id],
                    )
                    .map_err(repo_err)?;
                if changed > 0 {
                    deleted.push(*id);
                }
            }
        }
        Ok(deleted)
    })
    .await
}

/// `findByKey`: a numeric key is tried as a live tag id first, then as a name.
pub async fn find_by_key(db: &Db, workspace_id: i64, key: &str) -> Result<Option<Tag>> {
    let key = key.to_string();
    tx::read(db, move |conn| find_by_key_in(conn, workspace_id, &key)).await
}

/// `findByKeyOrFail`: 404 "Tag not found".
pub async fn find_by_key_or_fail(db: &Db, workspace_id: i64, key: &str) -> Result<Tag> {
    find_by_key(db, workspace_id, key)
        .await?
        .ok_or_else(|| BusinessError::not_found("Tag not found"))
}

/// `listForContact`: the contact's live tags by name.
pub async fn list_for_contact(db: &Db, workspace_id: i64, contact_id: i64) -> Result<Vec<Tag>> {
    tx::read(db, move |conn| {
        tags_where(
            conn,
            r#""workspaceId" = ? AND "deletedAt" IS NULL
               AND "id" IN (SELECT "tagId" FROM "ContactToTag" WHERE "contactId" = ?)
               ORDER BY "name" ASC"#,
            &[Sql::Integer(workspace_id), Sql::Integer(contact_id)],
        )
    })
    .await
}

/// `upsertByNames`: trimmed, unique, non-empty names; missing ones are
/// created. Returns `(id, name)` of every live match.
pub async fn upsert_by_names(
    db: &Db,
    workspace_id: i64,
    names: &[String],
) -> Result<Vec<(i64, String)>> {
    let names = names.to_vec();
    tx::write(db, move |conn| {
        Ok(upsert_in(conn, workspace_id, &names)?
            .into_iter()
            .map(|tag| (tag.id, tag.name))
            .collect())
    })
    .await
}

/// `attachToContact`: links the live workspace tags of `tag_ids` (others
/// ignored); 404 "Contact not found" for a contact of another workspace.
/// Returns the newly linked tag ids.
pub async fn attach_to_contact(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    tag_ids: &[i64],
) -> Result<Vec<i64>> {
    let tag_ids = tag_ids.to_vec();
    tx::write(db, move |conn| {
        if tag_ids.is_empty() {
            return Ok(Vec::new());
        }
        contact_in_workspace(conn, workspace_id, contact_id)?;
        let mut linked = Vec::new();
        let mut seen = HashSet::new();
        for tag_id in tag_ids {
            if seen.insert(tag_id)
                && live_tag(conn, workspace_id, tag_id)?.is_some()
                && link(conn, contact_id, tag_id)?
            {
                linked.push(tag_id);
            }
        }
        Ok(linked)
    })
    .await
}

/// `detachFromContact`: unlinks `tag_ids`; returns the removed tag ids.
pub async fn detach_from_contact(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    tag_ids: &[i64],
) -> Result<Vec<i64>> {
    let tag_ids = tag_ids.to_vec();
    tx::write(db, move |conn| {
        contact_in_workspace(conn, workspace_id, contact_id)?;
        let mut removed = Vec::new();
        for tag_id in tag_ids {
            if unlink(conn, contact_id, tag_id)? {
                removed.push(tag_id);
            }
        }
        Ok(removed)
    })
    .await
}

/// `attachByNamesToContacts` (contacts in chunks of 200).
pub async fn attach_by_names_to_contacts(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    names: &[String],
) -> Result<ContactsOutcome> {
    let contact_ids = contact_ids.to_vec();
    let names = names.to_vec();
    tx::write(db, move |conn| {
        if contact_ids.is_empty() || names.is_empty() {
            return Ok(ContactsOutcome {
                processed_contact_ids: Vec::new(),
                skipped_contact_ids: contact_ids,
            });
        }
        let tags = upsert_in(conn, workspace_id, &names)?;
        if tags.is_empty() {
            return Ok(ContactsOutcome {
                processed_contact_ids: Vec::new(),
                skipped_contact_ids: contact_ids,
            });
        }
        let mut processed = Vec::new();
        for chunk in contact_ids.chunks(CONTACT_CHUNK_SIZE) {
            let contacts = contacts_of(conn, workspace_id, chunk)?;
            for contact_id in &contacts {
                for tag in &tags {
                    link(conn, *contact_id, tag.id)?;
                }
            }
            processed.extend(contacts);
        }
        Ok(outcome(contact_ids, processed))
    })
    .await
}

/// `detachByNamesFromContacts`: never creates a tag.
pub async fn detach_by_names_from_contacts(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    names: &[String],
) -> Result<ContactsOutcome> {
    let contact_ids = contact_ids.to_vec();
    let names = names.to_vec();
    tx::write(db, move |conn| {
        if contact_ids.is_empty() || names.is_empty() {
            return Ok(ContactsOutcome {
                processed_contact_ids: Vec::new(),
                skipped_contact_ids: contact_ids,
            });
        }
        let tags = tags_named(conn, workspace_id, &clean_names(&names))?;
        let mut processed = Vec::new();
        for chunk in contact_ids.chunks(CONTACT_CHUNK_SIZE) {
            let contacts = contacts_of(conn, workspace_id, chunk)?;
            for contact_id in &contacts {
                for tag in &tags {
                    unlink(conn, *contact_id, tag.id)?;
                }
            }
            processed.extend(contacts);
        }
        Ok(outcome(contact_ids, processed))
    })
    .await
}

/// `replaceContactTagsByNames`: the contact's tag set becomes `names`
/// (created when missing); returns the resolved tags.
pub async fn replace_contact_tags_by_names(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    names: &[String],
) -> Result<Vec<Tag>> {
    let names = names.to_vec();
    tx::write(db, move |conn| {
        contact_in_workspace(conn, workspace_id, contact_id)?;
        let tags = upsert_in(conn, workspace_id, &names)?;
        let keep: HashSet<i64> = tags.iter().map(|tag| tag.id).collect();
        for tag in &tags {
            link(conn, contact_id, tag.id)?;
        }
        let mut stmt = conn
            .prepare(r#"SELECT "tagId" FROM "ContactToTag" WHERE "contactId" = ?1"#)
            .map_err(repo_err)?;
        let old: Vec<i64> = stmt
            .query_map(params![contact_id], |r| r.get(0))
            .map_err(repo_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        for tag_id in old.into_iter().filter(|id| !keep.contains(id)) {
            unlink(conn, contact_id, tag_id)?;
        }
        Ok(tags)
    })
    .await
}

fn outcome(contact_ids: Vec<i64>, processed: Vec<i64>) -> ContactsOutcome {
    let done: HashSet<i64> = processed.iter().copied().collect();
    ContactsOutcome {
        skipped_contact_ids: contact_ids
            .into_iter()
            .filter(|id| !done.contains(id))
            .collect(),
        processed_contact_ids: processed,
    }
}
