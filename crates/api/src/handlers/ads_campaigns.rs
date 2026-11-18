//! `features/ads-campaign/api/public.ts` (the campaign writes, insights,
//! wizard reads and uploads) and `ads.startRetargetAudienceSync`
//! (`features/ads/api/public.ts`). Scope `ads`; the workspace comes from the
//! token. A read-only token's `refresh` is ignored (`resolveForceRefresh`).

use base64::Engine as _;
use chatbotx_business::ads::campaigns::{self as service, CreateDraft, Ident};
use chatbotx_business::ads::retarget::{self, StartSync};
use chatbotx_business::ads::rules::CHANNELS;
use serde_json::{Map, Value, json};

use super::ads::{
    MESSAGING_CHANNELS, NON_EMPTY, ad_account_id, date_at, enum_field, integration_identity, opt_id,
};
use super::input::{Input, Num, Path, Seg, Str, child, key, type_name};
use super::{ApiState, Call, ErrorMap, Principal, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "ads.createCampaign",
    "ads.listCampaigns",
    "ads.retryCampaign",
    "ads.publishCampaign",
    "ads.pauseCampaign",
    "ads.deleteCampaign",
    "ads.getCampaignsInsights",
    "ads.listCampaignAdAccounts",
    "ads.getCampaignAdAccountDetails",
    "ads.createCampaignImageUpload",
    "ads.uploadCampaignVideo",
    "ads.getCampaignVideoStatus",
    "ads.listCampaignMessengerPages",
    "ads.startRetargetAudienceSync",
];

const RECONNECT: (&str, u16) = ("messagingAdsReconnectRequired", 409);
/// `possibleErrorsOnReadingMessagingAds`.
const READING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    RECONNECT,
    ("invalidRequest", 400),
];
/// `possibleErrorsOnCreatingMessagingAd`.
const CREATING_AD: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("notFound", 404),
    RECONNECT,
    ("invalidRequest", 400),
    ("messagingAdPageMissing", 400),
    ("messagingAdInstagramActorMissing", 400),
    ("messagingAdWhatsappPageRequired", 400),
    ("messagingAdWhatsappPhoneMissing", 400),
];
/// `possibleErrorsOnChangingMessagingAd` / `...RetryingMessagingAd`.
const CHANGING_AD: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    RECONNECT,
    ("messagingAdNotRetryable", 409),
    ("messagingAdNotPublishable", 409),
    ("invalidRequest", 400),
    ("messagingAdPageMissing", 400),
    ("messagingAdInstagramActorMissing", 400),
    ("messagingAdWhatsappPageRequired", 400),
    ("messagingAdWhatsappPhoneMissing", 400),
];
/// `possibleErrorsOnCreatingAdImageUpload`.
const IMAGE_UPLOAD: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("adsCreativeUnsupportedImage", 400),
    ("adsCreativeImageTooLarge", 400),
    RECONNECT,
];
/// `possibleErrorsOnCreatingResource`.
const CREATING: ErrorMap = super::CREATING;

/// `specialAdCategories`.
const SPECIAL_AD_CATEGORIES: &[&str] = &[
    "NONE",
    "HOUSING",
    "EMPLOYMENT",
    "CREDIT",
    "FINANCIAL_PRODUCTS_SERVICES",
    "ISSUES_ELECTIONS_POLITICS",
    "ONLINE_GAMBLING_AND_GAMING",
];
const DATE_PRESETS: &[&str] = &["maximum", "last_30d", "last_7d"];
const MAX_INSIGHTS_AD_IDS: usize = 500;
const MAX_VIDEO_BASE64_LENGTH: usize = 25_000_000;

const ACCOUNT: Str = Str {
    trim: true,
    min: None,
    max: None,
    pattern: Some(ad_account_id),
};

fn video_mime(text: &str) -> bool {
    matches!(text, "video/mp4" | "video/quicktime")
}

