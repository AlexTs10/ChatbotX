//! Port of `packages/auth/__tests__/schema-parity.test.ts`.
//!
//! The original diffs better-auth's expected schema (core + `anonymous`,
//! `jwt`, `mcp` plugins and the builder's additional fields) against the
//! Drizzle tables. Here the migrated SQLite schema is checked for every table
//! the auth port maps and the better-auth fields it reads and writes
//! (adapted: no better-auth schema generator in Rust).

use std::collections::HashSet;

use chatbotx_db::Db;

const TABLES: &[(&str, &[&str])] = &[
    (
        "User",
        &[
            "id",
            "name",
            "email",
            "emailVerified",
            "image",
            "createdAt",
            "updatedAt",
            "isAnonymous",
            "mustChangePassword",
            "tenantId",
        ],
    ),
    (
        "Session",
        &[
            "id",
            "expiresAt",
            "token",
            "createdAt",
            "updatedAt",
            "ipAddress",
            "userAgent",
            "userId",
        ],
    ),
    (
        "Account",
        &[
            "id",
            "accountId",
            "providerId",
            "userId",
            "accessToken",
            "refreshToken",
            "idToken",
            "password",
            "createdAt",
            "updatedAt",
        ],
    ),
    (
        "Verification",
        &[
            "id",
            "identifier",
            "value",
            "expiresAt",
            "createdAt",
            "updatedAt",
        ],
    ),
    (
        "Jwk",
        &["id", "publicKey", "privateKey", "createdAt", "expiresAt"],
    ),
    (
        "OAuthAccessToken",
        &[
            "id",
            "token",
            "clientId",
            "sessionId",
            "userId",
            "referenceId",
            "refreshId",
            "expiresAt",
            "scopes",
        ],
    ),
    (
        "OAuthClient",
        &[
            "id",
            "clientId",
            "clientSecret",
            "disabled",
            "skipConsent",
            "scopes",
            "userId",
            "name",
            "redirectUris",
            "tokenEndpointAuthMethod",
            "grantTypes",
            "responseTypes",
            "requirePKCE",
            "metadata",
        ],
    ),
    ("OAuthClientAssertion", &["id", "expiresAt"]),
    ("OAuthClientResource", &["id", "clientId", "resourceId"]),
    (
        "OAuthConsent",
        &["id", "clientId", "userId", "referenceId", "scopes"],
    ),
    (
        "OAuthRefreshToken",
        &[
            "id",
            "token",
            "clientId",
            "sessionId",
            "userId",
            "expiresAt",
            "revoked",
            "scopes",
        ],
    ),
    ("OAuthResource", &["id", "identifier", "name"]),
];

#[tokio::test]
async fn requires_every_better_auth_oauth_table_that_the_adapter_maps() {
    let db = Db::open_in_memory().expect("open");
    db.migrate().await.expect("migrate");
    let missing = db
        .read(|conn| {
            let mut missing = Vec::new();
            for (table, columns) in TABLES {
                let mut statement = conn.prepare(&format!(r#"PRAGMA table_info("{table}")"#))?;
                let actual: HashSet<String> = statement
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<Result<_, _>>()?;
                if actual.is_empty() {
                    missing.push(format!("table {table}"));
                }
                for column in *columns {
                    if !actual.is_empty() && !actual.contains(*column) {
                        missing.push(format!("{table}.{column}"));
                    }
                }
            }
            Ok(missing)
        })
        .await
        .expect("read schema");
    assert_eq!(missing, Vec::<String>::new());
}
