//! Port of `packages/database/__tests__/ads-conversion-event-repository.test.ts` (P3.2).
//!
//! The original mocks the Drizzle query chain and asserts call shapes (`innerJoin` with a
//! model, `where` called, SQL text / bound params, `limit(3)`). Each case is ported as the
//! observable effect of that shape on real rows in a migrated SQLite file (`adapted`): a
//! join on the selected integration = rows of the other integration are not counted; a
//! channel eq-filter = rows of the other channel are not counted; a bound time zone =
//! the day key moves with the zone; `limit + 1` = `has_more` with exactly `limit` rows.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::ads_conversion_event::{
    self as ace, AdReferralInput, AdsConversionEventCreate, ConversationCount, DateRangeInput,
    EventCount, ExportRow, ExportSegmentInput,
};
use chatbotx_db::repo::{self, Json};
use chatbotx_db::rusqlite::Connection;
use common::{fresh_db, seed};
use repo_common::{insert_inbox, ms, ok};
use serde_json::json;

const WS: i64 = 2001;
const WA1: i64 = 3101;
const WA2: i64 = 3102;
const MS: i64 = 3103;
const IG: i64 = 3104;
const MS2: i64 = 3105;
const IW1: i64 = 6001;
const IW2: i64 = 6002;
const IM: i64 = 7001;
const IM2: i64 = 7002;
const IGI: i64 = 8001;

fn since() -> i64 {
    ms("2026-08-01T00:00:00.000Z")
}
fn until() -> i64 {
    ms("2026-08-10T23:59:59.999Z")
}
fn july() -> (i64, i64) {
    (
        ms("2026-07-01T00:00:00.000Z"),
        ms("2026-07-31T23:59:59.999Z"),
    )
}

fn range() -> DateRangeInput {
    DateRangeInput {
        workspace_id: WS,
        since: since(),
        until: until(),
        ..Default::default()
    }
}

/// Seed graph: the common seed (workspace 2001) + 2 WhatsApp, 2 Messenger and 1 Instagram
/// inboxes with their integrations.
async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    repo_common::setup(&db, |tx| {
        insert_inbox(tx, WA1, WS, "whatsapp", "wa-1")?;
        insert_inbox(tx, WA2, WS, "whatsapp", "wa-2")?;
        insert_inbox(tx, MS, WS, "messenger", "page-ms")?;
        insert_inbox(tx, IG, WS, "instagram", "ig-1")?;
        insert_inbox(tx, MS2, WS, "messenger", "page-ms-2")?;
        for (id, inbox, waba) in [(IW1, WA1, "waba-1"), (IW2, WA2, "waba-2")] {
            ins!(tx, "IntegrationWhatsapp", {
                "id" => id, "auth" => "{}", "phoneNumberId" => format!("pn-{id}"),
                "wabaId" => waba, "businessId" => "biz", "name" => "WA",
                "workspaceId" => WS, "inboxId" => inbox,
            })?;
        }
        for (id, inbox) in [(IM, MS), (IM2, MS2)] {
            ins!(tx, "IntegrationMessenger", {
                "id" => id, "auth" => "{}", "pageId" => format!("page-{id}"), "name" => "Page",
                "conversationStarters" => "[]", "persistentMenus" => "[]", "personas" => "[]",
                "workspaceId" => WS, "inboxId" => inbox,
            })?;
        }
        ins!(tx, "IntegrationInstagram", {
            "id" => IGI, "auth" => "{}", "igId" => "ig", "pageId" => "page-ig", "name" => "IG",
            "username" => "ig", "conversationStarters" => "[]", "persistentMenus" => "[]",
            "workspaceId" => WS, "inboxId" => IG,
        })?;
        Ok(())
    })
    .await;
    db
}

/// Contact `id` (first name, phone, email) + one ContactInbox `id + 1` on `inbox`.
#[allow(clippy::too_many_arguments)]
fn contact_ci(
    tx: &Connection,
    id: i64,
    inbox: i64,
    channel: &str,
    referral: Option<serde_json::Value>,
    first_at: i64,
    name: &str,
    phone: Option<&str>,
    email: Option<&str>,
) -> i64 {
    ins!(tx, "Contact", {
        "id" => id, "workspaceId" => WS, "firstName" => name.to_owned(),
        "phoneNumber" => phone.map(str::to_owned), "email" => email.map(str::to_owned),
    })
    .expect("contact");
    ins!(tx, "ContactInbox", {
        "id" => id + 1, "originalContactId" => id, "contactId" => id, "inboxId" => inbox,
        "channel" => channel.to_owned(), "source" => channel.to_owned(),
        "sourceId" => format!("src-{id}"), "referral" => referral.map(|r| r.to_string()),
        "firstInteractionAt" => first_at,
    })
    .expect("contact inbox");
    id + 1
}

fn ctwa(ad: &str) -> Option<serde_json::Value> {
    Some(json!({ "ctwaClid": format!("clid-{ad}"), "adId": ad }))
}

fn ads(ad: &str) -> Option<serde_json::Value> {
    Some(json!({ "adId": ad, "source": "ADS" }))
}

/// One AdsConversionEvent (`integration` = the id of the channel's integration column).
#[allow(clippy::too_many_arguments)]
fn event(
    tx: &Connection,
    id: i64,
    channel: &str,
    integration: i64,
    event_type: &str,
    ad: &str,
    ci: Option<i64>,
    occurred_at: i64,
    capi_status: &str,
) {
    let (wa, msg, igc) = match channel {
        "whatsapp" => (Some(integration), None, None),
        "messenger" => (None, Some(integration), None),
        _ => (None, None, Some(integration)),
    };
    ins!(tx, "AdsConversionEvent", {
        "id" => id, "workspaceId" => WS, "channel" => channel.to_owned(),
        "integrationWhatsappId" => wa, "integrationMessengerId" => msg,
        "integrationInstagramId" => igc,
        "wabaId" => wa.map(|_| "waba-1".to_owned()), "ctwaClid" => wa.map(|_| "clid".to_owned()),
        "source" => "rule", "eventType" => event_type.to_owned(), "adId" => ad.to_owned(),
        "contactInboxId" => ci, "occurredAt" => occurred_at,
        "sourceEventId" => format!("src-{id}"), "capiStatus" => capi_status.to_owned(),
    })
    .expect("event");
}

fn conv(ad: &str, n: i64) -> ConversationCount {
    ConversationCount {
        date: None,
        ad_id: Some(ad.to_owned()),
        channel: None,
        conversations: n,
    }
}

fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
    v.sort();
    v
}

async fn read<F, R>(db: &Db, f: F) -> R
where
    F: FnOnce(&Connection) -> repo::Result<R> + Send + 'static,
    R: Send + 'static,
{
    ok(repo::read(db, f).await)
}

