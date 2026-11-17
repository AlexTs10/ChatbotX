//! `features/shared/comment-automation/api/public.ts`
//! (`commentAutomationsPublicRouter`).

use chatbotx_business::comment_automation::{self as service, LogFilter};
use chatbotx_business::connection::credentials::HttpUpstream;
use serde_json::{Value, json};

use super::input::{Input, key};
use super::polls::datetime_at;
use super::qr_codes::public_paging;
use super::{
    ApiState, Call, FINDING, LISTING, MUTATING, Reply, business_error, internal_error, json,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "commentAutomations.listAccounts",
    "commentAutomations.processMissedComments",
    "commentAutomations.getMissedCommentRun",
    "commentAutomations.listModerationLogs",
    "commentAutomations.getModerationSummary",
];

/// `commentAutomationTypes`.
const TYPES: &[&str] = &[
    "messenger",
    "instagram",
    "instagramFacebook",
    "threads",
    "tiktok",
];

fn filter(input: &mut Input) -> LogFilter {
    let from = datetime_at(input, "from", false);
    let to = datetime_at(input, "to", false);
    let value = input.get("action").cloned();
    let action = input.enum_at(&key("action"), value.as_ref(), &["hide", "delete"], false);
    let value = input.get("source").cloned();
    let source = input.enum_at(&key("source"), value.as_ref(), &["rule", "ai"], false);
    LogFilter {
        from,
        to,
        action,
        source,
    }
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let mut input = if route_id == "commentAutomations.processMissedComments" {
        Input::from_body(call.params, call.body)
    } else {
        Input::from_query(call.params, call.query)
    };
    if route_id == "commentAutomations.listAccounts" {
        let value = input.get("type").cloned();
        let kind = input.enum_at(&key("type"), value.as_ref(), TYPES, true);
        input.finish()?;
        let accounts = service::list_accounts(db, ws, &kind.unwrap_or_default())
            .await
            .map_err(|e| business_error(e, LISTING))?;
        let accounts: Vec<Value> = accounts
            .into_iter()
            .map(|(inbox_id, name)| json!({ "inboxId": inbox_id.to_string(), "name": name }))
            .collect();
        return Ok(json(200, json!({ "accounts": accounts })));
    }
    let id = input.digits_id("id").unwrap_or(-1);
    match route_id {
        "commentAutomations.processMissedComments" => {
            input.finish()?;
            let (Some(kv), Some(jobs)) = (state.kv.as_ref(), state.jobs.as_ref()) else {
                return Err(internal_error("kv or job queue is off"));
            };
            service::process_missed_comments(db, &kv.durable, jobs.as_ref(), &HttpUpstream, ws, id)
                .await
                .map(|result| json(200, result.to_json()))
                .map_err(|e| business_error(e, MUTATING))
        }
        "commentAutomations.getMissedCommentRun" => {
            input.finish()?;
            let automation = service::find_or_fail(db, ws, id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let kv = state
                .kv
                .as_ref()
                .ok_or_else(|| internal_error("kv is off"))?;
            let running = service::missed_comments_in_progress(&kv.durable, automation.id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, json!({ "inProgress": running })))
        }
        "commentAutomations.listModerationLogs" => {
            let filter = filter(&mut input);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let (data, total) = service::list_moderation_logs(db, ws, id, filter, page, per_page)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let page_count = (total + per_page - 1) / per_page;
            Ok(json(
                200,
                json!({ "data": data, "total": total, "page": page, "pageCount": page_count }),
            ))
        }
        _ => {
            let filter = filter(&mut input);
            input.finish()?;
            service::moderation_summary(db, ws, id, filter)
                .await
                .map(|summary| json(200, summary))
                .map_err(|e| business_error(e, FINDING))
        }
    }
}
