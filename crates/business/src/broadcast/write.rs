//! `create`, `updateDraft`, `scheduleDraft`, `cloneBroadcast` and
//! `resendWithPruning` of `broadcast/service.ts`.
//!
//! A payload is the validated request as JSON (camelCase keys, ids as
//! numeric strings), like the `UpdateDraftBroadcastData` of the original.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chatbotx_core::flow_config::steps::find_payment_request_mismatches;
use chatbotx_core::flow_config::steps::find_template_start_step;
use chatbotx_core::flow_config::steps::wa_template_params_schema;
use chatbotx_db::Db;
use chatbotx_db::repo::models::Broadcast;
use chatbotx_db::repo::partials::broadcast as partials;
use chatbotx_db::repo::queries::contact_filter::FilterCriteria;
use chatbotx_db::repo::queries::contact_filter::permission::prune_email_phone_filter_conditions;
use chatbotx_db::repo::{self, broadcast_recurrence as recurrence_repo, new_id, now_ms};
use chatbotx_db::rusqlite::types::Value as SqlValue;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName, Retention};
use serde_json::{Value, json};

use super::{
    activate_recurring_template, chatbotx_exception, guarded_update, is_recurring_template,
    lock_series, plan_policy,
};
use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::tx;

const DEFAULT_BROADCAST_NAME: &str = "Broadcast";
const BROADCAST_NAME_SEPARATOR: &str = " - ";
const BROADCAST_NAME_TARGET_SEPARATOR: &str = " / ";
const BROADCAST_NAME_MAX_LENGTH: usize = 255;
const APPROVED_TEMPLATE_STATUS: &str = "APPROVED";
const BROADCAST_COPY_LABEL: &str = " (Copy ";

/// `BroadcastValidationException(message, field)` (`validation`, 422).
fn invalid(message: &str, field: &str) -> BusinessError {
    BusinessError::validation(field, message)
}

/// A numeric-string (or number) id of the payload.
fn id_of(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

fn targets(data: &Value) -> &[Value] {
    data.get("targets")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn str_of<'a>(data: &'a Value, key: &str) -> Option<&'a str> {
    data.get(key).and_then(Value::as_str)
}

/// `assertDraftPayload`: the first violated channel / subaction / send rule.
fn assert_draft_payload(data: &Value) -> Result<()> {
    let channel = str_of(data, "channel").unwrap_or_default();
    let capability = partials::find_broadcast_channel_capability(channel);
    let sends_flow = partials::broadcast_sends_flow(data);
    let sends_template = partials::broadcast_sends_template(data);
    let subaction = str_of(data, "subaction").unwrap_or_default();
    let recurring = str_of(data, "schedulesType") == Some("recurring");
    let has_recurrence = data.get("recurrence").is_some_and(|r| !r.is_null());
    let rules: [(bool, &str, &str); 11] = [
        (
            capability.is_none(),
            "Unsupported broadcast channel",
            "channel",
        ),
        (
            !capability.is_some_and(|c| c.subactions.contains(&subaction)),
            "Unsupported broadcast subaction",
            "subaction",
        ),
        (
            !(sends_flow || sends_template),
            "Either flow or template must be selected",
            "flowId",
        ),
        (
            partials::has_flow_and_template(data),
            "A broadcast sends either a flow or a template, not both",
            "flowId",
        ),
        (
            sends_template && !capability.is_some_and(|c| c.supports_template_broadcast),
            "Template broadcasts are not supported for this channel",
            "templateId",
        ),
        (
            partials::is_targets_template_send_without_template(data),
            "Select a template for at least one page",
            "targets",
        ),
        (
            partials::is_targets_flow_send_without_flow(data),
            "Select a flow for at least one page",
            "targets",
        ),
        (
            partials::has_duplicate_broadcast_target(data),
            "A page can only be selected once",
            "targets",
        ),
        (
            partials::is_template_send_without_page(data),
            "Select the page the template belongs to",
            "targets",
        ),
        (
            recurring != has_recurrence,
            "Repeat settings are required for a repeating broadcast, and only for one",
            "recurrence",
        ),
        (
            !partials::is_audience_range_ordered(
                data.get("audienceRangeStart").and_then(Value::as_i64),
                data.get("audienceRangeEnd").and_then(Value::as_i64),
            ),
            "The end position must not be before the start position",
            "audienceRangeEnd",
        ),
    ];
    match rules.iter().find(|(violated, ..)| *violated) {
        Some((_, message, field)) => Err(invalid(message, field)),
        None => Ok(()),
    }
}

/// `resolveBroadcastTargetsToPersist`: a non-draft drops pages with nothing to send.
fn resolve_targets_to_persist(data: &Value) -> Result<Value> {
    let saved_as_draft = data.get("saveAsDraft").and_then(Value::as_bool) == Some(true);
    if targets(data).is_empty() || saved_as_draft {
        return Ok(data.clone());
    }
    let ready: Vec<Value> = targets(data)
        .iter()
        .filter(|t| id_of(t.get("templateId")).is_some() || id_of(t.get("flowId")).is_some())
        .cloned()
        .collect();
    if ready.is_empty() {
        return Err(invalid(
            "Select a template or flow for at least one page",
            "targets",
        ));
    }
    let mut data = data.clone();
    data["targets"] = Value::Array(ready);
    Ok(data)
}

/// One template of a broadcast (`BroadcastTemplateDetail`).
#[derive(Debug, Clone)]
struct TemplateDetail {
    id: i64,
    name: String,
    status: String,
    components: Value,
    inbox_id: i64,
    integration_name: Option<String>,
    language: String,
    category: String,
    /// Messenger only.
    parameter_format: Option<String>,
}

/// `BroadcastTemplateSelection`.
#[derive(Debug, Clone)]
struct Selection {
    template_id: i64,
    inbox_id: Option<i64>,
    integration_whatsapp_id: Option<i64>,
    integration_messenger_id: Option<i64>,
    template_data: Option<Value>,
}

