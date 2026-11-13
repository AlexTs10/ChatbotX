//! `features/integration-webchat/api/public.ts`: `webchats.*`, and the
//! `webchatConnectConfigSchema` / `createWebchatPublicRequest` /
//! `updateWebchatPublicRequest` checks.

use chatbotx_business::integration_webchat::{self, WebchatData, WebchatPatch, WebchatRow};
use serde_json::{Value, json};

use super::input::{Input, Num, Seg, Str, child, key};
use super::{ApiState, Call, Reply, business_error, date, id, json, no_content, opt_id};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "webchats.list",
    "webchats.get",
    "webchats.create",
    "webchats.update",
    "webchats.delete",
];

/// `webchatPublicResource` (no `workspaceId`, no `auth`).
pub fn webchat_json(row: &WebchatRow) -> Value {
    json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "name": row.name,
        "enable": row.enable,
        "authorizedDomains": row.authorized_domains,
        "conversationStarters": row.conversation_starters,
        "persistentMenus": row.persistent_menus,
        "brandColor": row.brand_color,
        "hideHeader": row.hide_header,
        "showLogo": row.show_logo,
        "hideMessageInput": row.hide_message_input,
        "customCss": row.custom_css,
        "inboxId": id(row.inbox_id),
        "welcomeFlowId": opt_id(row.welcome_flow_id),
    })
}

fn is_hostname(text: &str) -> bool {
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

fn is_brand_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// `z.array(z.hostname())`.
fn domains(input: &mut Input, value: &Value) -> Option<Vec<String>> {
    let path = key("authorizedDomains");
    let Value::Array(items) = value else {
        input.issue(&path, "invalid_type", "Invalid input: expected array");
        return None;
    };
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let mut item_path = path.clone();
        item_path.push(Seg::Index(index));
        match item {
            Value::String(text) if is_hostname(text) => out.push(text.clone()),
            Value::String(_) => input.issue(&item_path, "invalid_format", "Invalid hostname"),
            _ => input.issue(&item_path, "invalid_type", "Invalid input: expected string"),
        }
    }
    Some(out)
}

/// `webchatConversationStarter` / `webchatPersistentMenu` arrays.
fn items(input: &mut Input, name: &str, value: &Value, types: &[&str]) -> Option<Vec<Value>> {
    let path = key(name);
    let Value::Array(items) = value else {
        input.issue(&path, "invalid_type", "Invalid input: expected array");
        return None;
    };
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let mut item_path = path.clone();
        item_path.push(Seg::Index(index));
        let Value::Object(map) = item else {
            input.issue(&item_path, "invalid_type", "Invalid input: expected object");
            continue;
        };
        let kind = input.enum_at(&child(&item_path, "type"), map.get("type"), types, true);
        let label = input.string_at(
            &child(&item_path, "label"),
            map.get("label"),
            Str {
                min: Some(1),
                ..Str::default()
            },
            true,
        );
        let mut clean = serde_json::Map::new();
        if let Some(label) = label {
            clean.insert("label".into(), Value::String(label));
        }
        match kind.as_deref() {
            Some("flow") => {
                if let Some(flow_id) =
                    input.id_at(&child(&item_path, "flowId"), map.get("flowId"), true)
                {
                    clean.insert("flowId".into(), Value::String(flow_id.to_string()));
                }
            }
            Some("url") => {
                let url = input.string_at(
                    &child(&item_path, "url"),
                    map.get("url"),
                    Str::default(),
                    true,
                );
                if let Some(url) = url {
                    if url::Url::parse(&url).is_err() {
                        input.issue(&child(&item_path, "url"), "invalid_format", "Invalid URL");
                    } else {
                        clean.insert("url".into(), Value::String(url));
                    }
                }
            }
            _ => {}
        }
        if let Some(kind) = kind {
            clean.insert("type".into(), Value::String(kind));
        }
        out.push(Value::Object(clean));
    }
    Some(out)
}

const STARTER_TYPES: &[&str] = &["flow", "message", "url"];
const MENU_TYPES: &[&str] = &["flow", "url"];

/// `webchatConnectConfigSchema` (`public` = `createWebchatPublicRequest`:
/// `brandColor`, `conversationStarters`, `persistentMenus` required).
/// `None` = an issue was recorded.
pub fn parse_webchat_create(input: &mut Input, public: bool) -> Option<WebchatData> {
    let patch = parse_fields(input, public, true);
    if input.failed() {
        return None;
    }
    Some(WebchatData {
        name: patch.name.unwrap_or_default(),
        welcome_flow_id: patch.welcome_flow_id.flatten(),
        authorized_domains: patch.authorized_domains.unwrap_or_default(),
        conversation_starters: patch.conversation_starters.unwrap_or_default(),
        persistent_menus: patch.persistent_menus.unwrap_or_default(),
        brand_color: patch.brand_color.unwrap_or_else(|| "#007bff".to_string()),
        hide_header: patch.hide_header.unwrap_or(false),
        show_logo: patch.show_logo.unwrap_or(true),
        hide_message_input: patch.hide_message_input.unwrap_or(false),
        custom_css: patch.custom_css.flatten(),
        enable: patch.enable.unwrap_or(true),
    })
}

