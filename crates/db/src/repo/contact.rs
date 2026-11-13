//! Port of `repositories/contact/repository.ts`. The relational `where` objects of the TS
//! loaders are a SQL filter over `"Contact"` here (`build_contact_list_where` /
//! `build_contact_where`); `with` relations are loaded per contact and ordered by id (the
//! TS left nested relations unordered).

use rusqlite::Connection;

use super::models::{
    Contact, ContactCustomField, ContactInbox, Conversation, CustomField, Inbox, InboxTeam, Tag,
    User,
};
use super::sql::Sql;
use super::utils::{SortDir, order_by_sql};
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
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
    criteria: &crate::repo::queries::contact_filter::FilterCriteria,
    now: i64,
) -> Result<Vec<i64>> {
    if contact_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut q = Sql::new(
        "SELECT \"Contact\".\"id\" FROM \"Contact\" WHERE \"Contact\".\"workspaceId\" = ? \
         AND \"Contact\".\"id\" IN (SELECT value FROM json_each(?)) AND ",
        vec![workspace_id.into(), id_list(contact_ids).into()],
    );
    q.append(
        crate::repo::queries::contact_filter::build_contact_filter_match_sql(
            workspace_id,
            criteria,
            now,
        ),
    );
    let mut stmt = conn.prepare(&q.text)?;
    let ids = stmt
        .query_map(rusqlite::params_from_iter(q.params.iter()), |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(ids)
}

/// `findIdByIdentityWhere`: the first contact of the workspace whose given identity
/// columns (`id`, `email`, `phoneNumber`; `None` = not compared) all match.
pub fn find_id_by_identity_where(
    conn: &Connection,
    workspace_id: i64,
    id: Option<i64>,
    email: Option<&str>,
    phone_number: Option<&str>,
) -> Result<Option<i64>> {
    let mut q = Sql::bind(
        "SELECT \"id\" FROM \"Contact\" WHERE \"workspaceId\" = ",
        workspace_id,
    );
    if let Some(id) = id {
        q.push(" AND \"id\" = ");
        q.push_bind(id);
    }
    if let Some(email) = email {
        q.push(" AND \"email\" = ");
        q.push_bind(email.to_owned());
    }
    if let Some(phone) = phone_number {
        q.push(" AND \"phoneNumber\" = ");
        q.push_bind(phone.to_owned());
    }
    q.push(" LIMIT 1");
    let mut stmt = conn.prepare(&q.text)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(q.params.iter()))?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

/// A contact inbox `with: { inbox: true }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactInboxWithInbox {
    pub contact_inbox: ContactInbox,
    pub inbox: Inbox,
}

/// A conversation `with: { assignedUser: true, assignedInboxTeam: true }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationWithAssignees {
    pub conversation: Conversation,
    pub assigned_user: Option<User>,
    pub assigned_inbox_team: Option<InboxTeam>,
}

/// A contact with `PUBLIC_CONTACT_RELATIONS`.
#[derive(Debug, Clone, PartialEq)]
pub struct PublicContact {
    pub contact: Contact,
    pub tags: Vec<Tag>,
    pub contact_custom_fields: Vec<ContactCustomField>,
    pub contact_inboxes: Vec<ContactInboxWithInbox>,
    pub conversation: Option<ConversationWithAssignees>,
}

/// A contact with `contactInboxes.inbox` and `conversation.{assignedUser, assignedInboxTeam}`
/// (`listWithInboxesAndConversation`).
#[derive(Debug, Clone, PartialEq)]
pub struct ContactWithInboxesAndConversation {
    pub contact: Contact,
    pub contact_inboxes: Vec<ContactInboxWithInbox>,
    pub conversation: Option<ConversationWithAssignees>,
}

/// A contact custom field `with: { customField: true }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactCustomFieldWithDefinition {
    pub value: ContactCustomField,
    pub custom_field: CustomField,
}

