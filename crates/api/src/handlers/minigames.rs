//! `features/minigames/api/public.ts`: `minigames.*`, with the request
//! groups of `database/partials/minigame.ts` (defaults, the 100 % odds check,
//! the date-range check).

use chatbotx_business::minigame::contact::{self, ListPlayers};
use chatbotx_business::minigame::{self as service, ListMinigames, MinigameData, MinigamePatch};
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::Minigame;
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Path, Seg, Str, child, key};
use super::qr_codes::public_paging;
use super::{ApiState, Call, Reply, business_error, date, id, json, no_content, opt_id};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "minigames.list",
    "minigames.create",
    "minigames.get",
    "minigames.update",
    "minigames.updatePartial",
    "minigames.delete",
    "minigames.deleteMany",
    "minigames.setEnabled",
    "minigames.listPlays",
    "minigames.listPlayers",
];

/// `minigameTypes`.
const TYPES: &[&str] = &[
    "luckyWheel",
    "jackpot",
    "gashapon",
    "drawLots",
    "scratchOff",
];
/// `possibleErrorsOnCreatingMinigame`: creating + the name conflict.
const CREATING_MINIGAME: super::ErrorMap = &[
    ("businessError", 400),
    ("nameAlreadyExists", 409),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
const MUTATING_MINIGAME: super::ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("nameAlreadyExists", 409),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `buildMinigamePlayUrl`.
fn play_url(state: &ApiState, minigame_id: i64) -> String {
    format!(
        "{}/minigames?minigameId={minigame_id}&token={{{{minigame_play_token}}}}",
        state.app_url
    )
}

/// `minigamePublicResource`.
fn minigame_json(state: &ApiState, row: &Minigame) -> Value {
    json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "name": row.name,
        "type": row.r#type,
        "enabled": row.enabled,
        "generalSettings": row.general_settings.0,
        "appearance": row.appearance.0,
        "playerSettings": row.player_settings.0,
        "prizeSettings": row.prize_settings.0,
        "winningMessageSettings": row.winning_message_settings.0,
        "nonWinningMessageSettings": row.non_winning_message_settings.0,
        "playsCount": row.plays_count,
        "participantsCount": row.participants_count,
        "winnersCount": row.winners_count,
        "sharesCount": row.shares_count,
        "playUrl": play_url(state, row.id),
    })
}

// --- request groups (`database/partials/minigame.ts`) ---------------------------------

fn at(path: &[Seg], index: usize) -> Path {
    let mut next = path.to_vec();
    next.push(Seg::Index(index));
    next
}

/// The object at `path`; an issue when it is missing or not an object.
fn object<'v>(
    input: &mut Input,
    path: &[Seg],
    value: Option<&'v Value>,
) -> Option<&'v Map<String, Value>> {
    match value {
        Some(Value::Object(obj)) => Some(obj),
        None => {
            input.issue(
                path,
                "invalid_type",
                "Invalid input: expected object, received undefined",
            );
            None
        }
        Some(_) => {
            input.issue(path, "invalid_type", "Invalid input: expected object");
            None
        }
    }
}

/// `z.string().max(n).default(d)` (`trim` when asked).
fn text(
    input: &mut Input,
    obj: &Map<String, Value>,
    path: &[Seg],
    name: &str,
    spec: Str,
    default: &str,
) -> Option<Value> {
    match obj.get(name) {
        None => Some(json!(default)),
        Some(v) => input
            .string_at(&child(path, name), Some(v), spec, true)
            .map(Value::String),
    }
}

/// `z.boolean().default(d)`.
fn flag(
    input: &mut Input,
    obj: &Map<String, Value>,
    path: &[Seg],
    name: &str,
    default: bool,
) -> Option<Value> {
    match obj.get(name) {
        None => Some(json!(default)),
        Some(v) => input
            .bool_at(&child(path, name), Some(v), true)
            .map(Value::Bool),
    }
}

/// `z.number()` checks (`.default(d)` when `default` is given).
fn number(
    input: &mut Input,
    obj: &Map<String, Value>,
    path: &[Seg],
    name: &str,
    spec: Num,
    default: Option<f64>,
) -> Option<Option<Value>> {
    match obj.get(name) {
        None => match default {
            Some(d) => Some(Some(super::number(d))),
            None => Some(None),
        },
        Some(v) => input
            .number_at(&child(path, name), Some(v), spec, true)
            .map(|n| Some(super::number(n))),
    }
}

