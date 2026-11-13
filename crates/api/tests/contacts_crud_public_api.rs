//! Port of `apps/builder/__tests__/contacts-crud-public-api.test.ts` (the
//! single-contact routes: get, create, update, delete, block, unblock,
//! upsert) and `create-contact-request-whatsapp-identity.test.ts`
//! (P3.6-D2). Adapted: the mocked `contactService` / `resolveContactAvatars`
//! become a migrated database; the asserted delegations become the stored
//! rows and the responses; the schema `safeParse` issue paths become the
//! `422 invalidRequestData` issue paths of `POST /v1/contacts`. The list /
//! count / import tests belong to later slices.

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture, scalar};
use serde_json::{Value, json};

const WEBCHAT: i64 = 3001;
const WHATSAPP: i64 = 3003;
const CONTACT: i64 = 9001;

async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            for (id, channel) in [(WEBCHAT, "webchat"), (WHATSAPP, "whatsapp")] {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, ?3, ?4, ?5)"#,
                    params![id, WS, format!("Inbox {id}"), channel, format!("src-{id}")],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "lastName", "email", "avatar", "workspaceId")
                   VALUES (?1, 'Ada', 'Lovelace', 'ada@example.com', 'ws/avatar.jpg', ?2)"#,
                params![CONTACT, WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    f
}

fn ident(id: i64) -> String {
    format!("id:{id}")
}

