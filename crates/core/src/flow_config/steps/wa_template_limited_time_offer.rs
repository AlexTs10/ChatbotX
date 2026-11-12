//! Port of `packages/flow-config/src/steps/wa-template-limited-time-offer.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT isConfiguredExpirationTimeMs:
// function isConfiguredExpirationTimeMs(expirationTimeMs){return typeof expirationTimeMs==="number"&&Number.isFinite(expirationTimeMs)&&expirationTimeMs>0}

// HAND-PORT expirationTimeMsToDate:
// function expirationTimeMsToDate(expirationTimeMs){return isConfiguredExpirationTimeMs(expirationTimeMs)?new Date(expirationTimeMs):void 0}

// HAND-PORT dateToExpirationTimeMs:
// function dateToExpirationTimeMs(date){return date.getTime()}

// HAND-PORT isFutureExpirationTimeMs:
// function isFutureExpirationTimeMs(expirationTimeMs,now=new Date){return typeof expirationTimeMs==="number"&&expirationTimeMs>now.getTime()}

// HAND-PORT validateLimitedTimeOfferParams:
// function validateLimitedTimeOfferParams(params,ctx,basePath=[]){if(!params.limited_time_offer){return}if(!isConfiguredExpirationTimeMs(params.limited_time_offer.expiration_time_ms)){ctx.addIssue({code:"custom",message:flowValidationCodes.waTemplateLtoExpirationRequired,path:[...basePath,"limited_time_offer","expiration_time_ms"]})}}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn date_to_expiration_time_ms(date: chrono::DateTime<chrono::Utc>) -> f64 {
    todo!("P3.4")
}

pub fn expiration_time_ms_to_date(
    expiration_time_ms: Option<f64>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    todo!("P3.4")
}

pub fn is_configured_expiration_time_ms(expiration_time_ms: Option<f64>) -> bool {
    todo!("P3.4")
}

pub fn is_future_expiration_time_ms(
    expiration_time_ms: Option<f64>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    todo!("P3.4")
}

pub fn validate_limited_time_offer_params(
    params: &Value,
    ctx: &mut RefineCtx,
    base_path: &[PathSegment],
) {
    todo!("P3.4")
}
