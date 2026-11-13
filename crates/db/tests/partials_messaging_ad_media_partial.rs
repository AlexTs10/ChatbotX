//! Port of `packages/database/__tests__/messaging-ad-media-partial.test.ts` (P3.2b,
//! ported: the media input is the stored JSON).

use chatbotx_db::repo::partials::messaging_ad::{is_legacy_image_media, is_stored_image_media};
use serde_json::{Value, json};

fn legacy_image() -> Value {
    json!({ "kind": "image", "imageHash": "hash_1", "link": "https://example.com" })
}

fn stored_image() -> Value {
    json!({
        "kind": "image",
        "imageKey": "public/space/ws_1/ads-campaign/creatives/abc123",
        "fileId": "file_1",
        "link": "https://example.com",
    })
}

fn video() -> Value {
    json!({ "kind": "video", "videoId": "vid_1" })
}

#[test]
fn is_legacy_image_media_is_true_for_a_persisted_before_the_switch_row_with_an_image_hash() {
    assert!(is_legacy_image_media(&legacy_image()));
}

#[test]
fn is_legacy_image_media_is_false_for_a_stored_image_row_image_key_no_image_hash() {
    assert!(!is_legacy_image_media(&stored_image()));
}

#[test]
fn is_legacy_image_media_is_false_for_a_video() {
    assert!(!is_legacy_image_media(&video()));
}

#[test]
fn is_stored_image_media_is_true_for_a_presigned_s3_row_with_an_image_key() {
    assert!(is_stored_image_media(&stored_image()));
}

#[test]
fn is_stored_image_media_is_false_for_a_legacy_image_hash_row() {
    assert!(!is_stored_image_media(&legacy_image()));
}

#[test]
fn is_stored_image_media_is_false_for_a_video() {
    assert!(!is_stored_image_media(&video()));
}