/// `findDetailById`: tags, custom fields with their definition, the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactDetail {
    pub contact: Contact,
    pub tags: Vec<Tag>,
    pub contact_custom_fields: Vec<ContactCustomFieldWithDefinition>,
    pub conversation: Option<Conversation>,
}

/// `ContactListInput`: a filter over `"Contact"`, a page and an order
/// (`resolve_contact_order_by`).
#[derive(Debug, Clone, PartialEq)]
pub struct ContactListInput {
    pub filter: Sql,
    pub limit: i64,
    pub offset: i64,
    pub order_by: Vec<(String, SortDir)>,
}

fn tags_of(conn: &Connection, contact_id: i64) -> Result<Vec<Tag>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Tag\" AS t JOIN \"ContactToTag\" AS ct ON ct.\"tagId\" = t.\"id\" WHERE ct.\"contactId\" = ",
        Tag::columns_sql(Some("t"))
    ));
    q.push_bind(contact_id);
    q.push(" ORDER BY t.\"id\"");
    q.models(conn)
}

fn custom_fields_of(conn: &Connection, contact_id: i64) -> Result<Vec<ContactCustomField>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"ContactCustomField\" WHERE \"contactId\" = ",
        ContactCustomField::columns_sql(None)
    ));
    q.push_bind(contact_id);
    q.push(" ORDER BY \"id\"");
    q.models(conn)
}

fn inboxes_of(conn: &Connection, contact_id: i64) -> Result<Vec<ContactInboxWithInbox>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"ContactInbox\" WHERE \"contactId\" = ",
        ContactInbox::columns_sql(None)
    ));
    q.push_bind(contact_id);
    q.push(" ORDER BY \"id\"");
    let rows: Vec<ContactInbox> = q.models(conn)?;
    let mut out = Vec::with_capacity(rows.len());
    for contact_inbox in rows {
        let mut iq = Sql::raw(format!(
            "SELECT {} FROM \"Inbox\" WHERE \"id\" = ",
            Inbox::columns_sql(None)
        ));
        iq.push_bind(contact_inbox.inbox_id);
        if let Some(inbox) = iq.model(conn)? {
            out.push(ContactInboxWithInbox {
                contact_inbox,
                inbox,
            });
        }
    }
    Ok(out)
}

fn conversation_of(conn: &Connection, contact_id: i64) -> Result<Option<Conversation>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Conversation\" WHERE \"contactId\" = ",
        Conversation::columns_sql(None)
    ));
    q.push_bind(contact_id);
    q.push(" ORDER BY \"id\" LIMIT 1");
    q.model(conn)
}

fn by_id<M: Model>(conn: &Connection, id: Option<i64>) -> Result<Option<M>> {
    let Some(id) = id else {
        return Ok(None);
    };
    let mut q = Sql::raw(format!(
        "SELECT {} FROM {} WHERE \"id\" = ",
        M::columns_sql(None),
        M::TABLE
    ));
    q.push_bind(id);
    q.model(conn)
}

fn conversation_with_assignees(
    conn: &Connection,
    contact_id: i64,
) -> Result<Option<ConversationWithAssignees>> {
    let Some(conversation) = conversation_of(conn, contact_id)? else {
        return Ok(None);
    };
    Ok(Some(ConversationWithAssignees {
        assigned_user: by_id(conn, conversation.assigned_user_id)?,
        assigned_inbox_team: by_id(conn, conversation.assigned_inbox_team_id)?,
        conversation,
    }))
}

fn public_contact(conn: &Connection, contact: Contact) -> Result<PublicContact> {
    Ok(PublicContact {
        tags: tags_of(conn, contact.id)?,
        contact_custom_fields: custom_fields_of(conn, contact.id)?,
        contact_inboxes: inboxes_of(conn, contact.id)?,
        conversation: conversation_with_assignees(conn, contact.id)?,
        contact,
    })
}

