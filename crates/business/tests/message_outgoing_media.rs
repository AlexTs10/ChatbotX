//! `createOutgoing` with media library picks: the file is copied into the
//! conversation, sent as an attachment with its public URL; a foreign file
//! and an image the channel cannot carry are refused before any row exists.

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::message::{MediaStore, OutgoingInput, create_outgoing_with_media};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::rusqlite::params;
use chatbotx_files::{PutOptions, Uploader};
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::json;

const STORAGE: &str = "http://app/storage/";

/// Contact 1 on webchat inbox 5 (contact inbox 50) and Messenger inbox 7
/// (contact inbox 52), conversation 70; image 900 of `WS`, image 901 of
/// `OTHER_WS`.
async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        common::inbox(tx, 5, WS, "webchat")?;
        common::inbox(tx, 7, WS, "messenger")?;
        common::contact(tx, 1, WS)?;
        common::contact_inbox(tx, 50, 1, 5, "webchat", "guest-1")?;
        common::contact_inbox(tx, 52, 1, 7, "messenger", "psid-1")?;
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "createdAt", "updatedAt", "workspaceId", "contactId") VALUES (70, 1, 1, ?1, 1)"#,
            params![WS],
        )?;
        common::media_file(tx, 900, WS, "image/png")?;
        common::media_file(tx, 902, WS, "image/png")?;
        common::media_file(tx, 901, OTHER_WS, "image/png")?;
        Ok(())
    })
    .await;
    db
}

async fn models(db: &common::TestDb, contact_inbox_id: i64) -> (Conversation, ContactInbox) {
    chatbotx_business::tx::read(db, move |conn| {
        let conversation = chatbotx_db::repo::find_one::<Conversation>(conn, r#""id" = 70"#, &[])?
            .expect("conversation");
        let contact_inbox = chatbotx_db::repo::find_one::<ContactInbox>(
            conn,
            r#""id" = ?1"#,
            &[&contact_inbox_id],
        )?
        .expect("contact inbox");
        Ok((conversation, contact_inbox))
    })
    .await
    .expect("models")
}

async fn message_count(db: &common::TestDb) -> Option<i64> {
    common::scalar_i64(
        db,
        r#"SELECT COUNT(*) FROM "Message" WHERE "conversationId" = ?1"#,
        70,
    )
    .await
}

#[tokio::test]
async fn a_media_library_file_is_copied_into_the_conversation() {
    let db = seeded().await;
    let uploader = Uploader::in_memory();
    uploader
        .put_object("files/900", b"png".to_vec(), PutOptions::default(), None)
        .await
        .unwrap();
    let media = MediaStore {
        uploader: Some(&uploader),
        storage_url: STORAGE,
    };
    let (conversation, contact_inbox) = models(&db, 50).await;
    let input = OutgoingInput {
        text: Some("With a picture".into()),
        media_file_ids: vec![900],
        ..OutgoingInput::default()
    };
    let sent = create_outgoing_with_media(&db, media, &conversation, &contact_inbox, input, None)
        .await
        .expect("send");
    let message = sent.message.expect("message");
    assert_eq!(message.attachments.len(), 1);
    let attachment = &message.attachments[0];
    let prefix = format!("public/space/{WS}/conversations/70/");
    assert!(
        attachment.origin_path.starts_with(&prefix),
        "{}",
        attachment.origin_path
    );
    assert_eq!(
        (attachment.file_type.as_str(), attachment.mime_type.as_str()),
        ("image", "image/png")
    );
    assert_eq!(
        (attachment.name.as_deref(), attachment.size),
        (Some("logo"), 1)
    );
    let realtime = sent.realtime.expect("realtime");
    assert_eq!(
        realtime["attachments"][0]["url"],
        json!(format!("{STORAGE}{}", attachment.origin_path))
    );
    // The copy outlives the library object.
    uploader.delete_object("files/900").await.unwrap();
    assert_eq!(
        uploader
            .get_object(&attachment.origin_path)
            .await
            .unwrap()
            .as_ref(),
        b"png"
    );
}

#[tokio::test]
async fn a_foreign_or_unknown_file_is_refused() {
    let db = seeded().await;
    let uploader = Uploader::in_memory();
    let media = MediaStore {
        uploader: Some(&uploader),
        storage_url: STORAGE,
    };
    let (conversation, contact_inbox) = models(&db, 50).await;
    for ids in [vec![901], vec![900, 123]] {
        let input = OutgoingInput {
            text: Some("x".into()),
            media_file_ids: ids,
            ..OutgoingInput::default()
        };
        let error =
            create_outgoing_with_media(&db, media, &conversation, &contact_inbox, input, None)
                .await
                .unwrap_err();
        assert_eq!(
            (error.code(), error.status()),
            (Some("systemError"), Some(400))
        );
        assert_eq!(error.message(), "Media library file not found");
    }
    assert_eq!(message_count(&db).await, Some(0));
}

#[tokio::test]
async fn two_images_on_a_messenger_comment_reply_are_refused() {
    let db = seeded().await;
    let (conversation, contact_inbox) = models(&db, 52).await;
    let input = OutgoingInput {
        text: Some("Two pictures".into()),
        media_file_ids: vec![900, 902],
        reply_to_message_id: Some("1".into()),
        ..OutgoingInput::default()
    };
    let error = create_outgoing_with_media(
        &db,
        MediaStore::default(),
        &conversation,
        &contact_inbox,
        input,
        None,
    )
    .await
    .unwrap_err();
    let BusinessError::Exception {
        message,
        code,
        status,
        data,
        ..
    } = error
    else {
        panic!("not an exception");
    };
    assert_eq!(
        (message.as_str(), code.as_str(), status),
        (
            "The messenger channel can send at most 1 image(s) here",
            "imagesNotSupportedOnChannel",
            422
        )
    );
    assert_eq!(
        data,
        Some(json!({ "reason": "tooManyImages", "channel": "messenger", "max": 1 }))
    );
    assert_eq!(message_count(&db).await, Some(0));
}
