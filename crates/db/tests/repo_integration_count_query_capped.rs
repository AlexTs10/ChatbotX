//! Port of `packages/database/__tests__/integration/count-query-capped.test.ts` (P3.2).
//! Runs against a migrated SQLite temp file. `generate_series` does not exist in SQLite:
//! the row source is a recursive CTE with the same row count (adapted).

mod common;

use chatbotx_db::repo::{self, CappedCount};
use common::fresh_db;

const CAP: i64 = 5;

/// `SELECT 1 FROM generate_series(1, n)` as a recursive CTE.
fn rows_sql(count: i64) -> String {
    format!(
        "WITH RECURSIVE s(x) AS (SELECT 1 WHERE 1 <= {count} UNION ALL SELECT x + 1 FROM s WHERE x < {count}) SELECT 1 FROM s"
    )
}

async fn capped(count: i64) -> CappedCount {
    let db = fresh_db().await;
    let sql = rows_sql(count);
    repo::read(&db, move |conn| {
        repo::count_query_capped(conn, CAP, &sql, &[])
    })
    .await
    .expect("count")
}

/// Port of: packages/database/__tests__/integration/count-query-capped.test.ts:36 "countQueryCapped (real SQL) > $rows matching rows -> total $total, capped $capped" (rows 0)
#[tokio::test]
async fn count_query_capped_real_sql_0_matching_rows_total_0_capped_false() {
    assert_eq!(
        capped(0).await,
        CappedCount {
            total: 0,
            capped: false,
            cap: CAP
        }
    );
}

/// Port of: packages/database/__tests__/integration/count-query-capped.test.ts:36 "countQueryCapped (real SQL) > $rows matching rows -> total $total, capped $capped" (rows 4)
#[tokio::test]
async fn count_query_capped_real_sql_4_matching_rows_total_4_capped_false() {
    assert_eq!(
        capped(CAP - 1).await,
        CappedCount {
            total: CAP - 1,
            capped: false,
            cap: CAP
        }
    );
}

/// Port of: packages/database/__tests__/integration/count-query-capped.test.ts:36 "countQueryCapped (real SQL) > $rows matching rows -> total $total, capped $capped" (rows 5)
#[tokio::test]
async fn count_query_capped_real_sql_5_matching_rows_total_5_capped_false() {
    assert_eq!(
        capped(CAP).await,
        CappedCount {
            total: CAP,
            capped: false,
            cap: CAP
        }
    );
}

/// Port of: packages/database/__tests__/integration/count-query-capped.test.ts:36 "countQueryCapped (real SQL) > $rows matching rows -> total $total, capped $capped" (rows 6)
#[tokio::test]
async fn count_query_capped_real_sql_6_matching_rows_total_5_capped_true() {
    assert_eq!(
        capped(CAP + 1).await,
        CappedCount {
            total: CAP,
            capped: true,
            cap: CAP
        }
    );
}

/// Port of: packages/database/__tests__/integration/count-query-capped.test.ts:36 "countQueryCapped (real SQL) > $rows matching rows -> total $total, capped $capped" (rows 50)
#[tokio::test]
async fn count_query_capped_real_sql_50_matching_rows_total_5_capped_true() {
    assert_eq!(
        capped(CAP * 10).await,
        CappedCount {
            total: CAP,
            capped: true,
            cap: CAP
        }
    );
}

/// Port of: packages/database/__tests__/integration/count-query-capped.test.ts:46 "countQueryCapped (real SQL) > accepts a joined select such as the broadcast DM-join branch"
#[tokio::test]
async fn count_query_capped_real_sql_accepts_a_joined_select_such_as_the_broadcast_dm_join_branch()
{
    let db = fresh_db().await;
    let result = repo::read(&db, |conn| {
        repo::count_query_capped(
            conn,
            CAP,
            "WITH RECURSIVE s(id) AS (SELECT 1 UNION ALL SELECT id + 1 FROM s WHERE id < 8) \
             SELECT 1 FROM s AS ci INNER JOIN s AS conv ON conv.id = ci.id WHERE ci.id % 2 = 0",
            &[],
        )
    })
    .await
    .expect("count");
    assert_eq!(
        result,
        CappedCount {
            total: 4,
            capped: false,
            cap: CAP
        }
    );
}
