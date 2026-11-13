//! Shared fixtures of the ported business tests (P3.6-D1): a migrated SQLite
//! file per test and small row inserts.

#![allow(dead_code)]

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};

/// A migrated database in a temp directory (removed on drop).
pub struct TestDb {
    db: Db,
    dir: PathBuf,
}

impl Deref for TestDb {
    type Target = Db;
    fn deref(&self) -> &Db {
        &self.db
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub async fn fresh_db() -> TestDb {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-business-test-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let db = Db::open(dir.join("test.db")).expect("Db::open");
    db.migrate().await.expect("migrate");
    TestDb { db, dir }
}

/// Runs `f` in one write transaction (test setup).
pub async fn setup<F, R>(db: &Db, f: F) -> R
where
    F: FnOnce(&Connection) -> Result<R, DbError> + Send + 'static,
    R: Send + 'static,
{
    db.write(move |tx| f(tx)).await.expect("test setup write")
}

/// One query that returns one integer (or `None`).
pub async fn scalar_i64(db: &Db, sql: &'static str, id: i64) -> Option<i64> {
    db.read(move |conn| {
        Ok(conn
            .query_row(sql, params![id], |r| r.get::<_, Option<i64>>(0))
            .ok()
            .flatten())
    })
    .await
    .expect("scalar read")
}

/// Inserts a `User` (`id`) and a `Workspace` (`workspace_id`) it owns.
pub fn workspace(tx: &Connection, user_id: i64, workspace_id: i64) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
        params![user_id, format!("user-{user_id}@example.test")],
    )?;
    tx.execute(
        r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, ?2, ?3)"#,
        params![workspace_id, format!("Workspace {workspace_id}"), user_id],
    )?;
    Ok(workspace_id)
}

/// Inserts a `Contact` of a workspace.
pub fn contact(tx: &Connection, id: i64, workspace_id: i64) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'Ada', ?2)"#,
        params![id, workspace_id],
    )?;
    Ok(id)
}

/// The workspace ids the fixtures use: `WS` and a second one, `OTHER_WS`.
pub const WS: i64 = 2001;
pub const OTHER_WS: i64 = 2002;

/// A fresh database with workspaces `WS` and `OTHER_WS`.
pub async fn two_workspaces() -> TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        workspace(tx, 1001, WS)?;
        workspace(tx, 1002, OTHER_WS)?;
        Ok(())
    })
    .await;
    db
}

/// Inserts a `ProductCategory` (`parent` = its top-level parent).
pub fn category(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    parent: Option<i64>,
    name: &str,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "ProductCategory" ("id", "workspaceId", "parentId", "name") VALUES (?1, ?2, ?3, ?4)"#,
        params![id, workspace_id, parent, name],
    )?;
    Ok(id)
}

/// Inserts a `Vendor`.
pub fn vendor(tx: &Connection, id: i64, workspace_id: i64, name: &str) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Vendor" ("id", "workspaceId", "name") VALUES (?1, ?2, ?3)"#,
        params![id, workspace_id, name],
    )?;
    Ok(id)
}

/// Inserts a `Product` filed under a category / sub-category.
pub fn product(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    category: Option<i64>,
    subcategory: Option<i64>,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Product" ("id", "workspaceId", "name", "categoryId", "subcategoryId") VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![id, workspace_id, format!("Product {id}"), category, subcategory],
    )?;
    Ok(id)
}

/// Inserts an active `DiscountCode` (`applies_to` = `category` / `products`).
pub fn discount_code(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    applies_to: &str,
    category: Option<i64>,
    product_ids: &str,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "DiscountCode" ("id", "workspaceId", "code", "discountType", "appliesTo", "categoryId", "productIds")
           VALUES (?1, ?2, ?3, 'percentage', ?4, ?5, ?6)"#,
        params![id, workspace_id, format!("CODE{id}"), applies_to, category, product_ids],
    )?;
    Ok(id)
}

