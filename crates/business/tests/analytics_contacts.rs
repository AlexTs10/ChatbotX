//! Contact analytics (Q.D11 slice 8) on a migrated in-memory database.
//! Ports `contact-stats.repository`, `filtered-stats.repository` (contact
//! parts), `contacts-by-dimension-schema`, `event-filter` (normalization)
//! and `time-series` (empty ranges) of `packages/analytics/__tests__`. The
//! TS tests read the rendered SQL of a mocked driver; here the same cases
//! seed rows and check the result. The main seed is the one of the parity
//! group p1-03 `analytics-contacts.test.ts`.

use std::sync::Arc;

use chatbotx_business::analytics::contacts::{self, DimensionCount};
use chatbotx_business::analytics::{CountPoint, Filter, Granularity, TimeRange};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, params};

const WS: i64 = 501;
const OTHER_WS: i64 = 502;
const I1: i64 = 11; // messenger
const I2: i64 = 12; // telegram

fn ms(iso: &str) -> i64 {
    chatbotx_core::js::parse_date(iso).expect("iso")
}

#[allow(clippy::too_many_arguments)] // one column per argument, as the parity fixture rows
fn event(
    tx: &Connection,
    ws: i64,
    contact: i64,
    kind: &str,
    at: &str,
    source: Option<&str>,
    channel: Option<&str>,
    inbox: Option<i64>,
    lang: Option<(&str, &str)>,
) {
    tx.execute(
        r#"INSERT INTO "AnalyticsContactEvent" ("eventId", "workspaceId", "contactId", "eventType", "occurredAt", "source", "channel", "inboxId", "country", "language")
           VALUES (lower(hex(randomblob(8))), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
        params![ws, contact, kind, ms(at), source, channel, inbox, lang.map(|l| l.0), lang.map(|l| l.1)],
    )
    .unwrap();
}

/// The parity seed: inbox I1 (messenger, 2 contacts), I2 (telegram, 1);
/// contact events C0..C3.
async fn seeded() -> Arc<Db> {
    let db = Db::open_in_memory().unwrap();
    db.migrate().await.unwrap();
    db.write(|tx| {
        tx.execute(r#"INSERT INTO "User" ("id", "email") VALUES (1, 'o@x.test')"#, [])?;
        for ws in [WS, OTHER_WS] {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'W', 1)"#,
                [ws],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status") VALUES
               (11, 'I1', 'messenger', 's1', ?1, 'connected'), (12, 'I2', 'telegram', 's2', ?1, 'connected')"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "InboxContactStat" ("inboxId", "totalContacts") VALUES (11, 2), (12, 1)"#,
            [],
        )?;
        let (m, t) = ((Some("messenger"), Some(I1)), (Some("telegram"), Some(I2)));
        let vi = Some(("VN", "vi"));
        let rows = [
            (100, "contact_created", "2025-02-20T00:00:00Z", Some("direct"), m, vi),
            (101, "contact_created", "2025-03-02T10:00:00Z", Some("direct"), m, vi),
            (101, "contact_created", "2025-03-02T12:00:00Z", Some("direct"), m, vi),
            (102, "contact_created", "2025-03-02T23:30:00Z", Some("ads"), m, vi),
            (103, "contact_created", "2025-03-04T05:00:00Z", Some("direct"), t, Some(("US", "en"))),
            (103, "contact_deleted", "2025-03-05T08:00:00Z", Some("direct"), t, None),
            (101, "contact_blocked", "2025-03-03T01:00:00Z", None, m, None),
            (101, "contact_blocked", "2025-03-03T02:00:00Z", None, m, None),
            (102, "contact_blocked", "2025-03-04T16:30:00Z", None, m, None),
        ];
        for (contact, kind, at, source, (channel, inbox), lang) in rows {
            event(tx, WS, contact, kind, at, source, channel, inbox, lang);
        }
        // Another workspace's event never counts.
        event(tx, OTHER_WS, 900, "contact_created", "2025-03-02T10:00:00Z", Some("direct"), Some("messenger"), None, None);
        Ok(())
    })
    .await
    .unwrap();
    Arc::new(db)
}

fn range(from: &str, to: &str, tz: &str, filter: Filter) -> TimeRange {
    TimeRange::new(WS, Some(ms(from)), Some(ms(to)), tz, filter)
}

fn utc() -> TimeRange {
    range(
        "2025-03-01T00:00:00Z",
        "2025-03-05T23:59:59Z",
        "UTC",
        Filter::default(),
    )
}

fn hcm() -> TimeRange {
    range(
        "2025-03-01T00:00:00+07:00",
        "2025-03-05T23:59:59+07:00",
        "Asia/Ho_Chi_Minh",
        Filter::default(),
    )
}

fn months() -> TimeRange {
    range(
        "2025-01-01T00:00:00Z",
        "2025-03-31T23:59:59Z",
        "UTC",
        Filter::default(),
    )
}

fn inboxes(ids: &[i64]) -> Filter {
    Filter::new(Vec::new(), ids.to_vec())
}

fn channels(names: &[&str]) -> Filter {
    Filter::new(names.iter().map(|c| c.to_string()).collect(), Vec::new())
}

async fn read<R: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> R {
    db.read(move |c| f(c).map_err(|e| chatbotx_db::DbError::other(e.to_string())))
        .await
        .unwrap()
}

fn counts(points: Vec<CountPoint>) -> Vec<i64> {
    points.into_iter().map(|p| p.count).collect()
}

const MARCH: [&str; 5] = [
    "2025-03-01",
    "2025-03-02",
    "2025-03-03",
    "2025-03-04",
    "2025-03-05",
];

#[tokio::test]
async fn running_total_per_utc_day_from_the_baseline() {
    let db = seeded().await;
    let points = read(&db, |c| contacts::contact_counts_series(c, &utc())).await;
    let dates: Vec<String> = points.iter().map(|p| p.date.clone()).collect();
    assert_eq!(dates, MARCH.map(|d| format!("{d}T00:00:00.000Z")));
    assert_eq!(counts(points), [1, 4, 4, 5, 4]);
}

#[tokio::test]
async fn running_total_buckets_by_local_day() {
    let db = seeded().await;
    let points = read(&db, |c| contacts::contact_counts_series(c, &hcm())).await;
    assert_eq!(points[0].date, "2025-03-01T00:00:00.000Z");
    assert_eq!(counts(points), [1, 3, 4, 5, 4]);
}

#[tokio::test]
async fn running_total_uses_monthly_buckets_past_60_days() {
    let db = seeded().await;
    let points = read(&db, |c| contacts::contact_counts_series(c, &months())).await;
    let dates: Vec<String> = points.iter().map(|p| p.date.clone()).collect();
    assert_eq!(
        dates,
        [
            "2025-01-01T00:00:00.000Z",
            "2025-02-01T00:00:00.000Z",
            "2025-03-01T00:00:00.000Z"
        ]
    );
    assert_eq!(counts(points), [0, 1, 4]);
}

#[tokio::test]
async fn anchors_the_running_total_on_the_exact_per_inbox_count() {
    let db = seeded().await;
    let r = TimeRange {
        filter: inboxes(&[I1]),
        ..utc()
    };
    // I1 holds 2 contacts; its events since the start net +3: max(0, 2 - 3).
    assert_eq!(
        counts(read(&db, move |c| contacts::contact_counts_series(c, &r)).await),
        [0, 3, 3, 3, 3]
    );
}

#[tokio::test]
async fn an_inbox_anchored_total_never_drops_below_zero_and_monthly_anchors_the_same_way() {
    let db = seeded().await;
    db.write(|tx| {
        // Only the deletion carries the inbox (created before inbox tracking).
        tx.execute(
            r#"UPDATE "InboxContactStat" SET "totalContacts" = 0 WHERE "inboxId" = 12"#,
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let r = TimeRange {
        filter: inboxes(&[I2]),
        ..utc()
    };
    let points = read(&db, move |c| contacts::contact_counts_series(c, &r)).await;
    assert!(points.iter().all(|p| p.count >= 0), "{points:?}");

    let r = TimeRange {
        filter: inboxes(&[I1]),
        ..months()
    };
    // I1: 2 - (events since 2025-01-01: +4) floored at 0, then +1 in Feb, +3 in Mar.
    assert_eq!(
        counts(read(&db, move |c| contacts::contact_counts_series(c, &r)).await),
        [0, 1, 4]
    );
}

#[tokio::test]
async fn a_channel_only_filter_keeps_the_event_balance_before_the_start() {
    let db = seeded().await;
    let r = TimeRange {
        filter: channels(&["messenger"]),
        ..utc()
    };
    assert_eq!(
        counts(read(&db, move |c| contacts::contact_counts_series(c, &r)).await),
        [1, 4, 4, 4, 4]
    );
}

#[tokio::test]
async fn new_contacts_per_day_count_distinct_contacts() {
    let db = seeded().await;
    let new = |r: TimeRange| {
        let db = Arc::clone(&db);
        async move {
            counts(
                read(&db, move |c| {
                    contacts::distinct_contacts_series(c, &r, "contact_created")
                })
                .await,
            )
        }
    };
    assert_eq!(new(utc()).await, [0, 2, 0, 1, 0]);
    assert_eq!(new(hcm()).await, [0, 1, 1, 1, 0]);
    assert_eq!(new(months()).await, [0, 1, 3]);
    assert_eq!(
        new(TimeRange {
            filter: channels(&["telegram"]),
            ..utc()
        })
        .await,
        [0, 0, 0, 1, 0]
    );
}

#[tokio::test]
async fn blocked_contacts_per_day_count_distinct_contacts() {
    let db = seeded().await;
    let blocked = |r: TimeRange| {
        let db = Arc::clone(&db);
        async move {
            counts(
                read(&db, move |c| {
                    contacts::distinct_contacts_series(c, &r, "contact_blocked")
                })
                .await,
            )
        }
    };
    assert_eq!(blocked(utc()).await, [0, 0, 1, 1, 0]);
    assert_eq!(blocked(hcm()).await, [0, 0, 1, 1, 0]);
    assert_eq!(blocked(months()).await, [0, 0, 2]);
    assert_eq!(
        blocked(TimeRange {
            filter: inboxes(&[I2]),
            ..utc()
        })
        .await,
        [0, 0, 0, 0, 0]
    );
}

#[tokio::test]
async fn distinct_counts_honour_the_filters() {
    let db = seeded().await;
    let count = |r: TimeRange, kind: &'static str| {
        let db = Arc::clone(&db);
        async move { read(&db, move |c| contacts::distinct_contacts_count(c, &r, kind)).await }
    };
    assert_eq!(count(utc(), "contact_blocked").await, 2);
    assert_eq!(
        count(
            TimeRange {
                filter: channels(&["telegram"]),
                ..utc()
            },
            "contact_blocked"
        )
        .await,
        0
    );
    assert_eq!(count(utc(), "contact_created").await, 3);
    assert_eq!(
        count(
            TimeRange {
                filter: channels(&["telegram"]),
                ..utc()
            },
            "contact_created"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            TimeRange {
                filter: inboxes(&[I1]),
                ..utc()
            },
            "contact_created"
        )
        .await,
        2
    );
    assert_eq!(
        count(
            TimeRange {
                filter: channels(&["telegram", "messenger"]),
                ..utc()
            },
            "contact_created"
        )
        .await,
        3
    );
}

#[tokio::test]
async fn from_is_clamped_to_2020() {
    let db = seeded().await;
    db.write(|tx| {
        event(
            tx,
            OTHER_WS,
            1,
            "contact_created",
            "2019-06-01T00:00:00Z",
            None,
            None,
            None,
            None,
        );
        event(
            tx,
            OTHER_WS,
            2,
            "contact_created",
            "2020-01-01T00:00:00Z",
            None,
            None,
            None,
            None,
        );
        event(
            tx,
            OTHER_WS,
            3,
            "contact_created",
            "2020-02-01T00:00:00Z",
            None,
            None,
            None,
            None,
        );
        Ok(())
    })
    .await
    .unwrap();
    let r = TimeRange::new(
        OTHER_WS,
        Some(ms("2019-01-01T00:00:00Z")),
        Some(ms("2020-12-31T00:00:00Z")),
        "UTC",
        Filter::default(),
    );
    assert_eq!(
        read(&db, move |c| contacts::distinct_contacts_count(
            c,
            &r,
            "contact_created"
        ))
        .await,
        2
    );
}

#[tokio::test]
async fn contacts_count_filters_by_inbox_channel_and_inbox_id() {
    let db = seeded().await;
    let total = |f: Filter| {
        let db = Arc::clone(&db);
        async move { read(&db, move |c| contacts::contacts_count(c, WS, &f)).await }
    };
    assert_eq!(total(Filter::default()).await, 3);
    assert_eq!(total(channels(&["telegram"])).await, 1);
    assert_eq!(total(inboxes(&[I1])).await, 2);
}

fn rows(list: &[(&str, i64, i64)]) -> Vec<DimensionCount> {
    list.iter()
        .map(|(d, c, u)| DimensionCount {
            dimension: d.to_string(),
            count: *c,
            unique_contacts: *u,
        })
        .collect()
}

async fn by(
    db: &Arc<Db>,
    r: TimeRange,
    dimension: &'static str,
    metric: &'static str,
) -> Vec<DimensionCount> {
    read(db, move |c| {
        contacts::contacts_by_dimension(c, &r, dimension, metric)
    })
    .await
}

#[tokio::test]
async fn new_contacts_by_each_dimension() {
    let db = seeded().await;
    assert_eq!(
        by(&db, utc(), "channel", "new").await,
        rows(&[("messenger", 3, 2), ("telegram", 1, 1)])
    );
    assert_eq!(
        by(&db, utc(), "country", "new").await,
        rows(&[("VN", 3, 2), ("US", 1, 1)])
    );
    assert_eq!(
        by(&db, utc(), "source", "new").await,
        rows(&[("direct", 3, 2), ("ads", 1, 1)])
    );
    assert_eq!(
        by(&db, utc(), "language", "new").await,
        rows(&[("vi", 3, 2), ("en", 1, 1)])
    );
    let r = TimeRange {
        filter: channels(&["telegram"]),
        ..utc()
    };
    assert_eq!(by(&db, r, "language", "new").await, rows(&[("en", 1, 1)]));
}

#[tokio::test]
async fn total_counts_the_balance_as_of_to_and_drops_empty_groups() {
    let db = seeded().await;
    assert_eq!(
        by(&db, utc(), "channel", "total").await,
        rows(&[("messenger", 4, 4)])
    );
    assert_eq!(
        by(&db, utc(), "source", "total").await,
        rows(&[("direct", 3, 3), ("ads", 1, 1)])
    );
    // `to` bounds the balance; `from` is ignored.
    let early = range(
        "2025-03-04T00:00:00Z",
        "2025-03-04T23:59:59Z",
        "UTC",
        Filter::default(),
    );
    assert_eq!(
        by(&db, early, "channel", "total").await,
        rows(&[("messenger", 4, 4), ("telegram", 1, 1)])
    );
}

#[tokio::test]
async fn total_keeps_the_unknown_group() {
    let db = seeded().await;
    db.write(|tx| {
        event(
            tx,
            WS,
            104,
            "contact_created",
            "2025-03-01T00:00:00Z",
            Some(""),
            None,
            None,
            None,
        );
        event(
            tx,
            WS,
            105,
            "contact_created",
            "2025-03-01T00:00:00Z",
            None,
            None,
            None,
            None,
        );
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        by(&db, utc(), "source", "total").await,
        rows(&[("direct", 3, 3), ("", 2, 2), ("ads", 1, 1)])
    );
}

#[tokio::test]
async fn total_returns_an_empty_list_when_nothing_matches() {
    let db = seeded().await;
    let r = TimeRange {
        workspace_id: 999,
        ..utc()
    };
    assert!(by(&db, r, "channel", "total").await.is_empty());
}

#[tokio::test]
async fn total_by_channel_under_an_inbox_filter_reads_per_inbox_counts_minus_events_after_to() {
    let db = seeded().await;
    let r = TimeRange {
        filter: inboxes(&[I1]),
        ..utc()
    };
    assert_eq!(
        by(&db, r, "channel", "total").await,
        rows(&[("messenger", 2, 2)])
    );
    // I2 holds 1 contact; its deletion is after `to` = 03-04: 1 - (-1) = 2.
    let r = range(
        "2025-03-01T00:00:00Z",
        "2025-03-04T23:59:59Z",
        "UTC",
        inboxes(&[I2]),
    );
    assert_eq!(
        by(&db, r, "channel", "total").await,
        rows(&[("telegram", 2, 2)])
    );
}

#[tokio::test]
async fn total_by_channel_without_an_inbox_filter_keeps_the_event_balance() {
    let db = seeded().await;
    let r = TimeRange {
        filter: channels(&["messenger"]),
        ..utc()
    };
    assert_eq!(
        by(&db, r, "channel", "total").await,
        rows(&[("messenger", 4, 4)])
    );
}

#[test]
fn the_metric_total_is_only_supported_for_channel_and_source() {
    let none = Filter::default();
    assert!(contacts::is_supported_dimension_metric(
        "country", "new", &none
    ));
    assert!(contacts::is_supported_dimension_metric(
        "channel", "total", &none
    ));
    assert!(contacts::is_supported_dimension_metric(
        "source", "total", &none
    ));
    assert!(!contacts::is_supported_dimension_metric(
        "country", "total", &none
    ));
    assert!(!contacts::is_supported_dimension_metric(
        "language", "total", &none
    ));
    let inbox = inboxes(&[7]);
    assert!(contacts::is_supported_dimension_metric(
        "channel", "total", &inbox
    ));
    assert!(!contacts::is_supported_dimension_metric(
        "source", "total", &inbox
    ));
    assert!(contacts::is_supported_dimension_metric(
        "language", "new", &inbox
    ));
}

#[test]
fn filters_are_deduped_and_sorted() {
    let filter = Filter::new(
        vec!["zalo".into(), "messenger".into(), "zalo".into()],
        vec![9, 3, 9],
    );
    assert_eq!(filter.channels, ["messenger", "zalo"]);
    assert_eq!(filter.inbox_ids, [3, 9]);
    assert!(filter.is_set());
    assert!(!Filter::new(Vec::new(), Vec::new()).is_set());
    assert_eq!(Filter::new(Vec::new(), Vec::new()), Filter::default());
}

#[test]
fn a_range_with_from_after_to_has_no_buckets() {
    let r = range(
        "2025-03-05T00:00:00Z",
        "2025-03-01T00:00:00Z",
        "UTC",
        Filter::default(),
    );
    assert!(Granularity::Day.series(&r).is_empty());
    let r = range(
        "2025-05-05T00:00:00Z",
        "2025-03-01T00:00:00Z",
        "UTC",
        Filter::default(),
    );
    assert!(Granularity::Month.series(&r).is_empty());
    let invalid = TimeRange::new(WS, None, Some(1), "UTC", Filter::default());
    assert!(Granularity::Day.series(&invalid).is_empty());
}

#[test]
fn an_unknown_or_legacy_timezone_resolves() {
    let r = TimeRange::new(WS, Some(0), Some(0), "Asia/Saigon", Filter::default());
    assert_eq!(r.tz.name(), "Asia/Saigon");
    let r = TimeRange::new(WS, Some(0), Some(0), "Mars/Base", Filter::default());
    assert_eq!(r.tz.name(), "UTC");
}

#[tokio::test]
async fn active_contacts_count_distinct_contacts_with_activity_hours() {
    let db = seeded().await;
    db.write(|tx| {
        for (contact, contact_inbox, inbox, hour) in [
            (1, 1, I1, "2025-03-02T10:00:00Z"),
            (1, 1, I1, "2025-03-02T11:00:00Z"),
            (2, 3, I2, "2025-03-03T00:00:00Z"),
            (1, 2, I2, "2025-03-04T00:00:00Z"),
            (3, 4, I1, "2025-02-15T00:00:00Z"),
        ] {
            tx.execute(
                r#"INSERT INTO "ContactActiveHourly" ("workspaceId", "contactId", "contactInboxId", "inboxId", "hourBucket") VALUES (?1, ?2, ?3, ?4, ?5)"#,
                params![WS, contact, contact_inbox, inbox, ms(hour)],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let active = |r: TimeRange| {
        let db = Arc::clone(&db);
        async move { read(&db, move |c| contacts::active_contacts_count(c, &r)).await }
    };
    assert_eq!(active(utc()).await, 2);
    assert_eq!(
        active(TimeRange {
            filter: inboxes(&[I1]),
            ..utc()
        })
        .await,
        1
    );
    assert_eq!(
        active(TimeRange {
            filter: channels(&["telegram"]),
            ..utc()
        })
        .await,
        2
    );
    // Bounds are inclusive hour buckets.
    assert_eq!(
        active(range(
            "2025-03-02T11:00:00Z",
            "2025-03-03T00:00:00Z",
            "UTC",
            Filter::default()
        ))
        .await,
        2
    );
}

#[tokio::test]
async fn current_mac_count_reads_the_period_holding_now() {
    let db = seeded().await;
    let now = ms("2025-06-15T00:00:00Z");
    assert_eq!(
        read(&db, move |c| contacts::current_mac_count(c, WS, now)).await,
        0
    );
    db.write(move |tx| {
        let day = 86_400_000;
        tx.execute(
            r#"INSERT INTO "WorkspaceMac" ("id", "workspaceId", "ownerId", "periodStart", "periodEnd", "macCount") VALUES
               (1, ?1, 1, ?2, ?3, 99), (2, ?1, 1, ?3, ?4, 7)"#,
            params![WS, now - 40 * day, now - 10 * day, now + 20 * day],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        read(&db, move |c| contacts::current_mac_count(c, WS, now)).await,
        7
    );
}