/// `SELECT <Contact> FROM "Contact" WHERE <filter> [ORDER BY ..] [LIMIT ..] [OFFSET ..]`.
fn contacts_where(
    conn: &Connection,
    filter: &Sql,
    order_by: &[(String, SortDir)],
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<Contact>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Contact\" WHERE ",
        Contact::columns_sql(None)
    ));
    q.append(filter.clone());
    if !order_by.is_empty() {
        q.push(" ");
        q.push(&order_by_sql(order_by, Some("\"Contact\"")));
    }
    q.push(" LIMIT ");
    q.push_bind(limit.unwrap_or(-1));
    if let Some(offset) = offset {
        q.push(" OFFSET ");
        q.push_bind(offset);
    }
    q.models(conn)
}

/// `findPublicById`: the workspace's contact with `PUBLIC_CONTACT_RELATIONS`.
pub fn find_public_by_id(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<PublicContact>> {
    let mut filter = Sql::bind("\"Contact\".\"workspaceId\" = ", workspace_id);
    filter.push(" AND \"Contact\".\"id\" = ");
    filter.push_bind(id);
    match contacts_where(conn, &filter, &[], Some(1), None)?
        .into_iter()
        .next()
    {
        Some(contact) => Ok(Some(public_contact(conn, contact)?)),
        None => Ok(None),
    }
}

/// `listPublicByCustomField`: `{ data }` of the filtered contacts with
/// `PUBLIC_CONTACT_RELATIONS` (no offset).
pub fn list_public_by_custom_field(
    conn: &Connection,
    filter: &Sql,
    limit: i64,
    order_by: &[(String, SortDir)],
) -> Result<Vec<PublicContact>> {
    contacts_where(conn, filter, order_by, Some(limit), None)?
        .into_iter()
        .map(|c| public_contact(conn, c))
        .collect()
}

/// `listWithRelations`: one page with `PUBLIC_CONTACT_RELATIONS`.
pub fn list_with_relations(
    conn: &Connection,
    input: &ContactListInput,
) -> Result<Vec<PublicContact>> {
    contacts_where(
        conn,
        &input.filter,
        &input.order_by,
        Some(input.limit),
        Some(input.offset),
    )?
    .into_iter()
    .map(|c| public_contact(conn, c))
    .collect()
}

/// `listWithInboxesAndConversation`.
pub fn list_with_inboxes_and_conversation(
    conn: &Connection,
    input: &ContactListInput,
) -> Result<Vec<ContactWithInboxesAndConversation>> {
    contacts_where(
        conn,
        &input.filter,
        &input.order_by,
        Some(input.limit),
        Some(input.offset),
    )?
    .into_iter()
    .map(|contact| {
        Ok(ContactWithInboxesAndConversation {
            contact_inboxes: inboxes_of(conn, contact.id)?,
            conversation: conversation_with_assignees(conn, contact.id)?,
            contact,
        })
    })
    .collect()
}

/// A `listTableRows` contact inbox: `(channel, source, contactLastReadAt)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRowInbox {
    pub channel: String,
    pub source: String,
    pub contact_last_read_at: Option<i64>,
}

/// A `listTableRows` conversation: its id and the assigned user's `(name, email)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRowConversation {
    pub id: i64,
    pub assigned_user: Option<(Option<String>, String)>,
}

/// One `listTableRows` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactTableRow {
    pub id: i64,
    pub full_name: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub avatar: Option<String>,
    pub created_at: i64,
    pub contact_inboxes: Vec<TableRowInbox>,
    pub conversation: Option<TableRowConversation>,
}

