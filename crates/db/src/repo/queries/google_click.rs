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
    Sql::raw("(\"referral\"->>'gclid' IS NOT NULL OR \"referral\"->>'gbraid' IS NOT NULL)")
}

/// `ISO_TIMESTAMP_PATTERN` (`^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$`), checked by hand.
fn is_fixed_width_iso(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 24
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'.',
            23 => *c == b'Z',
            _ => c.is_ascii_digit(),
        })
}

/// JSON text of one `json_each` row's value (keeps `true` / `false` / `null` as JSON).
const EACH_VALUE_JSON: &str = "CASE \"type\" WHEN 'text' THEN json_quote(\"value\") \
     WHEN 'true' THEN 'true' WHEN 'false' THEN 'false' WHEN 'null' THEN 'null' \
     ELSE CAST(\"value\" AS TEXT) END";

/// `newestGoogleClickReferralMerge`: the new `referral` value that keeps the stored click
/// when it is strictly newer (fixed-width ISO text order) and otherwise applies `next`;
/// `None` when `next` has no fixed-width ISO `googleClickReceivedAt`.
///
/// `COALESCE("referral", '{}') || <document>` is a shallow merge: the stored keys, then the
/// document's keys (the document wins per key).
pub fn newest_google_click_referral_merge(next: &Map<String, Value>) -> Option<Sql> {
    let received_at = next.get("googleClickReceivedAt")?.as_str()?;
    if !is_fixed_width_iso(received_at) {
        return None;
    }
    let without_google: Map<String, Value> = next
        .iter()
        .filter(|(k, _)| !GOOGLE_CLICK_REFERRAL_KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let stored = "(\"referral\"->>'googleClickReceivedAt')";
    let mut q = Sql::raw(format!(
        "(SELECT json_group_object(\"k\", json(\"v\")) FROM (SELECT \"k\", \"v\", \
         row_number() OVER (PARTITION BY \"k\" ORDER BY \"src\" DESC) AS \"rn\" FROM (\
         SELECT \"key\" AS \"k\", {EACH_VALUE_JSON} AS \"v\", 0 AS \"src\" \
         FROM json_each(COALESCE(\"referral\", '{{}}')) \
         UNION ALL SELECT \"key\", {EACH_VALUE_JSON}, 1 FROM json_each(\
         CASE WHEN CASE WHEN {stored} REGEXP "
    ));
    q.push_bind(ISO_TIMESTAMP_SQL_PATTERN.to_owned());
    q.push(&format!(" THEN {stored} > "));
    q.push_bind(received_at.to_owned());
    q.push(" ELSE 0 END THEN ");
    q.push_bind(Value::Object(without_google).to_string());
    q.push(" ELSE ");
    q.push_bind(Value::Object(next.clone()).to_string());
    q.push(" END))) WHERE \"rn\" = 1)");
    Some(q)
}
