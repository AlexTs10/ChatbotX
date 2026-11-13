//! Port of the `getStats` case of
//! `apps/builder/__tests__/pipelines-public-api.test.ts` (line 367).
//! Adapted: `pipelineStatsPublicRequest.safeParse(...).success` is
//! `handlers::pipelines::stats_query(...).is_ok()` (path params + raw query).

use chatbotx_api::handlers::pipelines::stats_query;

fn accepted(query: &str) -> bool {
    let params = [("id", "1".to_string())];
    stats_query(&params, Some(query)).is_ok()
}

#[test]
fn stats_dates_must_be_yyyy_mm_dd_instead_of_silently_falling_back() {
    assert!(!accepted("from=2026%2F01%2F01&to=2026-01-31"));
    assert!(accepted("from=2026-01-01&to=2026-01-31"));
}
