//! Port of `packages/business/src/media/__tests__/resolve-media-url.test.ts`
//! (Q.P6-U1), the attachment half (`resolveAttachmentFallbackUrl`,
//! `resolveMediaUrl({ kind: "attachment" })`). Adapted: the mocked signer is
//! the process signer, so the token is verified instead of compared. The
//! avatar half is `contact::avatar`'s.

use chatbotx_business::media_url::{
    AttachmentRef, FALLBACK_TOKEN_TTL_MS, resolve_attachment_fallback_url, resolve_attachment_url,
    verify_media_token,
};
use chatbotx_core::encryption::{MediaTokenClaims, MediaTokenKind};

const APP: &str = "https://builder.example.com";

fn attachment<'a>(origin_path: &'a str, channel: &'a str) -> AttachmentRef<'a> {
    AttachmentRef {
        workspace_id: 1,
        attachment_id: 7,
        origin_path,
        channel,
        message_created_at: None,
    }
}

fn claims_of(url: &str) -> MediaTokenClaims {
    let token = url
        .strip_prefix("https://builder.example.com/media/attachment/")
        .expect("proxy url");
    verify_media_token(token).expect("valid token")
}

#[test]
fn fallback_returns_a_signed_proxy_url_for_a_mirrored_hydration_channel_attachment() {
    let at = 1_789_862_400_000; // 2026-09-20T00:00:00Z
    let r = AttachmentRef {
        message_created_at: Some(at),
        ..attachment("workspace/media/image.jpg", "messenger")
    };
    let url = resolve_attachment_fallback_url(&r, APP).expect("url");
    assert_eq!(
        claims_of(&url),
        MediaTokenClaims {
            workspace_id: "1".into(),
            kind: MediaTokenKind::Attachment,
            ref_id: "7".into(),
            message_created_at: Some(at),
        }
    );
    assert_eq!(FALLBACK_TOKEN_TTL_MS, 24 * 60 * 60 * 1000);
}

#[test]
fn fallback_returns_a_proxy_fallback_for_pending_attachments_so_a_failed_first_load_can_recover() {
    for (origin_path, channel) in [
        ("https://lookaside.facebook.com/media", "messenger"),
        ("wa-media:media-1", "whatsapp"),
    ] {
        let url = resolve_attachment_fallback_url(&attachment(origin_path, channel), APP);
        assert!(
            url.is_some_and(|u| u.starts_with("https://builder.example.com/media/attachment/")),
            "{origin_path}"
        );
    }
}

#[test]
fn fallback_returns_null_for_a_failed_attachment_or_a_channel_without_hydration() {
    for (origin_path, channel) in [
        ("failed:unresolvable", "instagram"),
        ("ws-1/files/a.png", "telegram"),
    ] {
        assert!(
            resolve_attachment_fallback_url(&attachment(origin_path, channel), APP).is_none(),
            "{origin_path}"
        );
    }
}

#[tokio::test]
async fn finalizes_a_mirrored_attachment_using_the_callers_url_form() {
    let r = attachment("workspace/media/image.jpg", "messenger");
    let url = resolve_attachment_url(
        &r,
        APP,
        |key| async move { Some(format!("presigned:{key}")) },
    )
    .await;
    assert_eq!(url.as_deref(), Some("presigned:workspace/media/image.jpg"));
}

#[tokio::test]
async fn returns_a_signed_proxy_url_for_a_pending_hydration_channel_attachment() {
    let r = attachment("https://lookaside.facebook.com/media", "instagram");
    let url = resolve_attachment_url(&r, APP, |_| async { Some("finalized".to_string()) })
        .await
        .expect("url");
    let claims = claims_of(&url);
    assert_eq!(claims.ref_id, "7");
    assert_eq!(claims.message_created_at, None);
}

#[tokio::test]
async fn finalizes_an_external_url_for_a_non_hydration_channel() {
    let r = attachment("https://cdn.tiktok.example/media.jpg", "tiktok");
    let url =
        resolve_attachment_url(&r, APP, |key| async move { Some(format!("final:{key}")) }).await;
    assert_eq!(
        url.as_deref(),
        Some("final:https://cdn.tiktok.example/media.jpg")
    );
}

#[tokio::test]
async fn returns_null_for_a_permanently_failed_attachment() {
    let r = attachment("failed:unresolvable", "messenger");
    let url = resolve_attachment_url(&r, APP, |_| async { Some("finalized".to_string()) }).await;
    assert!(url.is_none());
}

#[tokio::test]
async fn returns_a_signed_proxy_url_for_pending_whatsapp_media() {
    let r = attachment("wa-media:media-id-1", "whatsapp");
    let url = resolve_attachment_url(&r, APP, |_| async { Some("finalized".to_string()) })
        .await
        .expect("url");
    assert_eq!(claims_of(&url).kind, MediaTokenKind::Attachment);
}