/// `zodBigintAsString().nullable().default(null)`.
fn nullable_id(
    input: &mut Input,
    obj: &Map<String, Value>,
    path: &[Seg],
    name: &str,
) -> Option<Value> {
    match obj.get(name) {
        None | Some(Value::Null) => Some(Value::Null),
        Some(v) => input
            .id_at(&child(path, name), Some(v), true)
            .map(|n| json!(n.to_string())),
    }
}

/// `z.array(zodBigintAsString()).default([])`.
fn id_list(input: &mut Input, obj: &Map<String, Value>, path: &[Seg], name: &str) -> Option<Value> {
    let list_path = child(path, name);
    match obj.get(name) {
        None => Some(json!([])),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for (index, item) in items.iter().enumerate() {
                if let Some(n) = input.id_at(&at(&list_path, index), Some(item), true) {
                    out.push(json!(n.to_string()));
                }
            }
            Some(Value::Array(out))
        }
        Some(_) => {
            input.issue(&list_path, "invalid_type", "Invalid input: expected array");
            None
        }
    }
}

/// `minigameImageSchema`.
fn image(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<Value> {
    let obj = object(input, path, value)?;
    let mode = match obj.get("mode") {
        None => Some("file".to_string()),
        Some(v) => input.enum_at(&child(path, "mode"), Some(v), &["url", "file"], true),
    };
    let url = text(input, obj, path, "url", Str::default(), "");
    Some(json!({ "mode": mode?, "url": url? }))
}

/// `z.iso.datetime()`: `YYYY-MM-DDTHH:MM(:SS(.f+)?)?Z`.
pub(crate) fn is_iso_datetime(text: &str) -> bool {
    let Some(body) = text.strip_suffix('Z') else {
        return false;
    };
    let (date_part, time) = match body.split_once('T') {
        Some(parts) => parts,
        None => return false,
    };
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let date_ok = {
        let parts: Vec<&str> = date_part.split('-').collect();
        parts.len() == 3 && digits(parts[0], 4) && digits(parts[1], 2) && digits(parts[2], 2)
    };
    let (clock, fraction) = match time.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (time, None),
    };
    let clock_parts: Vec<&str> = clock.split(':').collect();
    let clock_ok = matches!(clock_parts.len(), 2 | 3) && clock_parts.iter().all(|p| digits(p, 2));
    let fraction_ok = fraction
        .is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
        && (fraction.is_none() || clock_parts.len() == 3);
    date_ok && clock_ok && fraction_ok && chatbotx_core::js::parse_date(text).is_some()
}

/// `minigameGeneralSettingsSchema`.
fn general_settings(input: &mut Input, value: Option<&Value>) -> Option<Value> {
    let path = key("generalSettings");
    let obj = object(input, &path, value)?;
    let name = input.string_at(
        &child(&path, "name"),
        obj.get("name"),
        Str::trimmed(1, 150),
        true,
    );
    let show_name = flag(input, obj, &path, "showName", false);
    let date_at = |input: &mut Input, field: &str| {
        let text = input.string_at(&child(&path, field), obj.get(field), Str::default(), true)?;
        if is_iso_datetime(&text) {
            Some(text)
        } else {
            input.issue(
                &child(&path, field),
                "invalid_format",
                "Invalid ISO datetime",
            );
            None
        }
    };
    let from = date_at(input, "playedAtFrom");
    let to = date_at(input, "playedAtTo");
    let rules = text(input, obj, &path, "rulesDescription", Str::max(5000), "");
    let opener = id_list(input, obj, &path, "openerTagIds");
    let player = id_list(input, obj, &path, "playerTagIds");
    let new_friend = id_list(input, obj, &path, "newFriendTagIds");
    let (name, show_name, from, to, rules, opener, player, new_friend) = (
        name?,
        show_name?,
        from?,
        to?,
        rules?,
        opener?,
        player?,
        new_friend?,
    );
    if to < from {
        input.issue(
            &child(&path, "playedAtTo"),
            "custom",
            "playedAtTo must be on or after playedAtFrom",
        );
        return None;
    }
    Some(json!({
        "name": name,
        "showName": show_name,
        "playedAtFrom": from,
        "playedAtTo": to,
        "rulesDescription": rules,
        "openerTagIds": opener,
        "playerTagIds": player,
        "newFriendTagIds": new_friend,
    }))
}

