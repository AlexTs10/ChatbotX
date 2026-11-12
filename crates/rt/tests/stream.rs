//! Port of `packages/realtime-protocol/__tests__/stream.test.ts` (P2.8 spec
//! 4.6, R94-R99). R94 (Redis hash-tagged shard numbers) is not applicable
//! (user decision 2026-11-12, DECISIONS P2.8): one buffer per workspace.

use chatbotx_rt::protocol::{is_realtime_seq_after, parse_stream_record};
use serde_json::json;

/// Port of: packages/realtime-protocol/__tests__/stream.test.ts:19 "member revoke stream records > requires the original revocation cutoff"
#[test]
fn member_revoke_stream_records_requires_the_original_revocation_cutoff() {
    let record = json!({
        "kind": "member-revoke",
        "reason": "reauth",
        "revokedAtMs": 1000,
        "userId": "user-1",
        "workspaceId": "workspace-1",
    });

    let parsed = parse_stream_record(&record).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), record);

    let mut without_cutoff = record.clone();
    without_cutoff
        .as_object_mut()
        .unwrap()
        .remove("revokedAtMs");
    assert!(parse_stream_record(&without_cutoff).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/stream.test.ts:38 "isRealtimeSeqAfter > a newer millisecond component wins regardless of sequence"
#[test]
fn isrealtimeseqafter_a_newer_millisecond_component_wins_regardless_of_sequence() {
    assert!(is_realtime_seq_after("11-0", "10-5"));
    assert!(!is_realtime_seq_after("10-5", "11-0"));
}

/// Port of: packages/realtime-protocol/__tests__/stream.test.ts:43 "isRealtimeSeqAfter > an equal millisecond component compares the sequence"
#[test]
fn isrealtimeseqafter_an_equal_millisecond_component_compares_the_sequence() {
    assert!(is_realtime_seq_after("10-5", "10-4"));
    assert!(!is_realtime_seq_after("10-4", "10-5"));
}

/// Port of: packages/realtime-protocol/__tests__/stream.test.ts:48 "isRealtimeSeqAfter > an identical id is not after itself"
#[test]
fn isrealtimeseqafter_an_identical_id_is_not_after_itself() {
    assert!(!is_realtime_seq_after("10-5", "10-5"));
}

/// Port of: packages/realtime-protocol/__tests__/stream.test.ts:52 "isRealtimeSeqAfter > compares the numeric value, not the string, of each component"
#[test]
fn isrealtimeseqafter_compares_the_numeric_value_not_the_string_of_each_component() {
    // A string comparison would rank "9-0" after "10-0".
    assert!(is_realtime_seq_after("10-0", "9-0"));
    assert!(!is_realtime_seq_after("9-0", "10-0"));
}
