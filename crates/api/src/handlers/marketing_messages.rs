//! `features/facebook-marketing-messages/api/public.ts`:
//! `facebookMarketingMessages.*` (scope `ads`).

use chatbotx_business::marketing_messages::{audiences, campaigns, content, delivery, statistics};
use chatbotx_core::flow_config::z::{self, Schema};
use serde_json::{Value, json};

use super::input::{Input, Str, key};
use super::{ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "facebookMarketingMessages.pages.list",
    "facebookMarketingMessages.adAccounts.list",
    "facebookMarketingMessages.campaigns.list",
    "facebookMarketingMessages.campaigns.get",
    "facebookMarketingMessages.campaigns.create",
    "facebookMarketingMessages.campaigns.update",
    "facebookMarketingMessages.campaigns.delete",
    "facebookMarketingMessages.campaigns.stop",
    "facebookMarketingMessages.campaigns.resume",
    "facebookMarketingMessages.campaigns.send",
    "facebookMarketingMessages.audiences.list",
    "facebookMarketingMessages.audiences.liveStatus",
    "facebookMarketingMessages.audiences.create",
    "facebookMarketingMessages.audiences.uploadMore",
    "facebookMarketingMessages.audiences.delete",
    "facebookMarketingMessages.statistics.get",
];

fn day(text: &str) -> bool {
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

fn act_id(text: &str) -> bool {
    text.strip_prefix("act_")
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// `AUDIENCE_PUBLIC_MAX_ROWS`.
const AUDIENCE_MAX_ROWS: usize = 100_000;

/// `audienceTableSchema`.
fn table() -> Vec<(&'static str, Schema)> {
    vec![
        ("headers", z::array(z::string()).min(1)),
        (
            "rows",
            z::array(z::array(z::string())).max(AUDIENCE_MAX_ROWS),
        ),
    ]
}

/// `createAudiencePublicRequest`.
fn create_audience_schema() -> Schema {
    let mut shape = table();
    shape.extend([
        ("adAccountId", z::string().trim().min(1)),
        ("pageId", z::string().trim().min(1)),
        ("name", z::string().trim().min(1).max(100)),
        ("description", z::string().trim().max(500).optional()),
    ]);
    z::object(shape)
}

/// `uploadMoreAudiencePublicRequest`.
fn upload_more_schema() -> Schema {
    let mut shape = table();
    shape.push(("id", z::string()));
    z::object(shape)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|s| s.as_str().unwrap_or_default().to_owned())
        .collect()
}

fn table_of(v: &Value) -> (Vec<String>, Vec<Vec<String>>) {
    let rows = v["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(strings)
        .collect();
    (strings(&v["headers"]), rows)
}

const MM_NAME_MAX: usize = 120;

fn fields() -> Vec<(&'static str, Schema)> {
    vec![
        ("name", z::string().trim().min(1).max(MM_NAME_MAX)),
        ("budgetMajorUnits", z::number().coerce().positive()),
        ("content", content::content_schema()),
    ]
}

/// `createCampaignPublicRequest`.
fn create_schema() -> Schema {
    let mut shape = fields();
    shape.extend([
        ("budgetType", z::enum_(&["daily", "lifetime"])),
        ("pageId", z::string().trim().min(1)),
        ("adAccountId", z::string().trim().min(1)),
        ("targetType", z::enum_(&["optIn", "customAudience"])),
        ("audienceId", z::string().nullable().optional()),
    ]);
    z::object(shape).refine(
        |v| v["targetType"] != "customAudience" || z::truthy(v.get("audienceId")),
        None,
        Some(&["audienceId"]),
    )
}

/// `updateCampaignPublicRequest`.
fn update_schema() -> Schema {
    let mut shape = fields();
    shape.push(("id", z::string()));
    z::object(shape)
}

/// Validates the whole input object with `schema` (issues go to `input`).
fn parse(input: &mut Input, schema: &Schema) -> Option<Value> {
    let parsed = schema.safe_parse(&Value::Object(input.obj.clone()));
    if parsed.success {
        return parsed.data;
    }
    let issues = parsed
        .issues()
        .iter()
        .filter_map(|issue| serde_json::to_value(issue).ok())
        .collect();
    input.extend_issues(issues);
    None
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let mut input = match route_id {
        "facebookMarketingMessages.campaigns.create"
        | "facebookMarketingMessages.campaigns.update" => Input::from_body(call.params, call.body),
        _ => Input::from_query(call.params, call.query),
    };
    let campaign =
        |row: chatbotx_db::repo::models::FacebookMarketingMessage| campaigns::resource(&row);
    let (status, errors, body) = match route_id {
        "facebookMarketingMessages.pages.list" => {
            input.finish()?;
            (
                200,
                LISTING,
                campaigns::list_pages(db, ws)
                    .await
                    .map(|p| json!({ "pages": p })),
            )
        }
        "facebookMarketingMessages.adAccounts.list" => {
            input.finish()?;
            let accounts = campaigns::list_ad_accounts(db, ws).await;
            (200, LISTING, accounts.map(|a| json!({ "accounts": a })))
        }
        "facebookMarketingMessages.campaigns.list" => {
            input.finish()?;
            let rows = campaigns::list(db, ws).await;
            (
                200,
                LISTING,
                rows.map(|r| json!({ "campaigns": r.iter().map(campaigns::resource).collect::<Vec<_>>() })),
            )
        }
        "facebookMarketingMessages.campaigns.get" => {
            let id = input.string("id", Str::default());
            input.finish()?;
            (
                200,
                FINDING,
                campaigns::get(db, ws, &id.unwrap_or_default()).await,
            )
        }
        "facebookMarketingMessages.campaigns.create" => {
            let parsed = parse(&mut input, &create_schema());
            input.finish()?;
            let v = parsed.unwrap_or_default();
            let created = campaigns::create(
                db,
                ws,
                campaigns::CreateInput {
                    name: text(&v, "name"),
                    page_id: text(&v, "pageId"),
                    ad_account_id: text(&v, "adAccountId"),
                    budget_type: text(&v, "budgetType"),
                    budget_major_units: v["budgetMajorUnits"].as_f64().unwrap_or_default(),
                    content: v["content"].clone(),
                    target_type: text(&v, "targetType"),
                    audience_id: v["audienceId"].as_str().map(str::to_owned),
                },
            )
            .await;
            (201, CREATING, created.map(campaign))
        }
        "facebookMarketingMessages.campaigns.update" => {
            let parsed = parse(&mut input, &update_schema());
            input.finish()?;
            let v = parsed.unwrap_or_default();
            let updated = campaigns::update(
                db,
                ws,
                &text(&v, "id"),
                text(&v, "name"),
                v["budgetMajorUnits"].as_f64().unwrap_or_default(),
                v["content"].clone(),
            )
            .await;
            (200, MUTATING, updated.map(campaign))
        }
        "facebookMarketingMessages.campaigns.delete" => {
            let id = input.string("id", Str::default()).unwrap_or_default();
            input.finish()?;
            let deleted = campaigns::delete(db, ws, &id).await;
            (200, MUTATING, deleted.map(|()| json!({ "id": id })))
        }
        "facebookMarketingMessages.campaigns.stop"
        | "facebookMarketingMessages.campaigns.resume"
        | "facebookMarketingMessages.campaigns.send" => {
            let id = input.string("id", Str::default()).unwrap_or_default();
            input.finish()?;
            let row = match route_id {
                "facebookMarketingMessages.campaigns.stop" => delivery::stop(db, ws, &id).await,
                "facebookMarketingMessages.campaigns.resume" => delivery::resume(db, ws, &id).await,
                _ => delivery::send(db, ws, &id).await,
            };
            (200, MUTATING, row.map(campaign))
        }
        "facebookMarketingMessages.audiences.list" => {
            let ad_account_id = input.opt_string("adAccountId", Str::default());
            let page_id = input.opt_string("pageId", Str::default());
            let ready_only = input.opt_coerced_bool("readyOnly");
            input.finish()?;
            let rows =
                audiences::list(db, ws, ad_account_id, page_id, ready_only.unwrap_or(false)).await;
            (200, LISTING, rows.map(|a| json!({ "audiences": a })))
        }
        "facebookMarketingMessages.audiences.liveStatus" => {
            input.finish()?;
            let live = audiences::live_status(db, ws).await;
            (200, LISTING, live.map(|s| json!({ "statuses": s })))
        }
        "facebookMarketingMessages.audiences.create" => {
            let parsed = parse(&mut input, &create_audience_schema());
            input.finish()?;
            let v = parsed.unwrap_or_default();
            let (headers, rows) = table_of(&v);
            let input = audiences::CreateInput {
                ad_account_id: text(&v, "adAccountId"),
                page_id: text(&v, "pageId"),
                name: text(&v, "name"),
                description: v["description"].as_str().map(str::to_owned),
                headers,
                rows,
            };
            (201, CREATING, audiences::create(db, ws, input).await)
        }
        "facebookMarketingMessages.audiences.uploadMore" => {
            let parsed = parse(&mut input, &upload_more_schema());
            input.finish()?;
            let v = parsed.unwrap_or_default();
            let (headers, rows) = table_of(&v);
            let outcome = audiences::upload_more(db, ws, &text(&v, "id"), &headers, &rows).await;
            (200, MUTATING, outcome)
        }
        "facebookMarketingMessages.audiences.delete" => {
            let id = input.string("id", Str::default()).unwrap_or_default();
            input.finish()?;
            let deleted = audiences::delete(db, ws, &id).await;
            (200, MUTATING, deleted.map(|()| json!({ "id": id })))
        }
        "facebookMarketingMessages.statistics.get" => {
            let pattern = |check: fn(&str) -> bool| Str {
                pattern: Some(check),
                ..Str::default()
            };
            let ad_account_id = input.opt_string("adAccountId", pattern(act_id));
            let since = input.opt_string("since", pattern(day));
            let until = input.opt_string("until", pattern(day));
            let search = input.opt_string(
                "search",
                Str {
                    trim: true,
                    max: Some(200),
                    ..Str::default()
                },
            );
            let (page, per_page) = super::qr_codes::public_paging(&mut input);
            let value = input.get("sort").cloned();
            let sort = input.enum_at(&key("sort"), value.as_ref(), &["campaignId", "name"], false);
            let value = input.get("order").cloned();
            let order = input.enum_at(&key("order"), value.as_ref(), &["asc", "desc"], false);
            input.finish()?;
            let stats = statistics::get(
                db,
                ws,
                statistics::Input {
                    ad_account_id,
                    since,
                    until,
                    search,
                    page,
                    per_page,
                    sort,
                    order_desc: order.as_deref() == Some("desc"),
                },
            )
            .await;
            (200, LISTING, stats)
        }
        _ => return Err(crate::routes::not_ported_error()),
    };
    body.map(|b| json(status, b))
        .map_err(|e| business_error(e, errors))
}
