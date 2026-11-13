//! `apps/builder/src/features/contact-filter/lib/list-contact-filter-fields.ts`
//! (`listContactFilterFieldsForAPI`) with the parts of
//! `contact-filter/schema/definitions.ts` (`CONTACT_FILTER_FIELD_DEFINITIONS`:
//! `schemaKind`, `optionSource`, `hidden`, `surfaces`),
//! `custom-field-filter.ts` (`convertCustomFieldTypeToConditionType`) and
//! `pipeline-item-filter.ts` (`PIPELINE_ITEM_ATTRIBUTE_DESCRIPTORS`) it reads
//! (P3.6-D2 slice filter fields).

use chatbotx_db::Db;
use chatbotx_db::repo::partials::contact_activity::{ValueFormat, contact_activity_filter};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use crate::contact_filter::{Surface, enabled_operators_for_static_field};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// One `CONTACT_FILTER_FIELD_DEFINITIONS` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticFieldDefinition {
    pub field: &'static str,
    pub schema_kind: &'static str,
    pub option_source: &'static str,
    /// Valid in saved filters, not offered for new conditions.
    pub hidden: bool,
    /// `surfaces: ["flowCondition"]`.
    pub flow_condition_only: bool,
}

const fn def(
    field: &'static str,
    schema_kind: &'static str,
    option_source: &'static str,
) -> StaticFieldDefinition {
    StaticFieldDefinition {
        field,
        schema_kind,
        option_source,
        hidden: false,
        flow_condition_only: false,
    }
}

const fn hidden(
    field: &'static str,
    schema_kind: &'static str,
    option_source: &'static str,
) -> StaticFieldDefinition {
    StaticFieldDefinition {
        hidden: true,
        ..def(field, schema_kind, option_source)
    }
}

/// `CONTACT_FILTER_FIELD_DEFINITIONS`, in order.
pub const CONTACT_FILTER_FIELD_DEFINITIONS: &[StaticFieldDefinition] = &[
    hidden("locale", "multiSelect", "languages"),
    def("language", "multiSelect", "languages"),
    def("fullName", "text", "none"),
    def("country", "multiSelect", "countries"),
    def("continent", "multiSelect", "continents"),
    def("gender", "select", "gender"),
    def("subscribedToBroadcast", "boolean", "none"),
    def("contactCreatedAt", "datetime", "none"),
    def("contactCreatedDateMinutesAgo", "number", "none"),
    def("source", "multiSelect", "contactSources"),
    def("interactedInLast24h", "boolean", "none"),
    def("conversationTransferredToHuman", "boolean", "none"),
    def("followUp", "boolean", "none"),
    def("archived", "boolean", "none"),
    def("blocked", "boolean", "none"),
    hidden("existingContact", "boolean", "none"),
    def("hasContactInfo", "multiSelect", "contactInfoFilterValues"),
    def("currentChannel", "multiSelect", "channels"),
    def("inbox", "multiSelect", "inboxes"),
    def("timezone", "multiSelect", "timezones"),
    def("followsBusinessOnInstagram", "boolean", "none"),
    def("businessFollowsUserOnInstagram", "boolean", "none"),
    def("verifiedAccountOnInstagram", "boolean", "none"),
    def("followerCountOnInstagram", "number", "none"),
    def("lastSent", "datetime", "none"),
    def("lastDelivered", "datetime", "none"),
    def("lastSeen", "datetime", "none"),
    def("lastSeenMinutesAgo", "number", "none"),
    def("lastInteraction", "datetime", "none"),
    def("lastInteractionMinutesAgo", "number", "none"),
    def("unreplied", "boolean", "none"),
    def("unread", "boolean", "none"),
    def("tags", "multiSelect", "tags"),
    def("subscribedToDripCampaign", "multiSelect", "sequences"),
    def("conversationAssigned", "multiSelect", "assignees"),
    def("entryPointsLinks", "multiSelect", "reflinks"),
    def("completedWhatsAppFlows", "multiSelect", "whatsappFlows"),
    def("sentMessage", "text", "none"),
    def("keywordsReceived", "multiSelect", "keywords"),
    def("executedFlow", "multiSelect", "flows"),
    def("executedStep", "multiSelect", "flowNodes"),
    def("consecutiveAiFailures", "number", "none"),
    def("questionnaireStarted", "boolean", "none"),
    def("questionnaireInProgress", "boolean", "none"),
    def("questionnaireFinished", "boolean", "none"),
    def("lastComment", "text", "none"),
    def("commentedOnPost", "multiSelect", "channelPosts"),
    def("phone", "text", "none"),
    def("broadcastSent", "multiSelect", "broadcasts"),
    def("broadcastDelivered", "multiSelect", "broadcasts"),
    def("broadcastSeen", "multiSelect", "broadcasts"),
    def("broadcastClicked", "multiSelect", "broadcasts"),
    def("broadcastFailed", "multiSelect", "broadcasts"),
    def("email", "text", "none"),
    def("emailWasVerified", "boolean", "none"),
    def("optedInForEmail", "boolean", "none"),
    def("emailSent", "multiSelect", "emailTopics"),
    def("emailDelivered", "multiSelect", "emailTopics"),
    def("emailOpened", "multiSelect", "emailTopics"),
    def("emailClicked", "multiSelect", "emailTopics"),
    def("fromCtwaAd", "boolean", "none"),
    def("fromGoogleAd", "boolean", "none"),
    def("clickedAd", "multiSelect", "ads"),
    def("ctwaConversion", "multiSelect", "ctwaConversionTypes"),
    def("totalSpent", "number", "none"),
    def("numberOfOrders", "number", "none"),
    def("bought", "datetime", "none"),
    def("boughtItems", "select", "products"),
    def("shoppingCartIsEmpty", "boolean", "none"),
    def("shoppingCartContainsItems", "select", "products"),
    def("shoppingCartSubtotal", "number", "none"),
    def("shoppingCartTotal", "number", "none"),
    StaticFieldDefinition {
        flow_condition_only: true,
        ..def("isWithinWorkingHours", "multiSelect", "workingHours")
    },
    def("lastUserInput", "text", "none"),
    def("lastUserInputType", "select", "lastUserInputTypes"),
];

