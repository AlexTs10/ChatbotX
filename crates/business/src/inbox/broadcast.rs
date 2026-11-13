//! `inboxService.resolveBroadcastInboxIds` (`packages/business/src/inbox/service.ts`):
//! the inbox ids an audience-style key means. Strategies in order: an
//! explicit `inboxIds` list (narrowed to the workspace and the channels),
//! the legacy WhatsApp / Messenger integration columns, then the channels.
//! Private queries on `"Inbox"`, `"IntegrationWhatsapp"`, `"IntegrationMessenger"`.

use chatbotx_db::repo;
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::rusqlite::types::Value;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params, params_from_iter};

/// `BroadcastInboxResolutionInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BroadcastInboxResolutionInput {
    pub workspace_id: i64,
    pub channels: Option<Vec<String>>,
    pub inbox_ids: Option<Vec<i64>>,
    pub integration_whatsapp_id: Option<i64>,
    pub integration_messenger_id: Option<i64>,
}

/// `resolveBroadcastInboxIds`.
pub fn resolve_broadcast_inbox_ids(
    conn: &Connection,
    input: &BroadcastInboxResolutionInput,
) -> repo::Result<Vec<i64>> {
    if let Some(ids) = &input.inbox_ids {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut q = Sql::bind(
            "SELECT \"id\" FROM \"Inbox\" WHERE \"workspaceId\" = ",
            input.workspace_id,
        );
        q.push(" AND ");
        q.append(id_list("\"id\"", ids));
        push_channel_where(&mut q, input.channels.as_deref());
        return ids_of(conn, &q);
    }
    for (table, id) in [
        ("IntegrationWhatsapp", input.integration_whatsapp_id),
        ("IntegrationMessenger", input.integration_messenger_id),
    ] {
        if let Some(id) = id {
            let sql = format!(
                "SELECT \"inboxId\" FROM \"{table}\" WHERE \"id\" = ?1 AND \"workspaceId\" = ?2 LIMIT 1"
            );
            let found: Option<i64> = conn
                .query_row(&sql, params![id, input.workspace_id], |r| r.get(0))
                .optional()?;
            return Ok(found.into_iter().collect());
        }
    }
    if input.channels.as_ref().is_none_or(Vec::is_empty) {
        return Ok(Vec::new());
    }
    let mut q = Sql::bind(
        "SELECT \"id\" FROM \"Inbox\" WHERE \"workspaceId\" = ",
        input.workspace_id,
    );
    push_channel_where(&mut q, input.channels.as_deref());
    ids_of(conn, &q)
}

/// `buildBroadcastChannelWhere`: no narrowing without channels or with
/// `omnichannel`.
fn push_channel_where(q: &mut Sql, channels: Option<&[String]>) {
    let mut distinct: Vec<&String> = Vec::new();
    for channel in channels.unwrap_or_default() {
        if !distinct.contains(&channel) {
            distinct.push(channel);
        }
    }
    if distinct.is_empty() || distinct.iter().any(|c| c.as_str() == "omnichannel") {
        return;
    }
    let marks = vec!["?"; distinct.len()].join(", ");
    q.push(" AND ");
    q.append(Sql::new(
        format!("\"channel\" IN ({marks})"),
        distinct
            .into_iter()
            .map(|c| Value::Text(c.clone()))
            .collect(),
    ));
}

fn id_list(column: &str, ids: &[i64]) -> Sql {
    let marks = vec!["?"; ids.len()].join(", ");
    Sql::new(
        format!("{column} IN ({marks})"),
        ids.iter().copied().map(Value::Integer).collect(),
    )
}

fn ids_of(conn: &Connection, q: &Sql) -> repo::Result<Vec<i64>> {
    let mut stmt = conn.prepare(&q.text)?;
    let rows = stmt.query_map(params_from_iter(q.params.iter()), |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
