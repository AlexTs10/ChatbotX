//! `packages/business/src/whatsapp-flow-response/{service,field-values,
//! resolve-sent-flow}.ts` (Q.P7-V4): a submitted WhatsApp Flow counts as a
//! completion (with the contact's `formCompleted` activity) and its mapped
//! answers are written to the contact's custom fields; a template FLOW
//! button's token resolves to the button it was sent with.

use std::collections::HashMap;

use chatbotx_core::flow_config::steps::{
    WHATSAPP_FLOW_FILE_ANSWER_FIELD_TYPES, find_send_wa_template_step,
};
use chatbotx_core::js;
use chatbotx_core::utils::datetime::{SourceTimezoneStrategy, is_temporal_custom_field_type};
use chatbotx_core::utils::form_answer_format::format_date_range;
use chatbotx_db::Db;
use chatbotx_db::repo::contact_activity::{ContactActivityRecord, upsert_activities};
use chatbotx_db::repo::partials::broadcast::resolve_broadcast_template_send;
use chatbotx_db::repo::small_repos::whatsapp_flow_increment_completed_count;
use chatbotx_db::repo::workspace::lock_workspace_for_child_write;
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Map, Value};

use crate::contact_custom_field::{SetValuesInput, SourceTimezoneResolver, set_values};
use crate::errors::Result;
use crate::tx;

/// `ApplyWhatsappFlowResponseInput`.
#[derive(Debug, Clone, Default)]
pub struct ApplyInput {
    pub workspace_id: i64,
    pub contact_id: i64,
    /// The contact inbox's inbox (looks the integration up when
    /// `integration_whatsapp_id` is `None`).
    pub inbox_id: Option<i64>,
    pub integration_whatsapp_id: Option<i64>,
    pub flow_source_id: String,
    /// `WhatsappFlowFieldMapping[]`.
    pub field_mappings: Vec<Value>,
    pub flow_response: Map<String, Value>,
    /// The stored files of the file answers, by response key.
    pub flow_files: Option<Map<String, Value>>,
}

/// `whatsappFlowResponseService.applyResponse`.
pub async fn apply_response(db: &Db, input: ApplyInput) -> Result<()> {
    let integration_id = match (input.integration_whatsapp_id, input.inbox_id) {
        (Some(id), _) => Some(id),
        (None, Some(inbox_id)) => {
            tx::read(db, move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "id" FROM "IntegrationWhatsapp" WHERE "inboxId" = ?1"#,
                        [inbox_id],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(repo::Error::from)?)
            })
            .await?
        }
        (None, None) => None,
    };
    let Some(integration_id) = integration_id else {
        tracing::warn!(
            inbox_id = ?input.inbox_id,
            "IntegrationWhatsapp not found while applying WhatsApp flow response"
        );
        return Ok(());
    };
    let (ws, contact, source) = (
        input.workspace_id,
        input.contact_id,
        input.flow_source_id.clone(),
    );
    tx::write(db, move |conn| {
        record_completion(conn, ws, contact, integration_id, &source)
    })
    .await?;
    apply_field_mappings(db, &input).await
}

/// `recordCompletion`: the count and the idempotent `formCompleted`
/// activity in one transaction.
fn record_completion(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    integration_whatsapp_id: i64,
    source_id: &str,
) -> Result<()> {
    let Some(flow_id) =
        whatsapp_flow_increment_completed_count(conn, integration_whatsapp_id, source_id)?
    else {
        return Ok(());
    };
    if lock_workspace_for_child_write(conn, workspace_id)? {
        upsert_activities(
            conn,
            workspace_id,
            &[ContactActivityRecord {
                activity_type: "formCompleted".into(),
                contact_id,
                occurred_at: now_ms(),
                parent_reference_id: 0,
                reference_id: flow_id,
                reference_type: "whatsappFlow".into(),
            }],
        )?;
    }
    Ok(())
}

/// `MappedValue`.
#[derive(Debug, Clone, PartialEq)]
struct Mapped {
    custom_field_id: i64,
    value: String,
    is_file_answer: bool,
}

const FILE_URL_SEPARATOR: &str = ", ";

