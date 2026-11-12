//! Port of `packages/variables/__tests__/last-input.test.ts`.

mod support;

use chatbotx_core::variables::*;
use support::variables::*;
use support::{iso_ms, now_ms};

fn harness() -> Harness {
    Harness::new(FakeState {
        latest_conversation: Some(ConversationModel {
            id: "conversation-1".into(),
            workspace_id: "workspace-1".into(),
            last_activity_at: Some(now_ms()),
            created_at: Some(now_ms()),
            ..Default::default()
        }),
        tenant: TenantSettings {
            storage_url: "https://cdn.example/storage/".into(),
            ..Default::default()
        },
        ..Default::default()
    })
}

fn text_message(text: Option<&str>, attachments: Vec<Attachment>) -> MessageWithAttachments {
    MessageWithAttachments {
        content_type: "text".into(),
        text: text.map(str::to_owned),
        attachments,
        ..Default::default()
    }
}

fn attachment(id: &str, file_type: &str, origin_path: &str, at: Option<i64>) -> Attachment {
    Attachment {
        id: id.into(),
        file_type: file_type.into(),
        origin_path: origin_path.into(),
        message_created_at: at,
    }
}

/// Port of: packages/variables/__tests__/last-input.test.ts:71 "last input helpers > last_input returns latest text message text"
#[tokio::test]
async fn last_input_helpers_last_input_returns_latest_text_message_text() {
    let h = harness();
    h.fake
        .set(|st| st.latest_input = Some(text_message(Some("latest incoming text"), vec![])));
    assert_eq!(
        h.v().get_contact_last_input("contact-1").await.unwrap(),
        s("latest incoming text")
    );
    let call = h
        .fake
        .last_call_to("find_latest_incoming_message_with_attachments")
        .unwrap();
    assert!(call.starts_with("find_latest_incoming_message_with_attachments(conversation-1, "));
    assert!(call.ends_with(", workspace-1)"));
}

/// Port of: packages/variables/__tests__/last-input.test.ts:89 "last input helpers > last_input returns a public URL for media messages"
#[tokio::test]
async fn last_input_helpers_last_input_returns_a_public_url_for_media_messages() {
    let h = harness();
    h.fake.set(|st| {
        st.latest_input = Some(text_message(
            None,
            vec![attachment(
                "",
                "image",
                "public/space/workspace-1/messages/image.png",
                None,
            )],
        ))
    });
    assert_eq!(
        h.v().get_contact_last_input("contact-1").await.unwrap(),
        s("https://cdn.example/storage/public/space/workspace-1/messages/image.png")
    );
}

/// Port of: packages/variables/__tests__/last-input.test.ts:106 "last input helpers > last_input stores a lazily-ingested attachment and returns the stored object's URL"
#[tokio::test]
async fn last_input_helpers_last_input_stores_a_lazily_ingested_attachment_and_returns_the_stored_object_s_url()
 {
    let h = harness();
    let at = iso_ms("2026-10-14T01:00:00.000Z");
    h.fake.set(|st| {
        st.latest_input = Some(text_message(
            None,
            vec![attachment(
                "att-1",
                "image",
                "https://scontent.example/photo.jpg?sig=abc",
                Some(at),
            )],
        ));
        st.ensure_stored = EnsureStored::Answer(Ok(s(
            "workspace/messenger/workspace-1/int-1/2026/10/14/abc.jpg",
        )));
    });
    assert_eq!(
        h.v().get_contact_last_input("contact-1").await.unwrap(),
        s("https://cdn.example/storage/workspace/messenger/workspace-1/int-1/2026/10/14/abc.jpg")
    );
    assert!(h.fake.called(
        "ensure_attachment_stored(att-1, workspace-1, 2026-10-14T01:00:00.000Z, https://scontent.example/photo.jpg?sig=abc)"
    ));
}

/// Port of: packages/variables/__tests__/last-input.test.ts:134 "last input helpers > last_input is null when the attachment is permanently unavailable"
#[tokio::test]
async fn last_input_helpers_last_input_is_null_when_the_attachment_is_permanently_unavailable() {
    let h = harness();
    h.fake.set(|st| {
        st.latest_input = Some(text_message(
            None,
            vec![attachment(
                "att-1",
                "image",
                "failed:unresolvable",
                Some(now_ms()),
            )],
        ));
        st.ensure_stored = EnsureStored::Answer(Ok(None));
    });
    assert_eq!(
        h.v().get_contact_last_input("contact-1").await.unwrap(),
        None
    );
}

/// Port of: packages/variables/__tests__/last-input.test.ts:152 "last input helpers > last_input falls back to the media proxy URL when the attachment is not stored in time"
#[tokio::test]
async fn last_input_helpers_last_input_falls_back_to_the_media_proxy_url_when_the_attachment_is_not_stored_in_time()
 {
    let h = harness();
    let at = now_ms();
    h.fake.set(|st| {
        st.latest_input = Some(MessageWithAttachments {
            contact_inbox_id: "ci-1".into(),
            ..text_message(
                None,
                vec![attachment(
                    "att-1",
                    "image",
                    "https://scontent.example/photo.jpg",
                    Some(at),
                )],
            )
        });
        st.ensure_stored = EnsureStored::Answer(Err("not ready".into()));
        st.contact_inbox = Ok(Some(ContactInboxModel {
            id: "ci-1".into(),
            channel: "messenger".into(),
            ..Default::default()
        }));
        st.attachment_media_url = Ok(s("https://app.example/media/attachment/token"));
    });
    assert_eq!(
        h.v().get_contact_last_input("contact-1").await.unwrap(),
        s("https://app.example/media/attachment/token")
    );
    assert!(h.fake.called("find_contact_inbox(ci-1)"));
    assert!(h.fake.called(&format!(
        "resolve_attachment_media_url(att-1, workspace-1, https://scontent.example/photo.jpg, messenger, {})",
        support::iso(at)
    )));
}