// ---------------------------------------------------------------------------
// account filters
// ---------------------------------------------------------------------------

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:50 "adsConversionEventRepository account filters > joins IntegrationWhatsapp when counting conversations for a selected account"
#[tokio::test]
async fn ads_conversion_event_repository_account_filters_joins_integration_whatsapp_when_counting_conversations_for_a_selected_account()
 {
    let db = fixture().await;
    let t = ms("2026-08-05T10:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        contact_ci(tx, 10, WA1, "whatsapp", ctwa("ad-1"), t, "A", None, None);
        contact_ci(tx, 20, WA2, "whatsapp", ctwa("ad-1"), t, "B", None, None);
        Ok(())
    })
    .await;
    let mut input = range();
    input.integration_whatsapp_id = Some(IW1);
    let rows = read(&db, move |c| ace::count_ctwa_conversations_by_ad(c, &input)).await;
    assert_eq!(rows, vec![conv("ad-1", 1)]);
    let all = read(&db, |c| ace::count_ctwa_conversations_by_ad(c, &range())).await;
    assert_eq!(all, vec![conv("ad-1", 2)]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:70 "adsConversionEventRepository account filters > adds a where condition when counting conversion events for a selected account"
#[tokio::test]
async fn ads_conversion_event_repository_account_filters_adds_a_where_condition_when_counting_conversion_events_for_a_selected_account()
 {
    let db = fixture().await;
    let t = ms("2026-08-05T10:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        event(tx, 1, "whatsapp", IW1, "lead", "ad-1", None, t, "pending");
        event(tx, 2, "whatsapp", IW2, "lead", "ad-1", None, t, "pending");
        Ok(())
    })
    .await;
    let mut input = range();
    input.integration_whatsapp_id = Some(IW1);
    let rows = read(&db, move |c| ace::count_conversion_events_by_ad(c, &input)).await;
    assert_eq!(
        rows,
        vec![EventCount {
            date: None,
            ad_id: Some("ad-1".into()),
            event_type: "lead".into(),
            channel: None,
            count: 1,
            purchase_value: None
        }]
    );
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:94 "adsConversionEventRepository account filters > counts conversion events by CAPI status"
#[tokio::test]
async fn ads_conversion_event_repository_account_filters_counts_conversion_events_by_capi_status() {
    let db = fixture().await;
    let t = ms("2026-08-05T10:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        for id in 1..=3 {
            event(tx, id, "whatsapp", IW1, "lead", "ad-1", None, t, "sent");
        }
        event(tx, 4, "whatsapp", IW1, "lead", "ad-1", None, t, "failed");
        Ok(())
    })
    .await;
    let rows = read(&db, |c| ace::count_by_capi_status(c, &range())).await;
    let got = sorted(rows.into_iter().map(|r| (r.capi_status, r.count)).collect());
    assert_eq!(got, vec![("failed".to_owned(), 1), ("sent".to_owned(), 3)]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:117 "adsConversionEventRepository account filters > joins IntegrationWhatsapp when counting daily conversations for a selected account"
#[tokio::test]
async fn ads_conversion_event_repository_account_filters_joins_integration_whatsapp_when_counting_daily_conversations_for_a_selected_account()
 {
    let db = fixture().await;
    let t = ms("2026-08-01T10:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        contact_ci(tx, 10, WA1, "whatsapp", ctwa("ad-1"), t, "A", None, None);
        contact_ci(tx, 20, WA2, "whatsapp", ctwa("ad-1"), t, "B", None, None);
        Ok(())
    })
    .await;
    let mut input = range();
    input.integration_whatsapp_id = Some(IW1);
    let rows = read(&db, move |c| {
        ace::count_ctwa_conversations_by_day_and_ad(c, &input)
    })
    .await;
    assert_eq!(
        rows,
        vec![ConversationCount {
            date: Some("2026-08-01".into()),
            ad_id: Some("ad-1".into()),
            channel: None,
            conversations: 1
        }]
    );
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:144 "adsConversionEventRepository account filters > groups daily conversion events by date, adId, and eventType"
#[tokio::test]
async fn ads_conversion_event_repository_account_filters_groups_daily_conversion_events_by_date_ad_id_and_event_type()
 {
    let db = fixture().await;
    let t = ms("2026-08-01T10:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        event(tx, 1, "whatsapp", IW1, "lead", "ad-1", None, t, "pending");
        event(
            tx,
            2,
            "whatsapp",
            IW1,
            "lead",
            "ad-1",
            None,
            t + 1000,
            "pending",
        );
        Ok(())
    })
    .await;
    let rows = read(&db, |c| {
        ace::count_conversion_events_by_day_and_ad(c, &range())
    })
    .await;
    assert_eq!(
        rows,
        vec![EventCount {
            date: Some("2026-08-01".into()),
            ad_id: Some("ad-1".into()),
            event_type: "lead".into(),
            channel: None,
            count: 2,
            purchase_value: None
        }]
    );
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:166 "adsConversionEventRepository account filters > adds a where condition when counting CAPI statuses for a selected account"
#[tokio::test]
async fn ads_conversion_event_repository_account_filters_adds_a_where_condition_when_counting_capi_statuses_for_a_selected_account()
 {
    let db = fixture().await;
    let t = ms("2026-08-05T10:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        for id in 1..=3 {
            event(tx, id, "whatsapp", IW1, "lead", "ad-1", None, t, "sent");
        }
        event(tx, 4, "whatsapp", IW2, "lead", "ad-1", None, t, "sent");
        Ok(())
    })
    .await;
    let mut input = range();
    input.integration_whatsapp_id = Some(IW1);
    let rows = read(&db, move |c| ace::count_by_capi_status(c, &input)).await;
    let got: Vec<(String, i64)> = rows.into_iter().map(|r| (r.capi_status, r.count)).collect();
    assert_eq!(got, vec![("sent".to_owned(), 3)]);
}

