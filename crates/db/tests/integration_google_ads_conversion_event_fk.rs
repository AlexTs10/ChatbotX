//! Port of `packages/database/__tests__/integration/google-ads-conversion-event-fk.test.ts`
//! (P2.5 spec section 3.1, rows 8.1-8.2). Class C: runs against a migrated SQLite file.
//!
//! Same shape as the original `withRolledBackTransaction`: one write job seeds the rows
//! (User, Workspace, Inbox, Contact, ContactInbox, Integration `googleAds`,
//! IntegrationGoogleAds, one event with the TS values), deletes the parent, reads the event
//! and rolls everything back (a SAVEPOINT that is rolled back before the job ends).

mod common;

use chatbotx_db::DbError;
use chatbotx_db::rusqlite::{self, params};
use common::{
    ColumnOverride, TestDb, fresh_db, insert_google_ads_event, insert_integration_google_ads,
    query_i64, seed_contact_inbox, seed_integration,
};

const WORKSPACE_ID: i64 = 2001;
const EVENT_ID: i64 = 9001;

/// Ids of the TS `seedLinkedEvent` result.
#[derive(Debug, Clone, Copy)]
struct LinkedEvent {
    event_id: i64,
    contact_inbox_id: i64,
    satellite_id: i64,
}

/// The TS `seedLinkedEvent`: a workspace with a connected satellite, a contact inbox and one
/// linked event.
fn seed_linked_event(tx: &rusqlite::Connection) -> Result<LinkedEvent, DbError> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (1001, 'google-ads-fk@example.test')"#,
        [],
    )?;
    tx.execute(
        r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'google-ads-fk', 1001)"#,
        params![WORKSPACE_ID],
    )?;
    let contact = seed_contact_inbox(tx, WORKSPACE_ID, "instagram", 3001)?;
    let integration_id = seed_integration(tx, 5001, WORKSPACE_ID, "googleAds")?;
    let satellite_id =
        insert_integration_google_ads(tx, 6001, WORKSPACE_ID, integration_id, "1234567890")?;
    use rusqlite::types::Value;
    // The TS `seedLinkedEvent` row: `channel` instagram, the two links, and no `options`.
    let overrides: [ColumnOverride; 4] = [
        ("integrationGoogleAdsId", Some(Value::Integer(satellite_id))),
        (
            "contactInboxId",
            Some(Value::Integer(contact.contact_inbox_id)),
        ),
        ("channel", Some(Value::Text("instagram".to_owned()))),
        ("options", None),
    ];
    insert_google_ads_event(tx, EVENT_ID, WORKSPACE_ID, "gads-fk-1", &overrides)?;
    Ok(LinkedEvent {
        event_id: EVENT_ID,
        contact_inbox_id: contact.contact_inbox_id,
        satellite_id,
    })
}

/// The TS `readEvent`: `(contactInboxId, integrationGoogleAdsId)` of the event, if it exists.
fn read_event(
    tx: &rusqlite::Connection,
    event_id: i64,
) -> Result<Option<(Option<i64>, Option<i64>)>, DbError> {
    let mut stmt = tx.prepare(
        r#"SELECT "contactInboxId", "integrationGoogleAdsId" FROM "GoogleAdsConversionEvent" WHERE "id" = ?1"#,
    )?;
    let mut rows = stmt.query(params![event_id])?;
    match rows.next()? {
        Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
        None => Ok(None),
    }
}

/// Seeds, runs `delete`, reads the event, and rolls back. Returns the seed ids and the row.
async fn with_rolled_back_transaction(
    db: &TestDb,
    delete: fn(&rusqlite::Connection, LinkedEvent) -> Result<(), DbError>,
) -> (LinkedEvent, Option<(Option<i64>, Option<i64>)>) {
    let result = db
        .write(move |tx| {
            let conn: &rusqlite::Connection = tx;
            conn.execute_batch("SAVEPOINT rolled_back")?;
            let outcome = (|| {
                let seeded = seed_linked_event(conn)?;
                delete(conn, seeded)?;
                let row = read_event(conn, seeded.event_id)?;
                Ok::<_, DbError>((seeded, row))
            })();
            conn.execute_batch("ROLLBACK TO rolled_back; RELEASE rolled_back")?;
            outcome
        })
        .await
        .expect("seed, delete and read");
    assert_eq!(
        query_i64(
            db,
            r#"SELECT count(*) FROM "Workspace" WHERE "id" = 2001"#,
            vec![]
        )
        .await,
        0,
        "the transaction must be rolled back"
    );
    result
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event-fk.test.ts:139 "GoogleAdsConversionEvent foreign keys > deleting the ContactInbox nulls contactInboxId and keeps the event"
#[tokio::test]
async fn deleting_the_contact_inbox_nulls_contact_inbox_id_and_keeps_the_event() {
    let db = fresh_db().await;
    let (seeded, row) = with_rolled_back_transaction(&db, |tx, seeded| {
        tx.execute(
            r#"DELETE FROM "ContactInbox" WHERE "id" = ?1"#,
            params![seeded.contact_inbox_id],
        )?;
        Ok(())
    })
    .await;
    let (contact_inbox_id, integration_google_ads_id) = row.expect("the event must be kept");
    assert_eq!(contact_inbox_id, None);
    assert_eq!(integration_google_ads_id, Some(seeded.satellite_id));
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event-fk.test.ts:153 "GoogleAdsConversionEvent foreign keys > deleting the IntegrationGoogleAds row nulls integrationGoogleAdsId and keeps the event"
#[tokio::test]
async fn deleting_the_integration_google_ads_row_nulls_integration_google_ads_id_and_keeps_the_event()
 {
    let db = fresh_db().await;
    let (seeded, row) = with_rolled_back_transaction(&db, |tx, seeded| {
        tx.execute(
            r#"DELETE FROM "IntegrationGoogleAds" WHERE "id" = ?1"#,
            params![seeded.satellite_id],
        )?;
        Ok(())
    })
    .await;
    let (contact_inbox_id, integration_google_ads_id) = row.expect("the event must be kept");
    assert_eq!(integration_google_ads_id, None);
    assert_eq!(contact_inbox_id, Some(seeded.contact_inbox_id));
}
