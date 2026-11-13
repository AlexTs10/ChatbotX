//! Port of `repositories/contact-inbox/repository.ts` (identity, attribution, ad
//! eligibility and thread-control writes; the bulk-AI eligibility and the relational
//! finders are listed as not ported yet in `conversion/specs/P3.2.md`).

use rusqlite::Connection;
use rusqlite::types::Value;
use serde_json::{Value as J, json};

use super::models::{Contact, ContactInbox, Conversation};
use super::partials::thread_control::{events_outranked_by, transition};
use super::queries::ad_referral::ad_conversation_predicate;
use super::queries::ai_handover_bulk_eligibility::{
    BulkEligibilityInput, bulk_eligibility_conditions, bulk_eligibility_where,
};
use super::sql::Sql;
use super::{Error, Json, Model, Result, id_list, text_list};

/// `CONTACT_INBOX_IDENTITY_HISTORY_LIMIT`.
pub const CONTACT_INBOX_IDENTITY_HISTORY_LIMIT: usize = 10;

/// `ContactInboxIdentityGuard` / `ContactInboxIdentitySet` fields: `None` = not part of
/// the guard / set; `Some(None)` = `IS NULL` / set NULL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityFields {
    pub source_id: Option<Option<String>>,
    pub source_user_id: Option<Option<String>>,
    pub source_parent_user_id: Option<Option<String>>,
}

impl IdentityFields {
    fn entries(&self) -> Vec<(&'static str, &Option<String>)> {
        let mut out = Vec::new();
        if let Some(v) = &self.source_id {
            out.push(("sourceId", v));
        }
        if let Some(v) = &self.source_user_id {
            out.push(("sourceUserId", v));
        }
        if let Some(v) = &self.source_parent_user_id {
            out.push(("sourceParentUserId", v));
        }
        out
    }
}

/// `appendIdentityHistory` of [`update_identity_guarded`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityHistoryAppend {
    pub changed_at: String,
    /// `ContactInboxIdentityChangeReason`.
    pub reason: String,
}

/// `updateIdentityGuarded`: compare-and-swap of the identity columns (at least one guard
/// field is required). With `append`, the row's identity BEFORE the update is appended to
/// `sourceIdentityHistory` (newest [`CONTACT_INBOX_IDENTITY_HISTORY_LIMIT`] kept). Runs
/// in one write transaction; `None` when the guard no longer matches.
pub fn update_identity_guarded(
    conn: &Connection,
    id: i64,
    guard: &IdentityFields,
    set: &IdentityFields,
    append: Option<&IdentityHistoryAppend>,
) -> Result<Option<ContactInbox>> {
    let guards = guard.entries();
    if guards.is_empty() {
        return Err(Error::msg("ContactInbox identity update requires a guard"));
    }
    let mut filter = vec![Some(Sql::bind("\"id\" = ", id))];
    for (col, value) in &guards {
        filter.push(Some(match value {
            Some(v) => Sql::bind(format!("\"{col}\" = "), v.clone()),
            None => Sql::raw(format!("\"{col}\" IS NULL")),
        }));
    }
    let filter = Sql::and(filter);
    let mut select = Sql::raw(format!("{} WHERE ", ContactInbox::select_sql()));
    select.append(filter.clone());
    let Some(current) = select.model::<ContactInbox>(conn)? else {
        return Ok(None);
    };
    let mut q = Sql::raw("UPDATE \"ContactInbox\" SET \"updatedAt\" = \"updatedAt\"");
    for (col, value) in set.entries() {
        q.push(&format!(", \"{col}\" = "));
        q.push_bind(value.clone().map_or(Value::Null, Value::Text));
    }
    if let Some(append) = append {
        let mut history = current
            .source_identity_history
            .as_ref()
            .and_then(|h| h.0.as_array().cloned())
            .unwrap_or_default();
        history.push(json!({
            "sourceId": current.source_id,
            "sourceUserId": current.source_user_id,
            "sourceParentUserId": current.source_parent_user_id,
            "changedAt": append.changed_at,
            "reason": append.reason,
        }));
        let skip = history
            .len()
            .saturating_sub(CONTACT_INBOX_IDENTITY_HISTORY_LIMIT);
        let kept: Vec<J> = history.into_iter().skip(skip).collect();
        q.push(", \"sourceIdentityHistory\" = ");
        q.push_bind(J::Array(kept).to_string());
    }
    q.push(" WHERE ");
    q.append(filter);
    q.push(&format!(" RETURNING {}", ContactInbox::columns_sql(None)));
    q.model(conn)
}

/// `ContactInboxWorkspaceRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRow {
    pub id: i64,
    pub channel: String,
    pub inbox_id: i64,
    pub source_id: String,
}