/// `CONTACT_ACTIVITY_TRACKING_STARTED_AT`: activity recording went live (no backfill).
pub const CONTACT_ACTIVITY_TRACKING_STARTED_AT: &str = "2026-10-23T00:00:00.000Z";

/// `getContactActivityTrackingStartedAt`: fields whose writer shipped later.
pub fn contact_activity_tracking_started_at(field: &str) -> &'static str {
    match field {
        "clickedAd" => "2026-10-26T00:00:00.000Z",
        _ => CONTACT_ACTIVITY_TRACKING_STARTED_AT,
    }
}

/// `CONTACT_ACTIVITY_TRACKED_FIELDS`: the registry fields plus the two
/// backed by their own tables.
fn is_tracked_field(field: &str) -> bool {
    contact_activity_filter(field).is_some() || matches!(field, "sentMessage" | "lastDelivered")
}

/// `convertCustomFieldTypeToConditionType`.
pub fn convert_custom_field_type_to_condition_type(field_type: &str) -> &'static str {
    match field_type {
        "number" => "number",
        "date" | "datetime" => "datetime",
        "boolean" => "boolean",
        _ => "text",
    }
}

/// `valueSourceForField`: where an activity-backed field's values come from.
fn value_source_for_field(field: &str) -> Option<Value> {
    if !is_tracked_field(field) {
        return None;
    }
    if let Some(config) = contact_activity_filter(field) {
        return Some(json!({
            "operation": config.value_source_operation,
            "scope": config.value_source_scope,
            "fixedValues": config.fixed_values,
            "valueFormat": match config.value_format {
                ValueFormat::Id => "id",
                ValueFormat::FlowIdNodeId => "flowId:nodeId",
            },
            "trackingStartedAt": contact_activity_tracking_started_at(field),
        }));
    }
    Some(json!({
        "fixedValues": [],
        "valueFormat": if field == "lastDelivered" { "datetime" } else { "text" },
        "trackingStartedAt": CONTACT_ACTIVITY_TRACKING_STARTED_AT,
    }))
}

/// The static fields offered on `surface` (`tracked_fields_enabled` =
/// `isContactActivityFiltersEnabled()`), as `ContactFilterFieldPublicResource`s.
pub fn static_filter_fields(surface: Surface, tracked_fields_enabled: bool) -> Vec<Value> {
    CONTACT_FILTER_FIELD_DEFINITIONS
        .iter()
        .filter(|def| {
            !def.hidden
                && (!def.flow_condition_only || surface == Surface::FlowCondition)
                && (tracked_fields_enabled || !is_tracked_field(def.field))
        })
        .map(|def| {
            let mut row = Map::new();
            row.insert("field".into(), json!(def.field));
            row.insert("schemaKind".into(), json!(def.schema_kind));
            row.insert("optionSource".into(), json!(def.option_source));
            row.insert(
                "operators".into(),
                json!(enabled_operators_for_static_field(def.field)),
            );
            if let Some(source) = value_source_for_field(def.field) {
                row.insert("valueSource".into(), source);
            }
            Value::Object(row)
        })
        .collect()
}

