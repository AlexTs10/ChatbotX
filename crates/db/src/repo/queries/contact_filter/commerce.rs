//! Port of `queries/contact-filter/commerce.ts`: purchase and web shop cart conditions.

use chrono_tz::Tz;
use serde_json::Value as Json;

use crate::repo::sql::Sql;

/// The purchase / cart condition of `field`.
pub(crate) fn commerce_where(
    field: &str,
    operator: &str,
    value: Option<&Json>,
    zone: Tz,
) -> Option<Sql> {
    let _ = (field, operator, value, zone);
    todo!("P3.2b")
}
