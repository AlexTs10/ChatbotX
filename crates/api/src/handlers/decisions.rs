//! `features/decision-connections/api/public.ts` and
//! `features/decision-profiles/api/public.ts`: `decisionConnections.*` and
//! `decisionProfiles.*` (scope `automation`), plus the input checks the
//! `/rpc` actions share.

use chatbotx_business::decision::{
    self as service, ConnectionInput, Deps, ListProfiles, TestResult, transport::PROVIDER_KINDS,
};
use chatbotx_business::net::ssrf_guard::HttpDoh;
use chatbotx_core::decision_config::{
    DecisionProfileForm, PathSegment, parse_decision_profile_form,
};
use serde_json::{Map, Value, json};

use super::input::{Input, Seg, Str, key};
use super::{ApiState, Call, ErrorMap, FINDING, LISTING, Reply, business_error, json, no_content};
use crate::orpc::OrpcError;

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "decisionConnections.list",
    "decisionConnections.create",
    "decisionConnections.update",
    "decisionConnections.setEnabled",
    "decisionConnections.test",
    "decisionProfiles.list",
    "decisionProfiles.create",
    "decisionProfiles.deleteMany",
    "decisionProfiles.get",
    "decisionProfiles.update",
    "decisionProfiles.setEnabled",
    "decisionProfiles.listGlobalRouterDestinations",
];

