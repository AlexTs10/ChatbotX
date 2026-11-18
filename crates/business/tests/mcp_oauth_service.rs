//! Port of `packages/business/__tests__/mcp-oauth.service.test.ts` (Q.A1).
//! The TS test mocks the repositories; here real rows are used. Not
//! ported: "synthetic platform-support membership" (support sessions are not
//! ported, see `mcp_oauth` module docs), "deleted token subject" via a mocked
//! 404 (a missing `User` row covers it), the revoke-failure / log assertions.

mod common;

use chatbotx_business::mcp_oauth::{
    Resolution, bind_consent_workspace, list_grants_for_user, purge_expired_tokens,
    resolve_principal, revoke_grant,
};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use common::{fresh_db, setup};

const USER: i64 = 7;
const WS: i64 = 2001;
const WS2: i64 = 2002;

fn client(
    tx: &Connection,
    client_id: &str,
    disabled: bool,
    name: Option<&str>,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "OAuthClient" ("clientId", "disabled", "redirectUris", "name") VALUES (?1, ?2, '[]', ?3)"#,
        params![client_id, disabled, name],
    )?;
    Ok(())
}

fn member(tx: &Connection, workspace: i64, role: &str, permissions: &str) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "WorkspaceMember" ("workspaceId", "userId", "role", "permissions") VALUES (?1, ?2, ?3, ?4)"#,
        params![workspace, USER, role, permissions],
    )?;
    Ok(())
}

fn consent(
    tx: &Connection,
    client_id: &str,
    workspace: Option<i64>,
    scopes: &str,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "OAuthConsent" ("clientId", "userId", "scopes", "workspaceId") VALUES (?1, ?2, ?3, ?4)"#,
        params![client_id, USER, scopes, workspace],
    )?;
    Ok(())
}

/// User 7, owner of `WS` ("Workspace") with client `client-1` consented to `WS`.
async fn base() -> common::TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (?1, 'u@example.com')"#,
            [USER],
        )?;
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (99, 'o@example.com')"#,
            [],
        )?;
        for (id, name) in [(WS, "Workspace"), (WS2, "Second workspace")] {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, ?2, 99)"#,
                params![id, name],
            )?;
        }
        member(tx, WS, "owner", "{}")?;
        client(tx, "client-1", false, Some("Claude"))?;
        Ok(())
    })
    .await;
    db
}

async fn resolve(db: &Db, workspace: Option<i64>) -> Resolution {
    resolve_principal(db, "client-1", USER, workspace)
        .await
        .unwrap()
}

fn is_principal(resolution: &Resolution, workspace: i64) -> bool {
    matches!(resolution, Resolution::Principal { workspace: w, user_id: USER, client_id } if w.id == workspace && client_id == "client-1")
}

async fn count(db: &Db, sql: &'static str) -> i64 {
    db.read(move |c| Ok(c.query_row(sql, [], |r| r.get(0))?))
        .await
        .unwrap()
}

#[tokio::test]
async fn accepts_a_valid_principal() {
    let db = base().await;
    setup(&db, |tx| consent(tx, "client-1", Some(WS), "[]")).await;
    assert!(is_principal(&resolve(&db, None).await, WS));
    assert!(is_principal(&resolve(&db, Some(WS)).await, WS));
}

#[tokio::test]
async fn rejects_a_disabled_or_missing_client() {
    let db = base().await;
    setup(&db, |tx| {
        client(tx, "off", true, None)?;
        consent(tx, "off", Some(WS), "[]")
    })
    .await;
    assert_eq!(
        resolve_principal(&db, "off", USER, None).await.unwrap(),
        Resolution::InvalidToken
    );
    assert_eq!(
        resolve_principal(&db, "missing", USER, None).await.unwrap(),
        Resolution::InvalidToken
    );
}

#[tokio::test]
async fn rejects_a_deleted_subject_and_a_missing_consent() {
    let db = base().await;
    assert_eq!(resolve(&db, None).await, Resolution::InvalidToken);
    setup(&db, |tx| consent(tx, "client-1", Some(WS), "[]")).await;
    assert_eq!(
        resolve_principal(&db, "client-1", 12345, None)
            .await
            .unwrap(),
        Resolution::InvalidToken
    );
}

#[tokio::test]
async fn rejects_a_requested_workspace_other_than_the_consented_one() {
    let db = base().await;
    setup(&db, |tx| {
        member(tx, WS2, "owner", "{}")?;
        consent(tx, "client-1", Some(WS), "[]")
    })
    .await;
    assert_eq!(resolve(&db, Some(WS2)).await, Resolution::Forbidden);
}

