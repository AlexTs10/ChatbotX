//! `contact/service.ts`: `resolveIdByIdentifier`, `findPublicContactOrFail`,
//! `block` / `unblock` (`*AndRecord`), `delete` / `deleteAndRecord`,
//! `upsertByIdentifier`.

use std::time::{SystemTime, UNIX_EPOCH};

use chatbotx_db::Db;
use chatbotx_db::repo::contact::{PublicContact, find_id_by_identity_where, find_public_by_id};
use chatbotx_db::repo::models::Contact;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};

use super::create::{
    NewContact, NewContactInbox, insert_with_inbox, load, new_id, new_source_id, repo_err,
};
use super::identifier::{IdentifierPrefix, parse_contact_identifier};
use crate::{BusinessError, Result, tx};

/// `CONTACT_DELETE_CHUNK_SIZE`.
const CONTACT_DELETE_CHUNK_SIZE: usize = 50;

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// `findByIdOrFail` (workspace-scoped).
pub(crate) fn find_or_fail(conn: &Connection, workspace_id: i64, id: i64) -> Result<Contact> {
    use chatbotx_db::repo::Model as _;
    let sql = format!(
        "{} WHERE \"workspaceId\" = ?1 AND \"id\" = ?2",
        Contact::select_sql()
    );
    conn.query_row(&sql, params![workspace_id, id], |row| {
        Contact::from_row(row)
    })
    .optional()
    .map_err(repo_err)?
    .ok_or_else(|| BusinessError::not_found("Contact not found"))
}

/// `contactService.update` of text columns (`None` = `NULL`): the 404
/// check, the write (`updatedAt` = now) and the stored row.
pub(crate) fn update_text_columns(
    tx: &Connection,
    workspace_id: i64,
    id: i64,
    columns: &[(&str, Option<String>)],
) -> Result<Contact> {
    find_or_fail(tx, workspace_id, id)?;
    if !columns.is_empty() {
        let sets: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(i, (column, _))| format!("\"{column}\" = ?{}", i + 3))
            .collect();
        let sql = format!(
            "UPDATE \"Contact\" SET {}, \"updatedAt\" = ?2 WHERE \"id\" = ?1",
            sets.join(", ")
        );
        let mut values: Vec<chatbotx_db::rusqlite::types::Value> = vec![id.into(), now_ms().into()];
        values.extend(columns.iter().map(|(_, value)| match value {
            Some(text) => text.clone().into(),
            None => chatbotx_db::rusqlite::types::Value::Null,
        }));
        tx.execute(&sql, params_from_iter(values))
            .map_err(repo_err)?;
    }
    load::<Contact>(tx, id)
}

fn set_blocked_at(tx: &Connection, workspace_id: i64, id: i64, at: Option<i64>) -> Result<Contact> {
    find_or_fail(tx, workspace_id, id)?;
    tx.execute(
        r#"UPDATE "Contact" SET "blockedAt" = ?2, "updatedAt" = ?3 WHERE "id" = ?1"#,
        params![id, at, now_ms()],
    )
    .map_err(repo_err)?;
    load::<Contact>(tx, id)
}

/// `resolveIdByIdentifier`.
pub async fn resolve_id_by_identifier(db: &Db, workspace_id: i64, identifier: &str) -> Result<i64> {
    let parsed = parse_contact_identifier(identifier)?;
    tx::read(db, move |conn| {
        find_id_by_identity_where(
            conn,
            workspace_id,
            parsed.id,
            parsed.email(),
            parsed.phone(),
        )?
        .ok_or_else(|| BusinessError::not_found("Contact not found"))
    })
    .await
}

/// `findPublicContactOrFail`.
pub async fn find_public_contact_or_fail(
    db: &Db,
    workspace_id: i64,
    id: i64,
) -> Result<PublicContact> {
    tx::read(db, move |conn| {
        find_public_by_id(conn, workspace_id, id)?
            .ok_or_else(|| BusinessError::not_found("Contact not found"))
    })
    .await
}

/// `blockAndRecord`: sets `blockedAt` to `now_ms`.
pub async fn block_and_record(db: &Db, workspace_id: i64, id: i64, now_ms: i64) -> Result<Contact> {
    tx::write(db, move |tx| {
        set_blocked_at(tx, workspace_id, id, Some(now_ms))
    })
    .await
}

