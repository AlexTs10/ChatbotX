//! `leaderboard/service.ts`, `participants.ts`, `rewards.ts`, `view.ts`,
//! `telegram-chats.ts` and the SQLite reads of `ranking.ts`.

use std::sync::Arc;

use chatbotx_core::encryption::Encryption;
use chatbotx_core::encryption::{ContactInboxClaims, LEADERBOARD_VIEW_TOKEN_TTL_MS};
use chatbotx_core::flow_config::steps::{
    choose_channel_step_default_fn, open_website_step_default_fn, send_text_step_default_fn,
};
use chatbotx_core::utils::csv::{CsvCell, CsvDialect, UTF8_BOM, to_csv_row};
use chatbotx_core::utils::id::create_id;
use chatbotx_db::Db;
use chatbotx_db::repo::leaderboard::{self as repo, LeaderboardFilter, LeaderboardWith};
use chatbotx_db::repo::leaderboard_history::{self as history, AuditLogInput};
use chatbotx_db::repo::leaderboard_participant::{
    self as participants, ParticipantListInput, ScoreChangeResult,
};
use chatbotx_db::repo::models::{File, Leaderboard};
use chatbotx_db::repo::patch::{Patch, json as json_bind};
use chatbotx_db::repo::{PaginationInput, iso, new_id, now_ms};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName, Retention};
use serde_json::{Value, json};

use super::scoring::{Change, apply_change};
use super::{
    MAX_SCORE, PUBLIC_TOP_SIZE, Stats, UpsertInput, assert_action_allowed, assert_valid_config,
    day_start, mask_name, mask_phone, platform_channel, rule_of,
};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `LEADERBOARD_WELCOME_TEXT`.
pub const WELCOME_TEXT: &str = "Hi {{first_name}}! 👋 {{referral_name}} invited you to join the leaderboard. Tap below to see the ranking and start earning points.";
const REWARD_RETRY_DELAY_MS: u64 = 30_000;
const HISTORY_GRACE_MS: i64 = 24 * 60 * 60 * 1000;
/// `leaderboardQueue` action names.
pub const AWARD: &str = "award";
pub const REVERSE: &str = "reverse";
pub const REBUILD_RANK: &str = "rebuildRank";
pub const TRIGGER_REWARDS: &str = "triggerRewards";
/// `DefaultJobAction.exportLeaderboardParticipants`.
pub const EXPORT_PARTICIPANTS: &str = "exportLeaderboardParticipants";

/// What the service needs besides the database.
#[derive(Clone)]
pub struct Deps<'a> {
    pub db: &'a Db,
    pub jobs: Option<Arc<dyn Enqueuer>>,
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Leaderboard not found")
}

fn changed_meanwhile() -> BusinessError {
    BusinessError::exception(
        "The leaderboard status changed meanwhile; reload and try again",
        "leaderboardActionNotAllowed",
        409,
    )
}

fn find_in(conn: &Connection, ws: i64, id: i64) -> Result<Leaderboard> {
    repo::find_by_id_and_workspace(conn, id, ws)?
        .map(|w| w.leaderboard)
        .ok_or_else(not_found)
}

/// The live counters of one leaderboard (`statsMany` from SQLite).
pub fn stats_in(conn: &Connection, board: &Leaderboard) -> Result<Stats> {
    let (participants, total_points, new_today) =
        participants::stats(conn, board.id, day_start(now_ms(), &board.reset_time))?;
    Ok(Stats {
        participants,
        total_points,
        new_today,
    })
}

/// `find`: the leaderboard and its counters.
pub async fn find(db: &Db, ws: i64, id: i64) -> Result<(Leaderboard, Stats)> {
    tx::read(db, move |conn| {
        let board = find_in(conn, ws, id)?;
        let stats = stats_in(conn, &board)?;
        Ok((board, stats))
    })
    .await
}

/// `findUnscoped` (the public page and the job consumer).
pub async fn find_unscoped(db: &Db, id: i64) -> Result<Option<Leaderboard>> {
    tx::read(db, move |conn| Ok(repo::find_by_id(conn, id)?)).await
}