/// `findByIdForWorkspace`: the row when its inbox belongs to the workspace.
pub fn find_by_id_for_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<WorkspaceRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT ci.\"id\", ci.\"channel\", ci.\"inboxId\", ci.\"sourceId\" FROM \"ContactInbox\" AS ci \
         INNER JOIN \"Inbox\" AS i ON i.\"id\" = ci.\"inboxId\" AND i.\"workspaceId\" = ?2 WHERE ci.\"id\" = ?1 LIMIT 1",
    )?;
    let mut rows = stmt.query([id, workspace_id])?;
    Ok(match rows.next()? {
        Some(r) => Some(WorkspaceRow {
            id: r.get(0)?,
            channel: r.get(1)?,
            inbox_id: r.get(2)?,
            source_id: r.get(3)?,
        }),
        None => None,
    })
}

/// `findModelByIdForWorkspace`.
pub fn find_model_by_id_for_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<ContactInbox>> {
    let sql = format!(
        "SELECT {} FROM \"ContactInbox\" AS ci INNER JOIN \"Inbox\" AS i ON i.\"id\" = ci.\"inboxId\" AND i.\"workspaceId\" = ?2 WHERE ci.\"id\" = ?1 LIMIT 1",
        ContactInbox::columns_sql(Some("ci"))
    );
    Ok(super::query_models(conn, &sql, &[&id, &workspace_id])?
        .into_iter()
        .next())
}

/// One row of [`list_whatsapp_ctwa_inboxes_by_contacts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtwaInboxRow {
    pub contact_id: i64,
    pub contact_inbox_id: i64,
    pub integration_whatsapp_id: i64,
}