/// `unblockAndRecord`: clears `blockedAt`.
pub async fn unblock_and_record(db: &Db, workspace_id: i64, id: i64) -> Result<Contact> {
    tx::write(db, move |tx| set_blocked_at(tx, workspace_id, id, None)).await
}

/// `delete`: the workspace's contacts of `ids` with their sub-rows; one
/// `MessageCleanup` tombstone per contact inbox. Returns the deleted ids.
pub async fn delete(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    let ids = ids.to_vec();
    tx::write(db, move |tx| delete_in(tx, workspace_id, &ids)).await
}

/// `deleteAndRecord` result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteOutcome {
    pub processed_contact_ids: Vec<i64>,
    pub skipped_contact_ids: Vec<i64>,
}

/// `deleteAndRecord`.
pub async fn delete_and_record(db: &Db, workspace_id: i64, ids: &[i64]) -> Result<DeleteOutcome> {
    let deleted = delete(db, workspace_id, ids).await?;
    let skipped = ids
        .iter()
        .copied()
        .filter(|id| !deleted.contains(id))
        .collect();
    Ok(DeleteOutcome {
        processed_contact_ids: deleted,
        skipped_contact_ids: skipped,
    })
}

/// The fields of an upsert (`None` = not given).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpsertData {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub gender: Option<String>,
}

/// `upsertByIdentifier` result.
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertOutcome {
    pub contact: Contact,
    pub is_new: bool,
}

