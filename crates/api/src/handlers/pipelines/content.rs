//! `pipelines.listComments`, `addComment`, `deleteComment`, `addAttachment`,
//! `removeAttachment`, `listActivity` (`api/public-items.ts`).

use chatbotx_business::pipeline::content::{self, ActivityView, ItemRef};
use chatbotx_db::repo::models::PipelineItemComment;
use chatbotx_db::repo::partials::pipeline::{PIPELINE_ACTIVITY_KINDS_BY_GROUP, activity_kinds_of};
use serde_json::{Value, json};

use super::super::input::{Input, Seg, Str, key};
use super::super::{
    ApiState, Call, FINDING, Reply, business_error, date, id, no_content, opt_date, opt_id,
};
use super::{IDEMPOTENCY_KEY, MUTATING_PIPELINE, TOKEN, money, no_nul, ok, publish, token_actor};
use crate::orpc::OrpcError;

/// `PIPELINE_LIMITS.comment`.
const COMMENT: Str = Str {
    pattern: Some(no_nul),
    ..Str::trimmed(1, 2000)
};

/// `groups` of `listActivityPublicRequest` (comma-separated).
fn groups_at(input: &mut Input) -> Option<Vec<String>> {
    let parts: Vec<String> = input
        .get("groups")?
        .as_str()?
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    if parts.is_empty() {
        return None;
    }
    let options: Vec<&str> = PIPELINE_ACTIVITY_KINDS_BY_GROUP
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let path = key("groups");
    if parts.len() > options.len() {
        input.issue(
            &path,
            "too_big",
            &format!("Too big: expected array to have <={} items", options.len()),
        );
        return None;
    }
    let mut out = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        let mut item_path = path.clone();
        item_path.push(Seg::Index(index));
        out.extend(input.enum_at(
            &item_path,
            Some(&Value::String(part.clone())),
            &options,
            true,
        ));
    }
    Some(out)
}

/// `listActivityPublicRequest.parse(...).groups`: `None` = every kind.
pub fn activity_groups_query(
    params: &[(&'static str, String)],
    query: Option<&str>,
) -> Result<Option<Vec<String>>, OrpcError> {
    let mut input = Input::from_query(params, query);
    let groups = groups_at(&mut input);
    input.finish()?;
    Ok(groups)
}

/// `toPublicComment`.
fn comment_json(row: &PipelineItemComment) -> Value {
    json!({
        "id": id(row.id),
        "itemId": id(row.item_id),
        "text": row.text,
        "createdById": opt_id(row.created_by_id),
        "createdAt": date(row.created_at),
    })
}

/// `toPublicActivity`.
fn activity_json(view: &ActivityView) -> Value {
    let row = &view.row;
    json!({
        "id": id(row.id),
        "kind": row.kind,
        "actorKind": row.actor_kind,
        "actorUserId": opt_id(row.actor_user_id),
        "fromStageId": opt_id(row.from_stage_id),
        "toStageId": opt_id(row.to_stage_id),
        "fromStatus": row.from_status,
        "toStatus": row.to_status,
        "fromValue": money(row.from_value),
        "toValue": money(row.to_value),
        "fromPriority": row.from_priority,
        "toPriority": row.to_priority,
        "fromDueDate": opt_date(row.from_due_date),
        "toDueDate": opt_date(row.to_due_date),
        "fromTitle": row.from_title,
        "toTitle": row.to_title,
        "fromPipelineId": opt_id(row.from_pipeline_id),
        "fromStageName": view.from_stage_name,
        "toStageName": view.to_stage_name,
        "fromPipelineName": view.from_pipeline_name,
        "pipelineName": view.pipeline_name,
        "occurredAt": date(row.occurred_at),
    })
}

fn target(workspace_id: i64, pipeline_id: Option<i64>, item_id: Option<i64>) -> ItemRef {
    ItemRef {
        workspace_id,
        pipeline_id: pipeline_id.unwrap_or_default(),
        item_id: item_id.unwrap_or_default(),
        scope: TOKEN,
    }
}

/// Runs the comment, attachment and activity routes.
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let finding = |e| business_error(e, FINDING);
    let mutating = |e| business_error(e, MUTATING_PIPELINE);
    match route_id {
        "pipelines.listComments" => {
            let mut input = Input::from_query(call.params, call.query);
            let item = target(ws, input.id("id"), input.id("itemId"));
            let cursor = input.opt_string("cursor", Str::default());
            input.finish()?;
            let page = content::list_comments(db, item, cursor)
                .await
                .map_err(finding)?;
            ok(
                200,
                json!({
                    "items": page.items.iter().map(comment_json).collect::<Vec<_>>(),
                    "nextCursor": page.next_cursor,
                }),
            )
        }
        "pipelines.addComment" => {
            let mut input = Input::from_body(call.params, call.body);
            let item = target(ws, input.id("id"), input.id("itemId"));
            let text = input.string("text", COMMENT);
            let idempotency_key = input.opt_string("idempotencyKey", IDEMPOTENCY_KEY);
            input.finish()?;
            let added = content::add_comment(
                db,
                item,
                token_actor(&call),
                text.unwrap_or_default(),
                idempotency_key,
            )
            .await
            .map_err(mutating)?;
            publish(state, &added).await;
            ok(201, comment_json(&added.value))
        }
        "pipelines.deleteComment" => {
            let mut input = super::delete_input(&call);
            let item = target(ws, input.id("id"), input.id("itemId"));
            let comment_id = input.id("commentId");
            input.finish()?;
            let done = content::delete_comment(
                db,
                item,
                token_actor(&call),
                comment_id.unwrap_or_default(),
            )
            .await
            .map_err(mutating)?;
            publish(state, &done).await;
            Ok(no_content())
        }
        "pipelines.addAttachment" => {
            let mut input = Input::from_body(call.params, call.body);
            let item = target(ws, input.id("id"), input.id("itemId"));
            let file_id = input.id("mediaLibraryFileId");
            input.finish()?;
            let done = content::add_attachments(
                db,
                item,
                token_actor(&call),
                vec![file_id.unwrap_or_default()],
            )
            .await
            .map_err(mutating)?;
            publish(state, &done).await;
            Ok(no_content())
        }
        "pipelines.removeAttachment" => {
            let mut input = super::delete_input(&call);
            let item = target(ws, input.id("id"), input.id("itemId"));
            let file_id = input.id("mediaLibraryFileId");
            input.finish()?;
            let done = content::remove_attachment(
                db,
                item,
                token_actor(&call),
                file_id.unwrap_or_default(),
            )
            .await
            .map_err(mutating)?;
            publish(state, &done).await;
            Ok(no_content())
        }
        "pipelines.listActivity" => {
            let mut input = Input::from_query(call.params, call.query);
            let item = target(ws, input.id("id"), input.id("itemId"));
            let cursor = input.opt_string("cursor", Str::default());
            let groups = groups_at(&mut input);
            input.finish()?;
            let names: Option<Vec<&str>> = groups
                .as_ref()
                .map(|groups| groups.iter().map(String::as_str).collect());
            let kinds = activity_kinds_of(names.as_deref())
                .map(|kinds| kinds.into_iter().map(str::to_string).collect());
            let page = content::list_activity(db, item, cursor, kinds)
                .await
                .map_err(finding)?;
            ok(
                200,
                json!({
                    "items": page.items.iter().map(activity_json).collect::<Vec<_>>(),
                    "nextCursor": page.next_cursor,
                }),
            )
        }
        _ => Err(super::super::internal_error(route_id)),
    }
}
