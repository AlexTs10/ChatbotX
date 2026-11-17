//! `user-persistent-menu/service.ts`: the saved Messenger persistent menus
//! of a workspace (settings `user-persistent-menus`, Q.F2 web run 7).

use chatbotx_db::Db;
use chatbotx_db::repo::Json;
use chatbotx_db::repo::models::UserPersistentMenu;
use chatbotx_db::repo::small_repos as repo;
use serde_json::Value;

use crate::{BusinessError, Result, tx};

fn not_found() -> BusinessError {
    BusinessError::not_found("User persistent menu not found")
}

/// `listByWorkspace`: newest first.
pub async fn list_by_workspace(db: &Db, workspace_id: i64) -> Result<Vec<UserPersistentMenu>> {
    tx::read(db, move |c| {
        Ok(repo::list_user_persistent_menus_by_workspace(
            c,
            workspace_id,
        )?)
    })
    .await
}

/// `find`: `None` outside the workspace.
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<Option<UserPersistentMenu>> {
    tx::read(db, move |c| {
        Ok(repo::find_user_persistent_menu_by_id(c, workspace_id, id)?)
    })
    .await
}

/// `create`; `menus` = the parsed `messengerPersistentMenuSchema` items.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    name: String,
    menus: Vec<Value>,
) -> Result<UserPersistentMenu> {
    tx::write(db, move |c| {
        repo::create_user_persistent_menu(c, workspace_id, &name, &Json(Value::Array(menus)))?
            .ok_or_else(|| {
                BusinessError::exception(
                    "Failed to create user persistent menu",
                    "systemError",
                    400,
                )
            })
    })
    .await
}

/// `update`: 404 outside the workspace.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: String,
    menus: Vec<Value>,
) -> Result<UserPersistentMenu> {
    tx::write(db, move |c| {
        repo::update_user_persistent_menu(c, workspace_id, id, &name, &Json(Value::Array(menus)))?
            .ok_or_else(not_found)
    })
    .await
}

/// `delete`: a one-id delete that removed nothing is a 404; a bulk delete
/// ignores ids already gone.
pub async fn delete(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    tx::write(db, move |c| {
        let deleted = repo::delete_user_persistent_menus(c, workspace_id, &ids)?;
        if ids.len() == 1 && deleted.is_empty() {
            return Err(not_found());
        }
        Ok(())
    })
    .await
}
