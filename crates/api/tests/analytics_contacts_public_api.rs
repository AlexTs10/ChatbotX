//! The contact `analytics.*` routes (Q.D11 slice 8): query parsing (repeated
//! keys build the filter arrays), the 422 issue paths of the parity group
//! p1-03 `analytics-contacts.test.ts` "input validation", the response
//! shapes, and `analytics.filterOptions` (`list-analytics-filter-options`
//! and p1-03 `analytics-filter-options.test.ts`). The counting itself is
//! covered by `crates/business/tests/analytics_contacts.rs`.

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

const RANGE: &str = "from=2025-03-01T00%3A00%3A00Z&to=2025-03-05T23%3A59%3A59Z";

async fn seed(fx: &handlers_common::Fixture) {
    fx.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status") VALUES
                   (11, 'Beta', 'telegram', 's1', ?1, 'connected'),
                   (12, 'Alpha', 'messenger', 's2', ?1, 'connected'),
                   (13, 'Gamma', 'smtp', 's3', ?1, 'connected'),
                   (14, 'Delta', 'instagram', 's4', ?1, 'disconnected'),
                   (15, 'Epsilon', 'not-a-channel', 's5', ?1, 'connected'),
                   (16, 'Charlie', 'whatsapp', 's6', ?1, 'connected'),
                   (17, 'Own', 'messenger', 's7', ?2, 'connected')"#,
                [WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "InboxContactStat" ("inboxId", "totalContacts") VALUES (11, 1), (12, 2)"#,
                [],
            )?;
            for (contact, channel, inbox) in [(1, "messenger", 12), (2, "telegram", 11)] {
                tx.execute(
                    r#"INSERT INTO "AnalyticsContactEvent" ("eventId", "workspaceId", "contactId", "eventType", "occurredAt", "channel", "inboxId")
                       VALUES (?1, ?2, ?1, 'contact_created', 1740873600000, ?3, ?4)"#,
                    chatbotx_db::rusqlite::params![contact, WS, channel, inbox],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
}

async fn get(fx: &handlers_common::Fixture, route: &str, query: &str) -> (u16, Value) {
    call(fx, route, WS, &[], Some(query), None).await
}

fn issue_paths(body: &Value) -> Vec<String> {
    body["data"]["issues"]
        .as_array()
        .map(|issues| {
            issues
                .iter()
                .map(|issue| {
                    issue["path"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|seg| match seg {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(".")
                })
                .collect()
        })
        .unwrap_or_default()
}

fn assert_invalid(status: u16, body: &Value, path: &str) {
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], "invalidRequestData", "{body}");
    assert!(
        issue_paths(body).contains(&path.to_string()),
        "{path}: {body}"
    );
}

#[tokio::test]
async fn filter_options_list_connected_known_channel_inboxes_except_smtp_by_name() {
    let fx = fixture().await;
    seed(&fx).await;
    let (status, body) = get(&fx, "analytics.filterOptions", "channels=messenger").await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({ "data": [
            { "id": "12", "name": "Alpha", "channel": "messenger" },
            { "id": "11", "name": "Beta", "channel": "telegram" },
            { "id": "16", "name": "Charlie", "channel": "whatsapp" },
        ] })
    );
    let (_, other) = call(
        &fx,
        "analytics.filterOptions",
        OTHER_WS + 1,
        &[],
        None,
        None,
    )
    .await;
    assert_eq!(other, json!({ "data": [] }));
}

#[tokio::test]
async fn repeated_query_keys_build_the_filter_arrays() {
    let fx = fixture().await;
    seed(&fx).await;
    let count = |query: String| {
        let fx = &fx;
        async move { get(fx, "analytics.newContactsCount", &query).await.1 }
    };
    assert_eq!(
        count(RANGE.to_string()).await,
        json!({ "data": { "count": 2 } })
    );
    assert_eq!(
        count(format!("{RANGE}&channels=telegram")).await,
        json!({ "data": { "count": 1 } })
    );
    assert_eq!(
        count(format!("{RANGE}&channels=telegram&channels=messenger")).await,
        json!({ "data": { "count": 2 } })
    );
    assert_eq!(
        count(format!("{RANGE}&inboxIds=12")).await,
        json!({ "data": { "count": 1 } })
    );
}

#[tokio::test]
async fn series_and_count_shapes() {
    let fx = fixture().await;
    seed(&fx).await;
    let (status, body) = get(&fx, "analytics.newContactCountsPerDay", RANGE).await;
    assert_eq!(status, 200);
    assert_eq!(body["data"].as_array().unwrap().len(), 5);
    assert_eq!(
        body["data"][1],
        json!({ "date": "2025-03-02T00:00:00.000Z", "count": 2 })
    );
    let (_, body) = get(
        &fx,
        "analytics.contactsCount",
        "from=2030-01-01T00:00:00Z&to=2030-01-02T00:00:00Z",
    )
    .await;
    assert_eq!(body, json!({ "data": { "count": 3 } }));
    let (_, body) = get(
        &fx,
        "analytics.contactsByDimension",
        &format!("{RANGE}&dimension=channel"),
    )
    .await;
    assert_eq!(
        body,
        json!({ "data": [
            { "dimension": "messenger", "count": 1, "uniqueContacts": 1 },
            { "dimension": "telegram", "count": 1, "uniqueContacts": 1 },
        ] })
    );
    let (_, body) = get(&fx, "analytics.macActiveContactCount", "").await;
    assert_eq!(body, json!({ "data": { "macCount": 0 } }));
}

#[tokio::test]
async fn input_validation_paths() {
    let fx = fixture().await;
    let to = "to=2025-03-05T23:59:59Z";
    let from = "from=2025-03-01T00:00:00Z";
    let ids: String = (1..=201).map(|i| format!("&inboxIds={i}")).collect();
    for (route, query, path) in [
        ("analytics.contactCountsPerDay", to.to_string(), "from"),
        ("analytics.newContactCountsPerDay", from.to_string(), "to"),
        (
            "analytics.blockedContactsPerDay",
            format!("{RANGE}&channels=fax"),
            "channels.0",
        ),
        (
            "analytics.blockedContactsCount",
            format!("{RANGE}&inboxIds=abc"),
            "inboxIds.0",
        ),
        (
            "analytics.newContactsCount",
            format!("{RANGE}&inboxIds=1234567890123456789"),
            "inboxIds.0",
        ),
        ("analytics.contactsCount", from.to_string(), "to"),
        ("analytics.activeContactsCount", to.to_string(), "from"),
        (
            "analytics.contactsByDimension",
            RANGE.to_string(),
            "dimension",
        ),
        (
            "analytics.contactsByDimension",
            format!("{RANGE}&dimension=city"),
            "dimension",
        ),
        (
            "analytics.contactsByDimension",
            format!("{RANGE}&dimension=channel&metric=all"),
            "metric",
        ),
        (
            "analytics.contactsByDimension",
            format!("{RANGE}&dimension=country&metric=total"),
            "metric",
        ),
        (
            "analytics.contactsByDimension",
            format!("{RANGE}&dimension=source&metric=total&inboxIds=1"),
            "metric",
        ),
        (
            "analytics.newContactsCount",
            format!("{RANGE}{ids}"),
            "inboxIds",
        ),
    ] {
        let (status, body) = get(&fx, route, &query).await;
        assert_invalid(status, &body, path);
    }
}
