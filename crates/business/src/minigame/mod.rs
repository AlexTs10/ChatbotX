//! `minigame/service.ts` (`minigameService`) and the admin reads of
//! `minigame-contact-service.ts` ([`contact`]). The settings groups are the
//! JSON the HTTP layer parsed (`database/partials/minigame.ts`); the service
//! stores them as given. The play runtime (`resolvePlayState`, `recordPlay`,
//! referral credits, outcome messages, leaderboard awards) is not here.

pub mod contact;

use std::collections::HashMap;

use chatbotx_db::Db;
use chatbotx_db::repo::models::Minigame;
use chatbotx_db::repo::utils::{order_by_sql, parse_order_by};
use chatbotx_db::repo::{
    Model, PaginationInput, SortItem, get_pagination_with_defaults, is_unique_violation_error,
    like_contains, new_id, now_ms,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::tx;

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Minigame not found")
}

/// `rethrowNameConflict`: the only unique index is the workspace name.
fn name_conflict(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    let error = chatbotx_db::repo::Error::from(error);
    if is_unique_violation_error(&error, None) {
        BusinessError::exception("Minigame name already exists", "nameAlreadyExists", 409)
    } else {
        BusinessError::Repo(error)
    }
}

pub(crate) fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Minigame>> {
    conn.query_row(
        &format!(
            r#"SELECT {} FROM "Minigame" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            Minigame::columns_sql(None)
        ),
        params![id, workspace_id],
        Minigame::from_row,
    )
    .optional()
    .map_err(db_error)
}

/// `toColumns`: the row columns of the groups (`name` = `generalSettings.name`).
fn write_columns(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    data: &MinigameData,
) -> Result<Minigame> {
    let name = data
        .general_settings
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    conn.execute(
        r#"UPDATE "Minigame"
           SET "name" = ?1, "type" = ?2, "generalSettings" = ?3, "appearance" = ?4, "playerSettings" = ?5,
               "prizeSettings" = ?6, "winningMessageSettings" = ?7, "nonWinningMessageSettings" = ?8, "updatedAt" = ?9
           WHERE "id" = ?10 AND "workspaceId" = ?11"#,
        params![
            name,
            data.r#type,
            data.general_settings.to_string(),
            data.appearance.to_string(),
            data.player_settings.to_string(),
            data.prize_settings.to_string(),
            data.winning_message_settings.to_string(),
            data.non_winning_message_settings.to_string(),
            now_ms(),
            id,
            workspace_id
        ],
    )
    .map_err(name_conflict)?;
    find_in(conn, workspace_id, id)?.ok_or_else(not_found)
}

/// `reconcilePrizeQuantities`: a prize whose submitted quantity still equals
/// the form's baseline keeps the stored (live) quantity.
fn reconcile_prize_quantities(
    submitted: &Value,
    current: &Value,
    original: &HashMap<String, Option<f64>>,
) -> Value {
    let live: HashMap<String, Option<Value>> = current
        .get("prizes")
        .and_then(Value::as_array)
        .map(|prizes| {
            prizes
                .iter()
                .filter_map(|p| {
                    let id = p.get("id")?.as_str()?.to_string();
                    Some((id, p.get("quantity").cloned()))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut out = submitted.clone();
    if let Some(prizes) = out.get_mut("prizes").and_then(Value::as_array_mut) {
        for prize in prizes.iter_mut() {
            let Some(id) = prize.get("id").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            // `liveQuantity !== undefined`: the stored prize has the key.
            let Some(Some(live_quantity)) = live.get(&id).cloned() else {
                continue;
            };
            let submitted_quantity = prize.get("quantity").and_then(Value::as_f64);
            let baseline = original.get(&id).copied().flatten();
            if submitted_quantity == baseline
                && let Some(obj) = prize.as_object_mut()
            {
                obj.insert("quantity".into(), live_quantity);
            }
        }
    }
    out
}

/// `UpsertInput` without the workspace id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MinigameData {
    pub r#type: String,
    /// `{ name, ... }`; the row's `name` follows `generalSettings.name`.
    pub general_settings: Value,
    pub appearance: Value,
    pub player_settings: Value,
    /// `{ prizes: [{ id, quantity, ... }], nonWinning }`.
    pub prize_settings: Value,
    pub winning_message_settings: Value,
    pub non_winning_message_settings: Value,
}

/// `PartialUpdateInput`: `None` keeps the stored group.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MinigamePatch {
    pub r#type: Option<String>,
    pub general_settings: Option<Value>,
    pub appearance: Option<Value>,
    pub player_settings: Option<Value>,
    pub prize_settings: Option<Value>,
    pub winning_message_settings: Option<Value>,
    pub non_winning_message_settings: Option<Value>,
}

/// `list` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListMinigames {
    pub workspace_id: i64,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub name: Option<String>,
    pub sort: Option<Vec<SortItem>>,
}

/// `list` result.
#[derive(Debug, Clone, PartialEq)]
pub struct MinigamePage {
    pub data: Vec<Minigame>,
    pub page_count: i64,
}

/// `list` (`parseOrderBy` over the row columns; default newest first).
pub async fn list(db: &Db, input: ListMinigames) -> Result<MinigamePage> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        let name = input
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(like_contains);
        let where_sql = r#""workspaceId" = ?1 AND (?2 IS NULL OR ilike("name", ?2))"#;
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT COUNT(*) FROM "Minigame" WHERE {where_sql}"#),
                params![input.workspace_id, name],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let order = parse_order_by(Minigame::COLUMNS, input.sort.as_deref(), None);
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT {} FROM "Minigame" WHERE {where_sql} {} LIMIT ?3 OFFSET ?4"#,
                Minigame::columns_sql(None),
                order_by_sql(&order, None)
            ))
            .map_err(db_error)?;
        let data = stmt
            .query_map(
                params![
                    input.workspace_id,
                    name,
                    pagination.limit,
                    pagination.offset
                ],
                Minigame::from_row,
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let per_page = input.per_page.unwrap_or(pagination.limit);
        let page_count = if per_page > 0 {
            (total + per_page - 1) / per_page
        } else {
            0
        };
        Ok(MinigamePage { data, page_count })
    })
    .await
}

