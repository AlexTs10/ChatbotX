//! Ports of the channel profile handlers' tests, over a migrated database, a
//! scripted Graph API and the in-memory store:
//! `integrations/messenger/__tests__/get-user-profile-avatar.test.ts`,
//! `integrations/instagram/__tests__/get-user-profile-avatar.test.ts`,
//! `integrations/instagram/__tests__/get-contact-profile-pic-url.test.ts`,
//! `integrations/instagram-facebook/__tests__/get-user-profile-avatar.test.ts`,
//! `integrations/instagram-facebook/__tests__/get-contact-profile-pic-url.test.ts`.

mod channel_fake;
mod common;

use channel_fake::{FakeHttp, channel_row, ok};
use chatbotx_business::contact::channel_profile::{
    ProfileDeps, ProfileSnapshot, fetch_profile, fetch_profile_pic_url, fetch_profile_snapshot,
};
use chatbotx_db::rusqlite::params;
use chatbotx_files::Uploader;
use common::{WS, setup, two_workspaces};
use serde_json::{Value, json};

const INBOX: i64 = 31;
const MESSENGER_PIC: &str = "https://scontent.xx.fbcdn.net/avatar.jpg";
const INSTAGRAM_PIC: &str = "https://scontent.cdninstagram.com/avatar.jpg";

fn auth(token: &str) -> Value {
    json!({ "tokens": { "accessToken": token }, "metadata": { "version": "v23.0" } })
}

