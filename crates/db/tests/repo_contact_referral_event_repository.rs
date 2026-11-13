//! Port of `packages/database/__tests__/contact-referral-event-repository.test.ts`
//! (P3.2b). The TS mocks the Drizzle select / execute chains and renders the SQL with the
//! Postgres dialect; here the queries run on a migrated temp file (adapted) and the rows
//! show the ad-touch scope, the pins, the bounded scan, the keyset and the report math.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use std::collections::BTreeMap;

use chatbotx_db::repo::contact_referral_event::{
    self as cre, AdStatsRange, AdTouchStatsRow, ContactAdTouchCursor, FindLatestAdTouchInput,
    LinkedConversionRow, ListRecentAdsInput, RECENT_AD_TOUCH_SCAN_LIMIT, RecentAd,
};
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value;
use chatbotx_db::{Db, DbError, WriteTx};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{insert_ci, insert_contact, insert_inbox, ms, ok, scalar_i64, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
/// Contact inboxes: 81 (4001), 82 (4002), 84 (4003) on inbox 3001 (WS); 83 (4201) on 3002
/// (OTHER).
const CI1: i64 = 81;
const CI2: i64 = 82;

fn t(day: u32, hour: u32) -> i64 {
    ms(&format!("2026-10-{day:02}T{hour:02}:00:00.000Z"))
}

async fn fixture() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER).await;
    setup(&db, |tx| {
        insert_inbox(tx, 3002, OTHER, "messenger", "page-other")?;
        insert_contact(tx, 4002, WS)?;
        insert_contact(tx, 4003, WS)?;
        insert_contact(tx, 4201, OTHER)?;
        insert_ci(tx, CI1, 4001, 3001, "messenger", "psid-81")?;
        insert_ci(tx, CI2, 4002, 3001, "messenger", "psid-82")?;
        insert_ci(tx, 83, 4201, 3002, "messenger", "psid-83")?;
        insert_ci(tx, 84, 4003, 3001, "messenger", "psid-84")?;
        Ok(())
    })
    .await;
    db
}

/// One `ContactReferralEvent` row (defaults: WS, contact 4001 on 81, messenger, `ad`).
#[derive(Clone)]
struct Touch {
    id: i64,
    ws: i64,
    contact: i64,
    ci: i64,
    channel: &'static str,
    kind: &'static str,
    provider: Option<&'static str>,
    ad: Option<String>,
    click: Option<(&'static str, &'static str)>,
    at: i64,
    title: Option<&'static str>,
}

fn touch(id: i64, ad: Option<&str>, at: i64) -> Touch {
    Touch {
        id,
        ws: WS,
        contact: 4001,
        ci: CI1,
        channel: "messenger",
        kind: "ad",
        provider: Some("meta"),
        ad: ad.map(str::to_owned),
        click: None,
        at,
        title: None,
    }
}

fn insert_touch(tx: &Connection, t: &Touch) -> Result<usize, DbError> {
    let metadata = t
        .title
        .map(|title| serde_json::json!({ "adTitle": title }).to_string());
    ins!(tx, "ContactReferralEvent", {
        "id" => t.id, "workspaceId" => t.ws, "contactId" => t.contact, "contactInboxId" => t.ci,
        "channel" => t.channel.to_owned(), "type" => t.kind.to_owned(),
        "provider" => t.provider.map(str::to_owned), "adId" => t.ad.clone(),
        "clickIdType" => t.click.map(|c| c.0.to_owned()), "clickId" => t.click.map(|c| c.1.to_owned()),
        "occurredAt" => t.at, "metadata" => metadata, "dedupKey" => format!("dedup-{}", t.id),
    })
}

async fn touches(db: &Db, rows: Vec<Touch>) {
    setup(db, move |tx| {
        for row in &rows {
            insert_touch(tx, row)?;
        }
        Ok(())
    })
    .await;
}

