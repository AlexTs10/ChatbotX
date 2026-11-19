//! Q.D13b: the public leaderboard and minigame pages (no session; the
//! signed token is the authority).
//!
//! - `app/leaderboards/page.tsx` + `app/api/leaderboards/[id]/{board,me}`:
//!   `POST /rpc/pages/leaderboardPublic` (`{ leaderboardId, token? }`) and the
//!   polling routes `GET /api/leaderboards/{id}/board` (ETag, 304) and
//!   `GET /api/leaderboards/{id}/me?token=`.
//! - `app/minigames/page.tsx` + `playMinigameAction`:
//!   `POST /rpc/pages/minigamePlay` (`{ minigameId, token }`) and
//!   `POST /rpc/actions/playMinigameAction` (guest rate limit as the original).
//!
//! - `app/bs/[workspaceId]/[webchatId]/page.tsx` (bot simulator preview):
//!   `POST /rpc/pages/botSimulatorPreview` (`{ workspaceId, webchatId, url }`).
//!
//! Answers are JSON for the SPA (React escapes the text).

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chatbotx_business::inbox::links::{
    build_inbox_link, can_receive_ref, list_connected_link_inboxes,
};
use chatbotx_business::leaderboard::service as leaderboards;
use chatbotx_business::minigame::{self, play};
use chatbotx_business::referral::RefConfig;
use chatbotx_business::tx;
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::handlers::ApiState;
use crate::rpc::rpc_error;

/// `MINIGAME_PLAY_SCREENS`: the types with a play screen.
const PLAYABLE: &[&str] = &[
    "jackpot",
    "luckyWheel",
    "gashapon",
    "drawLots",
    "scratchOff",
];
const FORBIDDEN: &str = "This link is invalid or has expired.";
const COMING_SOON: &str = "This minigame type is not playable yet.";

pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/rpc/pages/leaderboardPublic", post(leaderboard_page))
        .route("/api/leaderboards/{id}/board", get(board_route))
        .route("/api/leaderboards/{id}/me", get(me_route))
        .route("/rpc/pages/minigamePlay", post(minigame_page))
        .route("/rpc/actions/playMinigameAction", post(play_action))
        .route("/rpc/pages/botSimulatorPreview", post(bot_simulator_page))
        .with_state(state)
}

fn ok(value: Value) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "json": value, "meta": [] }).to_string(),
    )
        .into_response()
}

fn body_json(body: &Bytes) -> Value {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|payload| payload.get("json").cloned())
        .unwrap_or(Value::Null)
}

/// `LEADERBOARD_ID_PATTERN` / a minigame id.
fn id_of(text: Option<&str>) -> Option<i64> {
    text.filter(|t| (1..=19).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|t| t.parse().ok())
}

/// `loadServableWorkspace(...).servable`.
async fn servable(state: &ApiState, workspace_id: i64) -> bool {
    tx::read(&state.db, move |c| {
        Ok(c.query_row(
            r#"SELECT count(*) FROM "Workspace" WHERE "id" = ?1 AND "scheduledDeletionAt" IS NULL"#,
            [workspace_id],
            |r| r.get::<_, i64>(0),
        )
        .map_err(chatbotx_db::repo::Error::from)?
            > 0)
    })
    .await
    .unwrap_or(false)
}