/// `find` (`Minigame not found`).
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<Minigame> {
    tx::read(db, move |conn| {
        find_in(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `create` (`409 nameAlreadyExists` for a taken name).
pub async fn create(db: &Db, workspace_id: i64, data: MinigameData) -> Result<Minigame> {
    tx::write(db, move |conn| {
        let id = new_id();
        conn.execute(
            r#"INSERT INTO "Minigame"
                 ("id", "workspaceId", "name", "type", "generalSettings", "appearance", "playerSettings",
                  "prizeSettings", "winningMessageSettings", "nonWinningMessageSettings")
               VALUES (?1, ?2, '', ?3, '{}', '{}', '{}', '{}', '{}', '{}')"#,
            params![id, workspace_id, data.r#type],
        )
        .map_err(name_conflict)?;
        write_columns(conn, workspace_id, id, &data)
    })
    .await
}

/// `update`: replaces every group. `original_prize_quantities` = the
/// quantities the form loaded (`None` = the public API: submitted quantities
/// are kept as sent); a prize whose submitted quantity still equals its
/// baseline keeps the stored (live) quantity.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    data: MinigameData,
    original_prize_quantities: Option<HashMap<String, Option<f64>>>,
) -> Result<Minigame> {
    tx::write(db, move |conn| {
        let current = find_in(conn, workspace_id, id)?.ok_or_else(not_found)?;
        let mut data = data;
        if let Some(original) = &original_prize_quantities {
            data.prize_settings = reconcile_prize_quantities(
                &data.prize_settings,
                &current.prize_settings.0,
                original,
            );
        }
        write_columns(conn, workspace_id, id, &data)
    })
    .await
}

/// `updatePartial`: the sent groups over the stored row.
pub async fn update_partial(
    db: &Db,
    workspace_id: i64,
    id: i64,
    patch: MinigamePatch,
) -> Result<Minigame> {
    tx::write(db, move |conn| {
        let current = find_in(conn, workspace_id, id)?.ok_or_else(not_found)?;
        let merged = MinigameData {
            r#type: patch.r#type.unwrap_or(current.r#type),
            general_settings: patch.general_settings.unwrap_or(current.general_settings.0),
            appearance: patch.appearance.unwrap_or(current.appearance.0),
            player_settings: patch.player_settings.unwrap_or(current.player_settings.0),
            prize_settings: patch.prize_settings.unwrap_or(current.prize_settings.0),
            winning_message_settings: patch
                .winning_message_settings
                .unwrap_or(current.winning_message_settings.0),
            non_winning_message_settings: patch
                .non_winning_message_settings
                .unwrap_or(current.non_winning_message_settings.0),
        };
        write_columns(conn, workspace_id, id, &merged)
    })
    .await
}

/// `setEnabled`.
pub async fn set_enabled(db: &Db, workspace_id: i64, id: i64, enabled: bool) -> Result<Minigame> {
    tx::write(db, move |conn| {
        let changed = conn
            .execute(
                r#"UPDATE "Minigame" SET "enabled" = ?1, "updatedAt" = ?2 WHERE "id" = ?3 AND "workspaceId" = ?4"#,
                params![enabled, now_ms(), id, workspace_id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err(not_found());
        }
        find_in(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `delete` (`Minigame not found` first).
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    tx::write(db, move |conn| {
        find_in(conn, workspace_id, id)?.ok_or_else(not_found)?;
        conn.execute(
            r#"DELETE FROM "Minigame" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await
}

/// `deleteMany` (unknown ids are ignored).
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    tx::write(db, move |conn| {
        let marks = (0..ids.len())
            .map(|i| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let mut args: Vec<Sql> = vec![Sql::Integer(workspace_id)];
        args.extend(ids.iter().map(|id| Sql::Integer(*id)));
        conn.execute(
            &format!(r#"DELETE FROM "Minigame" WHERE "workspaceId" = ?1 AND "id" IN ({marks})"#),
            params_from_iter(args.iter()),
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await
}
