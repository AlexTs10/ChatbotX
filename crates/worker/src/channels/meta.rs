//! Helpers the Meta channels (Messenger, Instagram, Instagram-Facebook,
//! Threads) share: `normalizeMetaAdReferral` + `deriveAdSourcePlatform`
//! (`packages/business/src/referral`), `toPendingIncomingAttachment`
//! (`packages/sdk/src/lib/shared/message.ts`), the Meta messaging windows
//! (`meta-messaging.ts`) and the canonical reply payload of a button.

use chatbotx_core::flow_config::util::{ButtonPayload, encode_button_payload, extract_metadata};
use serde_json::{Map, Value, json};

use crate::channel::{Category, ChannelError, IncomingAttachment, SendFlowStep};

/// `META_RESPONSE_WINDOW_MS`.
pub const META_RESPONSE_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
/// `META_HUMAN_AGENT_WINDOW_MS`.
pub const META_HUMAN_AGENT_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// `COMMENT_PRIVATE_REPLY_SPENT_CODE`.
pub const COMMENT_PRIVATE_REPLY_SPENT_CODE: &str = "comment_private_reply_already_used";

/// `MESSENGER_NATIVE_QUICK_REPLY`.
pub const NATIVE_USER_EMAIL: &str = "messenger:native-quick-reply:user_email";
pub const NATIVE_USER_PHONE_NUMBER: &str = "messenger:native-quick-reply:user_phone_number";

/// `isWithinMetaResponseWindow`.
pub fn is_within_response_window(last_incoming_at: Option<i64>, now_ms: i64) -> bool {
    last_incoming_at.is_some_and(|at| now_ms - at <= META_RESPONSE_WINDOW_MS)
}

/// `assertCommentPrivateReplyFollowUpDeliverable`.
pub fn assert_private_reply_follow_up(
    last_incoming_at: Option<i64>,
    now_ms: i64,
) -> Result<(), ChannelError> {
    if is_within_response_window(last_incoming_at, now_ms) {
        return Ok(());
    }
    Err(ChannelError::new(
        "Meta allows one private reply per comment. The rest of this flow needs the contact to message first — no reply from them in the last 24 hours.",
        Category::PayloadInvalid,
    )
    .with_code(COMMENT_PRIVATE_REPLY_SPENT_CODE))
}

/// `getCanonicalReplyPayload`.
pub fn canonical_reply_payload(button: &Value) -> String {
    let postback = button.get("postback").and_then(Value::as_str);
    let payload = if button.get("buttonType").and_then(Value::as_str) == Some("url") {
        postback.or_else(|| button.get("url").and_then(Value::as_str))
    } else {
        postback
    };
    payload.unwrap_or_default().to_owned()
}

/// `str` field or `""`.
pub fn str_field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// `encodeButtonPayload` of a flow step button (`getButtonTemplate`):
/// flow, version, button, broadcast, sequence step, contact inbox (when
/// given) and comment automation.
pub fn flow_button_payload(
    data: &SendFlowStep,
    button_id: &str,
    contact_inbox_id: Option<String>,
) -> String {
    let metadata = data.metadata.as_ref();
    encode_button_payload(&ButtonPayload {
        flow_id: data.flow_id.clone(),
        flow_version_id: data.flow_version_id.clone(),
        button_id: Some(button_id.to_owned()),
        broadcast_id: extract_metadata("broadcastId", metadata),
        sequence_step_id: extract_metadata("sequenceStepId", metadata),
        contact_inbox_id,
        comment_automation_id: extract_metadata("commentAutomationId", metadata),
        ..ButtonPayload::default()
    })
}

/// `EXTENSION_MIME_TYPES`.
fn mime_from_extension(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "heic" => "image/heic",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "csv" => "text/csv",
        "txt" => "text/plain",
        "zip" => "application/zip",
        _ => return None,
    })
}

/// `guessFileTypeFromMimeType`.
pub fn file_type_of_mime(mime: &str) -> &'static str {
    match mime.split('/').next().unwrap_or_default() {
        "image" => "image",
        "video" => "video",
        "audio" => "audio",
        _ => "file",
    }
}

/// `isDeferrableAttachmentType`.
pub fn is_deferrable_type(kind: &str) -> bool {
    matches!(kind, "image" | "video" | "audio" | "file")
}

