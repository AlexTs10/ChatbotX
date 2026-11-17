//! The comment and story automation screens (`features/{fb,ig,threads,tiktok}-comments`,
//! `features/ig-stories`, `features/shared/comment-automation`): the list /
//! get queries, the create / update / delete / bulk delete actions, the
//! missed-comments actions and the private procedures. Every call runs the
//! ported public `/v1` route ([`public`]): the builder's private procedure /
//! action and the public route call the same service with the same schema.

use chatbotx_business::comment_automation as service;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, arg, public, schema_errors};
use crate::orpc::OrpcError;

/// `(query or action, public route group, operation)`.
const ROUTES: &[(&str, &str, &str)] = &[
    ("listFbComments", "fbComments", "list"),
    ("getFbComment", "fbComments", "get"),
    ("createFbCommentAction", "fbComments", "create"),
    ("updateFbCommentAction", "fbComments", "update"),
    ("deleteFbCommentAction", "fbComments", "delete"),
    ("bulkDeleteFbCommentsAction", "fbComments", "deleteMany"),
    ("listIgComments", "igComments", "list"),
    ("getIgComment", "igComments", "get"),
    ("createIgCommentAction", "igComments", "create"),
    ("updateIgCommentAction", "igComments", "update"),
    ("deleteIgCommentAction", "igComments", "delete"),
    ("bulkDeleteIgCommentsAction", "igComments", "deleteMany"),
    ("listIgStories", "igStories", "list"),
    ("getIgStory", "igStories", "get"),
    ("createIgStoryAction", "igStories", "create"),
    ("updateIgStoryAction", "igStories", "update"),
    ("deleteIgStoryAction", "igStories", "delete"),
    ("bulkDeleteIgStoriesAction", "igStories", "deleteMany"),
    ("listThreadsComments", "threadsComments", "list"),
    ("getThreadsComment", "threadsComments", "get"),
    ("createThreadsCommentAction", "threadsComments", "create"),
    ("updateThreadsCommentAction", "threadsComments", "update"),
    ("deleteThreadsCommentAction", "threadsComments", "delete"),
    ("listTiktokComments", "tiktokComments", "list"),
    ("getTiktokComment", "tiktokComments", "get"),
    ("createTiktokCommentAction", "tiktokComments", "create"),
    ("updateTiktokCommentAction", "tiktokComments", "update"),
    ("deleteTiktokCommentAction", "tiktokComments", "delete"),
];

const MISSED: &[&str] = &[
    "getMissedCommentsInProgressAction",
    "processMissedCommentsAction",
];

/// `(route group, operation)` of a query or action name.
fn route_of(name: &str) -> Option<(&'static str, &'static str)> {
    ROUTES
        .iter()
        .find(|(n, ..)| *n == name)
        .map(|(_, group, op)| (*group, *op))
}

/// Whether `name` is one of this module's actions (the no-workspace check).
pub(super) fn is_action(name: &str) -> bool {
    route_of(name).is_some() || MISSED.contains(&name)
}

/// The id bound at `index`.
fn bound(args: &[Value], index: usize) -> Result<String, OrpcError> {
    In(&json!({ "id": arg(args, index) }))
        .id("id")
        .map(|id| id.to_string())
}

/// The list / filter keys a screen's search sends.
const LIST_KEYS: &[&str] = &["name", "isActive", "inboxId", "folderId", "trigger"];

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if let Some((group, op)) = route_of(name) {
        return Some(run(ctx, group, op, args).await);
    }
    match name {
        "processMissedCommentsAction" => Some(match bound(args, 1) {
            Ok(id) => {
                public(
                    ctx,
                    "commentAutomations.processMissedComments",
                    &[("id", id)],
                    None,
                    Some(&json!({})),
                )
                .await
            }
            Err(error) => Err(error),
        }),
        "getMissedCommentsInProgressAction" => Some(in_progress(ctx, args).await),
        _ => None,
    }
}

