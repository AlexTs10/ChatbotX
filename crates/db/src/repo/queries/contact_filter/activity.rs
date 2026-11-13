//! Port of `queries/contact-filter/contact-activity.ts`: the activity-backed conditions
//! (`CONTACT_ACTIVITY_FILTERS` fields, `sentMessage`, `lastDelivered`). A condition that
//! cannot be evaluated (unknown operator, malformed id, no workspace, kill switch off)
//! fails closed: `FALSE` (`matchesNobody`), never `None` (that would widen an AND group).

use chrono_tz::Tz;
use serde_json::Value as Json;

use crate::repo::partials::contact_activity::ContactActivityFilterConfig;
use crate::repo::sql::Sql;

/// `buildContactActivityWhere`.
pub(super) fn contact_activity_where(
    _config: &ContactActivityFilterConfig,
    _operator: &str,
    _value: Option<&Json>,
    _workspace_id: Option<i64>,
    _enabled: bool,
) -> Sql {
    todo!("P3.2b")
}

/// `buildSentMessageWhere` (with `normalizeForMatch`).
pub(super) fn sent_message_where(
    _operator: &str,
    _value: Option<&Json>,
    _workspace_id: Option<i64>,
    _enabled: bool,
) -> Sql {
    todo!("P3.2b")
}

/// `buildLastDeliveredWhere`.
pub(super) fn last_delivered_where(
    _operator: &str,
    _value: Option<&Json>,
    _workspace_id: Option<i64>,
    _zone: Tz,
    _enabled: bool,
) -> Sql {
    todo!("P3.2b")
}
