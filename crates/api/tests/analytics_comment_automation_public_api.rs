//! The `analytics.commentAutomation*` routes (Q.D11 slice 8b), with the seed
//! and expectations of the parity group p1-03
//! `analytics-comment-automation.test.ts`, plus a Threads automation for the
//! counted public half and the `comment:missed` drill-down.

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

const AUTO: i64 = 80;
const THREADS: i64 = 81;
const UTC: &str = "from=2025-07-01T00:00:00Z&to=2025-07-03T23:59:59Z&timezone=UTC";
const HCM: &str =
    "from=2025-07-01T00:00:00%2B07:00&to=2025-07-03T23:59:59%2B07:00&timezone=Asia/Ho_Chi_Minh";

fn ms(iso: &str) -> i64 {
    chatbotx_core::js::parse_date(iso).expect("iso")
}

/// K1..K4: contact `100 + k`, contact inbox `200 + k`, conversation `300 + k`.
async fn seeded() -> handlers_common::Fixture {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (11, ?1, 'Page', 'messenger', 'src-11')"#,
                [WS],
            )?;
            for (k, last) in [(1, "One"), (2, "Two"), (3, "Three"), (4, "Four")] {
                tx.execute(
                    r#"INSERT INTO "Contact" ("id", "firstName", "lastName", "workspaceId") VALUES (?1, 'Kay', ?2, ?3)"#,
                    chatbotx_db::rusqlite::params![100 + k, last, WS],
                )?;
                tx.execute(
                    r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                       VALUES (?1, ?2, ?2, 11, 'messenger', 'direct', ?3)"#,
                    chatbotx_db::rusqlite::params![200 + k, 100 + k, format!("psid-{k}")],
                )?;
                tx.execute(
                    r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (?1, ?2, ?3)"#,
                    [300 + k, WS, 100 + k],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "CommentAutomation" ("id", "name", "workspaceId", "type", "inboxId", "sentCount", "deliveredCount", "seenCount", "clickedCount", "failedCount", "missedCount")
                   VALUES (?1, 'Auto', ?2, 'messenger', 11, 4, 3, 1, 1, 1, 0), (?3, 'Threads', ?2, 'threads', 11, 1, 1, 0, 0, 0, 1)"#,
                [AUTO, WS, THREADS],
            )?;
            type Event<'a> = (i64, i64, &'a str, &'a str, &'a str, Option<&'a str>, &'a str, &'a str, [Option<&'a str>; 4], Option<&'a str>);
            // (id, k, commentId, commentText, channel, replyText, status, occurredAt, [delivered, seen, clicked, failed], errorDetail)
            let events: [Event; 6] = [
                (500, 1, "c1", "price?", "private", Some("dm-1"), "sent", "2025-07-01T10:00:00Z", [Some("2025-07-01T10:00:05Z"), None, None, None], None),
                (501, 1, "c1", "price?", "public", Some("thanks"), "sent", "2025-07-01T10:00:01Z", [None; 4], None),
                (502, 2, "c2", "price?", "private", Some("dm-1"), "sent", "2025-07-01T23:30:00Z", [Some("2025-07-01T23:31:00Z"), Some("2025-07-02T00:00:00Z"), None, None], None),
                (503, 4, "c5", "how much", "private", None, "failed", "2025-07-02T12:00:00Z", [None, None, None, Some("2025-07-02T12:00:00Z")], Some("user blocked the page")),
                (504, 3, "c4", "what is the price", "private", Some("dm-2"), "sent", "2025-07-03T05:00:00Z", [Some("2025-07-03T05:00:10Z"), None, Some("2025-07-03T06:00:00Z"), None], None),
                (505, 1, "c9", "old comment", "private", Some("dm-old"), "sent", "2025-05-15T00:00:00Z", [Some("2025-05-15T00:00:01Z"), None, None, None], None),
            ];
            for (id, k, comment, text, channel, reply, status, at, [delivered, seen, clicked, failed], error) in events {
                tx.execute(
                    r#"INSERT INTO "CommentAutomationEvent" ("id", "workspaceId", "automationId", "contactId", "contactInboxId", "postId", "commentId", "commentText", "replyChannel", "replyType", "replyText", "status", "occurredAt", "deliveredAt", "seenAt", "clickedAt", "failedAt", "errorDetail", "httpCode")
                       VALUES (?1, ?2, ?3, ?4, ?5, 'post-1', ?6, ?7, ?8, 'text', ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"#,
                    chatbotx_db::rusqlite::params![
                        id, WS, AUTO, 100 + k, 200 + k, comment, text, channel, reply, status, ms(at),
                        delivered.map(ms), seen.map(ms), clicked.map(ms), failed.map(ms), error, error.map(|_| "400"),
                    ],
                )?;
            }
            // Threads: the public reply is the counted half; one miss.
            tx.execute(
                r#"INSERT INTO "CommentAutomationEvent" ("id", "workspaceId", "automationId", "contactId", "contactInboxId", "postId", "commentId", "replyChannel", "replyType", "status", "occurredAt", "deliveredAt")
                   VALUES (510, ?1, ?2, 101, 201, 'p', 't1', 'public', 'text', 'sent', ?3, ?3)"#,
                chatbotx_db::rusqlite::params![WS, THREADS, ms("2025-07-01T00:00:00Z")],
            )?;
            tx.execute(
                r#"INSERT INTO "CommentAutomationMiss" ("id", "workspaceId", "automationId", "contactId", "contactInboxId", "postId", "commentId", "commentText", "reason", "occurredAt")
                   VALUES (520, ?1, ?2, 102, 202, 'p', 't2', 'hi', 'keywordsNotMatched', ?3)"#,
                chatbotx_db::rusqlite::params![WS, THREADS, ms("2025-07-02T00:00:00Z")],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    fx
}

async fn get(fx: &handlers_common::Fixture, ws: i64, route: &str, query: &str) -> (u16, Value) {
    call(fx, route, ws, &[], Some(query), None).await
}

async fn ok(fx: &handlers_common::Fixture, route: &str, query: &str) -> Value {
    let (status, body) = get(fx, WS, route, query).await;
    assert_eq!(status, 200, "{route} {query}: {body}");
    body
}

fn days(rows: &[(&str, i64)]) -> Value {
    json!({ "data": rows.iter().map(|(d, c)| json!({ "dateReport": d, "count": c })).collect::<Vec<_>>() })
}

fn texts(rows: &[(&str, i64)], total: i64, page: i64, page_count: i64) -> Value {
    json!({
        "data": rows.iter().map(|(t, n)| json!({ "text": t, "total": n })).collect::<Vec<_>>(),
        "total": total, "page": page, "pageCount": page_count,
    })
}

#[tokio::test]
async fn reply_stats_and_text_lists() {
    let fx = seeded().await;
    let auto = format!("automationId={AUTO}");
    let replies = "analytics.commentAutomationReplyStats";
    assert_eq!(
        ok(&fx, replies, &format!("{auto}&{UTC}")).await,
        days(&[("2025-07-01", 3), ("2025-07-02", 0), ("2025-07-03", 1)])
    );
    assert_eq!(
        ok(&fx, replies, &format!("{auto}&{HCM}")).await,
        days(&[("2025-07-01", 2), ("2025-07-02", 1), ("2025-07-03", 1)])
    );
    assert_eq!(
        ok(
            &fx,
            replies,
            &format!("{auto}&from=2025-05-01T00:00:00Z&to=2025-07-31T23:59:59Z&timezone=UTC")
        )
        .await,
        days(&[("2025-05-01", 1), ("2025-06-01", 0), ("2025-07-01", 4)])
    );
    let (_, foreign) = get(&fx, OTHER_WS, replies, &format!("{auto}&{UTC}")).await;
    assert_eq!(foreign, json!({ "data": [] }));
    assert_eq!(
        ok(&fx, replies, &format!("automationId=1&{UTC}")).await,
        json!({ "data": [] })
    );

    let comments = "analytics.commentAutomationUserComments";
    assert_eq!(
        ok(&fx, comments, &format!("{auto}&{UTC}")).await,
        texts(
            &[("price?", 2), ("how much", 1), ("what is the price", 1)],
            3,
            1,
            1
        )
    );
    assert_eq!(
        ok(&fx, comments, &format!("{auto}&{UTC}&keyword=PRICE")).await,
        texts(&[("price?", 2), ("what is the price", 1)], 2, 1, 1)
    );
    assert_eq!(
        ok(&fx, comments, &format!("{auto}&{UTC}&perPage=1&page=2")).await,
        texts(&[("how much", 1)], 3, 2, 3)
    );

    let bot = "analytics.commentAutomationBotReplies";
    assert_eq!(
        ok(&fx, bot, &format!("{auto}&{UTC}")).await,
        texts(&[("dm-1", 2), ("dm-2", 1), ("thanks", 1)], 3, 1, 1)
    );
    assert_eq!(
        ok(&fx, bot, &format!("{auto}&{UTC}&keyword=zzz")).await,
        texts(&[], 0, 1, 0)
    );
    assert_eq!(
        ok(&fx, bot, &format!("automationId=1&{UTC}&page=2")).await,
        texts(&[], 0, 2, 0)
    );

    let errors = "analytics.commentAutomationErrors";
    assert_eq!(
        ok(&fx, errors, &format!("{auto}&{UTC}")).await,
        json!({
            "data": [{
                "id": "503", "replyChannel": "private", "replyType": "text",
                "errorDetail": "user blocked the page", "httpCode": "400", "commentText": "how much",
                "contact": { "firstName": "Kay", "lastName": "Four", "avatar": null },
                "occurredAt": "2025-07-02T12:00:00.000Z",
            }],
            "total": 1, "page": 1, "pageCount": 1,
        })
    );
    assert_eq!(
        ok(&fx, errors, &format!("{auto}&{UTC}&keyword=BLOCKED")).await["total"],
        1
    );
    assert_eq!(
        ok(&fx, errors, &format!("{auto}&{UTC}&keyword=nothing")).await,
        texts(&[], 0, 1, 0)
    );
}

/// One drill-down row of event `id` for contact `k`.
fn row(id: i64, k: i64, at: &str, error: Option<&str>) -> Value {
    json!({
        "rowKey": id.to_string(), "contactId": (100 + k).to_string(), "contactInboxId": (200 + k).to_string(),
        "sourceId": format!("psid-{k}"), "channel": "messenger", "conversationId": (300 + k).to_string(),
        "errorContent": error, "occurredAt": at, "commentText": null, "missReason": null,
    })
}

fn page(data: Vec<Value>, total: i64, contacts: i64, page: i64, page_count: i64) -> Value {
    json!({ "data": data, "total": total, "contactTotal": contacts, "page": page, "pageCount": page_count })
}

#[tokio::test]
async fn contacts_list_the_counted_events_newest_first() {
    let fx = seeded().await;
    let route = "analytics.commentAutomationContacts";
    let list = |event: &str, extra: &str| format!("automationId={AUTO}&eventType={event}{extra}");
    let blocked = Some("user blocked the page");
    assert_eq!(
        ok(&fx, route, &list("message:delivered", "")).await,
        page(
            vec![
                row(504, 3, "2025-07-03T05:00:10.000Z", None),
                row(502, 2, "2025-07-01T23:31:00.000Z", None),
                row(500, 1, "2025-07-01T10:00:05.000Z", None),
                row(505, 1, "2025-05-15T00:00:01.000Z", None),
            ],
            3,
            3,
            1,
            1
        )
    );
    assert_eq!(
        ok(&fx, route, &list("message:sent", "")).await,
        page(
            vec![
                row(504, 3, "2025-07-03T05:00:10.000Z", None),
                row(503, 4, "2025-07-02T12:00:00.000Z", blocked),
                row(502, 2, "2025-07-01T23:31:00.000Z", None),
                row(500, 1, "2025-07-01T10:00:05.000Z", None),
                row(505, 1, "2025-05-15T00:00:01.000Z", None),
            ],
            4,
            4,
            1,
            1
        )
    );
    assert_eq!(
        ok(&fx, route, &list("message:failed", "")).await,
        page(
            vec![row(503, 4, "2025-07-02T12:00:00.000Z", blocked)],
            1,
            1,
            1,
            1
        )
    );
    assert_eq!(
        ok(&fx, route, &list("message:seen", "")).await,
        page(
            vec![row(502, 2, "2025-07-02T00:00:00.000Z", None)],
            1,
            1,
            1,
            1
        )
    );
    assert_eq!(
        ok(&fx, route, &list("flow:clicked", "")).await,
        page(
            vec![row(504, 3, "2025-07-03T06:00:00.000Z", None)],
            1,
            1,
            1,
            1
        )
    );
    assert_eq!(
        ok(&fx, route, &list("message:delivered", "&perPage=1&page=2")).await,
        page(
            vec![row(502, 2, "2025-07-01T23:31:00.000Z", None)],
            3,
            3,
            2,
            3
        )
    );
    assert_eq!(
        ok(&fx, route, &list("comment:missed", "")).await,
        page(vec![], 0, 0, 1, 0)
    );

    // Threads has no DM: the public reply is listed; the miss carries its
    // comment and reason.
    let threads = |event: &str| format!("automationId={THREADS}&eventType={event}");
    assert_eq!(
        ok(&fx, route, &threads("message:sent")).await,
        page(
            vec![row(510, 1, "2025-07-01T00:00:00.000Z", None)],
            1,
            1,
            1,
            1
        )
    );
    let mut miss = row(520, 2, "2025-07-02T00:00:00.000Z", None);
    miss["commentText"] = json!("hi");
    miss["missReason"] = json!("keywordsNotMatched");
    assert_eq!(
        ok(&fx, route, &threads("comment:missed")).await,
        page(vec![miss], 1, 1, 1, 1)
    );

    for (ws, id) in [(OTHER_WS, AUTO), (WS, 1)] {
        let (status, body) = get(
            &fx,
            ws,
            route,
            &format!("automationId={id}&eventType=message:sent"),
        )
        .await;
        assert_eq!(status, 404, "{body}");
        assert_eq!(body["code"], "notFound", "{body}");
    }
}

#[tokio::test]
async fn input_validation_paths() {
    let fx = seeded().await;
    let auto = format!("automationId={AUTO}");
    let cases = [
        (
            "analytics.commentAutomationContacts",
            auto.clone(),
            "eventType",
        ),
        (
            "analytics.commentAutomationContacts",
            format!("{auto}&eventType=message:received"),
            "eventType",
        ),
        (
            "analytics.commentAutomationContacts",
            "automationId=abc&eventType=message:sent".into(),
            "automationId",
        ),
        (
            "analytics.commentAutomationContacts",
            format!("{auto}&eventType=message:sent&perPage=51"),
            "perPage",
        ),
        (
            "analytics.commentAutomationReplyStats",
            format!("{auto}&from=2025-07-01&to=2025-07-03T23:59:59Z&timezone=UTC"),
            "from",
        ),
        (
            "analytics.commentAutomationReplyStats",
            format!("{auto}&from=2025-07-01T00:00:00Z&to=2025-07-03T23:59:59&timezone=UTC"),
            "to",
        ),
        (
            "analytics.commentAutomationUserComments",
            format!(
                "{auto}&from=2025-07-01T00:00:00Z&to=2025-07-03T23:59:59Z&timezone=Mars/Olympus"
            ),
            "timezone",
        ),
        (
            "analytics.commentAutomationBotReplies",
            format!("{auto}&from=2025-07-01T00:00:00Z&to=2025-07-03T23:59:59Z"),
            "timezone",
        ),
        (
            "analytics.commentAutomationErrors",
            format!("automationId=x1&{UTC}"),
            "automationId",
        ),
        (
            "analytics.commentAutomationErrors",
            format!("{auto}&{UTC}&perPage=0"),
            "perPage",
        ),
    ];
    for (route, query, path) in cases {
        let (status, body) = get(&fx, WS, route, &query).await;
        assert_eq!(status, 422, "{route} {query}: {body}");
        assert!(
            body["data"]["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["path"] == json!([path])),
            "{route} {path}: {body}"
        );
    }
}