/// One inbox of `table` (`ig_type` = the Instagram login).
async fn seed(
    table: &'static str,
    ig_type: Option<&'static str>,
    token: &'static str,
) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        channel_row(tx, table, INBOX, WS, "page-1", &auth(token))?;
        if let Some(kind) = ig_type {
            tx.execute(
                r#"UPDATE "IntegrationInstagram" SET "type" = ?2 WHERE "id" = ?1"#,
                params![INBOX, kind],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

fn messenger_http() -> FakeHttp {
    FakeHttp::new(|_| {
        ok(json!({
            "id": "user-123",
            "first_name": "Ada",
            "last_name": "Lovelace",
            "profile_pic": MESSENGER_PIC,
        }))
    })
    .with_download(MESSENGER_PIC, &[1, 2, 3], "image/jpeg")
}

fn instagram_http() -> FakeHttp {
    FakeHttp::new(|_| {
        ok(json!({ "id": "igsid-1", "name": "Ada", "username": "ada", "profile_pic": INSTAGRAM_PIC }))
    })
    .with_download(INSTAGRAM_PIC, &[1, 2, 3], "image/jpeg")
}

fn storage_prefix() -> String {
    chatbotx_files::get_storage_prefix(&WS.to_string())
}

// ---- Messenger getProfile avatar ---------------------------------------------

#[tokio::test]
async fn messenger_downloads_and_stores_the_picture_by_default() {
    let db = seed("IntegrationMessenger", None, "page-token").await;
    let http = messenger_http();
    let uploader = Uploader::in_memory();
    let deps = ProfileDeps {
        db: &db,
        http: &http,
        uploader: Some(&uploader),
    };
    let profile = fetch_profile(deps, WS, "messenger", INBOX, "user-123", true)
        .await
        .unwrap();
    assert_eq!(
        http.fetched(),
        vec![(MESSENGER_PIC.to_owned(), Some("page-token".to_owned()))]
    );
    let avatar = profile.avatar.expect("avatar");
    assert!(
        avatar.starts_with(&format!("public/space/{}/avatars/", storage_prefix())),
        "{avatar}"
    );
    // Stored public-read through the avatar profile (undecodable bytes are kept as-is).
    assert_eq!(&uploader.get_object(&avatar).await.unwrap()[..], &[1, 2, 3]);
}

#[tokio::test]
async fn messenger_skips_the_picture_when_the_caller_does_not_want_it() {
    let db = seed("IntegrationMessenger", None, "page-token").await;
    let http = messenger_http();
    let uploader = Uploader::in_memory();
    let deps = ProfileDeps {
        db: &db,
        http: &http,
        uploader: Some(&uploader),
    };
    let profile = fetch_profile(deps, WS, "messenger", INBOX, "user-123", false)
        .await
        .unwrap();
    assert!(http.fetched().is_empty());
    assert_eq!(profile.source_id, "user-123");
    assert_eq!(profile.first_name.as_deref(), Some("Ada"));
    assert_eq!(profile.last_name.as_deref(), Some("Lovelace"));
    assert_eq!(profile.avatar, None);
}

// ---- Instagram (both logins) getProfile avatar -------------------------------

#[tokio::test]
async fn instagram_downloads_and_stores_the_picture_by_default() {
    for kind in ["instagram", "facebook"] {
        let db = seed("IntegrationInstagram", Some(kind), "page-token").await;
        let http = instagram_http();
        let uploader = Uploader::in_memory();
        let deps = ProfileDeps {
            db: &db,
            http: &http,
            uploader: Some(&uploader),
        };
        let profile = fetch_profile(deps, WS, "instagram", INBOX, "igsid-1", true)
            .await
            .unwrap();
        assert_eq!(http.fetched().len(), 1, "{kind}");
        let avatar = profile.avatar.expect("avatar");
        assert!(
            avatar.starts_with(&format!("{}/avatars/", storage_prefix())),
            "{kind}: {avatar}"
        );
        assert_eq!(&uploader.get_object(&avatar).await.unwrap()[..], &[1, 2, 3]);
    }
}

#[tokio::test]
async fn instagram_skips_the_picture_when_the_caller_does_not_want_it() {
    for kind in ["instagram", "facebook"] {
        let db = seed("IntegrationInstagram", Some(kind), "page-token").await;
        let http = instagram_http();
        let uploader = Uploader::in_memory();
        let deps = ProfileDeps {
            db: &db,
            http: &http,
            uploader: Some(&uploader),
        };
        let profile = fetch_profile(deps, WS, "instagram", INBOX, "igsid-1", false)
            .await
            .unwrap();
        assert!(http.fetched().is_empty(), "{kind}");
        assert_eq!(profile.source_id, "igsid-1");
        assert_eq!(profile.first_name.as_deref(), Some("Ada"));
        assert_eq!(profile.avatar, None);
    }
}

// ---- getContactProfilePicUrl ---------------------------------------------------

#[tokio::test]
async fn instagram_returns_the_raw_graph_profile_picture_url() {
    let db = seed("IntegrationInstagram", Some("instagram"), "instagram-token").await;
    let http = FakeHttp::new(|_| {
        ok(json!({ "id": "igsid-1", "profile_pic": "https://cdn.example/avatar.jpg" }))
    });
    let url = fetch_profile_pic_url(&db, &http, WS, "instagram", INBOX, "igsid-1")
        .await
        .unwrap();
    assert_eq!(url.as_deref(), Some("https://cdn.example/avatar.jpg"));
    let seen = http.seen();
    assert_eq!(
        seen[0].url,
        "https://graph.instagram.com/v23.0/igsid-1?fields=id%2Cname%2Cusername%2Cprofile_pic"
    );
    assert_eq!(
        seen[0].headers,
        vec![(
            "Authorization".to_owned(),
            "Bearer instagram-token".to_owned()
        )]
    );
    assert!(http.fetched().is_empty());
}

#[tokio::test]
async fn instagram_facebook_returns_the_raw_graph_profile_picture_url() {
    let db = seed("IntegrationInstagram", Some("facebook"), "page-token").await;
    let http = FakeHttp::new(|_| {
        ok(json!({ "id": "igsid-1", "profile_pic": "https://cdn.example/avatar.jpg" }))
    });
    let url = fetch_profile_pic_url(&db, &http, WS, "instagram", INBOX, "igsid-1")
        .await
        .unwrap();
    assert_eq!(url.as_deref(), Some("https://cdn.example/avatar.jpg"));
    let seen = http.seen();
    assert_eq!(
        seen[0].url,
        "https://graph.facebook.com/v23.0/igsid-1?fields=name%2Cusername%2Cprofile_pic"
    );
    assert_eq!(
        seen[0].headers,
        vec![("Authorization".to_owned(), "Bearer page-token".to_owned())]
    );
}

#[tokio::test]
async fn returns_null_when_graph_has_no_profile_picture() {
    for kind in ["instagram", "facebook"] {
        let db = seed("IntegrationInstagram", Some(kind), "page-token").await;
        let http = FakeHttp::new(|_| ok(json!({ "id": "igsid-1" })));
        let url = fetch_profile_pic_url(&db, &http, WS, "instagram", INBOX, "igsid-1")
            .await
            .unwrap();
        assert_eq!(url, None, "{kind}");
    }
}

#[tokio::test]
async fn instagram_facebook_uses_the_verified_field_for_snapshots() {
    let db = seed("IntegrationInstagram", Some("facebook"), "page-token").await;
    let http = FakeHttp::new(|_| {
        ok(json!({
            "id": "igsid-1",
            "follower_count": 8,
            "is_business_follow_user": true,
            "is_user_follow_business": false,
            "is_verified_user": true,
        }))
    });
    let snapshot = fetch_profile_snapshot(&db, &http, WS, "instagram", INBOX, "igsid-1")
        .await
        .unwrap();
    assert_eq!(
        snapshot,
        Some(ProfileSnapshot {
            follows_business: Some(false),
            follower_count: Some(8),
            business_follows_contact: Some(true),
            account_verified: Some(true),
            username: None,
        })
    );
}

// ---- updateContactAvatar / refreshContactProfile ---------------------------------
// `apps/worker/__tests__/update-contact-avatar.test.ts`: the job mirrors
// through the stored contact inbox (its `sourceId`, not the queued one).
// `apps/builder/src/features/contacts/lib/__tests__/refresh-contact-profile.test.ts`:
// the channel fetcher is the inbox's `contact.getProfile`.

const CONTACT: i64 = 41;
const CONTACT_INBOX: i64 = 42;

async fn seed_contact(db: &chatbotx_db::Db, avatar: Option<&'static str>) {
    setup(db, move |tx| {
        common::contact(tx, CONTACT, WS)?;
        tx.execute(
            r#"UPDATE "Contact" SET "avatar" = ?2, "firstName" = NULL WHERE "id" = ?1"#,
            params![CONTACT, avatar],
        )?;
        common::contact_inbox(tx, CONTACT_INBOX, CONTACT, INBOX, "messenger", "user-123")?;
        Ok(())
    })
    .await;
}

async fn stored_avatar(db: &chatbotx_db::Db) -> Option<String> {
    common::scalar_text(
        db,
        r#"SELECT "avatar" FROM "Contact" WHERE "id" = ?1"#,
        CONTACT,
    )
    .await
}

#[tokio::test]
async fn update_contact_avatar_mirrors_through_the_durable_contact_inbox_lookup() {
    use chatbotx_business::contact::channel_profile::ensure_contact_avatar_mirrored;
    let db = seed("IntegrationMessenger", None, "page-token").await;
    seed_contact(&db, None).await;
    let http = messenger_http();
    let uploader = Uploader::in_memory();
    let deps = ProfileDeps {
        db: &db,
        http: &http,
        uploader: Some(&uploader),
    };
    let avatar = ensure_contact_avatar_mirrored(deps, WS, CONTACT_INBOX)
        .await
        .expect("avatar");
    assert!(http.urls()[0].contains("/user-123?"), "{:?}", http.urls());
    assert_eq!(stored_avatar(&db).await.as_deref(), Some(avatar.as_str()));
    assert!(avatar.contains("/avatars/"));
}

#[tokio::test]
async fn update_contact_avatar_keeps_a_real_avatar_and_stores_a_sentinel_on_failure() {
    use chatbotx_business::contact::channel_profile::ensure_contact_avatar_mirrored;
    use chatbotx_core::utils::no_avatar_sentinel::parse_no_avatar_sentinel;
    let db = seed("IntegrationMessenger", None, "page-token").await;
    seed_contact(&db, Some("public/avatars/kept.jpg")).await;
    let http = FakeHttp::offline();
    let deps = ProfileDeps {
        db: &db,
        http: &http,
        uploader: None,
    };
    assert_eq!(
        ensure_contact_avatar_mirrored(deps, WS, CONTACT_INBOX)
            .await
            .as_deref(),
        Some("public/avatars/kept.jpg")
    );
    assert!(http.seen().is_empty());

    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Contact" SET "avatar" = NULL WHERE "id" = ?1"#,
            [CONTACT],
        )?;
        Ok(())
    })
    .await;
    let sentinel = ensure_contact_avatar_mirrored(deps, WS, CONTACT_INBOX)
        .await
        .expect("sentinel");
    assert!(parse_no_avatar_sentinel(&sentinel).is_some(), "{sentinel}");
    assert_eq!(stored_avatar(&db).await, Some(sentinel));
}

#[tokio::test]
async fn refresh_contact_profile_uses_the_inbox_channel_fetcher_and_forwards_the_result() {
    use chatbotx_business::contact::profile_refresh::{
        RefreshOutcome, RefreshServices, refresh_contact_profile_with,
    };
    let db = seed("IntegrationMessenger", None, "page-token").await;
    seed_contact(&db, None).await;
    let http = messenger_http();
    let services = RefreshServices {
        kv: None,
        uploader: None,
    };
    let outcome = refresh_contact_profile_with(&db, services, &http, WS, CONTACT, CONTACT_INBOX)
        .await
        .expect("refresh");
    let RefreshOutcome::Updated(contact) = outcome else {
        panic!("expected an update, got {outcome:?}");
    };
    assert_eq!(contact.first_name.as_deref(), Some("Ada"));
    assert_eq!(contact.last_name.as_deref(), Some("Lovelace"));
    assert!(http.urls()[0].starts_with("https://graph.facebook.com/v23.0/user-123?"));
}