/// `listWhatsappCtwaInboxesByContacts` (and `...ByContact` with one id): the contact's
/// WhatsApp inboxes with a CTWA click id, with the workspace's integration.
pub fn list_whatsapp_ctwa_inboxes_by_contacts(
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
) -> Result<Vec<CtwaInboxRow>> {
    if contact_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT ci.\"contactId\", ci.\"id\", iw.\"id\" FROM \"ContactInbox\" AS ci \
         INNER JOIN \"IntegrationWhatsapp\" AS iw ON ci.\"inboxId\" = iw.\"inboxId\" AND iw.\"workspaceId\" = ?1 \
         WHERE ci.\"contactId\" IN (SELECT value FROM json_each(?2)) AND ci.\"channel\" = 'whatsapp' \
           AND ci.\"referral\"->>'ctwaClid' IS NOT NULL ORDER BY ci.\"id\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, id_list(contact_ids)], |r| {
            Ok(CtwaInboxRow {
                contact_id: r.get(0)?,
                contact_inbox_id: r.get(1)?,
                integration_whatsapp_id: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listWhatsappCtwaInboxesByContact`: `(contactInboxId, integrationWhatsappId)`.
pub fn list_whatsapp_ctwa_inboxes_by_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<(i64, i64)>> {
    Ok(
        list_whatsapp_ctwa_inboxes_by_contacts(conn, workspace_id, &[contact_id])?
            .into_iter()
            .map(|r| (r.contact_inbox_id, r.integration_whatsapp_id))
            .collect(),
    )
}

/// `AdEligibleInboxByContactRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdEligibleRow {
    pub contact_id: i64,
    pub contact_inbox_id: i64,
    pub channel: String,
    pub integration_id: i64,
}

/// `listAdEligibleInboxesByContacts`: per ads-eligible channel (whatsapp: CTWA click id;
/// messenger / instagram: paid ad referral), tagged with the channel; channel order.
pub fn list_ad_eligible_inboxes_by_contacts(
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
) -> Result<Vec<AdEligibleRow>> {
    if contact_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for (channel, table) in [
        ("whatsapp", "IntegrationWhatsapp"),
        ("messenger", "IntegrationMessenger"),
        ("instagram", "IntegrationInstagram"),
    ] {
        let referral = if channel == "whatsapp" {
            Sql::raw("ci.\"referral\"->>'ctwaClid' IS NOT NULL")
        } else {
            ad_conversation_predicate("ci", channel)
        };
        let mut q = Sql::new(
            format!(
                "SELECT ci.\"contactId\", ci.\"id\", i.\"id\" FROM \"ContactInbox\" AS ci \
                 INNER JOIN \"{table}\" AS i ON ci.\"inboxId\" = i.\"inboxId\" AND i.\"workspaceId\" = ? \
                 WHERE ci.\"contactId\" IN (SELECT value FROM json_each(?)) AND ci.\"channel\" = ? AND "
            ),
            vec![
                Value::Integer(workspace_id),
                Value::Text(id_list(contact_ids)),
                Value::Text(channel.to_owned()),
            ],
        );
        q.append(referral);
        q.push(" ORDER BY ci.\"id\"");
        out.extend(q.query_map(conn, |r| {
            Ok(AdEligibleRow {
                contact_id: r.get(0)?,
                contact_inbox_id: r.get(1)?,
                channel: channel.to_owned(),
                integration_id: r.get(2)?,
            })
        })?);
    }
    Ok(out)
}

/// `ContactInboxBySourceIdRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BySourceIdRow {
    pub id: i64,
    pub source_id: String,
    pub last_incoming_message_at: Option<i64>,
    pub created_at: i64,
}

/// `findByInboxAndSourceIds`: rows of the inbox with one of `source_ids` (deduplicated;
/// no query for an empty list; rows without a source id are dropped).
pub fn find_by_inbox_and_source_ids(
    conn: &Connection,
    inbox_id: i64,
    source_ids: &[String],
) -> Result<Vec<BySourceIdRow>> {
    let mut ids: Vec<String> = source_ids.to_vec();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"sourceId\", \"lastIncomingMessageAt\", \"createdAt\" FROM \"ContactInbox\" \
         WHERE \"inboxId\" = ?1 AND \"sourceId\" IN (SELECT value FROM json_each(?2)) ORDER BY \"id\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![inbox_id, text_list(&ids)], |r| {
            Ok(BySourceIdRow {
                id: r.get(0)?,
                source_id: r.get(1)?,
                last_incoming_message_at: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|r| !r.source_id.is_empty())
        .collect())
}

/// `listLatestAnalyticsAttribution`: per contact of the workspace, the most recently
/// active connection `(contactId, inboxId, channel)` (`lastMessageAt DESC NULLS LAST, id
/// DESC`).
pub fn list_latest_analytics_attribution(
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
) -> Result<Vec<(i64, i64, String)>> {
    if contact_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"contactId\", \"inboxId\", \"channel\" FROM ( \
           SELECT ci.\"contactId\", ci.\"inboxId\", ci.\"channel\", \
             row_number() OVER (PARTITION BY ci.\"contactId\" ORDER BY ci.\"lastMessageAt\" IS NULL, ci.\"lastMessageAt\" DESC, ci.\"id\" DESC) AS rn \
           FROM \"ContactInbox\" AS ci INNER JOIN \"Contact\" AS c ON c.\"id\" = ci.\"contactId\" \
           WHERE ci.\"contactId\" IN (SELECT value FROM json_each(?1)) AND c.\"workspaceId\" = ?2) \
         WHERE rn = 1 ORDER BY \"contactId\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![id_list(contact_ids), workspace_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One row of [`list_analytics_attribution_by_contact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionRow {
    pub inbox_id: i64,
    pub channel: String,
    pub source: String,
    pub source_id: String,
}

/// `listAnalyticsAttributionByContact`: every connection of the contact in the workspace.
pub fn list_analytics_attribution_by_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<AttributionRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT ci.\"inboxId\", ci.\"channel\", ci.\"source\", ci.\"sourceId\" FROM \"ContactInbox\" AS ci \
         INNER JOIN \"Contact\" AS c ON c.\"id\" = ci.\"contactId\" WHERE ci.\"contactId\" = ?1 AND c.\"workspaceId\" = ?2 ORDER BY ci.\"id\"",
    )?;
    let rows = stmt
        .query_map([contact_id, workspace_id], |r| {
            Ok(AttributionRow {
                inbox_id: r.get(0)?,
                channel: r.get(1)?,
                source: r.get(2)?,
                source_id: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `ApplyThreadControlTransitionInput`. `thread_owner_expires_at`: `None` = untouched.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadControlTransition {
    pub id: i64,
    pub workspace_id: i64,
    pub event: String,
    pub owner_role: Option<String>,
    pub owner_app_id: Option<String>,
    pub previous_owner_app_id: Option<String>,
    pub thread_owner_expires_at: Option<Option<i64>>,
    pub occurred_at: i64,
}

fn workspace_exists(workspace_id: i64) -> Sql {
    Sql::bind(
        "EXISTS (SELECT 1 FROM \"Inbox\" AS i WHERE i.\"id\" = \"ContactInbox\".\"inboxId\" AND i.\"workspaceId\" = ",
        workspace_id,
    )
    .closed(")")
}

fn set_expires(q: &mut Sql, v: Option<Option<i64>>) {
    if let Some(v) = v {
        q.push(", \"threadOwnerExpiresAt\" = ");
        q.push_bind(v.map_or(Value::Null, Value::Integer));
    }
}

/// `applyThreadControlTransition`: writes the event's state when it is newer than the
/// stored transition, or ties and outranks it, or is an exact redelivery (same event,
/// state, role and owner app). Workspace-scoped; `None` when rejected.
pub fn apply_thread_control_transition(
    conn: &Connection,
    input: &ThreadControlTransition,
) -> Result<Option<ContactInbox>> {
    let Some(state) = transition(&input.event) else {
        return Err(Error::msg(format!(
            "unknown thread control event {}",
            input.event
        )));
    };
    let mut q = Sql::new(
        "UPDATE \"ContactInbox\" SET \"threadControlState\" = ?, \"threadOwnerRole\" = ?, \"threadOwnerAppId\" = ?, \
         \"threadPreviousOwnerAppId\" = ?, \"threadControlUpdatedAt\" = ?, \"threadControlLastEvent\" = ?",
        vec![
            Value::Text(state.to_owned()),
            input.owner_role.clone().map_or(Value::Null, Value::Text),
            input.owner_app_id.clone().map_or(Value::Null, Value::Text),
            input
                .previous_owner_app_id
                .clone()
                .map_or(Value::Null, Value::Text),
            Value::Integer(input.occurred_at),
            Value::Text(input.event.clone()),
        ],
    );
    set_expires(&mut q, input.thread_owner_expires_at);
    q.push(" WHERE \"id\" = ");
    q.push_bind(input.id);
    q.push(" AND ");
    q.append(workspace_exists(input.workspace_id));
    q.push(" AND (\"threadControlUpdatedAt\" IS NULL OR \"threadControlUpdatedAt\" < ");
    q.push_bind(input.occurred_at);
    q.push(" OR (\"threadControlUpdatedAt\" = ");
    q.push_bind(input.occurred_at);
    q.push(" AND (");
    let outranked = events_outranked_by(&input.event);
    if !outranked.is_empty() {
        q.push("\"threadControlLastEvent\" IN (SELECT value FROM json_each(");
        q.push_bind(text_list(outranked));
        q.push(")) OR ");
    }
    q.push("(\"threadControlLastEvent\" = ");
    q.push_bind(input.event.clone());
    q.push(" AND \"threadControlState\" = ");
    q.push_bind(state.to_owned());
    q.push(" AND \"threadOwnerRole\" IS ");
    q.push_bind(input.owner_role.clone().map_or(Value::Null, Value::Text));
    q.push(" AND \"threadOwnerAppId\" IS ");
    q.push_bind(input.owner_app_id.clone().map_or(Value::Null, Value::Text));
    q.push(")))) ");
    q.push(&format!("RETURNING {}", ContactInbox::columns_sql(None)));
    q.model(conn)
}

/// `promoteStandbyToOwnerDelivery`: the owner copy of a standby message, at the copy's
/// own time, only while the row is still that standby copy.
pub fn promote_standby_to_owner_delivery(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    owner_role: Option<&str>,
    previous_owner_app_id: Option<&str>,
    thread_owner_expires_at: Option<Option<i64>>,
    occurred_at: i64,
) -> Result<Option<ContactInbox>> {
    let mut q = Sql::new(
        "UPDATE \"ContactInbox\" SET \"threadControlState\" = 'owned', \"threadOwnerRole\" = ?, \"threadOwnerAppId\" = NULL, \
         \"threadPreviousOwnerAppId\" = ?, \"threadControlUpdatedAt\" = ?, \"threadControlLastEvent\" = 'inboundReceived'",
        vec![
            owner_role.map_or(Value::Null, |s| Value::Text(s.to_owned())),
            previous_owner_app_id.map_or(Value::Null, |s| Value::Text(s.to_owned())),
            Value::Integer(occurred_at),
        ],
    );
    set_expires(&mut q, thread_owner_expires_at);
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"threadControlLastEvent\" = 'standbyReceived' AND \"threadControlUpdatedAt\" = ");
    q.push_bind(occurred_at);
    q.push(" AND ");
    q.append(workspace_exists(workspace_id));
    q.push(&format!(" RETURNING {}", ContactInbox::columns_sql(None)));
    q.model(conn)
}

/// `setStandbyThreadOwnerExpiresAt`: lands only against the observed ownership version
/// (standby, same owner app, same `threadControlUpdatedAt`).
pub fn set_standby_thread_owner_expires_at(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    owner_app_id: Option<&str>,
    observed_updated_at: Option<i64>,
    thread_owner_expires_at: Option<i64>,
) -> Result<Option<ContactInbox>> {
    let mut q = Sql::bind(
        "UPDATE \"ContactInbox\" SET \"threadOwnerExpiresAt\" = ",
        thread_owner_expires_at.map_or(Value::Null, Value::Integer),
    );
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.push(" AND \"threadControlState\" = 'standby' AND \"threadOwnerAppId\" IS ");
    q.push_bind(owner_app_id.map_or(Value::Null, |s| Value::Text(s.to_owned())));
    match observed_updated_at {
        Some(at) => {
            q.push(" AND \"threadControlUpdatedAt\" = ");
            q.push_bind(at);
        }
        None => {
            q.push(" AND \"threadControlUpdatedAt\" IS NULL");
        }
    }
    q.push(" AND ");
    q.append(workspace_exists(workspace_id));
    q.push(&format!(" RETURNING {}", ContactInbox::columns_sql(None)));
    q.model(conn)
}

/// `ThreadControlledContactInboxRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadControlledRow {
    pub id: i64,
    pub contact_id: i64,
    pub inbox_id: i64,
    pub channel: String,
    pub thread_control_state: Option<String>,
    pub thread_control_updated_at: Option<i64>,
    pub thread_owner_expires_at: Option<i64>,
    pub last_incoming_message_at: Option<i64>,
}

/// `listThreadControlledByContactIds`: the `owned` rows of the workspace's contacts.
pub fn list_thread_controlled_by_contact_ids(
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
) -> Result<Vec<ThreadControlledRow>> {
    if contact_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT ci.\"id\", ci.\"contactId\", ci.\"inboxId\", ci.\"channel\", ci.\"threadControlState\", ci.\"threadControlUpdatedAt\", \
         ci.\"threadOwnerExpiresAt\", ci.\"lastIncomingMessageAt\" FROM \"ContactInbox\" AS ci \
         INNER JOIN \"Inbox\" AS i ON i.\"id\" = ci.\"inboxId\" AND i.\"workspaceId\" = ?1 \
         WHERE ci.\"contactId\" IN (SELECT value FROM json_each(?2)) AND ci.\"threadControlState\" = 'owned' ORDER BY ci.\"id\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, id_list(contact_ids)], |r| {
            Ok(ThreadControlledRow {
                id: r.get(0)?,
                contact_id: r.get(1)?,
                inbox_id: r.get(2)?,
                channel: r.get(3)?,
                thread_control_state: r.get(4)?,
                thread_control_updated_at: r.get(5)?,
                thread_owner_expires_at: r.get(6)?,
                last_incoming_message_at: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The stored identity history as JSON (`[]` when NULL).
pub fn identity_history(row: &ContactInbox) -> J {
    row.source_identity_history
        .clone()
        .map_or_else(|| J::Array(Vec::new()), |Json(v)| v)
}

/// One `listWithInboxNameByContactId` row (the selected columns + `inbox.name`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithInboxNameRow {
    pub id: i64,
    pub contact_id: i64,
    pub inbox_id: i64,
    pub channel: String,
    pub source: String,
    pub source_id: String,
    pub source_user_id: Option<String>,
    pub source_username: Option<String>,
    pub language: Option<String>,
    pub last_incoming_message_at: Option<i64>,
    pub contact_last_read_at: Option<i64>,
    pub inbox_name: String,
}

/// `listWithInboxNameByContactId`: the contact's rows whose inbox is in the workspace, by id.
pub fn list_with_inbox_name_by_contact_id(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<WithInboxNameRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT ci.\"id\", ci.\"contactId\", ci.\"inboxId\", ci.\"channel\", ci.\"source\", ci.\"sourceId\", \
         ci.\"sourceUserId\", ci.\"sourceUsername\", ci.\"language\", ci.\"lastIncomingMessageAt\", ci.\"contactLastReadAt\", i.\"name\" \
         FROM \"ContactInbox\" AS ci JOIN \"Inbox\" AS i ON i.\"id\" = ci.\"inboxId\" AND i.\"workspaceId\" = ?2 \
         WHERE ci.\"contactId\" = ?1 ORDER BY ci.\"id\" ASC",
    )?;
    let rows = stmt
        .query_map([contact_id, workspace_id], |r| {
            Ok(WithInboxNameRow {
                id: r.get(0)?,
                contact_id: r.get(1)?,
                inbox_id: r.get(2)?,
                channel: r.get(3)?,
                source: r.get(4)?,
                source_id: r.get(5)?,
                source_user_id: r.get(6)?,
                source_username: r.get(7)?,
                language: r.get(8)?,
                last_incoming_message_at: r.get(9)?,
                contact_last_read_at: r.get(10)?,
                inbox_name: r.get(11)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `(contactId, contactInboxId)` of the identity finders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactAndInbox {
    pub contact_id: i64,
    pub contact_inbox_id: i64,
}

/// The `Contact` identity column of [`find_latest_by_inbox_and_contact_identity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityField {
    PhoneNumber,
    Email,
}

/// `findLatestByInboxAndContactIdentity`: the inbox's row whose workspace contact has that
/// phone number / email, most recent message first.
pub fn find_latest_by_inbox_and_contact_identity(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    field: IdentityField,
    value: &str,
) -> Result<Option<ContactAndInbox>> {
    let column = match field {
        IdentityField::PhoneNumber => "phoneNumber",
        IdentityField::Email => "email",
    };
    let sql = format!(
        "SELECT ci.\"contactId\", ci.\"id\" FROM \"ContactInbox\" AS ci JOIN \"Contact\" AS c ON c.\"id\" = ci.\"contactId\" \
         WHERE ci.\"inboxId\" = ?1 AND c.\"workspaceId\" = ?2 AND c.\"{column}\" = ?3 \
         ORDER BY ci.\"lastMessageAt\" DESC NULLS LAST LIMIT 1"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query_map(rusqlite::params![inbox_id, workspace_id, value], |r| {
        Ok(ContactAndInbox {
            contact_id: r.get(0)?,
            contact_inbox_id: r.get(1)?,
        })
    })?;
    Ok(rows.next().transpose()?)
}

/// `normalizedFullNameSql`: runs of white space collapsed to one space, trimmed.
const NORMALIZED_FULL_NAME: &str =
    "trim(regexp_replace(c.\"fullName\", '[ \\t\\n\\v\\f\\r]+', ' ', 'g'))";

/// `findByInboxAndFullName`: at most 2 people (one row per contact, their most recent row)
/// of the inbox whose normalized full name equals `full_name`, most recently active first.
pub fn find_by_inbox_and_full_name(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    full_name: &str,
) -> Result<Vec<ContactAndInbox>> {
    let sql = format!(
        "SELECT x.\"contactId\", x.\"id\" FROM (SELECT ci.\"contactId\", ci.\"id\", ci.\"lastMessageAt\", \
         row_number() OVER (PARTITION BY ci.\"contactId\" ORDER BY ci.\"lastMessageAt\" DESC NULLS LAST, ci.\"id\" DESC) AS rn, \
         max(ci.\"lastMessageAt\") OVER (PARTITION BY ci.\"contactId\") AS active \
         FROM \"ContactInbox\" AS ci JOIN \"Contact\" AS c ON c.\"id\" = ci.\"contactId\" \
         WHERE ci.\"inboxId\" = ?1 AND c.\"workspaceId\" = ?2 AND {NORMALIZED_FULL_NAME} = ?3) AS x \
         WHERE x.rn = 1 ORDER BY x.active DESC NULLS LAST, x.\"contactId\" LIMIT 2"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params![inbox_id, workspace_id, full_name], |r| {
            Ok(ContactAndInbox {
                contact_id: r.get(0)?,
                contact_inbox_id: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `SELECT <ContactInbox> FROM "ContactInbox" AS ci JOIN "Inbox" AS i (workspace) WHERE ..`.
fn scoped_models(conn: &Connection, workspace_id: i64, tail: Sql) -> Result<Vec<ContactInbox>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"ContactInbox\" AS ci JOIN \"Inbox\" AS i ON i.\"id\" = ci.\"inboxId\" AND i.\"workspaceId\" = ",
        ContactInbox::columns_sql(Some("ci"))
    ));
    q.push_bind(workspace_id);
    q.push(" WHERE ");
    q.append(tail);
    q.models(conn)
}

fn workspace_row(ci: ContactInbox) -> WorkspaceRow {
    WorkspaceRow {
        id: ci.id,
        channel: ci.channel,
        inbox_id: ci.inbox_id,
        source_id: ci.source_id,
    }
}

/// `findFullByIdForWorkspace`: the full row when its inbox is in the workspace.
pub fn find_full_by_id_for_workspace(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<ContactInbox>> {
    let tail = Sql::bind("ci.\"id\" = ", id).closed(" LIMIT 1");
    Ok(scoped_models(conn, workspace_id, tail)?.into_iter().next())
}

/// `findByIdForContact`: the contact's row (inbox in the workspace).
pub fn find_by_id_for_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    id: i64,
) -> Result<Option<WorkspaceRow>> {
    let mut tail = Sql::bind("ci.\"id\" = ", id);
    tail.push(" AND ci.\"contactId\" = ");
    tail.push_bind(contact_id);
    tail.push(" LIMIT 1");
    Ok(scoped_models(conn, workspace_id, tail)?
        .into_iter()
        .next()
        .map(workspace_row))
}

/// `findMostRecentByContact`: the contact's row with the most recent message.
pub fn find_most_recent_by_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<WorkspaceRow>> {
    let tail = Sql::bind("ci.\"contactId\" = ", contact_id)
        .closed(" ORDER BY ci.\"lastMessageAt\" DESC NULLS LAST LIMIT 1");
    Ok(scoped_models(conn, workspace_id, tail)?
        .into_iter()
        .next()
        .map(workspace_row))
}

/// `GoogleClickInboxRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct GoogleClickInboxRow {
    pub id: i64,
    pub channel: String,
    pub contact_id: i64,
    pub referral: Option<Json>,
    pub source_id: String,
}

/// `googleClickPredicate` over `ci`.
const GOOGLE_CLICK: &str =
    "(ci.\"referral\"->>'gclid' IS NOT NULL OR ci.\"referral\"->>'gbraid' IS NOT NULL)";

fn google_click_row(ci: ContactInbox) -> GoogleClickInboxRow {
    GoogleClickInboxRow {
        id: ci.id,
        channel: ci.channel,
        contact_id: ci.contact_id,
        referral: ci.referral,
        source_id: ci.source_id,
    }
}

/// `findGoogleClickAttribution`: the row when it carries a Google click (gclid / gbraid).
pub fn find_google_click_attribution(
    conn: &Connection,
    workspace_id: i64,
    contact_inbox_id: i64,
) -> Result<Option<GoogleClickInboxRow>> {
    let tail =
        Sql::bind("ci.\"id\" = ", contact_inbox_id).closed(&format!(" AND {GOOGLE_CLICK} LIMIT 1"));
    Ok(scoped_models(conn, workspace_id, tail)?
        .into_iter()
        .next()
        .map(google_click_row))
}

/// `findLatestGoogleClickInboxByContact`: the contact's Google-click row with the latest
/// `googleClickReceivedAt` (ISO text order, as the TS).
pub fn find_latest_google_click_inbox_by_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<GoogleClickInboxRow>> {
    let tail = Sql::bind("ci.\"contactId\" = ", contact_id).closed(&format!(
        " AND {GOOGLE_CLICK} ORDER BY ci.\"referral\"->>'googleClickReceivedAt' DESC NULLS LAST LIMIT 1"
    ));
    Ok(scoped_models(conn, workspace_id, tail)?
        .into_iter()
        .next()
        .map(google_click_row))
}

/// A contact inbox with its contact (and the contact's conversation).
#[derive(Debug, Clone, PartialEq)]
pub struct WithContact {
    pub contact_inbox: ContactInbox,
    pub contact: Contact,
    /// Loaded by [`find_with_conversation_and_contact`] only.
    pub conversation: Option<Conversation>,
}

fn first_where(conn: &Connection, filter: &Sql) -> Result<Option<ContactInbox>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"ContactInbox\" WHERE ",
        ContactInbox::columns_sql(None)
    ));
    q.append(filter.clone());
    q.push(" LIMIT 1");
    q.model(conn)
}

fn by_id<M: Model>(conn: &Connection, column: &str, id: i64) -> Result<Option<M>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM {} WHERE \"{column}\" = ",
        M::columns_sql(None),
        M::TABLE
    ));
    q.push_bind(id);
    q.push(" ORDER BY \"id\" LIMIT 1");
    q.model(conn)
}

/// `findWithConversationAndContact`: the first row matching `filter` (over
/// `"ContactInbox"`) with its contact and the contact's conversation. The full row is
/// returned (the TS left out `sourceIdentityHistory`).
pub fn find_with_conversation_and_contact(
    conn: &Connection,
    filter: &Sql,
) -> Result<Option<WithContact>> {
    let Some(contact_inbox) = first_where(conn, filter)? else {
        return Ok(None);
    };
    let Some(contact) = by_id::<Contact>(conn, "id", contact_inbox.contact_id)? else {
        return Ok(None);
    };
    Ok(Some(WithContact {
        conversation: by_id(conn, "contactId", contact_inbox.contact_id)?,
        contact_inbox,
        contact,
    }))
}

/// `findWithContact`: as [`find_with_conversation_and_contact`] without the conversation.
pub fn find_with_contact(conn: &Connection, filter: &Sql) -> Result<Option<WithContact>> {
    let Some(contact_inbox) = first_where(conn, filter)? else {
        return Ok(None);
    };
    Ok(
        by_id::<Contact>(conn, "id", contact_inbox.contact_id)?.map(|contact| WithContact {
            contact_inbox,
            contact,
            conversation: None,
        }),
    )
}

/// `listIdsByInboxAndSourceIds`: `(id, contactId, sourceId)` of the inbox's rows.
pub fn list_ids_by_inbox_and_source_ids(
    conn: &Connection,
    inbox_id: i64,
    source_ids: &[String],
) -> Result<Vec<(i64, i64, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\", \"contactId\", \"sourceId\" FROM \"ContactInbox\" WHERE \"inboxId\" = ?1 \
         AND \"sourceId\" IN (SELECT value FROM json_each(?2)) ORDER BY \"id\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![inbox_id, text_list(source_ids)], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listIdentityColumnsByInboxAndSourceIds`: `(id, sourceId, lastIncomingMessageAt,
/// createdAt)` rows of the inbox ([`BySourceIdRow`] shape, unfiltered).
pub fn list_identity_columns_by_inbox_and_source_ids(
    conn: &Connection,
    inbox_id: i64,
    source_ids: &[String],
) -> Result<Vec<BySourceIdRow>> {
    if source_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"ContactInbox\" WHERE \"inboxId\" = ",
        ContactInbox::columns_sql(None)
    ));
    q.push_bind(inbox_id);
    q.push(" AND \"sourceId\" IN (SELECT value FROM json_each(");
    q.push_bind(text_list(source_ids));
    q.push(")) ORDER BY \"id\"");
    let rows: Vec<ContactInbox> = q.models(conn)?;
    Ok(rows
        .into_iter()
        .map(|ci| BySourceIdRow {
            id: ci.id,
            source_id: ci.source_id,
            last_incoming_message_at: ci.last_incoming_message_at,
            created_at: ci.created_at,
        })
        .collect())
}

/// A contact inbox with the contact's conversation (`findByIdWithConversation`).
#[derive(Debug, Clone, PartialEq)]
pub struct WithConversation {
    pub contact_inbox: ContactInbox,
    pub conversation: Option<Conversation>,
}

/// `findByIdWithConversation` (not workspace-scoped, as the TS).
pub fn find_by_id_with_conversation(
    conn: &Connection,
    id: i64,
) -> Result<Option<WithConversation>> {
    let Some(contact_inbox) = by_id::<ContactInbox>(conn, "id", id)? else {
        return Ok(None);
    };
    Ok(Some(WithConversation {
        conversation: by_id(conn, "contactId", contact_inbox.contact_id)?,
        contact_inbox,
    }))
}

/// `listByInboxPage`: one keyset page of the inbox's rows (`id > after_id`), by id.
pub fn list_by_inbox_page(
    conn: &Connection,
    inbox_id: i64,
    after_id: Option<i64>,
    limit: i64,
) -> Result<Vec<ContactInbox>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"ContactInbox\" WHERE \"inboxId\" = ",
        ContactInbox::columns_sql(None)
    ));
    q.push_bind(inbox_id);
    if let Some(after) = after_id {
        q.push(" AND \"id\" > ");
        q.push_bind(after);
    }
    q.push(" ORDER BY \"id\" ASC LIMIT ");
    q.push_bind(limit);
    q.models(conn)
}

