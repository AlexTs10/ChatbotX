//! `apps/builder/src/features/automated-response/api/public.ts`
//! (`keywordsPublicRouter`, `/v1/keywords`).

use chatbotx_business::automated_response::{
    self as service, CreateAutomatedResponse, ListAutomatedResponses, ResponseType,
    UpdateAutomatedResponse,
};
use chatbotx_db::repo::models::AutomatedResponse;
use chatbotx_db::repo::partials::keyword_condition_schema::parse_keyword_conditions;
use serde_json::{Value, json};

use super::input::{Input, Num, Seg, Str, key};
use super::{
    ApiState, CREATING, Call, ErrorMap, FINDING, LISTING, MUTATING, Reply, business_error, date,
    id, json, no_content, opt_id,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "keywords.list",
    "keywords.get",
    "keywords.create",
    "keywords.update",
    "keywords.updateStatus",
    "keywords.delete",
];

/// `UPDATABLE_KEYWORD_FIELDS`.
const UPDATABLE: &[&str] = &["keywords", "conditions", "text", "flowId", "folderId"];

/// `publicKeywordResource`.
pub fn keyword_json(row: &AutomatedResponse) -> Value {
    json!({
        "id": id(row.id),
        "keywords": row.keywords.0,
        "conditions": row.conditions.as_ref().map_or(Value::Null, |c| c.0.clone()),
        "status": row.status,
        "text": row.text,
        "flowId": opt_id(row.flow_id),
        "type": row.r#type,
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
    })
}

/// `automatedResponseTypes.default("inbound")`.
fn response_type(input: &mut Input) -> ResponseType {
    let value = input.get("type").cloned();
    match input
        .enum_at(
            &key("type"),
            value.as_ref(),
            &["inbound", "outbound"],
            false,
        )
        .as_deref()
    {
        Some("outbound") => ResponseType::Outbound,
        _ => ResponseType::Inbound,
    }
}

/// `zodBigintAsString()` at a top-level key; beyond `i64` = `-1` (no row).
fn bigint(input: &mut Input, name: &str, value: &Value) -> Option<i64> {
    let text = input.string_at(&key(name), Some(value), Str::default(), true)?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        input.issue(
            &key(name),
            "invalid_format",
            "Invalid string: must match pattern /^\\d+$/",
        );
        return None;
    }
    Some(text.parse().unwrap_or(-1))
}

fn path_id(input: &mut Input) -> Option<i64> {
    let value = input.get("id").cloned();
    match value {
        Some(value) => bigint(input, "id", &value),
        None => {
            input.issue(
                &key("id"),
                "invalid_type",
                "Invalid input: expected string, received undefined",
            );
            None
        }
    }
}

/// A `.nullish()` `zodBigintAsString()` key.
fn nullish_bigint(input: &mut Input, name: &str) -> Option<Option<i64>> {
    match input.get(name).cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => bigint(input, name, &value).map(Some),
    }
}

/// `z.array(z.string().min(1).max(255)).min(1).optional()`.
fn keywords(input: &mut Input) -> Option<Vec<String>> {
    let value = input.get("keywords").cloned()?;
    let Value::Array(items) = value else {
        input.issue(
            &key("keywords"),
            "invalid_type",
            "Invalid input: expected array",
        );
        return None;
    };
    if items.is_empty() {
        input.issue(
            &key("keywords"),
            "too_small",
            "Too small: expected array to have >=1 items",
        );
        return None;
    }
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let path = vec![Seg::Key("keywords".into()), Seg::Index(index)];
        if let Some(text) = input.string_at(
            &path,
            Some(item),
            Str {
                trim: false,
                min: Some(1),
                max: Some(255),
                pattern: None,
            },
            true,
        ) {
            out.push(text);
        }
    }
    (out.len() == items.len()).then_some(out)
}

/// `keywordConditionsSchema.optional()`: the issues go under `conditions`.
fn conditions(input: &mut Input) -> Option<Value> {
    let value = input.get("conditions").cloned()?;
    if let Err(error) = parse_keyword_conditions(&value) {
        for issue in error.issues {
            let mut path = key("conditions");
            path.extend(issue.path.split('.').filter(|s| !s.is_empty()).map(|s| {
                s.parse::<usize>()
                    .map_or_else(|_| Seg::Key(s.to_string()), Seg::Index)
            }));
            input.issue(&path, "custom", &issue.message);
        }
        return None;
    }
    Some(value)
}

/// oRPC's boolean coercion: `"true"` / `"on"` and `"false"` / `"off"`.
fn status(input: &mut Input) -> Option<bool> {
    let value = match input.get("status").cloned() {
        Some(Value::String(text)) => match text.as_str() {
            "true" | "on" => Some(Value::Bool(true)),
            "false" | "off" => Some(Value::Bool(false)),
            _ => Some(Value::String(text)),
        },
        other => other,
    };
    input.bool_at(&key("status"), value.as_ref(), true)
}

fn keyword_refinement(input: &mut Input, has_keywords: bool, has_conditions: bool) {
    if has_keywords && has_conditions {
        input.issue(
            &key("conditions"),
            "custom",
            "Send either keywords or conditions, not both",
        );
    }
}

