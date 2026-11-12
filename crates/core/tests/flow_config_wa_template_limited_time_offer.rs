//! Port of `packages/flow-config/__tests__/wa-template-limited-time-offer.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use chrono::{DateTime, Utc};

#[test]
fn expirationtimemstodate_converts_a_finite_epoch_ms_number_to_a_date() {
    let date = fc::expiration_time_ms_to_date(Some(1_700_000_000_000.0));
    assert_eq!(date.map(|d| d.timestamp_millis()), Some(1_700_000_000_000));
}

#[test]
fn expirationtimemstodate_returns_undefined_for_undefined_nan_and_non_finite_values() {
    assert!(fc::expiration_time_ms_to_date(None).is_none());
    assert!(fc::expiration_time_ms_to_date(Some(f64::NAN)).is_none());
    assert!(fc::expiration_time_ms_to_date(Some(f64::INFINITY)).is_none());
}

#[test]
fn expirationtimemstodate_treats_the_seeded_0_default_as_unset_never_the_1970_epoch() {
    assert!(fc::expiration_time_ms_to_date(Some(0.0)).is_none());
    assert!(fc::expiration_time_ms_to_date(Some(-1.0)).is_none());
}

#[test]
fn isconfiguredexpirationtimems_true_only_for_finite_positive_values() {
    assert!(fc::is_configured_expiration_time_ms(Some(
        1_700_000_000_000.0
    )));
    assert!(!fc::is_configured_expiration_time_ms(Some(0.0)));
    assert!(!fc::is_configured_expiration_time_ms(None));
    assert!(!fc::is_configured_expiration_time_ms(Some(f64::NAN)));
}

fn validate(params: Value) -> SafeParse {
    let schema = z::custom(|_| true, None)
        .super_refine(|value, ctx| fc::validate_limited_time_offer_params(value, ctx, &[]));
    schema.safe_parse(&params)
}

#[test]
fn validatelimitedtimeofferparams_rejects_an_lto_template_whose_expiration_is_still_the_seeded_0() {
    let result = validate(json!({ "limited_time_offer": { "expiration_time_ms": 0 } }));
    assert!(!result.success);
    assert_eq!(
        result.issues()[0].message,
        "waTemplateLtoExpirationRequired"
    );
}

#[test]
fn validatelimitedtimeofferparams_accepts_a_configured_expiration_including_a_past_one_legacy_resend()
 {
    assert!(validate(json!({ "limited_time_offer": { "expiration_time_ms": 1000 } })).success);
}

#[test]
fn validatelimitedtimeofferparams_templates_without_an_lto_component_are_untouched() {
    assert!(validate(json!({})).success);
}

#[test]
fn datetoexpirationtimems_round_trips_through_expirationtimemstodate() {
    let ms = 1_700_000_123_456_i64;
    let date = fc::expiration_time_ms_to_date(Some(ms as f64)).unwrap();
    assert_eq!(fc::date_to_expiration_time_ms(date), ms as f64);
}

fn now() -> DateTime<Utc> {
    "2026-08-21T00:00:00.000Z".parse().unwrap()
}

#[test]
fn isfutureexpirationtimems_true_when_the_expiration_is_after_now() {
    let now = now();
    assert!(fc::is_future_expiration_time_ms(
        Some((now.timestamp_millis() + 60_000) as f64),
        now
    ));
}

#[test]
fn isfutureexpirationtimems_false_when_the_expiration_is_before_or_equal_to_now() {
    let now = now();
    assert!(!fc::is_future_expiration_time_ms(
        Some((now.timestamp_millis() - 60_000) as f64),
        now
    ));
    assert!(!fc::is_future_expiration_time_ms(
        Some(now.timestamp_millis() as f64),
        now
    ));
}

#[test]
fn isfutureexpirationtimems_false_when_undefined_never_treated_as_future() {
    assert!(!fc::is_future_expiration_time_ms(None, now()));
}
