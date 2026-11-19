//! The `flow:clicked` / `flow:ref` listeners (`events/flow/listener.ts`) and
//! the `analytics:dashboard` listener (`events/analytics/listener.ts`) of the
//! `events` worker: the click stamps (broadcast, sequence, flow node, magic
//! link, comment automation), the ref-link stat, and the raw dashboard event
//! rows (the hourly rollups are kept by the table triggers). A failed write
//! returns its batch's entry ids, so the bus retries them.

use std::collections::BTreeMap;
use std::sync::Arc;

use chatbotx_business::analytics::mac::anchored_period;
use chatbotx_business::quota_enforcement::increment_by;
use chatbotx_business::user_quota::QuotaMetric;
use chatbotx_business::workspace_usage::{WorkspaceUsageMetric, adjust};
use chatbotx_db::repo::{new_id, now_ms, parse_iso};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};
use chatbotx_db::{Db, DbError};
use chatbotx_jobs::events::{EventListener, ListenerOutcome, Listeners, event_bus_message_id_key};
use chrono::{DateTime, Datelike, TimeZone as _, Utc};
use serde_json::Value;

/// The bus entry ids of a batch (retried on failure).
pub fn entry_ids(payloads: &[Value]) -> Vec<String> {
    payloads
        .iter()
        .filter_map(|p| p.get(event_bus_message_id_key())?.as_str())
        .map(str::to_owned)
        .collect()
}

/// An id field (string or number).
fn id(value: &Value) -> Option<i64> {
    match value {
        Value::String(s) => s.parse().ok(),
        other => other.as_i64(),
    }
}

/// `toDate(occurredAt)` in ms (ISO text or a number).
fn ms(value: &Value) -> Option<i64> {
    match value {
        Value::String(s) => parse_iso(s),
        other => other.as_f64().map(|n| n as i64),
    }
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn int(value: Option<i64>) -> Sql {
    value.map_or(Sql::Null, Sql::Integer)
}

fn opt_text(value: Option<String>) -> Sql {
    value.map_or(Sql::Null, Sql::Text)
}

fn json_text(value: &Value) -> Sql {
    if value.is_null() {
        Sql::Null
    } else {
        Sql::Text(value.to_string())
    }
}

fn bool_int(value: &Value) -> Sql {
    match value {
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Null => Sql::Null,
        other => int(other.as_i64()),
    }
}

/// `getEventBusEventId(p) ?? createId()` (`eventId` first where the original reads it).
fn event_id(payload: &Value, own_first: bool) -> String {
    let own = own_first
        .then(|| payload["eventId"].as_str().filter(|s| !s.is_empty()))
        .flatten();
    own.or_else(|| {
        payload
            .get(event_bus_message_id_key())
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    })
    .map_or_else(chatbotx_core::utils::id::create_id, str::to_owned)
}

fn insert_rows(
    conn: &Connection,
    table: &str,
    columns: &[&str],
    rows: Vec<Vec<Sql>>,
) -> Result<(), DbError> {
    let names = columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let slots = chatbotx_db::repo::placeholders(1, columns.len());
    let sql = format!("INSERT INTO \"{table}\" ({names}) VALUES ({slots}) ON CONFLICT DO NOTHING");
    let mut stmt = conn.prepare(&sql)?;
    for row in rows {
        stmt.execute(params_from_iter(row))?;
    }
    Ok(())
}

// --- flow:clicked --------------------------------------------------------------

struct Click<'a> {
    workspace_id: i64,
    contact_id: Option<i64>,
    contact_inbox_id: Option<i64>,
    occurred_at: i64,
    node_id: Option<&'a str>,
    action: &'a Value,
}