/// Inserts an `EcommerceOpenHours` schedule.
pub fn open_hours(tx: &Connection, id: i64, workspace_id: i64) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "EcommerceOpenHours" ("id", "workspaceId", "name", "ranges") VALUES (?1, ?2, 'Hours', '[]')"#,
        params![id, workspace_id],
    )?;
    Ok(id)
}

/// Sets the workspace's store source (`EcommerceSettings.storeSource`).
pub fn store_source(tx: &Connection, workspace_id: i64, source: &str) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "EcommerceSettings" ("id", "workspaceId", "storeSource", "orderingAvailable",
             "webshopProductView", "searchProducts", "productButtons", "productImageSize", "allowRateOrders",
             "notifyOnCartStatusChange", "paymentMethods", "currency", "currencyPosition", "serviceFeePercent",
             "shippingType", "shippingCountries", "deliveryCost", "checkoutMode", "minimumOrderValue",
             "requireName", "requireEmail", "requirePhone", "requireState", "requireZip")
           VALUES (?1, ?1, ?2, 1, 'list', 1, '[]', 'bigSquare', 0, 0, '[]', 'USD', 'left', 0,
             'none', '[]', 0, 'default', 0, 0, 0, 0, 0, 0)"#,
        params![workspace_id, source],
    )?;
    Ok(())
}

/// One text column of one row (or `None`).
pub async fn scalar_text(db: &Db, sql: &'static str, id: i64) -> Option<String> {
    db.read(move |conn| {
        Ok(conn
            .query_row(sql, params![id], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten())
    })
    .await
    .expect("scalar read")
}

/// Inserts a `Flow`.
pub fn flow(tx: &Connection, id: i64, workspace_id: i64) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (?1, ?2, ?3)"#,
        params![id, workspace_id, format!("Flow {id}")],
    )?;
    Ok(id)
}

/// Inserts an `Inbox` of a channel.
pub fn inbox(tx: &Connection, id: i64, workspace_id: i64, channel: &str) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![id, workspace_id, format!("Inbox {id}"), channel, format!("src-{id}")],
    )?;
    Ok(id)
}

/// Inserts a `MediaLibraryFile` with a MIME type.
pub fn media_file(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    mime_type: &str,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "MediaLibraryFile" ("id", "workspaceId", "name", "path", "mimeType", "size") VALUES (?1, ?2, 'logo', ?3, ?4, 1)"#,
        params![id, workspace_id, format!("files/{id}"), mime_type],
    )?;
    Ok(id)
}

/// Inserts a `Reflink` row (`kind` = `refLink` / `qrCode`).
pub fn reflink_row(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    kind: &str,
    name: &str,
    flow_id: i64,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Reflink" ("id", "workspaceId", "type", "name", "flowId") VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![id, workspace_id, kind, name, flow_id],
    )?;
    Ok(id)
}

/// Sets `Workspace.targetCountry` (P3.6-D2 contacts).
pub fn target_country(tx: &Connection, workspace_id: i64, country: &str) -> Result<(), DbError> {
    tx.execute(
        r#"UPDATE "Workspace" SET "targetCountry" = ?2 WHERE "id" = ?1"#,
        params![workspace_id, country],
    )?;
    Ok(())
}

/// Inserts a `Contact` with a first name, an email and a phone number.
pub fn contact_with(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    first_name: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Contact" ("id", "firstName", "email", "phoneNumber", "workspaceId") VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![id, first_name, email, phone, workspace_id],
    )?;
    Ok(id)
}

/// Inserts a `ContactInbox` (`source` = `direct`).
pub fn contact_inbox(
    tx: &Connection,
    id: i64,
    contact_id: i64,
    inbox_id: i64,
    channel: &str,
    source_id: &str,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
           VALUES (?1, ?2, ?2, ?3, ?4, 'direct', ?5)"#,
        params![id, contact_id, inbox_id, channel, source_id],
    )?;
    Ok(id)
}

