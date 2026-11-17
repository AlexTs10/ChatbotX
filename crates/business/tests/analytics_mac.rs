//! `macTrackingService.claimNewActiveContact` (Q.D11 slice 8c), the cases of
//! `packages/analytics/__tests__/mac-tracking.service.test.ts` against real
//! rows instead of repository mocks.

mod common;

use chatbotx_business::analytics::mac;
use common::{WS, setup, two_workspaces};

const PERIOD_START: &str = "2026-01-10T09:00:00.000Z";

fn ms(iso: &str) -> i64 {
    chatbotx_core::js::parse_date(iso).expect("iso")
}

async fn claim(db: &chatbotx_db::Db, ws: i64, contact: (i64, i64), at: &'static str) -> bool {
    db.write(move |tx| {
        mac::claim_new_active_contact(tx, ws, contact, 11, ms(PERIOD_START), ms(at))
            .map_err(|e| chatbotx_db::DbError::other(e.to_string()))
    })
    .await
    .unwrap()
}

async fn count(db: &chatbotx_db::Db, sql: &'static str) -> i64 {
    common::scalar_i64(db, sql, WS).await.unwrap_or(0)
}

#[tokio::test]
async fn claims_a_new_contact_once_per_period_and_records_each_hour() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        common::inbox(tx, 11, WS, "messenger")?;
        common::contact(tx, 101, WS)?;
        common::contact_inbox(tx, 201, 101, 11, "messenger", "psid-1")?;
        Ok(())
    })
    .await;
    // Records presence and reports counted for a brand-new contact.
    assert!(claim(&db, WS, (101, 201), "2026-05-01T10:05:00.000Z").await);
    assert_eq!(
        count(
            &db,
            r#"SELECT "macCount" FROM "WorkspaceMac" WHERE "workspaceId" = ?1"#
        )
        .await,
        1
    );
    assert_eq!(
        common::scalar_i64(
            &db,
            r#"SELECT "periodStart" FROM "WorkspaceMac" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(ms("2026-04-10T09:00:00.000Z"))
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "ContactActiveMonthly" WHERE "workspaceId" = ?1"#
        )
        .await,
        1
    );
    // The paired hourly row, at the start of the UTC hour.
    assert_eq!(
        common::scalar_i64(
            &db,
            r#"SELECT "hourBucket" FROM "ContactActiveHourly" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(ms("2026-05-01T10:00:00.000Z"))
    );
    // Already active this period: not counted, the hourly row still lands.
    assert!(!claim(&db, WS, (101, 201), "2026-05-01T12:00:00.000Z").await);
    assert_eq!(
        count(
            &db,
            r#"SELECT "macCount" FROM "WorkspaceMac" WHERE "workspaceId" = ?1"#
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "ContactActiveHourly" WHERE "workspaceId" = ?1"#
        )
        .await,
        2
    );
}

#[tokio::test]
async fn an_unknown_workspace_has_no_mac_row_and_counts_nothing() {
    let db = two_workspaces().await;
    assert!(!claim(&db, 9_999, (101, 201), "2026-05-01T10:05:00.000Z").await);
    for sql in [
        r#"SELECT COUNT(*) FROM "WorkspaceMac" WHERE "workspaceId" <> ?1"#,
        r#"SELECT COUNT(*) FROM "ContactActiveMonthly" WHERE "workspaceId" <> ?1"#,
        r#"SELECT COUNT(*) FROM "ContactActiveHourly" WHERE "workspaceId" <> ?1"#,
    ] {
        assert_eq!(count(&db, sql).await, 0, "{sql}");
    }
}