/// `list` filters.
#[derive(Debug, Clone, Default)]
pub struct ListInput {
    pub status: Option<String>,
    pub platform: Option<String>,
    pub keyword: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// `list` result.
#[derive(Debug, Clone)]
pub struct ListOutput {
    pub rows: Vec<(LeaderboardWith, Stats)>,
    pub total: i64,
    pub page_count: i64,
    pub summary: Value,
}

/// `list`: a page with counters and the per-status totals.
pub async fn list(db: &Db, ws: i64, input: ListInput) -> Result<ListOutput> {
    tx::read(db, move |conn| {
        let page = input.page.unwrap_or(1);
        let per_page = input.per_page.unwrap_or(20);
        let filter = LeaderboardFilter {
            workspace_id: ws,
            status: input.status,
            platform: input.platform,
            keyword: input.keyword,
        };
        let rows = repo::list_paginated(
            conn,
            &filter,
            PaginationInput {
                page: Some(page),
                per_page: Some(per_page),
            },
        )?;
        let total = repo::count(conn, &filter)?;
        let counts = repo::count_by_status(conn, ws)?;
        let count = |s: &str| counts.iter().find(|(k, _)| k == s).map_or(0, |(_, n)| *n);
        let rows = rows
            .into_iter()
            .map(|row| {
                let stats = stats_in(conn, &row.leaderboard)?;
                Ok((row, stats))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ListOutput {
            rows,
            total,
            page_count: (total + per_page - 1) / per_page.max(1),
            summary: json!({
                "total": counts.iter().map(|(_, n)| n).sum::<i64>(),
                "active": count("active"),
                "paused": count("paused"),
                "finished": count("finished"),
                "draft": count("draft"),
            }),
        })
    })
    .await
}

fn exists(conn: &Connection, table: &str, id: i64, ws: i64) -> Result<bool> {
    Ok(conn
        .query_row(
            &format!(r#"SELECT 1 FROM "{table}" WHERE "id" = ?1 AND "workspaceId" = ?2"#),
            params![id, ws],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

/// `assertReferences`: host channel / minigame / flows / tags of the workspace.
fn assert_references(conn: &Connection, ws: i64, input: &UpsertInput) -> Result<()> {
    if input.platform == "gamification" {
        let found = match input.minigame_id {
            Some(id) => exists(conn, "Minigame", id, ws)?,
            None => false,
        };
        if !found {
            return Err(BusinessError::validation(
                "minigameId",
                "Minigame not found",
            ));
        }
    } else {
        let channel: Option<String> = match input.inbox_id {
            Some(id) => conn
                .query_row(
                    r#"SELECT "channel" FROM "Inbox" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, ws],
                    |r| r.get(0),
                )
                .optional()?,
            None => None,
        };
        if channel.as_deref() != platform_channel(&input.platform) || channel.is_none() {
            return Err(BusinessError::validation(
                "inboxId",
                format!("Choose a connected {} channel", input.platform),
            ));
        }
    }
    let reward_flow = input.reward_settings["flowId"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok());
    for (field, flow) in [
        ("referralWelcomeFlowId", input.referral_welcome_flow_id),
        ("rewardSettings.flowId", reward_flow),
    ] {
        if let Some(flow) = flow
            && !exists(conn, "Flow", flow, ws)?
        {
            return Err(BusinessError::validation(field, "Flow not found"));
        }
    }
    let mut tags = input.tag_ids.clone();
    tags.sort_unstable();
    tags.dedup();
    if !tags.is_empty() {
        let found: i64 = conn.query_row(
            r#"SELECT count(*) FROM "Tag" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
            params![ws, serde_json::to_string(&tags).unwrap_or_default()],
            |r| r.get(0),
        )?;
        if found != tags.len() as i64 {
            return Err(BusinessError::validation("tagIds", "Tag not found"));
        }
    }
    Ok(())
}

/// `toColumns`: the row columns (gamification keeps only the minigame).
fn normalized(mut input: UpsertInput) -> UpsertInput {
    input.name = input.name.trim().to_owned();
    if input.platform == "gamification" {
        input.inbox_id = None;
    } else {
        input.minigame_id = None;
    }
    let mut seen = Vec::new();
    input.tag_ids.retain(|id| {
        let fresh = !seen.contains(id);
        seen.push(*id);
        fresh
    });
    input
}

fn opt_int(value: Option<i64>) -> Sql {
    value.map_or(Sql::Null, Sql::Integer)
}

fn columns(input: &UpsertInput) -> Patch {
    let tags: Vec<String> = input.tag_ids.iter().map(i64::to_string).collect();
    vec![
        ("name", Sql::Text(input.name.clone())),
        ("description", Sql::Text(input.description.clone())),
        ("coverImageUrl", Sql::Text(input.cover_image_url.clone())),
        ("startsAt", Sql::Integer(input.starts_at)),
        ("endsAt", Sql::Integer(input.ends_at)),
        ("platform", Sql::Text(input.platform.clone())),
        ("inboxId", opt_int(input.inbox_id)),
        ("minigameId", opt_int(input.minigame_id)),
        ("scope", json_bind(&input.scope)),
        ("rules", json_bind(&input.rules)),
        (
            "followersOnly",
            Sql::Integer(i64::from(input.followers_only)),
        ),
        ("tagIds", json_bind(&json!(tags))),
        ("resetTime", Sql::Text(input.reset_time.clone())),
        ("cooldownSeconds", Sql::Integer(input.cooldown_seconds)),
        (
            "referralWelcomeFlowId",
            opt_int(input.referral_welcome_flow_id),
        ),
        ("rewardSettings", json_bind(&input.reward_settings)),
    ]
}

/// `buildLeaderboardWelcomeNode`: the welcome text and a button to the board.
fn welcome_node(view_url: &str) -> (i64, Value) {
    let id = create_id();
    let mut website = open_website_step_default_fn();
    website["url"] = json!(view_url);
    let mut send = send_text_step_default_fn(&json!({ "text": WELCOME_TEXT }));
    send["buttons"] = json!([{
        "id": create_id(),
        "label": "View leaderboard",
        "buttonType": "openWebsite",
        "beforeStep": website,
        "steps": [],
    }]);
    let node = json!({
        "id": id,
        "position": { "x": 100, "y": 300 },
        "measured": { "width": 288, "height": 100 },
        "type": "sendMessage",
        "data": {
            "name": "Start",
            "isStartNode": true,
            "details": {
                "beforeStep": choose_channel_step_default_fn(&json!({})),
                "steps": [send],
                "quickReplies": [],
            },
        },
    });
    (id.parse().unwrap_or_default(), node)
}

/// `create`: a draft. With referral on and no welcome flow, a published
/// welcome flow is created; `view_url` is the public link with `{id}` for
/// the leaderboard id.
pub async fn create(
    db: &Db,
    ws: i64,
    input: UpsertInput,
    view_url: Option<String>,
) -> Result<Leaderboard> {
    let input = normalized(input);
    assert_valid_config(&input)?;
    tx::write(db, move |conn| {
        assert_references(conn, ws, &input)?;
        let wants_flow = input.referral_welcome_flow_id.is_none()
            && rule_of(&input.rules, "referral").is_some_and(|r| r.enabled)
            && input.platform != "email";
        let mut patch = columns(&input);
        patch.push(("workspaceId", Sql::Integer(ws)));
        patch.push(("status", Sql::Text("draft".into())));
        let row = repo::insert(conn, &patch)?;
        let Some(url) = view_url.filter(|_| wants_flow) else {
            return Ok(row);
        };
        let (start_node_id, node) = welcome_node(&url.replace("{id}", &row.id.to_string()));
        let ids = crate::flow::create_published_default(
            conn,
            crate::flow::PublishedDefault {
                workspace_id: ws,
                name: format!("Leaderboard welcome - {}", row.name),
                folder_id: None,
                start_node_id,
                nodes: vec![node],
                edges: vec![],
            },
        )?;
        Ok(repo::update(
            conn,
            row.id,
            ws,
            &vec![("referralWelcomeFlowId", Sql::Integer(ids.flow_id))],
        )?
        .unwrap_or(row))
    })
    .await
}

/// `update`: platform, channel and start are fixed once published.
pub async fn update(db: &Db, ws: i64, id: i64, input: UpsertInput) -> Result<Leaderboard> {
    let input = normalized(input);
    tx::write(db, move |conn| {
        let current = find_in(conn, ws, id)?;
        assert_action_allowed(&current.status, "edit")?;
        if current.status != "draft"
            && (input.platform != current.platform
                || input.inbox_id != current.inbox_id
                || input.minigame_id != current.minigame_id)
        {
            return Err(BusinessError::validation(
                "platform",
                "The platform and channel cannot change after publishing",
            ));
        }
        if current.status != "draft" && input.starts_at != current.starts_at {
            return Err(BusinessError::validation(
                "startsAt",
                "The start time cannot change after publishing",
            ));
        }
        assert_valid_config(&input)?;
        assert_references(conn, ws, &input)?;
        repo::update(conn, id, ws, &columns(&input))?.ok_or_else(not_found)
    })
    .await
}

async fn transition(
    db: &Db,
    ws: i64,
    id: i64,
    action: &'static str,
    next: Patch,
) -> Result<Leaderboard> {
    tx::write(db, move |conn| {
        let current = find_in(conn, ws, id)?;
        assert_action_allowed(&current.status, action)?;
        repo::transition_status(conn, id, ws, &current.status, &next)?.ok_or_else(changed_meanwhile)
    })
    .await
}

fn status(value: &str) -> Patch {
    vec![("status", Sql::Text(value.to_owned()))]
}

/// `publish`: needs an enabled rule and a future end.
pub async fn publish(db: &Db, ws: i64, id: i64) -> Result<Leaderboard> {
    let (current, _) = find(db, ws, id).await?;
    let any_enabled = current.rules.0.as_object().is_some_and(|rules| {
        rules
            .keys()
            .any(|k| rule_of(&current.rules.0, k).is_some_and(|r| r.enabled))
    });
    if !any_enabled {
        return Err(BusinessError::validation(
            "rules",
            "Enable at least one scoring rule",
        ));
    }
    if current.ends_at <= now_ms() {
        return Err(BusinessError::validation(
            "endsAt",
            "The end time has already passed",
        ));
    }
    transition(db, ws, id, "publish", status("active")).await
}

pub async fn pause(db: &Db, ws: i64, id: i64) -> Result<Leaderboard> {
    transition(db, ws, id, "pause", status("paused")).await
}

pub async fn start(db: &Db, ws: i64, id: i64) -> Result<Leaderboard> {
    transition(db, ws, id, "start", status("active")).await
}

async fn add_job(
    deps: &Deps<'_>,
    queue: QueueName,
    name: &str,
    data: Value,
    opts: JobOptions,
) -> Result<()> {
    let Some(jobs) = &deps.jobs else {
        return Ok(());
    };
    jobs.add(queue, name, data, opts)
        .await
        .map(|_| ())
        .map_err(|e| BusinessError::exception(e.to_string(), "systemError", 500))
}

fn rewards_job(board: &Leaderboard) -> Value {
    json!({ "type": TRIGGER_REWARDS, "data": {
        "workspaceId": board.workspace_id.to_string(),
        "leaderboardId": board.id.to_string(),
    }})
}

/// `finish`: closes the campaign; automatic rewards go out after the
/// late-event grace.
pub async fn finish(deps: &Deps<'_>, ws: i64, id: i64) -> Result<Leaderboard> {
    let mut next = status("finished");
    next.push(("finishedAt", Sql::Integer(now_ms())));
    let row = transition(deps.db, ws, id, "finish", next).await?;
    let settings = &row.reward_settings.0;
    if settings["autoOnFinish"] == true && settings["flowId"].is_string() {
        add_job(
            deps,
            QueueName::Leaderboard,
            TRIGGER_REWARDS,
            rewards_job(&row),
            JobOptions {
                job_id: Some(format!("lb-rewards-{}", row.id)),
                delay_ms: Some(super::LATE_EVENT_GRACE_MS as u64),
                ..JobOptions::default()
            },
        )
        .await?;
    }
    Ok(row)
}

/// `finishOverdue` (the minute cron): every active/paused board past its end.
pub async fn finish_overdue(deps: &Deps<'_>, now: i64) -> Result<usize> {
    let overdue = tx::read(deps.db, move |conn| Ok(repo::find_overdue(conn, now)?)).await?;
    let mut finished = 0;
    for (id, ws) in overdue {
        match finish(deps, ws, id).await {
            Ok(_) => finished += 1,
            Err(BusinessError::Exception { code, .. }) if code == "leaderboardActionNotAllowed" => {
            }
            Err(other) => return Err(other),
        }
    }
    Ok(finished)
}

/// `resetPoints`: every score to 0 (Active only), audited.
pub async fn reset_points(
    db: &Db,
    ws: i64,
    id: i64,
    actor_user_id: Option<i64>,
    reason: String,
) -> Result<usize> {
    tx::write(db, move |conn| {
        let current = find_in(conn, ws, id)?;
        assert_action_allowed(&current.status, "resetPoints")?;
        let count = participants::reset_all(conn, current.id, now_ms())?;
        history::insert_audit_log(
            conn,
            &AuditLogInput {
                leaderboard_id: current.id,
                workspace_id: current.workspace_id,
                contact_id: None,
                action: "resetAll".into(),
                delta: 0,
                actor_user_id,
                reason: reason.trim().to_owned(),
            },
        )?;
        Ok(count)
    })
    .await
}

/// `delete` (soft).
pub async fn delete(db: &Db, ws: i64, id: i64) -> Result<i64> {
    tx::write(db, move |conn| {
        let current = find_in(conn, ws, id)?;
        assert_action_allowed(&current.status, "delete")?;
        repo::update(conn, id, ws, &vec![("deletedAt", Sql::Integer(now_ms()))])?;
        Ok(current.id)
    })
    .await
}

/// `createExport`: a pending CSV file row and the export job.
pub async fn create_export(
    deps: &Deps<'_>,
    ws: i64,
    id: i64,
    user_id: Option<i64>,
    include_contact_info: bool,
) -> Result<Value> {
    let (board, _) = find(deps.db, ws, id).await?;
    assert_action_allowed(&board.status, "export")?;
    let stamp = iso(now_ms()).replace([':', '.'], "-");
    let file_name = format!("leaderboard-{}-{stamp}.csv", board.id);
    let path = format!("workspaces/{ws}/exports/{file_name}");
    let (name, p) = (file_name.clone(), path.clone());
    let file = tx::write(deps.db, move |conn| {
        Ok(repo::create_export_file(conn, ws, user_id, &name, &p)?)
    })
    .await?;
    add_job(
        deps,
        QueueName::Default,
        EXPORT_PARTICIPANTS,
        json!({ "type": EXPORT_PARTICIPANTS, "data": {
            "workspaceId": ws.to_string(),
            "leaderboardId": board.id.to_string(),
            "fileId": file.id.to_string(),
            "outputPath": path,
            "includeContactInfo": include_contact_info,
        }}),
        JobOptions::default(),
    )
    .await?;
    Ok(json!({ "fileId": file.id.to_string(), "fileName": file_name, "status": file.status }))
}

/// `getExport`.
pub async fn get_export(db: &Db, ws: i64, file_id: i64) -> Result<File> {
    tx::read(db, move |conn| {
        repo::find_export_file(conn, ws, file_id)?
            .ok_or_else(|| BusinessError::not_found("Export file not found"))
    })
    .await
}

/// One ranked contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopEntry {
    pub rank: i64,
    pub contact_id: i64,
    pub score: i64,
}

/// `top`: the ranked contacts (banned excluded) from SQLite.
pub fn top_in(conn: &Connection, board_id: i64, limit: i64, offset: i64) -> Result<Vec<TopEntry>> {
    Ok(
        participants::top_ranked(conn, board_id, limit, Some(offset))?
            .into_iter()
            .enumerate()
            .map(|(i, row)| TopEntry {
                rank: offset + i as i64 + 1,
                contact_id: row.contact_id,
                score: row.score,
            })
            .collect(),
    )
}

pub async fn top(db: &Db, board_id: i64, limit: i64) -> Result<Vec<TopEntry>> {
    tx::read(db, move |conn| top_in(conn, board_id, limit, 0)).await
}

/// `getLeaderboardTop`: the admin Top N with real names.
pub async fn admin_top(db: &Db, ws: i64, id: i64, limit: i64) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let board = find_in(conn, ws, id)?;
        let top = top_in(conn, board.id, limit.clamp(1, 500), 0)?;
        let ids: Vec<i64> = top.iter().map(|e| e.contact_id).collect();
        let profiles = participants::find_contact_profiles(conn, ws, &ids)?;
        Ok(top
            .iter()
            .map(|e| {
                let p = profiles.iter().find(|p| p.id == e.contact_id);
                json!({
                    "rank": e.rank,
                    "contactId": e.contact_id.to_string(),
                    "score": e.score,
                    "fullName": p.and_then(|p| p.full_name.clone()),
                    "avatar": p.and_then(|p| p.avatar.clone()),
                })
            })
            .collect())
    })
    .await
}

/// `getLeaderboardPublicBoard`: the shared part of the public page (`None`
/// for unknown, draft or deleted boards). Ranks 4+ are masked, no avatar.
pub async fn public_board(db: &Db, id: i64) -> Result<Option<Value>> {
    tx::read(db, move |conn| {
        let Some(board) = repo::find_by_id(conn, id)?.filter(|b| b.status != "draft") else {
            return Ok(None);
        };
        let top = top_in(conn, board.id, PUBLIC_TOP_SIZE, 0)?;
        let ids: Vec<i64> = top.iter().map(|e| e.contact_id).collect();
        let profiles = participants::find_contact_profiles(conn, board.workspace_id, &ids)?;
        let entries: Vec<Value> = top
            .iter()
            .map(|e| {
                let p = profiles.iter().find(|p| p.id == e.contact_id);
                let full = p.and_then(|p| p.full_name.clone());
                let podium = e.rank <= 3;
                json!({
                    "rank": e.rank,
                    "name": if podium { full.unwrap_or_else(|| mask_name(None)) } else { mask_name(full.as_deref()) },
                    "avatar": if podium { p.and_then(|p| p.avatar.clone()) } else { None },
                    "score": e.score,
                })
            })
            .collect();
        Ok(Some(json!({
            "id": board.id.to_string(),
            "name": board.name,
            "description": board.description,
            "coverImageUrl": board.cover_image_url,
            "status": board.status,
            "platform": board.platform,
            "startsAt": iso(board.starts_at),
            "endsAt": iso(board.ends_at),
            "top": entries,
        })))
    })
    .await
}

fn contact_inbox_for(
    conn: &Connection,
    ws: i64,
    contact: i64,
    inbox: Option<i64>,
) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            r#"SELECT "id" FROM "ContactInbox" WHERE "workspaceId" = ?1 AND "contactId" = ?2
               ORDER BY ("inboxId" IS ?3) DESC, "createdAt" ASC LIMIT 1"#,
            params![ws, contact, inbox],
            |r| r.get(0),
        )
        .optional()?)
}

