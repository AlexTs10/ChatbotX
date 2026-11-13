//! Port of `packages/database/__tests__/integration/google-ads-conversion-event-stats.test.ts`
//! (P3.2b). The stats reads on a migrated temp file: date bucketing (DST zones), the FILTER
//! counts, the latest-snapshot aggregates and the decimal sum. The TS rolled back one
//! transaction per case; here each case has its own database file. `seedWorkspace` is
//! `WS` / `OTHER_WS` (and a third workspace where a case needs one).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo11.rs"]
mod repo11;

use chatbotx_db::repo::google_ads_conversion_event::{self as gace, ListInput, StatsFilters};
use repo11::{OTHER_WS, WS, i, ms, null, read, stats_event, t, two_ws_db};

fn window(workspace_id: i64) -> StatsFilters {
    StatsFilters {
        workspace_id,
        since: ms("2026-10-01T00:00:00Z"),
        until: ms("2026-10-07T23:59:59.999Z"),
        ..StatsFilters::default()
    }
}

#[tokio::test]
async fn google_ads_conversion_event_stats_the_date_is_the_first_selected_column_and_rows_are_grouped_per_day_and_channel()
 {
    let db = two_ws_db().await;
    stats_event(&db, WS, vec![]).await;
    stats_event(&db, WS, vec![("channel", t("messenger"))]).await;
    stats_event(&db, WS, vec![("occurredAt", i(ms("2026-10-03T12:00:00Z")))]).await;
    let rows = read(&db, |c| {
        gace::stats_by_day_and_channel(c, &window(WS), "UTC")
    })
    .await;
    // The Rust row struct declares `date` first.
    let mut keys: Vec<String> = rows
        .iter()
        .map(|r| format!("{}/{}/{}", r.date, r.channel, r.counts.processed))
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "2026-10-02/messenger/1",
            "2026-10-02/whatsapp/1",
            "2026-10-03/whatsapp/1",
        ]
    );
}

#[tokio::test]
async fn google_ads_conversion_event_stats_a_back_dated_event_outside_the_window_is_excluded_and_the_bounds_are_inclusive()
 {
    let db = two_ws_db().await;
    let w = window(WS);
    stats_event(
        &db,
        WS,
        vec![("occurredAt", i(ms("2026-09-30T23:59:59.999Z")))],
    )
    .await;
    stats_event(&db, WS, vec![("occurredAt", i(w.since))]).await;
    stats_event(&db, WS, vec![("occurredAt", i(w.until))]).await;
    stats_event(&db, WS, vec![("occurredAt", i(ms("2026-10-08T00:00:00Z")))]).await;
    let rows = read(&db, move |c| gace::stats_by_day_and_channel(c, &w, "UTC")).await;
    assert_eq!(rows.iter().map(|r| r.counts.processed).sum::<i64>(), 2);
    let mut dates: Vec<&str> = rows.iter().map(|r| r.date.as_str()).collect();
    dates.sort_unstable();
    assert_eq!(dates, vec!["2026-10-01", "2026-10-07"]);
}

#[tokio::test]
async fn google_ads_conversion_event_stats_counts_every_status_and_each_failure_stage_and_the_stages_add_up_to_failed()
 {
    let db = two_ws_db().await;
    for stage in [
        Some("delivery"),
        Some("delivery"),
        Some("processing"),
        Some("timeout"),
        None,
    ] {
        stats_event(
            &db,
            WS,
            vec![
                ("status", t("failed")),
                ("failureStage", stage.map_or_else(null, t)),
            ],
        )
        .await;
    }
    stats_event(&db, WS, vec![("status", t("pending"))]).await;
    stats_event(
        &db,
        WS,
        vec![
            ("status", t("sending")),
            ("claimToken", t("token")),
            ("claimedAt", i(chatbotx_db::repo::now_ms())),
        ],
    )
    .await;
    stats_event(&db, WS, vec![("status", t("sent"))]).await;
    stats_event(&db, WS, vec![("status", t("skipped_no_account"))]).await;
    stats_event(&db, WS, vec![("status", t("skipped_expired"))]).await;
    let rows = read(&db, |c| {
        gace::stats_by_day_and_channel(c, &window(WS), "UTC")
    })
    .await;
    let row = &rows[0];
    assert_eq!(
        (
            row.counts.pending,
            row.counts.sending,
            row.counts.sent,
            row.counts.processed,
            row.counts.failed,
            row.counts.skipped_no_account,
            row.counts.skipped_expired,
        ),
        (1, 1, 1, 0, 5, 1, 1)
    );
    assert_eq!(
        (
            row.failed_delivery,
            row.failed_processing,
            row.failed_timeout,
            row.failed_unknown,
        ),
        (2, 1, 1, 1)
    );
    assert_eq!(
        row.failed_delivery + row.failed_processing + row.failed_timeout + row.failed_unknown,
        row.counts.failed
    );
}

