//! `place-near-me/contact-location.ts` (`findContactLocation`), with
//! `contactCustomFieldService.listValuesByKeys` and
//! `contactCustomFieldRepository.listValuesByFieldKeys` as private queries
//! (they move to the contact-custom-field port / `crates/db` later).

use chatbotx_core::flow_config::field_reference::is_bot_field_reference;
use chatbotx_core::utils::coordinates::parse_coordinate_pair;
use chatbotx_core::utils::id::is_numeric_id;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value;

use super::nearest::Coordinates;
use crate::errors::Result;
use crate::tx;

/// Input of [`find_contact_location`]: custom field ids or legacy names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactLocationInput {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub keys: Vec<String>,
}

/// One value of `listValuesByKeys`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactFieldValue {
    pub key: String,
    pub value: String,
    pub updated_at: i64,
}

struct FieldRow {
    custom_field_id: i64,
    name: String,
    value: String,
    updated_at: i64,
}

/// `contactCustomFieldRepository.listValuesByFieldKeys` (newest first).
fn list_values_by_field_keys(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    ids: &[i64],
    names: &[String],
) -> Result<Vec<FieldRow>> {
    if ids.is_empty() && names.is_empty() {
        return Ok(Vec::new());
    }
    let mut args: Vec<Value> = vec![Value::Integer(contact_id), Value::Integer(workspace_id)];
    let mut any = Vec::new();
    if !ids.is_empty() {
        let marks: Vec<String> = ids
            .iter()
            .map(|id| {
                args.push(Value::Integer(*id));
                format!("?{}", args.len())
            })
            .collect();
        any.push(format!("f.\"id\" IN ({})", marks.join(", ")));
    }
    if !names.is_empty() {
        let marks: Vec<String> = names
            .iter()
            .map(|name| {
                args.push(Value::Text(name.clone()));
                format!("?{}", args.len())
            })
            .collect();
        any.push(format!("f.\"name\" IN ({})", marks.join(", ")));
    }
    let sql = format!(
        "SELECT f.\"id\", f.\"name\", v.\"value\", v.\"updatedAt\" FROM \"ContactCustomField\" v \
         JOIN \"CustomField\" f ON v.\"customFieldId\" = f.\"id\" \
         JOIN \"Contact\" c ON v.\"contactId\" = c.\"id\" \
         WHERE v.\"contactId\" = ?1 AND c.\"workspaceId\" = ?2 AND f.\"workspaceId\" = ?2 AND ({}) \
         ORDER BY v.\"updatedAt\" DESC, v.\"id\" DESC",
        any.join(" OR ")
    );
    let mut stmt = conn.prepare(&sql).map_err(chatbotx_db::repo::Error::from)?;
    let rows = stmt
        .query_map(chatbotx_db::rusqlite::params_from_iter(args), |r| {
            Ok(FieldRow {
                custom_field_id: r.get(0)?,
                name: r.get(1)?,
                value: r.get(2)?,
                updated_at: r.get(3)?,
            })
        })
        .map_err(chatbotx_db::repo::Error::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(chatbotx_db::repo::Error::from)?;
    Ok(rows)
}

/// `contactCustomFieldService.listValuesByKeys`: one value per key (id match
/// first, then name), newest first.
pub async fn list_values_by_keys(
    db: &Db,
    input: ContactLocationInput,
) -> Result<Vec<ContactFieldValue>> {
    todo!("P3.6-D1")
}

/// `findContactLocation`: the newest `lat,lng` value the contact holds in any of
/// the given custom fields; an unparseable newest value falls through to the
/// next one; `None` when none qualifies.
pub async fn find_contact_location(
    db: &Db,
    input: ContactLocationInput,
) -> Result<Option<Coordinates>> {
    todo!("P3.6-D1")
}
