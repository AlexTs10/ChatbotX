//! `apps/worker/src/leaderboard` (the `leaderboard` queue and the follower
//! check), the award sources of `integration/handlers/leaderboard-sources.ts`
//! and `channel-audience-event.ts`, the "Change gamification scores" step,
//! the leaderboard crons and the participant export job.

use std::sync::Arc;

use chatbotx_business::leaderboard::scoring::{self, Award, FlowChange, FlowStatus, FollowerCheck};
use chatbotx_business::leaderboard::service;
use chatbotx_db::repo::models::{ContactInbox, Leaderboard, Message};
use chatbotx_jobs::{JobCtx, JobError};
use chatbotx_kv::cas::CasStore as _;
use serde_json::{Value, json};

use crate::channel::{InboundInbox, IncomingContact};
use crate::flow::StepStatus;
use crate::steps::StepCtx;
use crate::{Deps, db_fail, id_of, str_of};

/// A follower answer is reused for 10 minutes per (inbox, contact[, chat]).
const FOLLOWER_CACHE_MS: u64 = 10 * 60 * 1000;
/// How long a handled Telegram (un)block is remembered.
const BLOCK_EVENT_DEDUPE_MS: u64 = 24 * 60 * 60 * 1000;
/// Channels whose comments feed "comments" / "liveComments".
pub const COMMENT_CHANNELS: &[&str] = &["messenger", "instagram", "tiktok", "threads"];
pub const CHANNEL_AUDIENCE_EVENT: &str = "channelAudienceEvent";
pub const EXPORT_PARTICIPANTS: &str = service::EXPORT_PARTICIPANTS;

fn award_of(data: &Value) -> Option<Award> {
    let event_at = str_of(data, "eventAt").and_then(chatbotx_db::repo::parse_iso)?;
    Some(Award {
        workspace_id: id_of(&data["workspaceId"])?,
        contact_id: id_of(&data["contactId"])?,
        inbox_id: id_of(&data["inboxId"]),
        minigame_id: id_of(&data["minigameId"]),
        rule: str_of(data, "rule")?.to_owned(),
        source_ref: str_of(data, "sourceRef")?.to_owned(),
        event_at,
        context: data.get("context").cloned().unwrap_or(Value::Null),
    })
}

/// `processLeaderboardJob` behind the blocked-owner guard.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("").to_owned();
    let data = &job.data["data"];
    let workspace = data["workspaceId"].as_str().unwrap_or("").to_owned();
    if crate::is_blocked_workspace(&deps.db, &workspace).await? {
        return Ok(Value::Null);
    }
    let failed = |e: chatbotx_business::errors::BusinessError| crate::fail(e.to_string());
    match kind.as_str() {
        service::AWARD | service::REVERSE => {
            let Some(award) = award_of(data) else {
                return Err(JobError::unrecoverable(format!("{kind}: bad data")));
            };
            if !chatbotx_business::leaderboard::RULE_KEYS.contains(&award.rule.as_str()) {
                tracing::warn!(rule = award.rule, "unknown leaderboard rule");
                return Ok(Value::Null);
            }
            if kind == service::AWARD {
                let check = award.inbox_id.map(|inbox_id| Follower {
                    deps: deps.clone(),
                    workspace_id: award.workspace_id,
                    contact_id: award.contact_id,
                    inbox_id,
                });
                scoring::award(
                    &deps.db,
                    &award,
                    check.as_ref().map(|c| c as &dyn FollowerCheck),
                )
                .await
                .map_err(failed)?;
            } else {
                scoring::reverse(&deps.db, &award).await.map_err(failed)?;
            }
        }
        // ponytail: rankings are read from SQLite, so there is no ranking
        // cache to rebuild (DECISIONS 2026-11-19).
        service::REBUILD_RANK => {}
        service::TRIGGER_REWARDS => {
            let (Some(ws), Some(id)) = (id_of(&data["workspaceId"]), id_of(&data["leaderboardId"]))
            else {
                return Err(JobError::unrecoverable("triggerRewards: bad data"));
            };
            let deps_ = service::Deps {
                db: &deps.db,
                jobs: Some(Arc::clone(&deps.jobs)),
            };
            let result = service::trigger_rewards(&deps_, ws, id, false)
                .await
                .map_err(failed)?;
            let failures = result["failed"].as_array().map_or(0, Vec::len);
            tracing::info!(leaderboard_id = id, sent = %result["sent"], failed = failures, "leaderboard rewards sent");
            if failures > 0 {
                // The retry sends only the winners not recorded yet.
                return Err(crate::fail(format!(
                    "Leaderboard {id}: {failures} reward flow(s) failed to queue"
                )));
            }
        }
        _ => tracing::warn!(kind, "Unhandled leaderboard job type"),
    }
    Ok(Value::Null)
}

