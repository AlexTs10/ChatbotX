//! Port of the list / count part of
//! `apps/builder/__tests__/contacts-crud-public-api.test.ts` and of
//! `apps/builder/__tests__/contacts-public-filter-strict.test.ts` (P3.6-D2
//! slice contacts list). Adapted: the mocked `contactService.list / count`
//! and `resolveContactAvatars` become a migrated database; the asserted
//! delegation becomes the response; `schema.parse / safeParse` of
//! `listContactsPublicRequest` / `countContactsPublicRequest` becomes the
//! status of `GET /v1/contacts` and `GET /v1/contacts/count`.

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

const WHATSAPP: i64 = 3001;
const CONTACT: i64 = 9001;

/// One contact with a tag, a custom field value and a whatsapp contact inbox
/// (the `InboxContactStat` sum is 1).
async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, 'WA', 'whatsapp', 'src-wa')"#,
                params![WHATSAPP, WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "email", "workspaceId") VALUES (?1, 'Ada', 'ada@example.com', ?2)"#,
                params![CONTACT, WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                   VALUES (4001, ?1, ?1, ?2, 'whatsapp', 'direct', 'wa-1')"#,
                params![CONTACT, WHATSAPP],
            )?;
            tx.execute(
                r#"INSERT INTO "Tag" ("id", "workspaceId", "name") VALUES (1, ?1, 'vip')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES (?1, 1)"#,
                params![CONTACT],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    f
}

async fn get(f: &Fixture, route: &str, query: &str) -> (u16, Value) {
    call(f, route, WS, &[], Some(query), None).await
}

fn encoded(value: &Value) -> String {
    url::form_urlencoded::byte_serialize(value.to_string().as_bytes()).collect()
}

// ---- GET /v1/contacts ---------------------------------------------------------

#[tokio::test]
async fn get_v1_contacts_passes_include_with_count_through_as_separate_options_not_merged_into_the_query_filter()
 {
    let f = seeded().await;
    let (status, body) = get(
        &f,
        "contacts.list",
        "page=1&perPage=20&include%5B%5D=tags&withCount=false",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["totalCount"], json!(0));
    assert_eq!(body["pageCount"], json!(0));
    assert_eq!(body["totalCountCapped"], json!(false));
    let row = &body["data"][0];
    assert_eq!(row["id"], json!(CONTACT.to_string()));
    // Unscoped (token) caller: full PII.
    assert_eq!(row["email"], json!("ada@example.com"));
    assert_eq!(row["tags"].as_array().map(Vec::len), Some(1));
    for key in ["contactCustomFields", "contactInboxes", "conversation"] {
        assert!(row.get(key).is_none(), "{key} in {row}");
    }
}

// ---- GET /v1/contacts/count ---------------------------------------------------

#[tokio::test]
async fn get_v1_contacts_count_delegates_to_contact_service_count() {
    let f = seeded().await;
    let (status, body) = get(&f, "contacts.count", "page=1&perPage=20").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"total": 1, "totalCountCapped": false}));
}

#[tokio::test]
async fn get_v1_contacts_count_forwards_count_mode_and_returns_the_capped_outcome() {
    let f = seeded().await;
    let (status, body) = get(
        &f,
        "contacts.count",
        "page=1&perPage=20&countMode=bounded&keyword=ada",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({"total": 1, "totalCountCapped": false, "countCap": 100_000})
    );
}

// ---- public contacts %s contactFilter -----------------------------------------

const ROUTES: [&str; 2] = ["contacts.list", "contacts.count"];

fn valid_filter() -> Value {
    json!({"operator": "and", "conditions": [{"field": "inbox", "operator": "eq", "value": ["123"]}]})
}

#[tokio::test]
async fn public_contacts_s_contact_filter_parses_a_valid_json_contact_filter() {
    let f = seeded().await;
    for route in ROUTES {
        let (status, body) = get(
            &f,
            route,
            &format!("contactFilter={}", encoded(&valid_filter())),
        )
        .await;
        assert_eq!(status, 200, "{route}: {body}");
    }
}

#[tokio::test]
async fn public_contacts_s_contact_filter_rejects_malformed_json_instead_of_dropping_the_filter() {
    let f = seeded().await;
    for route in ROUTES {
        let (status, body) = get(&f, route, "contactFilter=%7Binvalid").await;
        assert_eq!(status, 422, "{route}: {body}");
        assert_eq!(body["code"], json!("invalidRequestData"), "{route}");
    }
}

#[tokio::test]
async fn public_contacts_s_contact_filter_rejects_a_json_value_that_fails_the_filter_schema() {
    let f = seeded().await;
    let bad = json!({"operator": "and", "conditions": [{}]});
    for route in ROUTES {
        let (status, body) = get(&f, route, &format!("contactFilter={}", encoded(&bad))).await;
        assert_eq!(status, 422, "{route}: {body}");
        assert_eq!(body["code"], json!("invalidRequestData"), "{route}");
    }
}

#[tokio::test]
async fn public_contacts_s_contact_filter_still_accepts_a_request_without_a_contact_filter() {
    let f = seeded().await;
    for route in ROUTES {
        let (status, body) = get(&f, route, "").await;
        assert_eq!(status, 200, "{route}: {body}");
    }
}

#[tokio::test]
async fn public_contacts_s_contact_filter_accepts_the_audience_style_keys() {
    let f = seeded().await;
    for route in ROUTES {
        let (status, body) = get(
            &f,
            route,
            "channels%5B%5D=whatsapp&inboxIds%5B%5D=1&subaction=whatsappWithin24Hours",
        )
        .await;
        assert_eq!(status, 200, "{route}: {body}");
    }
}
