//! Conversation, message, agent and bot-message analytics (Q.D11 slice 8)
//! on a migrated in-memory database. The seeds and expectations are the
//! ones of the parity group p1-03 (`analytics-conversations`,
//! `analytics-agents`, `analytics-bot-messages`); per-admin rows come in
//! member order here and are compared as maps where the TS tests sort them.
//! Ports `agent-response-stats.repository`, `human-agent-stats-response`
//! and `filtered-stats.repository` (conversation / message parts) of
//! `packages/analytics/__tests__`: those read the SQL of a mocked driver;
//! here the same cases seed rows and check the result.

use std::collections::BTreeMap;
use std::sync::Arc;

use chatbotx_business::analytics::bot_messages::{self, BotRow};
use chatbotx_business::analytics::conversations::{self, DayCount};
use chatbotx_business::analytics::messages::{self, ResponseAverages};
use chatbotx_business::analytics::{Filter, TimeRange};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, params};

const WS_CONV: i64 = 601;
const WS_AGENTS: i64 = 602;
const WS_SLA: i64 = 603;
const WS_BOT: i64 = 604;
const OWNER: i64 = 1;
const AGENT: i64 = 2;
const IDLE: i64 = 3;
const FORMER: i64 = 99;
const I1: i64 = 11; // messenger
const I2: i64 = 12; // telegram

fn ms(iso: &str) -> i64 {
    chatbotx_core::js::parse_date(iso).expect("iso")
}

fn day(key: &str) -> String {
    format!("{key}T00:00:00.000Z")
}

fn inbox(channel: &str) -> (Option<&str>, Option<i64>) {
    match channel {
        "messenger" => (Some("messenger"), Some(I1)),
        "telegram" => (Some("telegram"), Some(I2)),
        _ => (None, None),
    }
}

