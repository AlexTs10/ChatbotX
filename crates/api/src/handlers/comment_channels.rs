//! `features/threads-comments/api/public.ts` and
//! `features/tiktok-comments/api/public.ts` (`threadsComments.*`,
//! `tiktokComments.*`): `createThreadsCommentRequestSchema` /
//! `createTiktokCommentRequestSchema` (trimmed, de-duplicated lists and their
//! refinements) and the shared CRUD of `comment_automation_crud`.

use chatbotx_business::comment_automation_crud::{self as crud, COMMENTS, build_channel_write};
use serde_json::{Map, Value, json};

use super::comment_crud::{
    REPLY_AFTER, SHOW_AFTER, ai_moderation, bool_field, enum_field, ids, idx, list_filter,
    nullish_digits, object, page,
};
use super::input::{Input, Num, Seg, Str, child, key};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The routes of this module (`threadsComments.listPosts` is in `comment_crud`).
pub const ROUTES: &[&str] = &[
    "threadsComments.list",
    "threadsComments.create",
    "threadsComments.get",
    "threadsComments.update",
    "threadsComments.delete",
    "threadsComments.deleteMany",
    "tiktokComments.list",
    "tiktokComments.create",
    "tiktokComments.get",
    "tiktokComments.update",
    "tiktokComments.delete",
    "tiktokComments.deleteMany",
];

const MAX_REPLY: usize = 2000;

struct Channel {
    kind: &'static str,
    types: &'static [&'static str],
    not_found: &'static str,
    /// `…CommentAutomation.validation.` message key prefix.
    messages: &'static str,
}

const THREADS: Channel = Channel {
    kind: "threads",
    types: &["threads"],
    not_found: "Threads Comment Automation not found",
    messages: "threadsCommentAutomation.validation.",
};
const TIKTOK: Channel = Channel {
    kind: "tiktok",
    types: &["tiktok"],
    not_found: "TikTok Comment Automation not found",
    messages: "tiktokCommentAutomation.validation.",
};

fn message(channel: &Channel, name: &str) -> String {
    format!("{}{name}", channel.messages)
}

/// `trimmedArray(maxItems, maxLength)`: trimmed, non-empty, de-duplicated.
fn trimmed_array(
    input: &mut Input,
    path: &[Seg],
    value: Option<&Value>,
    max_items: usize,
    max_len: usize,
) -> Option<Vec<String>> {
    let Some(Value::Array(items)) = value else {
        let got = super::input::type_name(value);
        input.issue(
            path,
            "invalid_type",
            &format!("Invalid input: expected array, received {got}"),
        );
        return None;
    };
    let before = input.issue_count();
    let mut out: Vec<String> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if let Some(text) =
            input.string_at(&idx(path, i), Some(item), Str::trimmed(1, max_len), true)
            && !out.contains(&text)
        {
            out.push(text);
        }
    }
    if items.len() > max_items {
        input.issue(
            path,
            "too_big",
            &format!("Too big: expected array to have <={max_items} items"),
        );
    }
    (input.issue_count() == before).then_some(out)
}

fn digits_at(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<String> {
    let text = input.string_at(path, value, Str::default(), true)?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        input.issue(
            path,
            "invalid_format",
            "Invalid string: must match pattern /^\\d+$/",
        );
        return None;
    }
    Some(text)
}