/// `signLeaderboardViewTokenForContact` (`None` when the contact never
/// talked to a channel).
pub async fn sign_view_token(
    db: &Db,
    encryption: &Encryption,
    board: &Leaderboard,
    contact_id: i64,
) -> Result<Option<String>> {
    let (ws, inbox) = (board.workspace_id, board.inbox_id);
    let Some(contact_inbox) = tx::read(db, move |conn| {
        contact_inbox_for(conn, ws, contact_id, inbox)
    })
    .await?
    else {
        return Ok(None);
    };
    let claims = ContactInboxClaims {
        workspace_id: ws.to_string(),
        contact_id: contact_id.to_string(),
        contact_inbox_id: contact_inbox.to_string(),
    };
    encryption
        .sign_leaderboard_view_token(&claims, LEADERBOARD_VIEW_TOKEN_TTL_MS)
        .map(Some)
        .map_err(|e| BusinessError::exception(e.to_string(), "systemError", 500))
}

/// `getLeaderboardViewer`: "my rank" and the referral ref of a verified
/// view token (`None` for a bad token or a foreign workspace).
pub async fn viewer(
    db: &Db,
    encryption: &Encryption,
    id: i64,
    token: &str,
) -> Result<Option<Value>> {
    let Ok(payload) = encryption.verify_leaderboard_view_token(token) else {
        return Ok(None);
    };
    let claims = payload.claims;
    let (Ok(ws), Ok(contact)) = (
        claims.workspace_id.parse::<i64>(),
        claims.contact_id.parse::<i64>(),
    ) else {
        return Ok(None);
    };
    tx::read(db, move |conn| {
        let Some(board) = repo::find_by_id(conn, id)?
            .filter(|b| b.status != "draft" && b.workspace_id == ws)
        else {
            return Ok(None);
        };
        let rank = participants::rank_of(conn, board.id, contact)?;
        let profile = participants::find_contact_profiles(conn, ws, &[contact])?
            .into_iter()
            .next();
        let referral = rule_of(&board.rules.0, "referral").is_some_and(|r| r.enabled)
            && board.platform != "email";
        Ok(Some(json!({
            "contactId": claims.contact_id,
            "contactInboxId": claims.contact_inbox_id,
            "rank": rank.map(|(r, _)| r),
            "score": rank.map_or(0, |(_, s)| s),
            "name": profile.as_ref().and_then(|p| p.full_name.clone()),
            "avatar": profile.and_then(|p| p.avatar),
            "referralRef": referral.then(|| crate::referral::encode_ref(
                &crate::referral::RefConfig::Leaderboard { leaderboard_id: board.id, contact_id: contact },
            )),
        })))
    })
    .await
}