/// `withPublicPaging`-style `publicListRequest` (`perPage` 1–50).
fn paging(input: &mut Input) -> (i64, i64) {
    let page = input.opt_coerce_int(
        "page",
        Num {
            min: Some(1.0),
            ..Num::default()
        },
    );
    let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 50.0));
    (page.unwrap_or(1), per_page.unwrap_or(50))
}

fn fail(errors: ErrorMap) -> impl Fn(chatbotx_business::BusinessError) -> crate::orpc::OrpcError {
    move |e| business_error(e, errors)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "keywords.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let kind = response_type(&mut input);
            let (page, per_page) = paging(&mut input);
            input.finish()?;
            let result = service::list(
                db,
                ListAutomatedResponses {
                    workspace_id: ws,
                    kind,
                    folder_id: None,
                    keyword: None,
                    page,
                    per_page,
                },
            )
            .await
            .map_err(fail(LISTING))?;
            let data: Vec<Value> = result.data.iter().map(keyword_json).collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": result.page_count }),
            ))
        }
        "keywords.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let rule = path_id(&mut input);
            let kind = response_type(&mut input);
            input.finish()?;
            let row = service::find_or_fail(db, ws, rule.unwrap_or(-1), kind)
                .await
                .map_err(fail(FINDING))?;
            Ok(json(200, keyword_json(&row)))
        }
        "keywords.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let kind = response_type(&mut input);
            let has_keywords = input.has("keywords");
            let has_conditions = input.has("conditions");
            let words = keywords(&mut input);
            let conds = conditions(&mut input);
            let text = input.nullish_string(
                "text",
                Str {
                    min: Some(1),
                    ..Str::default()
                },
            );
            let flow_id = nullish_bigint(&mut input, "flowId");
            let folder_id = nullish_bigint(&mut input, "folderId");
            if !input.failed() {
                keyword_refinement(&mut input, has_keywords, has_conditions);
                if !has_keywords && !has_conditions {
                    input.issue(&key("keywords"), "custom", "Send keywords or conditions");
                }
            }
            input.finish()?;
            let row = service::create(
                db,
                ws,
                CreateAutomatedResponse {
                    kind,
                    text: text.flatten(),
                    flow_id: flow_id.flatten().map(|f| f.to_string()),
                    folder_id: folder_id.flatten(),
                    keywords: words,
                    conditions: conds,
                },
            )
            .await
            .map_err(fail(CREATING))?;
            state
                .record_audit("create", &service::create_audit(row.id))
                .await;
            Ok(json(201, keyword_json(&row)))
        }
        "keywords.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let rule = path_id(&mut input);
            let kind = response_type(&mut input);
            let has_keywords = input.has("keywords");
            let has_conditions = input.has("conditions");
            let words = keywords(&mut input);
            let conds = conditions(&mut input);
            let text = input.nullish_string(
                "text",
                Str {
                    min: Some(1),
                    ..Str::default()
                },
            );
            let flow_id = nullish_bigint(&mut input, "flowId");
            let folder_id = nullish_bigint(&mut input, "folderId");
            if !input.failed() {
                keyword_refinement(&mut input, has_keywords, has_conditions);
                if !UPDATABLE.iter().any(|name| input.has(name)) {
                    input.issue(
                        &[],
                        "custom",
                        "Nothing to update: send keywords, conditions, text, flowId or folderId. Use keywords.updateStatus to enable or disable.",
                    );
                }
            }
            input.finish()?;
            let rule = rule.unwrap_or(-1);
            let updated = service::update(
                db,
                ws,
                rule,
                kind,
                UpdateAutomatedResponse {
                    folder_id,
                    keywords: words,
                    conditions: conds,
                    text,
                    flow_id,
                },
            )
            .await
            .map_err(fail(MUTATING))?;
            if updated.changed {
                state
                    .record_audit("update", &service::update_audit(updated.row.id))
                    .await;
            }
            Ok(json(200, keyword_json(&updated.row)))
        }
        "keywords.updateStatus" => {
            let mut input = Input::from_body(call.params, call.body);
            let rule = path_id(&mut input);
            let enabled = status(&mut input);
            let kind = response_type(&mut input);
            input.finish()?;
            let rule = rule.unwrap_or(-1);
            let enabled = enabled.unwrap_or_default();
            service::find_or_fail(db, ws, rule, kind)
                .await
                .map_err(fail(MUTATING))?;
            let change = service::set_status(db, ws, rule, kind, enabled)
                .await
                .map_err(fail(MUTATING))?;
            let Some(row) = change.row else {
                return Err(business_error(
                    chatbotx_business::BusinessError::not_found("Automated response not found"),
                    MUTATING,
                ));
            };
            if change.changed {
                state
                    .record_audit("update", &service::status_audit(row.id, enabled))
                    .await;
            }
            Ok(json(200, keyword_json(&row)))
        }
        "keywords.delete" => {
            let mut input = Input::from_body(call.params, call.body);
            let rule = path_id(&mut input);
            let kind = response_type(&mut input);
            input.finish()?;
            let deleted = service::delete_many(db, ws, vec![rule.unwrap_or(-1)], kind)
                .await
                .map_err(fail(MUTATING))?;
            if let Some(detail) = service::delete_audit(&deleted) {
                state.record_audit("delete", &detail).await;
            }
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
