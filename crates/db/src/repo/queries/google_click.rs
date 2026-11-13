//! Port of `packages/database/src/queries/google-click.ts` (P3.2b).
//!
//! Both fragments read the unqualified `"referral"` column of `"ContactInbox"`.

use serde_json::{Map, Value};

use crate::repo::sql::Sql;

/// `GOOGLE_CLICK_REFERRAL_KEYS` (`@chatbotx.io/utils/google-click`).
pub const GOOGLE_CLICK_REFERRAL_KEYS: &[&str] = &[
    "gclid",
    "gbraid",
    "googleCampaignId",
    "googleAdGroupId",
    "googleAdId",
    "googleClickReceivedAt",
];

/// `ISO_TIMESTAMP_SQL_PATTERN`: the strict fixed-width `toISOString()` shape.
pub const ISO_TIMESTAMP_SQL_PATTERN: &str =
    "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}[.][0-9]{3}Z$";

/// `googleClickPredicate`: the row carries a Google click (gclid or gbraid).
pub fn google_click_predicate() -> Sql {
    todo!("P3.2b")
}

/// `newestGoogleClickReferralMerge`: the new `referral` value that keeps the stored click
/// when it is strictly newer (fixed-width ISO text order) and otherwise applies `next`;
/// `None` when `next` has no fixed-width ISO `googleClickReceivedAt`.
pub fn newest_google_click_referral_merge(next: &Map<String, Value>) -> Option<Sql> {
    let _ = next;
    todo!("P3.2b")
}