/// The reply unions (`threadsReplySchema`, TikTok's public / private reply).
fn reply(
    input: &mut Input,
    channel: &Channel,
    path: &[Seg],
    value: Option<&Value>,
    private: bool,
) -> Option<Value> {
    let obj = object(input, path, value, true)?;
    let allowed: &[&str] = if private {
        &["none", "text", "AIAgent"]
    } else {
        &["none", "text", "flow", "AIAgent"]
    };
    let kind = enum_field(input, &obj, path, "type", allowed, true)?;
    let at = child(path, "value");
    let kind = kind.as_str().unwrap_or_default().to_string();
    match kind.as_str() {
        "none" => match obj.get("value") {
            Some(Value::Null) => Some(json!({ "type": "none", "value": null })),
            other => {
                let got = super::input::type_name(other);
                input.issue(
                    &at,
                    "invalid_type",
                    &format!("Invalid input: expected null, received {got}"),
                );
                None
            }
        },
        "flow" | "AIAgent" => {
            digits_at(input, &at, obj.get("value")).map(|id| json!({ "type": kind, "value": id }))
        }
        _ => {
            let min = usize::from(private);
            let before = input.issue_count();
            let text = input.string_at(&at, obj.get("value"), Str::trimmed(min, MAX_REPLY), true);
            let mut values = None;
            if !private && let Some(list) = obj.get("values") {
                let vat = child(path, "values");
                match list {
                    Value::Array(items) => {
                        let mut parsed = Vec::new();
                        for (i, item) in items.iter().enumerate() {
                            let ip = idx(&vat, i);
                            if let Some(entry) = object(input, &ip, Some(item), true)
                                && let Some(t) = input.string_at(
                                    &child(&ip, "value"),
                                    entry.get("value"),
                                    Str::trimmed(0, MAX_REPLY),
                                    true,
                                )
                            {
                                parsed.push(t);
                            }
                        }
                        if items.len() > 10 {
                            input.issue(
                                &vat,
                                "too_big",
                                "Too big: expected array to have <=10 items",
                            );
                        }
                        values = Some(parsed);
                    }
                    other => {
                        let got = super::input::type_name(Some(other));
                        input.issue(
                            &vat,
                            "invalid_type",
                            &format!("Invalid input: expected array, received {got}"),
                        );
                    }
                }
            }
            if input.issue_count() != before {
                return None;
            }
            let text = text?;
            // `resolveReplyTexts(...).length > 0`.
            let texts: Vec<&String> = values
                .as_ref()
                .map_or_else(|| vec![&text], |v| v.iter().collect());
            if !private && texts.iter().all(|t| t.trim().is_empty()) {
                input.issue(
                    &child(path, "values"),
                    "custom",
                    &message(channel, "replyTextRequired"),
                );
                return None;
            }
            let mut out = json!({ "type": "text", "value": text });
            if let Some(values) = values {
                out["values"] =
                    Value::Array(values.into_iter().map(|v| json!({ "value": v })).collect());
            }
            Some(out)
        }
    }
}

fn post(
    input: &mut Input,
    channel: &Channel,
    path: &[Seg],
    value: Option<&Value>,
) -> Option<Value> {
    let obj = object(input, path, value, true)?;
    let kind = enum_field(input, &obj, path, "type", &["all", "postIds"], true);
    let values = trimmed_array(input, &child(path, "value"), obj.get("value"), 50, 120);
    let (kind, values) = (kind?, values?);
    if kind == "all" && !values.is_empty() {
        input.issue(
            &child(path, "value"),
            "custom",
            &message(channel, "postIdsMustBeEmptyForAll"),
        );
        return None;
    }
    if kind == "postIds" && values.is_empty() {
        input.issue(
            &child(path, "value"),
            "custom",
            &message(channel, "postIdsRequired"),
        );
        return None;
    }
    Some(json!({ "type": kind, "value": values }))
}

fn include_keywords(
    input: &mut Input,
    channel: &Channel,
    path: &[Seg],
    value: Option<&Value>,
) -> Option<Value> {
    let obj = object(input, path, value, true)?;
    let before = input.issue_count();
    let kind = enum_field(
        input,
        &obj,
        path,
        "type",
        &["all", "equal", "contain", "mentions"],
        true,
    );
    let values = trimmed_array(input, &child(path, "value"), obj.get("value"), 25, 120);
    let mentions = obj.get("mentionCount").and_then(|v| {
        input.coerce_number_at(
            &child(path, "mentionCount"),
            Some(v),
            Num::int(1.0, 5.0),
            true,
        )
    });
    if input.issue_count() != before {
        return None;
    }
    let (kind, values) = (kind?, values?);
    if kind != "mentions" {
        if kind == "all" && !values.is_empty() {
            input.issue(
                &child(path, "value"),
                "custom",
                &message(channel, "keywordsMustBeEmptyForAll"),
            );
            return None;
        }
        if kind != "all" && values.is_empty() {
            input.issue(
                &child(path, "value"),
                "custom",
                &message(channel, "keywordsRequired"),
            );
            return None;
        }
    }
    let mut out = json!({ "type": kind, "value": values });
    if let Some(count) = mentions {
        out["mentionCount"] = json!(count as i64);
    }
    Some(out)
}