/// `buildLeaderboardShareUrl` / `buildMinigameShareUrl`: a ref link for the
/// viewer's own channel; `None` when the channel drops refs, is webchat
/// (self-farmable) or has no deep link.
async fn share_link(
    state: &ApiState,
    workspace_id: i64,
    contact_inbox_id: i64,
    config: RefConfig,
) -> Option<String> {
    let app_url = state.app_url.clone();
    tx::read(&state.db, move |conn| {
        let found: Option<(String, i64)> = conn
            .query_row(
                r#"SELECT "channel", "inboxId" FROM "ContactInbox" WHERE "id" = ?1"#,
                params![contact_inbox_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?;
        let Some((channel, inbox_id)) = found.filter(|(c, _)| can_receive_ref(c) && c != "webchat")
        else {
            return Ok(None);
        };
        // Workspace-scoped: the id came from a verified token, but cross-check.
        let inbox = list_connected_link_inboxes(conn, workspace_id)?
            .into_iter()
            .find(|i| i.id == inbox_id && i.channel == channel);
        Ok(inbox.and_then(|inbox| build_inbox_link(&app_url, &inbox, Some(&config))))
    })
    .await
    .ok()
    .flatten()
}

// ---- leaderboards -----------------------------------------------------------

/// `loadPublicLeaderboardBoard`: `None` when it must not be shown.
async fn load_board(state: &ApiState, id: i64) -> Option<Value> {
    let board = tx::read(&state.db, move |c| {
        Ok(chatbotx_db::repo::leaderboard::find_by_id(c, id)?)
    })
    .await
    .ok()
    .flatten()?;
    if !servable(state, board.workspace_id).await {
        return None;
    }
    leaderboards::public_board(&state.db, id)
        .await
        .ok()
        .flatten()
}

/// `loadPublicLeaderboardViewer`: `None` for an anonymous or invalid token.
async fn load_viewer(state: &ApiState, id: i64, token: Option<&str>) -> Option<Value> {
    // An unresolved `{{leaderboard_view_token}}` (link opened outside a flow) is ignored.
    let token = token.filter(|t| !t.is_empty() && !t.starts_with("{{"))?;
    let encryption = state.encryption.as_deref()?;
    let board = tx::read(&state.db, move |c| {
        Ok(chatbotx_db::repo::leaderboard::find_by_id(c, id)?)
    })
    .await
    .ok()
    .flatten()?;
    if !servable(state, board.workspace_id).await {
        return None;
    }
    let viewer = leaderboards::viewer(&state.db, encryption, id, token)
        .await
        .ok()
        .flatten()?;
    let contact_inbox = viewer["contactInboxId"]
        .as_str()
        .and_then(|s| s.parse().ok());
    let contact = viewer["contactId"].as_str().and_then(|s| s.parse().ok());
    let share_url = match (viewer["referralRef"].is_string(), contact_inbox, contact) {
        (true, Some(contact_inbox_id), Some(contact_id)) => {
            let config = RefConfig::Leaderboard {
                leaderboard_id: id,
                contact_id,
            };
            share_link(state, board.workspace_id, contact_inbox_id, config).await
        }
        _ => None,
    };
    Some(json!({
        "rank": viewer["rank"],
        "score": viewer["score"],
        "name": viewer["name"],
        "avatar": viewer["avatar"],
        "shareUrl": share_url,
    }))
}

/// `app/leaderboards/page.tsx`: `null` = not found.
async fn leaderboard_page(State(state): State<Arc<ApiState>>, body: Bytes) -> Response {
    let input = body_json(&body);
    let Some(id) = id_of(input["leaderboardId"].as_str()) else {
        return ok(Value::Null);
    };
    let Some(board) = load_board(&state, id).await else {
        return ok(Value::Null);
    };
    let token = input["token"].as_str();
    let viewer = load_viewer(&state, id, token).await;
    let token = viewer.as_ref().and(token);
    ok(json!({ "board": board, "viewer": viewer, "token": token }))
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        axum::Json(json!({ "error": "notFound" })),
    )
        .into_response()
}

/// `GET /api/leaderboards/{id}/board`: identical for every viewer, so
/// CDN-cacheable for a few seconds, with an ETag.
async fn board_route(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = id_of(Some(&id)) else {
        return not_found();
    };
    let Some(board) = load_board(&state, id).await else {
        return not_found();
    };
    let body = board.to_string();
    use base64::Engine as _;
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, body.as_bytes());
    let etag = format!(
        "\"{}\"",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.as_ref())
    );
    let cache = "public, s-maxage=3, stale-while-revalidate=10";
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag.as_str())
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::CACHE_CONTROL, cache.to_owned()),
                (header::ETAG, etag),
            ],
        )
            .into_response();
    }
    (
        StatusCode::OK,
        [
            (header::CACHE_CONTROL, cache.to_owned()),
            (header::ETAG, etag),
            (header::CONTENT_TYPE, "application/json".to_owned()),
        ],
        body,
    )
        .into_response()
}

#[derive(serde::Deserialize)]
struct MeQuery {
    token: Option<String>,
}

