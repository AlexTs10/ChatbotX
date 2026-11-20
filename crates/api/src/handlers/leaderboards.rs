//! `features/leaderboards/api/public.ts`: `leaderboards.*` (scope
//! `leaderboards`), plus the config schema (`schema/action.ts`) and the
//! resource shape (`lib/to-resource.ts`) the `/rpc` actions share.

use chatbotx_business::leaderboard::service::{self, Deps, ListInput, ParticipantsInput};
use chatbotx_business::leaderboard::{
    PLATFORMS, RULE_KEYS, STATUSES, Stats, UpsertInput, actions_by_status, default_rules,
};
use chatbotx_db::repo::models::Leaderboard;
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Seg, Str, child, key};
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, Principal, Reply, business_error, date, json,
    no_content, opt_date,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "leaderboards.list",
    "leaderboards.create",
    "leaderboards.get",
    "leaderboards.update",
    "leaderboards.delete",
    "leaderboards.defaultRules",
    "leaderboards.telegramChats",
    "leaderboards.publish",
    "leaderboards.pause",
    "leaderboards.start",
    "leaderboards.finish",
    "leaderboards.resetPoints",
    "leaderboards.top",
    "leaderboards.getViewLink",
    "leaderboards.export",
    "leaderboards.getExport",
    "leaderboards.triggerRewards",
    "leaderboards.participants.list",
    "leaderboards.participants.adjust",
    "leaderboards.participants.ban",
    "leaderboards.participants.unban",
    "leaderboards.participants.reset",
    "leaderboards.participants.history",
    "leaderboards.auditLog.list",
];

