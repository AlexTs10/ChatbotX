//! `channelPosts.list` / `channelPosts.listSelected` (parity p1-04 `10`):
//! newest first, caption search, keyset cursor, ids of the workspace only,
//! and the zod checks of the query.

mod handlers_common;

use chatbotx_api::handlers::channel_posts::parse_sort_at;
use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

const JAN: i64 = 1_767_261_600_000; // 2026-01-01T10:00:00.000Z
const DAY: i64 = 86_400_000;

async fn seeded() -> handlers_common::Fixture {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            for (inbox, ws) in [(40_i64, WS), (41, OTHER_WS)] {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, 'Parity IG', 'instagram', ?3)"#,
                    chatbotx_db::rusqlite::params![inbox, ws, format!("ig{inbox}")],
                )?;
            }
            for (id, ws, inbox, caption, day, thumb) in [
                (101_i64, WS, 40_i64, Some("Summer sale"), 0, None),
                (102, WS, 40, Some("Winter SALE"), 1, Some("media/thumb-2.jpg")),
                (103, WS, 40, None, 2, None),
                (104, OTHER_WS, 41, Some("Other sale"), 3, None),
            ] {
                tx.execute(
                    r#"INSERT INTO "ChannelPost" ("id", "workspaceId", "inboxId", "channel", "integrationId", "sourceAccountId",
                         "externalPostId", "caption", "thumbnail", "permalink", "publishedAt")
                       VALUES (?1, ?2, ?3, 'instagram', 1, 'acct', ?4, ?5, ?6, NULL, ?7)"#,
                    chatbotx_db::rusqlite::params![id, ws, inbox, format!("post-{id}"), caption, thumb, JAN + day * DAY],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    fx
}

fn ids(out: &Value, key: &str) -> Vec<String> {
    out[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect()
}

fn issue_paths(out: &Value) -> Vec<String> {
    out["data"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| {
            issue["path"]
                .as_array()
                .unwrap()
                .iter()
                .map(|seg| seg.as_str().map_or_else(|| seg.to_string(), str::to_string))
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect()
}

#[tokio::test]
async fn lists_newest_first_searches_and_pages() {
    let fx = seeded().await;
    let (status, out) = call(&fx, "channelPosts.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(ids(&out, "items"), ["103", "102", "101"]);
    assert!(out.get("nextCursor").is_none(), "{out}");
    let item = &out["items"][1];
    assert_eq!(
        (
            item["inboxId"].clone(),
            item["inboxName"].clone(),
            item["publishedAt"].clone()
        ),
        (
            json!("40"),
            json!("Parity IG"),
            json!("2026-01-02T10:00:00.000Z")
        )
    );
    assert!(
        item["thumbnailUrl"]
            .as_str()
            .unwrap()
            .ends_with("/media/thumb-2.jpg"),
        "{item}"
    );

    let (_, out) = call(&fx, "channelPosts.list", WS, &[], Some("search=sale"), None).await;
    assert_eq!(ids(&out, "items"), ["102", "101"]);

    let (_, first) = call(&fx, "channelPosts.list", WS, &[], Some("limit=2"), None).await;
    assert_eq!(ids(&first, "items"), ["103", "102"]);
    assert_eq!(
        first["nextCursor"],
        json!({ "id": "102", "sortAt": "2026-01-02T10:00:00.000Z" })
    );
    // Postgres `timestamptz` text is accepted too.
    let query = "limit=2&cursor%5Bid%5D=102&cursor%5BsortAt%5D=2026-01-02%2010%3A00%3A00%2B00";
    let (status, second) = call(&fx, "channelPosts.list", WS, &[], Some(query), None).await;
    assert_eq!(status, 200, "{second}");
    assert_eq!(ids(&second, "items"), ["101"]);
    assert!(second.get("nextCursor").is_none());
}

#[tokio::test]
async fn list_selected_keeps_the_workspace_posts_only() {
    let fx = seeded().await;
    let query = "ids%5B%5D=101&ids%5B%5D=103&ids%5B%5D=104&ids%5B%5D=999";
    let (status, out) = call(&fx, "channelPosts.listSelected", WS, &[], Some(query), None).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(ids(&out, "data"), ["101", "103"]);
}

#[tokio::test]
async fn invalid_queries_are_422_at_the_failing_key() {
    let fx = seeded().await;
    let long = format!("search={}", "x".repeat(201));
    for (route, query, path) in [
        ("channelPosts.list", "limit=0", "limit"),
        ("channelPosts.list", "limit=101", "limit"),
        ("channelPosts.list", long.as_str(), "search"),
        ("channelPosts.list", "search=%20%20", "search"),
        (
            "channelPosts.list",
            "cursor%5Bid%5D=0&cursor%5BsortAt%5D=2026-01-01T00%3A00%3A00Z",
            "cursor.id",
        ),
        (
            "channelPosts.list",
            "cursor%5Bid%5D=5&cursor%5BsortAt%5D=yesterday",
            "cursor.sortAt",
        ),
        ("channelPosts.listSelected", "", "ids"),
        (
            "channelPosts.listSelected",
            "ids%5B%5D=5&ids%5B%5D=5",
            "ids",
        ),
        ("channelPosts.listSelected", "ids%5B%5D=0", "ids.0"),
        (
            "channelPosts.listSelected",
            "ids%5B%5D=99999999999999999999",
            "ids.0",
        ),
    ] {
        let (status, out) = call(&fx, route, WS, &[], Some(query), None).await;
        assert_eq!(status, 422, "{query}: {out}");
        assert!(
            issue_paths(&out).contains(&path.to_string()),
            "{query}: {out}"
        );
    }
    let many: Vec<String> = (1..=101).map(|i| format!("ids%5B%5D={i}")).collect();
    let (status, out) = call(
        &fx,
        "channelPosts.listSelected",
        WS,
        &[],
        Some(many.join("&").as_str()),
        None,
    )
    .await;
    assert_eq!(
        (status, issue_paths(&out).contains(&"ids".to_string())),
        (422, true),
        "{out}"
    );
}

#[test]
fn sort_at_reads_iso_and_postgres_text() {
    let at = Some(JAN);
    assert_eq!(parse_sort_at("2026-01-01T10:00:00.000Z"), at);
    assert_eq!(parse_sort_at("2026-01-01 10:00:00+00"), at);
    assert_eq!(parse_sort_at("2026-01-01 12:00:00.000000+02:00"), at);
    assert_eq!(parse_sort_at("2026-01-01 12:00:00+0200"), at);
    for bad in [
        "yesterday",
        "2026-02-30T00:00:00Z",
        "2026-01-01T10:00:60Z",
        "2026-01-01T10:00:00",
        "2026-01-01T10:00:00.1234567Z",
    ] {
        assert_eq!(parse_sort_at(bad), None, "{bad}");
    }
}
