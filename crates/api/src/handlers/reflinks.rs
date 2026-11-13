//! `features/reflinks/api/public.ts`: `reflinks.*` (the chat-widget routes
//! are not ported yet) and `lib/reflink-links.ts` (the open-chat links).

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
];

/// `createReflinkLinkBuilder`: the linkable connected inboxes, loaded once.
async fn linkable_inboxes(
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
fn links(app_url: &str, inboxes: &[LinkInbox], name: &str) -> Vec<Value> {
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
        _ => Err(crate::routes::not_ported_error()),
    }
}
