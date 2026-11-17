//! `features/mini-apps/api/public.ts` + `mappers.ts` + `lib/delete-mini-apps.ts`:
//! `miniApps.*`. The WhatsApp Flow routes (publish, draft, preview) need the
//! WhatsApp Flows Graph port and answer "not ported".

use std::collections::BTreeMap;

use chatbotx_business::mini_app::{self as service, DefinitionInput};
use chatbotx_core::mini_app::{self as app, collect_custom_field_mappings, locate_issues, validate};
use chatbotx_db::repo::mini_app::MiniAppListRow;
use chatbotx_db::repo::models::{MiniApp, MiniAppPublication};
use serde_json::{Map, Value, json};

use super::input::{Input, Str, Seg, key};
use super::qr_codes::public_paging;
use super::{ApiState, Call, Reply, business_error, date, id, json, opt_id};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "miniApps.list",
    "miniApps.get",
    "miniApps.validate",
    "miniApps.create",
    "miniApps.duplicate",
    "miniApps.update",
    "miniApps.delete",
    "miniApps.deleteMany",
    "miniApps.getFlowJson",
    "miniApps.listSubmissions",
];

/// `MINI_APP_BULK_DELETE_MAX_IDS`.
pub const BULK_DELETE_MAX_IDS: usize = 100;
/// `miniAppName`.
pub const NAME: Str = Str::trimmed(1, app::NAME_MAX_LENGTH);
/// The `failed` reason of a Flow removal while WhatsApp Flows are not ported.
pub const FLOW_REMOVAL_UNAVAILABLE: &str = "WhatsApp Flow removal is not available yet";

/// `buildMiniAppPublicUrl`.
pub fn public_url(app_url: &str, mini_app_id: i64) -> String {
    format!("{}/mini-apps?id={mini_app_id}&token={{{{mini_app_token}}}}", super::messenger_templates::broker_origin(app_url))
}

struct Summary<'a> {
    id: i64,
    name: &'a str,
    enabled: bool,
    submissions_count: i64,
    created_at: i64,
    updated_at: i64,
}

fn summary_json(app_url: &str, s: Summary) -> Map<String, Value> {
    let Value::Object(out) = json!({
        "id": id(s.id),
        "name": s.name,
        "enabled": s.enabled,
        "submissionsCount": s.submissions_count,
        "createdAt": date(s.created_at),
        "updatedAt": date(s.updated_at),
        "publicUrl": public_url(app_url, s.id),
    }) else {
        unreachable!("json! object")
    };
    out
}

/// `toSummary` of a list row.
pub fn list_row_json(app_url: &str, row: &MiniAppListRow) -> Value {
    Value::Object(summary_json(
        app_url,
        Summary {
            id: row.id,
            name: &row.name,
            enabled: row.enabled,
            submissions_count: row.submissions_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        },
    ))
}

fn summary_of(app_url: &str, row: &MiniApp) -> Map<String, Value> {
    summary_json(
        app_url,
        Summary {
            id: row.id,
            name: &row.name,
            enabled: row.enabled,
            submissions_count: row.submissions_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        },
    )
}

/// `toValidation`: issues addressed by Flow JSON screen id and path.
pub fn validation_json(definition: &Value) -> Value {
    let result = validate(definition);
    json!({ "valid": result.valid, "issues": locate_issues(definition, &result.issues) })
}

/// `toDetail`.
pub fn detail_json(app_url: &str, row: &MiniApp, publications: &[MiniAppPublication]) -> Value {
    let definition = &row.definition.0;
    let mappings = app::parse_definition(definition).map(|d| collect_custom_field_mappings(&d)).unwrap_or_default();
    let mut out = summary_of(app_url, row);
    out.insert("flowJson".into(), row.flow_json.0.clone());
    out.insert("customFieldMappings".into(), json!(mappings));
    out.insert("validation".into(), validation_json(definition));
    let publications: Vec<Value> = publications
        .iter()
        .map(|p| {
            json!({
                "integrationWhatsappId": id(p.integration_whatsapp_id),
                "status": p.status,
                "whatsappFlowId": opt_id(p.whatsapp_flow_id),
                "publishedAt": p.published_at.map_or(Value::Null, date),
            })
        })
        .collect();
    out.insert("publications".into(), Value::Array(publications));
    Value::Object(out)
}

/// `flowJsonSchema` on `name` (an issue when it is not a Flow JSON).
fn flow_json(input: &mut Input, name: &str, required: bool) -> Option<Value> {
    let value = input.get(name).cloned();
    match value {
        None if !required => None,
        Some(v) if app::serialize::is_flow_json(&v) => Some(v),
        other => {
            let message = if other.is_none() { "Invalid input: expected object, received undefined" } else { "Invalid input" };
            input.issue(&key(name), if other.is_none() { "invalid_type" } else { "custom" }, message);
            None
        }
    }
}

/// `customFieldMappingsInput`: name → digit id string or `null`.
fn mappings(input: &mut Input) -> Option<BTreeMap<String, Option<String>>> {
    let value = input.get("customFieldMappings").cloned()?;
    let Value::Object(map) = value else {
        input.issue(&key("customFieldMappings"), "invalid_type", "Invalid input: expected record, received other");
        return None;
    };
    let mut out = BTreeMap::new();
    for (name, value) in map {
        match value {
            Value::Null => {
                out.insert(name, None);
            }
            Value::String(s) if super::input::is_bigint_id(&s) => {
                out.insert(name, Some(s));
            }
            _ => input.issue(
                &[Seg::Key("customFieldMappings".into()), Seg::Key(name.clone())],
                "custom",
                "Invalid input",
            ),
        }
    }
    Some(out)
}

