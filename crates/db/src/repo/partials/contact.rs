//! Port of `partials/contact.ts` (profile snapshot capability).

/// `profileSnapshotChannels`.
pub const PROFILE_SNAPSHOT_CHANNELS: &[&str] = &["instagram"];

/// `supportsProfileSnapshot`.
pub fn supports_profile_snapshot(channel: &str) -> bool {
    let _ = channel;
    todo!("P3.2b")
}