/// `possibleErrorsOnCreatingDecisionConnection` / `...DecisionProfile`.
pub const CREATING: ErrorMap = &[
    ("invalidDecisionContract", 400),
    ("invalidDecisionEndpoint", 400),
    ("invalidDecisionModel", 400),
    ("invalidDecisionReference", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnMutatingDecisionConnection` / `...DecisionProfile`.
pub const MUTATING: ErrorMap = &[
    ("notFound", 404),
    ("invalidDecisionContract", 400),
    ("invalidDecisionEndpoint", 400),
    ("invalidDecisionModel", 400),
    ("invalidDecisionReference", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnTestingDecisionConnection`.
pub const TESTING: ErrorMap = &[
    ("notFound", 404),
    ("invalidDecisionModel", 400),
    ("decisionConnectionTestThrottled", 429),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

const TRIMMED_2048: Str = Str {
    trim: true,
    min: None,
    max: Some(2048),
    pattern: None,
};

const CONNECTION_KEYS: &[&str] = &[
    "credential",
    "defaultModel",
    "endpoint",
    "modelCatalog",
    "name",
    "providerKind",
];

/// The service dependencies of a request.
pub fn deps(state: &ApiState) -> Deps<'_, HttpDoh> {
    Deps {
        db: &state.db,
        encryption: state.encryption.as_deref(),
        kv: state.kv.as_deref(),
        doh: &HttpDoh,
    }
}

/// `.strict()`: unknown top-level keys (the path params excluded).
pub fn strict(input: &mut Input, allowed: &[&str], params: &[&str]) {
    let unknown: Vec<String> = input
        .obj
        .keys()
        .filter(|k| !allowed.contains(&k.as_str()) && !params.contains(&k.as_str()))
        .map(|k| format!("\"{k}\""))
        .collect();
    if !unknown.is_empty() {
        let plural = if unknown.len() > 1 { "s" } else { "" };
        input.issue(
            &[],
            "unrecognized_keys",
            &format!("Unrecognized key{plural}: {}", unknown.join(", ")),
        );
    }
}

/// `decisionConnectionActionSchema` (+ `credential` required on create).
pub fn connection_fields(input: &mut Input, credential_required: bool) -> ConnectionInput {
    let credential = if credential_required {
        input.string("credential", Str::trimmed(1, 2048))
    } else {
        input.opt_string("credential", TRIMMED_2048)
    };
    let default_model = input
        .nullish_string("defaultModel", Str::trimmed(1, 256))
        .flatten();
    let endpoint = input.nullish_string("endpoint", TRIMMED_2048).flatten();
    let mut models = Vec::new();
    match input.get("modelCatalog").cloned() {
        Some(Value::Array(items)) => {
            if items.is_empty() {
                input.issue(
                    &key("modelCatalog"),
                    "too_small",
                    "Too small: expected array to have >=1 items",
                );
            } else if items.len() > 100 {
                input.issue(
                    &key("modelCatalog"),
                    "too_big",
                    "Too big: expected array to have <=100 items",
                );
            }
            for (i, item) in items.iter().enumerate() {
                let path = vec![Seg::Key("modelCatalog".into()), Seg::Index(i)];
                if let Some(model) = input.string_at(&path, Some(item), Str::trimmed(1, 256), true)
                {
                    models.push(model);
                }
            }
        }
        other => {
            let received = match other {
                None => "undefined",
                Some(Value::Null) => "null",
                Some(Value::String(_)) => "string",
                Some(Value::Number(_)) => "number",
                Some(Value::Bool(_)) => "boolean",
                Some(_) => "object",
            };
            input.issue(
                &key("modelCatalog"),
                "invalid_type",
                &format!("Invalid input: expected array, received {received}"),
            );
        }
    }
    let name = input.string("name", Str::trimmed(1, 160));
    let provider = input.get("providerKind").cloned();
    let provider_kind = input.enum_at(
        &key("providerKind"),
        provider.as_ref(),
        PROVIDER_KINDS,
        true,
    );
    ConnectionInput {
        name: name.unwrap_or_default(),
        provider_kind: provider_kind.unwrap_or_default(),
        endpoint,
        model_catalog: models,
        default_model,
        credential,
    }
}

/// `decisionProfileFormSchema` over the body (path params excluded); the
/// issues are recorded on `input`.
pub fn profile_form(input: &mut Input, params: &[&str]) -> Option<DecisionProfileForm> {
    let body: Map<String, Value> = input
        .obj
        .iter()
        .filter(|(k, _)| !params.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    match parse_decision_profile_form(&Value::Object(body)) {
        Ok(form) => Some(form),
        Err(issues) => {
            input.extend_issues(
                issues
                    .into_iter()
                    .map(|issue| {
                        json!({
                            "code": "custom",
                            "path": issue.path.iter().map(|s| match s {
                                PathSegment::Key(k) => json!(k),
                                PathSegment::Index(i) => json!(i),
                            }).collect::<Vec<_>>(),
                            "message": issue.message,
                        })
                    })
                    .collect(),
            );
            None
        }
    }
}

/// `{ enabled: boolean }` (strict).
pub fn enabled_field(input: &mut Input) -> Option<bool> {
    strict(input, &["enabled"], &["id"]);
    input.bool("enabled")
}

/// `ids: zodBigintAsString[]` (min 1, strict).
pub fn ids_field(input: &mut Input) -> Vec<i64> {
    strict(input, &["ids"], &[]);
    let mut ids = Vec::new();
    match input.get("ids").cloned() {
        Some(Value::Array(items)) => {
            if items.is_empty() {
                input.issue(
                    &key("ids"),
                    "too_small",
                    "Too small: expected array to have >=1 items",
                );
            }
            for (i, item) in items.iter().enumerate() {
                let path = vec![Seg::Key("ids".into()), Seg::Index(i)];
                match item.as_str() {
                    Some(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
                        ids.push(text.parse().unwrap_or(-1));
                    }
                    _ => input.issue(
                        &path,
                        "invalid_format",
                        "Invalid string: must match pattern /^\\d+$/",
                    ),
                }
            }
        }
        _ => input.issue(
            &key("ids"),
            "invalid_type",
            "Invalid input: expected array, received undefined",
        ),
    }
    ids
}

/// `publicDecisionConnectionTestResultSchema`.
pub fn test_result_json(result: &TestResult) -> Value {
    json!({
        "code": result.code,
        "latencyMs": result.latency_ms,
        "model": result.model,
        "status": result.status,
    })
}

fn profile_out(row: &chatbotx_db::repo::models::DecisionProfile) -> Result<Value, OrpcError> {
    service::public_profile_json(row).map_err(|e| business_error(e, FINDING))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let deps = deps(state);
    match route_id {
        "decisionConnections.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = super::qr_codes::public_paging(&mut input);
            input.finish()?;
            let rows = service::list_connections(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            // `paginateInMemory`.
            let start = ((page - 1) * per_page).max(0) as usize;
            let data: Vec<Value> = rows
                .iter()
                .skip(start)
                .take(per_page as usize)
                .map(service::safe_connection_json)
                .collect();
            let page_count = ((rows.len() as i64 + per_page - 1) / per_page).max(1);
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "decisionConnections.create" => {
            let mut input = Input::from_body(call.params, call.body);
            strict(&mut input, CONNECTION_KEYS, &[]);
            let fields = connection_fields(&mut input, true);
            input.finish()?;
            let row = service::create_connection(&deps, ws, fields)
                .await
                .map_err(|e| business_error(e, CREATING))?;
            Ok(json(201, service::safe_connection_json(&row)))
        }
        "decisionConnections.update" => {
            let mut input = Input::from_body(call.params, call.body);
            strict(&mut input, CONNECTION_KEYS, &["id"]);
            let fields = connection_fields(&mut input, false);
            let id = input.digits_id("id");
            input.finish()?;
            let row = service::update_connection(&deps, ws, id.unwrap_or_default(), fields)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, service::safe_connection_json(&row)))
        }
        "decisionConnections.setEnabled" => {
            let mut input = Input::from_body(call.params, call.body);
            let enabled = enabled_field(&mut input);
            let id = input.digits_id("id");
            input.finish()?;
            let row = service::set_connection_enabled(
                db,
                ws,
                id.unwrap_or_default(),
                enabled.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, service::safe_connection_json(&row)))
        }
        "decisionConnections.test" => {
            let mut input = Input::from_body(call.params, call.body);
            strict(&mut input, &[], &["id"]);
            let id = input.digits_id("id");
            input.finish()?;
            let result = service::test_connection(&deps, ws, id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, TESTING))?;
            Ok(json(200, test_result_json(&result)))
        }
        "decisionProfiles.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = super::qr_codes::public_paging(&mut input);
            input.finish()?;
            let (rows, page_count) = service::list_profiles(
                db,
                ws,
                ListProfiles {
                    page: Some(page),
                    per_page: Some(per_page),
                    ..ListProfiles::default()
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data = rows
                .iter()
                .map(|r| profile_out(&r.profile))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "decisionProfiles.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id");
            input.finish()?;
            let row = service::find_profile(db, ws, id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, profile_out(&row)?))
        }
        "decisionProfiles.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let form = profile_form(&mut input, &[]);
            input.finish()?;
            let Some(form) = form else {
                return Err(super::internal_error("invalid profile form"));
            };
            let row = service::create_profile(db, ws, form)
                .await
                .map_err(|e| business_error(e, CREATING))?;
            Ok(json(201, profile_out(&row)?))
        }
        "decisionProfiles.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let form = profile_form(&mut input, &["id"]);
            let id = input.digits_id("id");
            input.finish()?;
            let Some(form) = form else {
                return Err(super::internal_error("invalid profile form"));
            };
            let row = service::update_profile(db, ws, id.unwrap_or_default(), form)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, profile_out(&row)?))
        }
        "decisionProfiles.setEnabled" => {
            let mut input = Input::from_body(call.params, call.body);
            let enabled = enabled_field(&mut input);
            let id = input.digits_id("id");
            input.finish()?;
            let row = service::set_profile_enabled(
                db,
                ws,
                id.unwrap_or_default(),
                enabled.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, profile_out(&row)?))
        }
        "decisionProfiles.deleteMany" => {
            let mut input = Input::from_body(call.params, call.body);
            let ids = ids_field(&mut input);
            input.finish()?;
            service::delete_profiles(db, ws, ids)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "decisionProfiles.listGlobalRouterDestinations" => {
            let input = Input::from_query(call.params, call.query);
            input.finish()?;
            let out = service::global_router_destinations(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            Ok(json(200, out))
        }
        _ => Err(super::internal_error("unknown decision route")),
    }
}
