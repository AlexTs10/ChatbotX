//! `apps/builder/src/features/email-topics/api/public.ts` (Q.D5).

use chatbotx_business::email_topic::{self as service, ListEmailTopics};
use chatbotx_db::repo::models::EmailTopic;
use serde_json::{Value, json};

use super::input::{Input, Str};
use super::qr_codes::{public_paging, sort_query};
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "emailTopics.list",
    "emailTopics.get",
    "emailTopics.create",
    "emailTopics.update",
    "emailTopics.delete",
];

/// `possibleErrorsOnCreatingEmailTopic` / `possibleErrorsOnMutatingEmailTopic`.
const TOPIC_WRITE: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("nameTaken", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `emailTopicPublicResource` (the row without `workspaceId`).
pub fn topic_json(topic: &EmailTopic) -> Value {
    let mut out = topic.to_json();
    if let Value::Object(map) = &mut out {
        map.remove("workspaceId");
    }
    out
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "emailTopics.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let name = input.nullish_string("name", Str::trimmed(1, 255)).flatten();
            let folder_id = input.nullish_id("folderId").flatten();
            let sort = sort_query(&mut input, None);
            input.finish()?;
            let page = service::list(
                db,
                ws,
                ListEmailTopics {
                    name,
                    folder_id,
                    page: Some(page),
                    per_page: Some(per_page),
                    sort,
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page.data.iter().map(topic_json).collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "emailTopics.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id");
            input.finish()?;
            service::find_or_fail(db, ws, id.unwrap_or_default())
                .await
                .map(|topic| json(200, topic_json(&topic)))
                .map_err(|e| business_error(e, FINDING))
        }
        "emailTopics.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", Str::trimmed(1, 255));
            let folder_id = input.nullish_id("folderId").flatten();
            input.finish()?;
            // `rootFolderId` ("0") is the root.
            let folder_id = folder_id.filter(|id| *id != 0);
            service::create(db, ws, &name.unwrap_or_default(), folder_id)
                .await
                .map(|topic| json(201, json!({ "id": topic.id.to_string() })))
                .map_err(|e| business_error(e, TOPIC_WRITE))
        }
        "emailTopics.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id");
            let name = input.string("name", Str::trimmed(1, 255));
            input.finish()?;
            service::update(db, ws, id.unwrap_or_default(), &name.unwrap_or_default())
                .await
                .map(|topic| json(200, topic_json(&topic)))
                .map_err(|e| business_error(e, TOPIC_WRITE))
        }
        "emailTopics.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id");
            input.finish()?;
            let deleted = service::delete(db, ws, vec![id.unwrap_or_default()])
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            if deleted == 0 {
                return Err(business_error(
                    chatbotx_business::BusinessError::not_found("Email topic not found"),
                    MUTATING,
                ));
            }
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
