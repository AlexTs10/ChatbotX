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
    "findQrCode",
    "createQrCodeAction",
    "updateQrCodeAction",
    "findReflink",
    "findMinigame",
    "listMinigameHistory",
    "createMinigameAction",
    "updateMinigameAction",
    "getMinigamePlaysAction",
    "findContactConversationAction",
    "listQuestionnaireSubmissions",
    "getQuestionnaireDashboardSummary",
    "getQuestionnaireSubmissionDetailAction",
    "deleteQuestionnaireSubmissionAction",
    "deleteQuestionnaireSubmissionsAction",
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
fn list_query(search: &Value, keys: &[&str]) -> String {
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
        "findQrCode" => Ok(qr_code::find(db, ws, In(search).id("id")?)
            .await
            .map_err(action_fail)?
            .map_or(Value::Null, |row| row.to_json())),
        // `findReflink`: `undefined` (here `null`) for "no such reflink".
        "findReflink" => match reflink::find_or_fail(db, ws, In(search).id("id")?).await {
            Ok(row) => Ok(row.to_json()),
            Err(error) if error.code() == Some("notFound") => Ok(Value::Null),
            Err(error) => Err(action_fail(error)),
        },
        "createQrCodeAction" | "updateQrCodeAction" => qr_code_write(ctx, name, args).await,
        "findMinigame" => match minigame::find(db, ws, In(search).id("id")?).await {
            Ok(row) => Ok(row.to_json()),
            Err(error) if error.code() == Some("notFound") => Ok(Value::Null),
            Err(error) => Err(action_fail(error)),
        },
        "listMinigameHistory" => minigame_history(ctx, search).await,
        "createMinigameAction" | "updateMinigameAction" => minigame_write(ctx, name, args).await,
        "getMinigamePlaysAction" => {
            let input = In(arg(args, 1));
            let plays = minigame::contact::list_plays(
                db,
                ws,
                input.id("minigameId")?,
                input.id("contactId")?,
            )
            .await
            .map_err(action_fail)?;
            Ok(plays
                .iter()
                .map(|play| {
                    json!({
                        "id": play.id.to_string(),
                        "isWinning": play.is_winning,
                        "prizeName": play.prize_name,
                        "createdAt": chatbotx_db::repo::iso(play.created_at),
                    })
                })
                .collect())
        }
        "findContactConversationAction" => {
            let input = In(arg(args, 1));
            let contact_id = input.id("contactId")?;
            let Some(contact_inbox_id) = input.opt_id("contactInboxId")? else {
                return Ok(json!({ "conversationId": null }));
            };
            // `findDMByContactInbox`: the contact inbox must exist.
            let exists = chatbotx_business::tx::read(db, move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT 1 FROM "ContactInbox" WHERE "id" = ?1"#,
                        [contact_inbox_id],
                        |_| Ok(()),
                    )
                    .is_ok())
            })
            .await
            .map_err(action_fail)?;
            let id = match exists {
                true => super::dm_conversation_id(ctx, contact_id).await?,
                false => None,
            };
            Ok(json!({ "conversationId": id.map(|id| id.to_string()) }))
        }
        "listQuestionnaireSubmissions" => {
            let id = In(search).id("questionnaireId")?.to_string();
            let query = list_query(search, &["name"]);
            public(
                ctx,
                "questionnaires.listSubmissions",
                &[("id", id)],
                Some(&query),
                None,
            )
            .await
        }
        "getQuestionnaireDashboardSummary" => {
            let id = In(search).id("questionnaireId")?.to_string();
            public(
                ctx,
                "questionnaires.getSubmissionStats",
                &[("id", id)],
                Some(""),
                None,
            )
            .await
        }
        "getQuestionnaireSubmissionDetailAction" | "deleteQuestionnaireSubmissionAction" => {
            let input = In(arg(args, 1));
            let params = [
                ("id", input.id("questionnaireId")?.to_string()),
                ("submissionId", input.id("submissionId")?.to_string()),
            ];
            let route = match name {
                "getQuestionnaireSubmissionDetailAction" => "questionnaires.getSubmission",
                _ => "questionnaires.deleteSubmission",
            };
            public(ctx, route, &params, Some(""), None).await
        }
        "deleteQuestionnaireSubmissionsAction" => {
            let input = In(arg(args, 1));
            let questionnaire_id = input.id("questionnaireId")?;
            let ids = input.ids("submissionIds")?;
            if ids.is_empty() {
                return Err(super::invalid());
            }
            done(questionnaire::submission::delete_submissions(db, ws, questionnaire_id, ids).await)
        }
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

