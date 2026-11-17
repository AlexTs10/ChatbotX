//! `features/ads/api/public.ts`: `ads.*` (scope `ads`).

use chatbotx_business::ads::referral;
use chatbotx_business::ads::rules::{self, CHANNELS, EVENT_TYPES};
use serde_json::{Value, json};

use super::input::{Input, Num, Path, Seg, Str, child, key};
use super::qr_codes::public_paging;
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "ads.listRules",
    "ads.createRule",
    "ads.getRule",
    "ads.updateRule",
    "ads.updateRuleLegacy",
    "ads.deleteRule",
    "ads.toggleRuleStatus",
    "ads.referralStats",
    "ads.listReferralAds",
];

/// `possibleErrorsOnReporting`.
const REPORTING: super::ErrorMap = &[("businessError", 400), ("reportTimeout", 503)];
/// `adsEligibleChannelTypes`.
const ELIGIBLE_CHANNELS: &[&str] = &["whatsapp", "messenger", "instagram"];

/// `YYYY-MM-DD`.
fn date_key(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

const DATE_KEY: Str = Str {
    trim: false,
    min: None,
    max: None,
    pattern: Some(date_key),
};

const NON_EMPTY: Str = Str {
    trim: true,
    min: Some(1),
    max: None,
    pattern: None,
};

const FK_KEYS: [&str; 4] = [
    "integrationWhatsappId",
    "integrationFacebookAdsId",
    "integrationMessengerId",
    "integrationInstagramId",
];

/// `nonEmptyStringArray` at `path` (trimmed items).
fn string_list(input: &mut Input, path: &Path, value: Option<&Value>) -> Option<Vec<String>> {
    let Some(Value::Array(items)) = value else {
        let message = format!(
            "Invalid input: expected array, received {}",
            super::input::type_name(value)
        );
        input.issue(path, "invalid_type", &message);
        return None;
    };
    if items.is_empty() {
        input.issue(
            path,
            "too_small",
            "Too small: expected array to have >=1 items",
        );
        return None;
    }
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let mut at = path.clone();
        at.push(Seg::Index(i));
        out.push(input.string_at(&at, Some(item), NON_EMPTY, true)?);
    }
    Some(out)
}

/// `adsConversionRuleTriggerSchema` (discriminated on `type`).
fn trigger(input: &mut Input, value: Option<&Value>) -> Option<Value> {
    let path = key("trigger");
    let Some(Value::Object(obj)) = value else {
        let message = format!(
            "Invalid input: expected object, received {}",
            super::input::type_name(value)
        );
        input.issue(&path, "invalid_type", &message);
        return None;
    };
    let kind = obj.get("type").and_then(Value::as_str).unwrap_or_default();
    let list_key = match kind {
        "templateSent" => "templateIds",
        "tagApplied" => "tagIds",
        "keywordMatched" => "automatedResponseIds",
        "contactReplied" => {
            let flag = input.bool_at(
                &child(&path, "firstReplyOnly"),
                obj.get("firstReplyOnly"),
                true,
            )?;
            return Some(json!({ "type": kind, "firstReplyOnly": flag }));
        }
        _ => {
            input.issue(&child(&path, "type"), "invalid_union", "Invalid input");
            return None;
        }
    };
    let items = string_list(input, &child(&path, list_key), obj.get(list_key))?;
    Some(json!({ "type": kind, list_key: items }))
}

fn enum_field(input: &mut Input, name: &str, options: &[&str], required: bool) -> Option<String> {
    let value = input.get(name).cloned();
    input.enum_at(&key(name), value.as_ref(), options, required)
}

