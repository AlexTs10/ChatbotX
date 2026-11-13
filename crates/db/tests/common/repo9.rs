//! Row fixtures of the P3.2b contact-filter ports (slices B-D; `repo8.rs` and the older
//! fixture files are locked). Include after `repo8`:
//! ```ignore
//! #[path = "common/repo9.rs"]
//! mod repo9;
//! ```

#![allow(dead_code)]

use chatbotx_db::Db;
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::rusqlite::{self, params};

use crate::common::{TestDb, fresh_db, seed_workspace};

/// The fixture workspace / user / custom field.
pub const WS: i64 = 1;
pub const USER: i64 = 1;
pub const CUSTOM_FIELD: i64 = 20;

/// Contacts `100 + i` of workspace [`WS`], each with `ContactCustomField(CUSTOM_FIELD)` =
/// `values[i]`.
pub async fn custom_field_rows(values: &[&str]) -> TestDb {
    let db = fresh_db().await;
    seed_workspace(&db, USER, WS).await;
    let values: Vec<String> = values.iter().map(|v| (*v).to_owned()).collect();
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (?1, 'cf', 'shortText', ?2)"#,
            params![CUSTOM_FIELD, WS],
        )?;
        for (i, v) in values.iter().enumerate() {
            let contact = 100 + i as i64;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (?1, ?2)"#,
                params![contact, WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactCustomField" ("id", "value", "contactId", "customFieldId") VALUES (?1, ?2, ?1, ?3)"#,
                params![contact, v, CUSTOM_FIELD],
            )?;
        }
        Ok(())
    })
    .await
    .expect("custom field fixture");
    db
}

/// The ids of the `"Contact"` rows the predicate matches, ascending.
pub async fn matching(db: &Db, q: &Sql) -> Vec<i64> {
    let sql = format!(
        "SELECT \"Contact\".\"id\" FROM \"Contact\" WHERE {} ORDER BY 1",
        q.text
    );
    let params = q.params.clone();
    db.read(move |c| {
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(rows)
    })
    .await
    .expect("matching query")
}
