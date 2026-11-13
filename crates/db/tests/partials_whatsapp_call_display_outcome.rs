//! Port of `packages/database/__tests__/whatsapp-call-display-outcome.test.ts` (P3.2b,
//! ported).

use chatbotx_db::repo::partials::whatsapp_call::resolve_display_call_outcome;

fn display(status: &str, outcome: Option<&str>) -> Option<String> {
    resolve_display_call_outcome(status, outcome)
}

#[test]
fn resolve_display_call_outcome_coalesce_outcome_status_reader_contract_returns_the_persisted_outcome_when_present()
 {
    assert_eq!(
        display("failed", Some("canceled")).as_deref(),
        Some("canceled")
    );
}

#[test]
fn resolve_display_call_outcome_coalesce_outcome_status_reader_contract_falls_back_to_status_for_a_legacy_terminal_row_with_null_outcome()
 {
    assert_eq!(display("completed", None).as_deref(), Some("completed"));
    assert_eq!(display("rejected", None).as_deref(), Some("rejected"));
    assert_eq!(display("failed", None).as_deref(), Some("failed"));
}

#[test]
fn resolve_display_call_outcome_coalesce_outcome_status_reader_contract_never_fabricates_canceled_from_status_alone_no_status_equivalent_exists()
 {
    assert_eq!(display("failed", None).as_deref(), Some("failed"));
}

#[test]
fn resolve_display_call_outcome_coalesce_outcome_status_reader_contract_returns_null_for_a_non_terminal_row_with_no_outcome()
 {
    assert_eq!(display("ringing", None), None);
    assert_eq!(display("accepted", None), None);
}
