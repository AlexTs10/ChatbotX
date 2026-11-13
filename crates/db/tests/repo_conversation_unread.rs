//! Port of `packages/database/__tests__/conversation-unread.test.ts` (P3.2b). The TS checks
//! the shape of the Drizzle relational filter; here the SQL predicate runs on real
//! `Conversation` rows of a migrated temp file (adapted).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::queries::conversation_unread::conversation_unread_where;
use chatbotx_db::repo::{self};
use common::{fresh_db, seed};
use repo_common::{insert_contact, insert_conv, ok, setup};

const WS: i64 = 2001;

#[tokio::test]
async fn conversation_unread_where_matches_the_canonical_unread_semantics_for_never_read_and_newer_activity_conversations()
 {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, |tx| {
        insert_contact(tx, 10, WS)?;
        // (id, agentLastReadAt, lastActivityAt)
        for (id, read, activity) in [
            (1, None, None),
            (2, None, Some(100)),
            (3, Some(100), Some(200)),
            (4, Some(200), Some(200)),
            (5, Some(300), Some(200)),
            (6, Some(300), None),
        ] {
            insert_conv(tx, id, WS, 10, Some(&format!("s{id}")))?;
            tx.execute(
                "UPDATE \"Conversation\" SET \"agentLastReadAt\" = ?1, \"lastActivityAt\" = ?2 WHERE \"id\" = ?3",
                (read, activity, id),
            )?;
        }
        Ok(())
    })
    .await;
    let predicate = conversation_unread_where("c");
    // A pure column comparison: no bind values.
    assert!(!predicate.contains('?'), "{predicate}");
    let unread: Vec<i64> = ok(repo::read(&db, move |conn| {
        chatbotx_db::repo::sql::Sql::raw(format!(
            "SELECT c.\"id\" FROM \"Conversation\" c WHERE {predicate} ORDER BY c.\"id\""
        ))
        .query_map(conn, |row| row.get(0))
    })
    .await);
    assert_eq!(unread, vec![2, 3]);
}