fn conversation(
    tx: &Connection,
    ws: i64,
    conv: i64,
    kind: &str,
    at: &str,
    to: Option<i64>,
    channel: &str,
) {
    let (channel, inbox) = inbox(channel);
    tx.execute(
        r#"INSERT INTO "AnalyticsConversationEvent" ("eventId", "workspaceId", "conversationId", "eventType", "occurredAt", "toAssignee", "channel", "inboxId")
           VALUES (lower(hex(randomblob(8))), ?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
        params![ws, conv, kind, ms(at), to, channel, inbox],
    )
    .unwrap();
}

fn message(
    tx: &Connection,
    contact: i64,
    kind: &str,
    at: &str,
    sender: Option<&str>,
    admin: Option<i64>,
    channel: &str,
) {
    let (channel, inbox) = inbox(channel);
    tx.execute(
        r#"INSERT INTO "AnalyticsMessageEvent" ("eventId", "workspaceId", "contactId", "eventType", "occurredAt", "senderType", "adminId", "channel", "inboxId")
           VALUES (lower(hex(randomblob(8))), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        params![WS_AGENTS, contact, kind, ms(at), sender, admin, channel, inbox],
    )
    .unwrap();
}

/// `seedResponses` row: (admin, at, responseSeconds, working, first, slaMet, channel).
type Response = (
    i64,
    &'static str,
    i64,
    Option<i64>,
    bool,
    Option<bool>,
    &'static str,
);

fn responses(tx: &Connection, ws: i64) {
    let rows: [Response; 5] = [
        (
            OWNER,
            "2025-04-01T10:00:00Z",
            100,
            Some(60),
            true,
            Some(true),
            "messenger",
        ),
        (
            OWNER,
            "2025-04-01T11:00:00Z",
            301,
            None,
            false,
            Some(false),
            "messenger",
        ),
        (
            AGENT,
            "2025-04-02T09:00:00Z",
            50,
            Some(50),
            true,
            None,
            "telegram",
        ),
        // 04-02 in UTC, 04-03 in Ho Chi Minh City
        (
            OWNER,
            "2025-04-02T20:00:00Z",
            30,
            Some(30),
            false,
            Some(true),
            "messenger",
        ),
        // an admin who is no longer a member: dropped from every per-admin read
        (
            FORMER,
            "2025-04-01T12:00:00Z",
            10,
            None,
            false,
            None,
            "messenger",
        ),
    ];
    for (i, (admin, at, seconds, working, first, met, channel)) in rows.into_iter().enumerate() {
        let (channel, inbox) = inbox(channel);
        let at = ms(at);
        tx.execute(
            r#"INSERT INTO "AnalyticsAgentResponseEvent" ("eventId", "workspaceId", "conversationId", "contactId", "adminId", "occurredAt", "awaitingSince", "measuredFrom", "responseSeconds", "workingResponseSeconds", "isFirstResponse", "slaTargetSeconds", "slaMet", "channel", "inboxId")
               VALUES (lower(hex(randomblob(8))), ?1, ?2, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
            params![ws, 1000 + i as i64, admin, at, at - seconds * 1000, seconds, working, first, met.map(|_| 300), met, channel, inbox],
        )
        .unwrap();
    }
}

#[allow(clippy::too_many_arguments)] // one column per argument, as the parity fixture rows
fn bot(
    tx: &Connection,
    id: i64,
    at: &str,
    answered: bool,
    kind: &str,
    route: Option<&str>,
    result: Option<&str>,
    provider: Option<&str>,
    channel: &str,
) {
    let (channel, inbox) = inbox(channel);
    tx.execute(
        r#"INSERT INTO "AnalyticsBotMessageEvent" ("eventId", "workspaceId", "messageId", "conversationId", "occurredAt", "hasResponse", "responseType", "routeType", "result", "aiProvider", "channel", "inboxId")
           VALUES (lower(hex(randomblob(8))), ?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
        params![WS_BOT, id, ms(at), answered, kind, route, result, provider, channel, inbox],
    )
    .unwrap();
}

async fn seeded() -> Arc<Db> {
    let db = Db::open_in_memory().unwrap();
    db.migrate().await.unwrap();
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email", "name") VALUES
               (1, 'owner@x.test', 'Owner'), (2, 'agent@x.test', 'Agent One'), (3, 'idle@x.test', 'Agent Idle')"#,
            [],
        )?;
        for (ws, members) in [
            (WS_CONV, &[OWNER, AGENT][..]),
            (WS_AGENTS, &[OWNER, AGENT, IDLE][..]),
            (WS_SLA, &[OWNER, AGENT][..]),
            (WS_BOT, &[OWNER][..]),
        ] {
            tx.execute(r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'W', 1)"#, [ws])?;
            for &user in members {
                let role = if user == OWNER { "owner" } else { "agent" };
                tx.execute(
                    r#"INSERT INTO "WorkspaceMember" ("workspaceId", "userId", "role") VALUES (?1, ?2, ?3)"#,
                    params![ws, user, role],
                )?;
            }
        }
        tx.execute(
            r#"INSERT INTO "ResponseSlaPolicy" ("workspaceId", "enabled", "firstResponseTargetSeconds", "nextResponseTargetSeconds") VALUES (?1, 1, 300, 300)"#,
            [WS_SLA],
        )?;

        // analytics-conversations: conversations 1 (v1) and 2 (v2).
        let c = |conv, kind, at, to, channel| conversation(tx, WS_CONV, conv, kind, at, to, channel);
        c(1, "conversation_transferred_to_human", "2025-03-08T15:00:00Z", None, "messenger");
        c(2, "conversation_transferred_to_human", "2025-03-09T04:30:00Z", None, "messenger");
        c(1, "conversation_transferred_to_bot", "2025-03-09T16:00:00Z", None, "telegram");
        c(1, "conversation_followed", "2025-03-10T03:30:00Z", None, "messenger");
        c(2, "conversation_followed", "2025-03-10T12:00:00Z", None, "messenger");
        c(2, "conversation_unfollowed", "2025-03-10T13:00:00Z", None, "messenger");
        c(1, "conversation_archived", "2025-03-11T02:00:00Z", None, "messenger");
        c(1, "conversation_unarchived", "2025-03-11T03:00:00Z", None, "messenger");
        c(1, "conversation_assigned", "2025-03-08T15:00:00Z", Some(OWNER), "messenger");
        c(2, "conversation_assigned", "2025-03-09T16:00:00Z", Some(OWNER), "messenger");
        c(1, "conversation_assigned", "2025-03-10T12:00:00Z", Some(AGENT), "telegram");
        c(1, "conversation_assigned", "2025-03-10T13:00:00Z", Some(OWNER), "messenger");
        c(2, "conversation_unassigned", "2025-03-10T14:00:00Z", None, "messenger");
        c(2, "conversation_assigned", "2025-03-01T00:00:00Z", Some(AGENT), "messenger");

        // analytics-agents: contacts 1..3.
        let h = Some("human");
        let b = Some("bot");
        message(tx, 1, "message_human_sent", "2025-04-01T10:00:00Z", h, Some(OWNER), "messenger");
        message(tx, 2, "message_human_sent", "2025-04-01T11:00:00Z", h, Some(OWNER), "messenger");
        message(tx, 1, "message_human_sent", "2025-04-02T09:00:00Z", h, Some(AGENT), "telegram");
        message(tx, 1, "message_bot_sent", "2025-04-01T12:00:00Z", b, None, "messenger");
        message(tx, 3, "message_bot_sent", "2025-04-03T23:30:00Z", b, None, "telegram");
        message(tx, 1, "message_human_sent", "2025-03-31T23:00:00Z", h, Some(OWNER), "messenger");
        message(tx, 2, "message_bot_sent", "2025-04-02T00:00:00Z", None, None, "");
        let a = |conv, at, to, channel| conversation(tx, WS_AGENTS, conv, "conversation_assigned", at, Some(to), channel);
        a(1, "2025-04-01T10:00:00Z", OWNER, "messenger");
        a(2, "2025-04-02T10:00:00Z", OWNER, "messenger");
        a(1, "2025-04-03T10:00:00Z", AGENT, "telegram");
        responses(tx, WS_AGENTS);
        responses(tx, WS_SLA);

        // analytics-bot-messages
        let (ai, ok, fb) = (Some("agent"), Some("success"), Some("fallback"));
        bot(tx, 1, "2025-05-01T10:00:00Z", true, "ai_agent", ai, ok, Some("openai"), "messenger");
        bot(tx, 2, "2025-05-01T11:00:00Z", true, "ai_agent", ai, ok, Some("openai"), "messenger");
        bot(tx, 3, "2025-05-02T09:00:00Z", true, "flow", Some("flow"), ok, None, "telegram");
        bot(tx, 4, "2025-05-02T20:00:00Z", true, "ai_agent", Some("fallback"), fb, Some("anthropic"), "messenger");
        bot(tx, 5, "2025-05-03T05:00:00Z", false, "none", None, None, None, "messenger");
        bot(tx, 6, "2025-05-03T06:00:00Z", false, "none", None, None, None, "");
        bot(tx, 7, "2025-05-03T07:00:00Z", true, "automated_response", None, ok, None, "messenger");
        bot(tx, 8, "2025-04-30T12:00:00Z", true, "ai_agent", None, ok, Some("openai"), "messenger");
        Ok(())
    })
    .await
    .unwrap();
    Arc::new(db)
}

fn range(ws: i64, from: &str, to: &str, tz: &str, channels: &[&str]) -> TimeRange {
    let filter = Filter::new(channels.iter().map(|c| c.to_string()).collect(), Vec::new());
    TimeRange::new(ws, Some(ms(from)), Some(ms(to)), tz, filter)
}

async fn read<R: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> R {
    db.read(move |c| f(c).map_err(|e| chatbotx_db::DbError::other(e.to_string())))
        .await
        .unwrap()
}

// ---- analytics-conversations -------------------------------------------

fn conv_utc(channels: &[&str]) -> TimeRange {
    range(
        WS_CONV,
        "2025-03-08T00:00:00Z",
        "2025-03-11T23:59:59Z",
        "UTC",
        channels,
    )
}

/// The New York range crosses the DST change of 2025-03-09.
fn conv_ny() -> TimeRange {
    range(
        WS_CONV,
        "2025-03-08T00:00:00-05:00",
        "2025-03-11T23:59:59-04:00",
        "America/New_York",
        &[],
    )
}

fn days(rows: Vec<DayCount>) -> Vec<(String, Option<String>, i64)> {
    rows.into_iter()
        .map(|r| (r.timestamp, r.direction, r.count))
        .collect()
}

fn d(key: &str, direction: Option<&str>, count: i64) -> (String, Option<String>, i64) {
    (day(key), direction.map(str::to_string), count)
}

#[tokio::test]
async fn handoffs_per_day_and_direction_in_utc_local_days_and_a_channel() {
    let db = seeded().await;
    let utc = read(&db, |c| conversations::handoffs_by_day(c, &conv_utc(&[]))).await;
    assert_eq!(
        days(utc),
        [
            d("2025-03-08", Some("to_human"), 1),
            d("2025-03-09", Some("to_bot"), 1),
            d("2025-03-09", Some("to_human"), 1)
        ]
    );
    let ny = read(&db, |c| conversations::handoffs_by_day(c, &conv_ny())).await;
    assert_eq!(
        days(ny),
        [
            d("2025-03-08", Some("to_human"), 2),
            d("2025-03-09", Some("to_bot"), 1)
        ]
    );
    let telegram = read(&db, |c| {
        conversations::handoffs_by_day(c, &conv_utc(&["telegram"]))
    })
    .await;
    assert_eq!(days(telegram), [d("2025-03-09", Some("to_bot"), 1)]);
}

#[tokio::test]
async fn follow_ups_archived_and_assigned_per_day() {
    let db = seeded().await;
    let by = |kind: &'static str, r: TimeRange| {
        let db = db.clone();
        async move { days(read(&db, move |c| conversations::events_by_day(c, &r, kind)).await) }
    };
    assert_eq!(
        by("conversation_followed", conv_utc(&[])).await,
        [d("2025-03-10", None, 2)]
    );
    assert_eq!(
        by("conversation_followed", conv_ny()).await,
        [d("2025-03-09", None, 1), d("2025-03-10", None, 1)]
    );
    assert_eq!(
        by("conversation_archived", conv_utc(&[])).await,
        [d("2025-03-11", None, 1)]
    );
    assert_eq!(
        by("conversation_archived", conv_ny()).await,
        [d("2025-03-10", None, 1)]
    );
    assert!(
        by("conversation_archived", conv_utc(&["telegram"]))
            .await
            .is_empty()
    );
    let assigned = [
        d("2025-03-08", None, 1),
        d("2025-03-09", None, 1),
        d("2025-03-10", None, 2),
    ];
    assert_eq!(by("conversation_assigned", conv_utc(&[])).await, assigned);
    assert_eq!(by("conversation_assigned", conv_ny()).await, assigned);
}

