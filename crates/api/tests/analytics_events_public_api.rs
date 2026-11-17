//! The conversation, message, agent and bot-message `analytics.*` routes
//! (Q.D11 slice 8): response shapes (string ids, `userName` / `userEmail` /
//! `result` left out when absent, SLA percents) and the 422 paths of the
//! parity group p1-03. The counting is covered by
//! `crates/business/tests/analytics_events.rs`.

mod handlers_common;

use handlers_common::{WS, call, fixture};
use serde_json::{Value, json};

const RANGE: &str = "from=2025-04-01T00%3A00%3A00Z&to=2025-04-03T23%3A59%3A59Z";
const OWNER: i64 = 1001;

async fn seed(fx: &handlers_common::Fixture) {
    fx.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "User" SET "name" = 'Owner' WHERE "id" = ?1"#,
                [OWNER],
            )?;
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("workspaceId", "userId", "role") VALUES (?1, ?2, 'owner')"#,
                [WS, OWNER],
            )?;
            tx.execute(
                r#"INSERT INTO "AnalyticsConversationEvent" ("eventId", "workspaceId", "conversationId", "eventType", "occurredAt", "toAssignee", "channel")
                   VALUES ('c1', ?1, 7, 'conversation_transferred_to_human', 1743501600000, NULL, 'messenger'),
                          ('c2', ?1, 7, 'conversation_assigned', 1743501600000, ?2, 'messenger')"#,
                [WS, OWNER],
            )?;
            tx.execute(
                r#"INSERT INTO "AnalyticsAgentResponseEvent" ("eventId", "workspaceId", "conversationId", "contactId", "adminId", "occurredAt", "awaitingSince", "measuredFrom", "responseSeconds", "isFirstResponse")
                   VALUES ('r1', ?1, 7, 8, ?2, 1743501600000, 1743501500000, 1743501500000, 100, 1)"#,
                [WS, OWNER],
            )?;
            tx.execute(
                r#"INSERT INTO "AnalyticsBotMessageEvent" ("eventId", "workspaceId", "messageId", "conversationId", "occurredAt", "hasResponse", "responseType", "result", "aiProvider")
                   VALUES ('b1', ?1, 1, 7, 1743501600000, 1, 'ai_agent', 'success', 'openai'),
                          ('b2', ?1, 2, 7, 1743501600000, 0, 'none', NULL, NULL)"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

async fn get(fx: &handlers_common::Fixture, route: &str, query: &str) -> (u16, Value) {
    call(fx, route, WS, &[], Some(query), None).await
}

const DAY: &str = "2025-04-01T00:00:00.000Z";

#[tokio::test]
async fn response_shapes() {
    let fx = fixture().await;
    seed(&fx).await;
    let data = |route: &'static str, query: String| {
        let fx = &fx;
        async move {
            let (status, body) = get(fx, route, &query).await;
            assert_eq!(status, 200, "{route}: {body}");
            body["data"].clone()
        }
    };
    let q = || RANGE.to_string();
    assert_eq!(
        data("analytics.conversationHandoffs", q()).await,
        json!([{ "timestamp": DAY, "direction": "to_human", "count": 1 }])
    );
    assert_eq!(
        data("analytics.conversationAssigned", q()).await,
        json!([{ "timestamp": DAY, "count": 1 }])
    );
    assert_eq!(
        data("analytics.uniqueConversationsByAdmin", q()).await,
        json!([{ "toAssignee": "1001", "count": 1, "userName": "Owner", "userEmail": "user-1001@example.test" }])
    );
    assert_eq!(
        data("analytics.messagesByAdmin", q()).await,
        json!([{ "adminId": "1001", "count": 0, "userName": "Owner", "userEmail": "user-1001@example.test" }])
    );
    assert_eq!(
        data("analytics.humanAgentStats", q()).await,
        json!([{
            "adminId": "1001", "userName": "Owner", "userEmail": "user-1001@example.test",
            "assignedConversations": 1, "messagesSent": 0, "uniqueContacts": 0,
            "avgResponseSeconds": 100, "avgWorkingResponseSeconds": null,
            "avgFirstResponseSeconds": 100, "avgWorkingFirstResponseSeconds": null,
            "responseCount": 1, "firstResponseCount": 1,
            "responseSlaEligibleCount": 0, "responseSlaMetCount": 0, "responseSlaMetPercent": null,
            "firstResponseSlaEligibleCount": 0, "firstResponseSlaMetCount": 0, "firstResponseSlaMetPercent": null,
        }])
    );
    assert_eq!(
        data("analytics.agentResponseSeries", q()).await[0]["date"],
        "2025-04-01"
    );
    assert_eq!(
        data("analytics.botMessagesNoResponse", q()).await,
        json!([{ "timestamp": DAY, "hasResponse": false, "responseType": "none", "aiProvider": "none", "count": 1 }])
    );
    assert_eq!(
        data(
            "analytics.botMessagesWithResponse",
            format!("{RANGE}&granularity=hour")
        )
        .await,
        json!([{ "timestamp": "2025-04-01T10:00:00.000Z", "hasResponse": true, "responseType": "ai_agent", "result": "success", "aiProvider": "openai", "count": 1 }])
    );
    assert_eq!(
        data("analytics.botMessagesByResult", q()).await[1],
        json!({ "timestamp": DAY, "hasResponse": true, "responseType": "none", "result": "fallback", "aiProvider": "none", "count": 0 })
    );
    assert_eq!(
        data("analytics.botMessagesAiProviders", q()).await,
        json!([{ "aiProvider": "openai", "count": 1, "percentage": 100.0 }])
    );
    assert_eq!(
        data(
            "analytics.messagesBySender",
            format!("{RANGE}&granularity=month")
        )
        .await,
        json!([])
    );
}