async fn run<T, F>(db: &Db, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> repo::Result<T> + Send + 'static,
{
    ok(repo::read(db, f).await)
}

async fn write<T, F>(db: &Db, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce(&mut WriteTx<'_>) -> repo::Result<T> + Send + 'static,
{
    ok(repo::write(db, f).await)
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

// ---------------------------------------------------------------------------
// listAdIdsByContact
// ---------------------------------------------------------------------------

#[tokio::test]
async fn contact_referral_event_repository_list_ad_ids_by_contact_is_the_batched_query_for_one_contact_so_ad_ids_and_the_export_share_one_implementation()
 {
    let db = fixture().await;
    let mut link = touch(5, Some("300"), t(1, 0));
    link.kind = "refLink";
    touches(
        &db,
        vec![
            touch(1, Some("200"), t(2, 0)),
            touch(2, Some("100"), t(1, 1)),
            touch(3, Some("100"), t(3, 0)),
            touch(4, Some(""), t(1, 0)),
            link,
        ],
    )
    .await;
    let ids = run(&db, |c| cre::list_ad_ids_by_contact(c, WS, 4001, 50)).await;
    assert_eq!(ids, strings(&["100", "200"]));
    let batched = run(&db, |c| cre::list_ad_ids_by_contacts(c, WS, &[4001], 50)).await;
    assert_eq!(batched.get(&4001), Some(&ids));
    assert_eq!(
        run(&db, |c| cre::list_ad_ids_by_contact(c, WS, 4001, 1)).await,
        strings(&["100"])
    );
}

#[tokio::test]
async fn contact_referral_event_repository_list_ad_ids_by_contact_is_empty_for_a_contact_with_no_ad_touch()
 {
    let db = fixture().await;
    touches(&db, vec![touch(1, Some("200"), t(2, 0))]).await;
    assert!(
        run(&db, |c| cre::list_ad_ids_by_contact(c, WS, 4002, 1))
            .await
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// findLatestAdTouchId
// ---------------------------------------------------------------------------

/// Google touches 11 (t1), 12 (t2) and 15 (t1, gclid, ad 999) on 81; meta touches 13
/// (t3, ad 120) and 16 (t5, ad 121) on 81; google 14 (t4) on 82.
async fn pin_fixture() -> common::TestDb {
    let db = fixture().await;
    let google = |id: i64, at: i64| Touch {
        provider: Some("google"),
        ..touch(id, None, at)
    };
    touches(
        &db,
        vec![
            google(11, t(1, 0)),
            google(12, t(2, 0)),
            Touch {
                click: Some(("gclid", "gclid-1234567890")),
                ad: Some("999".to_owned()),
                ..google(15, t(1, 0))
            },
            touch(13, Some("120"), t(3, 0)),
            touch(16, Some("121"), t(5, 0)),
            Touch {
                contact: 4002,
                ci: CI2,
                ..google(14, t(4, 0))
            },
        ],
    )
    .await;
    db
}

async fn latest(db: &Db, input: FindLatestAdTouchInput) -> Option<i64> {
    run(db, move |c| cre::find_latest_ad_touch_id(c, &input)).await
}

fn pin(provider: &str) -> FindLatestAdTouchInput {
    FindLatestAdTouchInput {
        workspace_id: WS,
        contact_inbox_id: CI1,
        provider: provider.to_owned(),
        ..FindLatestAdTouchInput::default()
    }
}

#[tokio::test]
async fn contact_referral_event_repository_find_latest_ad_touch_id_takes_the_newest_touch_of_the_provider_for_the_contact_inbox()
 {
    let db = pin_fixture().await;
    assert_eq!(latest(&db, pin("google")).await, Some(12));
}

#[tokio::test]
async fn contact_referral_event_repository_find_latest_ad_touch_id_pins_the_touch_to_its_click_id_when_the_conversion_names_one()
 {
    let db = pin_fixture().await;
    let input = FindLatestAdTouchInput {
        click_id: Some("gclid-1234567890".to_owned()),
        ..pin("google")
    };
    assert_eq!(latest(&db, input).await, Some(15));
}

#[tokio::test]
async fn contact_referral_event_repository_find_latest_ad_touch_id_pins_a_touch_without_a_click_id_to_the_ad_the_conversion_was_credited_to()
 {
    let db = pin_fixture().await;
    let input = FindLatestAdTouchInput {
        ad_id: Some("120".to_owned()),
        ..pin("meta")
    };
    assert_eq!(latest(&db, input).await, Some(13));
}

#[tokio::test]
async fn contact_referral_event_repository_find_latest_ad_touch_id_a_click_id_wins_over_the_ad_id()
{
    let db = pin_fixture().await;
    let input = FindLatestAdTouchInput {
        click_id: Some("gclid-1234567890".to_owned()),
        ad_id: Some("120".to_owned()),
        ..pin("google")
    };
    assert_eq!(latest(&db, input).await, Some(15));
}

#[tokio::test]
async fn contact_referral_event_repository_find_latest_ad_touch_id_is_null_when_no_touch_was_logged()
 {
    let db = pin_fixture().await;
    let input = FindLatestAdTouchInput {
        contact_inbox_id: CI2,
        ..pin("meta")
    };
    assert_eq!(latest(&db, input).await, None);
}

// ---------------------------------------------------------------------------
// insertIgnoreDuplicate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn contact_referral_event_repository_insert_ignore_duplicate_conflicts_on_the_workspace_contact_inbox_dedup_key_unique_index()
 {
    let db = fixture().await;
    let values = |ci: i64, contact: i64| {
        vec![
            ("workspaceId", Value::Integer(WS)),
            ("contactId", Value::Integer(contact)),
            ("contactInboxId", Value::Integer(ci)),
            ("channel", Value::Text("messenger".to_owned())),
            ("type", Value::Text("ad".to_owned())),
            ("occurredAt", Value::Integer(t(1, 0))),
            ("dedupKey", Value::Text("ad:ad-1:mid-1".to_owned())),
        ]
    };
    let first = write(&db, move |c| {
        cre::insert_ignore_duplicate(c, &values(CI1, 4001))
    })
    .await;
    assert_eq!(first.map(|r| r.dedup_key), Some("ad:ad-1:mid-1".to_owned()));
    let duplicate = write(&db, move |c| {
        cre::insert_ignore_duplicate(c, &values(CI1, 4001))
    })
    .await;
    assert_eq!(duplicate, None);
    let other_inbox = write(&db, move |c| {
        cre::insert_ignore_duplicate(c, &values(CI2, 4002))
    })
    .await;
    assert!(other_inbox.is_some());
}

// ---------------------------------------------------------------------------
// listRecentAds
// ---------------------------------------------------------------------------

async fn recent(db: &Db, input: ListRecentAdsInput) -> Vec<RecentAd> {
    run(db, move |c| cre::list_recent_ads(c, &input)).await
}

fn recent_input(limit: i64) -> ListRecentAdsInput {
    ListRecentAdsInput {
        workspace_id: WS,
        limit,
        ..ListRecentAdsInput::default()
    }
}

fn ad_ids(rows: &[RecentAd]) -> Vec<String> {
    rows.iter().map(|r| r.ad_id.clone()).collect()
}

#[tokio::test]
async fn contact_referral_event_repository_list_recent_ads_scans_only_a_bounded_run_of_the_workspace_s_newest_ad_touches()
 {
    let db = fixture().await;
    let base = t(10, 0);
    let mut rows: Vec<Touch> = (0..RECENT_AD_TOUCH_SCAN_LIMIT)
        .map(|i| touch(10_000 + i, Some("200"), base + i))
        .collect();
    // Older than the scanned run: never offered.
    rows.push(touch(1, Some("100"), t(1, 0)));
    // Another workspace's and a non-ad touch, both newer.
    rows.push(Touch {
        ws: OTHER,
        contact: 4201,
        ci: 83,
        ..touch(2, Some("300"), t(20, 0))
    });
    rows.push(Touch {
        kind: "refLink",
        ..touch(3, Some("400"), t(20, 0))
    });
    touches(&db, rows).await;
    assert_eq!(
        ad_ids(&recent(&db, recent_input(50)).await),
        strings(&["200"])
    );
}

#[tokio::test]
async fn contact_referral_event_repository_list_recent_ads_only_offers_ad_ids_the_clicked_ad_filter_accepts_before_the_scan_limit()
 {
    let db = fixture().await;
    touches(
        &db,
        vec![
            touch(1, Some("120"), t(1, 0)),
            touch(2, Some("0120"), t(2, 0)),
            touch(3, Some("abc"), t(3, 0)),
            touch(4, Some("9223372036854775808"), t(4, 0)),
            touch(5, Some("9223372036854775807"), t(5, 0)),
            touch(6, Some("12345678901234567890"), t(6, 0)),
        ],
    )
    .await;
    assert_eq!(
        ad_ids(&recent(&db, recent_input(50)).await),
        strings(&["9223372036854775807", "120"])
    );
}

#[tokio::test]
async fn contact_referral_event_repository_list_recent_ads_returns_each_ad_with_its_headline_bounded_by_the_limit()
 {
    let db = fixture().await;
    let titled = |id: i64, ad: &str, at: i64, title: Option<&'static str>| Touch {
        title,
        ..touch(id, Some(ad), at)
    };
    touches(
        &db,
        vec![
            titled(1, "120", t(1, 0), Some("Old")),
            titled(2, "120", t(2, 0), Some("Summer sale")),
            titled(3, "120", t(5, 0), None),
            titled(4, "121", t(4, 0), None),
            titled(5, "122", t(1, 0), Some("Oldest")),
        ],
    )
    .await;
    assert_eq!(
        recent(&db, recent_input(2)).await,
        vec![
            RecentAd {
                ad_id: "120".to_owned(),
                title: Some("Summer sale".to_owned())
            },
            RecentAd {
                ad_id: "121".to_owned(),
                title: None
            },
        ]
    );
}

#[tokio::test]
async fn contact_referral_event_repository_list_recent_ads_escapes_like_wildcards_in_the_search_so_it_cannot_match_everything()
 {
    let db = fixture().await;
    touches(
        &db,
        vec![
            Touch {
                title: Some("Get 50%_off now"),
                ..touch(1, Some("130"), t(1, 0))
            },
            Touch {
                title: Some("500 off"),
                ..touch(2, Some("131"), t(2, 0))
            },
            touch(3, Some("132"), t(3, 0)),
        ],
    )
    .await;
    let input = ListRecentAdsInput {
        search: Some("50%_off".to_owned()),
        ..recent_input(50)
    };
    assert_eq!(ad_ids(&recent(&db, input).await), strings(&["130"]));
}

#[tokio::test]
async fn contact_referral_event_repository_list_recent_ads_narrows_to_the_given_ad_ids_for_a_label_lookup()
 {
    let db = fixture().await;
    touches(
        &db,
        vec![
            touch(1, Some("120"), t(1, 0)),
            touch(2, Some("121"), t(2, 0)),
            touch(3, Some("122"), t(3, 0)),
        ],
    )
    .await;
    let input = ListRecentAdsInput {
        ad_ids: Some(strings(&["120", "121"])),
        ..recent_input(5)
    };
    assert_eq!(ad_ids(&recent(&db, input).await), strings(&["121", "120"]));
}

#[tokio::test]
async fn contact_referral_event_repository_list_recent_ads_drops_a_null_ad_id_the_database_should_never_return()
 {
    let db = fixture().await;
    touches(
        &db,
        vec![touch(1, None, t(5, 0)), touch(2, Some("120"), t(1, 0))],
    )
    .await;
    assert_eq!(
        ad_ids(&recent(&db, recent_input(50)).await),
        strings(&["120"])
    );
}

// ---------------------------------------------------------------------------
// listAdTouchesByContact
// ---------------------------------------------------------------------------

/// 4001: ad touches 20 (t1), 21 and 22 (both t2), a refLink 23 (t3); 4002: 24 (t3).
async fn touch_list_fixture() -> common::TestDb {
    let db = fixture().await;
    touches(
        &db,
        vec![
            touch(20, Some("120"), t(1, 0)),
            touch(21, Some("121"), t(2, 0)),
            touch(22, Some("122"), t(2, 0)),
            Touch {
                kind: "refLink",
                ..touch(23, None, t(3, 0))
            },
            Touch {
                contact: 4002,
                ci: CI2,
                ..touch(24, Some("120"), t(3, 0))
            },
        ],
    )
    .await;
    db
}

async fn touch_ids(db: &Db, limit: i64, cursor: Option<ContactAdTouchCursor>) -> Vec<i64> {
    run(db, move |c| {
        cre::list_ad_touches_by_contact(c, WS, 4001, limit, cursor)
    })
    .await
    .into_iter()
    .map(|r| r.id)
    .collect()
}

#[tokio::test]
async fn contact_referral_event_repository_list_ad_touches_by_contact_scopes_to_the_workspace_and_contact_and_fetches_one_row_past_the_page()
 {
    let db = touch_list_fixture().await;
    assert_eq!(touch_ids(&db, 1, None).await.len(), 2);
    assert_eq!(touch_ids(&db, 20, None).await.len(), 3);
    let other = run(&db, |c| {
        cre::list_ad_touches_by_contact(c, OTHER, 4001, 20, None)
    })
    .await;
    assert!(other.is_empty());
}

#[tokio::test]
async fn contact_referral_event_repository_list_ad_touches_by_contact_orders_newest_first_with_the_id_as_a_stable_tie_break()
 {
    let db = touch_list_fixture().await;
    assert_eq!(touch_ids(&db, 20, None).await, vec![22, 21, 20]);
}

#[tokio::test]
async fn contact_referral_event_repository_list_ad_touches_by_contact_continues_strictly_after_the_cursor_breaking_time_ties_by_id()
 {
    let db = touch_list_fixture().await;
    let cursor = ContactAdTouchCursor {
        occurred_at: t(2, 0),
        id: 22,
    };
    assert_eq!(touch_ids(&db, 20, Some(cursor)).await, vec![21, 20]);
}

// ---------------------------------------------------------------------------
// ad reports
// ---------------------------------------------------------------------------

fn range(channel: Option<&str>) -> AdStatsRange {
    AdStatsRange {
        workspace_id: WS,
        since: ms("2026-10-01T00:00:00.000Z"),
        until: ms("2026-10-31T00:00:00.000Z"),
        channel: channel.map(str::to_owned),
    }
}

#[tokio::test]
async fn contact_referral_event_repository_ad_reports_touch_stats_read_only_the_workspace_s_range_by_parameter()
 {
    let db = fixture().await;
    let second = |t: Touch| Touch {
        contact: 4002,
        ci: CI2,
        ..t
    };
    touches(
        &db,
        vec![
            Touch {
                title: Some("A"),
                ..touch(1, Some("120"), t(2, 0))
            },
            touch(2, Some("121"), t(3, 0)),
            second(touch(3, Some("121"), t(2, 1))),
            // `until` is inclusive; before `since`, an empty ad id and another workspace
            // are out.
            second(touch(4, Some("121"), ms("2026-10-31T00:00:00.000Z"))),
            touch(5, Some("122"), ms("2026-09-30T23:59:59.999Z")),
            touch(6, Some(""), t(2, 0)),
            Touch {
                ws: OTHER,
                contact: 4201,
                ci: 83,
                ..touch(7, Some("120"), t(2, 0))
            },
        ],
    )
    .await;
    let rows = run(&db, |c| cre::aggregate_ad_touch_stats(c, &range(None), 50)).await;
    assert_eq!(
        rows,
        vec![
            AdTouchStatsRow {
                ad_id: "121".to_owned(),
                title: None,
                any_touch_contacts: 2,
                first_touch_contacts: 1
            },
            AdTouchStatsRow {
                ad_id: "120".to_owned(),
                title: Some("A".to_owned()),
                any_touch_contacts: 1,
                first_touch_contacts: 1
            },
        ]
    );
    let limited = run(&db, |c| cre::aggregate_ad_touch_stats(c, &range(None), 1)).await;
    assert_eq!(limited.len(), 1);
}

#[tokio::test]
async fn contact_referral_event_repository_ad_reports_touch_stats_can_be_narrowed_to_one_channel_without_interpolating_it()
 {
    let db = fixture().await;
    touches(
        &db,
        vec![
            touch(1, Some("120"), t(2, 0)),
            Touch {
                channel: "whatsapp",
                ..touch(2, Some("121"), t(2, 0))
            },
        ],
    )
    .await;
    let hostile = run(&db, |c| {
        cre::aggregate_ad_touch_stats(c, &range(Some("messenger'; DROP TABLE x;--")), 5)
    })
    .await;
    assert!(hostile.is_empty());
    let whatsapp = run(&db, |c| {
        cre::aggregate_ad_touch_stats(c, &range(Some("whatsapp")), 5)
    })
    .await;
    assert_eq!(
        whatsapp
            .iter()
            .map(|r| r.ad_id.as_str())
            .collect::<Vec<_>>(),
        vec!["121"]
    );
    assert_eq!(
        scalar_i64(&db, "SELECT count(*) FROM \"ContactReferralEvent\"", vec![]).await,
        Some(2)
    );
}

#[tokio::test]
async fn contact_referral_event_repository_ad_reports_linked_conversions_look_up_nothing_for_no_ads()
 {
    let db = fixture().await;
    let rows = run(&db, |c| {
        cre::aggregate_meta_linked_conversions(c, &range(None), &[])
    })
    .await;
    assert!(rows.is_empty());
}

#[tokio::test]
async fn contact_referral_event_repository_ad_reports_linked_conversions_join_through_the_touch_and_bind_every_ad_id()
 {
    let db = fixture().await;
    touches(
        &db,
        vec![
            touch(1, Some("120"), t(1, 0)),
            touch(2, Some("121"), t(1, 0)),
            touch(3, Some("122"), t(1, 0)),
        ],
    )
    .await;
    setup(&db, |tx| {
        insert_inbox(tx, 3010, WS, "messenger", "page-3010")?;
        ins!(tx, "IntegrationMessenger", {
            "id" => 6001_i64, "auth" => "{}".to_owned(), "pageId" => "page-6001".to_owned(),
            "name" => "Page".to_owned(), "conversationStarters" => "[]".to_owned(),
            "persistentMenus" => "[]".to_owned(), "personas" => "[]".to_owned(),
            "workspaceId" => WS, "inboxId" => 3010_i64,
        })?;
        for (id, touch_id, kind, currency, value, at) in [
            (1_i64, 1_i64, "lead", None, Some("10.5"), t(2, 0)),
            (2, 1, "lead", None, Some("2"), t(3, 0)),
            (3, 2, "purchase", Some("USD"), Some("3"), t(2, 0)),
            (4, 3, "lead", None, Some("7"), t(2, 0)),
            // Out of range.
            (
                5,
                1,
                "lead",
                None,
                Some("100"),
                ms("2026-11-02T00:00:00.000Z"),
            ),
        ] {
            ins!(tx, "AdsConversionEvent", {
                "id" => id, "workspaceId" => WS, "channel" => "messenger".to_owned(),
                "integrationMessengerId" => 6001_i64, "source" => "rule".to_owned(),
                "eventType" => kind.to_owned(), "referralEventId" => touch_id,
                "currency" => currency.map(str::to_owned), "value" => value.map(str::to_owned),
                "occurredAt" => at, "sourceEventId" => format!("src-{id}"),
            })?;
        }
        Ok(())
    })
    .await;
    let rows = run(&db, |c| {
        cre::aggregate_meta_linked_conversions(c, &range(None), &strings(&["120", "121"]))
    })
    .await;
    assert_eq!(
        rows,
        vec![
            LinkedConversionRow {
                ad_id: "120".to_owned(),
                event_type: "lead".to_owned(),
                currency: None,
                count: 2,
                amount: "12.5".to_owned()
            },
            LinkedConversionRow {
                ad_id: "121".to_owned(),
                event_type: "purchase".to_owned(),
                currency: Some("USD".to_owned()),
                count: 1,
                amount: "3".to_owned()
            },
        ]
    );
}

// ---------------------------------------------------------------------------
// listAdIdsByContacts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn contact_referral_event_repository_list_ad_ids_by_contacts_answers_an_empty_page_without_a_query()
 {
    let db = fixture().await;
    assert!(
        run(&db, |c| cre::list_ad_ids_by_contacts(c, WS, &[], 50))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn contact_referral_event_repository_list_ad_ids_by_contacts_lists_a_whole_page_in_one_query_capped_per_contact_and_scoped_to_ad_touches()
 {
    let db = fixture().await;
    touches(
        &db,
        vec![
            touch(1, Some("1001"), t(1, 0)),
            touch(2, Some("1002"), t(2, 0)),
            // An empty ad id never takes a slot of the cap.
            touch(3, Some(""), t(1, 0)),
            Touch {
                contact: 4002,
                ci: CI2,
                ..touch(4, Some("1001"), t(3, 0))
            },
        ],
    )
    .await;
    let result = run(&db, |c| {
        cre::list_ad_ids_by_contacts(c, WS, &[4001, 4002, 4003], 50)
    })
    .await;
    let expected: BTreeMap<i64, Vec<String>> = [
        (4001, strings(&["1001", "1002"])),
        (4002, strings(&["1001"])),
    ]
    .into_iter()
    .collect();
    assert_eq!(result, expected);
    let capped = run(&db, |c| cre::list_ad_ids_by_contacts(c, WS, &[4001], 2)).await;
    assert_eq!(capped.get(&4001), Some(&strings(&["1001", "1002"])));
}