/// `assertEditable`: manual edits while running or paused.
fn assert_editable(board: &Leaderboard) -> Result<()> {
    match board.status.as_str() {
        "finished" => Err(finished_error()),
        "draft" => Err(BusinessError::exception(
            "Publish the leaderboard before editing scores",
            "leaderboardNotPublished",
            409,
        )),
        _ => Ok(()),
    }
}

fn finished_error() -> BusinessError {
    BusinessError::exception(
        "Scores are locked because the leaderboard has finished",
        "leaderboardFinished",
        409,
    )
}

fn banned_error() -> BusinessError {
    BusinessError::exception(
        "This participant is banned",
        "leaderboardParticipantBanned",
        409,
    )
}

/// `assertManualChangeApplied`.
fn manual_applied(result: ScoreChangeResult) -> Result<i64> {
    match result {
        ScoreChangeResult::Applied { score, .. } => Ok(score),
        ScoreChangeResult::Closed => Err(finished_error()),
        _ => Err(banned_error()),
    }
}

const MANUAL_STATUSES: &[&str] = &["active", "paused"];

/// The participants table filters.
#[derive(Debug, Clone, Default)]
pub struct ParticipantsInput {
    pub keyword: Option<String>,
    /// `all` | `top10` | `top50` | `banned`.
    pub filter: String,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// `leaderboardParticipantService.list`: masked phones and ranks.
pub async fn list_participants(
    db: &Db,
    ws: i64,
    id: i64,
    input: ParticipantsInput,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let board = find_in(conn, ws, id)?;
        let per_page = input.per_page.unwrap_or(20);
        let contact_ids = match input.filter.as_str() {
            "top10" => Some(10),
            "top50" => Some(50),
            _ => None,
        }
        .map(|n| top_in(conn, board.id, n, 0))
        .transpose()?
        .map(|top| top.iter().map(|e| e.contact_id).collect());
        let page = participants::list_paginated(
            conn,
            &ParticipantListInput {
                leaderboard_id: board.id,
                workspace_id: board.workspace_id,
                keyword: input.keyword,
                banned_only: input.filter == "banned",
                contact_ids,
                page: input.page.unwrap_or(1),
                per_page,
            },
        )?;
        let mut data = Vec::new();
        for row in page.rows {
            let rank = if row.is_banned {
                None
            } else {
                participants::rank_of(conn, board.id, row.contact_id)?.map(|(r, _)| r)
            };
            data.push(json!({
                "contactId": row.contact_id.to_string(),
                "score": row.score,
                "isBanned": row.is_banned,
                "breakdown": row.breakdown.0,
                "referrerContactId": row.referrer_contact_id.map(|id| id.to_string()),
                "fullName": row.full_name,
                "avatar": row.avatar,
                "phoneNumber": mask_phone(row.phone_number.as_deref()),
                "lastActivityAt": iso(row.last_activity_at),
                "createdAt": iso(row.created_at),
                "rank": rank,
            }));
        }
        Ok(json!({
            "data": data,
            "total": page.total,
            "pageCount": (page.total + per_page - 1) / per_page.max(1),
        }))
    })
    .await
}

