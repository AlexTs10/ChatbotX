//! Port of `packages/database/__tests__/google-click-referral-merge.test.ts` (P3.2b).
//! The TS renders the fragment with `PgDialect` and checks the Postgres text and params; here
//! the SQLite text and binds are checked (adapted): `~` is `REGEXP`, `COLLATE "C"` is the
//! default BINARY text order, `||` on jsonb is a shallow key merge of the bound document.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo10.rs"]
mod repo10;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::queries::google_click::{
    ISO_TIMESTAMP_SQL_PATTERN, newest_google_click_referral_merge as merge,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::rusqlite::types::Value as SqlValue;
use serde_json::{Map, Value, json};

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().expect("object")
}

fn google_click() -> Value {
    json!({
        "gclid": "gclid-1234567890",
        "gbraid": null,
        "googleCampaignId": "1",
        "googleAdGroupId": "2",
        "googleAdId": "3",
        "googleClickReceivedAt": "2026-10-01T00:00:00.000Z",
    })
}

fn with_meta() -> Value {
    let mut m = obj(google_click());
    m.insert("ctwaClid".to_owned(), json!("meta-1"));
    Value::Object(m)
}

fn render(v: Value) -> Sql {
    merge(&obj(v)).expect("expected a guarded merge")
}

fn text_param(q: &Sql, i: usize) -> String {
    match &q.params[i] {
        SqlValue::Text(t) => t.clone(),
        other => panic!("param {i} is not text: {other:?}"),
    }
}

fn returns_null_for(value: &str) {
    assert!(
        merge(&obj(json!({ "googleClickReceivedAt": value }))).is_none(),
        "{value:?}"
    );
}

#[test]
fn newest_google_click_referral_merge_returns_null_without_a_parseable_google_click_received_at() {
    assert!(merge(&obj(json!({ "ctwaClid": "c" }))).is_none());
    assert!(merge(&obj(json!({ "googleClickReceivedAt": null }))).is_none());
    assert!(merge(&obj(json!({ "googleClickReceivedAt": "nope" }))).is_none());
}

#[test]
fn newest_google_click_referral_merge_returns_null_for_the_non_fixed_width_iso_value_2026_10_01() {
    returns_null_for("2026-10-01");
}

#[test]
fn newest_google_click_referral_merge_returns_null_for_the_non_fixed_width_iso_value_2026_10_01t00_00_00z()
 {
    returns_null_for("2026-10-01T00:00:00Z");
}

#[test]
fn newest_google_click_referral_merge_returns_null_for_the_non_fixed_width_iso_value_2026_10_01t00_00_00_000_00_00()
 {
    returns_null_for("2026-10-01T00:00:00.000+00:00");
}

#[test]
fn newest_google_click_referral_merge_returns_null_for_the_non_fixed_width_iso_value_2026_10_01t00_00_00_00z()
 {
    returns_null_for("2026-10-01T00:00:00.00Z");
}

#[test]
fn newest_google_click_referral_merge_returns_null_for_the_non_fixed_width_iso_value_leading_space()
{
    returns_null_for(" 2026-10-01T00:00:00.000Z");
}

#[test]
fn newest_google_click_referral_merge_returns_null_for_the_non_fixed_width_iso_value_trailing_newline()
 {
    returns_null_for("2026-10-01T00:00:00.000Z\n");
}

#[test]
fn newest_google_click_referral_merge_is_one_parameterised_expression_stored_newer_branch_first() {
    let q = render(with_meta());
    assert!(
        q.text.contains(r#""referral"->>'googleClickReceivedAt'"#),
        "{}",
        q.text
    );
    assert!(q.text.contains(" > ?"), "{}", q.text);
    // The stale (stored-newer) document is bound before the incoming one.
    let then = q.text.find(" THEN ").expect("THEN");
    let else_ = q.text.rfind(" ELSE ").expect("ELSE");
    assert!(then < else_);
    assert_eq!(q.params.len(), 4);
    assert_eq!(text_param(&q, 1), "2026-10-01T00:00:00.000Z");
    assert!(!q.text.contains("gclid-1234567890"));
}

#[test]
fn newest_google_click_referral_merge_the_stale_branch_keeps_non_google_keys_and_drops_all_six_google_keys()
 {
    let q = render(with_meta());
    let stale: Value = serde_json::from_str(&text_param(&q, 2)).expect("JSON");
    assert_eq!(stale, json!({ "ctwaClid": "meta-1" }));
}

#[test]
fn newest_google_click_referral_merge_the_newer_branch_applies_everything_including_the_null_clearing_the_other_id()
 {
    let q = render(with_meta());
    let newer: Value = serde_json::from_str(&text_param(&q, 3)).expect("JSON");
    assert_eq!(newer, with_meta());
}

#[test]
fn newest_google_click_referral_merge_equal_timestamps_are_not_strictly_newer_so_the_incoming_click_applies()
 {
    let q = render(google_click());
    assert!(q.text.contains(" > ?"));
    assert!(!q.text.contains(">="));
}

#[test]
fn newest_google_click_referral_merge_never_casts_to_timestamptz_the_comparison_is_plain_text() {
    let q = render(google_click());
    assert!(!q.text.contains("timestamptz"));
    assert!(!q.text.to_lowercase().contains("datetime("));
    assert!(!q.text.to_lowercase().contains("julianday("));
}

#[test]
fn newest_google_click_referral_merge_the_stored_value_is_guarded_by_the_strict_full_match_format_regex()
 {
    let q = render(google_click());
    assert!(q.text.contains(" REGEXP ? THEN"), "{}", q.text);
    assert_eq!(text_param(&q, 0), ISO_TIMESTAMP_SQL_PATTERN);
    assert_eq!(
        ISO_TIMESTAMP_SQL_PATTERN,
        "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}[.][0-9]{3}Z$"
    );
}