fn options(
    input: &mut Input,
    channel: &Channel,
    path: &[Seg],
    value: Option<&Value>,
) -> Option<Value> {
    let obj = object(input, path, value, true)?;
    let before = input.issue_count();
    let mut out = Map::new();
    let mut required = vec![
        "replyToNewContactsOnly",
        "replyOncePerUserPerPost",
        "replyToUsersWhoCommentedOnOtherPosts",
        "ignoreCommentReplies",
    ];
    let mut optional = vec!["trackUserTags"];
    if channel.kind == "tiktok" {
        required.push("likeUserComment");
    } else {
        optional.push("onlyCommentsWithImage");
    }
    for name in required {
        if let Some(flag) = bool_field(input, &obj, path, name, true) {
            out.insert(name.into(), flag);
        }
    }
    for name in optional {
        if let Some(flag) = bool_field(input, &obj, path, name, false) {
            out.insert(name.into(), flag);
        }
    }
    (input.issue_count() == before).then_some(Value::Object(out))
}

fn hide_comments(
    input: &mut Input,
    channel: &Channel,
    path: &[Seg],
    value: Option<&Value>,
) -> Option<Value> {
    let obj = object(input, path, value, true)?;
    let before = input.issue_count();
    let mut out = Map::new();
    for name in ["all", "hasPhoneNumber", "hasLink", "hasKeywords"] {
        if let Some(flag) = bool_field(input, &obj, path, name, true) {
            out.insert(name.into(), flag);
        }
    }
    let optional: &[&str] = if channel.kind == "threads" {
        &["hasGif", "hasEmoji"]
    } else {
        &["hasEmoji"]
    };
    for name in optional {
        if let Some(flag) = bool_field(input, &obj, path, name, false) {
            out.insert((*name).into(), flag);
        }
    }
    if let Some(moderation) = obj.get("aiModeration")
        && let Some(parsed) = ai_moderation(input, &child(path, "aiModeration"), moderation)
    {
        out.insert("aiModeration".into(), parsed);
    }
    if let Some(words) = trimmed_array(
        input,
        &child(path, "keywords"),
        obj.get("keywords"),
        25,
        120,
    ) {
        out.insert("keywords".into(), json!(words));
    }
    if let Some(after) = enum_field(input, &obj, path, "showCommentsAfter", SHOW_AFTER, true) {
        out.insert("showCommentsAfter".into(), after);
    }
    if input.issue_count() != before {
        return None;
    }
    if out.get("hasKeywords") == Some(&Value::Bool(true))
        && out["keywords"].as_array().is_some_and(Vec::is_empty)
    {
        input.issue(
            &child(path, "keywords"),
            "custom",
            &message(channel, "hideKeywordsRequired"),
        );
        return None;
    }
    Some(Value::Object(out))
}

fn reply_after(
    input: &mut Input,
    channel: &Channel,
    path: &[Seg],
    value: Option<&Value>,
) -> Option<Value> {
    let obj = object(input, path, value, true)?;
    let kind = enum_field(input, &obj, path, "type", REPLY_AFTER, true);
    let amount = input.coerce_number_at(
        &child(path, "value"),
        obj.get("value"),
        Num::int(0.0, 86_400.0),
        true,
    );
    let (kind, amount) = (kind?, amount? as i64);
    let timed = matches!(kind.as_str(), Some("seconds" | "minutes" | "hours"));
    if timed && amount <= 0 {
        input.issue(
            &child(path, "value"),
            "custom",
            &message(channel, "delayMustBePositive"),
        );
        return None;
    }
    if !timed && amount != 0 {
        input.issue(
            &child(path, "value"),
            "custom",
            &message(channel, "delayMustBeZero"),
        );
        return None;
    }
    Some(json!({ "type": kind, "value": amount }))
}

