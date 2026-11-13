//! Port of the media type guards of `partials/messaging-ad.ts`. The stored
//! `MessagingAdCreativeMediaInput` is not re-validated on read, so the guards take the
//! stored JSON (the TS `in` checks are key checks).

use serde_json::Value;

/// `isLegacyImageMedia`: `kind == "image"` and an `imageHash` key.
pub fn is_legacy_image_media(media: &Value) -> bool {
    is_image_with(media, "imageHash")
}

/// `isStoredImageMedia`: `kind == "image"` and an `imageKey` key.
pub fn is_stored_image_media(media: &Value) -> bool {
    is_image_with(media, "imageKey")
}

fn is_image_with(media: &Value, key: &str) -> bool {
    media.get("kind").and_then(Value::as_str) == Some("image")
        && media
            .as_object()
            .is_some_and(|object| object.contains_key(key))
}