/// `buildLeaderboardFollowerCheck`: Instagram followers, Zalo OA followers,
/// Telegram group/channel members. A failed channel call counts as a
/// follower (and is not cached).
struct Follower {
    deps: Deps,
    workspace_id: i64,
    contact_id: i64,
    inbox_id: i64,
}

impl Follower {
    async fn cached(
        &self,
        key: String,
        check: impl AsyncFnOnce() -> Result<bool, String>,
    ) -> Result<bool, String> {
        let kv = crate::inbound::kv(&self.deps);
        let key = format!("leaderboard:follower:{key}");
        if let Ok(Some(hit)) = kv.get_json(&key).await
            && let Some(hit) = hit.as_bool()
        {
            return Ok(hit);
        }
        let answer = check().await?;
        let _ = kv
            .set_if_absent(&key, &json!(answer), FOLLOWER_CACHE_MS)
            .await;
        Ok(answer)
    }

    async fn check(&self, board: &Leaderboard) -> Result<bool, String> {
        let (inbox, contact) = (self.inbox_id, self.contact_id);
        let found = self
            .deps
            .db
            .read(move |conn| {
                Ok(chatbotx_db::rusqlite::OptionalExtension::optional(
                    conn.query_row(
                        &format!(
                            r#"{} WHERE "inboxId" = ?1 AND "contactId" = ?2 LIMIT 1"#,
                            ContactInbox::select_sql()
                        ),
                        chatbotx_db::rusqlite::params![inbox, contact],
                        ContactInbox::from_row,
                    ),
                )?)
            })
            .await
            .map_err(|e| e.to_string())?;
        let Some(contact_inbox) = found.filter(|ci| !ci.source_id.is_empty()) else {
            return Ok(true);
        };
        let key = format!("{inbox}:{contact}");
        match board.platform.as_str() {
            // ponytail: the live Instagram profile snapshot is not ported; the
            // stored `followsBusiness` answers (unknown = follower, as Instagram).
            "instagram" => Ok(contact_inbox.follows_business != Some(false)),
            "zalo" => {
                self.cached(key, async || {
                    let (_, ctx) = self.context(&contact_inbox).await?;
                    let token = ctx.auth["tokens"]["accessToken"].as_str().unwrap_or("");
                    let detail = crate::channels::zalo::Zalo::new()
                        .get_user_detail(token, &contact_inbox.source_id, None)
                        .await
                        .map_err(|e| e.message)?;
                    Ok(detail["user_is_follower"] != json!(false))
                })
                .await
            }
            "telegram" => {
                let chats: Vec<String> = board.scope.0["telegramChatIds"]
                    .as_array()
                    .map(|ids| {
                        ids.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                if chats.is_empty() {
                    return Ok(true);
                }
                let (_, ctx) = self.context(&contact_inbox).await?;
                let token = ctx.auth["secretText"].as_str().unwrap_or("").to_owned();
                for chat in chats {
                    let member = self
                        .cached(format!("{key}:{chat}"), async || {
                            crate::channels::telegram::Telegram::new()
                                .chat_member_status(&token, &chat, &contact_inbox.source_id)
                                .await
                                .map_err(|e| e.message)
                        })
                        .await?;
                    if member {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(true),
        }
    }

    async fn context(
        &self,
        contact_inbox: &ContactInbox,
    ) -> Result<(Arc<dyn crate::channel::Channel>, crate::channel::ChannelCtx), String> {
        self.deps
            .channels
            .resolve(
                &self.deps.db,
                self.deps.realtime.as_ref(),
                self.workspace_id,
                contact_inbox,
            )
            .await
            .map_err(|e| e.to_string())
    }
}

impl FollowerCheck for Follower {
    fn follows(&self, board: &Leaderboard) -> chatbotx_business::events::BoxFuture<'_, bool> {
        let board = board.clone();
        Box::pin(async move {
            match self.check(&board).await {
                Ok(answer) => answer,
                Err(err) => {
                    tracing::warn!(
                        err,
                        leaderboard_id = board.id,
                        inbox_id = self.inbox_id,
                        contact_id = self.contact_id,
                        "leaderboard follower check failed; counting the contact as a follower"
                    );
                    true
                }
            }
        })
    }
}

/// `enqueueLeaderboardAward` / `enqueueLeaderboardReverse` (never fails).
pub(crate) async fn enqueue(deps: &Deps, reverse: bool, award: Award) {
    service::enqueue(deps.jobs.as_ref(), &deps.db, reverse, &award).await;
}

fn award(
    inbox: &InboundInbox,
    contact_id: i64,
    rule: &str,
    source_ref: String,
    event_at: i64,
) -> Award {
    Award {
        workspace_id: inbox.workspace_id,
        contact_id,
        inbox_id: Some(inbox.inbox_id),
        minigame_id: None,
        rule: rule.to_owned(),
        source_ref,
        event_at,
        context: Value::Null,
    }
}

/// Instagram delivers a quick emoji reaction to a story as a story reply
/// whose text is only the emoji.
pub fn is_emoji_only_text(text: Option<&str>) -> bool {
    let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else {
        return false;
    };
    let pictographic = |c: char| matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2300..=0x23FF | 0x2B00..=0x2BFF | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x21AA);
    // Emoji components: ZWJ, variation selectors, skin tones, keycaps, tags.
    let component = |c: char| {
        c.is_whitespace()
            || matches!(c as u32, 0x200D | 0xFE0E | 0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0x1F1E6..=0x1F1FF | 0xE0020..=0xE007F | 0x23 | 0x2A | 0x30..=0x39)
    };
    text.chars().all(|c| pictographic(c) || component(c)) && text.chars().any(pictographic)
}

fn story_reply(attributes: &Value) -> bool {
    let story = if attributes["type"] == "story_reply" {
        &attributes["story"]
    } else {
        &attributes["storyReply"]
    };
    story.is_object()
}

/// `awardInboundMessageLeaderboard`: Zalo messages, click-to-WhatsApp ad
/// clicks, Instagram story mentions and quick story reactions of a NEW
/// inbound message.
pub(crate) async fn award_inbound_message(
    deps: &Deps,
    inbox: &InboundInbox,
    contact_id: i64,
    message: &Message,
    referral: Option<&Value>,
) {
    if message.message_type != "incoming" {
        return;
    }
    let source_ref = message
        .source_id
        .clone()
        .unwrap_or_else(|| message.id.to_string());
    let at = message.created_at;
    let attributes = message
        .content_attributes
        .as_ref()
        .map_or(&Value::Null, |a| &a.0);
    let rule = match inbox.channel.as_str() {
        "zalo" => Some(("message", source_ref)),
        "whatsapp" => referral
            .filter(|r| r["ctwaClid"].is_string() || r["adId"].is_string())
            .map(|r| {
                let clid = r["ctwaClid"].as_str().map_or(source_ref, str::to_owned);
                ("ctwaReactions", format!("ctwa:{clid}"))
            }),
        "instagram" if attributes["type"] == "story_mention" => Some(("storyMention", source_ref)),
        "instagram" if story_reply(attributes) && is_emoji_only_text(message.text.as_deref()) => {
            Some(("storyReaction", source_ref))
        }
        _ => None,
    };
    if let Some((rule, source_ref)) = rule {
        enqueue(deps, false, award(inbox, contact_id, rule, source_ref, at)).await;
    }
}

/// `awardReactionLeaderboard`: a NEW WhatsApp reaction scores once per
/// reacted message.
pub(crate) async fn award_reaction(
    deps: &Deps,
    inbox: &InboundInbox,
    contact_id: i64,
    reaction_source_id: &str,
    action: &str,
    event_at: i64,
) {
    if inbox.channel != "whatsapp" || action != "react" {
        return;
    }
    let input = award(
        inbox,
        contact_id,
        "ctwaReactions",
        reaction_source_id.to_owned(),
        event_at,
    );
    enqueue(deps, false, input).await;
}

/// `awardCommentLeaderboard`: "comments" / "liveComments" of a NEW
/// top-level comment.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn award_comment(
    deps: &Deps,
    inbox: &InboundInbox,
    contact_id: i64,
    comment_id: &str,
    post_id: &str,
    normalized_post_id: &str,
    text: Option<&str>,
    is_live: bool,
    event_at: i64,
) {
    if !COMMENT_CHANNELS.contains(&inbox.channel.as_str()) {
        return;
    }
    let rule = if is_live { "liveComments" } else { "comments" };
    let mut context = json!({
        "postId": normalized_post_id,
        "textLength": text.map_or(0, |t| t.encode_utf16().count()),
    });
    if !is_live && service::has_rule(&deps.db, Some(inbox.inbox_id), None, rule).await {
        let (ws, inbox_id, post) = (inbox.workspace_id, inbox.inbox_id, post_id.to_owned());
        let published: Option<i64> = deps
            .db
            .read(move |conn| {
                Ok(chatbotx_db::rusqlite::OptionalExtension::optional(conn.query_row(
                    r#"SELECT "publishedAt" FROM "ChannelPost" WHERE "workspaceId" = ?1 AND "inboxId" = ?2 AND "externalPostId" = ?3 LIMIT 1"#,
                    chatbotx_db::rusqlite::params![ws, inbox_id, post],
                    |r| r.get::<_, Option<i64>>(0),
                ))?
                .flatten())
            })
            .await
            .unwrap_or(None);
        if let Some(at) = published {
            context["postPublishedAt"] = json!(chatbotx_db::repo::iso(at));
        }
    }
    let mut input = award(inbox, contact_id, rule, comment_id.to_owned(), event_at);
    input.context = context;
    enqueue(deps, false, input).await;
}

/// The "Change gamification scores" step. A paused/finished/deleted
/// leaderboard blocks bot scoring by design: the step then does nothing.
pub(crate) async fn change_score_step(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let step = ctx.step;
    let user = step["targetUser"] == true;
    let referrer = step["targetReferrer"] == true;
    let Some(leaderboard_id) = id_of(&step["leaderboardId"]).filter(|_| user || referrer) else {
        return Ok(StepStatus::Success);
    };
    let run = ctx.run;
    let key = &run.flow_execution_key;
    let at = str_of(run.data, "triggerMessageCreatedAt").and_then(chatbotx_db::repo::parse_iso);
    let input = FlowChange {
        workspace_id: ctx.workspace_id(),
        leaderboard_id,
        contact_id: ctx.contact_id(),
        subtract: ctx.str("operation") == Some("subtract"),
        points: step["points"].as_f64().unwrap_or(0.0),
        user,
        referrer,
        // A retried flow job reuses its execution key, so the step never scores twice.
        dedup: (!key.is_empty()).then(|| (format!("{key}:{}", ctx.str("id").unwrap_or("")), at)),
    };
    let (status, _) = scoring::change_from_flow(&run.deps.db, input)
        .await
        .map_err(|e| crate::fail(e.to_string()))?;
    if status != FlowStatus::Applied {
        tracing::info!(leaderboard_id, ?status, "changeGamificationScore skipped");
    }
    Ok(StepStatus::Success)
}

/// The cross-workspace leaderboard crons (`leaderboard-schedules.ts`).
pub(crate) async fn run_schedule(deps: &Deps, name: &str) -> bool {
    match name {
        "finishOverdueLeaderboards" => {
            let deps_ = service::Deps {
                db: &deps.db,
                jobs: Some(Arc::clone(&deps.jobs)),
            };
            match service::finish_overdue(&deps_, chatbotx_db::repo::now_ms()).await {
                Ok(0) => {}
                Ok(finished) => tracing::info!(finished, "[finishOverdueLeaderboards] finished"),
                Err(err) => tracing::error!(err = %err, "[finishOverdueLeaderboards] failed"),
            }
            true
        }
        // ponytail: SQLite rankings need no Redis upkeep and have no
        // partitions (DECISIONS 2026-11-19).
        "maintainLeaderboardRankings" | "maintainLeaderboardPartitions" => true,
        _ => false,
    }
}

/// `exportLeaderboardParticipants` (default queue).
pub(crate) async fn export_participants(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let bad = || JobError::unrecoverable("exportLeaderboardParticipants: bad data");
    let job = service::ExportJob {
        workspace_id: id_of(&data["workspaceId"]).ok_or_else(bad)?,
        leaderboard_id: id_of(&data["leaderboardId"]).ok_or_else(bad)?,
        file_id: id_of(&data["fileId"]).ok_or_else(bad)?,
        output_path: str_of(data, "outputPath").ok_or_else(bad)?.to_owned(),
        include_contact_info: data["includeContactInfo"] == true,
    };
    let Some(files) = deps.files.as_ref() else {
        return Err(JobError::unrecoverable(
            "exportLeaderboardParticipants: no file store",
        ));
    };
    service::run_export(&deps.db, files, &job)
        .await
        .map_err(|e| crate::fail(e.to_string()))
}

fn incoming_contact(user: &Value) -> IncomingContact {
    let text = |key: &str| {
        str_of(user, key)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    IncomingContact {
        source_id: str_of(user, "sourceId").unwrap_or("").to_owned(),
        first_name: text("firstName"),
        last_name: text("lastName"),
        source_username: text("username"),
        ..IncomingContact::default()
    }
}

async fn existing_contact_inbox(
    deps: &Deps,
    inbox: &InboundInbox,
    user: &Value,
) -> Result<Option<ContactInbox>, JobError> {
    let (probe, contact) = (inbox.clone(), incoming_contact(user));
    deps.db
        .read(move |conn| {
            Ok(crate::inbound::find_existing_contact_inbox(
                conn, &probe, &contact,
            )?)
        })
        .await
        .map(|found| found.map(|(ci, _)| ci))
        .map_err(db_fail)
}

/// `resolveBotBlockTransition`.
pub fn bot_block_transition(old_status: Option<&str>, new_status: &str) -> Option<&'static str> {
    let active = |s: &str| crate::channels::telegram::is_member_status(s, None);
    let old = old_status?;
    if new_status == "kicked" && active(old) {
        return Some("blocked");
    }
    if old == "kicked" && active(new_status) {
        return Some("unblocked");
    }
    None
}

/// `handleTelegramPrivateBotStatus`: a user (un)blocked the bot in their
/// private chat; fires the matching trigger for the existing contact once.
async fn private_bot_status(
    deps: &Deps,
    inbox: &InboundInbox,
    event: &Value,
) -> Result<(), JobError> {
    let Some(transition) = bot_block_transition(
        str_of(event, "oldStatus"),
        str_of(event, "status").unwrap_or(""),
    ) else {
        return Ok(());
    };
    let user_source_id = str_of(&event["user"], "sourceId")
        .or_else(|| str_of(&event["chat"], "id"))
        .unwrap_or("")
        .to_owned();
    let Some(existing) =
        existing_contact_inbox(deps, inbox, &json!({ "sourceId": user_source_id })).await?
    else {
        return Ok(());
    };
    let occurred = str_of(event, "occurredAt").unwrap_or("");
    let key = format!(
        "telegram:bot-block:{}:{user_source_id}:{occurred}:{transition}",
        inbox.inbox_id
    );
    let claimed = crate::inbound::kv(deps)
        .set_if_absent(&key, &json!("1"), BLOCK_EVENT_DEDUPE_MS)
        .await
        .map_err(|e| crate::fail(e.to_string()))?;
    if !claimed {
        return Ok(());
    }
    let event_type = if transition == "blocked" {
        "contactBlockedBot"
    } else {
        "contactUnblockedBot"
    };
    let mut metadata = serde_json::Map::new();
    metadata.insert("channel".into(), json!(inbox.channel));
    let metadata = chatbotx_business::events::contact_inbox::with_contact_inbox_metadata(
        Some(metadata),
        Some(&existing.id.to_string()),
    )
    .unwrap_or_default();
    let trigger = chatbotx_business::events::TriggerEvent {
        event_type: event_type.into(),
        workspace_id: inbox.workspace_id.to_string(),
        contact_id: existing.contact_id.to_string(),
        metadata,
    };
    // The user's own action on the channel: a channel-originated event.
    let bus = crate::order_payment::bus(deps);
    chatbotx_business::events::context::with_webhook_execution_context(
        chatbotx_business::events::context::WebhookExecutionContext::webhook(),
        bus.publish(chatbotx_business::events::Event::Trigger(trigger)),
    )
    .await;
    Ok(())
}

/// `processChannelAudienceEvent`: follow/unfollow, group join/leave, group
/// messages and the bot's own chat status. Nothing is stored in a
/// conversation and no automation runs.
pub(crate) async fn channel_audience_event(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let integration_type = str_of(data, "integrationType").unwrap_or("");
    let identifier = match &data["integrationIdentifier"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let Some((_, _, inbox)) = deps
        .channels
        .resolve_by_identifier(
            &deps.db,
            deps.realtime.as_ref(),
            integration_type,
            &identifier,
        )
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?
    else {
        return Err(JobError::unrecoverable(format!(
            "Integration not found: {integration_type} {identifier}"
        )));
    };
    let event = &data["event"];
    let kind = str_of(event, "kind").unwrap_or("");
    if kind == "botChatStatus" {
        // A private chat is one user (un)blocking the bot, not a group.
        if str_of(&event["chat"], "type") == Some("private") {
            return private_bot_status(deps, &inbox, event).await;
        }
        let status = str_of(event, "status").unwrap_or("").to_owned();
        service::record_telegram_chat(
            &deps.db,
            inbox.workspace_id,
            inbox.inbox_id,
            &event["chat"],
            &status,
        )
        .await
        .map_err(|e| crate::fail(e.to_string()))?;
        return Ok(());
    }
    let rule = match kind {
        "follow" | "unfollow" => "follow",
        "groupMessage" => "groupDiscussion",
        "groupJoin" | "groupLeave" => "joinGroup",
        _ => {
            tracing::warn!(kind, "Unhandled audience event kind");
            return Ok(());
        }
    };
    // Check that a live leaderboard scores the rule BEFORE resolving the
    // contact, so a busy group does not mint a contact per member.
    if !service::has_rule(&deps.db, Some(inbox.inbox_id), None, rule).await {
        return Ok(());
    }
    let undo = matches!(kind, "unfollow" | "groupLeave");
    let contact_id = if undo {
        existing_contact_inbox(deps, &inbox, &event["user"])
            .await?
            .map(|ci| ci.contact_id)
    } else {
        let source = if kind == "groupMessage" {
            "inboundMessage"
        } else {
            "direct"
        };
        let (ci, _, _) = crate::inbound::detect_contact_and_conversation(
            deps,
            &inbox,
            &incoming_contact(&event["user"]),
            source,
        )
        .await?;
        Some(ci.contact_id)
    };
    let Some(contact_id) = contact_id else {
        return Ok(());
    };
    let event_at = str_of(event, "occurredAt")
        .and_then(chatbotx_db::repo::parse_iso)
        .unwrap_or_else(chatbotx_db::repo::now_ms);
    let toggle = format!(
        "{}@{event_at}",
        str_of(&event["user"], "sourceId").unwrap_or("")
    );
    let chat = str_of(event, "chatId").unwrap_or("");
    let (source_ref, context) = match kind {
        "follow" | "unfollow" => (toggle, Value::Null),
        "groupJoin" | "groupLeave" => (format!("{chat}:{toggle}"), json!({ "chatId": chat })),
        _ => (
            format!("{chat}:{}", str_of(event, "messageId").unwrap_or("")),
            json!({ "chatId": chat, "textLength": event["textLength"] }),
        ),
    };
    let mut input = award(&inbox, contact_id, rule, source_ref, event_at);
    input.context = context;
    enqueue(deps, undo, input).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_only_text_needs_a_pictograph_and_nothing_else() {
        assert!(is_emoji_only_text(Some("🔥")));
        assert!(is_emoji_only_text(Some(" ❤️ 👍🏽 ")));
        assert!(is_emoji_only_text(Some("👨‍👩‍👧")));
        assert!(!is_emoji_only_text(Some("nice 🔥")));
        assert!(!is_emoji_only_text(Some("123")));
        assert!(!is_emoji_only_text(Some("  ")));
        assert!(!is_emoji_only_text(None));
    }

    #[test]
    fn bot_block_transitions_match_telegram_statuses() {
        assert_eq!(
            bot_block_transition(Some("member"), "kicked"),
            Some("blocked")
        );
        assert_eq!(
            bot_block_transition(Some("kicked"), "member"),
            Some("unblocked")
        );
        assert_eq!(bot_block_transition(None, "member"), None);
        assert_eq!(bot_block_transition(Some("left"), "kicked"), None);
        assert_eq!(bot_block_transition(Some("member"), "member"), None);
    }

    #[test]
    fn award_job_data_round_trips() {
        let data = json!({
            "workspaceId": "1", "contactId": "2", "inboxId": "3", "rule": "comments",
            "sourceRef": "c1", "eventAt": "2026-01-01T00:00:00.000Z", "context": { "postId": "p" },
        });
        let award = award_of(&data).unwrap();
        assert_eq!(
            (award.workspace_id, award.contact_id, award.inbox_id),
            (1, 2, Some(3))
        );
        assert_eq!(award.minigame_id, None);
        assert_eq!(award.context["postId"], "p");
        assert!(award_of(&json!({ "workspaceId": "1" })).is_none());
    }
}
