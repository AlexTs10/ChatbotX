//! `flowAnalyticsService` reads and reset (`flowStatsRepository`) and the
//! smart-delay node counts (`smartDelayService.countByFlowStep` +
//! `buildSmartDelayNodeStats`).

use std::collections::HashMap;

use chatbotx_db::repo;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::Value;

/// `getFlowStatsResponse` of one send-message node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeStats {
    pub sent: i64,
    pub seen: i64,
    pub delivered: i64,
    pub clicked: i64,
    pub failed: i64,
    /// `(buttonId, distinct contact inboxes that clicked it)`, in button
    /// order (quick replies first), then clicked ids not on the node.
    pub buttons: Vec<(String, i64)>,
}

fn id_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The open (`deletedAt IS NULL`) analytics session of a flow.
fn active_session(conn: &Connection, workspace_id: i64, flow_id: i64) -> repo::Result<Option<i64>> {
    Ok(conn
        .query_row(
            r#"SELECT "id" FROM "FlowAnalyticsSession"
               WHERE "workspaceId" = ?1 AND "flowId" = ?2 AND "deletedAt" IS NULL LIMIT 1"#,
            [workspace_id, flow_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// `getFlowStats`: per send-message node of the published version, the
/// counts of the open session (`message:sent` = delivered + failed, `seen` =
/// delivered rows whose contact read a conversation since). Empty without a
/// session, a published version or send-message nodes.
pub fn flow_stats(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
) -> repo::Result<Vec<(String, NodeStats)>> {
    let Some(session) = active_session(conn, workspace_id, flow_id)? else {
        return Ok(Vec::new());
    };
    let Some(nodes) = repo::flow::find_current_version_nodes(conn, workspace_id, flow_id)? else {
        return Ok(Vec::new());
    };
    let mut out: Vec<(String, NodeStats)> = Vec::new();
    for node in nodes.iter().filter(|n| n["type"] == "sendMessage") {
        let details = &node["data"]["details"];
        let quick: Vec<&Value> = details["quickReplies"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        let mut buttons = Vec::new();
        // The last step with buttons wins (`stepButtonMap.set` per step).
        for step in details["steps"].as_array().into_iter().flatten() {
            let ids: Vec<String> = quick
                .iter()
                .copied()
                .chain(step["buttons"].as_array().into_iter().flatten())
                .map(|b| id_text(&b["id"]))
                .collect();
            if !ids.is_empty() {
                buttons = ids.into_iter().map(|id| (id, 0)).collect();
            }
        }
        out.push((
            id_text(&node["id"]),
            NodeStats {
                buttons,
                ..NodeStats::default()
            },
        ));
    }
    if out.is_empty() {
        return Ok(out);
    }
    let node_ids =
        Value::from(out.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>()).to_string();
    let in_nodes = r#""nodeId" IN (SELECT value FROM json_each(?3))"#;
    let index: HashMap<String, usize> = out
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (id.clone(), i))
        .collect();

    let mut stmt = conn.prepare(&format!(
        r#"SELECT "nodeId", "eventType", COUNT(*) FROM "FlowNodeStat"
           WHERE "workspaceId" = ?1 AND "analyticsId" = ?2 AND {in_nodes}
             AND "eventType" IN ('message:delivered', 'message:failed', 'flow:clicked')
           GROUP BY 1, 2"#
    ))?;
    let rows = stmt
        .query_map(params![workspace_id, session, node_ids], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (node, kind, total) in rows {
        let Some(&i) = index.get(&node) else { continue };
        let stats = &mut out[i].1;
        match kind.as_str() {
            "message:delivered" => stats.delivered = total,
            "message:failed" => stats.failed = total,
            _ => stats.clicked = total,
        }
    }

    let mut stmt = conn.prepare(&format!(
        r#"SELECT t."nodeId", COUNT(*) FROM "FlowNodeStat" t
           JOIN "Conversation" c ON c."contactId" = t."contactId"
           WHERE t."workspaceId" = ?1 AND t."analyticsId" = ?2 AND t.{in_nodes}
             AND t."eventType" = 'message:delivered' AND c."contactLastReadAt" >= t."occurredAt"
           GROUP BY 1"#
    ))?;
    let rows = stmt
        .query_map(params![workspace_id, session, node_ids], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (node, seen) in rows {
        if let Some(&i) = index.get(&node) {
            out[i].1.seen = seen;
        }
    }

    let mut stmt = conn.prepare(&format!(
        r#"SELECT "nodeId", "buttonId", COUNT(DISTINCT "contactInboxId") FROM "FlowNodeStat"
           WHERE "workspaceId" = ?1 AND "analyticsId" = ?2 AND {in_nodes} AND "flowId" = ?4
             AND "eventType" = 'flow:clicked' AND "buttonId" IS NOT NULL AND "buttonId" != ''
           GROUP BY 1, 2"#
    ))?;
    let rows = stmt
        .query_map(params![workspace_id, session, node_ids, flow_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (node, button, clicks) in rows {
        let Some(&i) = index.get(&node) else { continue };
        let buttons = &mut out[i].1.buttons;
        match buttons.iter_mut().find(|(id, _)| *id == button) {
            Some(entry) => entry.1 = clicks,
            None => buttons.push((button, clicks)),
        }
    }
    for (_, stats) in &mut out {
        stats.sent = stats.delivered + stats.failed;
    }
    Ok(out)
}

/// `resetStatsSession`: closes the flow's sessions and opens a new one; an
/// unknown or foreign flow is a no-op.
pub fn reset_stats_session(
    tx: &Connection,
    workspace_id: i64,
    flow_id: i64,
    now_ms: i64,
    new_id: i64,
) -> repo::Result<()> {
    let exists = tx
        .query_row(
            r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            [flow_id, workspace_id],
            |_| Ok(()),
        )
        .optional()?;
    if exists.is_none() {
        return Ok(());
    }
    tx.execute(
        r#"UPDATE "FlowAnalyticsSession" SET "deletedAt" = ?1 WHERE "workspaceId" = ?2 AND "flowId" = ?3"#,
        params![now_ms, workspace_id, flow_id],
    )?;
    tx.execute(
        r#"INSERT INTO "FlowAnalyticsSession" ("id", "flowId", "workspaceId") VALUES (?1, ?2, ?3)"#,
        params![new_id, flow_id, workspace_id],
    )?;
    Ok(())
}

/// `{ waiting, sent }` of one wait / follow-up node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DelayCounts {
    pub waiting: i64,
    pub sent: i64,
}

/// `countByFlowStep` + `buildSmartDelayNodeStats` over the draft `nodes`:
/// every wait / follow-up node (zeros by default), counted through its
/// first step id; pending / scheduled rows wait, completed rows were sent.
pub fn smart_delay_stats(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
    nodes: &[Value],
) -> repo::Result<Vec<(String, DelayCounts)>> {
    let mut out: Vec<(String, DelayCounts)> = Vec::new();
    let mut by_step: HashMap<String, usize> = HashMap::new();
    for node in nodes {
        if !matches!(node["type"].as_str(), Some("wait" | "followUp")) {
            continue;
        }
        if let Some(step) = node["data"]["details"]["steps"][0]["id"].as_str() {
            by_step.insert(step.to_string(), out.len());
        }
        out.push((id_text(&node["id"]), DelayCounts::default()));
    }
    let mut stmt = conn.prepare_cached(
        r#"SELECT "stepId", "status", COUNT(*) FROM "ContactOnSmartDelay"
           WHERE "workspaceId" = ?1 AND "flowId" = ?2 AND "stepId" IS NOT NULL
           GROUP BY 1, 2"#,
    )?;
    let binds = [Sql::Integer(workspace_id), Sql::Integer(flow_id)];
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (step, status, total) in rows {
        let Some(&i) = by_step.get(&step) else {
            continue;
        };
        match status.as_str() {
            "pending" | "scheduled" => out[i].1.waiting += total,
            "completed" => out[i].1.sent += total,
            _ => {}
        }
    }
    Ok(out)
}
