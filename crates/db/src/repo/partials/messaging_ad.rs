//! Port of the media type guards of `partials/messaging-ad.ts`. The stored
//! `MessagingAdCreativeMediaInput` is not re-validated on read, so the guards take the
//! stored JSON (the TS `in` checks are key checks).

use serde_json::Value;

/// `isLegacyImageMedia`: `kind == "image"` and an `imageHash` key.
pub fn is_legacy_image_media(media: &Value) -> bool {
    let _ = media;
    todo!("P3.2b")
}

/// `isStoredImageMedia`: `kind == "image"` and an `imageKey` key.
pub fn is_stored_image_media(media: &Value) -> bool {
    let _ = media;
    todo!("P3.2b")
}
