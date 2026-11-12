//! A small SQL fragment builder: text with positional `?` marks and its bind values in
//! order (the Drizzle `sql` template / `and(...)` of the TypeScript).

use rusqlite::types::Value;
use rusqlite::{Connection, Row};

use super::{Model, Result};

/// SQL text with positional `?` placeholders and their values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sql {
    pub text: String,
    pub params: Vec<Value>,
}

impl Sql {
    /// A fragment without bind values.
    pub fn raw(text: impl Into<String>) -> Sql {
        Sql {
            text: text.into(),
            params: Vec::new(),
        }
    }

    /// A fragment with bind values (one `?` per value, in order).
    pub fn new(text: impl Into<String>, params: Vec<Value>) -> Sql {
        Sql {
            text: text.into(),
            params,
        }
    }

    /// `<text> ?` with `value` bound.
    pub fn bind(text: impl Into<String>, value: impl Into<Value>) -> Sql {
        let mut text = text.into();
        text.push('?');
        Sql {
            text,
            params: vec![value.into()],
        }
    }

    /// `self` followed by raw `tail` (builder form of [`Sql::push`]).
    pub fn closed(mut self, tail: &str) -> Sql {
        self.text.push_str(tail);
        self
    }

    /// Appends raw text.
    pub fn push(&mut self, text: &str) -> &mut Sql {
        self.text.push_str(text);
        self
    }

    /// Appends one `?` and its value.
    pub fn push_bind(&mut self, value: impl Into<Value>) -> &mut Sql {
        self.text.push('?');
        self.params.push(value.into());
        self
    }

    /// Appends another fragment.
    pub fn append(&mut self, other: Sql) -> &mut Sql {
        self.text.push_str(&other.text);
        self.params.extend(other.params);
        self
    }

    /// `(a) AND (b) AND ...`; `None` items are skipped (Drizzle `and(...)` drops
    /// `undefined`). An empty list is `1 = 1`.
    pub fn and(parts: impl IntoIterator<Item = Option<Sql>>) -> Sql {
        Sql::join(parts, " AND ", "1 = 1")
    }

    /// `(a) OR (b) OR ...`; an empty list is `1 = 0`.
    pub fn or(parts: impl IntoIterator<Item = Option<Sql>>) -> Sql {
        Sql::join(parts, " OR ", "1 = 0")
    }

    fn join(parts: impl IntoIterator<Item = Option<Sql>>, sep: &str, empty: &str) -> Sql {
        let mut out = Sql::default();
        let mut n = 0;
        for part in parts.into_iter().flatten() {
            if n > 0 {
                out.text.push_str(sep);
            }
            out.text.push('(');
            out.text.push_str(&part.text);
            out.text.push(')');
            out.params.extend(part.params);
            n += 1;
        }
        if n == 0 {
            out.text.push_str(empty);
        }
        out
    }

    /// Runs the fragment as a query and maps each row with `f`.
    pub fn query_map<T>(
        &self,
        conn: &Connection,
        mut f: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let mut stmt = conn.prepare_cached(&self.text)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(self.params.iter()), |row| f(row))?
            .collect::<rusqlite::Result<Vec<T>>>()?;
        Ok(rows)
    }

    /// Runs the fragment as a query of `M` rows.
    pub fn models<M: Model>(&self, conn: &Connection) -> Result<Vec<M>> {
        self.query_map(conn, |row| M::from_row(row))
    }

    /// Runs the fragment as a query of at most one `M` row.
    pub fn model<M: Model>(&self, conn: &Connection) -> Result<Option<M>> {
        Ok(self.models::<M>(conn)?.into_iter().next())
    }

    /// Executes the fragment; returns the changed row count.
    pub fn execute(&self, conn: &Connection) -> Result<usize> {
        let mut stmt = conn.prepare_cached(&self.text)?;
        Ok(stmt.execute(rusqlite::params_from_iter(self.params.iter()))?)
    }
}

/// `Option<i64>` -> `Value` (NULL for `None`).
pub fn opt_int(v: Option<i64>) -> Value {
    v.map_or(Value::Null, Value::Integer)
}

/// `Option<&str>` -> `Value` (NULL for `None`).
pub fn opt_text(v: Option<&str>) -> Value {
    v.map_or(Value::Null, |s| Value::Text(s.to_owned()))
}

/// `bool` -> `Value::Integer(0/1)`.
pub fn bool_val(v: bool) -> Value {
    Value::Integer(i64::from(v))
}