/// `createQrCodeAction` / `updateQrCodeAction`: a `validation` error goes to
/// its field (`name` or `flowId`).
async fn qr_code_write(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    use crate::handlers::input::Input;
    use crate::handlers::qr_codes::{DUPLICATE_NAME, alnum_name, size};
    let create = name == "createQrCodeAction";
    let body = arg(args, if create { 1 } else { 2 }).to_string();
    let mut input = Input::from_body(&[], body.as_bytes());
    let qr_name = alnum_name(&mut input, create);
    let flow_id = if create || input.has("flowId") {
        input.id("flowId")
    } else {
        None
    };
    let size = size(&mut input).or(create.then_some(qr_code::DEFAULT_SIZE));
    input.finish().map_err(super::schema_errors)?;
    let data = qr_code::QrCodeData {
        name: qr_name,
        flow_id,
        custom_field_id: None,
        size,
    };
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    if create {
        let id = qr_code::create(db, ws, data, DUPLICATE_NAME)
            .await
            .map_err(action_fail)?;
        return Ok(json!({ "id": id.to_string() }));
    }
    qr_code::update(db, ws, bound(args, 1)?, data, DUPLICATE_NAME)
        .await
        .map_err(action_fail)?;
    Ok(Value::Null)
}

/// `createMinigameAction` / `updateMinigameAction` (bind args `[workspaceId,
/// id, originalPrizeQuantities]`): `nameAlreadyExists` is the
/// `generalSettings.name` validation error.
async fn minigame_write(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    use crate::handlers::input::Input;
    let create = name == "createMinigameAction";
    let body = arg(args, if create { 1 } else { 3 }).to_string();
    let mut input = Input::from_body(&[], body.as_bytes());
    let data = crate::handlers::minigames::minigame_data(&mut input);
    input.finish().map_err(super::schema_errors)?;
    let data = data.ok_or_else(super::invalid)?;
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    let result = if create {
        minigame::create(db, ws, data).await
    } else {
        let original = match arg(args, 2) {
            Value::Object(map) => map
                .iter()
                .map(|(key, value)| (key.clone(), value.as_f64()))
                .collect(),
            _ => Default::default(),
        };
        minigame::update(db, ws, bound(args, 1)?, data, Some(original)).await
    };
    match result {
        Ok(row) if create => Ok(json!({ "id": row.id.to_string() })),
        Ok(_) => Ok(Value::Null),
        Err(error) if error.code() == Some("nameAlreadyExists") => {
            let mut out = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
            out.data = Some(json!({
                "validationErrors": {
                    "generalSettings": {
                        "name": { "_errors": ["Minigame name already exists"] },
                    },
                }
            }));
            Err(out)
        }
        Err(error) => Err(action_fail(error)),
    }
}

/// `listMinigameHistory` (`minigameContactService.list`).
async fn minigame_history(ctx: &Ctx<'_>, search: &Value) -> Result<Value, OrpcError> {
    let input = In(search);
    let page = minigame::contact::list(
        &ctx.api.db,
        minigame::contact::ListPlayers {
            workspace_id: ctx.workspace_id,
            minigame_id: input.id("minigameId")?,
            page: input.opt_positive("page")?,
            per_page: input.opt_positive("perPage")?,
            name: input.opt_str("name")?,
            sort: input.sort(),
        },
    )
    .await
    .map_err(action_fail)?;
    let iso = chatbotx_db::repo::iso;
    let data: Vec<Value> = page
        .data
        .iter()
        .map(|player| {
            json!({
                "id": player.id.to_string(),
                "contactId": player.contact_id.to_string(),
                "contactInboxId": player.contact_inbox_id.map(|id| id.to_string()),
                "played": player.played,
                "remaining": player.remaining,
                "sharesCount": player.shares_count,
                "openedAt": iso(player.opened_at),
                "lastPlayedAt": iso(player.last_played_at),
                "contact": {
                    "id": player.contact.id.to_string(),
                    "fullName": player.contact.full_name,
                    "firstName": player.contact.first_name,
                    "lastName": player.contact.last_name,
                    "avatar": player.contact.avatar,
                },
            })
        })
        .collect();
    Ok(super::page(data, page.page_count))
}