async fn get(f: &Fixture, identifier: &str) -> (u16, Value) {
    call(
        f,
        "contacts.get",
        WS,
        &[("identifier", identifier)],
        None,
        None,
    )
    .await
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
                        .map(|path| {
                            path.iter()
                                .map(|seg| match seg {
                                    Value::String(s) => s.clone(),
                                    other => other.to_string(),
                                })
                                .collect::<Vec<_>>()
                                .join(".")
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The issue paths of a `POST /v1/contacts` (empty = accepted).
async fn create_issue_paths(f: &Fixture, body: Value) -> Vec<String> {
    let (status, response) = call(f, "contacts.create", WS, &[], None, Some(body)).await;
    if status == 422 && response["code"] == "invalidRequestData" {
        issue_paths(&response)
    } else {
        assert_eq!(status, 200, "{response}");
        Vec::new()
    }
}

fn whatsapp_base(extra: Value) -> Value {
    let mut body = json!({
        "email": "",
        "gender": "unknown",
        "channel": "whatsapp",
        "inboxId": WHATSAPP.to_string(),
    });
    for (k, v) in extra.as_object().expect("object") {
        body[k] = v.clone();
    }
    body
}

#[tokio::test]
async fn get_resolves_the_contact_id_via_resolve_id_by_identifier_then_returns_find_public_contact_or_fail()
 {
    let f = seeded().await;
    for identifier in [ident(CONTACT), "email:ada@example.com".to_string()] {
        let (status, body) = get(&f, &identifier).await;
        assert_eq!(status, 200, "{identifier} {body}");
        assert_eq!(body["id"], CONTACT.to_string());
        assert_eq!(body["workspaceId"], WS.to_string());
        assert_eq!(body["fullName"], "Ada Lovelace");
        assert_eq!(body["tags"], json!([]));
        assert_eq!(body["contactCustomFields"], json!([]));
        assert_eq!(body["contactInboxes"], json!([]));
        assert_eq!(body["conversation"], Value::Null);
    }
    let (status, body) = get(&f, "id:1").await;
    assert_eq!((status, body["code"].clone()), (404, json!("notFound")));
}

#[tokio::test]
async fn get_returns_the_avatar_as_a_resolved_url_not_the_stored_key() {
    let f = seeded().await;
    let (status, body) = get(&f, &ident(CONTACT)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["avatar"], "http://app/storage/ws/avatar.jpg");
}

#[tokio::test]
async fn post_creates_the_contact_then_returns_the_result_of_find_public_contact_or_fail() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "contacts.create",
        WS,
        &[],
        None,
        Some(json!({
            "firstName": "Grace",
            "inboxId": WEBCHAT.to_string(),
            "channel": "webchat",
            "email": "",
            "gender": "unknown",
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let id: i64 = body["id"].as_str().expect("id").parse().expect("numeric");
    assert_eq!(body["firstName"], "Grace");
    assert_eq!(body["contactInboxes"][0]["inboxId"], WEBCHAT.to_string());
    assert_eq!(
        body["contactInboxes"][0]["inbox"]["id"],
        WEBCHAT.to_string()
    );
    assert_eq!(body["conversation"]["contactId"], id.to_string());
    // The response is the public read of the stored contact.
    let (_, again) = get(&f, &ident(id)).await;
    assert_eq!(again, body);
}

#[tokio::test]
async fn patch_resolves_the_contact_id_via_resolve_id_by_identifier_before_updating_fields() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "contacts.update",
        WS,
        &[("identifier", "email:ada@example.com")],
        None,
        Some(json!({ "firstName": "Grace" })),
    )
    .await;
    assert_eq!((status, body), (204, Value::Null));
    let first = scalar(
        &f,
        r#"SELECT "firstName" FROM "Contact" WHERE "id" = ?1"#,
        CONTACT,
    )
    .await;
    assert_eq!(first, Some(json!("Grace")));
    let (status, _) = call(
        &f,
        "contacts.updateLegacy",
        WS,
        &[("identifier", "id:1")],
        None,
        Some(json!({ "firstName": "x" })),
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn delete_resolves_the_contact_id_via_resolve_id_by_identifier_before_deleting() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "contacts.delete",
        WS,
        &[("identifier", "email:ada@example.com")],
        None,
        None,
    )
    .await;
    assert_eq!((status, body), (204, Value::Null));
    assert_eq!(
        scalar(
            &f,
            r#"SELECT COUNT(*) FROM "Contact" WHERE "id" = ?1"#,
            CONTACT
        )
        .await,
        Some(json!(0))
    );
    let (status, _) = call(
        &f,
        "contacts.delete",
        WS,
        &[("identifier", &ident(CONTACT))],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn block_resolves_the_contact_id_via_resolve_id_by_identifier_before_blocking() {
    let f = seeded().await;
    let (status, _) = call(
        &f,
        "contacts.block",
        WS,
        &[("identifier", &ident(CONTACT))],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let blocked = scalar(
        &f,
        r#"SELECT "blockedAt" FROM "Contact" WHERE "id" = ?1"#,
        CONTACT,
    )
    .await;
    assert!(blocked.is_some_and(|v| v.is_i64()), "blockedAt is set");
}

#[tokio::test]
async fn unblock_resolves_the_contact_id_via_resolve_id_by_identifier_before_unblocking() {
    let f = seeded().await;
    call(
        &f,
        "contacts.block",
        WS,
        &[("identifier", &ident(CONTACT))],
        None,
        None,
    )
    .await;
    let (status, _) = call(
        &f,
        "contacts.unblock",
        WS,
        &[("identifier", &ident(CONTACT))],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let blocked = scalar(
        &f,
        r#"SELECT "blockedAt" FROM "Contact" WHERE "id" = ?1"#,
        CONTACT,
    )
    .await;
    assert_eq!(blocked, Some(Value::Null));
}

#[tokio::test]
async fn upsert_upserts_by_identifier_then_returns_the_result_of_find_public_contact_or_fail() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "contacts.upsert",
        WS,
        &[("identifier", "email:new@example.com")],
        None,
        Some(json!({ "firstName": " Upsy " })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["email"], "new@example.com");
    assert_eq!(body["firstName"], "Upsy");
    assert_eq!(body["contactInboxes"][0]["source"], "api");
    assert_eq!(body["contactInboxes"][0]["channel"], "webchat");
}

#[tokio::test]
async fn upsert_only_spreads_fields_that_were_actually_provided_not_blanking_unset_ones() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "contacts.upsert",
        WS,
        &[("identifier", "email:ada@example.com")],
        None,
        Some(json!({ "lastName": "Byron" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], CONTACT.to_string());
    assert_eq!(body["firstName"], "Ada");
    assert_eq!(body["lastName"], "Byron");
    assert_eq!(body["email"], "ada@example.com");
}

#[tokio::test]
async fn whatsapp_identity_accepts_a_phone_number_alone() {
    let f = seeded().await;
    assert_eq!(
        create_issue_paths(&f, whatsapp_base(json!({ "phoneNumber": "+15551234567" }))).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn whatsapp_identity_accepts_a_bsuid_alone() {
    let f = seeded().await;
    assert_eq!(
        create_issue_paths(
            &f,
            whatsapp_base(json!({ "sourceUserId": "US.13491208655302741918" }))
        )
        .await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn whatsapp_identity_accepts_both_a_phone_number_and_a_bsuid() {
    let f = seeded().await;
    assert_eq!(
        create_issue_paths(
            &f,
            whatsapp_base(json!({
                "phoneNumber": "+15551234567",
                "sourceUserId": "US.13491208655302741918",
            }))
        )
        .await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn whatsapp_identity_requires_a_phone_number_or_a_bsuid() {
    let f = seeded().await;
    assert_eq!(
        create_issue_paths(
            &f,
            whatsapp_base(json!({ "phoneNumber": "", "sourceUserId": "" }))
        )
        .await,
        vec!["phoneNumber".to_string()]
    );
}

#[tokio::test]
async fn whatsapp_identity_rejects_a_digits_only_bsuid_which_would_collide_with_a_phone_based_wa_id()
 {
    let f = seeded().await;
    let paths =
        create_issue_paths(&f, whatsapp_base(json!({ "sourceUserId": "15551234567" }))).await;
    assert!(paths.contains(&"sourceUserId".to_string()), "{paths:?}");
}

#[tokio::test]
async fn whatsapp_identity_rejects_a_bsuid_on_a_non_whatsapp_channel() {
    let f = seeded().await;
    assert_eq!(
        create_issue_paths(
            &f,
            whatsapp_base(json!({
                "channel": "messenger",
                "contactId": "psid-1",
                "sourceUserId": "US.13491208655302741918",
            }))
        )
        .await,
        vec!["sourceUserId".to_string()]
    );
}
