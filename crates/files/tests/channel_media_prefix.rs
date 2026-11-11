//! Port of `packages/filesystem/__tests__/channel-media-prefix.test.ts` (P2.9).
//!
//! JS fakes the system clock (`vi.setSystemTime(2026-10-06T08:00:00.000Z)`).
//! Rust passes the UTC date of that instant to the `*_at` variants instead
//! (DECISIONS 2026-11-11 P2.9: clock injected). Assertions are unchanged.

use chatbotx_files::{
    ChannelMediaLocation, get_channel_media_prefix_at, get_mirrored_channel_media_prefix_at,
};
use chrono::NaiveDate;

fn location() -> ChannelMediaLocation {
    ChannelMediaLocation {
        channel: "messenger".to_owned(),
        workspace_id: "11624027755544576".to_owned(),
        integration_id: "11700090861420544".to_owned(),
    }
}

/// UTC date of the fake system time `2026-10-06T08:00:00.000Z`.
fn system_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 6).expect("valid date")
}

/// Port of: packages/filesystem/__tests__/channel-media-prefix.test.ts:23 "channel media prefixes > puts the channel first so one storage prefix covers a whole channel"
#[test]
fn channel_media_prefixes_puts_the_channel_first_so_one_storage_prefix_covers_a_whole_channel() {
    assert_eq!(
        get_channel_media_prefix_at(&location(), system_date()),
        "public/messenger/11624027755544576/11700090861420544/2026/10/06",
    );
}

/// Port of: packages/filesystem/__tests__/channel-media-prefix.test.ts:29 "channel media prefixes > keeps mirrored copies under the private root with the same layout"
#[test]
fn channel_media_prefixes_keeps_mirrored_copies_under_the_private_root_with_the_same_layout() {
    assert_eq!(
        get_mirrored_channel_media_prefix_at(&location(), system_date()),
        "workspace/messenger/11624027755544576/11700090861420544/2026/10/06",
    );
}