/// `toMappedValue`: a file answer's kept URLs, else the serialized answer.
fn to_mapped_value(mapping: &Value, custom_field_id: i64, input: &ApplyInput) -> Option<Mapped> {
    let key = mapping["paramKey"].as_str().unwrap_or_default();
    let file_format = mapping["fileFormat"].as_str();
    if let Some(files) = input.flow_files.as_ref().and_then(|f| f.get(key)) {
        let files = files.as_array().map(Vec::as_slice).unwrap_or_default();
        let kept = if file_format == Some("firstUrl") {
            &files[..files.len().min(1)]
        } else {
            files
        };
        let urls: Vec<&str> = kept.iter().filter_map(|f| f["url"].as_str()).collect();
        return (!urls.is_empty()).then(|| Mapped {
            custom_field_id,
            value: urls.join(FILE_URL_SEPARATOR),
            is_file_answer: true,
        });
    }
    let raw = input.flow_response.get(key);
    // A file answer the customer left empty keeps the field as it was.
    if file_format.is_some() && raw.and_then(Value::as_array).is_some_and(Vec::is_empty) {
        return None;
    }
    serialize_flow_value(raw).map(|value| Mapped {
        custom_field_id,
        value,
        is_file_answer: false,
    })
}

/// `serializeFlowValue`.
pub fn serialize_flow_value(raw: Option<&Value>) -> Option<String> {
    match raw? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(js::number_to_string(n.as_f64().unwrap_or_default())),
        Value::Bool(b) => Some(b.to_string()),
        other => format_date_range(other).or_else(|| Some(js::json_stringify(other))),
    }
}

