//! Settings `user-persistent-menus` (Q.F2 web run 7): the builder queries
//! `listUserPersistentMenus` / `findUserPersistentMenu` and the three
//! `workspaceActionClient` actions over `business::user_persistent_menu`,
//! and the flow editor's list procedure.
//! The `/rpc` membership check ran (`assertCurrentUserCanAccessChatbot`).

use chatbotx_business::user_persistent_menu as menus;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail};
use crate::handlers::input::{Input, Str, key};
use crate::handlers::webchats::{MENU_TYPES, items};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "listUserPersistentMenus",
    "findUserPersistentMenu",
    "createUserPersistentMenuAction",
    "updateUserPersistentMenuAction",
    "deleteUserPersistentMenuAction",
];

/// `userPersistentMenusAPI.listUserPersistentMenusAuthenticatedAPI` (the
/// flow editor's "set persistent menu" step options): the same list.
pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str) -> Outcome {
    if path != "userPersistentMenusAPI.listUserPersistentMenusAuthenticatedAPI" {
        return None;
    }
    Some(run(ctx, "listUserPersistentMenus", &[]).await)
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

/// `userPersistentMenuRequest`: `name` (trimmed, 1..=255) and at most 20
/// `messengerPersistentMenuSchema` items.
fn request(body: &Value) -> Result<(String, Vec<Value>), OrpcError> {
    let body = body.to_string();
    let mut input = Input::from_body(&[], body.as_bytes());
    let name = input.string("name", Str::trimmed(1, 255));
    let list = match input.get("persistentMenus").cloned() {
        Some(value) => {
            if value.as_array().is_some_and(|a| a.len() > 20) {
                input.issue(
                    &key("persistentMenus"),
                    "too_big",
                    "Too big: expected array to have <=20 items",
                );
            }
            items(&mut input, "persistentMenus", &value, MENU_TYPES)
        }
        None => {
            input.issue(
                &key("persistentMenus"),
                "invalid_type",
                "Invalid input: expected array, received undefined",
            );
            None
        }
    };
    input.finish().map_err(super::schema_errors)?;
    Ok((name.unwrap_or_default(), list.unwrap_or_default()))
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    match name {
        "listUserPersistentMenus" => {
            let rows = menus::list_by_workspace(db, ws).await.map_err(fail)?;
            Ok(json!({ "data": rows.iter().map(|r| r.to_json()).collect::<Vec<_>>() }))
        }
        "findUserPersistentMenu" => {
            let id = In(arg(args, 0)).id("id")?;
            let row = menus::find(db, ws, id).await.map_err(fail)?;
            Ok(row.map_or(Value::Null, |r| r.to_json()))
        }
        "createUserPersistentMenuAction" => {
            let (name, list) = request(arg(args, 1))?;
            menus::create(db, ws, name, list)
                .await
                .map_err(action_fail)?;
            Ok(Value::Null)
        }
        "updateUserPersistentMenuAction" => {
            let id = In(&json!({ "id": arg(args, 1) })).id("id")?;
            let (name, list) = request(arg(args, 2))?;
            menus::update(db, ws, id, name, list)
                .await
                .map_err(action_fail)?;
            Ok(Value::Null)
        }
        _ => {
            let ids = In(arg(args, 1)).ids("ids")?;
            menus::delete(db, ws, ids).await.map_err(action_fail)?;
            Ok(Value::Null)
        }
    }
}