/// `minigameAppearanceSchema`.
fn appearance(input: &mut Input, value: Option<&Value>) -> Option<Value> {
    let path = key("appearance");
    let obj = object(input, &path, value)?;
    let background = text(
        input,
        obj,
        &path,
        "backgroundColor",
        Str::max(50),
        "#F5A623",
    );
    let machine = text(input, obj, &path, "machineColor", Str::max(50), "#4A90D9");
    let decorative = text(
        input,
        obj,
        &path,
        "decorativeColor",
        Str::max(50),
        "#FFFFFF",
    );
    let rule = text(input, obj, &path, "ruleTextColor", Str::max(50), "#000000");
    let background_image = image(
        input,
        &child(&path, "backgroundImage"),
        obj.get("backgroundImage"),
    );
    let prize_image = image(
        input,
        &child(&path, "prizeDescriptionImage"),
        obj.get("prizeDescriptionImage"),
    );
    let start_image = image(
        input,
        &child(&path, "startButtonImage"),
        obj.get("startButtonImage"),
    );
    Some(json!({
        "backgroundColor": background?,
        "machineColor": machine?,
        "decorativeColor": decorative?,
        "ruleTextColor": rule?,
        "backgroundImage": background_image?,
        "prizeDescriptionImage": prize_image?,
        "startButtonImage": start_image?,
    }))
}

const INT_MIN_1: Num = Num {
    int: true,
    min: Some(1.0),
    max: None,
};

/// `minigamePlayerSettingsSchema` (discriminated by `resetPolicy`).
fn player_settings(input: &mut Input, value: Option<&Value>) -> Option<Value> {
    let path = key("playerSettings");
    let obj = object(input, &path, value)?;
    let policy = match obj.get("resetPolicy") {
        Some(Value::String(p)) if p == "never" || p == "everyNDays" => p.clone(),
        _ => {
            input.issue(
                &child(&path, "resetPolicy"),
                "invalid_union",
                "Invalid input",
            );
            return None;
        }
    };
    let draws = number(input, obj, &path, "drawsPerPerson", INT_MIN_1, Some(1.0));
    let shares = number(
        input,
        obj,
        &path,
        "maxSharesPerPerson",
        Num::int(0.0, 100.0),
        Some(0.0),
    );
    let flow = nullable_id(input, obj, &path, "sharingFlowId");
    let node = nullable_id(input, obj, &path, "sharingNodeId");
    let mut out = json!({
        "resetPolicy": policy,
        "drawsPerPerson": draws??,
        "maxSharesPerPerson": shares??,
        "sharingFlowId": flow?,
        "sharingNodeId": node?,
    });
    if policy == "everyNDays" {
        let days = number(input, obj, &path, "resetIntervalDays", INT_MIN_1, Some(1.0))??;
        out["resetIntervalDays"] = days;
    }
    Some(out)
}

