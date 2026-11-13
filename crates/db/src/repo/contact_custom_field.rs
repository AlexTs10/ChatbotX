//! Port of `repositories/contact-custom-field/repository.ts`.

use rusqlite::Connection;

use super::contact::ContactCustomFieldWithDefinition;
use super::models::{ContactCustomField, CustomField};
use super::sql::Sql;
use super::{Model, Result, id_list, text_list};

/// `lockPairs`: serializes read-modify-write on `(contact, field)` pairs. With one SQLite
/// writer every write transaction is already serial, so this only validates the call
/// (the advisory locks of Postgres are not needed). Kept so callers stay 1:1.
pub fn lock_pairs(_conn: &Connection, pairs: &[(i64, i64)]) -> Result<()> {
    let _ = pairs;
    Ok(())
}

/// `listValuesForPairs`: `(contactId, customFieldId, value)` of every requested pair.
pub fn list_values_for_pairs(
    conn: &Connection,
    contact_ids: &[i64],
    custom_field_ids: &[i64],
) -> Result<Vec<(i64, i64, String)>> {
    if contact_ids.is_empty() || custom_field_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"contactId\", \"customFieldId\", \"value\" FROM \"ContactCustomField\" \
         WHERE \"contactId\" IN (SELECT value FROM json_each(?1)) AND \"customFieldId\" IN (SELECT value FROM json_each(?2))",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![id_list(contact_ids), id_list(custom_field_ids)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `deleteValue`: deletes one value and returns what it held.
pub fn delete_value(
    conn: &Connection,
    contact_id: i64,
    custom_field_id: i64,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare_cached(
        "DELETE FROM \"ContactCustomField\" WHERE \"contactId\" = ?1 AND \"customFieldId\" = ?2 RETURNING \"value\"",
    )?;
    let mut rows = stmt.query([contact_id, custom_field_id])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

/// One `listSelectedDefinitionValuesByContact` row: the definition and the stored value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedDefinitionValue {
    pub id: i64,
    pub name: String,
    pub r#type: String,
    pub value: String,
}

/// `listSelectedDefinitionValuesByContact`: the contact's values of `custom_field_ids`, both
/// the contact and the field scoped to the workspace (ordered by field id).
pub fn list_selected_definition_values_by_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    custom_field_ids: &[i64],
) -> Result<Vec<SelectedDefinitionValue>> {
    if custom_field_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT f.\"id\", f.\"name\", f.\"type\", v.\"value\" FROM \"ContactCustomField\" AS v \
         JOIN \"CustomField\" AS f ON f.\"id\" = v.\"customFieldId\" JOIN \"Contact\" AS c ON c.\"id\" = v.\"contactId\" \
         WHERE v.\"contactId\" = ?1 AND c.\"workspaceId\" = ?2 AND f.\"workspaceId\" = ?2 \
         AND v.\"customFieldId\" IN (SELECT value FROM json_each(?3)) ORDER BY f.\"id\"",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![contact_id, workspace_id, id_list(custom_field_ids)],
            |r| {
                Ok(SelectedDefinitionValue {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    r#type: r.get(2)?,
                    value: r.get(3)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One `listValuesByFieldKeys` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldKeyValue {
    pub custom_field_id: i64,
    pub name: String,
    pub value: String,
    pub updated_at: i64,
}

/// `listValuesByFieldKeys`: the contact's values for the fields matching any of `ids` or
/// `names`, newest write first (both ownership boundaries checked).
pub fn list_values_by_field_keys(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    ids: &[i64],
    names: &[String],
) -> Result<Vec<FieldKeyValue>> {
    if ids.is_empty() && names.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT f.\"id\", f.\"name\", v.\"value\", v.\"updatedAt\" FROM \"ContactCustomField\" AS v \
         JOIN \"CustomField\" AS f ON f.\"id\" = v.\"customFieldId\" JOIN \"Contact\" AS c ON c.\"id\" = v.\"contactId\" \
         WHERE v.\"contactId\" = ?1 AND c.\"workspaceId\" = ?2 AND f.\"workspaceId\" = ?2 \
         AND (f.\"id\" IN (SELECT value FROM json_each(?3)) OR f.\"name\" IN (SELECT value FROM json_each(?4))) \
         ORDER BY v.\"updatedAt\" DESC, v.\"id\" DESC",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![contact_id, workspace_id, id_list(ids), text_list(names)],
            |r| {
                Ok(FieldKeyValue {
                    custom_field_id: r.get(0)?,
                    name: r.get(1)?,
                    value: r.get(2)?,
                    updated_at: r.get(3)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn with_definition(
    conn: &Connection,
    contact_id: i64,
    workspace_id: i64,
    custom_field_id: Option<i64>,
) -> Result<Vec<ContactCustomFieldWithDefinition>> {
    let mut q = Sql::raw(format!(
        "SELECT {}, {} FROM \"ContactCustomField\" AS v JOIN \"CustomField\" AS f ON f.\"id\" = v.\"customFieldId\" WHERE v.\"contactId\" = ",
        ContactCustomField::columns_sql(Some("v")),
        CustomField::columns_sql(Some("f"))
    ));
    q.push_bind(contact_id);
    q.push(" AND f.\"workspaceId\" = ");
    q.push_bind(workspace_id);
    if let Some(id) = custom_field_id {
        q.push(" AND v.\"customFieldId\" = ");
        q.push_bind(id);
    }
    q.push(" ORDER BY v.\"id\"");
    let mut stmt = conn.prepare(&q.text)?;
    let width = ContactCustomField::COLUMNS.len();
    let rows = stmt
        .query_map(rusqlite::params_from_iter(q.params.iter()), |r| {
            Ok(ContactCustomFieldWithDefinition {
                value: ContactCustomField::from_row_at(r, 0)?,
                custom_field: CustomField::from_row_at(r, width)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `listWithDefinitionByContact`: the contact's values whose field is in the workspace,
/// each with its definition.
pub fn list_with_definition_by_contact(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<ContactCustomFieldWithDefinition>> {
    with_definition(conn, contact_id, workspace_id, None)
}

/// `findWithDefinition`: one value with its definition (field in the workspace).
pub fn find_with_definition(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    custom_field_id: i64,
) -> Result<Option<ContactCustomFieldWithDefinition>> {
    Ok(
        with_definition(conn, contact_id, workspace_id, Some(custom_field_id))?
            .into_iter()
            .next(),
    )
}
