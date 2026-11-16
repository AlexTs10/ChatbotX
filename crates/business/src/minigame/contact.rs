//! The admin reads of `minigame/minigame-contact-service.ts`
//! (`minigameContactService.list` / `listPlays`) and `deriveRemaining`.

use chatbotx_db::Db;
use chatbotx_db::repo::SortItem;
use serde_json::Value;

use crate::errors::Result;

/// `MAX_PLAY_RECORDS`.
pub const MAX_PLAY_RECORDS: i64 = 200;

/// `deriveRemaining`: the allowance plus the earned share bonus (capped at
/// `maxSharesPerPerson`, missing = 0) minus the draws played, never below 0.
pub fn derive_remaining(_player_settings: &Value, _played: i64, _shares_count: i64) -> i64 {
    todo!("P3.6-D1")
}

/// `list` input (players of one minigame).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListPlayers {
    pub workspace_id: i64,
    pub minigame_id: i64,
    pub page: Option<i64>,
    /// Default 10.
    pub per_page: Option<i64>,
    pub name: Option<String>,
    pub sort: Option<Vec<SortItem>>,
}

/// The contact columns of a player.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerContact {
    pub id: i64,
    pub full_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar: Option<String>,
}

/// One player (`MinigameContact` + contact).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Player {
    pub id: i64,
    pub contact_id: i64,
    pub contact_inbox_id: Option<i64>,
    pub played: i64,
    pub remaining: i64,
    pub shares_count: i64,
    /// Unix ms.
    pub opened_at: i64,
    /// `updatedAt` (unix ms).
    pub last_played_at: i64,
    pub contact: PlayerContact,
}

/// `list` result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerPage {
    pub data: Vec<Player>,
    pub page_count: i64,
}

/// One play of `listPlays`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Play {
    pub id: i64,
    pub is_winning: bool,
    pub prize_name: Option<String>,
    /// Unix ms.
    pub created_at: i64,
}

/// `list`: the minigame must be in the workspace (`Minigame not found`).
pub async fn list(_db: &Db, _input: ListPlayers) -> Result<PlayerPage> {
    todo!("P3.6-D1")
}

/// `listPlays`: one contact's draws, newest first, at most
/// [`MAX_PLAY_RECORDS`].
pub async fn list_plays(
    _db: &Db,
    _workspace_id: i64,
    _minigame_id: i64,
    _contact_id: i64,
) -> Result<Vec<Play>> {
    todo!("P3.6-D1")
}