fn index(path: &[Seg], i: usize) -> Path {
    let mut next = path.to_vec();
    next.push(Seg::Index(i));
    next
}

/// A required (or optional) object at `path`.
fn object_at<'a>(
    input: &mut Input,
    path: &[Seg],
    value: Option<&'a Value>,
    required: bool,
) -> Option<&'a Map<String, Value>> {
    match value {
        Some(Value::Object(map)) => Some(map),
        None if !required => None,
        other => {
            input.issue(
                path,
                "invalid_type",
                &format!(
                    "Invalid input: expected object, received {}",
                    type_name(other)
                ),
            );
            None
        }
    }
}

/// An array at `path` with its length bounds.
fn array_at<'a>(
    input: &mut Input,
    path: &[Seg],
    value: Option<&'a Value>,
    required: bool,
    min: Option<usize>,
    max: Option<usize>,
) -> Option<&'a Vec<Value>> {
    let items = match value {
        Some(Value::Array(items)) => items,
        None if !required => return None,
        other => {
            input.issue(
                path,
                "invalid_type",
                &format!(
                    "Invalid input: expected array, received {}",
                    type_name(other)
                ),
            );
            return None;
        }
    };
    if let Some(min) = min.filter(|&m| items.len() < m) {
        input.issue(
            path,
            "too_small",
            &format!("Too small: expected array to have >={min} items"),
        );
    }
    if let Some(max) = max.filter(|&m| items.len() > m) {
        input.issue(
            path,
            "too_big",
            &format!("Too big: expected array to have <={max} items"),
        );
    }
    Some(items)
}

/// A string key of `map` checked by `spec`, copied (trimmed) into `out`.
fn copy_string(
    input: &mut Input,
    out: &mut Map<String, Value>,
    path: &[Seg],
    map: &Map<String, Value>,
    name: &str,
    spec: Str,
    required: bool,
) -> Option<String> {
    let text = input.string_at(&child(path, name), map.get(name), spec, required)?;
    out.insert(name.into(), json!(text));
    Some(text)
}

/// A `z.coerce.number().int()` key of `map` copied into `out`.
fn copy_int(
    input: &mut Input,
    out: &mut Map<String, Value>,
    path: &[Seg],
    map: &Map<String, Value>,
    name: &str,
    spec: Num,
    required: bool,
) -> Option<i64> {
    let number = input.coerce_number_at(&child(path, name), map.get(name), spec, required)?;
    let number = number as i64;
    out.insert(name.into(), json!(number));
    Some(number)
}

fn string_list(input: &mut Input, path: &[Seg], items: &[Value], spec: Str) -> Vec<Value> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, v)| input.string_at(&index(path, i), Some(v), spec, true))
        .map(Value::String)
        .collect()
}

const COUNTRY: Str = Str {
    trim: true,
    min: Some(2),
    max: Some(2),
    pattern: None,
};

/// `messagingAdTargetingSchema` (with its `ageMin <= ageMax` refine).
fn targeting(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Value {
    let mut out = Map::new();
    let Some(map) = object_at(input, path, value, true) else {
        return Value::Object(out);
    };
    let before = input.issue_count();
    let countries_path = child(path, "countries");
    if let Some(items) = array_at(
        input,
        &countries_path,
        map.get("countries"),
        true,
        Some(1),
        None,
    ) {
        let list = string_list(input, &countries_path, items, COUNTRY);
        out.insert("countries".into(), Value::Array(list));
    }
    let age = Num::int(13.0, 65.0);
    let age_min = copy_int(input, &mut out, path, map, "ageMin", age, false);
    let age_max = copy_int(input, &mut out, path, map, "ageMax", age, false);
    let genders_path = child(path, "genders");
    if let Some(items) = array_at(input, &genders_path, map.get("genders"), false, None, None) {
        let mut list = Vec::new();
        for (i, item) in items.iter().enumerate() {
            if item == &json!(1) || item == &json!(2) {
                list.push(item.clone());
            } else {
                input.issue(&index(&genders_path, i), "invalid_union", "Invalid input");
            }
        }
        out.insert("genders".into(), Value::Array(list));
    }
    let locales_path = child(path, "locales");
    if let Some(items) = array_at(input, &locales_path, map.get("locales"), false, None, None) {
        let mut list = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let spec = Num {
                int: true,
                ..Num::default()
            };
            if let Some(n) =
                input.coerce_number_at(&index(&locales_path, i), Some(item), spec, true)
            {
                list.push(json!(n as i64));
            }
        }
        out.insert("locales".into(), Value::Array(list));
    }
    if input.issue_count() == before
        && let (Some(min), Some(max)) = (age_min, age_max)
        && min > max
    {
        input.issue(
            &child(path, "ageMax"),
            "custom",
            "ageMax must be greater than or equal to ageMin",
        );
    }
    Value::Object(out)
}

