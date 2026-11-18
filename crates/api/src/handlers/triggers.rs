//! `apps/builder/src/features/triggers/api/public.ts` (Q.D5) and the write
//! schemas: `conditions/schema` (`allConditions`) and
//! `triggers/components/actions/schema` (`allActions`), on the zod port.

use chatbotx_business::trigger::{self as service, ConditionInput, trigger_json};
use chatbotx_core::flow_config::field_reference::zod_field_reference;
use chatbotx_core::flow_config::steps::{
    google_ads_conversion_fields_schema, meta_capi_event_fields_schema, pipeline_action_schema,
    spreadsheet_column_filter_schema, spreadsheet_contact_to_sheet_mapping_schema,
    spreadsheet_sheet_to_contact_mapping_schema, start_external_step_action_schema,
    with_field_operation_operand_rule, with_google_ads_conversion_refinements,
    with_meta_capi_event_refinements,
};
use chatbotx_core::flow_config::z::{self, RefineCtx, Schema, path};
use serde_json::{Value, json};

use super::input::Input;
use super::qr_codes::public_paging;
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "triggers.list",
    "triggers.get",
    "triggers.create",
    "triggers.update",
    "triggers.updateSettings",
    "triggers.delete",
];

/// `MAX_SEQUENCES_PER_TRIGGER_ACTION`.
const MAX_SEQUENCES_PER_TRIGGER_ACTION: usize = 50;

/// `zodBigintAsString()`.
fn bigint() -> Schema {
    z::string().regex(r"^\d+$", "")
}

/// `zodBigintIdAsString()`.
fn bigint_id() -> Schema {
    bigint().max(18)
}

const CONDITION_ID: &str = "Existing condition id (numeric string) when keeping a condition returned by `triggers.get`; omit for a new condition.";

/// `union([zodBigintIdAsString(), literal("")]).nullish().transform(id => id || null)`.
fn optional_ref() -> Schema {
    z::union(vec![bigint_id(), z::literal(json!(""))])
        .nullish()
        // `(value) => value || null`: an omitted key also becomes `null`.
        .transform_opt(|value, _| match value {
            Some(Value::String(text)) if !text.is_empty() => Some(Value::String(text)),
            _ => Some(Value::Null),
        })
}

fn simple(kind: &str) -> Schema {
    z::object(vec![
        ("id", bigint().optional().describe(CONDITION_ID)),
        ("type", z::literal(json!(kind))),
    ])
}

fn with_source(kind: &str) -> Schema {
    z::object(vec![
        ("id", bigint().optional().describe(CONDITION_ID)),
        ("type", z::literal(json!(kind))),
        ("sourceId", z::string().min(1).msg("Required")),
    ])
}

fn with_optional_ref(kind: &str) -> Schema {
    z::object(vec![
        ("id", bigint().optional().describe(CONDITION_ID)),
        ("type", z::literal(json!(kind))),
        ("sourceId", optional_ref()),
    ])
}

fn string_source(kind: &str, source: Schema) -> Schema {
    z::object(vec![
        ("id", z::string().optional().describe(CONDITION_ID)),
        ("type", z::literal(json!(kind))),
        ("sourceId", source),
    ])
}

/// `dateTimeBasedTrigger`'s `superRefine`.
fn date_time_rules(data: &Value, ctx: &mut RefineCtx) {
    if data["sourceId"].as_str().unwrap_or("").is_empty() {
        ctx.issue(path!["sourceId"], "Custom field is required");
    }
    let value = &data["value"];
    if value.is_null() {
        ctx.issue(path!["value"], "Value configuration is required");
        return;
    }
    if value["triggerType"] == json!("atTheDayOf") {
        if value["at"].as_str().unwrap_or("").is_empty() {
            ctx.issue(
                path!["value", "at"],
                "The \"at\" field is required when triggerType is \"atTheDayOf\"",
            );
        }
    } else {
        let time_value = value["timeValue"].as_f64().is_some_and(|n| n != 0.0);
        let time_type = value["timeType"].as_str().is_some_and(|t| !t.is_empty());
        if !(time_value && time_type) {
            ctx.issue(
                path![],
                "The \"timeValue\" and \"timeType\" fields are required when triggerType is \"before\" or \"after\"",
            );
        }
    }
}

