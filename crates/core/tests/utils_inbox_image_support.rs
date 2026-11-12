//! Port of `packages/utils/__tests__/inbox-image-support.test.ts`.

use chatbotx_core::utils::channel::ChannelType;
use chatbotx_core::utils::inbox_image_support::{
    InboxImageCheckResult, InboxReplyKind, check_inbox_images, get_inbox_image_limit,
    inbox_image_support, is_image_mime_type, resolve_inbox_reply_kind,
};

use ChannelType::{Instagram, Messenger, Threads, Tiktok};
use InboxReplyKind::{Comment, Message, PrivateReply};

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:12 "INBOX_IMAGE_SUPPORT > declares a limit for every channel"
#[test]
fn inbox_image_support_declares_a_limit_for_every_channel() {
    // A total `match` declares an entry for every channel.
    for channel in ChannelType::ALL {
        let _ = inbox_image_support(*channel);
    }
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:18 "INBOX_IMAGE_SUPPORT > blocks images on threads and on instagram/tiktok comment replies"
#[test]
fn inbox_image_support_blocks_images_on_threads_and_on_instagram_tiktok_comment_replies() {
    assert_eq!(get_inbox_image_limit(Threads, Message), 0);
    assert_eq!(get_inbox_image_limit(Threads, Comment), 0);
    assert_eq!(get_inbox_image_limit(Instagram, Comment), 0);
    assert_eq!(get_inbox_image_limit(Tiktok, Comment), 0);
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:27 "INBOX_IMAGE_SUPPORT > allows a single image on a messenger comment reply"
#[test]
fn inbox_image_support_allows_a_single_image_on_a_messenger_comment_reply() {
    assert_eq!(get_inbox_image_limit(Messenger, Comment), 1);
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:33 "INBOX_IMAGE_SUPPORT > treats messenger/instagram private replies like DMs, tiktok's carry nothing"
#[test]
fn inbox_image_support_treats_messenger_instagram_private_replies_like_dms_tiktok_s_carry_nothing()
{
    assert_eq!(get_inbox_image_limit(Messenger, PrivateReply), 10);
    assert_eq!(get_inbox_image_limit(Instagram, PrivateReply), 10);
    assert_eq!(get_inbox_image_limit(Tiktok, PrivateReply), 0);
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:47 "resolveInboxReplyKind > a private reply wins over its reply target"
#[test]
fn resolveinboxreplykind_a_private_reply_wins_over_its_reply_target() {
    assert_eq!(resolve_inbox_reply_kind(true, true), PrivateReply);
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:53 "resolveInboxReplyKind > a reply target without the private flag is a comment reply"
#[test]
fn resolveinboxreplykind_a_reply_target_without_the_private_flag_is_a_comment_reply() {
    assert_eq!(resolve_inbox_reply_kind(true, false), Comment);
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:59 "resolveInboxReplyKind > no reply target is a direct message"
#[test]
fn resolveinboxreplykind_no_reply_target_is_a_direct_message() {
    assert_eq!(resolve_inbox_reply_kind(false, false), Message);
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:67 "checkInboxImages > accepts a reply without images anywhere"
#[test]
fn checkinboximages_accepts_a_reply_without_images_anywhere() {
    assert_eq!(
        check_inbox_images(Threads, Comment, 0),
        InboxImageCheckResult::Ok
    );
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:73 "checkInboxImages > accepts images in a direct message thread"
#[test]
fn checkinboximages_accepts_images_in_a_direct_message_thread() {
    assert_eq!(
        check_inbox_images(Tiktok, Message, 3),
        InboxImageCheckResult::Ok
    );
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:79 "checkInboxImages > accepts images on an instagram private reply"
#[test]
fn checkinboximages_accepts_images_on_an_instagram_private_reply() {
    assert_eq!(
        check_inbox_images(Instagram, PrivateReply, 1),
        InboxImageCheckResult::Ok
    );
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:89 "checkInboxImages > rejects images where the channel cannot send them"
#[test]
fn checkinboximages_rejects_images_where_the_channel_cannot_send_them() {
    assert_eq!(
        check_inbox_images(Instagram, Comment, 1),
        InboxImageCheckResult::ImagesNotSupported { max: 0 }
    );
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:99 "checkInboxImages > rejects more images than the channel allows"
#[test]
fn checkinboximages_rejects_more_images_than_the_channel_allows() {
    assert_eq!(
        check_inbox_images(Messenger, Comment, 2),
        InboxImageCheckResult::TooManyImages { max: 1 }
    );
}

/// Port of: packages/utils/__tests__/inbox-image-support.test.ts:111 "isImageMimeType > recognises images only"
#[test]
fn isimagemimetype_recognises_images_only() {
    assert!(is_image_mime_type("image/png"));
    assert!(!is_image_mime_type("application/pdf"));
    assert!(!is_image_mime_type("video/mp4"));
}
