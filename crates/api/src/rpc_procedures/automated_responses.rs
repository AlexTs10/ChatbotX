//! Keywords (automated responses): `features/automated-response/actions/*`
//! (Q.D5). The list / find reads are server components (no `/rpc` call).

use chatbotx_business::BusinessError;
use chatbotx_business::automated_response::{
    self as service, CreateAutomatedResponse, ResponseType, UpdateAutomatedResponse,
};
use serde_json::Value;

use super::{Ctx, In, Outcome, action_fail, arg, invalid};
use crate::orpc::OrpcError;

/// The ported actions.
pub const ACTIONS: &[&str] = &[
    "createAutomatedResponseAction",
    "updateAutomatedResponseAction",
    "deleteAutomatedResponseAction",
    "enableAutomatedResponseAction",
    // The list / edit page queries (`features/automated-response/queries`).
    "listAutomatedResponses",
    "findAutomatedResponse",
];

/// `listAutomatedResponses` / `findAutomatedResponse` (`[workspaceId, input]`);
/// the edit page's `findBy` miss is `null`.
// ponytail: the page guard runs here too, since the SPA has no server page.
async fn query(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    // The pages are `superAdmin` only (every keywords layout): 404 otherwise.
    if !ctx.has("superAdmin") {
        return Err(super::fail(BusinessError::not_found("Page not found")));
    }
    let input = In(arg(args, 1));
    let kind = response_type(input.get("type").unwrap_or(&Value::Null))?;
    if name == "findAutomatedResponse" {
        let Ok(id) = input.id("id") else {
            return Ok(Value::Null);
        };
        return service::find_by(&ctx.api.db, ctx.workspace_id, id, kind)
            .await
            .map(|row| row.map_or(Value::Null, |r| r.to_json()))
            .map_err(super::fail);
    }
    let result = service::list(
        &ctx.api.db,
        service::ListAutomatedResponses {
            workspace_id: ctx.workspace_id,
            kind,
            folder_id: Some(input.opt_id("folderId")?.unwrap_or(0).to_string()),
            keyword: input.opt_str("keyword")?.filter(|k| !k.is_empty()),
            page: input.opt_positive("page")?.unwrap_or(1),
            per_page: input.opt_positive("perPage")?.unwrap_or(10),
        },
    )
    .await
    .map_err(super::fail)?;
    Ok(super::page(
        result.data.iter().map(|r| r.to_json()),
        result.page_count,
    ))
}

/// The `automatedResponseTypes` bind arg.
fn response_type(value: &Value) -> Result<ResponseType, OrpcError> {
    match value.as_str() {
        Some("inbound") => Ok(ResponseType::Inbound),
        Some("outbound") => Ok(ResponseType::Outbound),
        _ => Err(invalid()),
    }
}

/// A `zodBigintAsString` bind arg.
fn bound_id(value: &Value) -> Result<i64, OrpcError> {
    match value {
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(text.parse::<i64>().unwrap_or(-1))
        }
        _ => Err(invalid()),
    }
}

/// `createAutomatedResponseRequest` after `toKeywordRuleMatch`.
struct Request {
    folder_id: Option<Option<i64>>,
    keywords: Option<Vec<String>>,
    conditions: Option<Value>,
    text: Option<Option<String>>,
    flow_id: Option<Option<String>>,
}

