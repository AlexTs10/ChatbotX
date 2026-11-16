//! Port of `apps/builder/__tests__/contacts-ad-referrals-public-api.test.ts`
//! (P3.6-D2, slice inboxes / ads / refresh). Adapted: the captured oRPC
//! handler and the mocked `listContactAdTouches` become `handlers::dispatch`
//! calls over real `ContactReferralEvent` rows; "was called with cursor X"
//! becomes the page that the cursor selects.
//!
//! Not applicable: "passes the caller's access scope through" (no member
//! access scope in the Rust API; it ports with mobile auth).

mod handlers_common;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chatbotx_api::routes::{PUBLIC_ROUTES, RouteAuth};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

/// 2026-10-20T08:00:00.000Z.
const OCT_20_8H: i64 = 1_792_483_200_000;
const DAY: i64 = 86_400_000;

/// Contact 123 (`+841234567890`) with messenger contact inbox 50.
async fn seed_contact(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (5, ?1, 'Page', 'messenger', 'page-5')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "phoneNumber") VALUES (123, ?1, '+841234567890')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                   VALUES (50, 123, 123, 5, 'messenger', 'direct', 'psid-123')"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("seed contact");
}

/// One ad touch of contact 123.
async fn touch(
    f: &Fixture,
    id: i64,
    occurred_at: i64,
    ad_id: Option<&'static str>,
    provider: &'static str,
    metadata: Option<Value>,
) {
    let metadata = metadata.map(|m| m.to_string());
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "ContactReferralEvent" ("id", "workspaceId", "contactId", "contactInboxId", "channel", "type",
                     "provider", "occurredAt", "adId", "source", "sourceUrl", "metadata", "clickIdType", "clickId", "dedupKey")
                   VALUES (?1, ?2, 123, 50, 'messenger', 'ad', ?3, ?4, ?5, 'ADS', 'https://fb.me/ad', ?6, 'ctwa', 'click-secret', 'ad:' || ?1)"#,
                params![id, WS, provider, occurred_at, ad_id, metadata],
            )?;
            Ok(())
        })
        .await
        .expect("seed touch");
}

fn summer_sale() -> Option<Value> {
    Some(json!({ "adTitle": "Summer sale", "adBody": "secret body" }))
}

async fn list(f: &Fixture, query: &str) -> (u16, Value) {
    call(
        f,
        "contacts.listAdReferrals",
        WS,
        &[("identifier", "id:123")],
        Some(query),
        None,
    )
    .await
}

fn ids(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| row["id"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn get_v1_contacts_identifier_ad_referrals_is_registered_under_the_contacts_scope() {
    let route = PUBLIC_ROUTES
        .iter()
        .find(|route| route.path == "/v1/contacts/{identifier}/ad-referrals")
        .expect("route");
    assert_eq!(route.method, "GET");
    assert!(matches!(route.auth, RouteAuth::WorkspaceToken(scopes) if scopes == ["contacts"]));
}

#[tokio::test]
async fn get_v1_contacts_identifier_ad_referrals_resolves_the_identifier_in_the_token_s_workspace_then_lists_that_contact_s_touches()
 {
    let f = fixture().await;
    seed_contact(&f).await;
    touch(&f, 9, OCT_20_8H, Some("120"), "meta", summer_sale()).await;
    let (status, body) = call(
        &f,
        "contacts.listAdReferrals",
        WS,
        &[("identifier", "phone:+841234567890")],
        Some("perPage=20"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["data"][0]["adId"], json!("120"));
    assert_eq!(body["data"][0]["title"], json!("Summer sale"));
    assert_eq!(body["nextCursor"], Value::Null);
}

#[tokio::test]
async fn list_contact_ad_referrals_exposes_the_ad_and_where_it_came_from_never_the_stored_metadata_or_click_ids()
 {
    let f = fixture().await;
    seed_contact(&f).await;
    touch(&f, 9, OCT_20_8H, Some("120"), "meta", summer_sale()).await;
    let (status, body) = list(&f, "perPage=20").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["data"],
        json!([{
            "id": "9",
            "adId": "120",
            "title": "Summer sale",
            "channel": "messenger",
            "provider": "meta",
            "campaignId": null,
            "adGroupId": null,
            "source": "ADS",
            "sourceUrl": "https://fb.me/ad",
            "occurredAt": "2026-10-20T08:00:00.000Z",
        }])
    );
    let text = body.to_string();
    assert!(!text.contains("secret body"));
    assert!(!text.contains("clickId"));
    assert!(!text.contains("click-secret"));
}

#[tokio::test]
async fn list_contact_ad_referrals_a_touch_with_no_headline_or_ad_id_is_still_listed() {
    let f = fixture().await;
    seed_contact(&f).await;
    touch(&f, 9, OCT_20_8H, None, "google", None).await;
    let (status, body) = list(&f, "perPage=20").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"][0]["adId"], Value::Null);
    assert_eq!(body["data"][0]["title"], Value::Null);
    assert_eq!(body["data"][0]["provider"], json!("google"));
}

#[tokio::test]
async fn list_contact_ad_referrals_hands_the_next_page_s_cursor_back_as_an_opaque_string_and_accepts_it_again()
 {
    let f = fixture().await;
    seed_contact(&f).await;
    touch(&f, 9, OCT_20_8H, Some("120"), "meta", summer_sale()).await;
    touch(&f, 8, OCT_20_8H - DAY, Some("119"), "meta", None).await;
    let (status, first) = list(&f, "perPage=1").await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(ids(&first), vec!["9"]);
    let cursor = first["nextCursor"]
        .as_str()
        .expect("string cursor")
        .to_string();
    let query = format!("perPage=1&cursor={}", urlencode(&cursor));
    let (status, second) = list(&f, &query).await;
    assert_eq!(status, 200, "{second}");
    assert_eq!(ids(&second), vec!["8"]);
    assert_eq!(second["nextCursor"], Value::Null);
}

#[tokio::test]
async fn list_contact_ad_referrals_a_malformed_cursor_is_a_400_not_a_silent_restart_that_would_repeat_rows()
 {
    let f = fixture().await;
    seed_contact(&f).await;
    touch(&f, 9, OCT_20_8H, Some("120"), "meta", None).await;
    let (status, body) = list(&f, "perPage=20&cursor=not-a-cursor").await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["code"], json!("invalidCursor"));
    assert_eq!(
        body["message"],
        json!("The cursor is invalid. Start again without a cursor.")
    );
}

#[tokio::test]
async fn list_contact_ad_referrals_a_cursor_the_encoder_produced_round_trips_its_time_and_id() {
    let f = fixture().await;
    seed_contact(&f).await;
    // The cursor names touch 8 (2026-10-19T08:00Z): the page is what is older.
    touch(&f, 9, OCT_20_8H, Some("120"), "meta", None).await;
    touch(&f, 8, OCT_20_8H - DAY, Some("119"), "meta", None).await;
    touch(&f, 7, OCT_20_8H - DAY, Some("118"), "meta", None).await;
    touch(&f, 6, OCT_20_8H - 2 * DAY, Some("117"), "meta", None).await;
    let cursor = STANDARD.encode(
        json!({ "direction": "next", "createdAt": "2026-10-19T08:00:00.000Z", "id": "8" })
            .to_string(),
    );
    let query = format!("perPage=20&cursor={}", urlencode(&cursor));
    let (status, body) = list(&f, &query).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(ids(&body), vec!["7", "6"]);
}

/// Percent-encodes the base64 characters that are not query-safe.
fn urlencode(text: &str) -> String {
    text.replace('+', "%2B")
        .replace('/', "%2F")
        .replace('=', "%3D")
}
