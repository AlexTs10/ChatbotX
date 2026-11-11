//! Port of `apps/worker/__tests__/job-attempts.test.ts`
//! (`isFinalAttempt` / `hasExhaustedAttempts`, spec 2.6).

use chatbotx_jobs::{has_exhausted_attempts, is_final_attempt};

/// Port of: apps/worker/__tests__/job-attempts.test.ts:15 "isFinalAttempt — called from INSIDE a handler > the last of three attempts is final"
#[test]
fn the_last_of_three_attempts_is_final() {
    assert!(is_final_attempt(2, Some(3)));
}

/// Port of: apps/worker/__tests__/job-attempts.test.ts:19 "isFinalAttempt — called from INSIDE a handler > an earlier attempt is not final"
#[test]
fn an_earlier_attempt_is_not_final() {
    assert!(!is_final_attempt(1, Some(3)));
}

/// Port of: apps/worker/__tests__/job-attempts.test.ts:23 "isFinalAttempt — called from INSIDE a handler > a job with no configured attempts gets exactly one, which is final"
#[test]
fn a_job_with_no_configured_attempts_gets_exactly_one_which_is() {
    assert!(is_final_attempt(0, None));
}

/// Port of: apps/worker/__tests__/job-attempts.test.ts:33 "hasExhaustedAttempts — called from a `failed` EVENT > a job is not exhausted while retries remain"
#[test]
fn a_job_is_not_exhausted_while_retries_remain() {
    assert!(!has_exhausted_attempts(1, Some(3)));
    assert!(!has_exhausted_attempts(2, Some(3)));
}

/// Port of: apps/worker/__tests__/job-attempts.test.ts:38 "hasExhaustedAttempts — called from a `failed` EVENT > a job is exhausted once every attempt has failed"
#[test]
fn a_job_is_exhausted_once_every_attempt_has_failed() {
    assert!(has_exhausted_attempts(3, Some(3)));
}

/// Port of: apps/worker/__tests__/job-attempts.test.ts:42 "hasExhaustedAttempts — called from a `failed` EVENT > a job with no configured attempts is exhausted after its first failure"
#[test]
fn a_job_with_no_configured_attempts_is_exhausted_after_its() {
    assert!(has_exhausted_attempts(1, None));
}

/// Port of: apps/worker/__tests__/job-attempts.test.ts:46 "hasExhaustedAttempts — called from a `failed` EVENT > the two helpers disagree by exactly one attempt — the reason both exist"
#[test]
fn the_two_helpers_disagree_by_exactly_one_attempt_the_reason() {
    let (made, attempts) = (2, Some(3));
    assert!(is_final_attempt(made, attempts));
    assert!(!has_exhausted_attempts(made, attempts));
}