/// The dates of `statsByDayAndChannel(window(WS), tz)`.
async fn dates(db: &chatbotx_db::Db, tz: &'static str) -> Vec<String> {
    read(db, move |c| {
        gace::stats_by_day_and_channel(c, &window(WS), tz)
    })
    .await
    .into_iter()
    .map(|r| r.date)
    .collect()
}

#[tokio::test]
async fn google_ads_conversion_event_stats_a_day_boundary_follows_the_timezone_including_ho_chi_minh_and_a_dst_zone()
 {
    let db = two_ws_db().await;
    // 17:30Z on the 2nd is already the 3rd at UTC+7.
    stats_event(&db, WS, vec![("occurredAt", i(ms("2026-10-02T17:30:00Z")))]).await;
    assert_eq!(dates(&db, "UTC").await, vec!["2026-10-02"]);
    assert_eq!(dates(&db, "Asia/Ho_Chi_Minh").await, vec!["2026-10-03"]);
    // The legacy spelling a browser can still report.
    assert_eq!(dates(&db, "Asia/Saigon").await, vec!["2026-10-03"]);
}

#[tokio::test]
async fn google_ads_conversion_event_stats_the_dst_day_in_new_york_is_bucketed_by_local_midnight() {
    let db = two_ws_db().await;
    // Spring forward 2026-03-08: EST until 07:00Z, EDT after.
    for instant in [
        "2026-03-08T04:59:59Z", // 23:59:59 EST on the 7th
        "2026-03-08T05:00:00Z", // 00:00:00 EST on the 8th
        "2026-03-09T03:59:59Z", // 23:59:59 EDT on the 8th (a 23-hour day)
        "2026-03-09T04:00:00Z", // 00:00:00 EDT on the 9th
    ] {
        stats_event(&db, WS, vec![("occurredAt", i(ms(instant)))]).await;
    }
    let filters = StatsFilters {
        workspace_id: WS,
        since: ms("2026-03-01T00:00:00Z"),
        until: ms("2026-03-20T00:00:00Z"),
        ..StatsFilters::default()
    };
    let rows = read(&db, move |c| {
        gace::stats_by_day_and_channel(c, &filters, "America/New_York")
    })
    .await;
    let mut days: Vec<(String, i64)> = rows
        .into_iter()
        .map(|r| (r.date, r.counts.processed))
        .collect();
    days.sort();
    assert_eq!(
        days,
        vec![
            ("2026-03-07".to_owned(), 1),
            ("2026-03-08".to_owned(), 2),
            ("2026-03-09".to_owned(), 1),
        ]
    );
}