const QUICK_REPLY: Str = Str::trimmed(1, 20);
const WELCOME_TEXT: Str = Str::trimmed(1, 2000);

fn quick_replies(
    input: &mut Input,
    path: &[Seg],
    map: &Map<String, Value>,
    out: &mut Map<String, Value>,
) {
    let path = child(path, "quickReplies");
    let Some(items) = array_at(input, &path, map.get("quickReplies"), false, None, Some(3)) else {
        return;
    };
    let mut list = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let item_path = index(&path, i);
        if let Some(reply) = object_at(input, &item_path, Some(item), true) {
            let mut one = Map::new();
            copy_string(
                input,
                &mut one,
                &item_path,
                reply,
                "title",
                QUICK_REPLY,
                true,
            );
            list.push(Value::Object(one));
        }
    }
    out.insert("quickReplies".into(), Value::Array(list));
}

/// A discriminated union's tag at `path.<tag>`.
fn tag(
    input: &mut Input,
    path: &[Seg],
    map: &Map<String, Value>,
    name: &str,
    options: &[&str],
) -> Option<String> {
    let value = map.get(name).and_then(Value::as_str);
    match value.filter(|v| options.contains(v)) {
        Some(v) => Some(v.to_owned()),
        None => {
            input.issue(&child(path, name), "invalid_union", "Invalid input");
            None
        }
    }
}

/// `welcomeMessageSchema`.
fn welcome(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Value {
    let mut out = Map::new();
    let Some(map) = object_at(input, path, value, true) else {
        return Value::Object(out);
    };
    let Some(kind) = tag(
        input,
        path,
        map,
        "type",
        &["default", "single", "templates"],
    ) else {
        return Value::Object(out);
    };
    out.insert("type".into(), json!(kind));
    match kind.as_str() {
        "single" => {
            copy_string(input, &mut out, path, map, "message", WELCOME_TEXT, true);
            quick_replies(input, path, map, &mut out);
        }
        "templates" => {
            let list_path = child(path, "templates");
            if let Some(items) = array_at(
                input,
                &list_path,
                map.get("templates"),
                true,
                Some(1),
                Some(5),
            ) {
                let mut list = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    let item_path = index(&list_path, i);
                    if let Some(template) = object_at(input, &item_path, Some(item), true) {
                        let mut one = Map::new();
                        let heading = Str {
                            trim: true,
                            max: Some(80),
                            ..Str::default()
                        };
                        copy_string(
                            input, &mut one, &item_path, template, "heading", heading, false,
                        );
                        copy_string(
                            input,
                            &mut one,
                            &item_path,
                            template,
                            "message",
                            WELCOME_TEXT,
                            true,
                        );
                        quick_replies(input, &item_path, template, &mut one);
                        list.push(Value::Object(one));
                    }
                }
                out.insert("templates".into(), Value::Array(list));
            }
        }
        _ => {}
    }
    Value::Object(out)
}

fn max_trim(max: usize) -> Str {
    Str {
        trim: true,
        max: Some(max),
        ..Str::default()
    }
}

