//! The admin reads of `minigame/minigame-contact-service.ts`
//! (`minigameContactService.list` / `listPlays`) and `deriveRemaining`.

use chatbotx_db::Db;
use chatbotx_db::repo::{PaginationInput, SortItem, get_pagination_with_defaults, like_contains};
use chatbotx_db::rusqlite::params;
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::tx;

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn require_minigame(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    id: i64,
) -> Result<()> {
    super::find_in(conn, workspace_id, id)?
        .map(|_| ())
        .ok_or_else(|| BusinessError::not_found("Minigame not found"))
}

/// `getMinigameContactListOrder` (Postgres `NULLS` order; `id` tie-break).
pub fn list_order_sql(sort: Option<&[SortItem]>) -> String {
    let tie = r#"mc."id" ASC"#;
    let Some(item) = sort.and_then(|s| s.first()) else {
        return format!(r#"mc."updatedAt" DESC, {tie}"#);
    };
    let column = match item.id.as_str() {
        "name" => r#"c."fullName""#,
        "played" => r#"mc."played""#,
        "remaining" => r#"mc."remaining""#,
        "sharesCount" => r#"mc."sharesCount""#,
        "openedAt" => r#"mc."openedAt""#,
        "lastPlayedAt" => r#"mc."updatedAt""#,
        _ => return format!(r#"mc."updatedAt" DESC, {tie}"#),
    };
    if item.desc {
        format!("{column} DESC NULLS FIRST, {tie}")
    } else {
        format!("{column} ASC NULLS LAST, {tie}")
    }
}

/// `MAX_PLAY_RECORDS`.
pub const MAX_PLAY_RECORDS: i64 = 200;

/// `deriveRemaining`: the allowance plus the earned share bonus (capped at
/// `maxSharesPerPerson`, missing = 0) minus the draws played, never below 0.
pub fn derive_remaining(player_settings: &Value, played: i64, shares_count: i64) -> i64 {
    let number = |key: &str| {
        player_settings
            .get(key)
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    let bonus = (shares_count as f64).min(number("maxSharesPerPerson"));
    (number("drawsPerPerson") + bonus - played as f64).max(0.0) as i64
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
pub async fn list(db: &Db, input: ListPlayers) -> Result<PlayerPage> {
    tx::read(db, move |conn| {
        require_minigame(conn, input.workspace_id, input.minigame_id)?;
        let per_page = input.per_page.unwrap_or(10);
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: Some(per_page),
        });
        let name = input.name.as_deref().filter(|n| !n.is_empty()).map(like_contains);
        let from = r#"FROM "MinigameContact" mc JOIN "Contact" c ON c."id" = mc."contactId"
            WHERE mc."minigameId" = ?1 AND (?2 IS NULL OR ilike(c."fullName", ?2))"#;
        let total: i64 = conn
            .query_row(&format!("SELECT COUNT(*) {from}"), params![input.minigame_id, name], |r| r.get(0))
            .map_err(db_error)?;
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT mc."id", mc."contactId", mc."contactInboxId", mc."played", mc."remaining", mc."sharesCount",
                          mc."openedAt", mc."updatedAt", c."id", c."fullName", c."firstName", c."lastName", c."avatar"
                   {from} ORDER BY {} LIMIT ?3 OFFSET ?4"#,
                list_order_sql(input.sort.as_deref())
            ))
            .map_err(db_error)?;
        let data = stmt
            .query_map(params![input.minigame_id, name, pagination.limit, pagination.offset], |r| {
                Ok(Player {
                    id: r.get(0)?,
                    contact_id: r.get(1)?,
                    contact_inbox_id: r.get(2)?,
                    played: r.get(3)?,
                    remaining: r.get(4)?,
                    shares_count: r.get(5)?,
                    opened_at: r.get(6)?,
                    last_played_at: r.get(7)?,
                    contact: PlayerContact {
                        id: r.get(8)?,
                        full_name: r.get(9)?,
                        first_name: r.get(10)?,
                        last_name: r.get(11)?,
                        avatar: r.get(12)?,
                    },
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let page_count = if per_page > 0 { (total + per_page - 1) / per_page } else { 0 };
        Ok(PlayerPage { data, page_count })
    })
    .await
}

/// `listPlays`: one contact's draws, newest first, at most
/// [`MAX_PLAY_RECORDS`].
pub async fn list_plays(
    db: &Db,
    workspace_id: i64,
    minigame_id: i64,
    contact_id: i64,
) -> Result<Vec<Play>> {
    tx::read(db, move |conn| {
        require_minigame(conn, workspace_id, minigame_id)?;
        let mut stmt = conn
            .prepare(
                r#"SELECT "id", "isWinning", "prizeName", "createdAt" FROM "MinigamePlay"
                   WHERE "minigameId" = ?1 AND "contactId" = ?2 ORDER BY "createdAt" DESC LIMIT ?3"#,
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map(params![minigame_id, contact_id, MAX_PLAY_RECORDS], |r| {
                Ok(Play {
                    id: r.get(0)?,
                    is_winning: r.get(1)?,
                    prize_name: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(rows)
    })
    .await
}
