//! Port of `repositories/contact-referral-event/repository.ts`
//! (`contactReferralEventRepository`, P3.2b).

use std::collections::BTreeMap;

use rusqlite::Connection;

use super::models::ContactReferralEvent;
use super::patch::Patch;
use super::{Json, Result};

/// `RECENT_AD_TOUCH_SCAN_LIMIT`: how many of the newest ad touches `listRecentAds` scans.
pub const RECENT_AD_TOUCH_SCAN_LIMIT: i64 = 2000;

/// `RecentAd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentAd {
    pub ad_id: String,
    pub title: Option<String>,
}

/// `ContactAdTouch`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactAdTouch {
    pub id: i64,
    pub channel: String,
    pub provider: Option<String>,
    pub occurred_at: i64,
    pub ad_id: Option<String>,
    pub campaign_id: Option<String>,
    pub ad_group_id: Option<String>,
    pub source: Option<String>,
    pub source_url: Option<String>,
    pub metadata: Option<Json>,
}

/// `ContactAdTouchCursor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactAdTouchCursor {
    pub occurred_at: i64,
    pub id: i64,
}

/// One row of `aggregateAdTouchStats`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdTouchStatsRow {
    pub ad_id: String,
    pub title: Option<String>,
    pub any_touch_contacts: i64,
    pub first_touch_contacts: i64,
}

/// One row of `aggregateMetaLinkedConversions` (`amount` = exact decimal text of the sum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedConversionRow {
    pub ad_id: String,
    pub event_type: String,
    pub currency: Option<String>,
    pub count: i64,
    pub amount: String,
}

/// `AdStatsRange` (`since` / `until` inclusive, unix ms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdStatsRange {
    pub workspace_id: i64,
    pub since: i64,
    pub until: i64,
    pub channel: Option<String>,
}

/// Input of `findLatestAdTouchId`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindLatestAdTouchInput {
    pub workspace_id: i64,
    pub contact_inbox_id: i64,
    pub provider: String,
    pub click_id: Option<String>,
    pub ad_id: Option<String>,
}

/// Input of `listRecentAds`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListRecentAdsInput {
    pub workspace_id: i64,
    pub limit: i64,
    pub search: Option<String>,
    pub ad_ids: Option<Vec<String>>,
}

/// `insertIgnoreDuplicate`: `None` when `(workspaceId, contactInboxId, dedupKey)` exists.
pub fn insert_ignore_duplicate(
    _conn: &Connection,
    _values: &Patch,
) -> Result<Option<ContactReferralEvent>> {
    todo!("P3.2b")
}

/// `findLatestAdTouchId`: the newest ad touch of the provider on the contact inbox,
/// pinned to the click id (else to the ad id) when given.
pub fn find_latest_ad_touch_id(
    _conn: &Connection,
    _input: &FindLatestAdTouchInput,
) -> Result<Option<i64>> {
    todo!("P3.2b")
}

/// `listRecentAds`: the ads of the workspace's newest ad touches (bounded scan).
pub fn list_recent_ads(_conn: &Connection, _input: &ListRecentAdsInput) -> Result<Vec<RecentAd>> {
    todo!("P3.2b")
}

/// `listAdTouchesByContact`: newest first, `limit + 1` rows past `cursor`.
pub fn list_ad_touches_by_contact(
    _conn: &Connection,
    _workspace_id: i64,
    _contact_id: i64,
    _limit: i64,
    _cursor: Option<ContactAdTouchCursor>,
) -> Result<Vec<ContactAdTouch>> {
    todo!("P3.2b")
}

/// `aggregateAdTouchStats`: contacts per ad (any touch / first touch) in the range.
pub fn aggregate_ad_touch_stats(
    _conn: &Connection,
    _range: &AdStatsRange,
    _limit: i64,
) -> Result<Vec<AdTouchStatsRow>> {
    todo!("P3.2b")
}

/// `aggregateMetaLinkedConversions`: conversions linked to a touch of one of `ad_ids`.
pub fn aggregate_meta_linked_conversions(
    _conn: &Connection,
    _range: &AdStatsRange,
    _ad_ids: &[String],
) -> Result<Vec<LinkedConversionRow>> {
    todo!("P3.2b")
}

/// `listAdIdsByContact`.
pub fn list_ad_ids_by_contact(
    _conn: &Connection,
    _workspace_id: i64,
    _contact_id: i64,
    _limit: i64,
) -> Result<Vec<String>> {
    todo!("P3.2b")
}

/// `listAdIdsByContacts`: each contact's distinct ad ids, oldest first touch first, at
/// most `limit_per_contact` (a contact without a touch has no entry).
pub fn list_ad_ids_by_contacts(
    _conn: &Connection,
    _workspace_id: i64,
    _contact_ids: &[i64],
    _limit_per_contact: i64,
) -> Result<BTreeMap<i64, Vec<String>>> {
    todo!("P3.2b")
}