/// `creativeMediaSchema`.
fn media(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Value {
    let mut out = Map::new();
    let Some(map) = object_at(input, path, value, true) else {
        return Value::Object(out);
    };
    let Some(kind) = tag(input, path, map, "kind", &["image", "video"]) else {
        return Value::Object(out);
    };
    out.insert("kind".into(), json!(kind));
    if kind == "image" {
        copy_string(
            input,
            &mut out,
            path,
            map,
            "imageKey",
            Str::trimmed(1, 1024),
            true,
        );
        copy_string(input, &mut out, path, map, "fileId", NON_EMPTY, true);
        copy_string(
            input,
            &mut out,
            path,
            map,
            "imageMimeType",
            max_trim(255),
            false,
        );
        copy_string(
            input,
            &mut out,
            path,
            map,
            "imageFileName",
            max_trim(255),
            false,
        );
        let link_path = child(path, "link");
        if let Some(link) = input.string_at(&link_path, map.get("link"), Str::default(), true) {
            if url::Url::parse(&link).is_ok() {
                out.insert("link".into(), json!(link));
            } else {
                input.issue(&link_path, "invalid_format", "Invalid URL");
            }
        }
        copy_string(input, &mut out, path, map, "message", max_trim(500), false);
        copy_string(input, &mut out, path, map, "headline", max_trim(40), false);
        copy_string(
            input,
            &mut out,
            path,
            map,
            "description",
            max_trim(200),
            false,
        );
        copy_string(input, &mut out, path, map, "caption", max_trim(30), false);
    } else {
        copy_string(input, &mut out, path, map, "videoId", NON_EMPTY, true);
        copy_string(
            input,
            &mut out,
            path,
            map,
            "thumbnailImageHash",
            NON_EMPTY,
            false,
        );
        copy_string(input, &mut out, path, map, "title", max_trim(40), false);
        copy_string(input, &mut out, path, map, "message", max_trim(500), false);
        copy_string(
            input,
            &mut out,
            path,
            map,
            "linkDescription",
            max_trim(200),
            false,
        );
    }
    Value::Object(out)
}

/// `createMessagingAdPublicRequest` re-checked by `createMessagingAdRequest`
/// (CREDIT rejection, the politics country, the ad set times and the image
/// key's workspace prefix).
fn create_campaign_input(input: &mut Input, ws: i64) -> Option<CreateDraft> {
    let body = Value::Object(input.obj.clone());
    let channel = enum_field(input, "channel", MESSAGING_CHANNELS, true);
    let integration_id = input.id("integrationId");
    let whatsapp_page = opt_id(input, "whatsappPageIntegrationId");
    let account = input.string("adAccountId", ACCOUNT);
    let name = input.string("name", Str::trimmed(1, 120));

    let mut campaign = Map::new();
    let campaign_path = key("campaign");
    if let Some(map) = object_at(input, &campaign_path, body.get("campaign"), true) {
        let before = input.issue_count();
        let path = child(&campaign_path, "specialAdCategories");
        if let Some(items) = array_at(
            input,
            &path,
            map.get("specialAdCategories"),
            true,
            Some(1),
            None,
        ) {
            let mut list = Vec::new();
            for (i, item) in items.iter().enumerate() {
                if let Some(c) =
                    input.enum_at(&index(&path, i), Some(item), SPECIAL_AD_CATEGORIES, true)
                {
                    list.push(json!(c));
                }
            }
            if input.issue_count() == before && list.iter().any(|c| c == "CREDIT") {
                input.issue(
                    &path,
                    "custom",
                    "CREDIT is deprecated — use FINANCIAL_PRODUCTS_SERVICES instead.",
                );
            }
            campaign.insert("specialAdCategories".into(), Value::Array(list));
        }
        let country_path = child(&campaign_path, "specialAdCategoryCountry");
        if let Some(items) = array_at(
            input,
            &country_path,
            map.get("specialAdCategoryCountry"),
            false,
            None,
            None,
        ) {
            let list = string_list(input, &country_path, items, COUNTRY);
            campaign.insert("specialAdCategoryCountry".into(), Value::Array(list));
        }
        let politics = campaign["specialAdCategories"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c == "ISSUES_ELECTIONS_POLITICS"));
        let has_country = campaign
            .get("specialAdCategoryCountry")
            .and_then(Value::as_array)
            .is_some_and(|c| !c.is_empty());
        if input.issue_count() == before && politics && !has_country {
            input.issue(
                &country_path,
                "custom",
                "A country is required for the social issues, elections or politics category.",
            );
        }
    }

    let mut ad_set = Map::new();
    let ad_set_path = key("adSet");
    if let Some(map) = object_at(input, &ad_set_path, body.get("adSet"), true) {
        let before = input.issue_count();
        let budget_path = child(&ad_set_path, "dailyBudgetMinorUnits");
        let spec = Num {
            int: true,
            ..Num::default()
        };
        if let Some(budget) =
            input.coerce_number_at(&budget_path, map.get("dailyBudgetMinorUnits"), spec, true)
        {
            if budget <= 0.0 {
                input.issue(
                    &budget_path,
                    "too_small",
                    "Too small: expected number to be >0",
                );
            }
            ad_set.insert("dailyBudgetMinorUnits".into(), json!(budget as i64));
        }
        let targeting = targeting(
            input,
            &child(&ad_set_path, "targeting"),
            map.get("targeting"),
        );
        ad_set.insert("targeting".into(), targeting);
        let time = Str {
            trim: true,
            ..Str::default()
        };
        let start = copy_string(
            input,
            &mut ad_set,
            &ad_set_path,
            map,
            "startTime",
            time,
            false,
        );
        let end = copy_string(
            input,
            &mut ad_set,
            &ad_set_path,
            map,
            "endTime",
            time,
            false,
        );
        if input.issue_count() == before
            && let (Some(start), Some(end)) = (
                start.filter(|s| !s.is_empty()),
                end.filter(|e| !e.is_empty()),
            )
        {
            let parse = chatbotx_core::js::parse_date;
            let ordered = matches!((parse(&start), parse(&end)), (Some(s), Some(e)) if s < e);
            if !ordered {
                input.issue(
                    &child(&ad_set_path, "endTime"),
                    "custom",
                    "endTime must be after startTime",
                );
            }
        }
    }

    let mut creative = Map::new();
    let creative_path = key("creative");
    if let Some(map) = object_at(input, &creative_path, body.get("creative"), true) {
        creative.insert(
            "media".into(),
            media(input, &child(&creative_path, "media"), map.get("media")),
        );
        creative.insert(
            "welcomeMessage".into(),
            welcome(
                input,
                &child(&creative_path, "welcomeMessage"),
                map.get("welcomeMessage"),
            ),
        );
    }

    if input.failed() {
        return None;
    }
    let media = &creative["media"];
    if media["kind"] == "image"
        && !media["imageKey"]
            .as_str()
            .is_some_and(|k| k.starts_with(&format!("{}/", service::creative_prefix(ws))))
    {
        input.issue(
            &[
                key("creative"),
                vec![Seg::Key("media".into()), Seg::Key("imageKey".into())],
            ]
            .concat(),
            "custom",
            "Image is not owned by this workspace.",
        );
        return None;
    }
    let name = name.unwrap_or_default();
    let account = account.unwrap_or_default();
    let mut operation = json!({
        "adAccountId": account,
        "campaign": { "name": name, "specialAdCategories": campaign["specialAdCategories"] },
        "adSet": ad_set,
        "creative": creative,
    });
    if let Some(page) = whatsapp_page {
        operation["whatsappPageIntegrationId"] = json!(page.to_string());
    }
    if let Some(country) = campaign.get("specialAdCategoryCountry") {
        operation["campaign"]["specialAdCategoryCountry"] = country.clone();
    }
    Some(CreateDraft {
        ident: Ident {
            workspace_id: ws,
            channel: channel.unwrap_or_default(),
            integration_id: integration_id.unwrap_or_default(),
        },
        whatsapp_page_integration_id: whatsapp_page,
        ad_account_id: account,
        name,
        input: operation,
    })
}