/// `listTableRows`: the contact table columns of one page.
pub fn list_table_rows(
    conn: &Connection,
    input: &ContactListInput,
) -> Result<Vec<ContactTableRow>> {
    let contacts = contacts_where(
        conn,
        &input.filter,
        &input.order_by,
        Some(input.limit),
        Some(input.offset),
    )?;
    let mut out = Vec::with_capacity(contacts.len());
    for c in contacts {
        let mut stmt = conn.prepare_cached(
            "SELECT \"channel\", \"source\", \"contactLastReadAt\" FROM \"ContactInbox\" WHERE \"contactId\" = ?1 ORDER BY \"id\"",
        )?;
        let contact_inboxes = stmt
            .query_map([c.id], |r| {
                Ok(TableRowInbox {
                    channel: r.get(0)?,
                    source: r.get(1)?,
                    contact_last_read_at: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let conversation = match conversation_of(conn, c.id)? {
            Some(conv) => Some(TableRowConversation {
                id: conv.id,
                assigned_user: by_id::<User>(conn, conv.assigned_user_id)?
                    .map(|u| (u.name, u.email)),
            }),
            None => None,
        };
        out.push(ContactTableRow {
            id: c.id,
            full_name: c.full_name,
            email: c.email,
            phone_number: c.phone_number,
            avatar: c.avatar,
            created_at: c.created_at,
            contact_inboxes,
            conversation,
        });
    }
    Ok(out)
}

/// `findDetailById`.
pub fn find_detail_by_id(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<ContactDetail>> {
    let mut filter = Sql::bind("\"Contact\".\"workspaceId\" = ", workspace_id);
    filter.push(" AND \"Contact\".\"id\" = ");
    filter.push_bind(id);
    let Some(contact) = contacts_where(conn, &filter, &[], Some(1), None)?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let mut fields = Vec::new();
    for value in custom_fields_of(conn, contact.id)? {
        if let Some(custom_field) = by_id::<CustomField>(conn, Some(value.custom_field_id))? {
            fields.push(ContactCustomFieldWithDefinition {
                value,
                custom_field,
            });
        }
    }
    Ok(Some(ContactDetail {
        tags: tags_of(conn, contact.id)?,
        contact_custom_fields: fields,
        conversation: conversation_of(conn, contact.id)?,
        contact,
    }))
}

/// `count`: `count(*)` of the contacts matching the filter.
pub fn count(conn: &Connection, filter: &Sql) -> Result<i64> {
    let sql = format!("SELECT count(*) FROM \"Contact\" WHERE {}", filter.text);
    Ok(conn.query_row(
        &sql,
        rusqlite::params_from_iter(filter.params.iter()),
        |r| r.get(0),
    )?)
}

/// `countCapped`: [`count`] up to `cap` (`countWithRelationsFilterCapped`).
pub fn count_capped(
    conn: &Connection,
    filter: &Sql,
    cap: i64,
) -> Result<super::client::CappedCount> {
    let query = format!("SELECT 1 FROM \"Contact\" WHERE {}", filter.text);
    let params: Vec<&dyn rusqlite::types::ToSql> = filter
        .params
        .iter()
        .map(|p| p as &dyn rusqlite::types::ToSql)
        .collect();
    super::client::count_query_capped(conn, cap, &query, &params)
}

/// `sumTotalContactsFromInboxStats`: the sum of `InboxContactStat.totalContacts` over the
/// workspace's inboxes (0 without stats).
pub fn sum_total_contacts_from_inbox_stats(conn: &Connection, workspace_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(s.\"totalContacts\"), 0) FROM \"Inbox\" AS i \
         JOIN \"InboxContactStat\" AS s ON s.\"inboxId\" = i.\"id\" WHERE i.\"workspaceId\" = ?1",
        [workspace_id],
        |r| r.get(0),
    )?)
}

/// `enrichIfNull`: fills `phoneNumber` / `email` only where they are still NULL.
pub fn enrich_if_null(
    conn: &Connection,
    contact_id: i64,
    phone_number: Option<&str>,
    email: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE \"Contact\" SET \"phoneNumber\" = COALESCE(\"phoneNumber\", ?2), \"email\" = COALESCE(\"email\", ?3) \
         WHERE \"id\" = ?1 AND ((?2 IS NOT NULL AND \"phoneNumber\" IS NULL) OR (?3 IS NOT NULL AND \"email\" IS NULL))",
        rusqlite::params![contact_id, phone_number, email],
    )?;
    Ok(())
}