/// `allConditions` (`features/conditions/schema/index.ts`), in its order.
pub fn all_conditions() -> Vec<Schema> {
    let mut out = vec![
        string_source("tagApplied", z::string()),
        string_source("tagRemoved", z::string()),
        z::object(vec![
            ("id", z::string().optional().describe(CONDITION_ID)),
            ("type", z::literal(json!("customFieldValueChanged"))),
            (
                "sourceId",
                z::string().min(1).msg("Custom field is required"),
            ),
            ("operator", z::string()),
            ("value", z::unknown()),
        ]),
        string_source("contactInfoUpdated", z::enum_(&["phone", "email"])),
        z::object(vec![
            ("id", z::string().optional().describe(CONDITION_ID)),
            ("type", z::literal(json!("dateTimeBasedTrigger"))),
            ("sourceId", z::string().optional()),
            ("operator", z::string()),
            (
                "value",
                z::object(vec![
                    ("triggerType", z::enum_(&["atTheDayOf", "before", "after"])),
                    ("timeValue", z::number().coerce().gte(1.0).optional()),
                    (
                        "timeType",
                        z::enum_(&["minutes", "hours", "days"]).optional(),
                    ),
                    ("at", z::string().optional()),
                    ("timezone", z::string().max(64).optional()),
                ]),
            ),
        ])
        .super_refine(date_time_rules),
    ];
    for kind in [
        "conversationTransferredToHuman",
        "conversationTransferredToBot",
        "newContact",
        "contactUnsubscribedFormBroadcast",
        "archived",
        "followUp",
        "conversationAssigned",
        "conversationUnassigned",
    ] {
        out.push(simple(kind));
    }
    out.push(with_source("subscribedToSequence"));
    out.push(with_source("unsubscribedFromSequence"));
    for kind in [
        "contactReferredANewContact",
        "contactReferredExistingContact",
        "contactBlockedBot",
        "contactUnblockedBot",
        "incomingCall",
        "missedAudioCall",
        "callEnded",
        "callRecorded",
        "callTranscribed",
    ] {
        out.push(simple(kind));
    }
    out.push(with_optional_ref("shoppingCartSent"));
    out.push(with_optional_ref("userAskedAboutProduct"));
    for kind in [
        "newOrder",
        "orderAccepted",
        "orderShipped",
        "orderConcluded",
        "orderCancelled",
        "orderPaid",
        "orderRefunded",
    ] {
        out.push(simple(kind));
    }
    for kind in [
        "productOrdered",
        "productAddedToCart",
        "productRemovedFromCart",
        "categoryAddedToCart",
    ] {
        out.push(with_optional_ref(kind));
    }
    out.push(simple("cartAbandoned"));
    let pipeline = |kind: &str| {
        z::object(vec![
            ("id", bigint().optional().describe(CONDITION_ID)),
            ("type", z::literal(json!(kind))),
            ("sourceId", optional_ref()),
        ])
    };
    out.push(pipeline("pipelineItemCreated"));
    out.push(pipeline("pipelineItemMovedToStage").extend(vec![(
        "value",
        z::object(vec![("stageId", optional_ref())]).nullish(),
    )]));
    for kind in [
        "pipelineItemValueChanged",
        "pipelineItemStatusChanged",
        "pipelineItemPriorityChanged",
    ] {
        out.push(pipeline(kind));
    }
    out
}

fn sequence_ids() -> Schema {
    z::array(bigint_id())
        .min(1)
        .max(MAX_SEQUENCES_PER_TRIGGER_ACTION)
        .describe("Sequence ids (numeric strings, from `sequences.list`).")
}