fn export(segment: &str, limit: i64, all_channels: bool, ad: Option<&str>) -> ExportSegmentInput {
    let (s, u) = july();
    ExportSegmentInput {
        range: DateRangeInput {
            workspace_id: WS,
            since: s,
            until: u,
            all_channels,
            ..Default::default()
        },
        segment: segment.to_owned(),
        ad_id: ad.map(str::to_owned),
        after_id: None,
        limit,
    }
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:183 "adsConversionEventRepository account filters > listExportSegmentRows resolves conversations via the shared ctwaRetarget predicate, keeping the same output shape"
#[tokio::test]
async fn ads_conversion_event_repository_account_filters_list_export_segment_rows_resolves_conversations_via_the_shared_ctwa_retarget_predicate_keeping_the_same_output_shape()
 {
    let db = fixture().await;
    let t = ms("2026-07-15T00:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        contact_ci(
            tx,
            10,
            WA1,
            "whatsapp",
            ctwa("ad-1"),
            t,
            "Ada",
            Some("+84900000000"),
            None,
        );
        contact_ci(
            tx,
            20,
            WA1,
            "whatsapp",
            ctwa("ad-2"),
            t,
            "Other",
            None,
            None,
        );
        Ok(())
    })
    .await;
    let input = export("conversations", 50, false, Some("ad-1"));
    let result = read(&db, move |c| ace::list_export_segment_rows(c, &input)).await;
    assert_eq!(
        result.rows,
        vec![ExportRow {
            id: 11,
            contact_id: 10,
            contact_name: Some("Ada".into()),
            phone_number: Some("+84900000000".into()),
            email: None,
            ad_id: Some("ad-1".into()),
            occurred_at: t,
            channel: None,
        }]
    );
    assert!(!result.has_more);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:225 "adsConversionEventRepository account filters > listExportSegmentRows resolves leads/purchases via the shared ctwaRetarget predicate, keeping the same output shape"
#[tokio::test]
async fn ads_conversion_event_repository_account_filters_list_export_segment_rows_resolves_leads_purchases_via_the_shared_ctwa_retarget_predicate_keeping_the_same_output_shape()
 {
    let db = fixture().await;
    let t = ms("2026-07-20T00:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        let ci = contact_ci(
            tx,
            10,
            WA1,
            "whatsapp",
            ctwa("ad-2"),
            t,
            "Bob",
            Some("+84900000001"),
            Some("bob@example.com"),
        );
        event(
            tx,
            500,
            "whatsapp",
            IW1,
            "purchase",
            "ad-2",
            Some(ci),
            t,
            "pending",
        );
        event(
            tx,
            501,
            "whatsapp",
            IW1,
            "lead",
            "ad-2",
            Some(ci),
            t,
            "pending",
        );
        Ok(())
    })
    .await;
    let input = export("purchases", 50, false, None);
    let result = read(&db, move |c| ace::list_export_segment_rows(c, &input)).await;
    assert_eq!(
        result.rows,
        vec![ExportRow {
            id: 500,
            contact_id: 10,
            contact_name: Some("Bob".into()),
            phone_number: Some("+84900000001".into()),
            email: Some("bob@example.com".into()),
            ad_id: Some("ad-2".into()),
            occurred_at: t,
            channel: None,
        }]
    );
    assert!(!result.has_more);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:271 "adsConversionEventRepository account filters > listExportSegmentRows (conversations) requests limit+1 rows and reports hasMore when the page is over-full"
#[tokio::test]
async fn ads_conversion_event_repository_account_filters_list_export_segment_rows_conversations_requests_limit_1_rows_and_reports_has_more_when_the_page_is_over_full()
 {
    let db = fixture().await;
    let t = ms("2026-07-15T00:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        for i in 1..=3 {
            contact_ci(
                tx,
                i * 10,
                WA1,
                "whatsapp",
                ctwa("ad-1"),
                t,
                "C",
                None,
                None,
            );
        }
        Ok(())
    })
    .await;
    let input = export("conversations", 2, false, None);
    let result = read(&db, move |c| ace::list_export_segment_rows(c, &input)).await;
    assert!(result.has_more);
    assert_eq!(result.rows.len(), 2);
    assert_eq!(
        result.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![11, 21]
    );
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:300 "adsConversionEventRepository account filters > listExportSegmentRows (leads/purchases) requests limit+1 rows and reports hasMore when the page is over-full"
#[tokio::test]
async fn ads_conversion_event_repository_account_filters_list_export_segment_rows_leads_purchases_requests_limit_1_rows_and_reports_has_more_when_the_page_is_over_full()
 {
    let db = fixture().await;
    let t = ms("2026-07-20T00:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        for i in 1..=3 {
            let ci = contact_ci(
                tx,
                i * 10,
                WA1,
                "whatsapp",
                ctwa("ad-2"),
                t,
                "C",
                None,
                None,
            );
            event(
                tx,
                500 + i,
                "whatsapp",
                IW1,
                "purchase",
                "ad-2",
                Some(ci),
                t,
                "pending",
            );
        }
        Ok(())
    })
    .await;
    let input = export("purchases", 2, false, None);
    let result = read(&db, move |c| ace::list_export_segment_rows(c, &input)).await;
    assert!(result.has_more);
    assert_eq!(result.rows.len(), 2);
    assert_eq!(
        result.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![501, 502]
    );
}

// ---------------------------------------------------------------------------
// insertIgnoreDuplicate — per-channel conflict target
// ---------------------------------------------------------------------------

fn create(channel: &str, source_event_id: &str) -> AdsConversionEventCreate {
    let mut v = AdsConversionEventCreate {
        workspace_id: WS,
        channel: Some(channel.to_owned()),
        source: "rule".into(),
        event_type: "lead".into(),
        occurred_at: ms("2026-08-10T00:00:00.000Z"),
        source_event_id: source_event_id.to_owned(),
        capi_status: Some("pending".into()),
        ..Default::default()
    };
    match channel {
        "whatsapp" => {
            v.integration_whatsapp_id = Some(IW1);
            v.waba_id = Some("waba-1".into());
            v.ctwa_clid = Some("clid-1".into());
        }
        "messenger" => v.integration_messenger_id = Some(IM),
        "instagram" => v.integration_instagram_id = Some(IGI),
        _ => {}
    }
    v
}

