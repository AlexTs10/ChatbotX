//! Shared helpers of the P3.2b contact-filter ports (on top of `common/mod.rs`;
//! `repo3.rs` .. `repo7.rs` and `log.rs` are locked).
//!
//! Include:
//! ```ignore
//! mod common;
//! #[path = "common/repo8.rs"]
//! mod repo8;
//! ```

#![allow(dead_code)]

use std::sync::OnceLock;

use chatbotx_db::Db;
use chatbotx_db::repo::queries::contact_filter::{FilterCriteria, apply_contact_filter};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::rusqlite::types::Value;
use serde_json::{Value as Json, json};

use crate::common::{TempDir, fresh_db};

/// The fixed "now" of the builder tests: `2026-06-01T00:00:00.000Z`.
pub const NOW: i64 = 1_780_272_000_000;

/// `Date.parse(iso)` (panics on bad input).
pub fn ms(iso: &str) -> i64 {
    chatbotx_db::repo::parse_iso(iso).unwrap_or_else(|| panic!("bad iso {iso}"))
}

/// A criteria from its JSON form (the TS literal).
pub fn crit(v: Json) -> FilterCriteria {
    serde_json::from_value(v).expect("criteria JSON")
}

/// `{ operator: "and", conditions: [c] }`.
pub fn and1(c: Json) -> Json {
    json!({ "operator": "and", "conditions": [c] })
}

/// `applyContactFilter(criteria)` (no workspace) at [`NOW`].
pub fn apply(v: Json) -> Option<Sql> {
    apply_contact_filter(&crit(v), None, NOW)
}

/// `applyContactFilter(criteria, workspaceId)` at [`NOW`].
pub fn apply_ws(v: Json, workspace_id: i64) -> Option<Sql> {
    apply_contact_filter(&crit(v), Some(workspace_id), NOW)
}

/// `renderContactWhere`: the predicate (panics on `{}`), checked to prepare on SQLite.
pub fn render(w: Option<Sql>) -> Sql {
    let q = w.expect("Expected contact filter to render SQL");
    prepares(&q);
    q
}

/// The one-condition filter rendered (`and1` + [`apply`] + [`render`]).
pub fn one(c: Json) -> Sql {
    render(apply(and1(c)))
}

/// `firstAnd(field, operator, value)`: the predicate of a one-condition AND filter.
pub fn first_and(field: &str, operator: &str, value: Option<Json>) -> Option<Sql> {
    let c = match value {
        Some(v) => json!({ "field": field, "operator": operator, "value": v }),
        None => json!({ "field": field, "operator": operator }),
    };
    apply(and1(c))
}

/// Asserts the exact SQL text and bind values.
pub fn same(q: &Option<Sql>, text: &str, params: Vec<Value>) {
    let q = q.as_ref().expect("a predicate");
    assert_eq!(q.text, text);
    assert_eq!(q.params, params);
    prepares(q);
}

pub fn has(q: &Sql, s: &str) {
    assert!(q.text.contains(s), "expected {s:?} in {:?}", q.text);
}

pub fn lacks(q: &Sql, s: &str) {
    assert!(!q.text.contains(s), "unexpected {s:?} in {:?}", q.text);
}

pub fn count(q: &Sql, s: &str) -> usize {
    q.text.matches(s).count()
}

pub fn has_param(q: &Sql, v: Value) {
    assert!(
        q.params.contains(&v),
        "expected param {v:?} in {:?}",
        q.params
    );
}

pub fn lacks_param(q: &Sql, v: Value) {
    assert!(
        !q.params.contains(&v),
        "unexpected param {v:?} in {:?}",
        q.params
    );
}

pub fn text(v: &str) -> Value {
    Value::Text(v.to_owned())
}

pub fn int(v: i64) -> Value {
    Value::Integer(v)
}

pub fn real(v: f64) -> Value {
    Value::Real(v)
}

/// An instant param (`Date.parse(iso)` as an integer bind).
pub fn at(iso: &str) -> Value {
    Value::Integer(ms(iso))
}

struct Shared {
    rt: tokio::runtime::Runtime,
    db: Db,
    _dir: TempDir,
}

fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let dir = TempDir::new();
        let db = rt.block_on(async {
            let db = Db::open(dir.path().join("test.db")).expect("Db::open");
            db.migrate().await.expect("migrate");
            db
        });
        Shared { rt, db, _dir: dir }
    })
}

/// Checks that the predicate prepares as `SELECT .. FROM "Contact" WHERE <q>` on the
/// migrated schema (with the app functions) and binds its values.
pub fn prepares(q: &Sql) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = shared();
    let sql = format!("SELECT count(*) FROM \"Contact\" WHERE {}", q.text);
    let params = q.params.clone();
    let res = s.rt.block_on(s.db.read(move |c| {
        let mut stmt = c.prepare(&sql)?;
        let n: i64 = stmt.query_row(
            chatbotx_db::rusqlite::params_from_iter(params.iter()),
            |r| r.get(0),
        )?;
        Ok(n)
    }));
    if let Err(e) = res {
        panic!("predicate does not run on SQLite: {e}\n{}", q.text);
    }
}

/// A migrated temp database (the integration fixtures).
pub async fn db() -> crate::common::TestDb {
    fresh_db().await
}
