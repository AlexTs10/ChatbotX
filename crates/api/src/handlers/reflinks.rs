//! `features/reflinks/api/public.ts`: `reflinks.*` (with the chat-widget
//! routes) and `lib/reflink-links.ts` (the open-chat links).

use chatbotx_business::inbox::links::{
    LinkInbox, build_inbox_link, build_inbox_profile_link, can_receive_ref, is_linkable_channel,
    is_profile_link_channel, list_connected_link_inboxes,
};
use chatbotx_business::referral::RefConfig;
use chatbotx_business::reflink::{self as service, ReflinkData, ReflinkListInput};
use chatbotx_business::tx;
use chatbotx_db::Db;
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::Reflink;
use serde_json::{Value, json};

use super::input::{Input, Str};
use super::qr_codes::{alnum_name, public_paging, qr_code_json, sort_query};
use super::{ApiState, Call, Reply, business_error, id, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "reflinks.list",
    "reflinks.create",
    "reflinks.get",
    "reflinks.update",
    "reflinks.delete",
    "reflinks.getChatWidget",
    "reflinks.updateChatWidget",
];

/// `createReflinkLinkBuilder`: the linkable connected inboxes, loaded once.
pub(crate) async fn linkable_inboxes(
    db: &Db,
    workspace_id: i64,
) -> Result<Vec<LinkInbox>, crate::orpc::OrpcError> {
    tx::read(db, move |conn| {
        list_connected_link_inboxes(conn, workspace_id)
    })
    .await
    .map(|inboxes| {
        inboxes
            .into_iter()
            .filter(|i| is_linkable_channel(&i.channel) || is_profile_link_channel(&i.channel))
            .collect()
    })
    .map_err(|e| business_error(e, super::LISTING))
}

/// `buildLinks(name)`.
pub(crate) fn links(app_url: &str, inboxes: &[LinkInbox], name: &str) -> Vec<Value> {
    let config = RefConfig::Reflink {
        name: name.to_string(),
    };
    inboxes
        .iter()
        .filter_map(|inbox| {
            let url = if is_profile_link_channel(&inbox.channel) {
                build_inbox_profile_link(inbox)
            } else {
                build_inbox_link(app_url, inbox, Some(&config))
            }?;
            Some(json!({
                "inboxId": id(inbox.id),
                "inboxName": inbox.name,
                "channel": inbox.channel,
                "url": url,
                "receivesRef": can_receive_ref(&inbox.channel),
            }))
        })
        .collect()
}

/// `toChatWidgetResource` (`reflinkChatWidgetPublicResource`).
fn chat_widget_json(
    state: &ApiState,
    inboxes: &[LinkInbox],
    row: &chatbotx_business::reflink::ReflinkWithLogo,
) -> Value {
    use crate::reflink_widget as widget;
    let tenant = super::contacts::tenant_urls(state);
    let brand = widget::widget_brand(row, &tenant.storage_url);
    let r = &row.reflink;
    let reflink_id = r.id.to_string();
    json!({
        "reflinkId": reflink_id,
        "authorizedDomains": widget::json_strings(&r.widget_authorized_domains.0),
        "hiddenInboxIds": widget::json_strings(&r.widget_hidden_inbox_ids.0),
        "logoFileId": r.widget_logo_file_id.map(|v| v.to_string()),
        "logoUrl": brand["logoUrl"],
        "logoBackgroundColor": brand["logoBackgroundColor"],
        "brandName": r.widget_brand_name,
        "brandUrl": r.widget_brand_url,
        "scriptUrl": widget::widget_script_url(&tenant.app_url),
        "embedCode": widget::widget_embed_code(&tenant.app_url, &reflink_id),
        "channels": widget::widget_links(&tenant.app_url, inboxes, row),
    })
}