fn body(input: &mut Input, channel: &Channel, create: bool) -> Map<String, Value> {
    let mut data = Map::new();
    if create || input.has("name") {
        if let Some(name) = input.string("name", Str::trimmed(1, 120)) {
            data.insert("name".into(), Value::String(name));
        }
    }
    if let Some(value) = nullish_digits(input, "inboxId") {
        data.insert("inboxId".into(), value);
    }
    type Parse = fn(&mut Input, &Channel, &[Seg], Option<&Value>) -> Option<Value>;
    let fields: [(&str, bool, Parse); 6] = [
        ("post", true, post),
        ("publicReply", true, |i, c, p, v| reply(i, c, p, v, false)),
        ("includeKeywords", true, include_keywords),
        ("options", true, options),
        ("hideComments", false, hide_comments),
        ("replyAfter", true, reply_after),
    ];
    for (name, required, parse) in fields {
        let value = input.get(name).cloned();
        if value.is_none() && !(create && required) {
            continue;
        }
        if let Some(parsed) = parse(input, channel, &key(name), value.as_ref()) {
            data.insert(name.into(), parsed);
        }
    }
    if channel.kind == "tiktok"
        && let Some(value) = input.get("privateReply").cloned()
    {
        if let Some(parsed) = reply(input, channel, &key("privateReply"), Some(&value), true) {
            data.insert("privateReply".into(), parsed);
        }
    }
    let value = input.get("excludeKeywords").cloned();
    if value.is_some() || create {
        if let Some(words) = trimmed_array(input, &key("excludeKeywords"), value.as_ref(), 25, 120)
        {
            data.insert("excludeKeywords".into(), json!(words));
        }
    }
    let value = input.get("excludeKeywordsType").cloned();
    if let Some(kind) = input.enum_at(
        &key("excludeKeywordsType"),
        value.as_ref(),
        &["equal", "contain"],
        false,
    ) {
        data.insert("excludeKeywordsType".into(), Value::String(kind));
    }
    if let Some(flag) = input.opt_bool("isActive") {
        data.insert("isActive".into(), Value::Bool(flag));
    }
    data
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let (group, op) = route_id.split_once('.').unwrap_or_default();
    let channel = if group == "threadsComments" {
        &THREADS
    } else {
        &TIKTOK
    };
    let mut input = if matches!(op, "list" | "get" | "delete") {
        Input::from_query(call.params, call.query)
    } else {
        Input::from_body(call.params, call.body)
    };
    match op {
        "list" => {
            let filter = list_filter(&mut input, false);
            input.finish()?;
            let (data, count) = crud::list(db, COMMENTS, ws, channel.types, filter)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data = data
                .into_iter()
                .map(|row| crud::channel_resource(channel.kind, row))
                .collect();
            page(data, count)
        }
        "get" => {
            let id = input.digits_id("id");
            input.finish()?;
            let row = crud::find_or_fail(
                db,
                COMMENTS,
                ws,
                id.unwrap_or(-1),
                channel.types,
                channel.not_found,
            )
            .await
            .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, crud::channel_resource(channel.kind, row)))
        }
        "delete" => {
            let id = input.digits_id("id");
            input.finish()?;
            crud::delete_one(
                db,
                COMMENTS,
                ws,
                id.unwrap_or(-1),
                channel.types,
                channel.not_found,
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "deleteMany" => {
            let list = ids(&mut input);
            input.finish()?;
            crud::delete_many(db, COMMENTS, ws, list, channel.types)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "create" => {
            let mut data = body(&mut input, channel, true);
            input.finish()?;
            build_channel_write(channel.kind, &mut data, true);
            let row = crud::create_comment(db, ws, channel.kind, data)
                .await
                .map_err(|e| business_error(e, CREATING))?;
            Ok(json(201, crud::channel_resource(channel.kind, row)))
        }
        _ => {
            let id = input.digits_id("id");
            let mut data = body(&mut input, channel, false);
            if !input.failed() && data.is_empty() {
                input.issue(&[], "custom", &message(channel, "atLeastOneFieldRequired"));
            }
            input.finish()?;
            build_channel_write(channel.kind, &mut data, false);
            let row = crud::update_comment(
                db,
                ws,
                id.unwrap_or(-1),
                channel.types,
                channel.not_found,
                data,
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, crud::channel_resource(channel.kind, row)))
        }
    }
}
