//! Port of `packages/flow-config/src/steps/wa-template-limited-time-offer.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

/// `isConfiguredExpirationTimeMs`: finite and positive.
pub fn is_configured_expiration_time_ms(expiration_time_ms: Option<f64>) -> bool {
    expiration_time_ms.is_some_and(|ms| ms.is_finite() && ms > 0.0)
}

/// `expirationTimeMsToDate`: the seeded 0 is unset, never the 1970 epoch.
pub fn expiration_time_ms_to_date(
    expiration_time_ms: Option<f64>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if !is_configured_expiration_time_ms(expiration_time_ms) {
        return None;
    }
    chrono::DateTime::from_timestamp_millis(expiration_time_ms? as i64)
}

/// `dateToExpirationTimeMs`.
pub fn date_to_expiration_time_ms(date: chrono::DateTime<chrono::Utc>) -> f64 {
    date.timestamp_millis() as f64
}

/// `isFutureExpirationTimeMs(ms, now)`.
pub fn is_future_expiration_time_ms(
    expiration_time_ms: Option<f64>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    expiration_time_ms.is_some_and(|ms| ms > now.timestamp_millis() as f64)
}

/// `validateLimitedTimeOfferParams(params, ctx, basePath)`.
pub fn validate_limited_time_offer_params(
    params: &Value,
    ctx: &mut RefineCtx,
    base_path: &[PathSegment],
) {
    let Some(offer) = params.get("limited_time_offer").filter(|v| truthy(Some(v))) else {
        return;
    };
    if !is_configured_expiration_time_ms(offer["expiration_time_ms"].as_f64()) {
        let mut path = base_path.to_vec();
        path.extend(path!["limited_time_offer", "expiration_time_ms"]);
        ctx.issue(path, "waTemplateLtoExpirationRequired");
    }
}
