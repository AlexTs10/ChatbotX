//! Port of `packages/business/__tests__/google-ads-processing-status.test.ts`.

use chatbotx_business::google_ads_api::RequestStatus;
use chatbotx_business::google_ads_housekeeping::{
    Outcome, classify_request_status, is_retryable_processing_reason,
};

fn status(request_status: &str, reasons: &[&str]) -> RequestStatus {
    RequestStatus {
        request_status: request_status.into(),
        record_count: Some(1),
        error_counts: reasons.iter().map(|r| ((*r).to_owned(), 1)).collect(),
        warning_counts: Vec::new(),
    }
}

fn is_failed(outcome: &Outcome<'_>, retryable: bool) -> bool {
    matches!(outcome, Outcome::Failed { retryable: r, .. } if *r == retryable)
}

/// Port of: "isRetryableProcessingReason > retries the documented prefixed transient reasons"
#[test]
fn retries_the_documented_prefixed_transient_reasons() {
    assert!(is_retryable_processing_reason(
        "PROCESSING_ERROR_REASON_INTERNAL_ERROR"
    ));
    assert!(is_retryable_processing_reason(
        "PROCESSING_ERROR_REASON_TOO_RECENT_CLICK"
    ));
}

/// Port of: "isRetryableProcessingReason > also accepts the unprefixed form"
#[test]
fn also_accepts_the_unprefixed_form() {
    assert!(is_retryable_processing_reason("INTERNAL_ERROR"));
    assert!(is_retryable_processing_reason("TOO_RECENT_CLICK"));
    assert!(is_retryable_processing_reason(
        "PROCESSING_ERROR_REASON_DESTINATION_TOO_RECENTLY_CREATED"
    ));
}

/// Port of: "isRetryableProcessingReason > does not retry terminal reasons"
#[test]
fn does_not_retry_terminal_reasons() {
    for reason in [
        "PROCESSING_ERROR_REASON_INVALID_GCLID",
        "PROCESSING_ERROR_REASON_EVENT_TOO_OLD",
        "PROCESSING_ERROR_REASON_DUPLICATE_GCLID",
        "PROCESSING_ERROR_REASON_DENIED_CONSENT",
        "PROCESSING_ERROR_REASON_CLICK_NOT_FOUND",
        "PROCESSING_ERROR_REASON_UNSPECIFIED",
        "DEADLINE_EXCEEDED",
        "",
    ] {
        assert!(!is_retryable_processing_reason(reason), "{reason}");
    }
}

/// Port of: "classifyRequestStatus > SUCCESS" / "PROCESSING"
#[test]
fn success_and_processing() {
    let success = [status("SUCCESS", &[])];
    assert_eq!(
        classify_request_status(&success),
        Outcome::Success(&success[0])
    );
    assert_eq!(
        classify_request_status(&[status("PROCESSING", &[])]),
        Outcome::Processing
    );
}

/// Port of: "classifyRequestStatus > FAILED with only transient reasons is retryable"
#[test]
fn failed_with_only_transient_reasons_is_retryable() {
    let failed = [status(
        "FAILED",
        &[
            "PROCESSING_ERROR_REASON_INTERNAL_ERROR",
            "PROCESSING_ERROR_REASON_TOO_RECENT_CLICK",
        ],
    )];
    assert_eq!(
        classify_request_status(&failed),
        Outcome::Failed {
            status: &failed[0],
            retryable: true
        }
    );
}

/// Port of: "FAILED with any permanent reason is not retryable", "FAILED with
/// EVENT_TOO_OLD is terminal, not retryable", "FAILED without error counts is
/// not retryable", "PARTIAL_SUCCESS is a failure for a one-event request"
#[test]
fn permanent_or_missing_reasons_are_terminal_failures() {
    for failed in [
        status(
            "FAILED",
            &[
                "PROCESSING_ERROR_REASON_INTERNAL_ERROR",
                "PROCESSING_ERROR_REASON_INVALID_GCLID",
            ],
        ),
        status("FAILED", &["PROCESSING_ERROR_REASON_EVENT_TOO_OLD"]),
        status("FAILED", &[]),
        status(
            "PARTIAL_SUCCESS",
            &["PROCESSING_ERROR_REASON_INVALID_GCLID"],
        ),
    ] {
        let list = [failed];
        assert!(
            is_failed(&classify_request_status(&list), false),
            "{list:?}"
        );
    }
}

/// Port of: "FAILED with only duplicate reasons (%s) is a duplicate"
#[test]
fn only_duplicate_reasons_is_a_duplicate() {
    for reasons in [
        &["DUPLICATE_TRANSACTION_ID"][..],
        &["PROCESSING_ERROR_REASON_DUPLICATE_TRANSACTION_ID"],
        &["DUPLICATE_GCLID"],
        &["PROCESSING_ERROR_REASON_DUPLICATE_GCLID"],
        &[
            "DUPLICATE_GCLID",
            "PROCESSING_ERROR_REASON_DUPLICATE_TRANSACTION_ID",
        ],
    ] {
        for kind in ["FAILED", "PARTIAL_SUCCESS"] {
            let list = [status(kind, reasons)];
            assert_eq!(classify_request_status(&list), Outcome::Duplicate(&list[0]));
        }
    }
}

/// Port of: "a duplicate mixed with %s keeps the other reason's precedence"
#[test]
fn a_duplicate_mixed_with_another_reason_keeps_its_precedence() {
    for other in [
        "PROCESSING_ERROR_REASON_DENIED_CONSENT",
        "INVALID_CLICK",
        "INTERNAL_ERROR",
    ] {
        let list = [status("FAILED", &["DUPLICATE_TRANSACTION_ID", other])];
        assert!(matches!(
            classify_request_status(&list),
            Outcome::Failed { .. }
        ));
    }
}

/// Port of: "a SUCCESS status is never a duplicate"
#[test]
fn a_success_status_is_never_a_duplicate() {
    let list = [status("SUCCESS", &["DUPLICATE_GCLID"])];
    assert!(matches!(
        classify_request_status(&list),
        Outcome::Success(_)
    ));
}

/// Port of: "zero entries is unknown", "multiple entries is unknown", "an unrecognised status is unknown"
#[test]
fn zero_several_or_unrecognised_entries_are_unknown() {
    assert_eq!(classify_request_status(&[]), Outcome::Unknown);
    assert_eq!(
        classify_request_status(&[status("SUCCESS", &[]), status("SUCCESS", &[])]),
        Outcome::Unknown
    );
    assert_eq!(
        classify_request_status(&[status("REQUEST_STATUS_UNKNOWN", &[])]),
        Outcome::Unknown
    );
}
