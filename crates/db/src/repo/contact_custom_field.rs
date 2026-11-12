//! Port of `repositories/contact-custom-field/repository.ts` (`lockPairs`,
//! `listValuesForPairs`, `deleteValue`; the definition readers are listed as not ported
//! yet in `conversion/specs/P3.2.md`).

use rusqlite::Connection;

use super::{Result, id_list};

/// `lockPairs`: serializes read-modify-write on `(contact, field)` pairs. With one SQLite
/// writer every write transaction is already serial, so this only validates the call
/// (the advisory locks of Postgres are not needed). Kept so callers stay 1:1.
pub fn lock_pairs(_conn: &Connection, pairs: &[(i64, i64)]) -> Result<()> {
    todo!("P3.2");
    let _ = pairs;
    Ok(())
}

/// `listValuesForPairs`: `(contactId, customFieldId, value)` of every requested pair.
pub fn list_values_for_pairs(conn: &Connection, contact_ids: &[i64], custom_field_ids: &[i64]) -> Result<Vec<(i64, i64, String)>> {
    todo!("P3.2");
    if contact_ids.is_empty() || custom_field_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"contactId\", \"customFieldId\", \"value\" FROM \"ContactCustomField\" \
         WHERE \"contactId\" IN (SELECT value FROM json_each(?1)) AND \"customFieldId\" IN (SELECT value FROM json_each(?2))",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![id_list(contact_ids), id_list(custom_field_ids)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `deleteValue`: deletes one value and returns what it held.
pub fn delete_value(conn: &Connection, contact_id: i64, custom_field_id: i64) -> Result<Option<String>> {
    todo!("P3.2");
    let mut stmt = conn.prepare_cached(
        "DELETE FROM \"ContactCustomField\" WHERE \"contactId\" = ?1 AND \"customFieldId\" = ?2 RETURNING \"value\"",
    )?;
    let mut rows = stmt.query([contact_id, custom_field_id])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}