fn is_widget_hostname(text: &str) -> bool {
    let text = text.strip_suffix('.').unwrap_or(text);
    !text.is_empty()
        && text.len() <= 253
        && text.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn is_widget_color(text: &str) -> bool {
    text.strip_prefix('#').is_some_and(|hex| {
        (hex.len() == 6 || hex.len() == 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// `updateReflinkWidgetRequest`.
fn widget_settings(input: &mut Input) -> chatbotx_business::reflink::WidgetSettings {
    use super::input::{Seg, key};
    let mut out = chatbotx_business::reflink::WidgetSettings::default();
    let item = |name: &str, index: usize| vec![Seg::Key(name.into()), Seg::Index(index)];
    match input.get("authorizedDomains").cloned() {
        Some(Value::Array(items)) => {
            if items.len() > 50 {
                input.issue(
                    &key("authorizedDomains"),
                    "too_big",
                    "Too big: expected array to have <=50 items",
                );
            }
            for (index, value) in items.iter().enumerate() {
                match value {
                    Value::String(text) if is_widget_hostname(text) => {
                        out.authorized_domains.push(text.clone())
                    }
                    Value::String(_) => input.issue(
                        &item("authorizedDomains", index),
                        "invalid_format",
                        "Invalid hostname",
                    ),
                    _ => input.issue(
                        &item("authorizedDomains", index),
                        "invalid_type",
                        "Invalid input: expected string",
                    ),
                }
            }
        }
        _ => input.issue(
            &key("authorizedDomains"),
            "invalid_type",
            "Invalid input: expected array",
        ),
    }
    match input.get("hiddenInboxIds").cloned() {
        Some(Value::Array(items)) => {
            if items.len() > 500 {
                input.issue(
                    &key("hiddenInboxIds"),
                    "too_big",
                    "Too big: expected array to have <=500 items",
                );
            }
            for (index, value) in items.iter().enumerate() {
                if let Some(id) = input.id_at(&item("hiddenInboxIds", index), Some(value), true) {
                    out.hidden_inbox_ids.push(id);
                }
            }
        }
        _ => input.issue(
            &key("hiddenInboxIds"),
            "invalid_type",
            "Invalid input: expected array",
        ),
    }
    match input.get("logoFileId").cloned() {
        Some(Value::String(text)) if text.is_empty() => {}
        other => out.logo_file_id = input.id_at(&key("logoFileId"), other.as_ref(), true),
    }
    match input.get("logoBackgroundColor").cloned() {
        Some(Value::String(text)) if is_widget_color(&text) => out.logo_background_color = text,
        Some(Value::String(_)) => input.issue(
            &key("logoBackgroundColor"),
            "invalid_format",
            "Invalid string: must match pattern",
        ),
        _ => input.issue(
            &key("logoBackgroundColor"),
            "invalid_type",
            "Invalid input: expected string",
        ),
    }
    out.brand_name = input
        .string("brandName", Str::trimmed(0, 100))
        .unwrap_or_default();
    match input.get("brandUrl").cloned() {
        Some(Value::String(text)) if text.is_empty() => {}
        Some(Value::String(text)) => {
            let ok = text.chars().count() <= 2048
                && url::Url::parse(&text)
                    .is_ok_and(|u| u.scheme() == "http" || u.scheme() == "https");
            if ok {
                out.brand_url = text;
            } else {
                input.issue(&key("brandUrl"), "invalid_union", "Invalid input");
            }
        }
        _ => input.issue(
            &key("brandUrl"),
            "invalid_type",
            "Invalid input: expected string",
        ),
    }
    if !input.failed() {
        if !out.brand_name.is_empty() && out.brand_url.is_empty() {
            input.issue(&key("brandUrl"), "custom", "Invalid input");
        }
        if !out.brand_url.is_empty() && out.brand_name.is_empty() {
            input.issue(&key("brandName"), "custom", "Invalid input");
        }
    }
    out
}

/// `reflinkPublicResource`.
fn reflink_json(app_url: &str, inboxes: &[LinkInbox], row: &Reflink) -> Value {
    let mut out = qr_code_json(row);
    out.insert("workspaceId".into(), id(row.workspace_id));
    out.insert(
        "links".into(),
        Value::Array(links(app_url, inboxes, &row.name)),
    );
    Value::Object(out)
}

/// `createReflinkRequest` fields (`partial` for the update).
fn form(input: &mut Input, partial: bool) -> ReflinkData {
    let name = alnum_name(input, !partial);
    let flow_id = if !partial || input.has("flowId") {
        input.id("flowId")
    } else {
        None
    };
    // `z.union([z.literal("").transform(() => null), zodBigintAsString()]).nullable()`.
    let custom_field_id = match input.get("customFieldId").cloned() {
        None if partial => None,
        None => {
            input.issue(
                &super::input::key("customFieldId"),
                "invalid_union",
                "Invalid input",
            );
            None
        }
        Some(Value::Null) => Some(None),
        Some(Value::String(text)) if text.is_empty() => Some(None),
        Some(value) => input
            .id_at(&super::input::key("customFieldId"), Some(&value), true)
            .map(Some),
    };
    ReflinkData {
        name,
        flow_id,
        custom_field_id,
    }
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let app = state.app_url.as_str();
    match route_id {
        "reflinks.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let keyword = input.nullish_string("keyword", Str::default()).flatten();
            let sort = sort_query(&mut input, Some(&["name", "createdAt", "updatedAt"]));
            input.finish()?;
            let sort = sort.unwrap_or_else(|| {
                vec![SortItem {
                    id: "createdAt".into(),
                    desc: true,
                }]
            });
            let page = service::list(
                db,
                ReflinkListInput {
                    workspace_id: ws,
                    keyword,
                    page,
                    per_page,
                    sort: Some(sort),
                },
            )
            .await
            .map_err(|e| business_error(e, super::LISTING))?;
            let inboxes = linkable_inboxes(db, ws).await?;
            let data: Vec<Value> = page
                .data
                .iter()
                .map(|row| reflink_json(app, &inboxes, &row.reflink))
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "reflinks.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let reflink_id = input.id("id");
            input.finish()?;
            let row = service::find_or_fail(db, ws, reflink_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            let inboxes = linkable_inboxes(db, ws).await?;
            Ok(json(200, reflink_json(app, &inboxes, &row)))
        }
        "reflinks.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = form(&mut input, false);
            input.finish()?;
            let row = service::create(db, ws, data)
                .await
                .map_err(|e| business_error(e, super::CREATING))?;
            let inboxes = linkable_inboxes(db, ws).await?;
            Ok(json(201, reflink_json(app, &inboxes, &row)))
        }
        "reflinks.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let reflink_id = input.id("id");
            let data = form(&mut input, true);
            input.finish()?;
            let row = service::update(db, ws, reflink_id.unwrap_or_default(), data)
                .await
                .map_err(|e| business_error(e, super::MUTATING))?;
            let inboxes = linkable_inboxes(db, ws).await?;
            Ok(json(200, reflink_json(app, &inboxes, &row)))
        }
        "reflinks.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let reflink_id = input.id("id");
            input.finish()?;
            service::delete_many(db, ws, vec![reflink_id.unwrap_or_default()])
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, super::MUTATING))
        }
        "reflinks.getChatWidget" => {
            let mut input = Input::from_query(call.params, call.query);
            let reflink_id = input.id("id");
            input.finish()?;
            let row = service::find_widget_or_fail(db, ws, reflink_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            let inboxes = linkable_inboxes(db, ws).await?;
            Ok(json(200, chat_widget_json(state, &inboxes, &row)))
        }
        "reflinks.updateChatWidget" => {
            let mut input = Input::from_body(call.params, call.body);
            let reflink_id = input.id("id").unwrap_or_default();
            let settings = widget_settings(&mut input);
            input.finish()?;
            service::update_widget_settings(db, ws, reflink_id, settings)
                .await
                .map_err(|e| business_error(e, super::MUTATING))?;
            let row = service::find_widget_or_fail(db, ws, reflink_id)
                .await
                .map_err(|e| business_error(e, super::MUTATING))?;
            let inboxes = linkable_inboxes(db, ws).await?;
            Ok(json(200, chat_widget_json(state, &inboxes, &row)))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