/// `deleteMiniApps`: deletes the rows; with `deleteWhatsappFlows`, each
/// publication reports `failed` (the WhatsApp Flows Graph port is missing).
pub async fn delete_mini_apps(state: &ApiState, ws: i64, ids: Vec<i64>, delete_flows: bool) -> Result<Value, chatbotx_business::errors::BusinessError> {
    let publications =
        if delete_flows { service::list_publications(&state.db, ws, ids.clone()).await? } else { Vec::new() };
    let deleted = service::delete_many(&state.db, ws, ids).await?;
    let flows: Vec<Value> = publications
        .iter()
        .map(|p| {
            json!({ "integrationWhatsappId": id(p.integration_whatsapp_id), "outcome": "failed", "error": FLOW_REMOVAL_UNAVAILABLE })
        })
        .collect();
    Ok(json!({ "deletedCount": deleted, "whatsappFlows": flows }))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let app_url = state.app_url.as_str();
    match route_id {
        "miniApps.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let name = input.opt_string("name", Str::trimmed(1, usize::MAX));
            input.finish()?;
            let (rows, page_count) =
                service::list(db, ws, name, page, per_page).await.map_err(|e| business_error(e, super::LISTING))?;
            let data: Vec<Value> = rows.iter().map(|r| list_row_json(app_url, r)).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "miniApps.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let app_id = input.id("id");
            input.finish()?;
            let found = service::find_or_fail(db, ws, app_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            Ok(json(200, detail_json(app_url, &found.mini_app, &found.publications)))
        }
        "miniApps.validate" => {
            let mut input = Input::from_body(call.params, call.body);
            let flow = flow_json(&mut input, "flowJson", true);
            input.finish()?;
            let definition =
                service::import_flow_json(&flow.unwrap_or_default()).map_err(|e| business_error(e, super::LISTING))?;
            Ok(json(200, validation_json(&definition.to_value())))
        }
        "miniApps.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", NAME);
            let flow = flow_json(&mut input, "flowJson", false);
            let custom_field_mappings = mappings(&mut input);
            input.finish()?;
            let definition = flow.is_none().then(|| app::starter_definition().to_value());
            let data = DefinitionInput { definition, flow_json: flow, custom_field_mappings };
            let created = service::create(db, ws, name.unwrap_or_default(), data)
                .await
                .map_err(|e| business_error(e, super::CREATING))?;
            Ok(json(201, detail_json(app_url, &created, &[])))
        }
        "miniApps.duplicate" => {
            let mut input = Input::from_body(call.params, call.body);
            let app_id = input.id("id");
            input.finish()?;
            let copy = service::duplicate(db, ws, app_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, super::CREATING))?;
            Ok(json(201, detail_json(app_url, &copy, &[])))
        }
        "miniApps.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let app_id = input.id("id").unwrap_or_default();
            let name = input.opt_string("name", NAME);
            let flow = flow_json(&mut input, "flowJson", false);
            let custom_field_mappings = mappings(&mut input);
            input.finish()?;
            let data = DefinitionInput { definition: None, flow_json: flow, custom_field_mappings };
            service::update(db, ws, app_id, name, None, data).await.map_err(|e| business_error(e, super::MUTATING))?;
            let found = service::find_or_fail(db, ws, app_id).await.map_err(|e| business_error(e, super::MUTATING))?;
            Ok(json(200, detail_json(app_url, &found.mini_app, &found.publications)))
        }
        "miniApps.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let app_id = input.id("id").unwrap_or_default();
            let delete_flows = input.opt_coerced_bool("deleteWhatsappFlows").unwrap_or(false);
            input.finish()?;
            service::find_or_fail(db, ws, app_id).await.map_err(|e| business_error(e, super::MUTATING))?;
            let out = delete_mini_apps(state, ws, vec![app_id], delete_flows)
                .await
                .map_err(|e| business_error(e, super::MUTATING))?;
            Ok(json(200, out))
        }
        "miniApps.deleteMany" => {
            let mut input = Input::from_body(call.params, call.body);
            let ids = super::contacts_bulk::ids(&mut input, "ids", 1, BULK_DELETE_MAX_IDS);
            let delete_flows = input.opt_bool("deleteWhatsappFlows").unwrap_or(false);
            input.finish()?;
            let out = delete_mini_apps(state, ws, ids.unwrap_or_default(), delete_flows)
                .await
                .map_err(|e| business_error(e, super::MUTATING))?;
            Ok(json(200, out))
        }
        "miniApps.getFlowJson" => {
            let mut input = Input::from_query(call.params, call.query);
            let app_id = input.id("id");
            input.finish()?;
            let found = service::find_or_fail(db, ws, app_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            Ok(json(200, found.mini_app.flow_json.0))
        }
        "miniApps.listSubmissions" => {
            let mut input = Input::from_query(call.params, call.query);
            let app_id = input.id("id").unwrap_or_default();
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            service::find_or_fail(db, ws, app_id).await.map_err(|e| business_error(e, super::FINDING))?;
            let (rows, page_count, _) = service::list_submissions(db, ws, app_id, page, per_page)
                .await
                .map_err(|e| business_error(e, super::LISTING))?;
            let data: Vec<Value> = rows
                .iter()
                .map(|r| {
                    let s = &r.submission;
                    json!({ "id": id(s.id), "contactId": opt_id(s.contact_id), "answers": s.answers.0, "createdAt": date(s.created_at) })
                })
                .collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
