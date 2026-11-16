//! `minigame/service.ts` (`minigameService`) and the admin reads of
//! `minigame-contact-service.ts` ([`contact`]). The settings groups are the
//! JSON the HTTP layer parsed (`database/partials/minigame.ts`); the service
//! stores them as given. The play runtime (`resolvePlayState`, `recordPlay`,
//! referral credits, outcome messages, leaderboard awards) is not here.

pub mod contact;

use std::collections::HashMap;

use chatbotx_db::Db;
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::Minigame;
use serde_json::Value;

use crate::errors::Result;

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
pub async fn list(_db: &Db, _input: ListMinigames) -> Result<MinigamePage> {
    todo!("P3.6-D1")
}

/// `find` (`Minigame not found`).
pub async fn find(_db: &Db, _workspace_id: i64, _id: i64) -> Result<Minigame> {
    todo!("P3.6-D1")
}

/// `create` (`409 nameAlreadyExists` for a taken name).
pub async fn create(_db: &Db, _workspace_id: i64, _data: MinigameData) -> Result<Minigame> {
    todo!("P3.6-D1")
}

/// `update`: replaces every group. `original_prize_quantities` = the
/// quantities the form loaded (`None` = the public API: submitted quantities
/// are kept as sent); a prize whose submitted quantity still equals its
/// baseline keeps the stored (live) quantity.
pub async fn update(
    _db: &Db,
    _workspace_id: i64,
    _id: i64,
    _data: MinigameData,
    _original_prize_quantities: Option<HashMap<String, Option<f64>>>,
) -> Result<Minigame> {
    todo!("P3.6-D1")
}

/// `updatePartial`: the sent groups over the stored row.
pub async fn update_partial(
    _db: &Db,
    _workspace_id: i64,
    _id: i64,
    _patch: MinigamePatch,
) -> Result<Minigame> {
    todo!("P3.6-D1")
}

/// `setEnabled`.
pub async fn set_enabled(
    _db: &Db,
    _workspace_id: i64,
    _id: i64,
    _enabled: bool,
) -> Result<Minigame> {
    todo!("P3.6-D1")
}

/// `delete` (`Minigame not found` first).
pub async fn delete(_db: &Db, _workspace_id: i64, _id: i64) -> Result<()> {
    todo!("P3.6-D1")
}

/// `deleteMany` (unknown ids are ignored).
pub async fn delete_many(_db: &Db, _workspace_id: i64, _ids: Vec<i64>) -> Result<()> {
    todo!("P3.6-D1")
}
