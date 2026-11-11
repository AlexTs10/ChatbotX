//! Port of `packages/auth/__tests__/schema-parity.test.ts` (P2.3 spec section 5.1, row 14.1).
//!
//! Replacement (DECISIONS 2026-11-11 "P2.3 disputed tests"): the better-auth
//! `getExpectedSchema(...)` output of the original test is frozen once in
//! `conversion/fixtures/better-auth-expected-schema.json`; this test runs better-auth's
//! `diffSchema` rules against the migrated SQLite tables.

mod common;

use std::collections::BTreeSet;

use common::{columns, fresh_db, APP_GENERATED_COLUMNS};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../conversion/fixtures/better-auth-expected-schema.json");

const OAUTH_TABLE_NAMES: &[&str] = &[
    "jwks",
    "oauthAccessToken",
    "oauthClient",
    "oauthClientAssertion",
    "oauthClientResource",
    "oauthConsent",
    "oauthRefreshToken",
    "oauthResource",
];

/// The `schema` object of the original test: adapter model name -> SQL table.
const ADAPTER_TABLES: &[(&str, &str)] = &[
    ("account", "Account"),
    ("jwks", "Jwk"),
    ("oauthAccessToken", "OAuthAccessToken"),
    ("oauthClient", "OAuthClient"),
    ("oauthClientAssertion", "OAuthClientAssertion"),
    ("oauthClientResource", "OAuthClientResource"),
    ("oauthConsent", "OAuthConsent"),
    ("oauthRefreshToken", "OAuthRefreshToken"),
    ("oauthResource", "OAuthResource"),
    ("session", "Session"),
    ("user", "User"),
    ("verification", "Verification"),
];

/// Port of: packages/auth/__tests__/schema-parity.test.ts:34 "Better Auth OAuth schema parity > requires every Better Auth OAuth table that the adapter maps"
#[tokio::test]
async fn better_auth_tables_have_the_expected_columns() {
    let text = std::fs::read_to_string(FIXTURE).unwrap_or_else(|e| {
        panic!("fixture {FIXTURE} is missing ({e}); generate it once from the TS repo (DECISIONS P2.3)")
    });
    let fixture: serde_json::Value = serde_json::from_str(&text).expect("fixture is JSON");
    let expected = fixture["tables"].as_object().expect("fixture.tables is an object");

    for name in OAUTH_TABLE_NAMES {
        assert!(expected.contains_key(*name), "expected schema lacks {name}");
    }

    let db = fresh_db().await;
    // diffSchema(expected, actual) of @better-auth/core 1.7.14 (`db/schema-diff`).
    let mut findings: Vec<String> = Vec::new();
    for (table_name, table) in expected {
        if table["disableMigrations"].as_bool() == Some(true) {
            continue;
        }
        let Some((_, sql_table)) = ADAPTER_TABLES.iter().find(|(k, _)| k == table_name) else {
            findings.push(format!("missing-table {table_name}"));
            continue;
        };
        let actual = columns(&db, sql_table).await;
        if actual.is_empty() {
            findings.push(format!("missing-table {table_name}"));
            continue;
        }
        let id_column = table["idColumn"].as_str().unwrap_or("id").to_owned();
        let mut written: BTreeSet<String> = BTreeSet::from([id_column]);
        written.extend(table["fields"].as_object().expect("fields is an object").keys().cloned());

        for column in &written {
            if !actual.iter().any(|c| &c.name == column) {
                findings.push(format!("missing-column {table_name}.{column}"));
            }
        }
        for column in &actual {
            let nullable = !column.notnull;
            // Drizzle `hasDefault` is also true for `$defaultFn` (the app-generated ids).
            let has_default = column.dflt_value.is_some()
                || column.hidden != 0
                || APP_GENERATED_COLUMNS.contains(&column.name.as_str());
            if written.contains(&column.name) || nullable || has_default {
                continue;
            }
            findings.push(format!("unexpected-required-column {table_name}.{}", column.name));
        }
    }
    assert_eq!(findings, Vec::<String>::new());
}
