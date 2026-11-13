//! Port of `packages/database/__tests__/whatsapp-call-outcome.test.ts` (P3.2b, ported;
//! an absent `canceledByBusiness` is `false`).

use chatbotx_db::repo::partials::whatsapp_call::{
    OUTCOME_BY_TERMINAL_STATUS, WhatsappCallOutcome as Outcome,
    WhatsappCallTerminalStatus as Status, resolve_whatsapp_call_outcome,
    resolve_whatsapp_call_terminal_outcome_pair,
};

#[test]
fn resolve_whatsapp_call_outcome_completed_always_resolves_to_completed() {
    assert_eq!(
        resolve_whatsapp_call_outcome(Status::Completed, false),
        Outcome::Completed
    );
    assert_eq!(
        resolve_whatsapp_call_outcome(Status::Completed, true),
        Outcome::Completed
    );
}

#[test]
fn resolve_whatsapp_call_outcome_rejected_always_resolves_to_rejected() {
    assert_eq!(
        resolve_whatsapp_call_outcome(Status::Rejected, false),
        Outcome::Rejected
    );
}

#[test]
fn resolve_whatsapp_call_outcome_failed_resolves_to_failed_when_not_a_business_cancel() {
    assert_eq!(
        resolve_whatsapp_call_outcome(Status::Failed, false),
        Outcome::Failed
    );
}

#[test]
fn resolve_whatsapp_call_outcome_failed_resolves_to_canceled_only_when_canceled_by_business_is_true_the_single_place_the_cancel_rule_lives()
 {
    assert_eq!(
        resolve_whatsapp_call_outcome(Status::Failed, true),
        Outcome::Canceled
    );
    assert_eq!(Outcome::Canceled.as_str(), "canceled");
}

#[test]
fn resolve_whatsapp_call_outcome_outcome_by_terminal_status_mirrors_every_terminal_status_1_1_the_non_cancel_default()
 {
    assert_eq!(
        OUTCOME_BY_TERMINAL_STATUS,
        &[
            ("completed", "completed"),
            ("rejected", "rejected"),
            ("failed", "failed")
        ]
    );
    for (status, outcome) in [
        (Status::Completed, "completed"),
        (Status::Rejected, "rejected"),
        (Status::Failed, "failed"),
    ] {
        assert_eq!(
            resolve_whatsapp_call_outcome(status, false).as_str(),
            outcome
        );
    }
}

#[test]
fn resolve_whatsapp_call_terminal_outcome_pair_returns_the_matching_status_outcome_pair_for_completed_and_rejected()
 {
    assert_eq!(
        resolve_whatsapp_call_terminal_outcome_pair(Status::Completed, false),
        (Status::Completed, Outcome::Completed)
    );
    assert_eq!(
        resolve_whatsapp_call_terminal_outcome_pair(Status::Rejected, false),
        (Status::Rejected, Outcome::Rejected)
    );
}

#[test]
fn resolve_whatsapp_call_terminal_outcome_pair_failed_without_a_business_cancel_pairs_with_outcome_failed()
 {
    assert_eq!(
        resolve_whatsapp_call_terminal_outcome_pair(Status::Failed, false),
        (Status::Failed, Outcome::Failed)
    );
}

#[test]
fn resolve_whatsapp_call_terminal_outcome_pair_failed_with_a_business_cancel_pairs_with_outcome_canceled_status_still_failed()
 {
    assert_eq!(
        resolve_whatsapp_call_terminal_outcome_pair(Status::Failed, true),
        (Status::Failed, Outcome::Canceled)
    );
}