/// The shared keys of create and update (`None` = not sent).
fn patch(input: &mut Input) -> rules::Patch {
    let channel = enum_field(input, "channel", CHANNELS, false);
    let fks = FK_KEYS.map(|k| input.nullish_id(k));
    let ad_account_id = input.nullish_string("adAccountId", NON_EMPTY);
    let event_type = enum_field(input, "eventType", EVENT_TYPES, false);
    let trigger = match input.get("trigger").cloned() {
        None => None,
        Some(value) => trigger(input, Some(&value)),
    };
    let mark_as = input.nullish_string("markAs", NON_EMPTY);
    let enabled = input.opt_bool("enabled");
    rules::Patch {
        channel,
        fks,
        ad_account_id,
        event_type,
        trigger,
        mark_as,
        enabled,
    }
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "ads.listRules" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let channel = enum_field(&mut input, "channel", CHANNELS, false);
            input.finish()?;
            let all = rules::list(db, ws, channel)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            // `paginateInMemory`.
            let start = ((page - 1) * per_page).max(0) as usize;
            let data: Vec<Value> = all
                .iter()
                .skip(start)
                .take(per_page as usize)
                .cloned()
                .collect();
            let page_count = ((all.len() as i64 + per_page - 1) / per_page).max(1);
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "ads.getRule" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.id("id");
            input.finish()?;
            rules::find_or_fail(db, ws, id.unwrap_or_default())
                .await
                .map(|rule| json(200, rule))
                .map_err(|e| business_error(e, FINDING))
        }
        "ads.createRule" => {
            let mut input = Input::from_body(call.params, call.body);
            if !input.has("channel") {
                enum_field(&mut input, "channel", CHANNELS, true);
            }
            if !input.has("eventType") {
                enum_field(&mut input, "eventType", EVENT_TYPES, true);
            }
            if !input.has("trigger") {
                trigger(&mut input, None);
            }
            let p = patch(&mut input);
            input.finish()?;
            rules::create(
                db,
                ws,
                rules::CreateInput {
                    channel: p.channel.unwrap_or_default(),
                    fks: p.fks.map(Option::flatten),
                    ad_account_id: p.ad_account_id.flatten(),
                    event_type: p.event_type.unwrap_or_default(),
                    trigger: p.trigger.unwrap_or_default(),
                    mark_as: p.mark_as.flatten(),
                    enabled: p.enabled,
                },
            )
            .await
            .map(|rule| json(201, rule))
            .map_err(|e| business_error(e, CREATING))
        }
        "ads.updateRule" | "ads.updateRuleLegacy" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.id("id");
            let p = patch(&mut input);
            input.finish()?;
            rules::update(db, ws, id.unwrap_or_default(), p)
                .await
                .map(|rule| json(200, rule))
                .map_err(|e| business_error(e, MUTATING))
        }
        "ads.toggleRuleStatus" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.id("id");
            let enabled = input.bool("enabled");
            input.finish()?;
            rules::toggle(db, ws, id.unwrap_or_default(), enabled.unwrap_or_default())
                .await
                .map(|rule| json(200, rule))
                .map_err(|e| business_error(e, MUTATING))
        }
        "ads.deleteRule" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.id("id");
            input.finish()?;
            rules::remove(db, ws, id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "ads.referralStats" => {
            let mut input = Input::from_query(call.params, call.query);
            let from = input.opt_string("from", DATE_KEY);
            let to = input.opt_string("to", DATE_KEY);
            let tz = input.opt_string("tz", Str::max(64));
            let channel = enum_field(&mut input, "channel", ELIGIBLE_CHANNELS, false);
            let limit = input.opt_coerce_int("limit", Num::int(1.0, 100.0));
            input.finish()?;
            referral::ad_stats(db, ws, from, to, tz, channel, limit.unwrap_or(50))
                .await
                .map(|data| json(200, json!({ "data": data })))
                .map_err(|e| business_error(e, REPORTING))
        }
        "ads.listReferralAds" => {
            let mut input = Input::from_query(call.params, call.query);
            let search = input.opt_string("search", Str::trimmed(0, 100));
            let limit = input.opt_coerce_int("limit", Num::int(1.0, 100.0));
            input.finish()?;
            let search = search.filter(|s| !s.is_empty());
            referral::recent_ads(db, ws, search, limit.unwrap_or(50))
                .await
                .map(|data| json(200, json!({ "data": data })))
                .map_err(|e| business_error(e, LISTING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
