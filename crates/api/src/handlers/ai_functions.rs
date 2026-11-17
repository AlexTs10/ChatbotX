//! `apps/builder/src/features/ai-functions/api/public.ts`
//! (`aiFunctionsPublicRouter`).

use chatbotx_business::ai_function::{self as service, AiFunctionInput, DataCollect};
use chatbotx_db::repo::models::AiFunction;
use chatbotx_db::repo::utils::PaginationInput;
use serde_json::{Value, json};

use super::input::{Input, Seg, Str, child, key};
use super::qr_codes::public_paging;
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json,
    no_content, opt_id,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "aiFunctions.list",
    "aiFunctions.create",
    "aiFunctions.get",
    "aiFunctions.update",
    "aiFunctions.delete",
];

/// `aiFunctionResource` (`createSelectSchema(aiFunctionModel)`).
pub fn ai_function_json(row: &AiFunction) -> Value {
    json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "name": row.name,
        "purpose": row.purpose,
        "dataCollect": row.data_collect.as_ref().map_or(Value::Null, |j| j.0.clone()),
        "outputMessage": row.output_message,
        "triggerFlowId": opt_id(row.trigger_flow_id),
        "workspaceId": id(row.workspace_id),
    })
}

/// `createAIFunctionRequest` (= `updateAIFunctionRequest`).
pub fn parse_ai_function_input(input: &mut Input) -> Option<AiFunctionInput> {
    let name = input.string("name", Str::trimmed(1, usize::MAX));
    let purpose = input.nullish_string("purpose", Str::trimmed(0, usize::MAX));
    let data_collect = data_collect(input);
    let output_message = input.nullish_string("outputMessage", Str::trimmed(0, usize::MAX));
    let trigger_flow_id = input.nullish_id("triggerFlowId");
    if input.failed() {
        return None;
    }
    Some(AiFunctionInput {
        name: name?,
        purpose: purpose.flatten(),
        data_collect: data_collect?,
        output_message: output_message.flatten(),
        trigger_flow_id: trigger_flow_id.flatten(),
    })
}

fn data_collect(input: &mut Input) -> Option<Vec<DataCollect>> {
    let items = match input.get("dataCollect").cloned() {
        Some(Value::Array(items)) => items,
        None => {
            input.issue(
                &key("dataCollect"),
                "invalid_type",
                "Invalid input: expected array, received undefined",
            );
            return None;
        }
        Some(_) => {
            input.issue(
                &key("dataCollect"),
                "invalid_type",
                "Invalid input: expected array",
            );
            return None;
        }
    };
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let path = vec![Seg::Key("dataCollect".into()), Seg::Index(index)];
        let Some(obj) = item.as_object() else {
            input.issue(&path, "invalid_type", "Invalid input: expected object");
            continue;
        };
        let spec = Str::trimmed(1, usize::MAX);
        let from = input.string_at(&child(&path, "from"), obj.get("from"), spec, true);
        let to = input.string_at(&child(&path, "to"), obj.get("to"), spec, true);
        if let (Some(from), Some(to)) = (from, to) {
            out.push(DataCollect { from, to });
        }
    }
    Some(out)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "aiFunctions.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let page = service::list(
                db,
                ws,
                PaginationInput {
                    page: Some(page),
                    per_page: Some(per_page),
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page.data.iter().map(ai_function_json).collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "aiFunctions.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let fn_id = input.id("id");
            input.finish()?;
            match service::find(db, ws, fn_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, FINDING))?
            {
                Some(row) => Ok(json(200, ai_function_json(&row))),
                None => Err(business_error(
                    chatbotx_business::BusinessError::not_found("AI function not found"),
                    FINDING,
                )),
            }
        }
        "aiFunctions.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_ai_function_input(&mut input);
            input.finish()?;
            service::create(db, &state.audit, ws, data.unwrap_or_default())
                .await
                .map(|row| json(201, ai_function_json(&row)))
                .map_err(|e| business_error(e, CREATING))
        }
        "aiFunctions.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_ai_function_input(&mut input);
            let fn_id = input.id("id");
            input.finish()?;
            service::update(
                db,
                &state.audit,
                ws,
                fn_id.unwrap_or_default(),
                data.unwrap_or_default(),
            )
            .await
            .map(|row| json(200, ai_function_json(&row)))
            .map_err(|e| business_error(e, MUTATING))
        }
        "aiFunctions.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let fn_id = input.id("id");
            input.finish()?;
            service::delete(db, &state.audit, ws, fn_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