fn audit_row(
    board: &Leaderboard,
    contact: Option<i64>,
    action: &str,
    delta: i64,
    actor: Option<i64>,
    reason: &str,
) -> AuditLogInput {
    AuditLogInput {
        leaderboard_id: board.id,
        workspace_id: board.workspace_id,
        contact_id: contact,
        action: action.into(),
        delta,
        actor_user_id: actor,
        reason: reason.to_owned(),
    }
}

async fn editable_board(db: &Db, ws: i64, id: i64) -> Result<Leaderboard> {
    let (board, _) = find(db, ws, id).await?;
    assert_editable(&board)?;
    Ok(board)
}

/// `adjust`: a manual +/- with a mandatory reason, audited.
pub async fn adjust(
    db: &Db,
    ws: i64,
    id: i64,
    contact_id: i64,
    delta: i64,
    reason: &str,
    actor: Option<i64>,
) -> Result<i64> {
    let reason = reason.trim().to_owned();
    if reason.is_empty() {
        return Err(BusinessError::validation(
            "reason",
            "Enter a reason for the adjustment",
        ));
    }
    if delta == 0 || delta.abs() > MAX_SCORE {
        return Err(BusinessError::validation(
            "delta",
            "Enter a non-zero number of points",
        ));
    }
    let board = editable_board(db, ws, id).await?;
    let found = tx::read(db, move |conn| exists(conn, "Contact", contact_id, ws)).await?;
    if !found {
        return Err(BusinessError::not_found("Contact not found"));
    }
    let audit = audit_row(&board, Some(contact_id), "adjust", delta, actor, &reason);
    let result = apply_change(
        db,
        &board,
        Change {
            contact_id,
            delta,
            source: "manual",
            breakdown_key: Some("manual".into()),
            event_id: new_id(),
            event_at: now_ms(),
            allowed_statuses: MANUAL_STATUSES,
            ..Change::default()
        },
        Some(Box::new(move |conn| {
            history::insert_audit_log(conn, &audit)?;
            Ok(())
        })),
    )
    .await?;
    manual_applied(result)
}