/// The trigger `setCustomField` action (`set-custom-field.ts`).
pub fn set_custom_field_action() -> Schema {
    with_field_operation_operand_rule(z::object(vec![
        ("type", z::literal(json!("setCustomField"))),
        ("customFieldId", zod_field_reference()),
        (
            "operation",
            z::enum_(&[
                "O01", "O02", "O03", "O04", "O05", "O06", "O07", "O08", "O09", "O10", "O11", "O12",
            ]),
        ),
        ("value", z::string().default(json!(""))),
        ("timezone", z::string().max(64).optional()),
    ]))
}

fn run_google_sheet() -> Schema {
    let base = |action: &str| {
        vec![
            ("type", z::literal(json!("runGoogleSheet"))),
            ("spreadsheetId", z::string()),
            ("sheetName", z::string()),
            ("lookup", spreadsheet_column_filter_schema()),
            ("action", z::literal(json!(action))),
        ]
    };
    let with_map = |action: &str, map: Schema| {
        let mut shape = base(action);
        shape.push(("map", z::array(map).min(1)));
        z::object(shape)
    };
    z::discriminated_union(
        "action",
        vec![
            with_map(
                "spreadsheetGetRow",
                spreadsheet_sheet_to_contact_mapping_schema(),
            ),
            with_map(
                "spreadsheetGetRandomRow",
                spreadsheet_sheet_to_contact_mapping_schema(),
            ),
            with_map(
                "spreadsheetUpdateRow",
                spreadsheet_contact_to_sheet_mapping_schema(),
            ),
            with_map(
                "spreadsheetSendData",
                spreadsheet_contact_to_sheet_mapping_schema(),
            ),
            z::object(base("spreadsheetClearRow")),
        ],
    )
}

/// The trigger `startExternalStep` action.
pub fn start_external_step_action() -> Schema {
    start_external_step_action_schema()
        .extend(vec![("type", z::literal(json!("startExternalStep")))])
}

/// The trigger `sendMetaCapiEvent` action.
pub fn send_meta_capi_event_action() -> Schema {
    with_meta_capi_event_refinements(
        meta_capi_event_fields_schema()
            .extend(vec![("type", z::literal(json!("sendMetaCapiEvent")))]),
    )
}

/// The trigger `sendGoogleAdsConversion` action.
pub fn send_google_ads_conversion_action() -> Schema {
    with_google_ads_conversion_refinements(
        google_ads_conversion_fields_schema()
            .extend(vec![("type", z::literal(json!("sendGoogleAdsConversion")))]),
    )
}

/// `allActions` (`components/actions/schema/index.ts`), in its order.
pub fn all_actions() -> Vec<Schema> {
    vec![
        z::object(vec![
            ("type", z::literal(json!("addTag"))),
            ("tagIds", z::array(z::string()).min(1)),
        ]),
        z::object(vec![
            ("type", z::literal(json!("removeTag"))),
            ("tagIds", z::array(z::string()).min(1)),
        ]),
        z::object(vec![
            ("type", z::literal(json!("addSequence"))),
            ("sequenceIds", sequence_ids()),
        ]),
        z::object(vec![
            ("type", z::literal(json!("removeSequence"))),
            ("sequenceIds", sequence_ids()),
        ]),
        set_custom_field_action(),
        z::object(vec![
            ("type", z::literal(json!("clearCustomField"))),
            ("customFieldId", zod_field_reference()),
        ]),
        z::object(vec![
            ("type", z::literal(json!("startAnotherFlow"))),
            ("flowId", bigint()),
        ]),
        start_external_step_action(),
        z::object(vec![
            ("type", z::literal(json!("transferConversationToHuman"))),
            ("notifyAdmins", z::boolean()),
        ]),
        send_meta_capi_event_action(),
        send_google_ads_conversion_action(),
        run_google_sheet(),
        pipeline_action_schema(),
    ]
}

/// `updateTriggerSchema`.
pub fn update_trigger_schema() -> Schema {
    z::object(vec![
        ("conditions", z::array(z::union(all_conditions()))),
        ("actions", z::array(z::union(all_actions()))),
    ])
}