/// `minigamePrizeSettingsSchema` with the 100 % check.
fn prize_settings(input: &mut Input, value: Option<&Value>) -> Option<Value> {
    let path = key("prizeSettings");
    let obj = object(input, &path, value)?;
    let prizes_path = child(&path, "prizes");
    let mut prizes = Vec::new();
    let mut total = 0.0_f64;
    let mut ok = true;
    match obj.get("prizes") {
        None => {}
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                let item_path = at(&prizes_path, index);
                let Some(prize) = object(input, &item_path, Some(item)) else {
                    ok = false;
                    continue;
                };
                let prize_id = input.string_at(
                    &child(&item_path, "id"),
                    prize.get("id"),
                    Str::default(),
                    true,
                );
                let name = input.string_at(
                    &child(&item_path, "name"),
                    prize.get("name"),
                    Str::trimmed(1, 100),
                    true,
                );
                let icon = image(input, &child(&item_path, "icon"), prize.get("icon"));
                let win_rate = input.number_at(
                    &child(&item_path, "winRate"),
                    prize.get("winRate"),
                    Num::range(0.0, 100.0),
                    true,
                );
                let quantity = number(
                    input,
                    prize,
                    &item_path,
                    "quantity",
                    Num {
                        int: true,
                        min: Some(0.0),
                        max: None,
                    },
                    None,
                );
                match (prize_id, name, icon, win_rate, quantity) {
                    (Some(prize_id), Some(name), Some(icon), Some(win_rate), Some(quantity)) => {
                        total += win_rate;
                        let mut out = json!({ "id": prize_id, "name": name, "icon": icon, "winRate": super::number(win_rate) });
                        if let Some(quantity) = quantity {
                            out["quantity"] = quantity;
                        }
                        prizes.push(out);
                    }
                    _ => ok = false,
                }
            }
        }
        Some(_) => {
            input.issue(
                &prizes_path,
                "invalid_type",
                "Invalid input: expected array",
            );
            ok = false;
        }
    }
    let lose_path = child(&path, "nonWinning");
    let non_winning = object(input, &lose_path, obj.get("nonWinning")).and_then(|lose| {
        let title = input.string_at(
            &child(&lose_path, "title"),
            lose.get("title"),
            Str::trimmed(1, 150),
            true,
        );
        let rate = input.number_at(
            &child(&lose_path, "loseRate"),
            lose.get("loseRate"),
            Num::range(0.0, 100.0),
            true,
        );
        let lose_image = image(
            input,
            &child(&lose_path, "loseImage"),
            lose.get("loseImage"),
        );
        Some((title?, rate?, lose_image?))
    });
    let custom_field = nullable_id(input, obj, &path, "prizeNameCustomFieldId");
    let (Some((title, lose_rate, lose_image)), Some(custom_field), true) =
        (non_winning, custom_field, ok)
    else {
        return None;
    };
    // `isMinigameProbabilityTotalValid`.
    if ((total + lose_rate) * 100.0).round() as i64 != 10_000 {
        input.issue(
            &child(&lose_path, "loseRate"),
            "custom",
            "Total probability of all prizes must equal 100%",
        );
        return None;
    }
    Some(json!({
        "prizes": prizes,
        "nonWinning": { "title": title, "loseRate": super::number(lose_rate), "loseImage": lose_image },
        "prizeNameCustomFieldId": custom_field,
    }))
}