/// `GET /api/leaderboards/{id}/me`: the viewer's bar, never cached.
async fn me_route(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    Query(query): Query<MeQuery>,
) -> Response {
    let Some(id) = id_of(Some(&id)) else {
        return not_found();
    };
    let viewer = load_viewer(&state, id, query.token.as_deref()).await;
    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "private, no-store")],
        axum::Json(json!({ "viewer": viewer })),
    )
        .into_response()
}

// ---- minigames ----------------------------------------------------------------

async fn find_minigame(state: &ApiState, id: i64) -> Option<chatbotx_db::repo::models::Minigame> {
    let workspace: Option<i64> = tx::read(&state.db, move |c| {
        Ok(c.query_row(
            r#"SELECT "workspaceId" FROM "Minigame" WHERE "id" = ?1"#,
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await
    .ok()
    .flatten();
    minigame::find(&state.db, workspace?, id).await.ok()
}

/// The verified play token: `(contact inbox id, contact id)` of the minigame's workspace.
async fn player(state: &ApiState, workspace_id: i64, token: Option<&str>) -> Option<(i64, i64)> {
    let payload = state
        .encryption
        .as_deref()?
        .verify_minigame_play_token(token?)
        .ok()?;
    let claims = payload.claims;
    if claims.workspace_id != workspace_id.to_string() {
        return None;
    }
    let contact_inbox_id: i64 = claims.contact_inbox_id.parse().ok()?;
    let contact = tx::read(&state.db, move |c| {
        Ok(c.query_row(
            r#"SELECT "contactId" FROM "ContactInbox" WHERE "id" = ?1"#,
            [contact_inbox_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await
    .ok()
    .flatten();
    Some((
        contact_inbox_id,
        contact.unwrap_or(claims.contact_id.parse().ok()?),
    ))
}

fn ids(value: Option<&Value>) -> Vec<i64> {
    value
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|v| {
                    v.as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| v.as_i64())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `app/minigames/page.tsx`: `null` = not found; else a `status` with the
/// screen data.
async fn minigame_page(State(state): State<Arc<ApiState>>, body: Bytes) -> Response {
    let input = body_json(&body);
    let Some(game) = find_minigame(&state, id_of(input["minigameId"].as_str()).unwrap_or(0))
        .await
        .filter(|m| m.enabled)
    else {
        return ok(Value::Null);
    };
    if !servable(&state, game.workspace_id).await {
        return ok(Value::Null);
    }
    if !PLAYABLE.contains(&game.r#type.as_str()) {
        return ok(json!({ "status": "comingSoon" }));
    }
    let token = input["token"].as_str();
    let Some((contact_inbox_id, contact_id)) = player(&state, game.workspace_id, token).await
    else {
        return ok(json!({ "status": "forbidden" }));
    };
    let opened = play::resolve_opener_play_state(
        &state.db,
        play::PlayStateInput {
            minigame_id: game.id,
            contact_id,
            player_settings: game.player_settings.0.clone(),
            contact_inbox_id: Some(contact_inbox_id),
            referrer_contact_id: None,
        },
    )
    .await;
    let Ok((contact_state, _)) = opened else {
        return rpc_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_SERVER_ERROR",
            "Internal server error",
        );
    };
    let opener_tags = ids(game.general_settings.0.get("openerTagIds"));
    if let Err(error) = chatbotx_business::tag::attach_to_contact(
        &state.db,
        game.workspace_id,
        contact_id,
        &opener_tags,
    )
    .await
    {
        tracing::warn!(err = %error, "minigame opener tags failed");
    }
    let share_url = if game.player_settings.0["sharingNodeId"].is_string() {
        let config = RefConfig::MinigameShare {
            minigame_id: game.id,
            referrer_contact_id: contact_id,
        };
        share_link(&state, game.workspace_id, contact_inbox_id, config).await
    } else {
        None
    };
    ok(json!({
        "status": "ok",
        "minigame": crate::handlers::minigames::minigame_json(&state, &game),
        "contactState": { "remaining": contact_state.remaining },
        "shareUrl": share_url,
        "token": token,
    }))
}

fn fail(status: StatusCode, message: &str) -> Response {
    rpc_error(status, "BAD_REQUEST", message)
}

/// `playMinigameAction`.
async fn play_action(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let input = body_json(&body)["args"][0].clone();
    let (Some(id), Some(token)) = (id_of(input["minigameId"].as_str()), input["token"].as_str())
    else {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "Input validation failed",
        );
    };
    let Some(game) = find_minigame(&state, id).await.filter(|m| m.enabled) else {
        return fail(StatusCode::NOT_FOUND, FORBIDDEN);
    };
    if !PLAYABLE.contains(&game.r#type.as_str()) {
        return fail(StatusCode::NOT_FOUND, COMING_SOON);
    }
    let Some((contact_inbox_id, contact_id)) = player(&state, game.workspace_id, Some(token)).await
    else {
        return fail(StatusCode::FORBIDDEN, FORBIDDEN);
    };
    // `resolveGuestRateLimitKey`: the client IP, else the verified player.
    let ip = crate::auth::guest_client_ip(&headers);
    let key = if ip == crate::auth::UNKNOWN_CLIENT_IP {
        format!("contact-inbox:{contact_inbox_id}")
    } else {
        ip
    };
    let limit = crate::reflink_widget::check_guest_rate_limit(
        &state,
        &format!("minigame:{}", game.id),
        &key,
        None,
        now_ms(),
    );
    if limit.limited {
        return fail(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests. Please wait a moment and try again.",
        );
    }
    let deps = play::PlayDeps::new(&state.db, state.jobs.clone());
    let contact_inbox = play::PlayContactInbox {
        id: contact_inbox_id,
        contact_id,
    };
    // ponytail: no `{{variable}}` resolver in the API process; a text-mode
    // outcome message goes out with its spintax only.
    match play::record_play_and_dispatch(&deps, &game, contact_id, contact_inbox, None).await {
        Ok((contact_state, result)) => {
            let result = match result {
                play::PlayResult::Prize(prize) => json!({ "type": "prize", "prize": prize }),
                play::PlayResult::NonWinning => json!({ "type": "nonWinning" }),
            };
            ok(json!({ "result": result, "remaining": contact_state.remaining }))
        }
        Err(error) if error.code() == Some("minigameNoDrawsLeft") => fail(
            StatusCode::FORBIDDEN,
            "You are out of draws for this minigame.",
        ),
        Err(error) if error.code() == Some("minigameNotActive") => {
            fail(StatusCode::FORBIDDEN, "This minigame has not started yet.")
        }
        Err(error) => fail(StatusCode::BAD_REQUEST, error.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_digit_strings_up_to_19_chars() {
        assert_eq!(id_of(Some("123")), Some(123));
        assert_eq!(id_of(Some("")), None);
        assert_eq!(id_of(Some("12a")), None);
        assert_eq!(id_of(Some("12345678901234567890")), None);
        assert_eq!(id_of(None), None);
    }
}

/// The `/bs` preview: an http(s) website, a servable workspace, an enabled
/// webchat and the website on its allowed domains; else not found (null).
async fn bot_simulator_page(State(state): State<Arc<ApiState>>, body: Bytes) -> Response {
    use chatbotx_business::integration_webchat as webchats;
    let input = body_json(&body);
    let ids = (
        id_of(input["workspaceId"].as_str()),
        id_of(input["webchatId"].as_str()),
        input["url"]
            .as_str()
            .and_then(webchats::parse_simulator_website_url),
    );
    let (Some(ws), Some(webchat_id), Some(website)) = ids else {
        return ok(Value::Null);
    };
    if !servable(&state, ws).await {
        return ok(Value::Null);
    }
    let webchat = webchats::find_by_id_for_workspace_or_null(&state.db, webchat_id, ws)
        .await
        .ok()
        .flatten()
        .filter(|w| {
            w.enable && webchats::is_simulator_website_allowed(&website, &w.authorized_domains)
        });
    let Some(webchat) = webchat else {
        return ok(Value::Null);
    };
    ok(json!({
        "webchatId": webchat.id.to_string(),
        "brandColor": webchat.brand_color,
        "websiteUrl": website.as_str(),
    }))
}
