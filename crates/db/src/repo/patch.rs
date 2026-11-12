//! A column patch: the Rust form of a Drizzle `.set({...})` / `.values({...})` object
//! whose keys the caller picks (P3.3). Column names are checked against the table's
//! [`Model::COLUMNS`](super::Model), so no caller text reaches the SQL.

use rusqlite::types::Value;

use super::sql::Sql;
use super::{Error, Result};

/// `(column, value)` pairs in caller order.
pub type Patch = Vec<(&'static str, Value)>;

/// Fails when `patch` names a column that `columns` does not have.
pub fn check(columns: &[&str], patch: &Patch) -> Result<()> {
    for (col, _) in patch {
        if !columns.contains(col) {
            return Err(Error::msg(format!("unknown column in patch: {col}")));
        }
    }
    Ok(())
}

/// `"a" = ?, "b" = ?` of the patch (checked against `columns`).
pub fn set_clause(columns: &[&str], patch: &Patch) -> Result<Sql> {
    check(columns, patch)?;
    let mut out = Sql::default();
    for (i, (col, value)) in patch.iter().enumerate() {
        if i > 0 {
            out.push(", ");
        }
        out.push(&format!("\"{col}\" = "));
        out.push_bind(value.clone());
    }
    Ok(out)
}

/// `("a", "b") VALUES (?, ?)` of the patch (checked against `columns`).
pub fn insert_clause(columns: &[&str], patch: &Patch) -> Result<Sql> {
    check(columns, patch)?;
    let names: Vec<String> = patch.iter().map(|(c, _)| format!("\"{c}\"")).collect();
    let marks: Vec<&str> = patch.iter().map(|_| "?").collect();
    Ok(Sql::new(
        format!("({}) VALUES ({})", names.join(", "), marks.join(", ")),
        patch.iter().map(|(_, v)| v.clone()).collect(),
    ))
}

/// `"a" = excluded."a", ...` for the patch columns.
pub fn excluded_clause(patch: &Patch) -> String {
    patch
        .iter()
        .map(|(c, _)| format!("\"{c}\" = excluded.\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A JSON value as a bind value (compact text).
pub fn json(value: &serde_json::Value) -> Value {
    Value::Text(value.to_string())
}