/// Inserts a `Conversation` of a contact.
pub fn conversation(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    contact_id: i64,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (?1, ?2, ?3)"#,
        params![id, workspace_id, contact_id],
    )?;
    Ok(id)
}

/// Inserts a `CustomField` of a type.
pub fn custom_field(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    name: &str,
    kind: &str,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "CustomField" ("id", "workspaceId", "name", "type") VALUES (?1, ?2, ?3, ?4)"#,
        params![id, workspace_id, name, kind],
    )?;
    Ok(id)
}

/// The rows of a query as JSON objects (column name -> value; integers,
/// reals, text and `null`).
pub async fn rows(
    db: &Db,
    sql: &'static str,
    id: i64,
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    db.read(move |conn| {
        let mut stmt = conn.prepare(sql)?;
        let names: Vec<String> = stmt.column_names().map(|n| n.to_string()).collect();
        let out = stmt
            .query_map(params![id], |r| {
                let mut row = serde_json::Map::new();
                for (i, name) in names.iter().enumerate() {
                    use chatbotx_db::rusqlite::types::ValueRef;
                    let value = match r.get_ref(i)? {
                        ValueRef::Null => serde_json::Value::Null,
                        ValueRef::Integer(n) => serde_json::Value::from(n),
                        ValueRef::Real(f) => serde_json::Value::from(f),
                        ValueRef::Text(t) => {
                            serde_json::Value::from(String::from_utf8_lossy(t).into_owned())
                        }
                        ValueRef::Blob(_) => serde_json::Value::Null,
                    };
                    row.insert(name.clone(), value);
                }
                Ok(row)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    })
    .await
    .expect("rows read")
}

/// Inserts a `Folder` of a type (`paths` `[]`).
pub fn folder(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    folder_type: &str,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Folder" ("id", "workspaceId", "name", "folderType", "paths") VALUES (?1, ?2, ?3, ?4, '[]')"#,
        params![id, workspace_id, format!("Folder {id}"), folder_type],
    )?;
    Ok(id)
}

/// Inserts a live `Tag`.
pub fn tag(tx: &Connection, id: i64, workspace_id: i64, name: &str) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Tag" ("id", "workspaceId", "name") VALUES (?1, ?2, ?3)"#,
        params![id, workspace_id, name],
    )?;
    Ok(id)
}

/// Links a contact and a tag (`"ContactToTag"`).
pub fn tag_link(tx: &Connection, contact_id: i64, tag_id: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES (?1, ?2)"#,
        params![contact_id, tag_id],
    )?;
    Ok(())
}

/// The tag ids linked to a contact, ascending.
pub async fn linked_tag_ids(db: &Db, contact_id: i64) -> Vec<i64> {
    db.read(move |conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "tagId" FROM "ContactToTag" WHERE "contactId" = ?1 ORDER BY "tagId""#,
        )?;
        let ids = stmt
            .query_map(params![contact_id], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    })
    .await
    .expect("linked tags")
}

/// Inserts a `ContactCustomField` value (P3.6-D2 contact custom-field values).
pub fn custom_field_value(
    tx: &Connection,
    id: i64,
    contact_id: i64,
    custom_field_id: i64,
    value: &str,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value") VALUES (?1, ?2, ?3, ?4)"#,
        params![id, contact_id, custom_field_id, value],
    )?;
    Ok(id)
}

/// The stored `(customFieldId, value)` pairs of a contact, by field id.
pub async fn custom_field_values(db: &Db, contact_id: i64) -> Vec<(i64, String)> {
    db.read(move |conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "customFieldId", "value" FROM "ContactCustomField" WHERE "contactId" = ?1 ORDER BY "customFieldId""#,
        )?;
        let out = stmt
            .query_map(params![contact_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    })
    .await
    .expect("custom field values read")
}