/// `setBanned`: ban / unban, audited.
pub async fn set_banned(
    db: &Db,
    ws: i64,
    id: i64,
    contact_id: i64,
    is_banned: bool,
    reason: &str,
    actor: Option<i64>,
) -> Result<()> {
    let board = editable_board(db, ws, id).await?;
    let action = if is_banned { "ban" } else { "unban" };
    let audit = audit_row(&board, Some(contact_id), action, 0, actor, reason.trim());
    tx::write(db, move |conn| {
        participants::set_banned(conn, board.id, contact_id, is_banned)?
            .ok_or_else(|| BusinessError::not_found("Participant not found"))?;
        history::insert_audit_log(conn, &audit)?;
        Ok(())
    })
    .await
}

/// `resetOne`: one participant to 0 (a negative adjustment).
pub async fn reset_one(
    db: &Db,
    ws: i64,
    id: i64,
    contact_id: i64,
    reason: &str,
    actor: Option<i64>,
) -> Result<i64> {
    let board = editable_board(db, ws, id).await?;
    let board_id = board.id;
    let participant = tx::read(db, move |conn| {
        Ok(participants::find(conn, board_id, contact_id)?)
    })
    .await?
    .ok_or_else(|| BusinessError::not_found("Participant not found"))?;
    if participant.score == 0 {
        return Ok(0);
    }
    if participant.is_banned {
        return Err(banned_error());
    }
    let delta = -participant.score;
    let audit = audit_row(
        &board,
        Some(contact_id),
        "resetOne",
        delta,
        actor,
        reason.trim(),
    );
    let result = apply_change(
        db,
        &board,
        Change {
            contact_id,
            delta,
            source: "reset",
            event_id: new_id(),
            event_at: now_ms(),
            allowed_statuses: MANUAL_STATUSES,
            ..Change::default()
        },
        Some(Box::new(move |conn| {
            history::insert_audit_log(conn, &audit)?;
            Ok(())
        })),
    )
    .await?;
    manual_applied(result).map(|_| 0)
}

/// `auditLog`.
pub async fn audit_log(
    db: &Db,
    ws: i64,
    id: i64,
    page: Option<i64>,
    per_page: Option<i64>,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let board = find_in(conn, ws, id)?;
        let per_page = per_page.unwrap_or(20);
        let (rows, total) = history::list_audit_logs(
            conn,
            board.id,
            ws,
            PaginationInput {
                page: Some(page.unwrap_or(1)),
                per_page: Some(per_page),
            },
        )?;
        let data: Vec<Value> = rows
            .into_iter()
            .map(|row| {
                json!({
                    "id": row.log.id.to_string(),
                    "createdAt": iso(row.log.created_at),
                    "action": row.log.action,
                    "delta": row.log.delta,
                    "reason": row.log.reason,
                    "contact": row.contact.map(|(id, name, avatar)| json!({
                        "id": id.to_string(), "fullName": name, "avatar": avatar,
                    })),
                    "actor": row.actor.map(|(id, name, email)| json!({
                        "id": id.to_string(), "name": name, "email": email,
                    })),
                })
            })
            .collect();
        Ok(json!({
            "data": data,
            "total": total,
            "pageCount": (total + per_page - 1) / per_page.max(1),
        }))
    })
    .await
}

/// `history`: one participant's score events in the campaign window.
pub async fn participant_history(db: &Db, ws: i64, id: i64, contact_id: i64) -> Result<Value> {
    tx::read(db, move |conn| {
        let board = find_in(conn, ws, id)?;
        let to = board.ends_at.max(now_ms()) + HISTORY_GRACE_MS;
        let rows = history::list_events_for_contact(
            conn,
            board.id,
            contact_id,
            board.starts_at - HISTORY_GRACE_MS,
            to,
            100,
        )?;
        Ok(Value::Array(
            rows.into_iter()
                .map(|e| {
                    json!({
                        "id": e.id.to_string(),
                        "createdAt": iso(e.created_at),
                        "delta": e.delta,
                        "source": e.source,
                        "rule": e.rule,
                        "sourceRef": e.source_ref,
                    })
                })
                .collect(),
        ))
    })
    .await
}