fn request(value: &Value) -> Result<Request, OrpcError> {
    if !value.is_object() {
        return Err(invalid());
    }
    let input = In(value);
    // `.nullish()`: absent = not sent, `null` = cleared.
    let nullish = |key: &str| value.get(key).map(|v| (!v.is_null()).then_some(v));
    let folder_id = match nullish("folderId") {
        None => None,
        Some(_) => Some(input.opt_id("folderId")?),
    };
    let keywords = match input.get("keywords") {
        None => None,
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .map(|item| match item.get("value").and_then(Value::as_str) {
                    Some(text) if (1..=255).contains(&text.chars().count()) => Ok(text.to_string()),
                    _ => Err(invalid()),
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err(invalid()),
    };
    let conditions = match input.get("conditions") {
        None => None,
        Some(list @ Value::Array(_)) => Some(list.clone()),
        Some(_) => return Err(invalid()),
    };
    let text = match nullish("text") {
        None => None,
        Some(_) => Some(input.opt_str("text")?),
    };
    let flow_id = match nullish("flowId") {
        None => None,
        Some(None) => Some(None),
        Some(Some(Value::String(text))) if text.is_empty() => Some(Some(String::new())),
        Some(Some(_)) => Some(input.opt_id("flowId")?.map(|id| id.to_string())),
    };
    // The two `.refine`s: a keyword or a condition, a flow or a text.
    let has_conditions = conditions
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|c| !c.is_empty());
    let has_keywords = keywords.as_ref().is_some_and(|k| !k.is_empty());
    let filled = |v: &Option<Option<String>>| {
        v.as_ref()
            .is_some_and(|t| t.as_ref().is_some_and(|t| !t.is_empty()))
    };
    if !(has_keywords || has_conditions) || !(filled(&flow_id) || filled(&text)) {
        return Err(invalid());
    }
    // `toKeywordRuleMatch`: conditions when sent, else the keyword values.
    let (keywords, conditions) = if has_conditions {
        (None, conditions)
    } else {
        (Some(keywords.unwrap_or_default()), None)
    };
    Ok(Request {
        folder_id,
        keywords,
        conditions,
        text,
        flow_id,
    })
}

/// `updateAutomatedResponse`'s `returnValidationErrors`: a field other than
/// `conditions` / `keywords` is shown on `flowId`.
fn update_fail(error: BusinessError) -> OrpcError {
    match error {
        BusinessError::Exception {
            message,
            code,
            status,
            field: Some(field),
            data,
        } if code == "validation" && field != "conditions" && field != "keywords" => {
            action_fail(BusinessError::Exception {
                message,
                code,
                status,
                field: Some("flowId".to_string()),
                data,
            })
        }
        other => action_fail(other),
    }
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    if matches!(name, "listAutomatedResponses" | "findAutomatedResponse") {
        return Some(query(ctx, name, args).await);
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    Some(
        async {
            match name {
                // [workspaceId, type], input
                "createAutomatedResponseAction" => {
                    let kind = response_type(arg(args, 1))?;
                    let input = request(arg(args, 2))?;
                    let row = service::create(
                        db,
                        ws,
                        CreateAutomatedResponse {
                            kind,
                            text: input.text.flatten(),
                            flow_id: input.flow_id.flatten(),
                            folder_id: input.folder_id.flatten(),
                            keywords: input.keywords,
                            conditions: input.conditions,
                        },
                    )
                    .await
                    .map_err(action_fail)?;
                    ctx.api
                        .record_audit("create", &service::create_audit(row.id))
                        .await;
                    Ok(Value::Null)
                }
                // [workspaceId, id, type], input
                "updateAutomatedResponseAction" => {
                    let id = bound_id(arg(args, 1))?;
                    let kind = response_type(arg(args, 2))?;
                    let input = request(arg(args, 3))?;
                    // The form sends the unused reply field as "": no flow.
                    let flow_id = input.flow_id.map(|flow| {
                        flow.filter(|f| !f.is_empty())
                            .map(|f| f.parse::<i64>().unwrap_or(-1))
                    });
                    let updated = service::update(
                        db,
                        ws,
                        id,
                        kind,
                        UpdateAutomatedResponse {
                            folder_id: input.folder_id,
                            keywords: input.keywords,
                            conditions: input.conditions,
                            text: input.text,
                            flow_id,
                        },
                    )
                    .await
                    .map_err(update_fail)?;
                    if updated.changed {
                        ctx.api
                            .record_audit("update", &service::update_audit(updated.row.id))
                            .await;
                    }
                    Ok(Value::Null)
                }
                // [workspaceId, type], { ids }
                "deleteAutomatedResponseAction" => {
                    let kind = response_type(arg(args, 1))?;
                    let ids = In(arg(args, 2)).ids("ids")?;
                    let deleted = service::delete_many(db, ws, ids, kind)
                        .await
                        .map_err(action_fail)?;
                    if let Some(detail) = service::delete_audit(&deleted) {
                        ctx.api.record_audit("delete", &detail).await;
                    }
                    Ok(Value::Null)
                }
                // [workspaceId, id, type], { status }
                _ => {
                    let id = bound_id(arg(args, 1))?;
                    let kind = response_type(arg(args, 2))?;
                    let Some(Value::Bool(status)) = arg(args, 3).get("status") else {
                        return Err(invalid());
                    };
                    let status = *status;
                    service::find_or_fail(db, ws, id, kind)
                        .await
                        .map_err(action_fail)?;
                    let change = service::set_status(db, ws, id, kind, status)
                        .await
                        .map_err(action_fail)?;
                    if change.changed {
                        ctx.api
                            .record_audit("update", &service::status_audit(id, status))
                            .await;
                    }
                    Ok(Value::Null)
                }
            }
        }
        .await,
    )
}