fn whole_number(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

fn calendar_date(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// `MAX_TIMESTAMP_MS`: JavaScript's date range.
const MAX_TIMESTAMP_MS: f64 = 8.64e15;

fn millisecond_timestamp(value: &str) -> Option<i64> {
    if !whole_number(value) {
        return None;
    }
    let ms: f64 = value.parse().ok()?;
    (ms.abs() <= MAX_TIMESTAMP_MS).then_some(ms as i64)
}

/// `dependsOnFieldType`.
fn depends_on_field_type(value: &Mapped) -> bool {
    value.is_file_answer || whole_number(&value.value) || calendar_date(&value.value)
}

/// `toTemporalFieldValue`: a legacy UTC millisecond timestamp is the
/// workspace's day in a `date` field and the instant in a `datetime` field;
/// a calendar date in a `datetime` field is local midnight.
fn to_temporal_field_value(
    value: &str,
    kind: &str,
    zone: &mut dyn FnMut() -> Result<String>,
) -> Result<String> {
    if let Some(ms) = millisecond_timestamp(value) {
        if kind == "date" {
            let zone: chrono_tz::Tz = zone()?.parse().unwrap_or(chrono_tz::UTC);
            let instant = chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default();
            return Ok(instant.with_timezone(&zone).format("%Y-%m-%d").to_string());
        }
        return Ok(js::to_iso_string(ms).unwrap_or_else(|| value.to_owned()));
    }
    if kind == "datetime" && calendar_date(value) {
        return Ok(format!("{value}T00:00:00"));
    }
    Ok(value.to_owned())
}

/// `applyFieldMappings` + `fitValuesToFieldTypes`: file links only fit a
/// text field (others are skipped); a date answer takes a date field's shape.
async fn apply_field_mappings(db: &Db, input: &ApplyInput) -> Result<()> {
    let values: Vec<Mapped> = input
        .field_mappings
        .iter()
        .filter_map(|mapping| {
            let id = js::string_of(&mapping["customFieldId"]).parse().ok()?;
            to_mapped_value(mapping, id, input)
        })
        .collect();
    let mut ids: Vec<i64> = values
        .iter()
        .filter(|v| depends_on_field_type(v))
        .map(|v| v.custom_field_id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let (ws, contact) = (input.workspace_id, input.contact_id);
    let fields = if ids.is_empty() {
        values
    } else {
        tx::read(db, move |conn| {
            let types = field_types(conn, ws, &ids)?;
            let mut resolver =
                SourceTimezoneResolver::new(ws, contact, SourceTimezoneStrategy::Workspace, None);
            let mut out = Vec::new();
            for value in values {
                let kind = types.get(&value.custom_field_id).map(String::as_str);
                if value.is_file_answer {
                    if kind.is_some_and(|k| WHATSAPP_FLOW_FILE_ANSWER_FIELD_TYPES.contains(&k)) {
                        out.push(value);
                    } else {
                        tracing::warn!(
                            workspace_id = ws,
                            custom_field_id = value.custom_field_id,
                            "Skipped WhatsApp Flow file links mapped to a non-text custom field"
                        );
                    }
                    continue;
                }
                match kind.filter(|k| is_temporal_custom_field_type(k)) {
                    Some(kind) => {
                        let fitted = to_temporal_field_value(&value.value, kind, &mut || {
                            resolver.resolve(conn)
                        })?;
                        out.push(Mapped {
                            value: fitted,
                            ..value
                        });
                    }
                    None => out.push(value),
                }
            }
            Ok(out)
        })
        .await?
    };
    if fields.is_empty() {
        return Ok(());
    }
    set_values(
        db,
        SetValuesInput {
            workspace_id: ws,
            contact_id: contact,
            fields: fields
                .into_iter()
                .map(|f| (f.custom_field_id, f.value))
                .collect(),
            ..SetValuesInput::default()
        },
    )
    .await?;
    Ok(())
}

/// `customFieldService.findManyByIds`: `id → type` of the workspace's fields.
fn field_types(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<HashMap<i64, String>> {
    let list = serde_json::to_string(ids).unwrap_or_default();
    let mut stmt = conn
        .prepare(
            r#"SELECT "id", "type" FROM "CustomField" WHERE "workspaceId" = ?1
               AND "id" IN (SELECT value FROM json_each(?2))"#,
        )
        .map_err(repo::Error::from)?;
    let rows = stmt
        .query_map(params![workspace_id, list], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(repo::Error::from)?
        .collect::<chatbotx_db::rusqlite::Result<HashMap<i64, String>>>()
        .map_err(repo::Error::from)?;
    Ok(rows)
}

// ---- resolveTemplateFlowButton ----------------------------------------------

/// `ResolvedTemplateFlowButton`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedButton {
    pub integration_whatsapp_id: Option<i64>,
    pub flow_source_id: String,
    /// `WaTemplateButtonParam`.
    pub param: Value,
}

/// `resolveButtonParam`: the card's button when the token names a card.
fn button_param(params: &Value, token: &Value) -> Option<Value> {
    let index = token["buttonIndex"].as_u64()? as usize;
    let buttons = match token["cardIndex"].as_u64() {
        Some(card) => params["carousel"]
            .as_array()?
            .iter()
            .find(|c| c["card_index"].as_u64() == Some(card))?
            .get("button")?,
        None => params.get("button")?,
    };
    buttons.as_array()?.get(index).cloned()
}

fn flow_button(param: Option<Value>) -> Option<(String, Value)> {
    let param = param.filter(|p| p["sub_type"] == "flow")?;
    let source = param["flowSourceId"]
        .as_str()
        .filter(|s| !s.is_empty())?
        .to_owned();
    Some((source, param))
}

fn id(value: &Value) -> Option<i64> {
    js::string_of(value).parse().ok()
}

/// `findFlowVersion`: the pinned version, else the active flow's current one.
fn flow_version_nodes(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
    version_id: Option<i64>,
) -> Result<Option<Vec<Value>>> {
    let version_id = match version_id {
        Some(id) => Some(id),
        None => conn
            .query_row(
                r#"SELECT "currentVersionId" FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1"#,
                params![flow_id, workspace_id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(repo::Error::from)?
            .flatten(),
    };
    let Some(version_id) = version_id else {
        return Ok(None);
    };
    let nodes: Option<String> = conn
        .query_row(
            r#"SELECT "nodes" FROM "FlowVersion" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![version_id, workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(repo::Error::from)?;
    Ok(nodes.and_then(|n| serde_json::from_str(&n).ok()))
}

/// `resolveTemplateFlowButton`: the FLOW button of the broadcast (per page)
/// or flow step template a decoded template Flow token was sent from.
pub fn resolve_template_flow_button(
    conn: &Connection,
    token: &Value,
    workspace_id: i64,
    inbox_id: i64,
) -> Result<Option<ResolvedButton>> {
    if token["origin"] == "b" {
        let Some(broadcast_id) = id(&token["broadcastId"]) else {
            return Ok(None);
        };
        let row = repo::find_one::<chatbotx_db::repo::models::Broadcast>(
            conn,
            r#""id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
            &[&broadcast_id, &workspace_id],
        )?;
        let Some(row) = row else {
            tracing::warn!(
                workspace_id,
                broadcast_id,
                "[template-flow-response] broadcast not found"
            );
            return Ok(None);
        };
        let integration = row.integration_whatsapp_id;
        let json = crate::broadcast::send::with_targets(conn, row)?.json;
        let params = resolve_broadcast_template_send(&json, &inbox_id.to_string())
            .map(|send| send.template_data)
            .unwrap_or(Value::Null);
        let Some((flow_source_id, param)) = flow_button(button_param(&params, token)) else {
            tracing::warn!(
                workspace_id,
                broadcast_id,
                "[template-flow-response] broadcast FLOW button params not found"
            );
            return Ok(None);
        };
        return Ok(Some(ResolvedButton {
            integration_whatsapp_id: integration,
            flow_source_id,
            param,
        }));
    }
    let (Some(flow_id), Some(step_id)) = (id(&token["flowId"]), token["stepId"].as_str()) else {
        return Ok(None);
    };
    let version_id = id(&token["flowVersionId"]);
    let Some(nodes) = flow_version_nodes(conn, workspace_id, flow_id, version_id)? else {
        tracing::warn!(
            workspace_id,
            flow_id,
            "[template-flow-response] flow version not found"
        );
        return Ok(None);
    };
    let step = find_send_wa_template_step(&nodes, step_id);
    let params = step
        .as_ref()
        .map_or(Value::Null, |s| s["template"]["params"].clone());
    let Some((flow_source_id, param)) = flow_button(button_param(&params, token)) else {
        tracing::warn!(
            workspace_id,
            flow_id,
            step_id,
            "[template-flow-response] flow-step FLOW button params not found"
        );
        return Ok(None);
    };
    Ok(Some(ResolvedButton {
        integration_whatsapp_id: None,
        flow_source_id,
        param,
    }))
}

/// `SentWhatsappFlow`.
#[derive(Debug, Clone, PartialEq)]
pub struct SentFlow {
    pub flow_source_id: String,
    pub integration_whatsapp_id: Option<i64>,
}

/// `resolveSentWhatsappFlow`: the WhatsApp Flow a response's `flow_token`
/// was sent with (a template FLOW button's token, or a "Send WhatsApp Flow"
/// step's button payload whose `flow.id` wins over a stale `sourceId`);
/// `None` for a token this workspace did not issue.
pub fn resolve_sent_whatsapp_flow(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    flow_token: Option<&str>,
) -> Result<Option<SentFlow>> {
    let Some(flow_token) = flow_token.filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if let Some(token) = chatbotx_core::flow_config::steps::decode_template_flow_token(Some(flow_token)) {
        return Ok(
            resolve_template_flow_button(conn, &token, workspace_id, inbox_id)?.map(|r| SentFlow {
                flow_source_id: r.flow_source_id,
                integration_whatsapp_id: r.integration_whatsapp_id,
            }),
        );
    }
    let Some(payload) = chatbotx_core::flow_config::util::decode_button_payload(flow_token, false)
    else {
        return Ok(None);
    };
    let (Ok(flow_id), Some(button_id)) = (payload.flow_id.parse::<i64>(), payload.button_id) else {
        return Ok(None);
    };
    let version_id = payload.flow_version_id.and_then(|v| v.parse().ok());
    let Some(nodes) = flow_version_nodes(conn, workspace_id, flow_id, version_id)? else {
        return Ok(None);
    };
    let Some(step) =
        chatbotx_core::flow_config::steps::find_whatsapp_flow_step_by_button_id(&nodes, &button_id)
    else {
        return Ok(None);
    };
    let current = match id(&step["flow"]["id"]) {
        Some(flow_row) => conn
            .query_row(
                r#"SELECT f."sourceId" FROM "WhatsappFlow" f JOIN "IntegrationWhatsapp" i ON i."id" = f."integrationWhatsappId"
                   WHERE f."id" = ?1 AND i."workspaceId" = ?2"#,
                params![flow_row, workspace_id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(repo::Error::from)?
            .filter(|s| !s.is_empty()),
        None => None,
    };
    let source = current.or_else(|| {
        step["flow"]["sourceId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    });
    Ok(source.map(|flow_source_id| SentFlow {
        flow_source_id,
        integration_whatsapp_id: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_shapes() {
        assert!(whole_number("-12") && !whole_number("-") && !whole_number("1.5"));
        assert!(calendar_date("2024-10-21") && !calendar_date("2024-1-021"));
        assert_eq!(millisecond_timestamp(&"9".repeat(17)), None);
    }
}
