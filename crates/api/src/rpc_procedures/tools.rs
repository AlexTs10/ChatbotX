//! The workspace tools lists: polls (`features/polls/api/private.ts`,
//! the same operations as the public `polls.*` routes), questionnaires,
//! reflinks, QR codes and minigames (their list queries and actions). The
//! lists and most writes run the ported public `/v1` routes ([`public`]):
//! the builder's private procedure / action and the public route call the
//! same service.

use chatbotx_business::{minigame, qr_code, questionnaire, reflink};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, public, query_of};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "listPolls",
    "listQuestionnaires",
    "listReflinks",
    "listQrCodes",
    "listMinigames",
    "createQuestionnaireAction",
    "deleteQuestionnairesAction",
    "duplicateQuestionnaireAction",
    "renameQuestionnaireAction",
    "getQuestionnaire",
    "updateQuestionnaireAction",
    "createReflinkAction",
    "updateReflinkAction",
    "updateReflinkWidgetAction",
    "deleteReflinksAction",
    "deleteQrCodesAction",
    "deleteMinigamesAction",
    "enableMinigameAction",
];

/// `pollsAPI.*` → `polls.*`: `(route, path id?, GET)`.
fn poll_route(path: &str) -> Option<(&'static str, bool, bool)> {
    Some(match path {
        "pollsAPI.listPollsAPI" => ("polls.list", false, true),
        "pollsAPI.getPollAPI" => ("polls.get", true, true),
        "pollsAPI.createPollAPI" => ("polls.create", false, false),
        "pollsAPI.updatePollAPI" => ("polls.update", true, false),
        "pollsAPI.deletePollAPI" => ("polls.delete", true, false),
        "pollsAPI.bulkDeletePollsAPI" => ("polls.bulkDelete", false, false),
        "pollsAPI.getPollLinksAPI" => ("polls.getLinks", true, true),
        "pollsAPI.getPollAnalyticsAPI" => ("polls.getAnalytics", true, true),
        _ => return None,
    })
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let (route, with_id, get) = poll_route(path)?;
    let mut body = input.clone();
    if let Some(object) = body.as_object_mut() {
        object.remove("workspaceId");
    }
    let params = match with_id {
        true => match In(input).id("id") {
            Ok(id) => vec![("id", id.to_string())],
            Err(error) => return Some(Err(error)),
        },
        false => vec![],
    };
    Some(if get {
        let query = query_of(input, &["page", "perPage", "name"]);
        public(ctx, route, &params, Some(&query), None).await
    } else {
        public(ctx, route, &params, None, Some(&body)).await
    })
}

/// `?page=&perPage=&<keys>&sort[i][id]=&sort[i][desc]=` of a list search.
pub(super) fn list_query(search: &Value, keys: &[&str]) -> String {
    let mut all = vec!["page", "perPage"];
    all.extend_from_slice(keys);
    let mut query = query_of(search, &all);
    if let Some(items) = search.get("sort").and_then(Value::as_array) {
        for (index, item) in items.iter().enumerate() {
            let (Some(id), Some(desc)) = (
                item.get("id").and_then(Value::as_str),
                item.get("desc").and_then(Value::as_bool),
            ) else {
                continue;
            };
            let pair = query_of(&json!({ "id": id, "desc": desc }), &["id", "desc"]);
            for part in pair.split('&') {
                let (key, value) = part.split_once('=').unwrap_or((part, ""));
                if !query.is_empty() {
                    query.push('&');
                }
                query.push_str(&format!("sort%5B{index}%5D%5B{key}%5D={value}"));
            }
        }
    }
    query
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

/// The id bound at `index`.
fn bound(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    In(&json!({ "id": arg(args, index) })).id("id")
}

/// A route `validation` error as `validationErrors.name` (the reflink
/// actions' `returnValidationErrors`).
fn name_error(mut error: OrpcError) -> OrpcError {
    if error.code == "validation" {
        error.data = Some(json!({
            "validationErrors": {
                "_errors": ["Validation Exception"],
                "name": { "_errors": [error.message] },
            }
        }));
    }
    error
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    let search = arg(args, 0);
    let list = |route: &'static str, keys: &'static [&'static str]| async move {
        let query = list_query(search, keys);
        public(ctx, route, &[], Some(&query), None).await
    };
    let ids = || In(arg(args, 1)).ids("ids");
    let done =
        |result: chatbotx_business::Result<()>| result.map(|()| Value::Null).map_err(action_fail);
    match name {
        "listPolls" => list("polls.list", &["name"]).await,
        "listQuestionnaires" => list("questionnaires.list", &["name"]).await,
        "listReflinks" => list("reflinks.list", &["keyword"]).await,
        "listQrCodes" => list("qrCodes.list", &["keyword"]).await,
        "listMinigames" => list("minigames.list", &["name"]).await,
        "createQuestionnaireAction" => {
            let name = In(arg(args, 1)).str("name", true, 1, 255)?;
            questionnaire::create(db, ws, &name)
                .await
                .map(|id| json!({ "id": id.to_string() }))
                .map_err(action_fail)
        }
        "deleteQuestionnairesAction" => done(questionnaire::delete_many(db, ws, ids()?).await),
        "duplicateQuestionnaireAction" => questionnaire::duplicate(db, ws, bound(args, 1)?)
            .await
            .map(|id| json!({ "id": id.to_string() }))
            .map_err(action_fail),
        "renameQuestionnaireAction" => {
            let id = bound(args, 1)?;
            let name = In(arg(args, 2)).str("name", true, 1, 255)?;
            done(questionnaire::rename(db, ws, id, &name).await)
        }
        "getQuestionnaire" => {
            let id = In(search).id("id")?.to_string();
            public(ctx, "questionnaires.get", &[("id", id)], Some(""), None).await
        }
        "updateQuestionnaireAction" => {
            let id = bound(args, 1)?.to_string();
            public(
                ctx,
                "questionnaires.update",
                &[("id", id)],
                None,
                Some(arg(args, 2)),
            )
            .await
            .map(|_| Value::Null)
            .map_err(super::schema_errors)
        }
        "createReflinkAction" => public(ctx, "reflinks.create", &[], None, Some(arg(args, 1)))
            .await
            .map(|_| Value::Null)
            .map_err(name_error),
        "updateReflinkAction" => {
            let id = bound(args, 1)?.to_string();
            public(
                ctx,
                "reflinks.update",
                &[("id", id)],
                None,
                Some(arg(args, 2)),
            )
            .await
            .map(|_| Value::Null)
            .map_err(name_error)
        }
        "updateReflinkWidgetAction" => {
            let id = bound(args, 1)?.to_string();
            public(
                ctx,
                "reflinks.updateChatWidget",
                &[("id", id)],
                None,
                Some(arg(args, 2)),
            )
            .await
            .map(|_| Value::Null)
        }
        "deleteReflinksAction" => done(reflink::delete_many(db, ws, ids()?).await),
        "deleteQrCodesAction" => done(qr_code::delete_many(db, ws, ids()?).await),
        "deleteMinigamesAction" => done(minigame::delete_many(db, ws, ids()?).await),
        _ => {
            let id = bound(args, 1)?;
            let enabled = In(arg(args, 2))
                .opt_bool("enabled")?
                .ok_or_else(super::invalid)?;
            minigame::set_enabled(db, ws, id, enabled)
                .await
                .map(|_| Value::Null)
                .map_err(action_fail)
        }
    }
}
