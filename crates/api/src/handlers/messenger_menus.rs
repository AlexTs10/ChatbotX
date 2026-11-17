//! `features/personas/api/public.ts` (`messengerPersonasPublicRouter`) and
//! `features/user-persistent-menus/api/public.ts`
//! (`userPersistentMenusPublicRouter`), scope `channels` (Q.I1b slice 5),
//! plus the JSON the two `/rpc` list procedures share.

use chatbotx_business::{BusinessError, tx};
use chatbotx_db::Db;
use chatbotx_db::repo::models::UserPersistentMenu;
use chatbotx_db::repo::{Json, integration_messenger, small_repos};
use serde_json::{Value, json};

use super::input::{Input, Seg, Str, child, type_name};
use super::polls::url_at;
use super::qr_codes::public_paging;
use super::tags::{array, digits_at};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "messengerPersonas.list",
    "userPersistentMenus.list",
    "userPersistentMenus.create",
    "userPersistentMenus.get",
    "userPersistentMenus.update",
    "userPersistentMenus.delete",
];

const NOT_FOUND: &str = "User persistent menu not found";

/// `listMessengerPersonaOptions`: the registered personas of every page.
pub async fn persona_options(db: &Db, workspace_id: i64) -> Result<Value, BusinessError> {
    let pages = tx::read(db, move |conn| {
        Ok(integration_messenger::list_personas_by_workspace_id(
            conn,
            workspace_id,
        )?)
    })
    .await?;
    let data: Vec<Value> = pages
        .iter()
        .flat_map(|page| {
            page.personas
                .0
                .as_array()
                .into_iter()
                .flatten()
                // `selectRegisteredPersonas`: a `facebookPersonaId` is set.
                .filter(|p| {
                    p.get("facebookPersonaId")
                        .and_then(Value::as_str)
                        .is_some_and(|id| !id.is_empty())
                })
                .map(|p| json!({ "id": p["id"], "name": p["name"], "pageName": page.name }))
        })
        .collect();
    Ok(json!({ "data": data }))
}

/// `userPersistentMenuResource`; the public resource omits `workspaceId`.
pub fn menu_json(menu: &UserPersistentMenu, public: bool) -> Value {
    let mut out = menu.to_json();
    if public && let Value::Object(map) = &mut out {
        map.remove("workspaceId");
    }
    out
}

/// `listUserPersistentMenusByWorkspace` (newest first).
pub async fn list_menus(
    db: &Db,
    workspace_id: i64,
) -> Result<Vec<UserPersistentMenu>, BusinessError> {
    tx::read(db, move |conn| {
        Ok(small_repos::list_user_persistent_menus_by_workspace(
            conn,
            workspace_id,
        )?)
    })
    .await
}

/// `messengerPersistentMenuSchema` (a `type` discriminated union).
pub(crate) fn menu_item(input: &mut Input, path: &[Seg], value: &Value) -> Option<Value> {
    let Value::Object(item) = value else {
        input.issue(
            path,
            "invalid_type",
            &format!(
                "Invalid input: expected object, received {}",
                type_name(Some(value))
            ),
        );
        return None;
    };
    let label_spec = Str {
        min: Some(1),
        ..Str::default()
    };
    let path = path.to_vec();
    match item.get("type").and_then(Value::as_str) {
        Some("flow") => {
            let label =
                input.string_at(&child(&path, "label"), item.get("label"), label_spec, true);
            let flow = digits_at(input, &child(&path, "flowId"), item.get("flowId"), true);
            let flow_id = item.get("flowId").cloned();
            (label.is_some() && flow.is_some())
                .then(|| json!({ "label": label, "type": "flow", "flowId": flow_id }))
        }
        Some("url") => {
            let label =
                input.string_at(&child(&path, "label"), item.get("label"), label_spec, true);
            let url = url_at(input, &child(&path, "url"), item.get("url"), true);
            (label.is_some() && url.is_some())
                .then(|| json!({ "label": label, "type": "url", "url": url }))
        }
        _ => {
            input.issue(&child(&path, "type"), "invalid_union", "Invalid input");
            None
        }
    }
}

/// `userPersistentMenuRequest`: `name` and `persistentMenus` (max 20).
fn menu_request(input: &mut Input) -> (Option<String>, Option<Vec<Value>>) {
    let name = input.string("name", Str::trimmed(1, 255));
    let menus = array(input, "persistentMenus", 0, 20, menu_item);
    (name, menus)
}

fn menu_id(input: &mut Input) -> Option<i64> {
    let value = input.get("id").cloned();
    digits_at(input, &super::input::key("id"), value.as_ref(), true)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "messengerPersonas.list" => persona_options(db, ws)
            .await
            .map(|out| json(200, out))
            .map_err(|e| business_error(e, LISTING)),
        "userPersistentMenus.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let menus = list_menus(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            // `paginateInMemory`.
            let start = ((page - 1) * per_page).max(0) as usize;
            let data: Vec<Value> = menus
                .iter()
                .skip(start)
                .take(per_page as usize)
                .map(|m| menu_json(m, true))
                .collect();
            let page_count = ((menus.len() as i64 + per_page - 1) / per_page).max(1);
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "userPersistentMenus.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = menu_id(&mut input).unwrap_or_default();
            input.finish()?;
            tx::read(db, move |conn| {
                small_repos::find_user_persistent_menu_by_id(conn, ws, id)?
                    .ok_or_else(|| BusinessError::not_found(NOT_FOUND))
            })
            .await
            .map(|menu| json(200, menu_json(&menu, true)))
            .map_err(|e| business_error(e, FINDING))
        }
        "userPersistentMenus.create" | "userPersistentMenus.update" => {
            let create = route_id == "userPersistentMenus.create";
            let mut input = Input::from_body(call.params, call.body);
            let (name, menus) = menu_request(&mut input);
            let id = if create { None } else { menu_id(&mut input) };
            input.finish()?;
            let (name, menus) = (
                name.unwrap_or_default(),
                Json(Value::Array(menus.unwrap_or_default())),
            );
            let out = tx::write(db, move |conn| {
                if create {
                    small_repos::create_user_persistent_menu(conn, ws, &name, &menus)?.ok_or_else(
                        || {
                            BusinessError::exception(
                                "Failed to create user persistent menu",
                                "businessError",
                                400,
                            )
                        },
                    )
                } else {
                    small_repos::update_user_persistent_menu(
                        conn,
                        ws,
                        id.unwrap_or_default(),
                        &name,
                        &menus,
                    )?
                    .ok_or_else(|| BusinessError::not_found(NOT_FOUND))
                }
            })
            .await
            .map_err(|e| business_error(e, if create { CREATING } else { MUTATING }))?;
            Ok(json(if create { 201 } else { 200 }, menu_json(&out, true)))
        }
        "userPersistentMenus.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = menu_id(&mut input).unwrap_or_default();
            input.finish()?;
            tx::write(db, move |conn| {
                let deleted = small_repos::delete_user_persistent_menus(conn, ws, &[id])?;
                // A single-id delete 404s when nothing was deleted.
                if deleted.is_empty() {
                    return Err(BusinessError::not_found(NOT_FOUND));
                }
                Ok(())
            })
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
