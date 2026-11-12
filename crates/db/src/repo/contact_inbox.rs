//! Port of `repositories/contact-inbox/repository.ts` (identity, attribution, ad
//! eligibility and thread-control writes; the bulk-AI eligibility and the relational
//! finders are listed as not ported yet in `conversion/specs/P3.2.md`).

use rusqlite::Connection;
use rusqlite::types::Value;
use serde_json::{Value as J, json};

use super::models::ContactInbox;
use super::partials::thread_control::{events_outranked_by, transition};
use super::queries::ad_referral::ad_conversation_predicate;
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