#[tokio::test]
async fn assignments_and_distinct_conversations_per_current_member() {
    let db = seeded().await;
    let by = |distinct: bool, r: TimeRange| {
        let db = db.clone();
        async move {
            read(&db, move |c| {
                conversations::assigned_by_admin(c, &r, distinct)
            })
            .await
            .into_iter()
            .map(|m| {
                (
                    m.user_id,
                    m.count,
                    m.user_name.unwrap(),
                    m.user_email.unwrap(),
                )
            })
            .collect::<Vec<_>>()
        }
    };
    let owner = |n: i64| (OWNER, n, "Owner".to_string(), "owner@x.test".to_string());
    let agent = |n: i64| {
        (
            AGENT,
            n,
            "Agent One".to_string(),
            "agent@x.test".to_string(),
        )
    };
    assert_eq!(by(false, conv_utc(&[])).await, [owner(3), agent(1)]);
    assert_eq!(by(true, conv_utc(&[])).await, [owner(2), agent(1)]);
    assert_eq!(
        by(true, conv_utc(&["telegram"])).await,
        [owner(0), agent(1)]
    );
}

// ---- analytics-agents --------------------------------------------------

fn agents_utc(ws: i64, channels: &[&str]) -> TimeRange {
    range(
        ws,
        "2025-04-01T00:00:00Z",
        "2025-04-03T23:59:59Z",
        "UTC",
        channels,
    )
}