/// `PIPELINE_ITEM_ATTRIBUTE_DESCRIPTORS` as JSON.
pub fn pipeline_item_attribute_descriptors() -> Vec<Value> {
    let membership = ["in", "notIn"];
    vec![
        json!({
            "attribute": "exists",
            "operators": ["isNotEmpty", "isEmpty"],
            "pipelineIdRequired": false,
            "valueShape": "single",
            "values": ["any", "open", "won", "lost"],
        }),
        json!({
            "attribute": "status",
            "operators": membership,
            "pipelineIdRequired": false,
            "valueShape": "list",
            "values": ["open", "won", "lost"],
        }),
        json!({
            "attribute": "stage",
            "operators": membership,
            "pipelineIdRequired": true,
            "valueShape": "list",
            "values": [],
        }),
        json!({
            "attribute": "priority",
            "operators": membership,
            "pipelineIdRequired": false,
            "valueShape": "list",
            "values": ["low", "medium", "high"],
        }),
    ]
}

/// `{id, name, type, valueType}` rows of `table` (`CustomField` / `BotField`).
fn typed_fields(
    conn: &Connection,
    sql: &str,
    workspace_id: i64,
) -> chatbotx_db::rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![workspace_id], |row| {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let kind: String = row.get(2)?;
        Ok(json!({
            "id": id.to_string(),
            "name": name,
            "valueType": convert_custom_field_type_to_condition_type(&kind),
            "type": kind,
        }))
    })?;
    rows.collect()
}

/// `listContactFilterFieldsForAPI`: `{staticFields, customFields, botFields,
/// tags, pipelineItems}` of a workspace (a workspace token sees every pipeline).
pub async fn list_contact_filter_fields_for_api(
    db: &Db,
    workspace_id: i64,
    surface: Surface,
    tracked_fields_enabled: bool,
) -> Result<Value> {
    let (custom_fields, bot_fields, tags, pipelines) = tx::read(db, move |conn| {
        let run = || -> chatbotx_db::rusqlite::Result<_> {
            // `customFieldService.list({workspaceId})`: every row, `id` desc
            // (`withIdTieBreak({})`).
            let custom_fields = typed_fields(
                conn,
                r#"SELECT "id", "name", "type" FROM "CustomField" WHERE "workspaceId" = ?1 ORDER BY "id" DESC"#,
                workspace_id,
            )?;
            // `botFieldService.list({workspaceId})`: every row, no order (id order).
            let bot_fields = typed_fields(
                conn,
                r#"SELECT "id", "name", "type" FROM "BotField" WHERE "workspaceId" = ?1 ORDER BY "id""#,
                workspace_id,
            )?;
            // `tagService.listActive`.
            let mut stmt = conn.prepare(
                r#"SELECT "id", "name" FROM "Tag" WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL ORDER BY "name", "id""#,
            )?;
            let tags = stmt
                .query_map(params![workspace_id], |row| {
                    let id: i64 = row.get(0)?;
                    let name: String = row.get(1)?;
                    Ok(json!({ "id": id.to_string(), "name": name }))
                })?
                .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()?;
            Ok((custom_fields, bot_fields, tags))
        };
        let (custom_fields, bot_fields, tags) = run().map_err(|e| BusinessError::Repo(e.into()))?;
        // Flow conditions offer no pipeline-item filter, so skip the lookup.
        let pipelines = if surface == Surface::FlowCondition {
            Vec::new()
        } else {
            let pipelines = chatbotx_db::repo::pipeline::list_by_workspace(conn, workspace_id)?;
            let ids: Vec<i64> = pipelines.iter().map(|p| p.id).collect();
            let stages =
                chatbotx_db::repo::pipeline_stage::list_by_pipeline_ids(conn, workspace_id, &ids)?;
            pipelines
                .iter()
                .map(|pipeline| {
                    let stages: Vec<Value> = stages
                        .iter()
                        .filter(|stage| stage.pipeline_id == pipeline.id)
                        .map(|stage| json!({ "id": stage.id.to_string(), "name": stage.name }))
                        .collect();
                    json!({ "id": pipeline.id.to_string(), "name": pipeline.name, "stages": stages })
                })
                .collect()
        };
        Ok((custom_fields, bot_fields, tags, pipelines))
    })
    .await?;
    let pipeline_items = if surface == Surface::FlowCondition {
        json!({ "pipelines": [], "attributes": [] })
    } else {
        json!({ "pipelines": pipelines, "attributes": pipeline_item_attribute_descriptors() })
    };
    Ok(json!({
        "staticFields": static_filter_fields(surface, tracked_fields_enabled),
        "customFields": custom_fields,
        "botFields": bot_fields,
        "tags": tags,
        "pipelineItems": pipeline_items,
    }))
}