async fn insert(db: &Db, values: AdsConversionEventCreate) -> repo::Result<Option<i64>> {
    repo::write(db, move |tx| {
        Ok(ace::insert_ignore_duplicate(tx, &values)?.map(|r| r.id))
    })
    .await
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:344 "adsConversionEventRepository.insertIgnoreDuplicate — per-channel conflict target > targets the whatsapp partial unique index for channel='whatsapp'"
#[tokio::test]
async fn ads_conversion_event_repository_insert_ignore_duplicate_per_channel_conflict_target_targets_the_whatsapp_partial_unique_index_for_channel_whatsapp()
 {
    let db = fixture().await;
    let row = ok(repo::write(&db, |tx| {
        ace::insert_ignore_duplicate(tx, &create("whatsapp", "src-1"))
    })
    .await)
    .expect("inserted");
    assert_eq!(row.channel, "whatsapp");
    assert_eq!(row.integration_whatsapp_id, Some(IW1));
    // Same (workspaceId, integrationWhatsappId, source, sourceEventId) -> the target matches.
    assert_eq!(ok(insert(&db, create("whatsapp", "src-1")).await), None);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:379 "adsConversionEventRepository.insertIgnoreDuplicate — per-channel conflict target > targets the messenger partial unique index for channel='messenger'"
#[tokio::test]
async fn ads_conversion_event_repository_insert_ignore_duplicate_per_channel_conflict_target_targets_the_messenger_partial_unique_index_for_channel_messenger()
 {
    let db = fixture().await;
    assert!(ok(insert(&db, create("messenger", "src-2")).await).is_some());
    assert_eq!(ok(insert(&db, create("messenger", "src-2")).await), None);
    // Another messenger integration is a different key of the same index.
    let mut other = create("messenger", "src-2");
    other.integration_messenger_id = Some(IM2);
    assert!(ok(insert(&db, other).await).is_some());
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:411 "adsConversionEventRepository.insertIgnoreDuplicate — per-channel conflict target > targets the instagram partial unique index for channel='instagram'"
#[tokio::test]
async fn ads_conversion_event_repository_insert_ignore_duplicate_per_channel_conflict_target_targets_the_instagram_partial_unique_index_for_channel_instagram()
 {
    let db = fixture().await;
    assert!(ok(insert(&db, create("instagram", "src-3")).await).is_some());
    assert_eq!(ok(insert(&db, create("instagram", "src-3")).await), None);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:443 "adsConversionEventRepository.insertIgnoreDuplicate — per-channel conflict target > dedup: a second insert of the same messenger row (same conflict target + values) returns null"
#[tokio::test]
async fn ads_conversion_event_repository_insert_ignore_duplicate_per_channel_conflict_target_dedup_a_second_insert_of_the_same_messenger_row_same_conflict_target_values_returns_null()
 {
    let db = fixture().await;
    let mut first = create("messenger", "src-2");
    first.contact_inbox_id = None;
    assert!(ok(insert(&db, first.clone()).await).is_some());
    assert_eq!(ok(insert(&db, first).await), None);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:467 "adsConversionEventRepository.insertIgnoreDuplicate — per-channel conflict target > dedup semantics: the same sourceEventId under whatsapp vs messenger channels targets two distinct partial unique indexes, so both can insert"
#[tokio::test]
async fn ads_conversion_event_repository_insert_ignore_duplicate_per_channel_conflict_target_dedup_semantics_the_same_source_event_id_under_whatsapp_vs_messenger_channels_targets_two_distinct_partial_unique_indexes_so_both_can_insert()
 {
    let db = fixture().await;
    let wa = ok(insert(&db, create("whatsapp", "shared-source-event-id")).await);
    let me = ok(insert(&db, create("messenger", "shared-source-event-id")).await);
    assert!(wa.is_some());
    assert!(me.is_some());
    assert_ne!(wa, me);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:509 "adsConversionEventRepository.insertIgnoreDuplicate — per-channel conflict target > throws for a channel with no dedup unique index (facebook is a dead channel)"
#[tokio::test]
async fn ads_conversion_event_repository_insert_ignore_duplicate_per_channel_conflict_target_throws_for_a_channel_with_no_dedup_unique_index_facebook_is_a_dead_channel()
 {
    let db = fixture().await;
    let message = repo_common::err_message(insert(&db, create("facebook", "src-4")).await);
    assert!(message.contains("no dedup unique index"), "{message}");
}

// ---------------------------------------------------------------------------
// findAttributionByAdReferral
// ---------------------------------------------------------------------------

fn referral_input(channel: &str, integration: Option<i64>, ci: i64) -> AdReferralInput {
    AdReferralInput {
        workspace_id: WS,
        channel: channel.to_owned(),
        integration_messenger_id: if channel == "messenger" {
            integration
        } else {
            None
        },
        integration_instagram_id: if channel == "instagram" {
            integration
        } else {
            None
        },
        contact_inbox_id: ci,
    }
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:534 "adsConversionEventRepository.findAttributionByAdReferral > messenger: joins IntegrationMessenger and returns the attributed contact inbox row"
#[tokio::test]
async fn ads_conversion_event_repository_find_attribution_by_ad_referral_messenger_joins_integration_messenger_and_returns_the_attributed_contact_inbox_row()
 {
    let db = fixture().await;
    repo_common::setup(&db, |tx| {
        contact_ci(tx, 10, MS, "messenger", ads("ad-1"), 1, "A", None, None);
        Ok(())
    })
    .await;
    let row = read(&db, |c| {
        ace::find_attribution_by_ad_referral(c, &referral_input("messenger", Some(IM), 11))
    })
    .await
    .expect("attributed");
    assert_eq!(row.id, 11);
    assert_eq!(
        row.referral,
        Some(Json(json!({ "adId": "ad-1", "source": "ADS" })))
    );
    // Another integration's inbox does not join.
    let none = read(&db, |c| {
        ace::find_attribution_by_ad_referral(c, &referral_input("messenger", Some(IM2), 11))
    })
    .await;
    assert_eq!(none, None);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:560 "adsConversionEventRepository.findAttributionByAdReferral > messenger: the where clause pins ContactInbox.channel = 'messenger' (shared-inbox guard)"
#[tokio::test]
async fn ads_conversion_event_repository_find_attribution_by_ad_referral_messenger_the_where_clause_pins_contact_inbox_channel_messenger_shared_inbox_guard()
 {
    let db = fixture().await;
    // An instagram-channel contact inbox in the messenger integration's inbox.
    repo_common::setup(&db, |tx| {
        contact_ci(tx, 10, MS, "instagram", ads("ad-1"), 1, "A", None, None);
        Ok(())
    })
    .await;
    let row = read(&db, |c| {
        ace::find_attribution_by_ad_referral(c, &referral_input("messenger", Some(IM), 11))
    })
    .await;
    assert_eq!(row, None);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:583 "adsConversionEventRepository.findAttributionByAdReferral > messenger: returns null when no row matches (SHORTLINK referral, or no referral at all)"
#[tokio::test]
async fn ads_conversion_event_repository_find_attribution_by_ad_referral_messenger_returns_null_when_no_row_matches_shortlink_referral_or_no_referral_at_all()
 {
    let db = fixture().await;
    repo_common::setup(&db, |tx| {
        contact_ci(
            tx,
            10,
            MS,
            "messenger",
            Some(json!({ "adId": "ad-1", "source": "SHORTLINK" })),
            1,
            "A",
            None,
            None,
        );
        contact_ci(tx, 20, MS, "messenger", None, 1, "B", None, None);
        Ok(())
    })
    .await;
    for ci in [11, 21] {
        let row = read(&db, move |c| {
            ace::find_attribution_by_ad_referral(c, &referral_input("messenger", Some(IM), ci))
        })
        .await;
        assert_eq!(row, None);
    }
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:600 "adsConversionEventRepository.findAttributionByAdReferral > messenger: short-circuits without querying when integrationMessengerId is missing"
#[tokio::test]
async fn ads_conversion_event_repository_find_attribution_by_ad_referral_messenger_short_circuits_without_querying_when_integration_messenger_id_is_missing()
 {
    let db = fixture().await;
    repo_common::setup(&db, |tx| {
        contact_ci(tx, 10, MS, "messenger", ads("ad-1"), 1, "A", None, None);
        Ok(())
    })
    .await;
    let row = read(&db, |c| {
        ace::find_attribution_by_ad_referral(c, &referral_input("messenger", None, 11))
    })
    .await;
    assert_eq!(row, None);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:617 "adsConversionEventRepository.findAttributionByAdReferral > instagram: joins IntegrationInstagram and returns the attributed contact inbox row"
#[tokio::test]
async fn ads_conversion_event_repository_find_attribution_by_ad_referral_instagram_joins_integration_instagram_and_returns_the_attributed_contact_inbox_row()
 {
    let db = fixture().await;
    repo_common::setup(&db, |tx| {
        contact_ci(tx, 10, IG, "instagram", ads("ad-2"), 1, "A", None, None);
        Ok(())
    })
    .await;
    let row = read(&db, |c| {
        ace::find_attribution_by_ad_referral(c, &referral_input("instagram", Some(IGI), 11))
    })
    .await
    .expect("attributed");
    assert_eq!(row.id, 11);
    assert_eq!(
        row.referral,
        Some(Json(json!({ "adId": "ad-2", "source": "ADS" })))
    );
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:643 "adsConversionEventRepository.findAttributionByAdReferral > instagram: short-circuits without querying when integrationInstagramId is missing"
#[tokio::test]
async fn ads_conversion_event_repository_find_attribution_by_ad_referral_instagram_short_circuits_without_querying_when_integration_instagram_id_is_missing()
 {
    let db = fixture().await;
    repo_common::setup(&db, |tx| {
        contact_ci(tx, 10, IG, "instagram", ads("ad-2"), 1, "A", None, None);
        Ok(())
    })
    .await;
    let row = read(&db, |c| {
        ace::find_attribution_by_ad_referral(c, &referral_input("instagram", None, 11))
    })
    .await;
    assert_eq!(row, None);
}

// ---------------------------------------------------------------------------
// countAdConversationsByAd/ByDayAndAd — channel scoping
// ---------------------------------------------------------------------------

/// A messenger-channel and an instagram-channel ad conversation (same ad) in each of the
/// messenger and instagram inboxes.
async fn mixed_ad_conversations(db: &Db) {
    let t = ms("2026-08-05T10:00:00.000Z");
    repo_common::setup(db, move |tx| {
        contact_ci(tx, 10, MS, "messenger", ads("ad-1"), t, "A", None, None);
        contact_ci(tx, 20, IG, "instagram", ads("ad-1"), t, "B", None, None);
        contact_ci(tx, 30, MS2, "messenger", ads("ad-1"), t, "C", None, None);
        contact_ci(tx, 40, MS, "instagram", ads("ad-1"), t, "D", None, None);
        contact_ci(tx, 50, IG, "messenger", ads("ad-1"), t, "E", None, None);
        Ok(())
    })
    .await;
}

fn channel_range(channel: &str) -> DateRangeInput {
    let mut r = range();
    r.channel = Some(channel.to_owned());
    r
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:669 "adsConversionEventRepository.countAdConversationsByAd/ByDayAndAd — channel scoping > messenger without an integration id still excludes instagram-channel ContactInbox rows"
#[tokio::test]
async fn ads_conversion_event_repository_count_ad_conversations_by_ad_by_day_and_ad_channel_scoping_messenger_without_an_integration_id_still_excludes_instagram_channel_contact_inbox_rows()
 {
    let db = fixture().await;
    mixed_ad_conversations(&db).await;
    let rows = read(&db, |c| {
        ace::count_ad_conversations_by_ad(c, &channel_range("messenger"))
    })
    .await;
    // Messenger-channel rows: 10 (MS), 30 (MS2), 50 (IG inbox) = 3.
    assert_eq!(rows, vec![conv("ad-1", 3)]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:688 "adsConversionEventRepository.countAdConversationsByAd/ByDayAndAd — channel scoping > instagram without an integration id still excludes messenger-channel ContactInbox rows"
#[tokio::test]
async fn ads_conversion_event_repository_count_ad_conversations_by_ad_by_day_and_ad_channel_scoping_instagram_without_an_integration_id_still_excludes_messenger_channel_contact_inbox_rows()
 {
    let db = fixture().await;
    mixed_ad_conversations(&db).await;
    let rows = read(&db, |c| {
        ace::count_ad_conversations_by_ad(c, &channel_range("instagram"))
    })
    .await;
    // Instagram-channel rows: 20 (IG), 40 (MS inbox) = 2.
    assert_eq!(rows, vec![conv("ad-1", 2)]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:707 "adsConversionEventRepository.countAdConversationsByAd/ByDayAndAd — channel scoping > messenger WITH an integration id keeps the channel scope alongside the EXISTS scope"
#[tokio::test]
async fn ads_conversion_event_repository_count_ad_conversations_by_ad_by_day_and_ad_channel_scoping_messenger_with_an_integration_id_keeps_the_channel_scope_alongside_the_exists_scope()
 {
    let db = fixture().await;
    mixed_ad_conversations(&db).await;
    let mut input = channel_range("messenger");
    input.integration_messenger_id = Some(IM);
    let rows = read(&db, move |c| ace::count_ad_conversations_by_ad(c, &input)).await;
    // Only row 10: messenger channel AND in IM's inbox (40 is instagram, 30 is MS2).
    assert_eq!(rows, vec![conv("ad-1", 1)]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:728 "adsConversionEventRepository.countAdConversationsByAd/ByDayAndAd — channel scoping > instagram WITH an integration id keeps the channel scope alongside the EXISTS scope"
#[tokio::test]
async fn ads_conversion_event_repository_count_ad_conversations_by_ad_by_day_and_ad_channel_scoping_instagram_with_an_integration_id_keeps_the_channel_scope_alongside_the_exists_scope()
 {
    let db = fixture().await;
    mixed_ad_conversations(&db).await;
    let mut input = channel_range("instagram");
    input.integration_instagram_id = Some(IGI);
    let rows = read(&db, move |c| ace::count_ad_conversations_by_ad(c, &input)).await;
    // Only row 20: instagram channel AND in IGI's inbox (50 is messenger).
    assert_eq!(rows, vec![conv("ad-1", 1)]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:749 "adsConversionEventRepository.countAdConversationsByAd/ByDayAndAd — channel scoping > countAdConversationsByDayAndAd scopes messenger by channel even without an integration id"
#[tokio::test]
async fn ads_conversion_event_repository_count_ad_conversations_by_ad_by_day_and_ad_channel_scoping_count_ad_conversations_by_day_and_ad_scopes_messenger_by_channel_even_without_an_integration_id()
 {
    let db = fixture().await;
    mixed_ad_conversations(&db).await;
    let rows = read(&db, |c| {
        ace::count_ad_conversations_by_day_and_ad(c, &channel_range("messenger"))
    })
    .await;
    assert_eq!(
        rows,
        vec![ConversationCount {
            date: Some("2026-08-05".into()),
            ad_id: Some("ad-1".into()),
            channel: None,
            conversations: 3
        }]
    );
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:768 "adsConversionEventRepository.countAdConversationsByAd/ByDayAndAd — channel scoping > countAdConversationsByDayAndAd scopes instagram by channel even without an integration id"
#[tokio::test]
async fn ads_conversion_event_repository_count_ad_conversations_by_ad_by_day_and_ad_channel_scoping_count_ad_conversations_by_day_and_ad_scopes_instagram_by_channel_even_without_an_integration_id()
 {
    let db = fixture().await;
    mixed_ad_conversations(&db).await;
    let rows = read(&db, |c| {
        ace::count_ad_conversations_by_day_and_ad(c, &channel_range("instagram"))
    })
    .await;
    assert_eq!(
        rows,
        vec![ConversationCount {
            date: Some("2026-08-05".into()),
            ad_id: Some("ad-1".into()),
            channel: None,
            conversations: 2
        }]
    );
}

// ---------------------------------------------------------------------------
// event counts default to whatsapp when channel omitted
// ---------------------------------------------------------------------------

async fn events_on_all_channels(db: &Db) {
    let t = ms("2026-08-05T10:00:00.000Z");
    repo_common::setup(db, move |tx| {
        event(tx, 1, "whatsapp", IW1, "lead", "ad-1", None, t, "sent");
        event(tx, 2, "messenger", IM, "lead", "ad-1", None, t, "sent");
        event(tx, 3, "instagram", IGI, "lead", "ad-1", None, t, "sent");
        event(tx, 4, "instagram", IGI, "lead", "ad-1", None, t, "sent");
        Ok(())
    })
    .await;
}

fn lead(count: i64, channel: Option<&str>) -> EventCount {
    EventCount {
        date: None,
        ad_id: Some("ad-1".into()),
        event_type: "lead".into(),
        channel: channel.map(str::to_owned),
        count,
        purchase_value: None,
    }
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:796 "adsConversionEventRepository — event counts default to whatsapp when channel omitted > countConversionEventsByAd without channel filters to whatsapp events only"
#[tokio::test]
async fn ads_conversion_event_repository_event_counts_default_to_whatsapp_when_channel_omitted_count_conversion_events_by_ad_without_channel_filters_to_whatsapp_events_only()
 {
    let db = fixture().await;
    events_on_all_channels(&db).await;
    let rows = read(&db, |c| ace::count_conversion_events_by_ad(c, &range())).await;
    assert_eq!(rows, vec![lead(1, None)]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:815 "adsConversionEventRepository — event counts default to whatsapp when channel omitted > countByCapiStatus without channel filters to whatsapp events only"
#[tokio::test]
async fn ads_conversion_event_repository_event_counts_default_to_whatsapp_when_channel_omitted_count_by_capi_status_without_channel_filters_to_whatsapp_events_only()
 {
    let db = fixture().await;
    events_on_all_channels(&db).await;
    let rows = read(&db, |c| ace::count_by_capi_status(c, &range())).await;
    let got: Vec<(String, i64)> = rows.into_iter().map(|r| (r.capi_status, r.count)).collect();
    assert_eq!(got, vec![("sent".to_owned(), 1)]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:832 "adsConversionEventRepository — event counts default to whatsapp when channel omitted > countConversionEventsByAd with an explicit channel filters to that channel"
#[tokio::test]
async fn ads_conversion_event_repository_event_counts_default_to_whatsapp_when_channel_omitted_count_conversion_events_by_ad_with_an_explicit_channel_filters_to_that_channel()
 {
    let db = fixture().await;
    events_on_all_channels(&db).await;
    let rows = read(&db, |c| {
        ace::count_conversion_events_by_ad(c, &channel_range("messenger"))
    })
    .await;
    assert_eq!(rows, vec![lead(1, None)]);
}

// ---------------------------------------------------------------------------
// allChannels
// ---------------------------------------------------------------------------

fn all_range() -> DateRangeInput {
    let mut r = range();
    r.all_channels = true;
    r
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:860 "adsConversionEventRepository — allChannels ('All channels' Ads Analytics default) > countConversionEventsByAd with allChannels drops the channel eq-filter entirely"
#[tokio::test]
async fn ads_conversion_event_repository_all_channels_all_channels_ads_analytics_default_count_conversion_events_by_ad_with_all_channels_drops_the_channel_eq_filter_entirely()
 {
    let db = fixture().await;
    events_on_all_channels(&db).await;
    let rows = read(&db, |c| ace::count_conversion_events_by_ad(c, &all_range())).await;
    assert_eq!(rows.iter().map(|r| r.count).sum::<i64>(), 4);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:878 "adsConversionEventRepository — allChannels ('All channels' Ads Analytics default) > countConversionEventsByAd with allChannels additionally groups by channel"
#[tokio::test]
async fn ads_conversion_event_repository_all_channels_all_channels_ads_analytics_default_count_conversion_events_by_ad_with_all_channels_additionally_groups_by_channel()
 {
    let db = fixture().await;
    let t = ms("2026-08-05T10:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        event(tx, 2, "messenger", IM, "lead", "ad-1", None, t, "sent");
        event(tx, 3, "instagram", IGI, "lead", "ad-1", None, t, "sent");
        event(tx, 4, "instagram", IGI, "lead", "ad-1", None, t, "sent");
        Ok(())
    })
    .await;
    let mut rows = read(&db, |c| ace::count_conversion_events_by_ad(c, &all_range())).await;
    rows.sort_by(|a, b| b.channel.cmp(&a.channel));
    assert_eq!(
        rows,
        vec![lead(1, Some("messenger")), lead(2, Some("instagram"))]
    );
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:929 "adsConversionEventRepository — allChannels ('All channels' Ads Analytics default) > countConversionEventsByAd single-channel path is byte-identical whether allChannels is omitted or explicitly false"
#[tokio::test]
async fn ads_conversion_event_repository_all_channels_all_channels_ads_analytics_default_count_conversion_events_by_ad_single_channel_path_is_byte_identical_whether_all_channels_is_omitted_or_explicitly_false()
 {
    let db = fixture().await;
    events_on_all_channels(&db).await;
    let omitted = read(&db, |c| ace::count_conversion_events_by_ad(c, &range())).await;
    let mut input = range();
    input.all_channels = false;
    let explicit = read(&db, move |c| ace::count_conversion_events_by_ad(c, &input)).await;
    assert_eq!(omitted, explicit);
    // Grouped by (adId, eventType) only: no channel on the rows.
    assert!(omitted.iter().all(|r| r.channel.is_none()));
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:965 "adsConversionEventRepository — allChannels ('All channels' Ads Analytics default) > countByCapiStatus with allChannels drops the channel eq-filter (delivery totals are channel-agnostic)"
#[tokio::test]
async fn ads_conversion_event_repository_all_channels_all_channels_ads_analytics_default_count_by_capi_status_with_all_channels_drops_the_channel_eq_filter_delivery_totals_are_channel_agnostic()
 {
    let db = fixture().await;
    events_on_all_channels(&db).await;
    let rows = read(&db, |c| ace::count_by_capi_status(c, &all_range())).await;
    let got: Vec<(String, i64)> = rows.into_iter().map(|r| (r.capi_status, r.count)).collect();
    assert_eq!(got, vec![("sent".to_owned(), 4)]);
}

fn all_conv(date: Option<&str>, ad: &str, channel: &str, n: i64) -> ConversationCount {
    ConversationCount {
        date: date.map(str::to_owned),
        ad_id: Some(ad.to_owned()),
        channel: Some(channel.to_owned()),
        conversations: n,
    }
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:982 "adsConversionEventRepository — allChannels ('All channels' Ads Analytics default) > countAllChannelConversationsByAd reuses the ctwaClid-OR-ad-referral predicate and groups by (adId, channel)"
#[tokio::test]
async fn ads_conversion_event_repository_all_channels_all_channels_ads_analytics_default_count_all_channel_conversations_by_ad_reuses_the_ctwa_clid_or_ad_referral_predicate_and_groups_by_ad_id_channel()
 {
    let db = fixture().await;
    let t = ms("2026-08-05T10:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        for i in 0..3 {
            contact_ci(
                tx,
                100 + i * 2,
                WA1,
                "whatsapp",
                ctwa("ad-1"),
                t,
                "W",
                None,
                None,
            );
        }
        contact_ci(tx, 200, MS, "messenger", ads("ad-1"), t, "M", None, None);
        for i in 0..5 {
            contact_ci(
                tx,
                300 + i * 2,
                IG,
                "instagram",
                ads("ad-2"),
                t,
                "I",
                None,
                None,
            );
        }
        // Not an ad conversation: a SHORTLINK referral and an empty click id.
        contact_ci(
            tx,
            400,
            MS,
            "messenger",
            Some(json!({ "adId": "ad-1", "source": "SHORTLINK" })),
            t,
            "X",
            None,
            None,
        );
        contact_ci(
            tx,
            402,
            WA1,
            "whatsapp",
            Some(json!({ "adId": "ad-1", "ctwaClid": "" })),
            t,
            "Y",
            None,
            None,
        );
        Ok(())
    })
    .await;
    let rows = read(&db, |c| {
        ace::count_all_channel_conversations_by_ad(c, &range())
    })
    .await;
    let mut rows = rows;
    rows.sort_by(|a, b| (&a.ad_id, b.channel.clone()).cmp(&(&b.ad_id, a.channel.clone())));
    assert_eq!(
        rows,
        vec![
            all_conv(None, "ad-1", "whatsapp", 3),
            all_conv(None, "ad-1", "messenger", 1),
            all_conv(None, "ad-2", "instagram", 5),
        ]
    );
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:1019 "adsConversionEventRepository — allChannels ('All channels' Ads Analytics default) > countAllChannelConversationsByDayAndAd groups by (date, adId, channel)"
#[tokio::test]
async fn ads_conversion_event_repository_all_channels_all_channels_ads_analytics_default_count_all_channel_conversations_by_day_and_ad_groups_by_date_ad_id_channel()
 {
    let db = fixture().await;
    let t = ms("2026-08-01T10:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        contact_ci(tx, 10, WA1, "whatsapp", ctwa("ad-1"), t, "W", None, None);
        Ok(())
    })
    .await;
    let rows = read(&db, |c| {
        ace::count_all_channel_conversations_by_day_and_ad(c, &range())
    })
    .await;
    assert_eq!(
        rows,
        vec![all_conv(Some("2026-08-01"), "ad-1", "whatsapp", 1)]
    );
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:1054 "adsConversionEventRepository — allChannels ('All channels' Ads Analytics default) > listExportSegmentRows (conversations) with allChannels selects the row's channel and keeps the shared any-channel predicate"
#[tokio::test]
async fn ads_conversion_event_repository_all_channels_all_channels_ads_analytics_default_list_export_segment_rows_conversations_with_all_channels_selects_the_row_s_channel_and_keeps_the_shared_any_channel_predicate()
 {
    let db = fixture().await;
    let t = ms("2026-07-15T00:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        contact_ci(
            tx,
            10,
            IG,
            "instagram",
            ads("ad-1"),
            t,
            "Ada",
            Some("+84900000000"),
            None,
        );
        Ok(())
    })
    .await;
    let input = export("conversations", 50, true, None);
    let result = read(&db, move |c| ace::list_export_segment_rows(c, &input)).await;
    assert_eq!(
        result.rows,
        vec![ExportRow {
            id: 11,
            contact_id: 10,
            contact_name: Some("Ada".into()),
            phone_number: Some("+84900000000".into()),
            email: None,
            ad_id: Some("ad-1".into()),
            occurred_at: t,
            channel: Some("instagram".into()),
        }]
    );
    assert!(!result.has_more);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:1097 "adsConversionEventRepository — allChannels ('All channels' Ads Analytics default) > listExportSegmentRows (leads/purchases) with allChannels selects the row's channel"
#[tokio::test]
async fn ads_conversion_event_repository_all_channels_all_channels_ads_analytics_default_list_export_segment_rows_leads_purchases_with_all_channels_selects_the_row_s_channel()
 {
    let db = fixture().await;
    let t = ms("2026-07-20T00:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        let ci = contact_ci(
            tx,
            10,
            MS,
            "messenger",
            ads("ad-2"),
            t,
            "Bob",
            Some("+84900000001"),
            Some("bob@example.com"),
        );
        event(
            tx,
            500,
            "messenger",
            IM,
            "purchase",
            "ad-2",
            Some(ci),
            t,
            "pending",
        );
        Ok(())
    })
    .await;
    let input = export("purchases", 50, true, None);
    let result = read(&db, move |c| ace::list_export_segment_rows(c, &input)).await;
    assert_eq!(
        result.rows,
        vec![ExportRow {
            id: 500,
            contact_id: 10,
            contact_name: Some("Bob".into()),
            phone_number: Some("+84900000001".into()),
            email: Some("bob@example.com".into()),
            ad_id: Some("ad-2".into()),
            occurred_at: t,
            channel: Some("messenger".into()),
        }]
    );
    assert!(!result.has_more);
}

// ---------------------------------------------------------------------------
// day-bucketing — viewer timezone parameterization
// ---------------------------------------------------------------------------

fn tz_range(tz: Option<&str>) -> DateRangeInput {
    let mut r = range();
    r.timezone = tz.map(str::to_owned);
    r
}

fn dates(rows: &[ConversationCount]) -> Vec<String> {
    rows.iter().filter_map(|r| r.date.clone()).collect()
}

/// 2026-08-01T20:00Z: 2026-08-01 in UTC, 2026-08-02 in Asia/Saigon (UTC+7).
fn late_evening() -> i64 {
    ms("2026-08-01T20:00:00.000Z")
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:1157 "adsConversionEventRepository day-bucketing — viewer timezone parameterization > countCtwaConversationsByDayAndAd defaults day-bucketing to UTC when timezone is omitted"
#[tokio::test]
async fn ads_conversion_event_repository_day_bucketing_viewer_timezone_parameterization_count_ctwa_conversations_by_day_and_ad_defaults_day_bucketing_to_utc_when_timezone_is_omitted()
 {
    let db = fixture().await;
    let t = late_evening();
    repo_common::setup(&db, move |tx| {
        contact_ci(tx, 10, WA1, "whatsapp", ctwa("ad-1"), t, "W", None, None);
        Ok(())
    })
    .await;
    let rows = read(&db, |c| {
        ace::count_ctwa_conversations_by_day_and_ad(c, &tz_range(None))
    })
    .await;
    assert_eq!(dates(&rows), vec!["2026-08-01".to_owned()]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:1174 "adsConversionEventRepository day-bucketing — viewer timezone parameterization > countCtwaConversationsByDayAndAd parameterizes AT TIME ZONE on the given viewer timezone"
#[tokio::test]
async fn ads_conversion_event_repository_day_bucketing_viewer_timezone_parameterization_count_ctwa_conversations_by_day_and_ad_parameterizes_at_time_zone_on_the_given_viewer_timezone()
 {
    let db = fixture().await;
    let t = late_evening();
    repo_common::setup(&db, move |tx| {
        contact_ci(tx, 10, WA1, "whatsapp", ctwa("ad-1"), t, "W", None, None);
        Ok(())
    })
    .await;
    let rows = read(&db, |c| {
        ace::count_ctwa_conversations_by_day_and_ad(c, &tz_range(Some("Asia/Saigon")))
    })
    .await;
    assert_eq!(dates(&rows), vec!["2026-08-02".to_owned()]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:1193 "adsConversionEventRepository day-bucketing — viewer timezone parameterization > countAdConversationsByDayAndAd defaults to UTC and parameterizes an explicit viewer timezone"
#[tokio::test]
async fn ads_conversion_event_repository_day_bucketing_viewer_timezone_parameterization_count_ad_conversations_by_day_and_ad_defaults_to_utc_and_parameterizes_an_explicit_viewer_timezone()
 {
    let db = fixture().await;
    // 2026-08-02T02:00Z: 2026-08-02 in UTC, 2026-08-01 in America/New_York (UTC-4).
    let t = ms("2026-08-02T02:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        contact_ci(tx, 10, MS, "messenger", ads("ad-1"), t, "M", None, None);
        Ok(())
    })
    .await;
    let utc = read(&db, |c| {
        ace::count_ad_conversations_by_day_and_ad(c, &channel_range("messenger"))
    })
    .await;
    assert_eq!(dates(&utc), vec!["2026-08-02".to_owned()]);
    let mut input = channel_range("messenger");
    input.timezone = Some("America/New_York".into());
    let ny = read(&db, move |c| {
        ace::count_ad_conversations_by_day_and_ad(c, &input)
    })
    .await;
    assert_eq!(dates(&ny), vec!["2026-08-01".to_owned()]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:1220 "adsConversionEventRepository day-bucketing — viewer timezone parameterization > countAllChannelConversationsByDayAndAd defaults to UTC and parameterizes an explicit viewer timezone"
#[tokio::test]
async fn ads_conversion_event_repository_day_bucketing_viewer_timezone_parameterization_count_all_channel_conversations_by_day_and_ad_defaults_to_utc_and_parameterizes_an_explicit_viewer_timezone()
 {
    let db = fixture().await;
    let t = late_evening();
    repo_common::setup(&db, move |tx| {
        contact_ci(tx, 10, WA1, "whatsapp", ctwa("ad-1"), t, "W", None, None);
        Ok(())
    })
    .await;
    let utc = read(&db, |c| {
        ace::count_all_channel_conversations_by_day_and_ad(c, &tz_range(None))
    })
    .await;
    assert_eq!(dates(&utc), vec!["2026-08-01".to_owned()]);
    let saigon = read(&db, |c| {
        ace::count_all_channel_conversations_by_day_and_ad(c, &tz_range(Some("Asia/Saigon")))
    })
    .await;
    assert_eq!(dates(&saigon), vec!["2026-08-02".to_owned()]);
}

/// Port of: packages/database/__tests__/ads-conversion-event-repository.test.ts:1245 "adsConversionEventRepository day-bucketing — viewer timezone parameterization > countConversionEventsByDayAndAd defaults to UTC and parameterizes an explicit viewer timezone"
#[tokio::test]
async fn ads_conversion_event_repository_day_bucketing_viewer_timezone_parameterization_count_conversion_events_by_day_and_ad_defaults_to_utc_and_parameterizes_an_explicit_viewer_timezone()
 {
    let db = fixture().await;
    let t = late_evening();
    repo_common::setup(&db, move |tx| {
        event(tx, 1, "whatsapp", IW1, "lead", "ad-1", None, t, "sent");
        Ok(())
    })
    .await;
    let utc = read(&db, |c| {
        ace::count_conversion_events_by_day_and_ad(c, &tz_range(None))
    })
    .await;
    assert_eq!(utc[0].date.as_deref(), Some("2026-08-01"));
    let saigon = read(&db, |c| {
        ace::count_conversion_events_by_day_and_ad(c, &tz_range(Some("Asia/Saigon")))
    })
    .await;
    assert_eq!(saigon[0].date.as_deref(), Some("2026-08-02"));
}