/// `listContactIdsByIds`: the `contactId` of each row of `ids`.
pub fn list_contact_ids_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"contactId\" FROM \"ContactInbox\" WHERE \"id\" IN (SELECT value FROM json_each(?1)) ORDER BY \"id\"",
    )?;
    let rows = stmt
        .query_map([id_list(ids)], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

/// `listByContactId`: the contact's rows, by id (full rows; the TS left out
/// `sourceIdentityHistory`).
pub fn list_by_contact_id(conn: &Connection, contact_id: i64) -> Result<Vec<ContactInbox>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"ContactInbox\" WHERE \"contactId\" = ",
        ContactInbox::columns_sql(None)
    ));
    q.push_bind(contact_id);
    q.push(" ORDER BY \"id\"");
    q.models(conn)
}

/// `BulkAiContactInboxRow`: the contact inbox with its direct-message conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct BulkAiContactInboxRow {
    pub contact_inbox: ContactInbox,
    pub assigned_inbox_team_id: Option<i64>,
    pub assigned_user_id: Option<i64>,
    pub conversation_id: i64,
}

/// Input of [`list_bulk_ai_page`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkAiPageInput {
    pub eligibility: BulkEligibilityInput,
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub after_id: Option<i64>,
    pub limit: i64,
}

/// `bulkConversationJoin`: the contact's direct-message conversation of the workspace.
fn bulk_from(select: &str, workspace_id: i64) -> Sql {
    Sql::bind(
        format!(
            "SELECT {select} FROM \"ContactInbox\" INNER JOIN \"Conversation\" AS bcv ON \
             bcv.\"contactId\" = \"ContactInbox\".\"contactId\" AND bcv.\"sourceId\" IS NULL AND \
             bcv.\"workspaceId\" = "
        ),
        workspace_id,
    )
}