#[tokio::test]
async fn forbids_and_revokes_a_consented_workspace_no_longer_administered() {
    let db = base().await;
    setup(&db, |tx| {
        tx.execute(r#"UPDATE "WorkspaceMember" SET "role" = 'agent'"#, [])?;
        consent(tx, "client-1", Some(WS), "[]")
    })
    .await;
    assert_eq!(resolve(&db, None).await, Resolution::Forbidden);
    assert_eq!(
        count(&db, r#"SELECT count(*) FROM "OAuthConsent""#).await,
        0
    );
}

#[tokio::test]
async fn forbids_a_workspace_the_user_cannot_access() {
    let db = base().await;
    setup(&db, |tx| consent(tx, "client-1", Some(WS2), "[]")).await;
    assert_eq!(resolve(&db, None).await, Resolution::Forbidden);
}

#[tokio::test]
async fn legacy_unbound_consent_auto_selects_the_only_admin_workspace() {
    let db = base().await;
    setup(&db, |tx| consent(tx, "client-1", None, "[]")).await;
    assert!(is_principal(&resolve(&db, None).await, WS));
}

#[tokio::test]
async fn legacy_unbound_consent_is_forbidden_without_an_authorizable_workspace() {
    let db = base().await;
    setup(&db, |tx| {
        tx.execute(r#"UPDATE "WorkspaceMember" SET "role" = 'agent'"#, [])?;
        member(tx, WS2, "owner", "{}")?;
        tx.execute(
            r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
            [WS2],
        )?;
        consent(tx, "client-1", None, "[]")
    })
    .await;
    assert_eq!(resolve(&db, None).await, Resolution::Forbidden);
}

#[tokio::test]
async fn legacy_unbound_consent_requires_a_selection_among_several() {
    let db = base().await;
    setup(&db, |tx| {
        member(tx, WS2, "agent", r#"{"superAdmin":true}"#)?;
        consent(tx, "client-1", None, "[]")
    })
    .await;
    assert_eq!(
        resolve(&db, None).await,
        Resolution::WorkspaceSelectionRequired(vec![
            (WS, "Workspace".to_string()),
            (WS2, "Second workspace".to_string()),
        ])
    );
}

#[tokio::test]
async fn bind_consent_workspace_checks_admin_access_before_writing() {
    let db = base().await;
    setup(&db, |tx| {
        member(tx, WS2, "agent", "{}")?;
        consent(tx, "client-1", None, "[]")
    })
    .await;
    // Not an admin of WS2: refused, nothing written.
    let error = bind_consent_workspace(&db, USER, "client-1", WS2)
        .await
        .unwrap_err();
    assert_eq!(
        error.message(),
        "You must be an admin of the selected workspace"
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "OAuthConsent" WHERE "workspaceId" IS NOT NULL"#
        )
        .await,
        0
    );

    // Scheduled for deletion: refused.
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    assert!(
        bind_consent_workspace(&db, USER, "client-1", WS)
            .await
            .is_err()
    );
    setup(&db, |tx| {
        tx.execute(r#"UPDATE "Workspace" SET "scheduledDeletionAt" = NULL"#, [])?;
        Ok(())
    })
    .await;

    bind_consent_workspace(&db, USER, "client-1", WS)
        .await
        .unwrap();
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "OAuthConsent" WHERE "workspaceId" = 2001"#
        )
        .await,
        1
    );

    // No consent row at all: 404.
    let error = bind_consent_workspace(&db, USER, "other", WS)
        .await
        .unwrap_err();
    assert_eq!(error.message(), "OAuth consent not found");
}

#[tokio::test]
async fn grants_keep_all_scopes_and_fall_back_to_the_client_id() {
    let db = base().await;
    setup(&db, |tx| {
        client(tx, "nameless", false, None)?;
        consent(
            tx,
            "client-1",
            Some(WS),
            r#"["openid","offline_access","openid"]"#,
        )?;
        consent(tx, "nameless", Some(WS), r#"["email"]"#)
    })
    .await;
    let grants = list_grants_for_user(&db, USER, 0).await.unwrap();
    let one = grants.iter().find(|g| g.client_id == "client-1").unwrap();
    assert_eq!(one.client_name, "Claude");
    assert_eq!(one.scopes, vec!["openid", "offline_access"]);
    let two = grants.iter().find(|g| g.client_id == "nameless").unwrap();
    assert_eq!(two.client_name, "nameless");
}

#[tokio::test]
async fn revoke_deletes_consents_and_tokens_and_purge_drops_expired_tokens() {
    let db = base().await;
    setup(&db, |tx| {
        consent(tx, "client-1", Some(WS), "[]")?;
        for (token, expires) in [("live", i64::MAX / 2), ("old", 1)] {
            tx.execute(
                r#"INSERT INTO "OAuthRefreshToken" ("token", "clientId", "userId", "scopes", "expiresAt") VALUES (?1, 'client-1', ?2, '[]', ?3)"#,
                params![token, USER, expires],
            )?;
        }
        Ok(())
    })
    .await;
    assert_eq!(purge_expired_tokens(&db, 1000).await.unwrap(), 1);
    assert!(revoke_grant(&db, USER, "client-1").await.unwrap());
    assert_eq!(
        count(&db, r#"SELECT count(*) FROM "OAuthConsent""#).await,
        0
    );
    assert_eq!(
        count(&db, r#"SELECT count(*) FROM "OAuthRefreshToken""#).await,
        0
    );
    assert!(!revoke_grant(&db, USER, "client-1").await.unwrap());
}
