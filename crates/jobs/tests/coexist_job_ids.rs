//! Port of `packages/worker-config/__tests__/coexist-job-ids.test.ts` (spec 5.1).

use std::collections::HashSet;

use chatbotx_jobs::queues::coexist::{
    coexist_flush_job_id, coexist_page_job_id, coexist_revive_job_suffix, coexist_run_job_id,
};

/// `/^-revive-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/` without a regex crate.
fn matches_uuid_suffix(suffix: &str) -> bool {
    let Some(uuid) = suffix.strip_prefix("-revive-") else {
        return false;
    };
    let groups: Vec<&str> = uuid.split('-').collect();
    let lengths = [8, 4, 4, 4, 12];
    groups.len() == lengths.len()
        && groups.iter().zip(lengths).all(|(group, len)| {
            group.len() == len
                && group
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

/// Port of: packages/worker-config/__tests__/coexist-job-ids.test.ts:18 "coexist job ids > the buffer flush id carries the v2 generation prefix"
#[test]
fn the_buffer_flush_id_carries_the_v2_generation_prefix() {
    assert_eq!(
        coexist_flush_job_id("15550001111"),
        "coexist-flush-v2-15550001111"
    );
}

/// Port of: packages/worker-config/__tests__/coexist-job-ids.test.ts:24 "coexist job ids > the flush id is stable per phone number so burst webhooks coalesce"
#[test]
fn the_flush_id_is_stable_per_phone_number_so_burst_webhooks() {
    assert_eq!(coexist_flush_job_id("pn-1"), coexist_flush_job_id("pn-1"));
    assert_ne!(coexist_flush_job_id("pn-1"), coexist_flush_job_id("pn-2"));
}

/// Port of: packages/worker-config/__tests__/coexist-job-ids.test.ts:31 "coexist job ids > a run job id pins run and attempts, and takes an optional suffix"
#[test]
fn a_run_job_id_pins_run_and_attempts_and_takes_an_optional() {
    assert_eq!(coexist_run_job_id("run-1", 2, None), "coexist-run-run-1-2");
    assert_eq!(
        coexist_run_job_id("run-1", 2, Some("-x")),
        "coexist-run-run-1-2-x"
    );
}

/// Port of: packages/worker-config/__tests__/coexist-job-ids.test.ts:40 "coexist job ids > a continuation id extends the run id with the next page"
#[test]
fn a_continuation_id_extends_the_run_id_with_the_next_page() {
    assert_eq!(
        coexist_page_job_id("run-1", 0, 3),
        "coexist-run-run-1-0-page-3"
    );
}

/// Port of: packages/worker-config/__tests__/coexist-job-ids.test.ts:52 "coexist job ids > revive suffixes keep the greppable prefix and never repeat"
#[test]
fn revive_suffixes_keep_the_greppable_prefix_and_never_repeat() {
    let suffixes: Vec<String> = (0..50).map(|_| coexist_revive_job_suffix()).collect();
    for suffix in &suffixes {
        assert!(matches_uuid_suffix(suffix), "{suffix}");
    }
    let unique: HashSet<&String> = suffixes.iter().collect();
    assert_eq!(unique.len(), suffixes.len());
}