/// The parsed `conditions` as service input.
pub fn condition_inputs(parsed: &Value) -> Vec<ConditionInput> {
    parsed["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| ConditionInput {
            id: c["id"].as_str().map(str::to_string),
            r#type: c["type"].as_str().unwrap_or_default().to_string(),
            source_id: c["sourceId"].as_str().map(str::to_string),
            operator: c["operator"].as_str().map(str::to_string),
            value: c.get("value").filter(|v| !v.is_null()).cloned(),
        })
        .collect()
}

/// Validates `{ conditions, actions }` of `obj` into `input`'s issues.
pub fn parse_update(input: &mut Input) -> Option<Value> {
    let parsed = update_trigger_schema().safe_parse(&Value::Object(input.obj.clone()));
    if parsed.success {
        return parsed.data;
    }
    let issues = parsed
        .issues()
        .iter()
        .filter_map(|issue| serde_json::to_value(issue).ok())
        .collect();
    input.extend_issues(issues);
    None
}

fn not_found() -> chatbotx_business::BusinessError {
    chatbotx_business::BusinessError::not_found("Trigger not found")
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "triggers.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let (data, page_count) = service::list(db, ws, None, None, page, per_page)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = data.iter().map(trigger_json).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "triggers.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id");
            input.finish()?;
            match service::find(db, ws, id.unwrap_or_default()).await {
                Ok(Some(found)) => Ok(json(200, trigger_json(&found))),
                Ok(None) => Err(business_error(not_found(), FINDING)),
                Err(e) => Err(business_error(e, FINDING)),
            }
        }
        "triggers.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let parsed = z::object(vec![
                ("name", z::string().min(1).msg("Trigger name is required")),
                ("folderId", bigint().nullable()),
            ])
            .safe_parse(&Value::Object(input.obj.clone()));
            if !parsed.success {
                let issues = parsed
                    .issues()
                    .iter()
                    .filter_map(|issue| serde_json::to_value(issue).ok())
                    .collect();
                input.extend_issues(issues);
            }
            input.finish()?;
            let data = parsed.data.unwrap_or(Value::Null);
            let name = data["name"].as_str().unwrap_or_default().to_string();
            let folder_id = data["folderId"].as_str().and_then(|id| id.parse().ok());
            let created = service::create(db, ws, name, folder_id)
                .await
                .map_err(|e| business_error(e, CREATING))?;
            state
                .record_audit(
                    "create",
                    &format!("created a new trigger (#{})", created.id),
                )
                .await;
            let mut out = created.to_json();
            out["conditions"] = json!([]);
            Ok(json(201, out))
        }
        "triggers.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id");
            let parsed = parse_update(&mut input);
            input.finish()?;
            let parsed = parsed.unwrap_or(Value::Null);
            let actions = parsed["actions"].as_array().cloned().unwrap_or_default();
            let id = id.unwrap_or_default();
            match service::update_with_conditions(db, ws, id, actions, condition_inputs(&parsed))
                .await
            {
                Ok(Some((found, changed))) => {
                    if changed {
                        state
                            .record_audit("update", &format!("updated a trigger (#{id})"))
                            .await;
                    }
                    Ok(json(200, trigger_json(&found)))
                }
                Ok(None) => Err(business_error(not_found(), MUTATING)),
                Err(e) => Err(business_error(e, MUTATING)),
            }
        }
        "triggers.updateSettings" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id");
            let name = input.opt_string("name", super::input::Str::trimmed(1, 255));
            let active = input.opt_bool("active");
            input.finish()?;
            let (found, detail) =
                service::update_settings(db, ws, id.unwrap_or_default(), name, active)
                    .await
                    .map_err(|e| business_error(e, MUTATING))?;
            if let Some(detail) = detail {
                state.record_audit("update", &detail).await;
            }
            Ok(json(200, trigger_json(&found)))
        }
        "triggers.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id");
            input.finish()?;
            let deleted = service::delete_many(db, ws, vec![id.unwrap_or_default()])
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            if let Some(detail) = service::delete_audit_detail(&deleted) {
                state.record_audit("delete", &detail).await;
            }
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
