//! Port of `packages/database/__tests__/inbox-repository.test.ts`, schema case only
//! (P2.3 spec section 5.1, row 8.1). The repository cases of the file belong to P3.
//! Class C: runs against a migrated SQLite file.

mod common;

use chatbotx_db::rusqlite::params;
use common::{column, fresh_db, seed};

/// Port of: packages/database/__tests__/inbox-repository.test.ts:29 "Inbox schema > defaults outbound read marking to disabled"
///
/// `default: false, hasDefault: true, notNull: true` -> `notnull = 1`, `dflt_value = '0'`, and an
/// Inbox inserted without the column reads back `0`.
#[tokio::test]
async fn defaults_outbound_read_marking_to_disabled() {
    let db = fresh_db().await;
    let col = column(&db, "Inbox", "markReadOnOutbound").await;
    assert!(col.notnull);
    assert_eq!(col.dflt_value.as_deref(), Some("0"));

    let s = seed(&db).await;
    let value: i64 = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "markReadOnOutbound" FROM "Inbox" WHERE "id" = ?1"#,
                params![s.inbox_id],
                |row| row.get(0),
            )?)
        })
        .await
        .expect("read the seeded Inbox");
    assert_eq!(value, 0);
}