#[tokio::test]
async fn google_ads_conversion_event_stats_another_workspace_s_events_are_never_counted() {
    let db = two_ws_db().await;
    stats_event(&db, WS, vec![("value", t("10")), ("currency", t("USD"))]).await;
    stats_event(
        &db,
        OTHER_WS,
        vec![("value", t("99")), ("currency", t("USD"))],
    )
    .await;
    stats_event(&db, OTHER_WS, vec![("conversionActionId", t("222"))]).await;
    let days = read(&db, |c| {
        gace::stats_by_day_and_channel(c, &window(WS), "UTC")
    })
    .await;
    let actions = read(&db, |c| gace::stats_by_action(c, &window(WS), 50)).await;
    let values = read(&db, |c| gace::confirmed_value_by_currency(c, &window(WS))).await;
    assert_eq!(days.iter().map(|r| r.counts.processed).sum::<i64>(), 1);
    assert_eq!(
        actions
            .iter()
            .map(|r| r.conversion_action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["111"]
    );
    assert_eq!(
        values,
        vec![gace::ConfirmedValueRow {
            currency: Some("USD".to_owned()),
            value: "10".to_owned(),
            count: 1,
        }]
    );
}

#[tokio::test]
async fn google_ads_conversion_event_stats_a_renamed_conversion_action_shows_its_latest_name_and_category()
 {
    let db = two_ws_db().await;
    stats_event(
        &db,
        WS,
        vec![
            ("conversionActionName", t("Old name")),
            ("conversionActionCategory", t("PURCHASE")),
            ("occurredAt", i(ms("2026-10-02T10:00:00Z"))),
        ],
    )
    .await;
    stats_event(
        &db,
        WS,
        vec![
            ("conversionActionName", t("New name")),
            ("conversionActionCategory", t("QUALIFIED_LEAD")),
            ("occurredAt", i(ms("2026-10-04T10:00:00Z"))),
        ],
    )
    .await;
    let rows = read(&db, |c| gace::stats_by_action(c, &window(WS), 50)).await;
    let row = &rows[0];
    assert_eq!(row.conversion_action_id, "111");
    assert_eq!(row.name.as_deref(), Some("New name"));
    assert_eq!(row.category.as_deref(), Some("QUALIFIED_LEAD"));
    assert_eq!(row.counts.processed, 2);
}

#[tokio::test]
async fn google_ads_conversion_event_stats_actions_are_ordered_by_volume_then_id_and_limit_1_rows_are_returned()
 {
    let db = two_ws_db().await;
    for action in ["300", "200", "100", "300"] {
        stats_event(&db, WS, vec![("conversionActionId", t(action))]).await;
    }
    let rows = read(&db, |c| gace::stats_by_action(c, &window(WS), 2)).await;
    assert_eq!(
        rows.iter()
            .map(|r| r.conversion_action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["300", "100", "200"]
    );
}

#[tokio::test]
async fn google_ads_conversion_event_stats_channel_and_conversion_action_filters_narrow_every_read()
{
    let db = two_ws_db().await;
    stats_event(&db, WS, vec![]).await;
    stats_event(&db, WS, vec![("channel", t("messenger"))]).await;
    stats_event(&db, WS, vec![("conversionActionId", t("222"))]).await;
    let filters = StatsFilters {
        channel: Some("whatsapp".to_owned()),
        conversion_action_id: Some("111".to_owned()),
        ..window(WS)
    };
    let rows = read(&db, move |c| {
        gace::stats_by_day_and_channel(c, &filters, "UTC")
    })
    .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].channel.as_str(), rows[0].counts.processed),
        ("whatsapp", 1)
    );
}

#[tokio::test]
async fn google_ads_conversion_event_stats_confirmed_value_is_an_exact_decimal_sum_per_currency_over_processed_events_only()
 {
    let db = two_ws_db().await;
    stats_event(&db, WS, vec![("value", t("0.1")), ("currency", t("USD"))]).await;
    stats_event(&db, WS, vec![("value", t("0.2")), ("currency", t("USD"))]).await;
    stats_event(
        &db,
        WS,
        vec![("value", t("1000.505")), ("currency", t("VND"))],
    )
    .await;
    // Not confirmed, or no value: neither adds anything.
    stats_event(
        &db,
        WS,
        vec![
            ("status", t("failed")),
            ("value", t("500")),
            ("currency", t("USD")),
        ],
    )
    .await;
    stats_event(
        &db,
        WS,
        vec![
            ("status", t("sent")),
            ("value", t("7")),
            ("currency", t("USD")),
        ],
    )
    .await;
    stats_event(&db, WS, vec![]).await;
    let rows = read(&db, |c| gace::confirmed_value_by_currency(c, &window(WS))).await;
    let row = |currency: &str, value: &str, count: i64| gace::ConfirmedValueRow {
        currency: Some(currency.to_owned()),
        value: value.to_owned(),
        count,
    };
    assert_eq!(rows, vec![row("USD", "0.3", 2), row("VND", "1000.505", 1)]);
}

#[tokio::test]
async fn google_ads_conversion_event_stats_exists_for_workspace_is_true_only_for_a_workspace_with_events_whatever_their_date()
 {
    let db = two_ws_db().await;
    stats_event(&db, WS, vec![("occurredAt", i(ms("2020-01-01T00:00:00Z")))]).await;
    assert!(read(&db, |c| gace::exists_for_workspace(c, WS)).await);
    assert!(!read(&db, |c| gace::exists_for_workspace(c, OTHER_WS)).await);
}

#[tokio::test]
async fn google_ads_conversion_event_stats_list_by_workspace_filters_by_conversion_action() {
    let db = two_ws_db().await;
    stats_event(&db, WS, vec![]).await;
    stats_event(&db, WS, vec![("conversionActionId", t("222"))]).await;
    let input = ListInput {
        workspace_id: WS,
        conversion_action_id: Some("222".to_owned()),
        page: 1,
        per_page: 10,
        ..ListInput::default()
    };
    let result = read(&db, move |c| gace::list_by_workspace(c, &input)).await;
    assert_eq!(result.total, 1);
    assert_eq!(result.rows[0].conversion_action_id, "222");
}