fn clicks(payloads: &[Value]) -> Vec<Click<'_>> {
    payloads
        .iter()
        .filter_map(|p| {
            Some(Click {
                workspace_id: id(&p["context"]["workspaceId"])?,
                contact_id: id(&p["context"]["contactId"]),
                contact_inbox_id: id(&p["context"]["contactInboxId"]),
                occurred_at: ms(&p["occurredAt"]).unwrap_or_else(now_ms),
                node_id: p["nodeId"].as_str().filter(|n| !n.is_empty()),
                action: &p["action"],
            })
        })
        .collect()
}

/// `startOfSecond`.
fn start_of_second(at: i64) -> i64 {
    at.div_euclid(1000) * 1000
}

/// `broadcastAnalyticsService.onClicked`: the first click per recipient.
pub fn broadcast_clicked(conn: &Connection, payloads: &[Value]) -> Result<(), DbError> {
    for c in clicks(payloads) {
        let (Some(broadcast), Some(inbox)) = (id(&c.action["broadcastId"]), c.contact_inbox_id)
        else {
            continue;
        };
        conn.execute(
            r#"UPDATE "ContactOnBroadcast" SET "clickedAt" = COALESCE("clickedAt", ?3)
               WHERE "broadcastId" = ?1 AND "contactInboxId" = ?2"#,
            params![broadcast, inbox, c.occurred_at],
        )?;
    }
    Ok(())
}

/// `sequenceAnalyticsService.onClicked`: `clickedAt` of the step dispatch.
pub fn sequence_clicked(conn: &Connection, payloads: &[Value]) -> Result<(), DbError> {
    for c in clicks(payloads) {
        let Some(step) = id(&c.action["sequenceStepId"]) else {
            continue;
        };
        let sequence: Option<i64> = conn
            .query_row(
                r#"SELECT "sequenceId" FROM "SequenceStep" WHERE "id" = ?1"#,
                [step],
                |r| r.get(0),
            )
            .optional()?;
        let (Some(sequence), Some(inbox)) = (sequence, c.contact_inbox_id) else {
            continue;
        };
        conn.execute(
            r#"UPDATE "SequenceDispatch" SET "clickedAt" = ?5
               WHERE "workspaceId" = ?1 AND "sequenceId" = ?2 AND "stepId" = ?3 AND "contactInboxId" = ?4"#,
            params![c.workspace_id, sequence, step, inbox, c.occurred_at],
        )?;
    }
    Ok(())
}

/// `flowAnalyticsService.onClicked`: a `FlowNodeStat` row for a button tap
/// of a flow with a live analytics session.
pub fn flow_clicked(conn: &Connection, payloads: &[Value]) -> Result<(), DbError> {
    let mut rows = Vec::new();
    for c in clicks(payloads) {
        let (Some(node), Some(flow)) = (c.node_id, id(&c.action["flowId"])) else {
            continue;
        };
        let Some(button) = c.action["buttonId"].as_str().filter(|b| !b.is_empty()) else {
            continue;
        };
        let analytics: Option<i64> = conn
            .query_row(
                r#"SELECT "id" FROM "FlowAnalyticsSession" WHERE "flowId" = ?1 AND "deletedAt" IS NULL"#,
                [flow],
                |r| r.get(0),
            )
            .optional()?;
        let Some(analytics) = analytics else {
            continue;
        };
        rows.push(vec![
            Sql::Integer(new_id()),
            Sql::Integer(c.workspace_id),
            Sql::Integer(flow),
            Sql::Integer(analytics),
            Sql::Text(node.to_owned()),
            Sql::Text(button.to_owned()),
            int(c.contact_id),
            int(c.contact_inbox_id),
            Sql::Text("flow:clicked".into()),
            Sql::Integer(c.occurred_at),
        ]);
    }
    insert_rows(
        conn,
        "FlowNodeStat",
        &[
            "id",
            "workspaceId",
            "flowId",
            "analyticsId",
            "nodeId",
            "buttonId",
            "contactId",
            "contactInboxId",
            "eventType",
            "occurredAt",
        ],
        rows,
    )
}

