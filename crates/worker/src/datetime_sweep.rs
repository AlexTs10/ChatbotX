//! The date/time sweeps of the `schedule` queue: `evaluateTriggers` /
//! `cleanupTriggers` (`trigger/datetime-trigger-scanner.ts`,
//! `services/datetime-trigger-evaluator.ts`) and `evaluateDateTimeWebhooks`
//! / `cleanupWebhookExecutions` (`webhook/datetime-webhook-scanner.ts`,
//! `services/datetime-webhook-evaluator.ts`), with `shared/datetime-sweep.ts`
//! and `trigger/utils/datetime-calculator.ts`.
//!
//! ponytail: the active entities load in one query (the original pages them
//! 100 at a time; same order, same 10 000 cap). The Redis execution cache and
//! the 30 s lock are DurableKv keys with the same names and TTLs.

use std::collections::{HashMap, HashSet};

use chatbotx_core::js;
use chatbotx_core::utils::crypto::sha256_hex;
use chatbotx_db::repo::{iso, new_id, now_ms};
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{JobError, JobOptions, QueueName, Retention};
use serde_json::{Value, json};

use crate::condition::{parse_date_time_value, wall_ms};
use crate::{Deps, db_fail};

const PAGE_SIZE: usize = 1000;
const MAX_PER_SCAN: usize = 10_000;
const DAY_MS: i64 = 86_400_000;
const EXECUTED_TTL_SECS: u64 = 86_400 * 90;
const LOCK_TTL_SECS: u64 = 30;

/// `DateTimeCondition`.
#[derive(Debug, Clone, Default)]
pub(crate) struct DtCondition {
    pub trigger_type: String,
    pub time_value: Option<Value>,
    pub time_type: Option<String>,
    pub at: Option<Value>,
    pub timezone: Option<String>,
    /// `customFieldId` (the condition `sourceId`).
    pub field: String,
}

/// A trigger or webhook with its date/time conditions (`DateTimeSweepEntity`).
#[derive(Debug)]
struct Entity {
    id: i64,
    workspace_id: i64,
    /// `workspace.timezone || "UTC"`.
    timezone: String,
    conditions: Vec<DtCondition>,
    actions: Vec<Value>,
}

/// One `ContactCustomField` row of the sweep.
struct FieldRow {
    contact_id: i64,
    field: String,
    workspace_id: i64,
    value: String,
}

#[derive(Default)]
struct Stats {
    matched: usize,
    failed: usize,
    total: usize,
}

#[derive(Clone, Copy)]
enum Kind {
    Trigger,
    Webhook,
}

impl Kind {
    fn table(self) -> &'static str {
        match self {
            Kind::Trigger => "Trigger",
            Kind::Webhook => "Webhook",
        }
    }
    /// The key prefix and the execution table.
    fn prefix(self) -> &'static str {
        match self {
            Kind::Trigger => "trigger",
            Kind::Webhook => "webhook",
        }
    }
}

fn kv_fail(err: impl std::fmt::Display) -> String {
    err.to_string()
}

/// `scanDateTimeTriggers`.
pub async fn scan_triggers(deps: &Deps) -> Result<(), JobError> {
    scan(deps, Kind::Trigger, now_ms()).await
}

/// `scanDateTimeWebhooks`.
pub async fn scan_webhooks(deps: &Deps) -> Result<(), JobError> {
    scan(deps, Kind::Webhook, now_ms()).await
}

async fn scan(deps: &Deps, kind: Kind, start: i64) -> Result<(), JobError> {
    let start_of_minute = start - start.rem_euclid(60_000);
    let entities = load_entities(deps, kind).await?;
    let mut stats = Stats::default();
    if !entities.is_empty() {
        sweep(deps, kind, &entities, start_of_minute, &mut stats).await?;
    }
    tracing::info!(
        "Datetime {} scan completed: matched={}, failed={}, total={}, durationMs={}",
        kind.prefix(),
        stats.matched,
        stats.failed,
        stats.total,
        now_ms() - start
    );
    Ok(())
}

