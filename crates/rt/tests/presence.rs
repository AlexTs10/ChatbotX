//! Port of `packages/realtime-protocol/__tests__/presence.test.ts` (P2.8
//! spec 4.6, R40-R45). R43-R45 (`hashPresenceUserIds`) are not applicable
//! (user decision 2026-11-12, DECISIONS P2.8): the hash only bound the
//! removed realtime -> builder HTTP report.

use chatbotx_rt::protocol::{
    MAX_PRESENCE_USER_IDS_PER_REPORT, PRESENCE_REPORT_INTERVAL_MS, PRESENCE_TTL_MS,
    truncate_presence_user_ids,
};

/// Port of: packages/realtime-protocol/__tests__/presence.test.ts:13 "presence config invariant > keeps the report interval at most half the TTL — one slow/lost report must never flap a member offline (HIGH-1)"
#[test]
fn presence_config_invariant_keeps_the_report_interval_at_most_half_the_ttl_one_slow_lost_report_must_never_flap_a_member_offline_high_1()
 {
    let interval = PRESENCE_REPORT_INTERVAL_MS;
    assert!(interval * 2 <= PRESENCE_TTL_MS);
}

/// Port of: packages/realtime-protocol/__tests__/presence.test.ts:19 "truncatePresenceUserIds > passes a batch under the cap through unchanged"
#[test]
fn truncatepresenceuserids_passes_a_batch_under_the_cap_through_unchanged() {
    assert_eq!(
        truncate_presence_user_ids(&["a".to_owned(), "b".to_owned()]),
        vec!["a".to_owned(), "b".to_owned()]
    );
}

/// Port of: packages/realtime-protocol/__tests__/presence.test.ts:23 "truncatePresenceUserIds > truncates a batch over the cap instead of rejecting it (LOW-7)"
#[test]
fn truncatepresenceuserids_truncates_a_batch_over_the_cap_instead_of_rejecting_it_low_7() {
    let user_ids: Vec<String> = (0..MAX_PRESENCE_USER_IDS_PER_REPORT + 10)
        .map(|i| format!("u_{i}"))
        .collect();

    let result = truncate_presence_user_ids(&user_ids);

    assert_eq!(result.len(), MAX_PRESENCE_USER_IDS_PER_REPORT);
    assert_eq!(
        result,
        user_ids[..MAX_PRESENCE_USER_IDS_PER_REPORT].to_vec()
    );
}