fn ident(ws: i64, (channel, integration_id): (String, i64)) -> Ident {
    Ident {
        workspace_id: ws,
        channel,
        integration_id,
    }
}

/// `resolveForceRefresh`.
fn force(principal: &Principal, refresh: Option<bool>) -> bool {
    !principal.read_only && refresh.unwrap_or(false)
}

fn op(record: &chatbotx_db::repo::models::MessagingAdOperation) -> Value {
    service::resource(record, None)
}

pub async fn handle(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let files = state.files.as_deref();
    match route_id {
        "ads.createCampaign" => {
            let mut input = Input::from_body(call.params, call.body);
            let draft = create_campaign_input(&mut input, ws);
            input.finish()?;
            let Some(draft) = draft else {
                return Err(super::internal_error("invalid campaign input"));
            };
            service::create_draft(db, files, draft)
                .await
                .map(|r| json(201, op(&r)))
                .map_err(|e| business_error(e, CREATING_AD))
        }
        "ads.retryCampaign"
        | "ads.publishCampaign"
        | "ads.pauseCampaign"
        | "ads.deleteCampaign" => {
            let mut input = if route_id == "ads.deleteCampaign" {
                Input::from_query(call.params, call.query)
            } else {
                Input::from_body(call.params, call.body)
            };
            let id = input.id("id");
            input.finish()?;
            let id = id.unwrap_or_default();
            let result = match route_id {
                "ads.retryCampaign" => service::retry_draft(db, files, ws, id).await,
                "ads.publishCampaign" => service::publish(db, ws, id).await,
                "ads.pauseCampaign" => service::pause(db, ws, id).await,
                _ => service::delete(db, files, ws, id).await,
            };
            result
                .map(|r| json(200, op(&r)))
                .map_err(|e| business_error(e, CHANGING_AD))
        }
        "ads.listCampaigns" => {
            let mut input = Input::from_query(call.params, call.query);
            let identity = integration_identity(&mut input);
            let refresh = input.opt_coerced_bool("refresh");
            input.finish()?;
            service::list(db, &ident(ws, identity), force(principal, refresh))
                .await
                .map(|b| json(200, b))
                .map_err(|e| business_error(e, READING))
        }
        "ads.getCampaignsInsights" => {
            let mut input = Input::from_body(call.params, call.body);
            let identity = integration_identity(&mut input);
            let account = input.string("adAccountId", ACCOUNT);
            let ids_path = key("adIds");
            let ids_value = input.get("adIds").cloned();
            let ad_ids = array_at(
                &mut input,
                &ids_path,
                ids_value.as_ref(),
                true,
                Some(1),
                Some(MAX_INSIGHTS_AD_IDS),
            )
            .cloned()
            .map(|items| string_list(&mut input, &ids_path, &items, NON_EMPTY));
            let preset = enum_field(&mut input, "datePreset", DATE_PRESETS, false);
            let refresh = input.opt_bool("refresh");
            input.finish()?;
            let ad_ids: Vec<String> = ad_ids
                .unwrap_or_default()
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            service::insights(
                db,
                &ident(ws, identity),
                &account.unwrap_or_default(),
                &ad_ids,
                preset.as_deref(),
                force(principal, refresh),
            )
            .await
            .map(|b| json(200, b))
            .map_err(|e| business_error(e, READING))
        }
        "ads.listCampaignAdAccounts" => {
            let mut input = Input::from_query(call.params, call.query);
            let identity = integration_identity(&mut input);
            let refresh = input.opt_coerced_bool("refresh");
            input.finish()?;
            service::ad_accounts(db, &ident(ws, identity), force(principal, refresh))
                .await
                .map(|data| json(200, json!({ "data": data })))
                .map_err(|e| business_error(e, READING))
        }
        "ads.getCampaignAdAccountDetails" => {
            let mut input = Input::from_query(call.params, call.query);
            let account = input.string("adAccountId", ACCOUNT);
            let identity = integration_identity(&mut input);
            let refresh = input.opt_coerced_bool("refresh");
            input.finish()?;
            service::ad_account_details(
                db,
                &ident(ws, identity),
                &account.unwrap_or_default(),
                force(principal, refresh),
            )
            .await
            .map(|b| json(200, b))
            .map_err(|e| business_error(e, READING))
        }
        "ads.createCampaignImageUpload" => {
            let mut input = Input::from_body(call.params, call.body);
            let file_name = input.string("fileName", Str::trimmed(1, 255));
            let mime_type = input.string("mimeType", Str::trimmed(1, 255));
            let size_value = input.get("fileSize").cloned();
            let size = input.number_at(
                &key("fileSize"),
                size_value.as_ref(),
                Num {
                    int: true,
                    ..Num::default()
                },
                true,
            );
            if size.is_some_and(|s| s <= 0.0) {
                input.issue(
                    &key("fileSize"),
                    "too_small",
                    "Too small: expected number to be >0",
                );
            }
            input.finish()?;
            let Some(uploader) = files else {
                return Err(super::internal_error("file storage is off"));
            };
            let storage_url = super::contacts::tenant_urls(state).storage_url;
            service::create_image_upload(
                db,
                uploader,
                &storage_url,
                ws,
                &file_name.unwrap_or_default(),
                &mime_type.unwrap_or_default(),
                size.unwrap_or_default(),
            )
            .await
            .map(|b| json(201, b))
            .map_err(|e| business_error(e, IMAGE_UPLOAD))
        }
        "ads.uploadCampaignVideo" => {
            let mut input = Input::from_body(call.params, call.body);
            let identity = integration_identity(&mut input);
            let account = input.string("adAccountId", ACCOUNT);
            let file_name = input.string("fileName", Str::trimmed(1, 255));
            let mime_type = input.string(
                "mimeType",
                Str {
                    trim: true,
                    pattern: Some(video_mime),
                    ..Str::default()
                },
            );
            let base64 = input.string("base64", Str::trimmed(1, MAX_VIDEO_BASE64_LENGTH));
            input.finish()?;
            let bytes = decode_base64(&base64.unwrap_or_default());
            service::upload_video(
                db,
                &ident(ws, identity),
                &account.unwrap_or_default(),
                &file_name.unwrap_or_default(),
                &mime_type.unwrap_or_default(),
                &bytes,
            )
            .await
            .map(|b| json(200, b))
            .map_err(|e| business_error(e, CREATING_AD))
        }
        "ads.getCampaignVideoStatus" => {
            let mut input = Input::from_query(call.params, call.query);
            let video_id = input.string("videoId", NON_EMPTY);
            let identity = integration_identity(&mut input);
            input.finish()?;
            service::video_status(db, &ident(ws, identity), &video_id.unwrap_or_default())
                .await
                .map(|b| json(200, b))
                .map_err(|e| business_error(e, READING))
        }
        "ads.listCampaignMessengerPages" => {
            let mut input = Input::from_query(call.params, call.query);
            let channel = enum_field(&mut input, "channel", MESSAGING_CHANNELS, true);
            let _ = opt_id(&mut input, "integrationId");
            input.finish()?;
            if channel.as_deref() != Some("whatsapp") {
                return Err(business_error(
                    chatbotx_business::errors::BusinessError::exception(
                        "Messenger pages are only listed for the WhatsApp channel",
                        "invalidRequest",
                        400,
                    ),
                    READING,
                ));
            }
            service::messenger_pages(db, ws)
                .await
                .map(|b| json(200, b))
                .map_err(|e| business_error(e, READING))
        }
        "ads.startRetargetAudienceSync" => {
            let mut input = Input::from_body(call.params, call.body);
            let segment = enum_field(
                &mut input,
                "segment",
                &["conversations", "leads", "purchases"],
                true,
            );
            let ad_id = input.nullish_string("adId", NON_EMPTY).flatten();
            let channel = enum_field(&mut input, "channel", CHANNELS, false);
            let fk = |input: &mut Input, name: &str| input.opt_string(name, Str::default());
            let integration_whatsapp_id = fk(&mut input, "integrationWhatsappId");
            let integration_messenger_id = fk(&mut input, "integrationMessengerId");
            let integration_instagram_id = fk(&mut input, "integrationInstagramId");
            let since = date_at(&mut input, "since");
            let until = date_at(&mut input, "until");
            let account = input.string("adAccountId", NON_EMPTY);
            let audience_name = input.opt_string("audienceName", NON_EMPTY);
            let custom_audience_id = input.opt_string("customAudienceId", NON_EMPTY);
            if !input.failed() && audience_name.is_none() && custom_audience_id.is_none() {
                input.issue(
                    &key("audienceName"),
                    "custom",
                    "Either audienceName or customAudienceId is required",
                );
            }
            input.finish()?;
            let start = StartSync {
                workspace_id: ws,
                segment: segment.unwrap_or_default(),
                ad_id,
                channel,
                integration_whatsapp_id,
                integration_messenger_id,
                integration_instagram_id,
                since: since.unwrap_or_default(),
                until: until.unwrap_or_default(),
                ad_account_id: account.unwrap_or_default(),
                audience_name,
                custom_audience_id,
            };
            retarget::start_sync(db, state.jobs.as_ref(), start)
                .await
                .map(|b| json(202, b))
                .map_err(|e| business_error(e, CREATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `Buffer.from(base64, "base64")`: lenient (whitespace and padding are
/// optional; other characters stop the decode).
fn decode_base64(text: &str) -> Vec<u8> {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            other => other,
        })
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/')
        .collect();
    let usable = clean.len() - clean.len() % 4
        + if clean.len() % 4 >= 2 {
            clean.len() % 4
        } else {
            0
        };
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_allow_trailing_bits(true)
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    engine.decode(&clean[..usable]).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64_like_node() {
        assert_eq!(decode_base64("aGk="), b"hi");
        assert_eq!(decode_base64("aGk"), b"hi");
        assert_eq!(decode_base64("aG k=\n"), b"hi");
        assert_eq!(decode_base64("aGVsbG8"), b"hello");
    }

    #[test]
    fn validates_a_campaign() {
        let body = json!({
            "channel": "messenger",
            "integrationId": "5",
            "adAccountId": "act_1",
            "name": " Ad ",
            "campaign": { "specialAdCategories": ["NONE"] },
            "adSet": { "dailyBudgetMinorUnits": "750", "targeting": { "countries": ["US"], "ageMin": 30, "ageMax": 20 } },
            "creative": { "media": { "kind": "video", "videoId": "1" }, "welcomeMessage": { "type": "default" } },
        });
        let mut input = Input::from_body(&[], body.to_string().as_bytes());
        assert!(create_campaign_input(&mut input, 1).is_none());
        assert_eq!(input.issue_count(), 1);

        let mut ok = body.clone();
        ok["adSet"]["targeting"] = json!({ "countries": ["US"] });
        let mut input = Input::from_body(&[], ok.to_string().as_bytes());
        let draft = create_campaign_input(&mut input, 1).unwrap();
        assert_eq!(draft.name, "Ad");
        assert_eq!(draft.input["adSet"]["dailyBudgetMinorUnits"], 750);
        assert_eq!(draft.input["campaign"]["name"], "Ad");
    }
}