#[tokio::test]
async fn input_validation_paths() {
    let fx = fixture().await;
    let from_only = "from=2025-04-01T00:00:00Z".to_string();
    let mut cases = Vec::new();
    for route in [
        "analytics.conversationHandoffs",
        "analytics.conversationFollowUps",
        "analytics.conversationArchived",
        "analytics.conversationAssigned",
        "analytics.conversationAssignedByAdmin",
        "analytics.uniqueConversationsByAdmin",
    ] {
        cases.push((route, from_only.clone(), "to"));
        cases.push((route, format!("{RANGE}&channels=fax"), "channels.0"));
    }
    cases.extend([
        (
            "analytics.messagesBySender",
            format!("{RANGE}&granularity=hour"),
            "granularity",
        ),
        (
            "analytics.botMessagesByResult",
            format!("{RANGE}&granularity=month"),
            "granularity",
        ),
        (
            "analytics.agentResponseSeries",
            "to=2025-04-03T23:59:59Z".to_string(),
            "from",
        ),
        ("analytics.botMessagesAiProviders", from_only, "to"),
    ]);
    for (route, query, path) in cases {
        let (status, body) = get(&fx, route, &query).await;
        assert_eq!(status, 422, "{route}: {body}");
        let paths: Vec<String> = body["data"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                i["path"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().map_or_else(|| s.to_string(), str::to_string))
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .collect();
        assert!(paths.contains(&path.to_string()), "{route} {path}: {body}");
    }
}

/// p1-03 `analytics-links.test.ts`: the legacy `startDate` / `endDate`
/// names, the response shapes and the 422 paths of the link routes.
#[tokio::test]
async fn link_routes() {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "MagicLink" ("id", "workspaceId", "name", "url") VALUES (70, ?1, 'promo', 'https://x.test')"#,
                [WS],
            )?;
            tx.execute(
                r#"INSERT INTO "MagicLinkStat" ("workspaceId", "linkId", "contactId", "contactInboxId", "occurredAt") VALUES (?1, 70, 5, 6, 1754042400000)"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    let range = "from=2025-08-01T00:00:00Z&to=2025-08-03T23:59:59Z";
    let legacy = "startDate=2025-08-01T00:00:00Z&endDate=2025-08-03T23:59:59Z";
    for query in [range, legacy] {
        let (status, body) = get(
            &fx,
            "analytics.magicLinkStats",
            &format!("{query}&linkId=70&timezone=UTC"),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(
            body,
            json!({ "data": [{ "dateReport": "2025-08-01", "count": 1 }] })
        );
        let (_, body) = get(
            &fx,
            "analytics.magicLinkContacts",
            &format!("{query}&linkId=70"),
        )
        .await;
        assert_eq!(
            body,
            json!({
                "data": [{ "contactId": "", "contactInboxId": "", "sourceId": null, "channel": null, "conversationId": "", "occurredAt": "2025-08-01T10:00:00.000Z" }],
                "total": 1, "page": 1, "pageCount": 1,
            })
        );
    }
    let (_, body) = get(&fx, "analytics.refLinkContacts", "linkId=70&page=3").await;
    assert_eq!(
        body,
        json!({ "data": [], "total": 0, "page": 1, "pageCount": 0 })
    );
    for (route, query, path) in [
        (
            "analytics.magicLinkStats",
            format!("{range}&timezone=UTC"),
            "linkId",
        ),
        (
            "analytics.refLinkStats",
            format!("{range}&linkId=1"),
            "timezone",
        ),
        (
            "analytics.magicLinkStats",
            "to=2025-08-03T23:59:59Z&linkId=1&timezone=UTC".to_string(),
            "from",
        ),
        ("analytics.refLinkContacts", range.to_string(), "linkId"),
        (
            "analytics.magicLinkContacts",
            "linkId=1&perPage=51".to_string(),
            "perPage",
        ),
        (
            "analytics.refLinkContacts",
            "linkId=1&page=0".to_string(),
            "page",
        ),
    ] {
        let (status, body) = get(&fx, route, &query).await;
        assert_eq!(status, 422, "{route} {path}: {body}");
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
