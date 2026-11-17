//! `features/webhooks/api/public.ts` (`webhooksPublicRouter`, scope
//! `integrations`), plus the JSON and checks the `/rpc` actions share.

use chatbotx_business::webhook::{self as service, ConditionInput};
use chatbotx_db::repo::models::Webhook;
use chatbotx_db::repo::webhook::{ListWebhooksInput, WebhookWithConditions};
use serde_json::{Value, json};

use super::conditions::condition_at;
use super::input::{Input, Seg, Str, key};
use super::qr_codes::public_paging;
use super::tags::{array, digits_at};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json,
    no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "webhooks.list",
    "webhooks.create",
    "webhooks.get",
    "webhooks.update",
    "webhooks.delete",
    "webhooks.updateSettings",
    "webhooks.deleteMany",
];

/// `publicWebhookResource`.
pub fn public_json(w: &Webhook) -> Value {
    json!({
        "id": id(w.id),
        "name": w.name,
        "url": w.url,
        "active": w.active,
        "createdAt": date(w.created_at),
        "updatedAt": date(w.updated_at),
    })
}

/// `webhookWithConditionsResource`.
pub fn with_conditions_json(row: &WebhookWithConditions) -> Value {
    let mut out = public_json(&row.webhook);
    out["conditions"] = row
        .conditions
        .iter()
        .map(|c| {
            json!({
                "id": id(c.id),
                "type": c.r#type,
                "sourceId": c.source_id,
                "operator": c.operator,
                "value": c.value.as_ref().map_or(Value::Null, |v| v.0.clone()),
            })
        })
        .collect();
    out
}

/// The full row (`webhookResource`, `createSelectSchema(webhookModel)`).
pub fn row_json(w: &Webhook) -> Value {
    w.to_json()
}

/// `z.string().trim().url().max(1000)` (or `z.url().max(1000)`: `trim` false).
pub fn url(input: &mut Input, trim: bool) -> Option<String> {
    let value = input.get("url").cloned();
    let text = input.string_at(
        &key("url"),
        value.as_ref(),
        Str {
            trim,
            ..Str::default()
        },
        true,
    )?;
    if url::Url::parse(&text).is_err() {
        input.issue(&key("url"), "invalid_format", "Invalid URL");
        return None;
    }
    if text.chars().count() > 1000 {
        input.issue(
            &key("url"),
            "too_big",
            "Too big: expected string to have <=1000 characters",
        );
        return None;
    }
    Some(text)
}

/// `z.array(conditionSchema)` (`.min(min)`).
pub fn conditions(input: &mut Input, min: usize) -> Option<Vec<ConditionInput>> {
    array(input, "conditions", min, usize::MAX, condition_at)
}

/// `updateWebhookSettingsRequest`: `(name, active)`.
pub fn settings(input: &mut Input) -> (Option<String>, Option<bool>) {
    let name = input.opt_string("name", Str::trimmed(1, 255));
    let active = input.opt_bool("active");
    (name, active)
}

fn webhook_id(input: &mut Input) -> i64 {
    let value = input.get("id").cloned();
    digits_at(input, &[Seg::Key("id".into())], value.as_ref(), true).unwrap_or_default()
}

/// `findWebhookResource`.
pub async fn find(state: &ApiState, ws: i64, webhook: i64) -> chatbotx_business::Result<Value> {
    service::find_with_conditions_or_fail(&state.db, ws, webhook)
        .await
        .map(|row| with_conditions_json(&row))
}

/// `updateWithConditions` + its audit; `None` = not found.
pub async fn update(
    state: &ApiState,
    ws: i64,
    webhook: i64,
    url: String,
    conditions: Vec<ConditionInput>,
) -> chatbotx_business::Result<Option<Webhook>> {
    let out = service::update_with_conditions(&state.db, ws, webhook, url, conditions).await?;
    Ok(match out {
        Some((row, detail)) => {
            state.record_audit("update", &detail).await;
            Some(row)
        }
        None => None,
    })
}

/// `updateSettings` + its audit.
pub async fn update_settings(
    state: &ApiState,
    ws: i64,
    webhook: i64,
    name: Option<String>,
    active: Option<bool>,
) -> chatbotx_business::Result<()> {
    if let Some(detail) = service::update_settings(&state.db, ws, webhook, name, active).await? {
        state.record_audit("update", &detail).await;
    }
    Ok(())
}

/// `deleteMany` + its audit.
pub async fn delete_many(
    state: &ApiState,
    ws: i64,
    ids: Vec<i64>,
) -> chatbotx_business::Result<()> {
    if let Some(detail) = service::delete_many(&state.db, ws, ids).await? {
        state.record_audit("delete", &detail).await;
    }
    Ok(())
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    let db = &state.db;
    match route_id {
        "webhooks.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let list_input = ListWebhooksInput {
                workspace_id: ws,
                limit: per_page,
                offset: (page - 1) * per_page,
                ..ListWebhooksInput::default()
            };
            let (rows, page_count) = service::list(db, list_input)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = rows.iter().map(|r| public_json(&r.webhook)).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "webhooks.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", Str::trimmed(1, 255));
            let url = url(&mut input, true);
            let conditions = conditions(&mut input, 1);
            input.finish()?;
            let (row, detail) = service::register(
                db,
                ws,
                name.unwrap_or_default(),
                url.unwrap_or_default(),
                conditions.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, CREATING))?;
            state.record_audit("create", &detail).await;
            Ok(json(201, row_json(&row)))
        }
        "webhooks.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let webhook = webhook_id(&mut input);
            input.finish()?;
            find(state, ws, webhook)
                .await
                .map(|out| json(200, out))
                .map_err(|e| business_error(e, FINDING))
        }
        "webhooks.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let webhook = webhook_id(&mut input);
            let url = url(&mut input, true);
            let conditions = conditions(&mut input, 1);
            input.finish()?;
            let updated = update(
                state,
                ws,
                webhook,
                url.unwrap_or_default(),
                conditions.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            if updated.is_none() {
                return Err(business_error(
                    chatbotx_business::BusinessError::not_found("Webhook not found"),
                    MUTATING,
                ));
            }
            find(state, ws, webhook)
                .await
                .map(|out| json(200, out))
                .map_err(|e| business_error(e, MUTATING))
        }
        "webhooks.updateSettings" => {
            let mut input = Input::from_body(call.params, call.body);
            let webhook = webhook_id(&mut input);
            let (name, active) = settings(&mut input);
            input.finish()?;
            update_settings(state, ws, webhook, name, active)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            find(state, ws, webhook)
                .await
                .map(|out| json(200, out))
                .map_err(|e| business_error(e, MUTATING))
        }
        "webhooks.deleteMany" => {
            let mut input = Input::from_body(call.params, call.body);
            let ids = array(&mut input, "ids", 0, usize::MAX, |input, path, value| {
                digits_at(input, path, Some(value), true)
            });
            input.finish()?;
            delete_many(state, ws, ids.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "webhooks.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let webhook = webhook_id(&mut input);
            input.finish()?;
            let detail = service::unregister(db, ws, webhook)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            state.record_audit("delete", &detail).await;
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