/// `minigameOutcomeMessageSchema` (default `{ enabled: false, mode: text, text: "" }`).
fn outcome_message(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<Value> {
    let Some(value) = value else {
        return Some(json!({ "enabled": false, "mode": "text", "text": "" }));
    };
    let obj = object(input, path, Some(value))?;
    let enabled = flag(input, obj, path, "enabled", false)?;
    match obj.get("mode").and_then(Value::as_str) {
        Some("text") => {
            let text = text(input, obj, path, "text", Str::max(1000), "")?;
            Some(json!({ "enabled": enabled, "mode": "text", "text": text }))
        }
        Some("flow") => {
            let flow = nullable_id(input, obj, path, "flowId")?;
            Some(json!({ "enabled": enabled, "mode": "flow", "flowId": flow }))
        }
        Some("node") => {
            let flow = nullable_id(input, obj, path, "flowId")?;
            let node = nullable_id(input, obj, path, "nodeId")?;
            Some(json!({ "enabled": enabled, "mode": "node", "flowId": flow, "nodeId": node }))
        }
        _ => {
            input.issue(&child(path, "mode"), "invalid_union", "Invalid input");
            None
        }
    }
}

/// `minigameWinningMessageSettingsSchema` / `minigameNonWinningMessageSettingsSchema`.
fn message_settings(
    input: &mut Input,
    name: &str,
    value: Option<&Value>,
    winning: bool,
) -> Option<Value> {
    let path = key(name);
    let obj = object(input, &path, value)?;
    let title = text(input, obj, &path, "title", Str::max(150), "");
    let description = text(input, obj, &path, "description", Str::max(1000), "");
    let accept = winning.then(|| text(input, obj, &path, "acceptButtonText", Str::max(50), ""));
    let share = text(input, obj, &path, "shareButtonText", Str::max(50), "");
    let share_description = text(
        input,
        obj,
        &path,
        "shareButtonDescription",
        Str::max(300),
        "",
    );
    let outcome = outcome_message(
        input,
        &child(&path, "outcomeMessage"),
        obj.get("outcomeMessage"),
    );
    let mut out = json!({
        "title": title?,
        "description": description?,
        "shareButtonText": share?,
        "shareButtonDescription": share_description?,
        "outcomeMessage": outcome?,
    });
    if let Some(accept) = accept {
        out["acceptButtonText"] = accept?;
    }
    Some(out)
}

fn minigame_type(input: &mut Input) -> Option<String> {
    let value = input.get("type").cloned();
    input.enum_at(&key("type"), value.as_ref(), TYPES, true)
}

/// `createMinigameRequest`.
fn minigame_data(input: &mut Input) -> Option<MinigameData> {
    let kind = minigame_type(input);
    let get = |input: &Input, name: &str| input.get(name).cloned();
    let general = general_settings(input, get(input, "generalSettings").as_ref());
    let looks = appearance(input, get(input, "appearance").as_ref());
    let players = player_settings(input, get(input, "playerSettings").as_ref());
    let prizes = prize_settings(input, get(input, "prizeSettings").as_ref());
    let winning = message_settings(
        input,
        "winningMessageSettings",
        get(input, "winningMessageSettings").as_ref(),
        true,
    );
    let losing = message_settings(
        input,
        "nonWinningMessageSettings",
        get(input, "nonWinningMessageSettings").as_ref(),
        false,
    );
    Some(MinigameData {
        r#type: kind?,
        general_settings: general?,
        appearance: looks?,
        player_settings: players?,
        prize_settings: prizes?,
        winning_message_settings: winning?,
        non_winning_message_settings: losing?,
    })
}

/// `patchMinigamePublicRequest`: the sent groups; at least one.
fn minigame_patch(input: &mut Input) -> MinigamePatch {
    let mut patch = MinigamePatch::default();
    let get = |input: &Input, name: &str| input.get(name).cloned();
    if input.has("type") {
        patch.r#type = minigame_type(input);
    }
    if let Some(v) = get(input, "generalSettings") {
        patch.general_settings = general_settings(input, Some(&v));
    }
    if let Some(v) = get(input, "appearance") {
        patch.appearance = appearance(input, Some(&v));
    }
    if let Some(v) = get(input, "playerSettings") {
        patch.player_settings = player_settings(input, Some(&v));
    }
    if let Some(v) = get(input, "prizeSettings") {
        patch.prize_settings = prize_settings(input, Some(&v));
    }
    if let Some(v) = get(input, "winningMessageSettings") {
        patch.winning_message_settings =
            message_settings(input, "winningMessageSettings", Some(&v), true);
    }
    if let Some(v) = get(input, "nonWinningMessageSettings") {
        patch.non_winning_message_settings =
            message_settings(input, "nonWinningMessageSettings", Some(&v), false);
    }
    let sent = [
        "type",
        "generalSettings",
        "appearance",
        "playerSettings",
        "prizeSettings",
        "winningMessageSettings",
        "nonWinningMessageSettings",
    ]
    .iter()
    .any(|name| input.has(name));
    if !input.failed() && !sent {
        input.issue(&[], "custom", "At least one field must be provided");
    }
    patch
}

/// `listMinigamePlayersPublicRequest` / `listMinigamesPublicRequest` `name`.
const NAME_FILTER: Str = Str::trimmed(1, usize::MAX);

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "minigames.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let name = input.opt_string("name", NAME_FILTER);
            input.finish()?;
            let page = service::list(
                db,
                ListMinigames {
                    workspace_id: ws,
                    page: Some(page),
                    per_page: Some(per_page),
                    name,
                    sort: Some(vec![SortItem {
                        id: "createdAt".into(),
                        desc: true,
                    }]),
                },
            )
            .await
            .map_err(|e| business_error(e, super::LISTING))?;
            let data: Vec<Value> = page
                .data
                .iter()
                .map(|row| minigame_json(state, row))
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "minigames.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let minigame_id = input.id("id");
            input.finish()?;
            service::find(db, ws, minigame_id.unwrap_or_default())
                .await
                .map(|row| json(200, minigame_json(state, &row)))
                .map_err(|e| business_error(e, super::FINDING))
        }
        "minigames.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = minigame_data(&mut input);
            input.finish()?;
            let Some(data) = data else {
                return Err(super::internal_error("minigame create input"));
            };
            service::create(db, ws, data)
                .await
                .map(|row| json(201, minigame_json(state, &row)))
                .map_err(|e| business_error(e, CREATING_MINIGAME))
        }
        "minigames.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let minigame_id = input.id("id");
            let data = minigame_data(&mut input);
            input.finish()?;
            let Some(data) = data else {
                return Err(super::internal_error("minigame update input"));
            };
            // The public API sends no form baseline: quantities as submitted.
            service::update(db, ws, minigame_id.unwrap_or_default(), data, None)
                .await
                .map(|row| json(200, minigame_json(state, &row)))
                .map_err(|e| business_error(e, MUTATING_MINIGAME))
        }
        "minigames.updatePartial" => {
            let mut input = Input::from_body(call.params, call.body);
            let minigame_id = input.id("id");
            let patch = minigame_patch(&mut input);
            input.finish()?;
            service::update_partial(db, ws, minigame_id.unwrap_or_default(), patch)
                .await
                .map(|row| json(200, minigame_json(state, &row)))
                .map_err(|e| business_error(e, MUTATING_MINIGAME))
        }
        "minigames.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let minigame_id = input.id("id");
            input.finish()?;
            service::delete(db, ws, minigame_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, super::MUTATING))
        }
        "minigames.deleteMany" => {
            let mut input = Input::from_body(call.params, call.body);
            let mut ids = Vec::new();
            match input.get("ids").cloned() {
                Some(Value::Array(items)) => {
                    for (index, item) in items.iter().enumerate() {
                        if let Some(n) = input.id_at(&at(&key("ids"), index), Some(item), true) {
                            ids.push(n);
                        }
                    }
                }
                _ => input.issue(&key("ids"), "invalid_type", "Invalid input: expected array"),
            }
            input.finish()?;
            service::delete_many(db, ws, ids)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, super::MUTATING))
        }
        "minigames.setEnabled" => {
            let mut input = Input::from_body(call.params, call.body);
            let minigame_id = input.id("id");
            let enabled = input.bool("enabled");
            input.finish()?;
            service::set_enabled(
                db,
                ws,
                minigame_id.unwrap_or_default(),
                enabled.unwrap_or_default(),
            )
            .await
            .map(|row| json(200, minigame_json(state, &row)))
            .map_err(|e| business_error(e, super::MUTATING))
        }
        "minigames.listPlays" => {
            let mut input = Input::from_query(call.params, call.query);
            let minigame_id = input.id("id");
            let contact_id = input.id("contactId");
            input.finish()?;
            let plays = contact::list_plays(
                db,
                ws,
                minigame_id.unwrap_or_default(),
                contact_id.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, super::FINDING))?;
            let data: Vec<Value> = plays
                .iter()
                .map(|play| {
                    json!({
                        "id": id(play.id),
                        "isWinning": play.is_winning,
                        "prizeName": play.prize_name,
                        "createdAt": date(play.created_at),
                    })
                })
                .collect();
            Ok(json(200, json!({ "data": data })))
        }
        "minigames.listPlayers" => {
            let mut input = Input::from_query(call.params, call.query);
            let minigame_id = input.id("id");
            let (page, per_page) = public_paging(&mut input);
            let name = input.opt_string("name", NAME_FILTER);
            input.finish()?;
            let page = contact::list(
                db,
                ListPlayers {
                    workspace_id: ws,
                    minigame_id: minigame_id.unwrap_or_default(),
                    page: Some(page),
                    per_page: Some(per_page),
                    name,
                    sort: None,
                },
            )
            .await
            .map_err(|e| business_error(e, super::FINDING))?;
            let data: Vec<Value> = page
                .data
                .iter()
                .map(|player| {
                    json!({
                        "id": id(player.id),
                        "contactId": id(player.contact_id),
                        "contactInboxId": opt_id(player.contact_inbox_id),
                        "played": player.played,
                        "remaining": player.remaining,
                        "sharesCount": player.shares_count,
                        "openedAt": date(player.opened_at),
                        "lastPlayedAt": date(player.last_played_at),
                        "contact": {
                            "id": id(player.contact.id),
                            "fullName": player.contact.full_name,
                            "firstName": player.contact.first_name,
                            "lastName": player.contact.last_name,
                            "avatar": player.contact.avatar,
                        },
                    })
                })
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