/// `toPendingIncomingAttachment`: the provider URL kept as the origin
/// (no download). `None` = a file whose URL names no known type.
pub fn pending_attachment(url: &str, kind: &str) -> Option<IncomingAttachment> {
    let from_url = url::Url::parse(url).ok().and_then(|u| {
        let extension = u.path().rsplit('.').next()?.to_lowercase();
        mime_from_extension(&extension)
    });
    if kind == "file" && from_url.is_none() {
        return None;
    }
    let default = match kind {
        "image" => "image/jpeg",
        "video" => "video/mp4",
        "audio" => "audio/mpeg",
        _ => "application/octet-stream",
    };
    let mime = from_url
        .filter(|m| kind == "file" || file_type_of_mime(m) == kind)
        .unwrap_or(default);
    Some(IncomingAttachment {
        source_id: None,
        file_type: if kind == "file" {
            file_type_of_mime(mime).to_owned()
        } else {
            kind.to_owned()
        },
        mime_type: mime.to_owned(),
        origin_path: url.to_owned(),
        size: Some(0),
        width: None,
        height: None,
        name: None,
        is_photo_message: kind == "image",
        optimized: false,
    })
}

/// `deriveAdSourcePlatform`.
pub fn derive_ad_source_platform(source_url: Option<&str>) -> Option<&'static str> {
    let host = url::Url::parse(source_url?)
        .ok()?
        .host_str()?
        .to_lowercase();
    let on = |hosts: &[&str]| {
        hosts
            .iter()
            .any(|h| host == *h || host.ends_with(&format!(".{h}")))
    };
    if on(&["facebook.com", "fb.me", "fb.com", "fb.watch"]) {
        Some("facebook")
    } else if on(&["instagram.com", "ig.me"]) {
        Some("instagram")
    } else {
        None
    }
}

/// `normalizeMetaAdReferral`: the stored `MessageReferral` of a Meta
/// `referral` object (its `text` already removed by the caller).
pub fn normalize_meta_ad_referral(referral: &Value) -> Value {
    let text = |k: &str| referral.get(k).and_then(Value::as_str);
    let ads = |k: &str| {
        referral
            .pointer(&format!("/ads_context_data/{k}"))
            .and_then(Value::as_str)
    };
    let source_url = text("referer_uri").or_else(|| text("source_url"));
    let mut out = Map::new();
    if let Some(r) = referral.get("ref").filter(|r| !r.is_null()) {
        out.insert("ref".into(), r.clone());
    }
    out.insert("source".into(), json!(text("source")));
    out.insert("type".into(), json!(text("type")));
    out.insert("adId".into(), json!(text("ad_id")));
    out.insert("adTitle".into(), json!(ads("ad_title")));
    out.insert("sourceUrl".into(), json!(source_url));
    out.insert(
        "sourcePlatform".into(),
        json!(text("source_platform").or_else(|| derive_ad_source_platform(source_url))),
    );
    out.insert("postId".into(), json!(ads("post_id")));
    out.insert("photoUrl".into(), json!(ads("photo_url")));
    out.insert("videoUrl".into(), json!(ads("video_url")));
    out.insert("productId".into(), json!(ads("product_id")));
    out.insert("flowId".into(), json!(ads("flow_id")));
    out.insert("raw".into(), referral.clone());
    Value::Object(out)
}

/// Drops the `null` members of a JSON object (`JSON.stringify` of
/// `undefined` fields).
pub fn compact(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.retain(|_, v| !v.is_null());
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_attachment_keeps_the_url_and_types_it() {
        let image = pending_attachment("https://scontent.fbcdn.net/a/b.png?x=1", "image").unwrap();
        assert_eq!(
            (image.mime_type.as_str(), image.is_photo_message),
            ("image/png", true)
        );
        let video = pending_attachment("https://video.fbcdn.net/v", "video").unwrap();
        assert_eq!(video.mime_type, "video/mp4");
        assert!(pending_attachment("https://cdn.fbsbx.com/file", "file").is_none());
        let pdf = pending_attachment("https://cdn.fbsbx.com/doc.pdf", "file").unwrap();
        assert_eq!(
            (pdf.file_type.as_str(), pdf.mime_type.as_str()),
            ("file", "application/pdf")
        );
    }

    #[test]
    fn referral_is_normalized_with_the_platform() {
        let stored = normalize_meta_ad_referral(&json!({
            "source": "ADS", "type": "OPEN_THREAD", "ad_id": "1",
            "referer_uri": "https://m.facebook.com/ad", "ads_context_data": { "ad_title": "T" },
        }));
        assert_eq!(stored["sourcePlatform"], "facebook");
        assert_eq!(stored["adTitle"], "T");
        assert!(stored.get("ref").is_none());
        assert_eq!(
            derive_ad_source_platform(Some("https://ig.me/x")),
            Some("instagram")
        );
    }
}