fn link_stat_rows(clicks: Vec<(i64, i64, Click<'_>)>) -> Vec<Vec<Sql>> {
    clicks
        .into_iter()
        .map(|(link, inbox, c)| {
            vec![
                Sql::Integer(c.workspace_id),
                Sql::Integer(link),
                int(c.contact_id),
                Sql::Integer(inbox),
                Sql::Integer(start_of_second(c.occurred_at)),
            ]
        })
        .collect()
}

const LINK_STAT_COLUMNS: &[&str] = &[
    "workspaceId",
    "linkId",
    "contactId",
    "contactInboxId",
    "occurredAt",
];

/// `magicLinkAnalyticsService.onClicked`: one stat per magic-link click.
pub fn magic_link_clicked(conn: &Connection, payloads: &[Value]) -> Result<(), DbError> {
    let items = clicks(payloads)
        .into_iter()
        .filter(|c| c.action["clickType"] == "magic_link")
        .filter_map(|c| Some((id(&c.action["magicLinkId"])?, c.contact_inbox_id?, c)))
        .collect();
    insert_rows(
        conn,
        "MagicLinkStat",
        LINK_STAT_COLUMNS,
        link_stat_rows(items),
    )
}

/// `refLinkAnalyticsService.handler` (`flow:ref`).
pub fn ref_link_opened(conn: &Connection, payloads: &[Value]) -> Result<(), DbError> {
    let items = clicks(payloads)
        .into_iter()
        .filter_map(|c| Some((id(&c.action["refId"])?, c.contact_inbox_id?, c)))
        .collect();
    insert_rows(
        conn,
        "RefLinkStat",
        LINK_STAT_COLUMNS,
        link_stat_rows(items),
    )
}

/// `commentAutomationAnalyticsService.onClicked`: stamps the latest unclicked
/// private reply of the automation to the contact, and counts it once.
pub fn comment_automation_clicked(conn: &Connection, payloads: &[Value]) -> Result<(), DbError> {
    for c in clicks(payloads) {
        let (Some(automation), Some(inbox)) =
            (id(&c.action["commentAutomationId"]), c.contact_inbox_id)
        else {
            tracing::warn!(
                flow_id = %c.action["flowId"],
                "[analytics:commentAutomation] click dropped before lookup — payload carries no attribution"
            );
            continue;
        };
        let marked = conn.execute(
            r#"UPDATE "CommentAutomationEvent" SET "clickedAt" = ?3, "updatedAt" = ?4
               WHERE "id" = (SELECT "id" FROM "CommentAutomationEvent"
                             WHERE "automationId" = ?1 AND "contactInboxId" = ?2
                               AND "replyChannel" = 'private' AND "clickedAt" IS NULL
                             ORDER BY "occurredAt" DESC LIMIT 1)
                 AND "clickedAt" IS NULL"#,
            params![automation, inbox, c.occurred_at, now_ms()],
        )?;
        if marked == 0 {
            tracing::warn!(
                automation,
                inbox,
                "[analytics:commentAutomation] click matched no event row — nothing counted"
            );
            continue;
        }
        conn.execute(
            r#"UPDATE "CommentAutomation" SET "clickedCount" = MAX(0, "clickedCount" + 1) WHERE "id" = ?1"#,
            [automation],
        )?;
    }
    Ok(())
}

/// The `flow:clicked` listeners, by name (`flowListeners`).
pub const FLOW_CLICKED_LISTENERS: &[(&str, fn(&Connection, &[Value]) -> Result<(), DbError>)] = &[
    ("broadcast-ops", broadcast_clicked),
    ("sequence-ops", sequence_clicked),
    ("flow-ops", flow_clicked),
    ("magic-link-stats", magic_link_clicked),
    ("comment-automation-ops", comment_automation_clicked),
];

// --- analytics:dashboard -------------------------------------------------------

fn occurred(p: &Value) -> Sql {
    Sql::Integer(ms(&p["occurredAt"]).unwrap_or_else(now_ms))
}

fn contact_rows(payloads: &[&Value], event_type: &str) -> Vec<Vec<Sql>> {
    payloads
        .iter()
        .map(|p| {
            vec![
                Sql::Text(event_id(p, false)),
                int(id(&p["workspaceId"])),
                int(id(&p["contactId"])),
                Sql::Text(event_type.to_owned()),
                occurred(p),
                opt_text(text(&p["source"])),
                opt_text(text(&p["sourceId"])),
                opt_text(text(&p["channel"])),
                int(id(&p["inboxId"])),
                opt_text(text(&p["language"])),
                opt_text(text(&p["country"])),
                json_text(&p["metadata"]),
            ]
        })
        .collect()
}

fn message_rows(payloads: &[&Value], event_type: &str) -> Vec<Vec<Sql>> {
    payloads
        .iter()
        .map(|p| {
            vec![
                Sql::Text(event_id(p, true)),
                int(id(&p["workspaceId"])),
                int(id(&p["contactId"])),
                Sql::Text(event_type.to_owned()),
                occurred(p),
                opt_text(text(&p["senderType"])),
                int(id(&p["adminId"])),
                opt_text(text(&p["channel"])),
                int(id(&p["inboxId"])),
                opt_text(text(&p["source"])),
                opt_text(text(&p["sourceId"])),
                json_text(&p["metadata"]),
            ]
        })
        .collect()
}

fn conversation_rows(payloads: &[&Value], event_type: &str) -> Vec<Vec<Sql>> {
    payloads
        .iter()
        .map(|p| {
            vec![
                Sql::Text(event_id(p, false)),
                int(id(&p["workspaceId"])),
                int(id(&p["conversationId"])),
                Sql::Text(event_type.to_owned()),
                occurred(p),
                int(id(&p["fromAssignee"])),
                int(id(&p["toAssignee"])),
                opt_text(text(&p["channel"])),
                int(id(&p["inboxId"])),
                json_text(&p["metadata"]),
            ]
        })
        .collect()
}

/// `botMessageStatsRepository.insertEvents`: rows without the message,
/// workspace or conversation are dropped.
fn bot_received_rows(payloads: &[&Value]) -> Vec<Vec<Sql>> {
    payloads
        .iter()
        .filter_map(|p| {
            let (ws, message, conversation) = (
                id(&p["workspaceId"])?,
                id(&p["messageId"])?,
                id(&p["conversationId"])?,
            );
            Some(vec![
                Sql::Text(event_id(p, false)),
                Sql::Integer(ws),
                Sql::Integer(message),
                Sql::Integer(conversation),
                occurred(p),
                Sql::Integer(i64::from(p["hasResponse"] == true)),
                opt_text(text(&p["responseType"])),
                opt_text(text(&p["routeType"])),
                opt_text(text(&p["result"])),
                opt_text(text(&p["aiProvider"])),
                opt_text(text(&p["channel"])),
                int(id(&p["inboxId"])),
                opt_text(text(&p["source"])),
                json_text(&p["metadata"]),
            ])
        })
        .collect()
}

/// `handleAgentReplied`: rows that cannot be a real answered window are dropped.
fn agent_replied_rows(payloads: &[&Value]) -> Vec<Vec<Sql>> {
    payloads
        .iter()
        .filter_map(|p| {
            let (at, awaiting, from) = (
                ms(&p["occurredAt"])?,
                ms(&p["awaitingSince"])?,
                ms(&p["measuredFrom"])?,
            );
            let valid = from < at
                && p["responseSeconds"].is_number()
                && p["isFirstResponse"].is_boolean()
                && id(&p["adminId"]).is_some();
            if !valid {
                tracing::warn!(
                    workspace_id = %p["workspaceId"],
                    conversation_id = %p["conversationId"],
                    "[analytics] dropped invalid agent:replied event"
                );
                return None;
            }
            Some(vec![
                Sql::Text(event_id(p, true)),
                int(id(&p["workspaceId"])),
                int(id(&p["conversationId"])),
                int(id(&p["contactId"])),
                int(id(&p["adminId"])),
                Sql::Integer(at),
                Sql::Integer(awaiting),
                Sql::Integer(from),
                int(p["responseSeconds"].as_f64().map(|n| n as i64)),
                int(p["workingResponseSeconds"].as_f64().map(|n| n as i64)),
                bool_int(&p["isFirstResponse"]),
                int(id(&p["workingHoursId"])),
                int(p["slaTargetSeconds"].as_f64().map(|n| n as i64)),
                bool_int(&p["slaMet"]),
                opt_text(text(&p["channel"])),
                int(id(&p["inboxId"])),
                json_text(&p["metadata"]),
            ])
        })
        .collect()
}

const CONTACT_COLUMNS: &[&str] = &[
    "eventId",
    "workspaceId",
    "contactId",
    "eventType",
    "occurredAt",
    "source",
    "sourceId",
    "channel",
    "inboxId",
    "language",
    "country",
    "metadata",
];
const MESSAGE_COLUMNS: &[&str] = &[
    "eventId",
    "workspaceId",
    "contactId",
    "eventType",
    "occurredAt",
    "senderType",
    "adminId",
    "channel",
    "inboxId",
    "source",
    "sourceId",
    "metadata",
];
const CONVERSATION_COLUMNS: &[&str] = &[
    "eventId",
    "workspaceId",
    "conversationId",
    "eventType",
    "occurredAt",
    "fromAssignee",
    "toAssignee",
    "channel",
    "inboxId",
    "metadata",
];
const BOT_RECEIVED_COLUMNS: &[&str] = &[
    "eventId",
    "workspaceId",
    "messageId",
    "conversationId",
    "occurredAt",
    "hasResponse",
    "responseType",
    "routeType",
    "result",
    "aiProvider",
    "channel",
    "inboxId",
    "source",
    "metadata",
];
const AGENT_REPLIED_COLUMNS: &[&str] = &[
    "eventId",
    "workspaceId",
    "conversationId",
    "contactId",
    "adminId",
    "occurredAt",
    "awaitingSince",
    "measuredFrom",
    "responseSeconds",
    "workingResponseSeconds",
    "isFirstResponse",
    "workingHoursId",
    "slaTargetSeconds",
    "slaMet",
    "channel",
    "inboxId",
    "metadata",
];

/// `calendarPeriod`: the UTC month of `now`.
fn calendar_period(now: i64) -> Option<(i64, i64)> {
    let at = DateTime::from_timestamp_millis(now)?;
    let start = Utc
        .with_ymd_and_hms(at.year(), at.month(), 1, 0, 0, 0)
        .single()?;
    let end = start.checked_add_months(chrono::Months::new(1))?;
    Some((start.timestamp_millis(), end.timestamp_millis()))
}

/// `handleBotMessageSent`'s accounting: the owner quotas, the workspace usage
/// and the period ledger, per workspace; best effort (logged, never retried).
fn count_bot_messages(conn: &Connection, workspace_id: i64, count: i64) {
    let run = || -> chatbotx_db::repo::Result<()> {
        let Some((owner, tenant)) = conn
            .query_row(
                r#"SELECT "ownerId", "tenantId" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .optional()?
        else {
            return Ok(());
        };
        increment_by(conn, owner, QuotaMetric::BotMessages, count)?;
        increment_by(conn, owner, QuotaMetric::MonthlyBotMessages, count)?;
        adjust(conn, workspace_id, WorkspaceUsageMetric::BotMessages, count)?;
        let anchor: Option<i64> = conn
            .query_row(
                r#"SELECT "periodStart" FROM "UserQuota" WHERE "userId" = ?1"#,
                [owner],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let now = now_ms();
        let period = anchor.map_or_else(|| calendar_period(now), |a| anchored_period(now, a));
        if let Some((start, end)) = period {
            conn.execute(
                r#"INSERT INTO "WorkspaceBotMessageUsage" ("id", "workspaceId", "ownerId", "tenantId", "periodStart", "periodEnd", "count")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                   ON CONFLICT ("workspaceId", "periodStart") DO UPDATE SET
                     "count" = "WorkspaceBotMessageUsage"."count" + excluded."count", "updatedAt" = ?8"#,
                params![new_id(), workspace_id, owner, tenant.unwrap_or_default(), start, end, count, now],
            )?;
        }
        Ok(())
    };
    if let Err(err) = run() {
        tracing::error!(err = %err, workspace_id, count, "[analytics] bot message quota increment failed");
    }
}

/// One dashboard sub-handler: its rows, or the bot-message accounting.
fn dashboard_group(
    conn: &Connection,
    event_type: &str,
    payloads: &[&Value],
) -> Result<(), DbError> {
    if let Some(kind) = event_type.strip_prefix("contact:") {
        return insert_rows(
            conn,
            "AnalyticsContactEvent",
            CONTACT_COLUMNS,
            contact_rows(payloads, &format!("contact_{kind}")),
        );
    }
    if let Some(kind) = event_type.strip_prefix("conversation:") {
        return insert_rows(
            conn,
            "AnalyticsConversationEvent",
            CONVERSATION_COLUMNS,
            conversation_rows(payloads, &format!("conversation_{kind}")),
        );
    }
    match event_type {
        "message:human_sent" => insert_rows(
            conn,
            "AnalyticsMessageEvent",
            MESSAGE_COLUMNS,
            message_rows(payloads, "message_human_sent"),
        ),
        "message:bot_sent" => {
            insert_rows(
                conn,
                "AnalyticsMessageEvent",
                MESSAGE_COLUMNS,
                message_rows(payloads, "message_bot_sent"),
            )?;
            let mut counts: BTreeMap<i64, i64> = BTreeMap::new();
            for ws in payloads.iter().filter_map(|p| id(&p["workspaceId"])) {
                *counts.entry(ws).or_default() += 1;
            }
            for (ws, count) in counts {
                count_bot_messages(conn, ws, count);
            }
            Ok(())
        }
        "message:bot_received" => {
            let rows = bot_received_rows(payloads);
            // The original logs a failed bot-message insert and acks the batch.
            if let Err(err) =
                insert_rows(conn, "AnalyticsBotMessageEvent", BOT_RECEIVED_COLUMNS, rows)
            {
                tracing::error!(err = %err, "[bot-message-stats] insertEvents failed");
            }
            Ok(())
        }
        "agent:replied" => insert_rows(
            conn,
            "AnalyticsAgentResponseEvent",
            AGENT_REPLIED_COLUMNS,
            agent_replied_rows(payloads),
        ),
        _ => Ok(()),
    }
}

/// The `analytics-dashboard` listener: each event type in its own write; the
/// entry ids of a failed type are returned for retry.
pub async fn record_dashboard(db: &Db, payloads: Vec<Value>) -> Vec<String> {
    let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for p in payloads {
        if let Some(kind) = p["eventType"].as_str() {
            groups.entry(kind.to_owned()).or_default().push(p);
        }
    }
    let mut failed = Vec::new();
    for (event_type, group) in groups {
        let kind = event_type.clone();
        let rows = group.clone();
        let written = db
            .write(move |conn| {
                let refs: Vec<&Value> = rows.iter().collect();
                dashboard_group(conn, &kind, &refs)
            })
            .await;
        if let Err(err) = written {
            tracing::error!(err = %err, event_type, count = group.len(), "[analytics] dashboard handler failed");
            failed.extend(entry_ids(&group));
        }
    }
    failed
}

/// Runs one click writer; the batch's entry ids on failure.
pub async fn record_clicks(
    db: &Db,
    name: &'static str,
    write: fn(&Connection, &[Value]) -> Result<(), DbError>,
    payloads: Vec<Value>,
) -> Vec<String> {
    let ids = entry_ids(&payloads);
    match db.write(move |conn| write(conn, &payloads)).await {
        Ok(()) => Vec::new(),
        Err(err) => {
            tracing::error!(err = %err, listener = name, "flow event listener failed");
            ids
        }
    }
}

fn click_listener(
    db: &Arc<Db>,
    name: &'static str,
    write: fn(&Connection, &[Value]) -> Result<(), DbError>,
) -> EventListener {
    let db = Arc::clone(db);
    EventListener {
        name: name.to_owned(),
        handler: Arc::new(move |payloads, _| {
            let db = Arc::clone(&db);
            Box::pin(async move {
                let failed_message_ids = record_clicks(&db, name, write, payloads).await;
                Ok(ListenerOutcome { failed_message_ids })
            })
        }),
    }
}

/// `flowListeners`: `flow:clicked` and `flow:ref`.
pub fn flow_listeners(db: &Arc<Db>) -> Listeners {
    let mut listeners = Listeners::new();
    listeners.insert(
        "flow:clicked".to_owned(),
        FLOW_CLICKED_LISTENERS
            .iter()
            .map(|(name, write)| click_listener(db, name, *write))
            .collect(),
    );
    listeners.insert(
        "flow:ref".to_owned(),
        vec![click_listener(db, "ref-link-stats", ref_link_opened)],
    );
    listeners
}

/// `analyticsDashboardListeners`.
pub fn dashboard_listeners(db: &Arc<Db>) -> Listeners {
    let db = Arc::clone(db);
    let mut listeners = Listeners::new();
    listeners.insert(
        "analytics:dashboard".to_owned(),
        vec![EventListener {
            name: "analytics-dashboard".to_owned(),
            handler: Arc::new(move |payloads, _| {
                let db = Arc::clone(&db);
                Box::pin(async move {
                    let failed_message_ids = record_dashboard(&db, payloads).await;
                    Ok(ListenerOutcome { failed_message_ids })
                })
            }),
        }],
    );
    listeners
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ids_and_times_accept_text_and_numbers() {
        assert_eq!(id(&json!("12")), Some(12));
        assert_eq!(id(&json!(12)), Some(12));
        assert_eq!(id(&json!(null)), None);
        assert_eq!(
            ms(&json!("2026-10-01T00:00:00.500Z")),
            Some(1_790_812_800_500)
        );
        assert_eq!(start_of_second(1_790_812_800_500), 1_790_812_800_000);
    }

    #[test]
    fn the_calendar_period_is_the_utc_month() {
        let (start, end) = calendar_period(1_790_812_800_500).expect("period");
        assert_eq!(start, 1_790_812_800_000);
        assert_eq!(end, 1_793_491_200_000);
    }

    #[test]
    fn agent_replied_rows_drop_an_empty_window() {
        let ok = json!({ "workspaceId": "1", "conversationId": "2", "contactId": "3", "adminId": "4",
            "occurredAt": "2026-10-01T00:01:00Z", "awaitingSince": "2026-10-01T00:00:00Z",
            "measuredFrom": "2026-10-01T00:00:00Z", "responseSeconds": 60, "isFirstResponse": true });
        let mut empty = ok.clone();
        empty["measuredFrom"] = json!("2026-10-01T00:01:00Z");
        assert_eq!(agent_replied_rows(&[&ok, &empty]).len(), 1);
    }

    #[test]
    fn bot_received_rows_need_the_message_and_conversation() {
        let ok = json!({ "workspaceId": "1", "messageId": "2", "conversationId": "3", "hasResponse": true });
        let missing = json!({ "workspaceId": "1", "messageId": "2" });
        assert_eq!(bot_received_rows(&[&ok, &missing]).len(), 1);
    }
}