/// `sendLeaderboardFlow`: starts `flow_id` on the contact's DM conversation.
async fn send_flow(
    deps: &Deps<'_>,
    board: &Leaderboard,
    contact_id: i64,
    flow_id: &str,
    job_id: String,
) -> Result<bool> {
    let (ws, inbox) = (board.workspace_id, board.inbox_id);
    let target = tx::read(deps.db, move |conn| {
        let conversation: Option<i64> = conn
            .query_row(
                r#"SELECT "id" FROM "Conversation" WHERE "workspaceId" = ?1 AND "contactId" = ?2 AND "sourceId" IS NULL LIMIT 1"#,
                params![ws, contact_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(conversation) = conversation else {
            return Ok(None);
        };
        Ok(contact_inbox_for(conn, ws, contact_id, inbox)?.map(|ci| (conversation, ci)))
    })
    .await?;
    let Some((conversation, contact_inbox)) = target else {
        return Ok(false);
    };
    add_job(
        deps,
        QueueName::Integration,
        "sendFlow",
        json!({ "type": "sendFlow", "data": {
            "conversationId": conversation.to_string(),
            "contactInboxId": contact_inbox.to_string(),
            "flowId": flow_id,
        }}),
        JobOptions {
            job_id: Some(job_id),
            ..JobOptions::default()
        },
    )
    .await?;
    Ok(true)
}

/// `triggerLeaderboardRewards`: the reward flow to the top N of a finished
/// board, each winner claimed once.
pub async fn trigger_rewards(
    deps: &Deps<'_>,
    ws: i64,
    id: i64,
    retry_in_background: bool,
) -> Result<Value> {
    let (board, _) = find(deps.db, ws, id).await?;
    if board.status != "finished" {
        return Err(BusinessError::exception(
            "Rewards can only be sent after the leaderboard finishes",
            "leaderboardNotFinished",
            409,
        ));
    }
    let settings = board.reward_settings.0.clone();
    let Some(flow_id) = settings["flowId"].as_str().map(str::to_owned) else {
        return Err(BusinessError::exception(
            "Choose a reward flow first",
            "leaderboardRewardFlowMissing",
            422,
        ));
    };
    let top_n = settings["topN"].as_i64().unwrap_or(3);
    let board_id = board.id;
    let (claim, winners) = tx::write(deps.db, move |conn| {
        let claim = repo::claim_rewards(conn, board_id, ws, now_ms())?.ok_or_else(not_found)?;
        let winners = top_in(conn, board_id, top_n, 0)?;
        Ok((claim, winners))
    })
    .await?;
    let unsent: Vec<TopEntry> = winners
        .iter()
        .copied()
        .filter(|w| claim.recipients.get(w.contact_id.to_string()).is_none())
        .collect();
    if claim.previously_triggered && unsent.is_empty() {
        return Err(BusinessError::exception(
            "Rewards were already sent",
            "leaderboardRewardsAlreadySent",
            409,
        ));
    }
    let (mut sent, mut skipped, mut failed) = (0, Vec::new(), Vec::new());
    for winner in unsent {
        let contact = winner.contact_id;
        let claimed = tx::write(deps.db, move |conn| {
            Ok(repo::claim_reward_recipient(conn, board_id, contact)?)
        })
        .await?;
        if !claimed {
            continue;
        }
        let job_id = format!("lb-reward-{board_id}-{contact}");
        let delivery = match send_flow(deps, &board, contact, &flow_id, job_id).await {
            Ok(true) => Some("sent"),
            Ok(false) => Some("skipped"),
            Err(error) => {
                tracing::error!(err = %error, leaderboard_id = board_id, "failed to send leaderboard reward flow");
                None
            }
        };
        let release = tx::write(deps.db, move |conn| {
            Ok(repo::set_reward_recipient(
                conn, board_id, contact, delivery,
            )?)
        })
        .await;
        if let Err(error) = release {
            tracing::error!(err = %error, leaderboard_id = board_id, "failed to record a leaderboard reward");
        }
        match delivery {
            Some("sent") => sent += 1,
            Some(_) => skipped.push(contact.to_string()),
            None => failed.push(contact.to_string()),
        }
    }
    if !failed.is_empty() && retry_in_background {
        let retry = add_job(
            deps,
            QueueName::Leaderboard,
            TRIGGER_REWARDS,
            rewards_job(&board),
            JobOptions {
                job_id: Some(format!("lb-rewards-retry-{board_id}")),
                delay_ms: Some(REWARD_RETRY_DELAY_MS),
                remove_on_complete: Some(Retention::Remove),
                ..JobOptions::default()
            },
        )
        .await;
        if let Err(error) = retry {
            tracing::error!(err = %error, leaderboard_id = board_id, "failed to queue the leaderboard reward retry");
        }
    }
    Ok(json!({
        "winners": winners.iter().map(|w| json!({
            "rank": w.rank, "contactId": w.contact_id.to_string(), "score": w.score,
        })).collect::<Vec<_>>(),
        "sent": sent,
        "skipped": skipped,
        "failed": failed,
    }))
}

/// `recordTelegramBotChatStatus`: the bot's membership in a group/channel.
pub async fn record_telegram_chat(
    db: &Db,
    ws: i64,
    inbox_id: i64,
    chat: &Value,
    status: &str,
) -> Result<()> {
    const TYPES: &[&str] = &["private", "group", "supergroup", "channel"];
    const STATUSES: &[&str] = &[
        "creator",
        "administrator",
        "member",
        "restricted",
        "left",
        "kicked",
    ];
    let kind = chat["type"].as_str().unwrap_or_default().to_owned();
    if !TYPES.contains(&kind.as_str()) || !STATUSES.contains(&status) {
        tracing::warn!(chat_type = %kind, status, "unknown telegram chat type/status; not recorded");
        return Ok(());
    }
    let chat_id = match &chat["id"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let title = chat["title"].as_str().map(str::to_owned);
    let status = status.to_owned();
    tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "TelegramChat" ("id", "createdAt", "updatedAt", "workspaceId", "inboxId", "chatId", "title", "type", "botStatus")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
               ON CONFLICT ("inboxId", "chatId") DO UPDATE SET "title" = excluded."title", "type" = excluded."type",
                 "botStatus" = excluded."botStatus", "updatedAt" = excluded."updatedAt""#,
            params![new_id(), now, ws, inbox_id, chat_id, title, kind, status],
        )?;
        Ok(())
    })
    .await
}

/// `listTelegramLeaderboardChats`: groups where the bot is an admin.
pub async fn telegram_chats(db: &Db, ws: i64, inbox_id: i64) -> Result<Value> {
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "chatId", "title", "type" FROM "TelegramChat" WHERE "workspaceId" = ?1 AND "inboxId" = ?2
               AND "botStatus" IN ('administrator', 'creator') ORDER BY "title" ASC, "chatId" ASC"#,
        )?;
        let rows = stmt
            .query_map(params![ws, inbox_id], |r| {
                Ok(json!({
                    "chatId": r.get::<_, String>(0)?,
                    "title": r.get::<_, Option<String>>(1)?,
                    "type": r.get::<_, String>(2)?,
                }))
            })?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()?;
        Ok(Value::Array(rows))
    })
    .await
}

/// `enqueueLeaderboardAward` / `enqueueLeaderboardReverse` for the event
/// sources: queues when a live leaderboard of the host scores the rule.
/// Never fails.
/// `inboxHasLeaderboardRule` (or the minigame's): a scorable board listens
/// for `rule`. A read failure counts as "no".
pub async fn has_rule(db: &Db, inbox: Option<i64>, minigame: Option<i64>, rule: &str) -> bool {
    let rule = rule.to_owned();
    let since = now_ms() - super::LATE_EVENT_GRACE_MS;
    tx::read(db, move |conn| {
        let boards = match (minigame, inbox) {
            (Some(id), _) => repo::find_scorable_for_minigame(conn, id, since)?,
            (None, Some(id)) => repo::find_scorable_for_inbox(conn, id, since)?,
            _ => Vec::new(),
        };
        Ok(boards
            .iter()
            .any(|b| rule_of(&b.rules.0, &rule).is_some_and(|r| r.enabled)))
    })
    .await
    .unwrap_or(false)
}