/// The fields of the webchat schemas; `create` = `name` required and the
/// public required keys enforced when `public`.
fn parse_fields(input: &mut Input, public: bool, create: bool) -> WebchatPatch {
    let mut patch = WebchatPatch::default();
    let name_spec = Str {
        min: Some(1),
        ..Str::max(40)
    };
    patch.name = if create {
        input.string("name", name_spec)
    } else {
        input.opt_string("name", name_spec)
    };
    patch.welcome_flow_id = input.nullish_id("welcomeFlowId");
    let required = public && create;
    if let Some(value) = input.obj.get("authorizedDomains").cloned() {
        patch.authorized_domains = domains(input, &value);
    }
    match input.obj.get("conversationStarters").cloned() {
        Some(value) => {
            patch.conversation_starters =
                items(input, "conversationStarters", &value, STARTER_TYPES);
        }
        None if required => input.issue(
            &key("conversationStarters"),
            "invalid_type",
            "Invalid input: expected array, received undefined",
        ),
        None => {}
    }
    match input.obj.get("persistentMenus").cloned() {
        Some(value) => {
            patch.persistent_menus = items(input, "persistentMenus", &value, MENU_TYPES);
        }
        None if required => input.issue(
            &key("persistentMenus"),
            "invalid_type",
            "Invalid input: expected array, received undefined",
        ),
        None => {}
    }
    let color_spec = Str {
        pattern: Some(is_brand_color),
        ..Str::default()
    };
    patch.brand_color = if required {
        input.string("brandColor", color_spec)
    } else {
        input.opt_string("brandColor", color_spec)
    };
    patch.hide_header = input.opt_bool("hideHeader");
    patch.show_logo = input.opt_bool("showLogo");
    patch.hide_message_input = input.opt_bool("hideMessageInput");
    patch.enable = input.opt_bool("enable");
    if input.has("customCss") {
        patch.custom_css = input.opt_string("customCss", Str::max(20_000)).map(Some);
    }
    patch
}

fn path_id(input: &mut Input) -> Option<i64> {
    input.id("id")
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "webchats.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let page = input.opt_coerce_int(
                "page",
                Num {
                    min: Some(1.0),
                    ..Num::default()
                },
            );
            let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 50.0));
            input.finish()?;
            integration_webchat::list(db, ws, page.unwrap_or(1), per_page.unwrap_or(50))
                .await
                .map(|(rows, page_count)| {
                    json(
                        200,
                        json!({
                            "data": rows.iter().map(webchat_json).collect::<Vec<_>>(),
                            "pageCount": page_count,
                        }),
                    )
                })
                .map_err(|e| business_error(e, super::LISTING))
        }
        "webchats.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let webchat_id = path_id(&mut input);
            input.finish()?;
            integration_webchat::find_by_id_for_workspace(db, webchat_id.unwrap_or_default(), ws)
                .await
                .map(|row| json(200, webchat_json(&row)))
                .map_err(|e| business_error(e, super::FINDING))
        }
        "webchats.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_webchat_create(&mut input, true);
            input.finish()?;
            let Some(data) = data else {
                return Err(super::internal_error("webchat parse"));
            };
            let now = chatbotx_db::repo::now_ms();
            integration_webchat::create(db, ws, None, data, &state.app_url, now)
                .await
                .map(|(row, _)| json(201, webchat_json(&row)))
                .map_err(|e| business_error(e, super::CREATING))
        }
        "webchats.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let webchat_id = path_id(&mut input);
            let patch = parse_fields(&mut input, true, false);
            input.finish()?;
            let webchat_id = webchat_id.unwrap_or_default();
            async {
                integration_webchat::find_by_id_for_workspace(db, webchat_id, ws).await?;
                integration_webchat::update(db, ws, webchat_id, patch, &state.app_url).await?;
                integration_webchat::find_by_id_for_workspace(db, webchat_id, ws).await
            }
            .await
            .map(|row| json(200, webchat_json(&row)))
            .map_err(|e| business_error(e, super::MUTATING))
        }
        "webchats.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let webchat_id = path_id(&mut input);
            input.finish()?;
            integration_webchat::delete(db, ws, webchat_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, super::MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
