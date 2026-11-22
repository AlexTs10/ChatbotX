//! Q.P7-V4: ports `apps/worker/__tests__/messenger-contact-data.test.ts`
//! (`updateMessengerContactData`) over `overwrite_contact_profile`, the merge
//! the step runs once it has the contact's Messenger inbox.
//!
//! Adapted: the mocked profile fetch is a closure, the mocked contact service
//! a real row, the mocked uploader an in-memory `Uploader` ("deleted" = the
//! object is gone). The "no Messenger inbox" case runs through the worker
//! (`crates/worker/src/steps/messenger_contact_data_tests.rs`). A failing
//! delete is not reproducible in memory; the delete is best effort the same
//! way the refresh's is.

mod common;

use chatbotx_business::contact::profile_refresh::{
    FetchResult, IncomingProfile, overwrite_contact_profile,
};
use chatbotx_files::Uploader;
use common::{WS, contact, contact_name, scalar_text, setup, two_workspaces};

const CONTACT: i64 = 10;
const OLD: &str = "public/space/ws-1/avatars/old";
const NEW: &str = "public/space/ws-1/avatars/abc";

/// Contact 10 named "Ada Lovelace" with `avatar`.
async fn fixture(avatar: Option<&'static str>) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        contact(tx, CONTACT, WS)?;
        contact_name(tx, CONTACT, Some("Ada"), Some("Lovelace"), avatar)?;
        Ok(())
    })
    .await;
    db
}

async fn uploader() -> Uploader {
    let uploader = Uploader::in_memory();
    for path in [OLD, NEW] {
        uploader
            .put_object(path, b"img".to_vec(), Default::default(), None)
            .await
            .expect("put object");
    }
    uploader
}

async fn exists(uploader: &Uploader, path: &str) -> bool {
    uploader.head_object(path).await.is_ok()
}

fn full_profile() -> IncomingProfile {
    IncomingProfile {
        source_id: "psid-1".into(),
        first_name: Some("Jane".into()),
        last_name: Some("Doe".into()),
        avatar: Some(NEW.into()),
        locale: Some("en_US".into()),
        timezone: Some("+07:00".into()),
        gender: Some("female".into()),
        ..IncomingProfile::default()
    }
}

async fn run(db: &common::TestDb, uploader: &Uploader, result: FetchResult) {
    overwrite_contact_profile(db, Some(uploader), WS, CONTACT, || async move { result }).await;
}

async fn column(db: &common::TestDb, sql: &'static str) -> Option<String> {
    scalar_text(db, sql, CONTACT).await
}

async fn row(db: &common::TestDb) -> Vec<Option<String>> {
    let mut out = Vec::new();
    for sql in [
        r#"SELECT "firstName" FROM "Contact" WHERE "id" = ?1"#,
        r#"SELECT "lastName" FROM "Contact" WHERE "id" = ?1"#,
        r#"SELECT "avatar" FROM "Contact" WHERE "id" = ?1"#,
        r#"SELECT "locale" FROM "Contact" WHERE "id" = ?1"#,
        r#"SELECT "timezone" FROM "Contact" WHERE "id" = ?1"#,
        r#"SELECT "gender" FROM "Contact" WHERE "id" = ?1"#,
    ] {
        out.push(column(db, sql).await);
    }
    out
}

fn some(values: &[&str]) -> Vec<Option<String>> {
    values.iter().map(|v| Some((*v).to_owned())).collect()
}

#[tokio::test]
async fn writes_every_field_the_profile_returned_overwriting_current_values() {
    let (db, files) = (fixture(Some(OLD)).await, uploader().await);
    run(&db, &files, Ok(Some(full_profile()))).await;
    assert_eq!(
        row(&db).await,
        some(&["Jane", "Doe", NEW, "en_US", "+07:00", "female"])
    );
}

#[tokio::test]
async fn undefined_profile_fields_are_never_written() {
    let (db, files) = (fixture(Some(OLD)).await, uploader().await);
    let profile = IncomingProfile {
        first_name: Some("Jane".into()),
        ..IncomingProfile::default()
    };
    run(&db, &files, Ok(Some(profile))).await;
    let got = row(&db).await;
    assert_eq!(got[..3], some(&["Jane", "Lovelace", OLD])[..]);
    // No avatar in the profile: the previous avatar is left in place.
    assert!(exists(&files, OLD).await);
}

#[tokio::test]
async fn a_profile_fetch_error_or_an_empty_profile_writes_nothing() {
    let (db, files) = (fixture(Some(OLD)).await, uploader().await);
    run(&db, &files, Err("graph error".into())).await;
    run(&db, &files, Ok(None)).await;
    run(&db, &files, Ok(Some(IncomingProfile::default()))).await;
    let got = row(&db).await;
    assert_eq!(got[..3], some(&["Ada", "Lovelace", OLD])[..]);
    assert!(exists(&files, OLD).await);
}

#[tokio::test]
async fn deletes_the_superseded_avatar_object_after_a_new_one_is_written() {
    let (db, files) = (fixture(Some(OLD)).await, uploader().await);
    run(&db, &files, Ok(Some(full_profile()))).await;
    assert!(!exists(&files, OLD).await);
    assert!(exists(&files, NEW).await);
}

#[tokio::test]
async fn an_unchanged_or_external_or_missing_previous_avatar_is_not_deleted() {
    for previous in [Some(NEW), Some("https://cdn.example.com/pic.jpg"), None] {
        let (db, files) = (fixture(previous).await, uploader().await);
        run(&db, &files, Ok(Some(full_profile()))).await;
        assert!(exists(&files, NEW).await, "{previous:?}");
        assert!(exists(&files, OLD).await, "{previous:?}");
        assert_eq!(
            column(&db, r#"SELECT "avatar" FROM "Contact" WHERE "id" = ?1"#).await,
            Some(NEW.into())
        );
    }
}

#[tokio::test]
async fn a_contact_of_another_workspace_is_not_written() {
    let (db, files) = (fixture(Some(OLD)).await, uploader().await);
    overwrite_contact_profile(&db, Some(&files), common::OTHER_WS, CONTACT, || async {
        Ok(Some(full_profile()))
    })
    .await;
    assert_eq!(
        column(&db, r#"SELECT "firstName" FROM "Contact" WHERE "id" = ?1"#).await,
        Some("Ada".into())
    );
}
