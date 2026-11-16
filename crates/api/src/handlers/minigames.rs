//! `features/minigames/api/public.ts`: `minigames.*`.

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "minigames.list",
    "minigames.create",
    "minigames.get",
    "minigames.update",
    "minigames.updatePartial",
    "minigames.delete",
    "minigames.deleteMany",
    "minigames.setEnabled",
    "minigames.listPlays",
    "minigames.listPlayers",
];

/// Runs one of [`ROUTES`].
pub async fn handle(_state: &ApiState, _route_id: &str, _call: Call<'_>) -> Reply {
    todo!("P3.6-D1")
}
