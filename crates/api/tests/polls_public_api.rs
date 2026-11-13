//! Port of `apps/builder/__tests__/polls-public-api.test.ts`.
//! Adapted: the mocked repositories become a real migrated database and the
//! procedures are called through `handlers::dispatch`; the two token tests
//! (scope / read_only) check the route table entry the P3.5 auth runs on.

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_db::rusqlite::params;
use handlers_common::{WS, call, fixture};
use serde_json::{Value, json};

fn poll_body(candidates: Value) -> Value {
    json!({
        "name": "Best cat",
        "imageUrl": "https://x/y.png",
        "startsAt": "2026-01-01T00:00:00Z",
        "endsAt": "2099-02-01T00:00:00Z",
        "candidates": candidates,
    })
}

async fn new_poll(f: &handlers_common::Fixture) -> Value {
    let (status, body) = call(
        f,
        "polls.create",
        WS,
        &[],
        None,
        Some(poll_body(json!([{ "name": "c1" }, { "name": "c2" }]))),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body
}

/// Votes `count` times for `candidate` with new contacts of `gender`.
async fn votes(
    f: &handlers_common::Fixture,
    poll: &Value,
    candidate: usize,
    gender: &'static str,
    count: i64,
    first_contact: i64,
) {
    let poll_id: i64 = poll["id"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .expect("poll id");
    let candidate_id: i64 = poll["candidates"][candidate]["id"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .expect("candidate id");
    f.state
        .db
        .write(move |tx| {
            for n in 0..count {
                let contact = first_contact + n;
                tx.execute(
                    r#"INSERT INTO "Contact" ("id", "firstName", "gender", "workspaceId") VALUES (?1, 'V', ?2, ?3)"#,
                    params![contact, gender, WS],
                )?;
                tx.execute(
                    r#"INSERT INTO "PollVote" ("id", "workspaceId", "pollId", "candidateId", "contactId") VALUES (?1, ?2, ?3, ?4, ?5)"#,
                    params![contact + 100_000, WS, poll_id, candidate_id, contact],
                )?;
            }
            Ok(())
        })
        .await
        .expect("votes");
}

#[tokio::test]
async fn polls_public_api_a_token_without_the_polls_scope_gets_forbidden() {
    let route = route_by_id("polls.list").expect("route");
    assert_eq!(route.auth, RouteAuth::WorkspaceToken(&["polls"]));
}

#[tokio::test]
async fn polls_public_api_a_polls_scoped_token_can_list_with_iso_dates_and_votes_count() {
    let f = fixture().await;
    let poll = new_poll(&f).await;
    votes(&f, &poll, 0, "male", 7, 5000).await;
    let (status, body) = call(&f, "polls.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    let row = &body["data"][0];
    assert_eq!(row["id"], poll["id"]);
    assert_eq!(row["votesCount"], json!(7));
    assert_eq!(row["startsAt"], json!("2026-01-01T00:00:00.000Z"));
    assert_eq!(row["state"], json!("running"));
}

#[tokio::test]
async fn polls_public_api_create_with_1_candidate_is_a_400_poll_candidate_count() {
    let f = fixture().await;
    let (status, body) = call(
        &f,
        "polls.create",
        WS,
        &[],
        None,
        Some(poll_body(json!([{ "name": "only" }]))),
    )
    .await;
    assert_eq!(
        (status, body["code"].clone(), body["status"].clone()),
        (400, json!("pollCandidateCount"), json!(400))
    );
    let (_, list) = call(&f, "polls.list", WS, &[], None, None).await;
    assert_eq!(list["data"], json!([]));
}

#[tokio::test]
async fn polls_public_api_get_attaches_each_candidate_s_votes_count() {
    let f = fixture().await;
    let poll = new_poll(&f).await;
    votes(&f, &poll, 0, "male", 3, 5000).await;
    let id = poll["id"].as_str().expect("id").to_string();
    let (status, body) = call(&f, "polls.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{body}");
    let counts: Vec<Value> = body["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .map(|c| c["votesCount"].clone())
        .collect();
    assert_eq!(counts, vec![json!(3), json!(0)]);
}

#[tokio::test]
async fn polls_public_api_get_links_keeps_only_ref_capable_inboxes_whats_app_carries_ref_pl() {
    let f = fixture().await;
    let poll = new_poll(&f).await;
    f.state
        .db
        .write(|tx| {
            for (id, channel, name) in [(901_i64, "whatsapp", "WA"), (902, "threads", "Th"), (903, "zalo", "Za")] {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (?1, ?2, ?3, ?4, ?5)"#,
                    params![id, name, channel, format!("s{id}"), WS],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name",
                     "workspaceId", "inboxId", "displayPhoneNumber")
                   VALUES (911, '{}', 'phone-1', 'waba-1', 'business-1', 'WhatsApp', ?1, 901, '15551234567')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("inboxes");
    let id = poll["id"].as_str().expect("id").to_string();
    let (status, body) = call(&f, "polls.getLinks", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["omnichannelUrl"],
        json!(format!("http://app/l/{WS}/p/{id}"))
    );
    let links = body["links"].as_array().expect("links");
    assert_eq!(
        links
            .iter()
            .map(|l| l["inboxId"].clone())
            .collect::<Vec<_>>(),
        vec![json!("901")]
    );
    assert!(
        links[0]["url"]
            .as_str()
            .is_some_and(|u| u.contains("text=%2Fref-pl_")),
        "{body}"
    );
}

#[tokio::test]
async fn polls_public_api_get_analytics_returns_totals_gender_split_and_per_candidate_shares() {
    let f = fixture().await;
    let poll = new_poll(&f).await;
    votes(&f, &poll, 0, "male", 3, 5000).await;
    votes(&f, &poll, 1, "female", 1, 6000).await;
    let id = poll["id"].as_str().expect("id").to_string();
    let (status, body) = call(&f, "polls.getAnalytics", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["poll"],
        json!({
            "id": id,
            "name": "Best cat",
            "imageUrl": "https://x/y.png",
            "startsAt": "2026-01-01T00:00:00.000Z",
            "endsAt": "2099-02-01T00:00:00.000Z",
        })
    );
    assert_eq!(body["totalVotes"], json!(4));
    assert_eq!(
        body["gender"],
        json!({ "male": 3, "female": 1, "unknown": 0 })
    );
    let c = &body["candidates"];
    assert_eq!(
        (
            c[0]["totalVotes"].clone(),
            c[0]["totalPercent"].clone(),
            c[0]["maleVotes"].clone()
        ),
        (json!(3), json!(75), json!(3))
    );
    assert_eq!(
        (
            c[1]["totalVotes"].clone(),
            c[1]["totalPercent"].clone(),
            c[1]["femaleVotes"].clone()
        ),
        (json!(1), json!(25), json!(1))
    );
}

#[tokio::test]
async fn polls_public_api_a_read_only_token_cannot_delete() {
    let route = route_by_id("polls.delete").expect("route");
    assert_eq!(
        (route.method, route.auth),
        ("DELETE", RouteAuth::WorkspaceToken(&["polls"]))
    );
}