/// `broadcastTemplateSelections`: one per target template, else the legacy one.
fn template_selections(data: &Value) -> Vec<Selection> {
    let from_targets: Vec<Selection> = targets(data)
        .iter()
        .filter_map(|t| {
            Some(Selection {
                template_id: id_of(t.get("templateId"))?,
                inbox_id: id_of(t.get("inboxId")),
                integration_whatsapp_id: None,
                integration_messenger_id: None,
                template_data: t.get("templateData").cloned(),
            })
        })
        .collect();
    let legacy = id_of(data.get("templateId"));
    if !from_targets.is_empty() || legacy.is_none() {
        return from_targets;
    }
    vec![Selection {
        template_id: legacy.unwrap_or_default(),
        inbox_id: None,
        integration_whatsapp_id: id_of(data.get("integrationWhatsappId")),
        integration_messenger_id: id_of(data.get("integrationMessengerId")),
        template_data: data.get("templateData").cloned(),
    }]
}

/// `loadWhatsappTemplateDetail` / `loadMessengerTemplateDetail`.
fn load_template_details(
    conn: &Connection,
    channel: &str,
    workspace_id: i64,
    template_ids: &[i64],
    integration_id: Option<i64>,
) -> Result<Vec<TemplateDetail>> {
    let (template, integration, fk, format) = match channel {
        "whatsapp" => (
            "WhatsappMessageTemplate",
            "IntegrationWhatsapp",
            "integrationWhatsappId",
            "NULL",
        ),
        "messenger" => (
            "MessengerMessageTemplate",
            "IntegrationMessenger",
            "integrationMessengerId",
            r#"t."parameterFormat""#,
        ),
        _ => return Ok(Vec::new()),
    };
    if template_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        r#"SELECT t."id", t."name", t."status", t."components", i."inboxId", i."name", t."language", t."category", {format}
           FROM "{template}" AS t INNER JOIN "{integration}" AS i ON i."id" = t."{fk}"
           WHERE t."id" IN (SELECT value FROM json_each(?1)) AND i."workspaceId" = ?2
             AND (?3 IS NULL OR t."{fk}" = ?3)
           LIMIT ?4"#
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            params![
                json!(template_ids).to_string(),
                workspace_id,
                integration_id,
                template_ids.len() as i64
            ],
            |row| {
                Ok(TemplateDetail {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    status: row.get(2)?,
                    components: serde_json::from_str(&row.get::<_, String>(3)?)
                        .unwrap_or(Value::Null),
                    inbox_id: row.get(4)?,
                    integration_name: row.get(5)?,
                    language: row.get(6)?,
                    category: row.get(7)?,
                    parameter_format: row.get(8)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()
        .map_err(repo::Error::from)?;
    Ok(rows)
}

/// `resolveSelectedTemplates`: one detail per selection that matches its page.
fn resolve_selected_templates(
    conn: &Connection,
    workspace_id: i64,
    channel: &str,
    selections: &[Selection],
) -> Result<Vec<TemplateDetail>> {
    let Some(first) = selections.first() else {
        return Ok(Vec::new());
    };
    let mut ids: Vec<i64> = selections.iter().map(|s| s.template_id).collect();
    ids.sort_unstable();
    ids.dedup();
    let integration = match channel {
        "whatsapp" => first.integration_whatsapp_id,
        _ => first.integration_messenger_id,
    };
    let details = load_template_details(conn, channel, workspace_id, &ids, integration)?;
    Ok(selections
        .iter()
        .filter_map(|s| {
            details
                .iter()
                .find(|d| {
                    d.id == s.template_id && s.inbox_id.is_none_or(|inbox| inbox == d.inbox_id)
                })
                .cloned()
        })
        .collect())
}

/// `listTemplateDetails`: the templates a broadcast sends, one per page.
pub async fn list_template_details(
    db: &Db,
    workspace_id: i64,
    broadcast_id: i64,
) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let Some(broadcast) = repo::find_one::<Broadcast>(
            conn,
            r#""id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
            &[&broadcast_id, &workspace_id],
        )?
        else {
            return Ok(Vec::new());
        };
        let mut stmt = conn.prepare(
            r#"SELECT "inboxId", "templateId" FROM "BroadcastTarget" WHERE "broadcastId" = ?1"#,
        )?;
        let targets = stmt
            .query_map([broadcast_id], |row| {
                Ok(json!({
                    "inboxId": row.get::<_, i64>(0)?.to_string(),
                    "templateId": row.get::<_, Option<i64>>(1)?.map(|id| id.to_string()),
                }))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo::Error::from)?;
        let data = json!({
            "templateId": broadcast.template_id.map(|id| id.to_string()),
            "integrationWhatsappId": broadcast.integration_whatsapp_id.map(|id| id.to_string()),
            "integrationMessengerId": broadcast.integration_messenger_id.map(|id| id.to_string()),
            "targets": targets,
        });
        let details = resolve_selected_templates(
            conn,
            workspace_id,
            &broadcast.channel,
            &template_selections(&data),
        )?;
        Ok(details
            .into_iter()
            .map(|d| {
                let mut out = json!({
                    "channel": broadcast.channel,
                    "id": d.id.to_string(),
                    "name": d.name,
                    "language": d.language,
                    "category": d.category,
                    "status": d.status,
                    "components": d.components,
                    "inboxId": d.inbox_id.to_string(),
                    "integrationName": d.integration_name,
                });
                if let Some(format) = d.parameter_format {
                    out["parameterFormat"] = json!(format);
                }
                out
            })
            .collect())
    })
    .await
}

fn truncate(text: String) -> String {
    text.chars().take(BROADCAST_NAME_MAX_LENGTH).collect()
}

fn join_template_names(details: &[TemplateDetail]) -> String {
    truncate(
        details
            .iter()
            .map(|d| match &d.integration_name {
                Some(page) if !page.is_empty() => {
                    format!("{page}{BROADCAST_NAME_SEPARATOR}{}", d.name)
                }
                _ => d.name.clone(),
            })
            .collect::<Vec<_>>()
            .join(BROADCAST_NAME_TARGET_SEPARATOR),
    )
}

/// `requireTemplateName`: every template found, approved, payment buttons matched.
fn require_template_name(
    conn: &Connection,
    workspace_id: i64,
    channel: &str,
    selections: &[Selection],
) -> Result<String> {
    let details = resolve_selected_templates(conn, workspace_id, channel, selections)?;
    if details.is_empty() || details.len() != selections.len() {
        return Err(invalid("Template not found", "templateId"));
    }
    if details.iter().any(|d| d.status != APPROVED_TEMPLATE_STATUS) {
        return Err(invalid("Template is not approved", "targets"));
    }
    if channel == "whatsapp" {
        let schema = wa_template_params_schema();
        for (selection, detail) in selections.iter().zip(&details) {
            // `templateData ?? {}`: absent or null params are an empty set.
            let empty = json!({});
            let data = selection.template_data.as_ref().filter(|v| !v.is_null());
            let parsed = schema.safe_parse(data.unwrap_or(&empty));
            let Some(params) = parsed.data.filter(|_| parsed.success) else {
                continue;
            };
            let components = detail.components.as_array().map_or(&[][..], Vec::as_slice);
            if !find_payment_request_mismatches(&params, components).is_empty() {
                return Err(invalid(
                    "Payment request does not match the template's payment buttons",
                    "templateId",
                ));
            }
        }
    }
    Ok(join_template_names(&details))
}

/// What the ownership check loaded (`BroadcastTargetContext`).
#[derive(Debug, Default)]
struct TargetContext {
    inboxes: HashMap<i64, String>,
    flows: HashMap<i64, String>,
}

fn exists(conn: &Connection, table: &str, id: i64, workspace_id: i64) -> Result<bool> {
    let sql = format!(r#"SELECT 1 FROM "{table}" WHERE "id" = ?1 AND "workspaceId" = ?2"#);
    Ok(conn
        .query_row(&sql, params![id, workspace_id], |_| Ok(()))
        .optional()
        .map_err(repo::Error::from)?
        .is_some())
}

/// `assertBroadcastTargetsOwned` (+ `listOwnedTargetInboxes` / `listOwnedTargetFlows`).
fn assert_targets_owned(
    conn: &Connection,
    workspace_id: i64,
    data: &Value,
) -> Result<TargetContext> {
    let channel = str_of(data, "channel").unwrap_or_default();
    if let Some(id) = id_of(data.get("integrationMessengerId"))
        && !exists(conn, "IntegrationMessenger", id, workspace_id)?
    {
        return Err(invalid("Integration not found", "integrationMessengerId"));
    }
    if let Some(id) = id_of(data.get("integrationWhatsappId"))
        && !exists(conn, "IntegrationWhatsapp", id, workspace_id)?
    {
        return Err(invalid("Integration not found", "integrationWhatsappId"));
    }
    let mut context = TargetContext::default();
    let inbox_ids: Vec<i64> = targets(data)
        .iter()
        .filter_map(|t| id_of(t.get("inboxId")))
        .collect();
    if !inbox_ids.is_empty() {
        let mut stmt = conn.prepare(
            r#"SELECT "id", "name" FROM "Inbox" WHERE "id" IN (SELECT value FROM json_each(?1))
               AND "workspaceId" = ?2 AND "channel" = ?3"#,
        )?;
        let rows = stmt
            .query_map(
                params![json!(inbox_ids).to_string(), workspace_id, channel],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo::Error::from)?;
        context.inboxes = rows.into_iter().collect();
    }
    // Flows run before the inbox check, as `Promise.all` order of errors.
    let flow_targets: Vec<(i64, i64)> = targets(data)
        .iter()
        .filter_map(|t| Some((id_of(t.get("inboxId"))?, id_of(t.get("flowId"))?)))
        .collect();
    if !flow_targets.is_empty() {
        let flow_ids: Vec<i64> = flow_targets
            .iter()
            .map(|(_, flow)| *flow)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut stmt = conn.prepare(
            r#"SELECT f."id", f."name", (SELECT v."nodes" FROM "FlowVersion" AS v WHERE v."flowId" = f."id" AND v."isLatest" = 1 LIMIT 1)
               FROM "Flow" AS f WHERE f."id" IN (SELECT value FROM json_each(?1)) AND f."workspaceId" = ?2"#,
        )?;
        let flows = stmt
            .query_map(params![json!(flow_ids).to_string(), workspace_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo::Error::from)?;
        if flow_targets
            .iter()
            .any(|(_, flow)| !flows.iter().any(|f| f.0 == *flow))
        {
            return Err(invalid("Flow not found", "flowId"));
        }
        let step_type = match channel {
            "whatsapp" => Some("sendWaTemplateMessage"),
            "messenger" => Some("sendMessengerTemplateMessage"),
            _ => None,
        };
        if let Some(step_type) = step_type {
            let start_templates: HashMap<i64, i64> = flows
                .iter()
                .filter_map(|(id, _, nodes)| {
                    let nodes: Value = serde_json::from_str(nodes.as_deref()?).ok()?;
                    let (_, template) = find_template_start_step(&nodes, step_type)?;
                    Some((*id, template.parse().ok()?))
                })
                .collect();
            let template_ids: Vec<i64> = start_templates
                .values()
                .copied()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            let details = load_template_details(conn, channel, workspace_id, &template_ids, None)?;
            let on_its_page = flow_targets.iter().all(|(inbox, flow)| {
                start_templates
                    .get(flow)
                    .and_then(|template| details.iter().find(|d| d.id == *template))
                    .is_some_and(|d| d.inbox_id == *inbox)
            });
            if !on_its_page {
                return Err(invalid(
                    "The flow's template does not belong to the selected page",
                    "targets",
                ));
            }
        }
        context.flows = flows.into_iter().map(|(id, name, _)| (id, name)).collect();
    }
    if !inbox_ids.iter().all(|id| context.inboxes.contains_key(id)) {
        return Err(invalid("Inbox not found", "targets"));
    }
    Ok(context)
}

/// `resolveDraftBroadcastName`: template names, else flow targets, else the flow, else the default.
fn resolve_draft_name(
    conn: &Connection,
    workspace_id: i64,
    data: &Value,
    context: &TargetContext,
) -> Result<String> {
    let channel = str_of(data, "channel").unwrap_or_default();
    let selections = template_selections(data);
    if !selections.is_empty() {
        return require_template_name(conn, workspace_id, channel, &selections);
    }
    let segments: Vec<String> = targets(data)
        .iter()
        .filter_map(|t| {
            let flow = context.flows.get(&id_of(t.get("flowId"))?)?;
            Some(
                match id_of(t.get("inboxId")).and_then(|i| context.inboxes.get(&i)) {
                    Some(page) => format!("{page}{BROADCAST_NAME_SEPARATOR}{flow}"),
                    None => flow.clone(),
                },
            )
        })
        .collect();
    if !segments.is_empty() {
        return Ok(truncate(segments.join(BROADCAST_NAME_TARGET_SEPARATOR)));
    }
    match id_of(data.get("flowId")) {
        Some(flow_id) => conn
            .query_row(
                r#"SELECT "name" FROM "Flow" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                params![workspace_id, flow_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(repo::Error::from)?
            .ok_or_else(|| invalid("Flow not found", "flowId")),
        None => Ok(DEFAULT_BROADCAST_NAME.to_owned()),
    }
}

/// `buildStoredTemplateData`: params + `buttons` for a template send only.
fn stored_template_data(slot: &Value) -> Option<String> {
    id_of(slot.get("templateId"))?;
    let mut data = slot.get("templateData").filter(|d| d.is_object())?.clone();
    data["buttons"] = slot.get("buttons").cloned().unwrap_or_else(|| json!([]));
    Some(data.to_string())
}

/// `minuteStart`: `startOfMinute` of a unix-ms instant.
pub fn start_of_minute(ms: i64) -> i64 {
    ms - ms.rem_euclid(60_000)
}

/// `Date.parse` of an ISO time to unix ms.
fn parse_time(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// `buildBroadcastColumns`: the editable columns as `(column, value)`.
fn broadcast_columns(
    data: &Value,
    can_view_email_and_phone: bool,
) -> Vec<(&'static str, SqlValue)> {
    let text = |v: Option<String>| v.map_or(SqlValue::Null, SqlValue::Text);
    let int = |v: Option<i64>| v.map_or(SqlValue::Null, SqlValue::Integer);
    let target_mode = partials::resolve_broadcast_target_mode(targets(data));
    let legacy = target_mode == "channel";
    let filter = data
        .get("contactFilter")
        .filter(|f| !f.is_null())
        .and_then(|f| serde_json::from_value::<FilterCriteria>(f.clone()).ok());
    let pruned = prune_email_phone_filter_conditions(filter.as_ref(), can_view_email_and_phone)
        .and_then(|f| serde_json::to_string(&f).ok());
    let schedules_at = str_of(data, "schedulesAt")
        .and_then(parse_time)
        .unwrap_or_else(now_ms);
    vec![
        ("channel", text(str_of(data, "channel").map(str::to_owned))),
        (
            "subaction",
            text(str_of(data, "subaction").map(str::to_owned)),
        ),
        ("targetMode", SqlValue::Text(target_mode.to_owned())),
        ("flowId", int(id_of(data.get("flowId")).filter(|_| legacy))),
        (
            "templateId",
            int(id_of(data.get("templateId")).filter(|_| legacy)),
        ),
        (
            "templateData",
            text(stored_template_data(data).filter(|_| legacy)),
        ),
        (
            "integrationWhatsappId",
            int(id_of(data.get("integrationWhatsappId")).filter(|_| legacy)),
        ),
        (
            "integrationMessengerId",
            int(id_of(data.get("integrationMessengerId")).filter(|_| legacy)),
        ),
        ("contactFilter", text(pruned)),
        (
            "schedulesType",
            text(str_of(data, "schedulesType").map(str::to_owned)),
        ),
        (
            "schedulesAt",
            SqlValue::Integer(start_of_minute(schedules_at)),
        ),
        (
            "audienceRangeStart",
            int(data.get("audienceRangeStart").and_then(Value::as_i64)),
        ),
        (
            "audienceRangeEnd",
            int(data.get("audienceRangeEnd").and_then(Value::as_i64)),
        ),
        (
            "sendRatePerMinute",
            int(data.get("sendRatePerMinute").and_then(Value::as_i64)),
        ),
    ]
}

/// `replaceTargets`: delete + insert the page rows.
fn replace_targets(conn: &Connection, broadcast_id: i64, data: &Value) -> Result<()> {
    conn.execute(
        r#"DELETE FROM "BroadcastTarget" WHERE "broadcastId" = ?1"#,
        [broadcast_id],
    )?;
    for target in targets(data) {
        conn.execute(
            r#"INSERT INTO "BroadcastTarget" ("broadcastId", "inboxId", "flowId", "templateId", "templateData") VALUES (?1, ?2, ?3, ?4, ?5)"#,
            params![
                broadcast_id,
                id_of(target.get("inboxId")),
                id_of(target.get("flowId")),
                id_of(target.get("templateId")),
                stored_template_data(target),
            ],
        )?;
    }
    Ok(())
}

/// `copyBroadcastTargets`.
fn copy_targets(conn: &Connection, source_id: i64, broadcast_id: i64) -> Result<()> {
    conn.execute(
        r#"INSERT INTO "BroadcastTarget" ("broadcastId", "inboxId", "flowId", "templateId", "templateData")
           SELECT ?2, "inboxId", "flowId", "templateId", "templateData" FROM "BroadcastTarget" WHERE "broadcastId" = ?1"#,
        params![source_id, broadcast_id],
    )?;
    Ok(())
}

fn insert_broadcast(conn: &Connection, columns: &[(&str, SqlValue)]) -> Result<Broadcast> {
    let id = new_id();
    let names: Vec<String> = columns.iter().map(|(c, _)| format!("\"{c}\"")).collect();
    let marks: Vec<String> = (2..=columns.len() + 1).map(|i| format!("?{i}")).collect();
    let sql = format!(
        r#"INSERT INTO "Broadcast" ("id", {}) VALUES (?1, {})"#,
        names.join(", "),
        marks.join(", ")
    );
    let mut values = vec![SqlValue::Integer(id)];
    values.extend(columns.iter().map(|(_, v)| v.clone()));
    conn.execute(&sql, params_from_iter(values.iter()))?;
    repo::find_one::<Broadcast>(conn, r#""id" = ?1"#, &[&id])?
        .ok_or_else(|| chatbotx_exception("Broadcast not created"))
}

/// The SET list of [`broadcast_columns`] plus `extra`, bound from `?3`.
fn set_clause(columns: &[(&str, SqlValue)]) -> (String, Vec<SqlValue>) {
    let set = columns
        .iter()
        .enumerate()
        .map(|(i, (c, _))| format!("\"{c}\" = ?{}", i + 3))
        .collect::<Vec<_>>()
        .join(", ");
    (set, columns.iter().map(|(_, v)| v.clone()).collect())
}

/// A send-rate patch as a column.
fn rate_column(rate: Option<i64>) -> (&'static str, SqlValue) {
    (
        "sendRatePerMinute",
        rate.map_or(SqlValue::Null, SqlValue::Integer),
    )
}

fn with_rate(
    mut columns: Vec<(&'static str, SqlValue)>,
    rate: Option<Option<i64>>,
) -> Vec<(&'static str, SqlValue)> {
    if let Some(rate) = rate {
        columns.retain(|(c, _)| *c != "sendRatePerMinute");
        columns.push(rate_column(rate));
    }
    columns
}

/// `create`: validates, inserts the broadcast and its pages in one transaction.
pub async fn create(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    data: Value,
    can_view_email_and_phone: bool,
) -> Result<Broadcast> {
    assert_draft_payload(&data)?;
    let data = resolve_targets_to_persist(&data)?;
    let recurring = str_of(&data, "schedulesType") == Some("recurring");
    let draft = data.get("saveAsDraft").and_then(Value::as_bool) == Some(true);
    let schedules_type = str_of(&data, "schedulesType")
        .unwrap_or_default()
        .to_owned();
    let created = tx::write(db, move |tx| {
        let context = assert_targets_owned(tx, workspace_id, &data)?;
        let name = resolve_draft_name(tx, workspace_id, &data, &context)?;
        let status = if draft || recurring {
            "draft"
        } else {
            "scheduled"
        };
        let channel = str_of(&data, "channel").unwrap_or_default();
        let restriction = (!(draft || recurring) && plan_policy::applies_to_channel(channel))
            .then(|| {
                plan_policy::restriction_for(
                    &plan_policy::resolve_for_workspace(workspace_id),
                    channel,
                )
            })
            .flatten();
        let rate = data.get("sendRatePerMinute").and_then(Value::as_i64);
        let override_rate = match &restriction {
            Some(restriction) => Some(Some(plan_policy::assert_activation_allowed(
                tx,
                workspace_id,
                channel,
                restriction,
                rate,
                None,
            )?)),
            None => None,
        };
        let mut columns = vec![
            ("workspaceId", SqlValue::Integer(workspace_id)),
            ("name", SqlValue::Text(name)),
            ("status", SqlValue::Text(status.to_owned())),
        ];
        columns.extend(with_rate(
            broadcast_columns(&data, can_view_email_and_phone),
            override_rate,
        ));
        let inserted = insert_broadcast(tx, &columns)?;
        replace_targets(tx, inserted.id, &data)?;
        if !recurring {
            return Ok(inserted);
        }
        let rule = data
            .get("recurrence")
            .filter(|r| !r.is_null())
            .ok_or_else(repeat_settings_required)?;
        let series = super::recurrence::upsert_rule(tx, workspace_id, inserted.id, rule)?;
        if draft {
            return Ok(inserted);
        }
        let id = activate_recurring_template(
            tx,
            workspace_id,
            r#""id" = ?1"#,
            &[SqlValue::Integer(inserted.id)],
            &inserted,
            None,
            &series,
        )?;
        repo::find_one::<Broadcast>(tx, r#""id" = ?1"#, &[&id])?
            .ok_or_else(|| chatbotx_exception("Broadcast is not a draft"))
    })
    .await?;
    audit
        .audit(
            "create",
            &format!("created a new broadcast (#{})", created.id),
        )
        .await;
    if schedules_type == "now" && !draft {
        audit
            .audit("launch", &format!("launched a broadcast (#{})", created.id))
            .await;
    }
    if schedules_type == "recurring" && !draft {
        audit_recurrence_started(audit, created.id).await;
    }
    Ok(created)
}

fn repeat_settings_required() -> BusinessError {
    invalid("Repeat settings are required", "recurrence")
}

async fn audit_recurrence_started(audit: &AuditService, id: i64) {
    audit
        .audit(
            "broadcast_recurrence_started",
            &format!("started a repeating broadcast (#{id})"),
        )
        .await;
}

/// `broadcastDraftPurgeJobId`.
fn draft_purge_job_id(broadcast_id: i64, requester: &str) -> String {
    format!("schedule-purge-draft-broadcast-{broadcast_id}-{requester}")
}

/// `queueDraftRecipientsPurge`: one cleanup job per draft and requester.
pub async fn queue_draft_recipients_purge(
    jobs: Option<&Arc<dyn Enqueuer>>,
    workspace_id: i64,
    broadcast_id: i64,
    requester: &str,
) {
    let Some(jobs) = jobs else { return };
    let opts = JobOptions {
        job_id: Some(draft_purge_job_id(broadcast_id, requester)),
        remove_on_complete: Some(Retention::Remove),
        remove_on_fail: Some(Retention::Remove),
        ..JobOptions::default()
    };
    let data = json!({
        "type": "purgeDraftBroadcastRecipients",
        "data": { "workspaceId": workspace_id.to_string(), "broadcastId": broadcast_id.to_string() },
    });
    if let Err(error) = jobs
        .add(
            QueueName::Schedule,
            "purgeDraftBroadcastRecipients",
            data,
            opts,
        )
        .await
    {
        tracing::error!(%error, broadcast_id, "Failed to queue the draft recipients purge");
    }
}

/// `isPrepareJobLive` (the `jobs` table of this database) or leftover recipients.
fn has_pending_preparation(conn: &Connection, broadcast_id: i64) -> Result<bool> {
    let job_id =
        chatbotx_jobs::queues::broadcast::broadcast_prepare_job_id(&broadcast_id.to_string());
    let live: Option<String> = conn
        .query_row(
            "SELECT state FROM jobs WHERE queue = 'schedule' AND job_id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(repo::Error::from)?;
    if live.is_some_and(|state| state != "failed" && state != "completed") {
        return Ok(true);
    }
    Ok(chatbotx_db::repo::broadcast_purge::has_broadcast_recipients(conn, broadcast_id)?)
}

const DRAFT_SCOPE: &str =
    r#""id" = ?1 AND "workspaceId" = ?2 AND "status" = 'draft' AND "deletedAt" IS NULL"#;

/// `draftActivationScope`: refuses (and queues the cleanup) while a previous
/// prepare may still write recipients; else the draft scope pinned to its epoch.
async fn draft_activation_scope(
    db: &Db,
    jobs: Option<&Arc<dyn Enqueuer>>,
    workspace_id: i64,
    broadcast_id: i64,
) -> Result<String> {
    let pending = tx::read(db, move |conn| {
        let draft =
            repo::find_one::<Broadcast>(conn, DRAFT_SCOPE, &[&broadcast_id, &workspace_id])?;
        match draft {
            None => Ok(None),
            Some(draft) => Ok(Some((
                draft.resume_count,
                has_pending_preparation(conn, broadcast_id)?,
            ))),
        }
    })
    .await?;
    match pending {
        None => Ok(DRAFT_SCOPE.to_owned()),
        Some((_, true)) => {
            queue_draft_recipients_purge(jobs, workspace_id, broadcast_id, "draft").await;
            Err(chatbotx_exception(
                "Broadcast is still clearing recipients from its previous schedule. Try again in a moment.",
            ))
        }
        Some((epoch, false)) => Ok(format!(r#"{DRAFT_SCOPE} AND "resumeCount" = {epoch}"#)),
    }
}

/// `dropUndeliverableTargets`: removes pages with nothing to send; none left fails.
fn drop_undeliverable_targets(conn: &Connection, broadcast: &Broadcast) -> Result<()> {
    if broadcast.target_mode != "targets" {
        return Ok(());
    }
    let total: i64 = conn.query_row(
        r#"SELECT count(*) FROM "BroadcastTarget" WHERE "broadcastId" = ?1"#,
        [broadcast.id],
        |row| row.get(0),
    )?;
    let empty: i64 = conn.query_row(
        r#"SELECT count(*) FROM "BroadcastTarget" WHERE "broadcastId" = ?1 AND "templateId" IS NULL AND "flowId" IS NULL"#,
        [broadcast.id],
        |row| row.get(0),
    )?;
    if empty == total {
        return Err(invalid(
            "Select a template or flow for at least one page",
            "targets",
        ));
    }
    conn.execute(
        r#"DELETE FROM "BroadcastTarget" WHERE "broadcastId" = ?1 AND "templateId" IS NULL AND "flowId" IS NULL"#,
        [broadcast.id],
    )?;
    Ok(())
}

fn scope_params(workspace_id: i64, broadcast_id: i64) -> [SqlValue; 2] {
    [
        SqlValue::Integer(broadcast_id),
        SqlValue::Integer(workspace_id),
    ]
}

/// `scheduleDraft`: `draft` -> `scheduled` (or `recurring`).
#[allow(clippy::too_many_arguments)]
pub async fn schedule_draft(
    db: &Db,
    audit: &AuditService,
    jobs: Option<&Arc<dyn Enqueuer>>,
    workspace_id: i64,
    broadcast_id: i64,
    schedules_type: String,
    schedules_at: i64,
    send_rate_per_minute: Option<Option<i64>>,
    recurrence: Option<Value>,
) -> Result<Value> {
    if schedules_type == "recurring" {
        let id = tx::write(db, move |tx| {
            let scope = format!(r#"{DRAFT_SCOPE} AND "occurrenceAt" IS NULL"#);
            let draft = repo::find_one::<Broadcast>(tx, &scope, &[&broadcast_id, &workspace_id])?
                .ok_or_else(|| chatbotx_exception("Broadcast is not a draft"))?;
            let series = match &recurrence {
                Some(rule) => Some(super::recurrence::upsert_rule(
                    tx,
                    workspace_id,
                    draft.id,
                    rule,
                )?),
                None => lock_series(tx, workspace_id, draft.id)?,
            }
            .ok_or_else(repeat_settings_required)?;
            drop_undeliverable_targets(tx, &draft)?;
            activate_recurring_template(
                tx,
                workspace_id,
                &scope,
                &scope_params(workspace_id, broadcast_id),
                &draft,
                send_rate_per_minute,
                &series,
            )
        })
        .await?;
        audit_recurrence_started(audit, id).await;
        return Ok(json!({ "id": id.to_string() }));
    }
    let scope = draft_activation_scope(db, jobs, workspace_id, broadcast_id).await?;
    let launch = schedules_type == "now";
    let id = tx::write(db, move |tx| {
        let draft = repo::find_one::<Broadcast>(tx, &scope, &[&broadcast_id, &workspace_id])?
            .ok_or_else(|| chatbotx_exception("Broadcast is not a draft"))?;
        let ctx = plan_policy::resolve_for_workspace(workspace_id);
        let mut rate_patch = send_rate_per_minute;
        if let Some(restriction) = plan_policy::restriction_for(&ctx, &draft.channel) {
            drop_undeliverable_targets(tx, &draft)?;
            let rate = partials::resolve_activation_send_rate(send_rate_per_minute, draft.send_rate_per_minute);
            rate_patch = Some(Some(plan_policy::assert_activation_allowed(
                tx, workspace_id, &draft.channel, &restriction, rate, Some(draft.id),
            )?));
        }
        let mut set = format!(r#""status" = 'scheduled', "schedulesType" = '{schedules_type}', "schedulesAt" = {schedules_at}"#);
        if let Some(rate) = rate_patch {
            set.push_str(&format!(r#", "sendRatePerMinute" = {}"#, rate.map_or("NULL".to_owned(), |r| r.to_string())));
        }
        let id = guarded_update(tx, &set, &scope, &scope_params(workspace_id, broadcast_id))?
            .ok_or_else(|| chatbotx_exception("Broadcast is not a draft"))?;
        drop_undeliverable_targets(tx, &draft)?;
        Ok(id)
    })
    .await?;
    if launch {
        audit
            .audit("launch", &format!("launched a broadcast (#{id})"))
            .await;
    }
    Ok(json!({ "id": id.to_string() }))
}

/// `updateDraft`: re-applies a create payload to a draft; `{ id, status }`.
pub async fn update_draft(
    db: &Db,
    audit: &AuditService,
    jobs: Option<&Arc<dyn Enqueuer>>,
    workspace_id: i64,
    broadcast_id: i64,
    data: Value,
    can_view_email_and_phone: bool,
) -> Result<Value> {
    assert_draft_payload(&data)?;
    let data = resolve_targets_to_persist(&data)?;
    let recurring = str_of(&data, "schedulesType") == Some("recurring");
    let draft = data.get("saveAsDraft").and_then(Value::as_bool) == Some(true);
    let status = if draft {
        "draft"
    } else if recurring {
        "recurring"
    } else {
        "scheduled"
    };
    let scope = if status == "scheduled" {
        draft_activation_scope(db, jobs, workspace_id, broadcast_id).await?
    } else {
        DRAFT_SCOPE.to_owned()
    };
    let launch = status == "scheduled" && str_of(&data, "schedulesType") == Some("now");
    let id = tx::write(db, move |tx| {
        let context = assert_targets_owned(tx, workspace_id, &data)?;
        let derived = resolve_draft_name(tx, workspace_id, &data, &context)?;
        let channel = str_of(&data, "channel").unwrap_or_default().to_owned();
        let name_set = r#""name" = COALESCE(NULLIF(TRIM("name"), ''), ?3)"#;
        if recurring {
            let scope = format!(r#"{DRAFT_SCOPE} AND "occurrenceAt" IS NULL"#);
            let rule = data
                .get("recurrence")
                .filter(|r| !r.is_null())
                .ok_or_else(repeat_settings_required)?;
            repo::find_one::<Broadcast>(tx, &scope, &[&broadcast_id, &workspace_id])?
                .ok_or_else(|| chatbotx_exception("Broadcast is not a draft"))?;
            let series = super::recurrence::upsert_rule(tx, workspace_id, broadcast_id, rule)?;
            let columns = broadcast_columns(&data, can_view_email_and_phone);
            let id = update_columns(
                tx,
                &scope,
                workspace_id,
                broadcast_id,
                name_set,
                &derived,
                "draft",
                &columns,
            )?;
            replace_targets(tx, id, &data)?;
            if draft {
                return Ok(id);
            }
            let updated = repo::find_one::<Broadcast>(tx, r#""id" = ?1"#, &[&id])?
                .ok_or_else(|| chatbotx_exception("Broadcast is not a draft"))?;
            return activate_recurring_template(
                tx,
                workspace_id,
                r#""id" = ?1"#,
                &[SqlValue::Integer(id)],
                &updated,
                None,
                &series,
            );
        }
        let restriction = (!draft && plan_policy::applies_to_channel(&channel))
            .then(|| {
                plan_policy::restriction_for(
                    &plan_policy::resolve_for_workspace(workspace_id),
                    &channel,
                )
            })
            .flatten();
        let mut columns = broadcast_columns(&data, can_view_email_and_phone);
        if let Some(restriction) = &restriction {
            repo::find_one::<Broadcast>(tx, &scope, &[&broadcast_id, &workspace_id])?
                .ok_or_else(|| chatbotx_exception("Broadcast is not a draft"))?;
            let rate = data.get("sendRatePerMinute").and_then(Value::as_i64);
            let allowed = plan_policy::assert_activation_allowed(
                tx,
                workspace_id,
                &channel,
                restriction,
                rate,
                Some(broadcast_id),
            )?;
            columns = with_rate(columns, Some(Some(allowed)));
        }
        let id = update_columns(
            tx,
            &scope,
            workspace_id,
            broadcast_id,
            name_set,
            &derived,
            status,
            &columns,
        )?;
        replace_targets(tx, id, &data)?;
        Ok(id)
    })
    .await?;
    if launch {
        audit
            .audit("launch", &format!("launched a broadcast (#{id})"))
            .await;
    }
    if status == "recurring" {
        audit_recurrence_started(audit, id).await;
    }
    Ok(json!({ "id": id.to_string(), "status": status }))
}

#[allow(clippy::too_many_arguments)]
fn update_columns(
    conn: &Connection,
    scope: &str,
    workspace_id: i64,
    broadcast_id: i64,
    name_set: &str,
    derived_name: &str,
    status: &str,
    columns: &[(&'static str, SqlValue)],
) -> Result<i64> {
    let mut all = vec![("status", SqlValue::Text(status.to_owned()))];
    all.extend(columns.iter().cloned());
    let (set, values) = set_clause(&all);
    // `?3` is the derived name; the columns follow from `?4`.
    let set = set
        .split(", ")
        .enumerate()
        .map(|(i, part)| part.replace(&format!("?{}", i + 3), &format!("?{}", i + 4)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut params = vec![
        SqlValue::Integer(broadcast_id),
        SqlValue::Integer(workspace_id),
        SqlValue::Text(derived_name.to_owned()),
    ];
    params.extend(values);
    guarded_update(conn, &format!("{name_set}, {set}"), scope, &params)?
        .ok_or_else(|| chatbotx_exception("Broadcast is not a draft"))
}

/// The clone name: the base (no `(Copy N)`) with the next free copy number.
fn clone_name(conn: &Connection, workspace_id: i64, source_name: &str) -> Result<String> {
    let base = match source_name.rfind(BROADCAST_COPY_LABEL) {
        Some(at)
            if source_name.ends_with(')')
                && source_name[at + BROADCAST_COPY_LABEL.len()..source_name.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_digit())
                && source_name.len() > at + BROADCAST_COPY_LABEL.len() + 1 =>
        {
            &source_name[..at]
        }
        _ => source_name,
    };
    let prefix = format!("{base}{BROADCAST_COPY_LABEL}");
    let mut stmt = conn.prepare(
        r#"SELECT "name" FROM "Broadcast" WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL AND substr("name", 1, ?3) = ?2"#,
    )?;
    let names = stmt
        .query_map(
            params![workspace_id, prefix, prefix.chars().count() as i64],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()
        .map_err(repo::Error::from)?;
    let highest = names
        .iter()
        .filter_map(|name| {
            let rest = name.strip_prefix(&prefix)?.strip_suffix(')')?;
            (!rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
                .then(|| rest.parse::<i64>().ok())?
        })
        .max()
        .unwrap_or(0);
    Ok(format!("{prefix}{})", highest + 1))
}

fn pruned_filter(
    filter: Option<&chatbotx_db::repo::Json>,
    can_view_email_and_phone: bool,
) -> SqlValue {
    let parsed = filter.and_then(|f| serde_json::from_value::<FilterCriteria>(f.0.clone()).ok());
    prune_email_phone_filter_conditions(parsed.as_ref(), can_view_email_and_phone)
        .and_then(|f| serde_json::to_string(&f).ok())
        .map_or(SqlValue::Null, SqlValue::Text)
}

fn copied_columns(source: &Broadcast) -> Vec<(&'static str, SqlValue)> {
    let int = |v: Option<i64>| v.map_or(SqlValue::Null, SqlValue::Integer);
    vec![
        ("workspaceId", SqlValue::Integer(source.workspace_id)),
        ("channel", SqlValue::Text(source.channel.clone())),
        ("subaction", SqlValue::Text(source.subaction.clone())),
        ("targetMode", SqlValue::Text(source.target_mode.clone())),
        ("flowId", int(source.flow_id)),
        ("templateId", int(source.template_id)),
        (
            "templateData",
            source
                .template_data
                .as_ref()
                .map_or(SqlValue::Null, |d| SqlValue::Text(d.0.to_string())),
        ),
        ("integrationWhatsappId", int(source.integration_whatsapp_id)),
        (
            "integrationMessengerId",
            int(source.integration_messenger_id),
        ),
        ("audienceRangeStart", int(source.audience_range_start)),
        ("audienceRangeEnd", int(source.audience_range_end)),
        ("sendRatePerMinute", int(source.send_rate_per_minute)),
    ]
}

/// `cloneBroadcast`: a new `draft` copy with the next `(Copy N)` name.
pub async fn clone_broadcast(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    broadcast_id: i64,
    can_view_email_and_phone: bool,
) -> Result<Broadcast> {
    let cloned = tx::write(db, move |tx| {
        let source = repo::find_one::<Broadcast>(
            tx,
            r#""id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
            &[&broadcast_id, &workspace_id],
        )?
        .ok_or_else(|| BusinessError::not_found("Broadcast not found"))?;
        let mut columns = copied_columns(&source);
        columns.extend([
            (
                "name",
                SqlValue::Text(clone_name(tx, workspace_id, &source.name)?),
            ),
            ("status", SqlValue::Text("draft".into())),
            (
                "contactFilter",
                pruned_filter(source.contact_filter.as_ref(), can_view_email_and_phone),
            ),
            (
                "schedulesType",
                SqlValue::Text(source.schedules_type.clone()),
            ),
            ("schedulesAt", SqlValue::Integer(source.schedules_at)),
        ]);
        let clone = insert_broadcast(tx, &columns)?;
        copy_targets(tx, source.id, clone.id)?;
        if is_recurring_template(&source)
            && let Some(series) = recurrence_repo::find_by_broadcast_id(tx, source.id)?
        {
            recurrence_repo::insert_if_missing(tx, source.workspace_id, clone.id, &series.rule)?;
        }
        Ok(clone)
    })
    .await?;
    audit
        .audit("create", &format!("cloned a broadcast (#{})", cloned.id))
        .await;
    Ok(cloned)
}

/// `isContactFilterShape`: `and`/`or` with known condition fields only.
fn is_contact_filter_shape(value: &Value) -> bool {
    let operator_ok = matches!(
        value.get("operator").and_then(Value::as_str),
        Some("and" | "or")
    );
    let Some(conditions) = value.get("conditions").and_then(Value::as_array) else {
        return false;
    };
    operator_ok
        && conditions.iter().all(|c| {
            c.get("field").and_then(Value::as_str).is_some_and(|f| {
                chatbotx_db::repo::partials::contact::CONTACT_FILTER_FIELDS.contains(&f)
            })
        })
}

/// `resendWithPruning`: a sent/failed broadcast cloned as a new send-now one.
pub async fn resend_with_pruning(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    id: i64,
    can_view_email_and_phone: bool,
) -> Result<Broadcast> {
    let created = tx::write(db, move |tx| {
        let source = repo::find_one::<Broadcast>(
            tx,
            r#""id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
            &[&id, &workspace_id],
        )?
        .ok_or_else(|| BusinessError::not_found("Broadcast not found"))?;
        if !(source.status == "sent" || source.status == "failed") || is_recurring_template(&source)
        {
            return Err(chatbotx_exception("Broadcast is not sent"));
        }
        let filter = source
            .contact_filter
            .as_ref()
            .filter(|f| is_contact_filter_shape(&f.0));
        let mut rate = None;
        if plan_policy::applies_to_channel(&source.channel)
            && let Some(restriction) = plan_policy::restriction_for(
                &plan_policy::resolve_for_workspace(workspace_id),
                &source.channel,
            )
        {
            rate = Some(Some(plan_policy::assert_activation_allowed(
                tx,
                workspace_id,
                &source.channel,
                &restriction,
                source.send_rate_per_minute,
                None,
            )?));
        }
        let mut columns = with_rate(copied_columns(&source), rate);
        columns.extend([
            ("name", SqlValue::Text(format!("{} (Resend)", source.name))),
            ("status", SqlValue::Text("scheduled".into())),
            ("schedulesType", SqlValue::Text("now".into())),
            ("schedulesAt", SqlValue::Integer(now_ms())),
            (
                "contactFilter",
                pruned_filter(filter, can_view_email_and_phone),
            ),
        ]);
        let inserted = insert_broadcast(tx, &columns)?;
        copy_targets(tx, source.id, inserted.id)?;
        Ok(inserted)
    })
    .await?;
    audit
        .audit("launch", &format!("launched a broadcast (#{})", created.id))
        .await;
    Ok(created)
}