fn agents_hcm(ws: i64) -> TimeRange {
    range(
        ws,
        "2025-04-01T00:00:00+07:00",
        "2025-04-03T23:59:59+07:00",
        "Asia/Ho_Chi_Minh",
        &[],
    )
}

#[tokio::test]
async fn messages_per_current_member_and_channel() {
    let db = seeded().await;
    let by = |channels: &'static [&'static str]| {
        let db = db.clone();
        async move {
            read(&db, move |c| {
                messages::messages_by_admin(c, &agents_utc(WS_AGENTS, channels))
            })
            .await
            .into_iter()
            .map(|r| (r.admin_id, r.count, r.user_name.unwrap()))
            .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        by(&[]).await,
        [
            (OWNER, 2, "Owner".to_string()),
            (AGENT, 1, "Agent One".to_string()),
            (IDLE, 0, "Agent Idle".to_string())
        ]
    );
    let telegram: Vec<_> = by(&["telegram"])
        .await
        .into_iter()
        .map(|r| (r.0, r.1))
        .collect();
    assert_eq!(telegram, [(OWNER, 0), (AGENT, 1), (IDLE, 0)]);
}

#[tokio::test]
async fn messages_by_sender_per_day_local_day_and_month() {
    let db = seeded().await;
    let by = |r: TimeRange, month: bool| {
        let db = db.clone();
        async move {
            read(&db, move |c| messages::messages_by_sender(c, &r, month))
                .await
                .into_iter()
                .map(|r| (r.timestamp, r.channel, r.sender_type, r.count))
                .collect::<Vec<_>>()
        }
    };
    let row = |key: &str, channel: &str, sender: &str, count: i64| {
        (day(key), channel.to_string(), sender.to_string(), count)
    };
    assert_eq!(
        by(agents_utc(WS_AGENTS, &[]), false).await,
        [
            row("2025-04-01", "messenger", "bot", 1),
            row("2025-04-01", "messenger", "human", 2),
            row("2025-04-02", "telegram", "human", 1),
            row("2025-04-03", "telegram", "bot", 1),
        ]
    );
    assert_eq!(
        by(agents_hcm(WS_AGENTS), false).await,
        [
            row("2025-04-01", "messenger", "bot", 1),
            row("2025-04-01", "messenger", "human", 3),
            row("2025-04-02", "telegram", "human", 1),
        ]
    );
    assert_eq!(
        by(agents_utc(WS_AGENTS, &[]), true).await,
        [
            row("2025-04-01", "messenger", "bot", 1),
            row("2025-04-01", "messenger", "human", 2),
            row("2025-04-01", "telegram", "bot", 1),
            row("2025-04-01", "telegram", "human", 1),
        ]
    );
}