/// `upsertByIdentifier`: updates the matching contact, or creates an
/// `email:` / `phone:` one on the workspace webchat inbox.
pub async fn upsert_by_identifier(
    db: &Db,
    workspace_id: i64,
    identifier: &str,
    data: UpsertData,
    source: &str,
    avatar: Option<String>,
) -> Result<UpsertOutcome> {
    let parsed = parse_contact_identifier(identifier)?;
    let source = source.to_string();
    tx::write(db, move |tx| {
        let existing =
            find_id_by_identity_where(tx, workspace_id, parsed.id, parsed.email(), parsed.phone())?;
        let mut columns: Vec<(&str, Option<String>)> = Vec::new();
        for (column, value) in [
            ("firstName", &data.first_name),
            ("lastName", &data.last_name),
            ("email", &data.email),
            ("phoneNumber", &data.phone_number),
            ("gender", &data.gender),
        ] {
            if let Some(value) = value {
                columns.push((column, Some(value.clone())));
            }
        }
        if let Some(id) = existing {
            if let Some(avatar) = avatar.clone().filter(|a| !a.is_empty()) {
                columns.push(("avatar", Some(avatar)));
            }
            let contact = update_text_columns(tx, workspace_id, id, &columns)?;
            return Ok(UpsertOutcome {
                contact,
                is_new: false,
            });
        }
        if parsed.prefix == IdentifierPrefix::Id {
            return Err(BusinessError::not_found("Contact not found"));
        }
        let phone = data
            .phone_number
            .clone()
            .or_else(|| parsed.phone().map(str::to_string));
        if let Some(phone) = phone.filter(|p| !p.is_empty())
            && super::create::phone_taken(tx, workspace_id, &phone)?
        {
            return Err(BusinessError::exception(
                "Phone number already exists",
                "phoneExists",
                422,
            ));
        }
        let owner: Option<i64> = tx
            .query_row(
                r#"SELECT "ownerId" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?;
        if owner.is_none() {
            return Err(BusinessError::not_found("Workspace not found"));
        }
        let inbox_id: i64 = tx
            .query_row(
                r#"SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?1 AND "channel" = 'webchat' LIMIT 1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?
            .ok_or_else(|| BusinessError::not_found("Inbox not found"))?;
        // `{ ...identifierData, ...data }`.
        let mut new = NewContact {
            email: parsed.email().map(str::to_string),
            phone_number: parsed.phone().map(str::to_string),
            ..NewContact::default()
        };
        if data.email.is_some() {
            new.email.clone_from(&data.email);
        }
        if data.phone_number.is_some() {
            new.phone_number.clone_from(&data.phone_number);
        }
        new.first_name = data.first_name.clone();
        new.last_name = data.last_name.clone();
        new.gender = data.gender.clone();
        let source_id = new_source_id("webchat", None, None, None);
        let created = insert_with_inbox(
            tx,
            workspace_id,
            &new,
            &NewContactInbox {
                inbox_id,
                channel: "webchat",
                source: &source,
                source_id: &source_id,
                source_user_id: None,
            },
        )?;
        let contact = match avatar.filter(|a| !a.is_empty()) {
            Some(avatar) => update_text_columns(
                tx,
                workspace_id,
                created.contact.id,
                &[("avatar", Some(avatar))],
            )?,
            None => created.contact,
        };
        Ok(UpsertOutcome {
            contact,
            is_new: true,
        })
    })
    .await
}

/// `delete` in one transaction: the contacts of the workspace, chunked as
/// the original (`CONTACT_DELETE_CHUNK_SIZE`).
fn delete_in(tx: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    let id_json = serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_string());
    let mut stmt = tx
        .prepare(
            r#"SELECT "id" FROM "Contact" WHERE "workspaceId" = ?1
               AND "id" IN (SELECT value FROM json_each(?2)) ORDER BY "id""#,
        )
        .map_err(repo_err)?;
    let found: Vec<i64> = stmt
        .query_map(params![workspace_id, id_json], |r| r.get(0))
        .map_err(repo_err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(repo_err)?;
    drop(stmt);
    for chunk in found.chunks(CONTACT_DELETE_CHUNK_SIZE) {
        let chunk_json = serde_json::to_string(chunk).unwrap_or_else(|_| "[]".to_string());
        // `messageCleanupService.record`: one tombstone per contact inbox
        // with the contact's conversation ids and the inbox `sinceTime`.
        let mut stmt = tx
            .prepare(
                r#"SELECT ci."id", ci."contactId", ci."inboxId", ci."sourceId",
                          COALESCE(ci."firstInteractionAt", ci."createdAt"),
                          (SELECT json_group_array(CAST(c."id" AS TEXT)) FROM (SELECT "id" FROM "Conversation"
                             WHERE "contactId" = ci."contactId" ORDER BY "id") c)
                   FROM "ContactInbox" ci WHERE ci."contactId" IN (SELECT value FROM json_each(?1))
                   ORDER BY ci."inboxId", ci."sourceId""#,
            )
            .map_err(repo_err)?;
        let entries: Vec<(i64, i64, i64, String, i64, String)> = stmt
            .query_map([&chunk_json], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .map_err(repo_err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(repo_err)?;
        drop(stmt);
        tx.execute(
            r#"DELETE FROM "ContactInboxPost" WHERE "workspaceId" = ?1 AND "contactInboxId" IN
               (SELECT "id" FROM "ContactInbox" WHERE "contactId" IN (SELECT value FROM json_each(?2)))"#,
            params![workspace_id, chunk_json],
        )
        .map_err(repo_err)?;
        chatbotx_db::repo::contact_activity::delete_by_contact_ids(tx, workspace_id, chunk)?;
        for (contact_inbox_id, contact_id, inbox_id, source_id, since, conversations) in &entries {
            tx.execute(
                r#"INSERT INTO "MessageCleanup" ("id", "workspaceId", "contactId", "contactInboxId", "inboxId", "sourceId", "conversationIds", "sinceTime")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                   ON CONFLICT ("inboxId", "sourceId") DO UPDATE SET
                     "contactId" = excluded."contactId",
                     "contactInboxId" = excluded."contactInboxId",
                     "conversationIds" = (SELECT json_group_array(DISTINCT value) FROM
                        (SELECT value FROM json_each("MessageCleanup"."conversationIds")
                         UNION SELECT value FROM json_each(excluded."conversationIds"))),
                     "sinceTime" = min(COALESCE("MessageCleanup"."sinceTime", excluded."sinceTime"), COALESCE(excluded."sinceTime", "MessageCleanup"."sinceTime")),
                     "deletedAt" = excluded."deletedAt",
                     "status" = 'pending', "attempts" = 0, "lastError" = NULL, "processedAt" = NULL"#,
                params![new_id(), workspace_id, contact_id, contact_inbox_id, inbox_id, source_id, conversations, since],
            )
            .map_err(repo_err)?;
        }
        tx.execute(
            r#"DELETE FROM "Contact" WHERE "id" IN (SELECT value FROM json_each(?1))"#,
            [&chunk_json],
        )
        .map_err(repo_err)?;
        chatbotx_db::repo::pipeline_activity::scrub_contact_titles(tx, workspace_id, chunk)?;
    }
    Ok(found)
}
