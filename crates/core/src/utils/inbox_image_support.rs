//! `packages/utils/src/inbox-image-support.ts`.

use super::channel::ChannelType;

/// `SAVED_REPLY_MAX_IMAGES`.
pub const SAVED_REPLY_MAX_IMAGES: usize = 10;

/// `isImageMimeType`.
pub fn is_image_mime_type(mime_type: &str) -> bool {
    mime_type.starts_with("image/")
}

string_enum! {
    /// `InboxReplyKind`.
    pub enum InboxReplyKind {
        Message = "message",
        Comment = "comment",
        PrivateReply = "privateReply",
    }
}

/// `INBOX_IMAGE_SUPPORT[channel]`: (message, comment, privateReply).
pub fn inbox_image_support(channel: ChannelType) -> (usize, usize, usize) {
    use ChannelType as C;
    match channel {
        C::Messenger => (10, 1, 10),
        C::Instagram => (10, 0, 10),
        C::Tiktok | C::Whatsapp | C::Telegram | C::Zalo | C::Webchat | C::Api | C::Omnichannel => {
            (10, 0, 0)
        }
        C::Threads | C::Smtp => (0, 0, 0),
    }
}

/// `resolveInboxReplyKind`: a private reply wins over its reply target.
pub fn resolve_inbox_reply_kind(has_reply_target: bool, is_private_reply: bool) -> InboxReplyKind {
    if is_private_reply {
        InboxReplyKind::PrivateReply
    } else if has_reply_target {
        InboxReplyKind::Comment
    } else {
        InboxReplyKind::Message
    }
}

/// `getInboxImageLimit`.
pub fn get_inbox_image_limit(channel: ChannelType, kind: InboxReplyKind) -> usize {
    let (message, comment, private_reply) = inbox_image_support(channel);
    match kind {
        InboxReplyKind::Message => message,
        InboxReplyKind::Comment => comment,
        InboxReplyKind::PrivateReply => private_reply,
    }
}

/// `InboxImageCheckResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxImageCheckResult {
    Ok,
    ImagesNotSupported { max: usize },
    TooManyImages { max: usize },
}

/// `checkInboxImages`.
pub fn check_inbox_images(
    channel: ChannelType,
    kind: InboxReplyKind,
    image_count: usize,
) -> InboxImageCheckResult {
    let max = get_inbox_image_limit(channel, kind);
    if image_count == 0 {
        InboxImageCheckResult::Ok
    } else if max == 0 {
        InboxImageCheckResult::ImagesNotSupported { max }
    } else if image_count > max {
        InboxImageCheckResult::TooManyImages { max }
    } else {
        InboxImageCheckResult::Ok
    }
}
