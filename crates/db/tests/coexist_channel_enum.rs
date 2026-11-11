//! Port of `packages/database/__tests__/coexist-channel-enum.test.ts`
//! (P2.4 spec section 3.1, row 6.1). Class C: runs against a migrated SQLite file.

mod common;

use common::{column_in_list, fresh_db, in_list_values, table_sql};

/// `COEXIST_CHANNELS` of `@chatbotx.io/utils/channel`, same order.
// TODO(P2.4 test port): use the Rust `COEXIST_CHANNELS` of the core crate once it exists.
const COEXIST_CHANNELS: &[&str] = &["whatsapp", "messenger", "instagram"];

/// Port of: packages/database/__tests__/coexist-channel-enum.test.ts:6 "coexistChannel pgEnum > enumValues matches COEXIST_CHANNELS exactly (same order, no drift)"
///
/// Adapted: the pgEnum is the CHECK `"channel" IN (...)` list of `CoexistSyncRun`.
#[tokio::test]
async fn enum_values_matches_coexist_channels_exactly_same_order_no_drift() {
    let db = fresh_db().await;
    let sql = table_sql(&db, "CoexistSyncRun")
        .await
        .expect("table CoexistSyncRun");
    let list = column_in_list(&sql, "channel").expect("CHECK (\"channel\" IN (...))");
    assert_eq!(in_list_values(&list), COEXIST_CHANNELS);
}