/// `listBulkAiPage`: one keyset page (by contact-inbox id) of the eligible threads of an
/// inbox, each joined to the contact's direct-message conversation of the workspace.
pub fn list_bulk_ai_page(
    conn: &Connection,
    input: &BulkAiPageInput,
) -> Result<Vec<BulkAiContactInboxRow>> {
    let select = format!(
        "{}, bcv.\"assignedInboxTeamId\", bcv.\"assignedUserId\", bcv.\"id\"",
        ContactInbox::columns_sql(Some("ContactInbox"))
    );
    let mut q = bulk_from(&select, input.workspace_id);
    q.push(" WHERE ");
    q.append(bulk_eligibility_where(
        input.inbox_id,
        &input.eligibility,
        input.after_id,
    ));
    q.push(" ORDER BY \"ContactInbox\".\"id\" LIMIT ");
    q.push_bind(input.limit);
    let n = ContactInbox::COLUMNS.len();
    q.query_map(conn, |r| {
        Ok(BulkAiContactInboxRow {
            contact_inbox: ContactInbox::from_row(r)?,
            assigned_inbox_team_id: r.get(n)?,
            assigned_user_id: r.get(n + 1)?,
            conversation_id: r.get(n + 2)?,
        })
    })
}

/// `listStillBulkAiEligible`: of `ids`, those still eligible (empty `ids`: no query), by id.
pub fn list_still_bulk_ai_eligible(
    conn: &Connection,
    input: &BulkEligibilityInput,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut q = bulk_from("\"ContactInbox\".\"id\"", workspace_id);
    q.push(" WHERE ");
    let mut parts = vec![Some(
        Sql::bind(
            "\"ContactInbox\".\"id\" IN (SELECT value FROM json_each(",
            id_list(ids),
        )
        .closed("))"),
    )];
    parts.extend(bulk_eligibility_conditions(input).into_iter().map(Some));
    q.append(Sql::and(parts));
    q.push(" ORDER BY \"ContactInbox\".\"id\"");
    q.query_map(conn, |r| r.get(0))
}

/// `countBulkAiEligible`: eligible threads of one inbox after `after_id` (all when `None`).
pub fn count_bulk_ai_eligible(
    conn: &Connection,
    input: &BulkEligibilityInput,
    workspace_id: i64,
    inbox_id: i64,
    after_id: Option<i64>,
) -> Result<i64> {
    let mut q = bulk_from("count(*)", workspace_id);
    q.push(" WHERE ");
    q.append(bulk_eligibility_where(inbox_id, input, after_id));
    Ok(q.query_map(conn, |r| r.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}