/// `cleanupTriggerExecutions` / `cleanupWebhookExecutions`: executions older
/// than 90 days.
pub async fn cleanup(deps: &Deps, webhook: bool) -> Result<(), JobError> {
    let table = if webhook {
        "WebhookExecution"
    } else {
        "TriggerExecution"
    };
    let cutoff = now_ms() - 90 * DAY_MS;
    let deleted = deps
        .db
        .write(move |tx| {
            Ok(tx.execute(
                &format!(r#"DELETE FROM "{table}" WHERE "executedAt" < ?1"#),
                [cutoff],
            )?)
        })
        .await
        .map_err(db_fail)?;
    tracing::info!("{table} cleanup completed: deletedCount={deleted}");
    Ok(())
}

/// `collectDateTimeTriggerMap` / `collectDateTimeWebhookMap`.
async fn load_entities(deps: &Deps, kind: Kind) -> Result<Vec<Entity>, JobError> {
    let table = kind.table();
    let (actions, fk) = match kind {
        Kind::Trigger => (r#"e."actions""#, "triggerId"),
        Kind::Webhook => ("'[]'", "webhookId"),
    };
    let rows = deps
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(&format!(
                r#"SELECT e."id", e."workspaceId", w."timezone", {actions}, c."sourceId", c."value"
                   FROM "{table}" e
                   JOIN "Condition" c ON c."{fk}" = e."id" AND c."type" = 'dateTimeBasedTrigger'
                   LEFT JOIN "Workspace" w ON w."id" = e."workspaceId"
                   WHERE e."active" = 1 ORDER BY e."id", c."id""#
            ))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(db_fail)?;
    let mut out: Vec<Entity> = Vec::new();
    for (id, workspace_id, timezone, actions, source_id, value) in rows {
        let condition = source_id
            .filter(|s| !s.is_empty())
            .and_then(|field| parse_condition(field, value.as_deref()));
        if out.last().is_none_or(|e| e.id != id) {
            out.push(Entity {
                id,
                workspace_id,
                timezone: timezone
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "UTC".into()),
                conditions: Vec::new(),
                actions: serde_json::from_str::<Vec<Value>>(&actions).unwrap_or_default(),
            });
        }
        if let (Some(condition), Some(last)) = (condition, out.last_mut()) {
            last.conditions.push(condition);
        }
    }
    out.retain(|e| !e.conditions.is_empty());
    if out.len() > MAX_PER_SCAN {
        tracing::warn!(
            dropped = out.len() - MAX_PER_SCAN,
            max = MAX_PER_SCAN,
            "Datetime {} scan exceeded the per-scan {} cap",
            kind.prefix(),
            kind.prefix()
        );
        out.truncate(MAX_PER_SCAN);
    }
    Ok(out)
}

/// `condition.value as DateTimeTriggerValue`; `None` without a `triggerType`.
fn parse_condition(field: String, value: Option<&str>) -> Option<DtCondition> {
    let value: Value = serde_json::from_str(value?).ok()?;
    let trigger_type = value
        .get("triggerType")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let text = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_owned);
    Some(DtCondition {
        trigger_type: trigger_type.to_owned(),
        time_value: value.get("timeValue").cloned(),
        time_type: text("timeType"),
        at: value.get("at").cloned(),
        timezone: text("timezone"),
        field,
    })
}