pub async fn enqueue(
    jobs: &dyn Enqueuer,
    db: &Db,
    reverse: bool,
    input: &super::scoring::Award,
) -> bool {
    let (inbox, minigame) = (input.inbox_id, input.minigame_id);
    if !has_rule(db, inbox, minigame, &input.rule).await {
        return false;
    }
    let host = match minigame {
        Some(id) => format!("mg{id}"),
        None => format!("in{}", inbox.unwrap_or_default()),
    };
    let (name, rule_part) = if reverse {
        (REVERSE, format!("reverse-{}", input.rule))
    } else {
        (AWARD, input.rule.clone())
    };
    let mut data = json!({
        "workspaceId": input.workspace_id.to_string(),
        "contactId": input.contact_id.to_string(),
        "inboxId": inbox.map(|id| id.to_string()),
        "rule": input.rule,
        "sourceRef": input.source_ref,
        "eventAt": iso(input.event_at),
    });
    if !reverse {
        data["minigameId"] = json!(minigame.map(|id| id.to_string()));
    }
    if !input.context.is_null() {
        data["context"] = input.context.clone();
    }
    let job_id = format!(
        "lb-award-{}-{host}-{rule_part}-{}",
        input.workspace_id, input.source_ref
    )
    .replace(':', "-");
    let opts = JobOptions {
        job_id: Some(job_id),
        remove_on_complete: Some(Retention::KeepAge {
            age_secs: 24 * 60 * 60,
            count: None,
        }),
        ..JobOptions::default()
    };
    match jobs
        .add(
            QueueName::Leaderboard,
            name,
            json!({ "type": name, "data": data }),
            opts,
        )
        .await
    {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(err = %error, rule = %input.rule, "failed to enqueue leaderboard award");
            false
        }
    }
}

const EXPORT_PAGE_SIZE: i64 = 1000;
const MAX_EXPORT_ROWS: i64 = 500_000;

/// The `exportLeaderboardParticipants` job data.
#[derive(Debug, Clone)]
pub struct ExportJob {
    pub workspace_id: i64,
    pub leaderboard_id: i64,
    pub file_id: i64,
    pub output_path: String,
    pub include_contact_info: bool,
}

fn export_header(with_contact: bool) -> Vec<CsvCell> {
    let mut cells = vec!["Rank", "Contact ID", "Name"];
    if with_contact {
        cells.extend(["Phone", "Email"]);
    }
    cells.extend(["Score", "Banned", "Breakdown", "Joined at", "Last activity"]);
    cells.into_iter().map(CsvCell::from).collect()
}

/// `formatBreakdown`: `rule: count; ...` of the counts above zero.
fn format_breakdown(breakdown: &Value) -> String {
    breakdown
        .as_object()
        .map(|map| {
            map.iter()
                .filter(|(_, n)| n.as_f64().is_some_and(|n| n > 0.0))
                .map(|(rule, n)| format!("{rule}: {n}"))
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default()
}

/// `buildCells`: banned participants have no rank.
fn export_cells(
    row: &participants::ExportRow,
    rank: Option<i64>,
    with_contact: bool,
) -> Vec<CsvCell> {
    let mut cells = vec![
        CsvCell::from(rank),
        CsvCell::from(row.contact_id.to_string()),
        CsvCell::from(row.full_name.clone()),
    ];
    if with_contact {
        cells.push(CsvCell::from(row.phone_number.clone()));
        cells.push(CsvCell::from(row.email.clone()));
    }
    cells.extend([
        CsvCell::from(row.score),
        CsvCell::from(if row.is_banned { "yes" } else { "no" }),
        CsvCell::from(format_breakdown(&row.breakdown.0)),
        CsvCell::from(iso(row.created_at)),
        CsvCell::from(iso(row.last_activity_at)),
    ]);
    cells
}

/// The CSV body of every participant in ranking order (banned last) and its row count.
pub async fn export_csv(db: &Db, job: &ExportJob) -> Result<(String, i64)> {
    let (id, with_contact) = (job.leaderboard_id, job.include_contact_info);
    tx::read(db, move |conn| {
        let mut body = format!(
            "{UTF8_BOM}{}",
            to_csv_row(&export_header(with_contact), CsvDialect::Always)
        );
        let (mut rank, mut total, mut offset) = (0, 0, 0);
        while offset < MAX_EXPORT_ROWS {
            let rows = participants::export_page(conn, id, EXPORT_PAGE_SIZE, offset)?;
            for row in &rows {
                if !row.is_banned {
                    rank += 1;
                }
                body.push_str(&to_csv_row(
                    &export_cells(row, (!row.is_banned).then_some(rank), with_contact),
                    CsvDialect::Always,
                ));
            }
            total += rows.len() as i64;
            if (rows.len() as i64) < EXPORT_PAGE_SIZE {
                break;
            }
            offset += EXPORT_PAGE_SIZE;
        }
        Ok((body, total))
    })
    .await
}

async fn update_export_file(
    db: &Db,
    job: &ExportJob,
    uploaded: Option<usize>,
    total: i64,
) -> Result<()> {
    let (id, ws) = (job.file_id, job.workspace_id);
    let status = if uploaded.is_some() {
        "uploaded"
    } else {
        "failed"
    };
    let size = uploaded.and_then(|s| i64::try_from(s).ok());
    let meta = json!({ "totalRecords": total }).to_string();
    tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"UPDATE "File" SET "status" = ?1, "meta" = ?2, "fileSize" = COALESCE(?3, "fileSize"),
               "uploadedAt" = CASE WHEN ?3 IS NULL THEN "uploadedAt" ELSE ?4 END, "updatedAt" = ?4
               WHERE "id" = ?5 AND "workspaceId" = ?6"#,
            params![status, meta, size, now, id, ws],
        )?;
        Ok(())
    })
    .await
}

/// `exportLeaderboardParticipants`: writes the CSV and marks the File
/// `uploaded` (`failed` on any error, which is returned).
pub async fn run_export(
    db: &Db,
    uploader: &chatbotx_files::Uploader,
    job: &ExportJob,
) -> Result<()> {
    let (body, total) = match export_csv(db, job).await {
        Ok(built) => built,
        Err(error) => {
            update_export_file(db, job, None, 0).await?;
            return Err(error);
        }
    };
    let size = body.len();
    let options = chatbotx_files::PutOptions {
        content_type: Some("text/csv; charset=utf-8".into()),
        ..chatbotx_files::PutOptions::default()
    };
    if let Err(error) = uploader
        .put_object(&job.output_path, body.into_bytes(), options, None)
        .await
    {
        update_export_file(db, job, None, total).await?;
        return Err(BusinessError::exception(
            error.to_string(),
            "systemError",
            500,
        ));
    }
    update_export_file(db, job, Some(size), total).await
}