/// `possibleErrorsOnMutatingLeaderboard`.
pub const MUTATING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("leaderboardActionNotAllowed", 409),
    ("leaderboardFinished", 409),
    ("leaderboardNotPublished", 409),
    ("leaderboardParticipantBanned", 409),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnTriggeringLeaderboardRewards`.
pub const REWARDS: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("leaderboardActionNotAllowed", 409),
    ("leaderboardFinished", 409),
    ("leaderboardNotPublished", 409),
    ("leaderboardParticipantBanned", 409),
    ("leaderboardNotFinished", 409),
    ("leaderboardRewardFlowMissing", 422),
    ("leaderboardRewardsAlreadySent", 409),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `LEADERBOARD_VIEW_TOKEN_VARIABLE`.
pub const VIEW_TOKEN_VARIABLE: &str = "{{leaderboard_view_token}}";

/// `buildLeaderboardViewUrl(id, token)`.
pub fn view_url(app_url: &str, id: &str, token: &str) -> String {
    format!(
        "{}/leaderboards?leaderboardId={id}&token={token}",
        super::messenger_templates::broker_origin(app_url)
    )
}

/// `toLeaderboardResource`.
pub fn resource(app_url: &str, board: &Leaderboard, stats: Stats) -> Value {
    json!({
        "id": board.id.to_string(),
        "name": board.name,
        "description": board.description,
        "coverImageUrl": board.cover_image_url,
        "startsAt": date(board.starts_at),
        "endsAt": date(board.ends_at),
        "status": board.status,
        "platform": board.platform,
        "inboxId": board.inbox_id.map(|id| id.to_string()),
        "minigameId": board.minigame_id.map(|id| id.to_string()),
        "scope": board.scope.0,
        "rules": board.rules.0,
        "followersOnly": board.followers_only,
        "tagIds": board.tag_ids.0,
        "resetTime": board.reset_time,
        "cooldownSeconds": board.cooldown_seconds,
        "referralWelcomeFlowId": board.referral_welcome_flow_id.map(|id| id.to_string()),
        "rewardSettings": board.reward_settings.0,
        "rewardsTriggeredAt": opt_date(board.rewards_triggered_at),
        "finishedAt": opt_date(board.finished_at),
        "createdAt": date(board.created_at),
        "updatedAt": date(board.updated_at),
        "stats": stats.to_json(),
        "availableActions": actions_by_status(&board.status),
        "viewUrl": view_url(app_url, &board.id.to_string(), VIEW_TOKEN_VARIABLE),
    })
}

/// The view link template of `create` (`{id}` = the new leaderboard id).
pub fn view_url_template(app_url: &str) -> String {
    view_url(app_url, "{id}", VIEW_TOKEN_VARIABLE)
}

fn bigint_list(input: &mut Input, path: &[Seg], value: Option<&Value>, max: usize) -> Vec<i64> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        input.issue(
            path,
            "invalid_type",
            &format!(
                "Invalid input: expected array, received {}",
                super::input::type_name(Some(value))
            ),
        );
        return Vec::new();
    };
    if items.len() > max {
        input.issue(
            path,
            "too_big",
            &format!("Too big: expected array to have <={max} items"),
        );
    }
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let mut at = path.to_vec();
        at.push(Seg::Index(i));
        if let Some(id) = input.id_at(&at, Some(item), true) {
            out.push(id);
        }
    }
    out
}

fn string_list(
    input: &mut Input,
    path: &[Seg],
    value: Option<&Value>,
    max_items: usize,
    max_len: usize,
) -> Vec<Value> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        input.issue(
            path,
            "invalid_type",
            &format!(
                "Invalid input: expected array, received {}",
                super::input::type_name(Some(value))
            ),
        );
        return Vec::new();
    };
    if items.len() > max_items {
        input.issue(
            path,
            "too_big",
            &format!("Too big: expected array to have <={max_items} items"),
        );
    }
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let mut at = path.to_vec();
        at.push(Seg::Index(i));
        if let Some(text) = input.string_at(&at, Some(item), Str::trimmed(1, max_len), true) {
            out.push(Value::String(text));
        }
    }
    out
}

fn object<'a>(input: &mut Input, path: &[Seg], value: &'a Value) -> Option<&'a Map<String, Value>> {
    let map = value.as_object();
    if map.is_none() {
        input.issue(
            path,
            "invalid_type",
            &format!(
                "Invalid input: expected object, received {}",
                super::input::type_name(Some(value))
            ),
        );
    }
    map
}

fn nullable_int(input: &mut Input, path: &[Seg], value: Option<&Value>, spec: Num) -> Value {
    match value {
        None | Some(Value::Null) => Value::Null,
        Some(v) => input
            .number_at(path, Some(v), spec, true)
            .map_or(Value::Null, |n| json!(n as i64)),
    }
}

/// `leaderboardRuleSchema` (defaults filled).
fn rule(input: &mut Input, path: &[Seg], value: &Value) -> Option<Value> {
    let map = object(input, path, value)?;
    let enabled = input
        .bool_at(&child(path, "enabled"), map.get("enabled"), false)
        .unwrap_or(false);
    let points = input
        .number_at(
            &child(path, "points"),
            map.get("points"),
            Num::int(0.0, 1_000_000.0),
            false,
        )
        .map_or(0, |n| n as i64);
    Some(json!({
        "enabled": enabled,
        "points": points,
        "maxPerDay": nullable_int(input, &child(path, "maxPerDay"), map.get("maxPerDay"), Num::int(1.0, 100_000.0)),
        "minChars": nullable_int(input, &child(path, "minChars"), map.get("minChars"), Num::int(0.0, 2000.0)),
        "bonusPoints": nullable_int(input, &child(path, "bonusPoints"), map.get("bonusPoints"), Num::int(0.0, 1_000_000.0)),
        "bonusMinutes": nullable_int(input, &child(path, "bonusMinutes"), map.get("bonusMinutes"), Num::int(1.0, 10_080.0)),
    }))
}

fn rules(input: &mut Input, value: Option<&Value>) -> Value {
    let path = key("rules");
    let Some(value) = value else {
        input.issue(
            &path,
            "invalid_type",
            "Invalid input: expected record, received undefined",
        );
        return json!({});
    };
    let Some(map) = object(input, &path, value) else {
        return json!({});
    };
    let mut out = Map::new();
    for (name, raw) in map {
        if !RULE_KEYS.contains(&name.as_str()) {
            input.issue(&child(&path, name), "invalid_key", "Invalid key in record");
            continue;
        }
        if let Some(rule) = rule(input, &child(&path, name), raw) {
            out.insert(name.clone(), rule);
        }
    }
    Value::Object(out)
}

fn scope(input: &mut Input, value: Option<&Value>) -> Value {
    let path = key("scope");
    let default = Value::Object(Map::new());
    let value = value.unwrap_or(&default);
    let Some(map) = object(input, &path, value) else {
        return Value::Null;
    };
    let post_scope = input
        .enum_at(
            &child(&path, "postScope"),
            map.get("postScope"),
            &["all", "specific"],
            false,
        )
        .unwrap_or_else(|| "all".to_owned());
    let post_ids = string_list(
        input,
        &child(&path, "postIds"),
        map.get("postIds"),
        200,
        200,
    );
    let chats = string_list(
        input,
        &child(&path, "telegramChatIds"),
        map.get("telegramChatIds"),
        50,
        50,
    );
    json!({ "postScope": post_scope, "postIds": post_ids, "telegramChatIds": chats })
}

fn reward_settings(input: &mut Input, value: Option<&Value>) -> Value {
    let path = key("rewardSettings");
    let default = Value::Object(Map::new());
    let value = value.unwrap_or(&default);
    let Some(map) = object(input, &path, value) else {
        return Value::Null;
    };
    let flow = match map.get("flowId") {
        None | Some(Value::Null) => None,
        Some(v) => input.id_at(&child(&path, "flowId"), Some(v), true),
    };
    let top_n = input
        .number_at(
            &child(&path, "topN"),
            map.get("topN"),
            Num::int(1.0, 1000.0),
            false,
        )
        .map_or(3, |n| n as i64);
    let auto = input
        .bool_at(
            &child(&path, "autoOnFinish"),
            map.get("autoOnFinish"),
            false,
        )
        .unwrap_or(false);
    json!({ "flowId": flow.map(|id| id.to_string()), "topN": top_n, "autoOnFinish": auto })
}

fn date_field(input: &mut Input, name: &str) -> i64 {
    match input.get(name).map(super::broadcasts::coerce_date) {
        Some(Some(ms)) => ms,
        _ => {
            input.issue(
                &key(name),
                "invalid_type",
                "Invalid input: expected date, received Date",
            );
            0
        }
    }
}

fn nullable_id(input: &mut Input, name: &str) -> Option<i64> {
    input.nullish_id(name).flatten()
}

fn reset_time_ok(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && b[3].is_ascii_digit()
        && (b'0'..=b'5').contains(&b[3])
        && b[4].is_ascii_digit()
        && match (b[0], b[1]) {
            (b'0' | b'1', d) => d.is_ascii_digit(),
            (b'2', d) => (b'0'..=b'3').contains(&d),
            _ => false,
        }
}

/// `leaderboardConfigRequest` (defaults filled).
pub fn config(input: &mut Input) -> UpsertInput {
    let name = input
        .string("name", Str::trimmed(1, 150))
        .unwrap_or_default();
    let description = input
        .opt_string("description", Str::max(5000))
        .unwrap_or_default();
    let cover_image_url = input
        .opt_string("coverImageUrl", Str::max(2000))
        .unwrap_or_default();
    let starts_at = date_field(input, "startsAt");
    let ends_at = date_field(input, "endsAt");
    let platform_value = input.get("platform").cloned();
    let platform = input
        .enum_at(&key("platform"), platform_value.as_ref(), PLATFORMS, true)
        .unwrap_or_default();
    let inbox_id = nullable_id(input, "inboxId");
    let minigame_id = nullable_id(input, "minigameId");
    let scope_value = input.get("scope").cloned();
    let scope = scope(input, scope_value.as_ref());
    let rules_value = input.get("rules").cloned();
    let rules = rules(input, rules_value.as_ref());
    let followers_only = input.opt_bool("followersOnly").unwrap_or(false);
    let tags_value = input.get("tagIds").cloned();
    let tag_ids = bigint_list(input, &key("tagIds"), tags_value.as_ref(), 50);
    let reset_time = input
        .opt_string(
            "resetTime",
            Str {
                pattern: Some(reset_time_ok),
                ..Str::default()
            },
        )
        .unwrap_or_else(|| "22:00".to_owned());
    let cooldown_seconds = input
        .opt_number("cooldownSeconds", Num::int(0.0, 86_400.0))
        .map_or(0, |n| n as i64);
    let referral_welcome_flow_id = nullable_id(input, "referralWelcomeFlowId");
    let rewards_value = input.get("rewardSettings").cloned();
    let reward_settings = reward_settings(input, rewards_value.as_ref());
    UpsertInput {
        name,
        description,
        cover_image_url,
        starts_at,
        ends_at,
        platform,
        inbox_id,
        minigame_id,
        scope,
        rules,
        followers_only,
        tag_ids,
        reset_time,
        cooldown_seconds,
        referral_welcome_flow_id,
        reward_settings,
    }
}

/// `reason` of the ban / unban / reset / reset-points requests.
pub fn reason(input: &mut Input) -> String {
    input
        .opt_string(
            "reason",
            Str {
                trim: true,
                max: Some(500),
                ..Str::default()
            },
        )
        .unwrap_or_default()
}

/// `adjustLeaderboardScoreRequest`: `(delta, reason)`.
pub fn adjust_fields(input: &mut Input) -> (i64, String) {
    let delta = input
        .number(
            "delta",
            Num {
                int: true,
                ..Num::default()
            },
        )
        .map_or(0, |n| n as i64);
    if input.get("delta").is_some_and(|d| d.as_f64() == Some(0.0)) {
        input.issue(&key("delta"), "custom", "Enter a non-zero number of points");
    }
    let reason = input
        .string("reason", Str::trimmed(1, 500))
        .unwrap_or_default();
    (delta, reason)
}

fn opt_text(input: &mut Input, name: &str) -> Option<String> {
    input.opt_string(
        name,
        Str {
            trim: true,
            min: Some(1),
            ..Str::default()
        },
    )
}

fn opt_enum(input: &mut Input, name: &str, options: &[&str]) -> Option<String> {
    let value = input.get(name).cloned();
    input.enum_at(&key(name), value.as_ref(), options, false)
}

/// The list filters of the board list (`listLeaderboardsFilter`).
pub fn list_filters(input: &mut Input) -> ListInput {
    ListInput {
        status: opt_enum(input, "status", STATUSES),
        platform: opt_enum(input, "platform", PLATFORMS),
        keyword: opt_text(input, "keyword"),
        page: None,
        per_page: None,
    }
}

/// The participants filters (`keyword`, `filter`).
pub fn participant_filters(input: &mut Input) -> ParticipantsInput {
    ParticipantsInput {
        keyword: opt_text(input, "keyword"),
        filter: opt_enum(input, "filter", &["all", "top10", "top50", "banned"])
            .unwrap_or_else(|| "all".to_owned()),
        page: None,
        per_page: None,
    }
}

/// `context.workspace.ownerId`.
async fn workspace_owner(state: &ApiState, ws: i64) -> Result<i64, crate::orpc::OrpcError> {
    state
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "ownerId" FROM "Workspace" WHERE "id" = ?1"#,
                [ws],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .await
        .map_err(|e| super::internal_error(&e.to_string()))
}

fn deps(state: &ApiState) -> Deps<'_> {
    Deps {
        db: &state.db,
        jobs: state.jobs.clone(),
    }
}

fn id_of(input: &mut Input, name: &str) -> i64 {
    input.digits_id(name).unwrap_or_default()
}

/// Runs one of [`ROUTES`].
pub async fn handle(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let app = state.app_url.as_str();
    let board_out = |row: &Leaderboard| -> Value {
        // A fresh row (create / status change): its counters are read below.
        resource(app, row, Stats::default())
    };
    match route_id {
        "leaderboards.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = super::qr_codes::public_paging(&mut input);
            let mut filters = list_filters(&mut input);
            input.finish()?;
            filters.page = Some(page);
            filters.per_page = Some(per_page);
            let out = service::list(db, ws, filters)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = out
                .rows
                .iter()
                .map(|(row, stats)| resource(app, &row.leaderboard, *stats))
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": out.page_count, "total": out.total, "summary": out.summary }),
            ))
        }
        "leaderboards.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_of(&mut input, "id");
            input.finish()?;
            let (row, stats) = service::find(db, ws, id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, resource(app, &row, stats)))
        }
        "leaderboards.defaultRules" => {
            let mut input = Input::from_query(call.params, call.query);
            let platform = opt_enum(&mut input, "platform", PLATFORMS);
            if input.get("platform").is_none() {
                input.issue(
                    &key("platform"),
                    "invalid_value",
                    &format!(
                        "Invalid option: expected one of {}",
                        PLATFORMS
                            .iter()
                            .map(|p| format!("\"{p}\""))
                            .collect::<Vec<_>>()
                            .join("|")
                    ),
                );
            }
            input.finish()?;
            Ok(json(
                200,
                json!({ "rules": default_rules(&platform.unwrap_or_default()) }),
            ))
        }
        "leaderboards.telegramChats" => {
            let mut input = Input::from_query(call.params, call.query);
            let inbox = input.string("inboxId", Str::default());
            input.finish()?;
            let inbox = inbox.and_then(|s| s.parse().ok()).unwrap_or(-1);
            let data = service::telegram_chats(db, ws, inbox)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            Ok(json(200, json!({ "data": data })))
        }
        "leaderboards.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let cfg = config(&mut input);
            input.finish()?;
            let row = service::create(db, ws, cfg, Some(view_url_template(app)))
                .await
                .map_err(|e| business_error(e, super::CREATING))?;
            Ok(json(201, board_out(&row)))
        }
        "leaderboards.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let cfg = config(&mut input);
            let id = id_of(&mut input, "id");
            input.finish()?;
            let row = service::update(db, ws, id, cfg)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            let (row, stats) = service::find(db, ws, row.id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, resource(app, &row, stats)))
        }
        "leaderboards.delete" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_of(&mut input, "id");
            input.finish()?;
            service::delete(db, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "leaderboards.publish"
        | "leaderboards.pause"
        | "leaderboards.start"
        | "leaderboards.finish" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_of(&mut input, "id");
            input.finish()?;
            let row = match route_id {
                "leaderboards.publish" => service::publish(db, ws, id).await,
                "leaderboards.pause" => service::pause(db, ws, id).await,
                "leaderboards.start" => service::start(db, ws, id).await,
                _ => service::finish(&deps(state), ws, id).await,
            }
            .map_err(|e| business_error(e, MUTATING))?;
            let (row, stats) = service::find(db, ws, row.id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, resource(app, &row, stats)))
        }
        "leaderboards.resetPoints" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_of(&mut input, "id");
            let reason = reason(&mut input);
            input.finish()?;
            let reset = service::reset_points(db, ws, id, None, reason)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, json!({ "reset": reset })))
        }
        "leaderboards.top" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_of(&mut input, "id");
            let limit = input
                .opt_coerce_int("limit", Num::int(1.0, 500.0))
                .unwrap_or(50);
            input.finish()?;
            let data = service::admin_top(db, ws, id, limit)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, json!({ "data": data })))
        }
        "leaderboards.getViewLink" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_of(&mut input, "id");
            let contact = input.opt_string("contactId", Str::default());
            input.finish()?;
            let (row, _) = service::find(db, ws, id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let board_id = row.id.to_string();
            let token = match (
                contact.and_then(|c| c.parse::<i64>().ok()),
                &state.encryption,
            ) {
                (Some(contact), Some(enc)) => service::sign_view_token(db, enc, &row, contact)
                    .await
                    .map_err(|e| business_error(e, FINDING))?,
                _ => None,
            };
            Ok(json(
                200,
                match token {
                    Some(token) => {
                        json!({ "url": view_url(app, &board_id, &token), "personalized": true })
                    }
                    None => {
                        json!({ "url": view_url(app, &board_id, VIEW_TOKEN_VARIABLE), "personalized": false })
                    }
                },
            ))
        }
        "leaderboards.export" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_of(&mut input, "id");
            input.finish()?;
            let owner = workspace_owner(state, ws).await?;
            let include = principal
                .scopes
                .as_ref()
                .is_none_or(|s| s.iter().any(|s| s == "contacts"));
            let out = service::create_export(&deps(state), ws, id, Some(owner), include)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(201, out))
        }
        "leaderboards.getExport" => {
            let mut input = Input::from_query(call.params, call.query);
            let file_id = input.string("fileId", Str::default());
            input.finish()?;
            let file_id = file_id.and_then(|s| s.parse().ok()).unwrap_or(-1);
            let file = service::get_export(db, ws, file_id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let download = if file.status == "uploaded" {
                match &state.files {
                    Some(files) => Some(
                        files
                            .get_presigned_download(&file.path, Some(5 * 60))
                            .await
                            .map_err(|e| super::internal_error(&e.to_string()))?,
                    ),
                    None => None,
                }
            } else {
                None
            };
            let total = file
                .meta
                .as_ref()
                .and_then(|m| m.0["totalRecords"].as_i64())
                .unwrap_or(0);
            Ok(json(
                200,
                json!({
                    "fileId": file.id.to_string(),
                    "fileName": file.file_name,
                    "status": file.status,
                    "path": file.path,
                    "totalRecords": total,
                    "downloadUrl": download,
                }),
            ))
        }
        "leaderboards.triggerRewards" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_of(&mut input, "id");
            input.finish()?;
            let out = service::trigger_rewards(&deps(state), ws, id, true)
                .await
                .map_err(|e| business_error(e, REWARDS))?;
            Ok(json(200, out))
        }
        "leaderboards.participants.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = super::qr_codes::public_paging(&mut input);
            let id = id_of(&mut input, "id");
            let mut filters = participant_filters(&mut input);
            input.finish()?;
            filters.page = Some(page);
            filters.per_page = Some(per_page);
            let out = service::list_participants(db, ws, id, filters)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, out))
        }
        "leaderboards.participants.adjust" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_of(&mut input, "id");
            let contact = id_of(&mut input, "contactId");
            let (delta, reason) = adjust_fields(&mut input);
            input.finish()?;
            let score = service::adjust(db, ws, id, contact, delta, &reason, None)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, json!({ "score": score })))
        }
        "leaderboards.participants.ban" | "leaderboards.participants.unban" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_of(&mut input, "id");
            let contact = id_of(&mut input, "contactId");
            let reason = reason(&mut input);
            input.finish()?;
            let ban = route_id == "leaderboards.participants.ban";
            service::set_banned(db, ws, id, contact, ban, &reason, None)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, json!({ "ok": true })))
        }
        "leaderboards.participants.reset" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_of(&mut input, "id");
            let contact = id_of(&mut input, "contactId");
            let reason = reason(&mut input);
            input.finish()?;
            let score = service::reset_one(db, ws, id, contact, &reason, None)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, json!({ "score": score })))
        }
        "leaderboards.participants.history" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_of(&mut input, "id");
            let contact = id_of(&mut input, "contactId");
            input.finish()?;
            let data = service::participant_history(db, ws, id, contact)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, json!({ "data": data })))
        }
        "leaderboards.auditLog.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = super::qr_codes::public_paging(&mut input);
            let id = id_of(&mut input, "id");
            input.finish()?;
            let out = service::audit_log(db, ws, id, Some(page), Some(per_page))
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, out))
        }
        _ => Err(super::internal_error("unknown leaderboard route")),
    }
}