/// `forEachSweepPage` over `listContactCustomFieldsForDateTimeSweep`.
async fn sweep(
    deps: &Deps,
    kind: Kind,
    entities: &[Entity],
    start_of_minute: i64,
    stats: &mut Stats,
) -> Result<(), JobError> {
    let field_ids: Vec<i64> = entities
        .iter()
        .flat_map(|e| {
            e.conditions
                .iter()
                .filter_map(|c| c.field.parse::<i64>().ok())
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if field_ids.is_empty() {
        return Ok(());
    }
    let ids_json = json!(field_ids).to_string();
    let mut cursor = (i64::MIN, i64::MIN);
    let mut executed_keys: HashSet<(i64, i64)> = HashSet::new();
    loop {
        let (ids, (field, id)) = (ids_json.clone(), cursor);
        let page = deps
            .db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    r#"SELECT ccf."id", ccf."contactId", ccf."customFieldId", c."workspaceId", ccf."value"
                       FROM "ContactCustomField" ccf JOIN "Contact" c ON c."id" = ccf."contactId"
                       WHERE ccf."customFieldId" IN (SELECT value FROM json_each(?1))
                         AND (ccf."customFieldId", ccf."id") > (?2, ?3)
                       ORDER BY ccf."customFieldId", ccf."id" LIMIT ?4"#,
                )?;
                let rows = stmt
                    .query_map(params![ids, field, id, PAGE_SIZE as i64], |r| {
                        Ok((
                            (r.get::<_, i64>(2)?, r.get::<_, i64>(0)?),
                            FieldRow {
                                contact_id: r.get(1)?,
                                field: r.get::<_, i64>(2)?.to_string(),
                                workspace_id: r.get(3)?,
                                value: r.get(4)?,
                            },
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(db_fail)?;
        let Some((last, _)) = page.last() else {
            return Ok(());
        };
        cursor = *last;
        let full = page.len() == PAGE_SIZE;
        let rows: Vec<FieldRow> = page.into_iter().map(|(_, row)| row).collect();
        match kind {
            Kind::Trigger => {
                trigger_batch(
                    deps,
                    entities,
                    &rows,
                    &mut executed_keys,
                    start_of_minute,
                    stats,
                )
                .await?
            }
            Kind::Webhook => webhook_batch(deps, entities, &rows, start_of_minute, stats).await?,
        }
        if !full {
            return Ok(());
        }
    }
}

/// `filterContactsWithAnyCustomField`: the page's contacts (first-seen order)
/// in the entity's workspace with one of its fields.
fn contacts_to_check(rows: &[FieldRow], entity: &Entity) -> Vec<i64> {
    let fields: HashSet<&str> = entity.conditions.iter().map(|c| c.field.as_str()).collect();
    let mut seen = HashSet::new();
    rows.iter()
        .filter(|r| r.workspace_id == entity.workspace_id && fields.contains(r.field.as_str()))
        .filter(|r| seen.insert(r.contact_id))
        .map(|r| r.contact_id)
        .collect()
}

/// `buildContactCustomFieldMap`.
fn value_map(rows: &[FieldRow]) -> HashMap<i64, HashMap<String, String>> {
    let mut map: HashMap<i64, HashMap<String, String>> = HashMap::new();
    for row in rows {
        map.entry(row.contact_id)
            .or_default()
            .insert(row.field.clone(), row.value.clone());
    }
    map
}

async fn executed_pairs(
    deps: &Deps,
    kind: Kind,
    entity_ids: Vec<i64>,
    contact_ids: Vec<i64>,
) -> Result<HashSet<(i64, i64)>, JobError> {
    if entity_ids.is_empty() || contact_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let (table, column) = match kind {
        Kind::Trigger => ("TriggerExecution", "triggerId"),
        Kind::Webhook => ("WebhookExecution", "webhookId"),
    };
    let (entities, contacts) = (
        json!(entity_ids).to_string(),
        json!(contact_ids).to_string(),
    );
    deps.db
        .read(move |conn| {
            let mut stmt = conn.prepare(&format!(
                r#"SELECT "{column}", "contactId" FROM "{table}"
                   WHERE "{column}" IN (SELECT value FROM json_each(?1)) AND "contactId" IN (SELECT value FROM json_each(?2))"#
            ))?;
            let set = stmt
                .query_map(params![entities, contacts], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<HashSet<_>, _>>()?;
            Ok(set)
        })
        .await
        .map_err(db_fail)
}

/// `processContactBatch` of the trigger evaluator: every condition must match,
/// with the values re-read for the candidate contacts and fields.
async fn trigger_batch(
    deps: &Deps,
    entities: &[Entity],
    rows: &[FieldRow],
    executed_keys: &mut HashSet<(i64, i64)>,
    start_of_minute: i64,
    stats: &mut Stats,
) -> Result<(), JobError> {
    let mut candidates: Vec<(&Entity, Vec<i64>)> = Vec::new();
    let (mut contact_ids, mut field_ids) = (HashSet::new(), HashSet::new());
    for entity in entities {
        let contacts = contacts_to_check(rows, entity);
        if contacts.is_empty() {
            continue;
        }
        contact_ids.extend(contacts.iter().copied());
        field_ids.extend(
            entity
                .conditions
                .iter()
                .filter_map(|c| c.field.parse::<i64>().ok()),
        );
        candidates.push((entity, contacts));
    }
    if contact_ids.is_empty() || field_ids.is_empty() {
        return Ok(());
    }
    let contact_list: Vec<i64> = contact_ids.into_iter().collect();
    let trigger_ids = candidates.iter().map(|(e, _)| e.id).collect();
    let executed = executed_pairs(deps, Kind::Trigger, trigger_ids, contact_list.clone()).await?;
    let (contacts, fields) = (
        json!(contact_list).to_string(),
        json!(field_ids.into_iter().collect::<Vec<_>>()).to_string(),
    );
    let hydrated = deps
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT ccf."contactId", ccf."customFieldId", c."workspaceId", ccf."value"
                   FROM "ContactCustomField" ccf JOIN "Contact" c ON c."id" = ccf."contactId"
                   WHERE ccf."contactId" IN (SELECT value FROM json_each(?1))
                     AND ccf."customFieldId" IN (SELECT value FROM json_each(?2))"#,
            )?;
            let rows = stmt
                .query_map(params![contacts, fields], |r| {
                    Ok(FieldRow {
                        contact_id: r.get(0)?,
                        field: r.get::<_, i64>(1)?.to_string(),
                        workspace_id: r.get(2)?,
                        value: r.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(db_fail)?;
    let values = value_map(&hydrated);
    for (entity, contacts) in candidates {
        for contact_id in contacts {
            let key = (entity.id, contact_id);
            if executed_keys.contains(&key) || executed.contains(&key) {
                continue;
            }
            let Some(fields) = values.get(&contact_id) else {
                continue;
            };
            let all = entity.conditions.iter().all(|c| {
                matches(
                    fields.get(&c.field).map(String::as_str),
                    c,
                    &entity.timezone,
                    start_of_minute,
                )
            });
            if all {
                let occurrence = scheduled_occurrence(entity, fields);
                record(
                    stats,
                    run_once(
                        deps,
                        Kind::Trigger,
                        entity,
                        contact_id,
                        &Run::Actions(&occurrence),
                    )
                    .await,
                );
                executed_keys.insert(key);
            }
        }
    }
    Ok(())
}

/// `processContactBatch` of the webhook evaluator: the first matching
/// condition (page values only, like the original).
async fn webhook_batch(
    deps: &Deps,
    entities: &[Entity],
    rows: &[FieldRow],
    start_of_minute: i64,
    stats: &mut Stats,
) -> Result<(), JobError> {
    let contact_ids: Vec<i64> = rows
        .iter()
        .map(|r| r.contact_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let webhook_ids = entities.iter().map(|e| e.id).collect();
    let mut executed = executed_pairs(deps, Kind::Webhook, webhook_ids, contact_ids).await?;
    let values = value_map(rows);
    for entity in entities {
        for contact_id in contacts_to_check(rows, entity) {
            let key = (entity.id, contact_id);
            if executed.contains(&key) {
                continue;
            }
            let Some(fields) = values.get(&contact_id) else {
                continue;
            };
            let matched = entity.conditions.iter().find(|c| {
                matches(
                    fields.get(&c.field).map(String::as_str),
                    c,
                    &entity.timezone,
                    start_of_minute,
                )
            });
            if let Some(condition) = matched {
                record(
                    stats,
                    run_once(
                        deps,
                        Kind::Webhook,
                        entity,
                        contact_id,
                        &Run::Enqueue(&condition.field),
                    )
                    .await,
                );
                executed.insert(key);
            }
        }
    }
    Ok(())
}

fn record(stats: &mut Stats, result: Result<bool, String>) {
    stats.total += 1;
    match result {
        Ok(true) => stats.matched += 1,
        Ok(false) => {}
        Err(_) => stats.failed += 1,
    }
}

/// `scheduledOccurrence`: a hash of the sorted `fieldId=value` pairs, so a
/// changed date is a new occurrence for the actions' idempotency keys.
fn scheduled_occurrence(entity: &Entity, values: &HashMap<String, String>) -> String {
    let mut ids: Vec<&str> = entity.conditions.iter().map(|c| c.field.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    let parts: Vec<String> = ids
        .into_iter()
        .map(|id| {
            format!(
                "{id}={}",
                values.get(id).map_or("undefined", String::as_str)
            )
        })
        .collect();
    sha256_hex(&parts.join("|"))[..24].to_owned()
}

enum Run<'a> {
    /// `executeActions` with the occurrence hash.
    Actions(&'a str),
    /// `enqueueWebhookEvaluation` for the matched field.
    Enqueue(&'a str),
}

/// `executeAndMarkTrigger` / `enqueueAndMarkWebhook`: `Ok(false)` = already
/// run or locked by another scan.
async fn run_once(
    deps: &Deps,
    kind: Kind,
    entity: &Entity,
    contact_id: i64,
    run: &Run<'_>,
) -> Result<bool, String> {
    let kv = crate::inbound::kv(deps);
    let prefix = kind.prefix();
    let cache = format!("{prefix}:executed:{}:{contact_id}", entity.id);
    let lock = format!("{prefix}:lock:{}:{contact_id}", entity.id);
    if kv.exists(&cache).await.map_err(kv_fail)? {
        return Ok(false);
    }
    if !kv
        .set_number_if_not_exists(&lock, 1.0, LOCK_TTL_SECS)
        .await
        .map_err(kv_fail)?
    {
        return Ok(false);
    }
    let result = async {
        if kv.exists(&cache).await.map_err(kv_fail)? {
            return Ok(false);
        }
        match run {
            Run::Actions(occurrence) => {
                crate::trigger::run_datetime_actions(
                    deps,
                    entity.id,
                    entity.workspace_id,
                    contact_id,
                    &entity.actions,
                    occurrence,
                )
                .await
            }
            Run::Enqueue(field) => {
                deps.jobs
                    .add(
                        QueueName::Webhook,
                        "evaluate-webhooks",
                        json!({ "type": "evaluateWebhooks", "data": {
                            "workspaceId": entity.workspace_id.to_string(),
                            "contactId": contact_id.to_string(),
                            "eventType": "dateTimeBasedTrigger",
                            "eventData": { "sourceId": field },
                            "timestamp": iso(now_ms()),
                        } }),
                        JobOptions {
                            remove_on_complete: Some(Retention::Remove),
                            remove_on_fail: Some(Retention::KeepCount(100)),
                            ..JobOptions::default()
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        let (table, column) = match kind {
            Kind::Trigger => ("TriggerExecution", "triggerId"),
            Kind::Webhook => ("WebhookExecution", "webhookId"),
        };
        let (id, ws) = (entity.id, entity.workspace_id);
        deps.db
            .write(move |tx| {
                Ok(tx.execute(
                    &format!(
                        r#"INSERT INTO "{table}" ("id", "{column}", "contactId", "workspaceId") VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING"#
                    ),
                    params![new_id(), id, contact_id, ws],
                )?)
            })
            .await
            .map_err(|e| e.to_string())?;
        kv.set_number(&cache, 1.0, EXECUTED_TTL_SECS).await.map_err(kv_fail)?;
        Ok(true)
    }
    .await;
    if let Err(err) = kv.del(&lock).await {
        tracing::warn!(err = %err, "Failed to release the datetime execution lock");
    }
    if let Err(err) = &result {
        tracing::warn!(err = %err, id = entity.id, contact_id, "Failed to run a datetime {prefix}");
    }
    result
}

/// `matchesDateTimeCondition(parseDateTimeValue(value, zone), …)` with the
/// condition zone, else the workspace zone.
pub(crate) fn matches(
    value: Option<&str>,
    condition: &DtCondition,
    workspace_timezone: &str,
    start_of_minute: i64,
) -> bool {
    let timezone = condition
        .timezone
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or(workspace_timezone);
    let Some(date) =
        value.and_then(|v| parse_date_time_value(&Value::String(v.to_owned()), Some(timezone)))
    else {
        return false;
    };
    if date.is_nan() {
        return false;
    }
    let now = wall_ms(start_of_minute, Some(timezone)) as f64;
    match condition.trigger_type.as_str() {
        "before" | "after" => {
            let amount = match &condition.time_value {
                Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
                Some(Value::String(s)) if !s.is_empty() => js::to_number(s),
                _ => 0.0,
            };
            let Some(unit) = condition.time_type.as_deref().filter(|u| !u.is_empty()) else {
                return false;
            };
            if amount == 0.0 || amount.is_nan() {
                return false;
            }
            let signed = if condition.trigger_type == "before" {
                -amount.abs()
            } else {
                amount.abs()
            };
            // date-fns: hours are added as milliseconds, days with `setDate`.
            let target = match unit {
                "minutes" => (date + signed / 60.0 * 3_600_000.0).trunc(),
                "hours" => (date + signed * 3_600_000.0).trunc(),
                _ => {
                    let day = f64::from(js::utc_fields(date as i64).day);
                    date + ((day + signed).trunc() - day) * DAY_MS as f64
                }
            };
            let tolerance = match unit {
                "hours" => 5.0,
                "days" => 30.0,
                _ => 1.0,
            };
            (now - target) / 60_000.0 <= tolerance
        }
        "atTheDayOf" => {
            let (d, n) = (js::utc_fields(date as i64), js::utc_fields(now as i64));
            let at = match &condition.at {
                None | Some(Value::Null) => d.hour.to_string(),
                Some(Value::String(s)) if s.is_empty() => d.hour.to_string(),
                Some(Value::String(s)) => s.clone(),
                // `condition.at || ""`: a falsy non-empty value is no hour.
                Some(Value::Bool(false)) => String::new(),
                Some(Value::Number(x)) if x.as_f64() == Some(0.0) => String::new(),
                Some(v) => js::string_of(v),
            };
            let same_day = n.year == d.year && n.month == d.month && n.day == d.day;
            same_day && js::parse_int(&at).is_some_and(|h| h == f64::from(n.hour))
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "datetime_sweep_tests.rs"]
mod tests;