fn averages(avg: [Option<i64>; 4], counts: [i64; 2], sla: [i64; 4]) -> ResponseAverages {
    ResponseAverages {
        avg_response_seconds: avg[0],
        avg_working_response_seconds: avg[1],
        avg_first_response_seconds: avg[2],
        avg_working_first_response_seconds: avg[3],
        response_count: counts[0],
        first_response_count: counts[1],
        response_sla_eligible_count: sla[0],
        response_sla_met_count: sla[1],
        first_response_sla_eligible_count: sla[2],
        first_response_sla_met_count: sla[3],
    }
}

const NO_SLA: [i64; 4] = [0; 4];

fn owner_all(sla: [i64; 4]) -> ResponseAverages {
    averages([Some(144), Some(45), Some(100), Some(60)], [3, 1], sla)
}

fn agent_all() -> ResponseAverages {
    averages([Some(50); 4], [1, 1], NO_SLA)
}

#[tokio::test]
async fn human_agent_stats_merge_messages_assignments_and_responses() {
    let db = seeded().await;
    let by = |r: TimeRange| {
        let db = db.clone();
        async move {
            read(&db, move |c| messages::human_agent_stats(c, &r))
                .await
                .into_iter()
                .map(|s| {
                    (
                        s.admin_id,
                        (
                            s.assigned_conversations,
                            s.messages_sent,
                            s.unique_contacts,
                            s.response,
                        ),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        }
    };
    let none = ResponseAverages::default();
    assert_eq!(
        by(agents_utc(WS_AGENTS, &[])).await,
        BTreeMap::from([
            (OWNER, (2, 2, 2, owner_all(NO_SLA))),
            (AGENT, (1, 1, 1, agent_all())),
            (IDLE, (0, 0, 0, none.clone())),
        ])
    );
    assert_eq!(
        by(agents_utc(WS_AGENTS, &["telegram"])).await,
        BTreeMap::from([
            (OWNER, (0, 0, 0, none.clone())),
            (AGENT, (1, 1, 1, agent_all())),
            (IDLE, (0, 0, 0, none)),
        ])
    );
    // With an enabled SLA policy the outcome counts are reported.
    let sla = by(agents_utc(WS_SLA, &[])).await;
    assert_eq!(
        sla,
        BTreeMap::from([
            (OWNER, (0, 0, 0, owner_all([3, 2, 1, 1]))),
            (AGENT, (0, 0, 0, agent_all()))
        ])
    );
    let owner = &sla[&OWNER].3;
    assert_eq!(owner.response_sla_met_percent(), Some(66.7));
    assert_eq!(owner.first_response_sla_met_percent(), Some(100.0));
    assert_eq!(agent_all().response_sla_met_percent(), None);
}

#[tokio::test]
async fn agent_response_series_per_admin_and_local_day() {
    let db = seeded().await;
    let by = |r: TimeRange| {
        let db = db.clone();
        async move {
            read(&db, move |c| messages::agent_response_series(c, &r))
                .await
                .into_iter()
                .map(|s| (s.admin_id, s.date, s.user_email.unwrap(), s.response))
                .collect::<Vec<_>>()
        }
    };
    let owner_day1 = (
        OWNER,
        "2025-04-01".to_string(),
        "owner@x.test".to_string(),
        averages(
            [Some(201), Some(60), Some(100), Some(60)],
            [2, 1],
            [2, 1, 1, 1],
        ),
    );
    let owner_late = |date: &str| {
        (
            OWNER,
            date.to_string(),
            "owner@x.test".to_string(),
            averages([Some(30), Some(30), None, None], [1, 0], [1, 1, 0, 0]),
        )
    };
    let agent_day = (
        AGENT,
        "2025-04-02".to_string(),
        "agent@x.test".to_string(),
        agent_all(),
    );
    assert_eq!(
        by(agents_utc(WS_SLA, &[])).await,
        [
            owner_day1.clone(),
            owner_late("2025-04-02"),
            agent_day.clone()
        ]
    );
    assert_eq!(
        by(agents_hcm(WS_SLA)).await,
        [
            owner_day1.clone(),
            agent_day.clone(),
            owner_late("2025-04-03")
        ]
    );
    assert_eq!(by(agents_utc(WS_SLA, &["telegram"])).await, [agent_day]);
    assert_eq!(owner_day1.3.response_sla_met_percent(), Some(50.0));
}

// ---- analytics-bot-messages --------------------------------------------

fn bot_utc(channels: &[&str]) -> TimeRange {
    range(
        WS_BOT,
        "2025-05-01T00:00:00Z",
        "2025-05-03T23:59:59Z",
        "UTC",
        channels,
    )
}

fn bot_hcm() -> TimeRange {
    range(
        WS_BOT,
        "2025-05-01T00:00:00+07:00",
        "2025-05-03T23:59:59+07:00",
        "Asia/Ho_Chi_Minh",
        &[],
    )
}

fn hit(timestamp: &str, result: &str, kind: &str, provider: &str, count: i64) -> BotRow {
    BotRow {
        timestamp: timestamp.into(),
        has_response: true,
        response_type: Some(kind.into()),
        result: Some(result.into()),
        ai_provider: provider.into(),
        count,
    }
}

fn zero(key: &str, result: &str) -> BotRow {
    hit(&day(key), result, "none", "none", 0)
}

fn unanswered(timestamp: &str, count: i64) -> BotRow {
    BotRow {
        timestamp: timestamp.into(),
        has_response: false,
        response_type: Some("none".into()),
        result: None,
        ai_provider: "none".into(),
        count,
    }
}

async fn bot_read(
    db: &Arc<Db>,
    f: fn(&Connection, &TimeRange, &str) -> chatbotx_db::repo::Result<Vec<BotRow>>,
    r: TimeRange,
    granularity: &'static str,
) -> Vec<BotRow> {
    read(db, move |c| f(c, &r, granularity)).await
}

#[tokio::test]
async fn by_result_zero_fills_days_and_keeps_hour_groups() {
    let db = seeded().await;
    assert_eq!(
        bot_read(&db, bot_messages::by_result, bot_utc(&[]), "day").await,
        [
            hit(&day("2025-05-01"), "success", "ai_agent", "openai", 2),
            zero("2025-05-01", "fallback"),
            hit(&day("2025-05-02"), "success", "flow", "none", 1),
            hit(&day("2025-05-02"), "fallback", "ai_agent", "anthropic", 1),
            hit(
                &day("2025-05-03"),
                "success",
                "automated_response",
                "none",
                1
            ),
            zero("2025-05-03", "fallback"),
        ]
    );
    assert_eq!(
        bot_read(&db, bot_messages::by_result, bot_hcm(), "day").await,
        [
            hit(&day("2025-05-01"), "success", "ai_agent", "openai", 2),
            zero("2025-05-01", "fallback"),
            hit(&day("2025-05-02"), "success", "flow", "none", 1),
            zero("2025-05-02", "fallback"),
            hit(
                &day("2025-05-03"),
                "success",
                "automated_response",
                "none",
                1
            ),
            hit(&day("2025-05-03"), "fallback", "ai_agent", "anthropic", 1),
        ]
    );
    assert_eq!(
        bot_read(
            &db,
            bot_messages::by_result,
            bot_utc(&["messenger"]),
            "hour"
        )
        .await,
        [
            hit(
                "2025-05-01T10:00:00.000Z",
                "success",
                "ai_agent",
                "openai",
                1
            ),
            hit(
                "2025-05-01T11:00:00.000Z",
                "success",
                "ai_agent",
                "openai",
                1
            ),
            hit(
                "2025-05-02T20:00:00.000Z",
                "fallback",
                "ai_agent",
                "anthropic",
                1
            ),
            hit(
                "2025-05-03T07:00:00.000Z",
                "success",
                "automated_response",
                "none",
                1
            ),
        ]
    );
}

#[tokio::test]
async fn by_result_past_60_days_sums_months() {
    let db = seeded().await;
    let r = range(
        WS_BOT,
        "2025-03-01T00:00:00Z",
        "2025-05-31T23:59:59Z",
        "UTC",
        &[],
    );
    let rows = bot_read(&db, bot_messages::by_result, r, "day").await;
    let counts: Vec<_> = rows
        .iter()
        .map(|r| (r.timestamp.as_str(), r.result.as_deref().unwrap(), r.count))
        .collect();
    assert_eq!(
        counts,
        [
            ("2025-03-01T00:00:00.000Z", "success", 0),
            ("2025-03-01T00:00:00.000Z", "fallback", 0),
            ("2025-04-01T00:00:00.000Z", "success", 1),
            ("2025-04-01T00:00:00.000Z", "fallback", 0),
            ("2025-05-01T00:00:00.000Z", "success", 4),
            ("2025-05-01T00:00:00.000Z", "fallback", 1),
        ]
    );
    assert_eq!(rows[0], zero("2025-03-01", "success"));
    assert_eq!(
        rows[2],
        hit(&day("2025-04-01"), "success", "ai_agent", "openai", 1)
    );
}

#[tokio::test]
async fn with_response_per_day_local_day_and_minute() {
    let db = seeded().await;
    let sorted = |mut rows: Vec<BotRow>| {
        rows.sort_by(|a, b| {
            (&a.timestamp, &a.response_type, &a.result).cmp(&(
                &b.timestamp,
                &b.response_type,
                &b.result,
            ))
        });
        rows
    };
    assert_eq!(
        sorted(bot_read(&db, bot_messages::with_response, bot_utc(&[]), "day").await),
        sorted(vec![
            hit(&day("2025-05-01"), "success", "ai_agent", "openai", 2),
            hit(&day("2025-05-02"), "success", "flow", "none", 1),
            hit(&day("2025-05-02"), "fallback", "ai_agent", "anthropic", 1),
            hit(
                &day("2025-05-03"),
                "success",
                "automated_response",
                "none",
                1
            ),
        ])
    );
    assert_eq!(
        sorted(bot_read(&db, bot_messages::with_response, bot_hcm(), "day").await),
        sorted(vec![
            hit(&day("2025-05-01"), "success", "ai_agent", "openai", 2),
            hit(&day("2025-05-02"), "success", "flow", "none", 1),
            hit(&day("2025-05-03"), "fallback", "ai_agent", "anthropic", 1),
            hit(
                &day("2025-05-03"),
                "success",
                "automated_response",
                "none",
                1
            ),
        ])
    );
    let one_day = range(
        WS_BOT,
        "2025-05-01T00:00:00Z",
        "2025-05-01T23:59:59Z",
        "UTC",
        &[],
    );
    assert_eq!(
        bot_read(&db, bot_messages::with_response, one_day, "minute").await,
        [
            hit(
                "2025-05-01T10:00:00.000Z",
                "success",
                "ai_agent",
                "openai",
                1
            ),
            hit(
                "2025-05-01T11:00:00.000Z",
                "success",
                "ai_agent",
                "openai",
                1
            ),
        ]
    );
}

#[tokio::test]
async fn no_response_per_day_hour_and_minute() {
    let db = seeded().await;
    assert_eq!(
        bot_read(&db, bot_messages::no_response, bot_utc(&[]), "day").await,
        [unanswered(&day("2025-05-03"), 2)]
    );
    assert_eq!(
        bot_read(
            &db,
            bot_messages::no_response,
            bot_utc(&["messenger"]),
            "hour"
        )
        .await,
        [unanswered("2025-05-03T05:00:00.000Z", 1)]
    );
    assert_eq!(
        bot_read(&db, bot_messages::no_response, bot_utc(&[]), "minute").await,
        [
            unanswered("2025-05-03T05:00:00.000Z", 1),
            unanswered("2025-05-03T06:00:00.000Z", 1)
        ]
    );
}

#[tokio::test]
async fn ai_provider_shares() {
    let db = seeded().await;
    let all = read(&db, |c| bot_messages::ai_providers(c, &bot_utc(&[]))).await;
    assert_eq!(
        all,
        [
            ("openai".to_string(), 2, (2.0 / 3.0) * 100.0),
            ("anthropic".to_string(), 1, (1.0 / 3.0) * 100.0)
        ]
    );
    let telegram = read(&db, |c| {
        bot_messages::ai_providers(c, &bot_utc(&["telegram"]))
    })
    .await;
    assert!(telegram.is_empty());
}