/// Port of: packages/variables/__tests__/last-input.test.ts:194 "last input helpers > last_input is null instead of throwing when neither the store nor the proxy fallback works"
#[tokio::test]
async fn last_input_helpers_last_input_is_null_instead_of_throwing_when_neither_the_store_nor_the_proxy_fallback_works()
 {
    let h = harness();
    h.fake.set(|st| {
        st.latest_input = Some(MessageWithAttachments {
            contact_inbox_id: "ci-1".into(),
            ..text_message(
                None,
                vec![attachment(
                    "att-1",
                    "image",
                    "https://scontent.example/photo.jpg",
                    Some(now_ms()),
                )],
            )
        });
        st.ensure_stored = EnsureStored::Answer(Err("not ready".into()));
        st.contact_inbox = Err("db down".into());
    });
    assert_eq!(
        h.v().get_contact_last_input("contact-1").await.unwrap(),
        None
    );
}

/// Port of: packages/variables/__tests__/last-input.test.ts:214 "last input helpers > last_input and last_input_type return null when no message exists"
#[tokio::test]
async fn last_input_helpers_last_input_and_last_input_type_return_null_when_no_message_exists() {
    let h = harness();
    assert_eq!(
        h.v().get_contact_last_input("contact-1").await.unwrap(),
        None
    );
    assert_eq!(
        h.v()
            .get_contact_last_input_type("contact-1")
            .await
            .unwrap(),
        None
    );
}

/// Port of: packages/variables/__tests__/last-input.test.ts:221 "last input helpers > last_input returns null when the contact has no conversation"
#[tokio::test]
async fn last_input_helpers_last_input_returns_null_when_the_contact_has_no_conversation() {
    let h = harness();
    h.fake.set(|st| st.latest_conversation = None);
    assert_eq!(
        h.v().get_contact_last_input("contact-1").await.unwrap(),
        None
    );
    assert_eq!(
        h.fake
            .count("find_latest_incoming_message_with_attachments"),
        0
    );
}

/// Port of: packages/variables/__tests__/last-input.test.ts:228 "last input helpers > last_input resolves the conversation the contact is most recently active in"
#[tokio::test]
async fn last_input_helpers_last_input_resolves_the_conversation_the_contact_is_most_recently_active_in()
 {
    let h = harness();
    h.fake
        .set(|st| st.latest_input = Some(text_message(Some("latest incoming text"), vec![])));
    h.v().get_contact_last_input("contact-1").await.unwrap();
    assert!(
        h.fake
            .called("find_latest_conversation_by_contact(contact-1)")
    );
}

/// Port of: packages/variables/__tests__/last-input.test.ts:242 "last input helpers > last_input_type returns latest message content type"
#[tokio::test]
async fn last_input_helpers_last_input_type_returns_latest_message_content_type() {
    let h = harness();
    h.fake
        .set(|st| st.latest_input = Some(text_message(Some("latest incoming text"), vec![])));
    assert_eq!(
        h.v()
            .get_contact_last_input_type("contact-1")
            .await
            .unwrap(),
        s("text")
    );
}

/// Port of: packages/variables/__tests__/last-input.test.ts:254 "last input helpers > last_input_type returns the attachment file type for media messages"
#[tokio::test]
async fn last_input_helpers_last_input_type_returns_the_attachment_file_type_for_media_messages() {
    let h = harness();
    h.fake.set(|st| {
        st.latest_input = Some(text_message(
            None,
            vec![attachment("", "image", "image.png", None)],
        ))
    });
    assert_eq!(
        h.v()
            .get_contact_last_input_type("contact-1")
            .await
            .unwrap(),
        s("image")
    );
}

/// Port of: packages/variables/__tests__/last-input.test.ts:279 "last_text_input helper > returns the text of the latest incoming text message"
#[tokio::test]
async fn last_text_input_helper_returns_the_text_of_the_latest_incoming_text_message() {
    let h = harness();
    h.fake
        .set(|st| st.latest_input = Some(text_message(Some("typed by the contact"), vec![])));
    assert_eq!(
        h.v()
            .get_contact_last_text_input("contact-1")
            .await
            .unwrap(),
        s("typed by the contact")
    );
}

/// Port of: packages/variables/__tests__/last-input.test.ts:291 "last_text_input helper > never returns a media URL: an attachment message has no text input"
#[tokio::test]
async fn last_text_input_helper_never_returns_a_media_url_an_attachment_message_has_no_text_input()
{
    let h = harness();
    h.fake.set(|st| {
        st.latest_input = Some(text_message(
            None,
            vec![attachment(
                "",
                "image",
                "public/space/workspace-1/messages/image.png",
                None,
            )],
        ))
    });
    assert_eq!(
        h.v()
            .get_contact_last_text_input("contact-1")
            .await
            .unwrap(),
        None
    );
}

/// Port of: packages/variables/__tests__/last-input.test.ts:306 "last_text_input helper > returns null when there is no incoming message or no conversation"
#[tokio::test]
async fn last_text_input_helper_returns_null_when_there_is_no_incoming_message_or_no_conversation()
{
    let h = harness();
    assert_eq!(
        h.v()
            .get_contact_last_text_input("contact-1")
            .await
            .unwrap(),
        None
    );
    h.fake.set(|st| st.latest_conversation = None);
    assert_eq!(
        h.v()
            .get_contact_last_text_input("contact-1")
            .await
            .unwrap(),
        None
    );
}