async fn run(
    ctx: &Ctx<'_>,
    group: &'static str,
    op: &'static str,
    args: &[Value],
) -> Result<Value, OrpcError> {
    let route = format!("{group}.{op}");
    match op {
        "list" => {
            let query = super::tools::list_query(arg(args, 0), LIST_KEYS);
            public(ctx, &route, &[], Some(&query), None).await
        }
        "get" => {
            let id = In(arg(args, 0)).id("id")?.to_string();
            public(ctx, &route, &[("id", id)], Some(""), None).await
        }
        "create" => public(ctx, &route, &[], None, Some(arg(args, 1)))
            .await
            .map(|row| json!({ "id": row.get("id").cloned().unwrap_or(Value::Null) }))
            .map_err(schema_errors),
        "update" => {
            let id = bound(args, 1)?;
            public(ctx, &route, &[("id", id)], None, Some(arg(args, 2)))
                .await
                .map(|_| Value::Null)
                .map_err(schema_errors)
        }
        "delete" => {
            let id = bound(args, 1)?;
            public(ctx, &route, &[("id", id)], Some(""), None)
                .await
                .map(|_| Value::Null)
        }
        _ => public(ctx, &route, &[], None, Some(arg(args, 1)))
            .await
            .map(|_| Value::Null)
            .map_err(schema_errors),
    }
}

/// `findMissedCommentsInProgress`: the ids (of this workspace, at most 500)
/// whose missed-comments run holds its lock.
async fn in_progress(ctx: &Ctx<'_>, args: &[Value]) -> Result<Value, OrpcError> {
    let ids = In(arg(args, 1)).ids("ids")?;
    if ids.len() > 500 {
        return Err(super::invalid());
    }
    let Some(kv) = ctx.api.kv.as_ref() else {
        return Ok(json!({ "ids": [] }));
    };
    let mut out = Vec::new();
    for id in ids {
        let running = service::missed_comments_in_progress(&kv.durable, id)
            .await
            .map_err(super::fail)?;
        // ponytail: the workspace check only for running ids (rare), not per listed id.
        if running
            && service::find_or_fail(&ctx.api.db, ctx.workspace_id, id)
                .await
                .is_ok()
        {
            out.push(Value::String(id.to_string()));
        }
    }
    Ok(json!({ "ids": out }))
}

/// The private procedures of the screens: `(public route, path id key)`.
fn procedure_route(path: &str) -> Option<(&'static str, Option<&'static str>)> {
    Some(match path {
        "commentAutomationsAPI.automationAccountsAPI" => ("commentAutomations.listAccounts", None),
        "commentAutomationsAPI.moderationLogsAPI" => {
            ("commentAutomations.listModerationLogs", Some("id"))
        }
        "commentAutomationsAPI.moderationSummaryAPI" => {
            ("commentAutomations.getModerationSummary", Some("id"))
        }
        "igCommentsAPI.instagramMediaAPI" => ("igComments.listMedia", None),
        "igStoriesAPI.instagramStoriesAPI" => ("igStories.listStories", None),
        "fbCommentsAPI.facebookPostsAPI" => ("fbComments.listPosts", None),
        "threadsCommentsAPI.threadsPostsAPI" => ("threadsComments.listPosts", None),
        "fbCommentsAPI.privateListCommentAutomationContactsAPI" => {
            ("analytics.commentAutomationContacts", None)
        }
        "analyticsRoutes.commentAutomationReplyStats" => {
            ("analytics.commentAutomationReplyStats", None)
        }
        "analyticsRoutes.commentAutomationUserComments" => {
            ("analytics.commentAutomationUserComments", None)
        }
        "analyticsRoutes.commentAutomationBotReplies" => {
            ("analytics.commentAutomationBotReplies", None)
        }
        "analyticsRoutes.commentAutomationErrors" => ("analytics.commentAutomationErrors", None),
        _ => return None,
    })
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let (route, id_key) = procedure_route(path)?;
    let params = match id_key {
        Some(key) => match In(input).id(key) {
            Ok(id) => vec![(key, id.to_string())],
            Err(error) => return Some(Err(error)),
        },
        None => vec![],
    };
    let mut rest = input.clone();
    if let (Some(object), Some(key)) = (rest.as_object_mut(), id_key) {
        object.remove(key);
    }
    let query = super::analytics::query(&rest);
    Some(
        public(ctx, route, &params, Some(&query), None)
            .await
            .map_err(|error| match error.status {
                422 => super::invalid(),
                _ => error,
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_screen_names_to_public_routes() {
        assert_eq!(route_of("listFbComments"), Some(("fbComments", "list")));
        assert_eq!(route_of("getIgStory"), Some(("igStories", "get")));
        assert_eq!(
            route_of("bulkDeleteIgStoriesAction"),
            Some(("igStories", "deleteMany"))
        );
        assert_eq!(
            route_of("updateTiktokCommentAction"),
            Some(("tiktokComments", "update"))
        );
        assert_eq!(route_of("bulkDeleteThreadsCommentsAction"), None);
        assert_eq!(route_of("listPolls"), None);
        assert!(is_action("processMissedCommentsAction"));
    }
}
