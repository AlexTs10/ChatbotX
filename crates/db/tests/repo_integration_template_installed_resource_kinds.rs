//! Port of `packages/database/__tests__/integration/template-installed-resource-kinds.test.ts`
//! (P3.3). The TS seeds inside a rolled-back Postgres transaction; each port seeds a fresh
//! migrated SQLite file.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo;
use chatbotx_db::repo::template_installation_sync::list_installed_resources;
use chatbotx_db::repo::template_installed_resource::{
    ListPageInput, UnknownResourceKind, list_page, list_unknown_resource_kinds,
};
use chatbotx_db::rusqlite::params;
use repo_common::{ok, setup};

const OWNER: i64 = 1001;
const WS: i64 = 2001;
const INSTALLATION: i64 = 7001;

/// The TS `seed`: an owner, a workspace, an installation and three installed rows (`flow`,
/// an unknown kind, `tag`).
async fn seeded() -> common::TestDb {
    let db = common::fresh_db().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (?1, 'kinds@example.test')"#,
            params![OWNER],
        )?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'kinds', ?2)"#,
            params![WS, OWNER],
        )?;
        tx.execute(
            r#"INSERT INTO "TemplateInstallation" ("id", "workspaceId", "templateName", "sourceWorkspaceId", "formatVersion", "permissions")
               VALUES (?1, ?2, 'Kinds', ?2, 1, '{"allowEdit":true,"allowDelete":true}')"#,
            params![INSTALLATION, WS],
        )?;
        for (n, (kind, resource_id)) in [
            ("flow", 1001),
            ("legacyKindFromANewerBuild", 1002),
            ("tag", 1003),
        ]
        .into_iter()
        .enumerate()
        {
            tx.execute(
                r#"INSERT INTO "TemplateInstalledResource" ("id", "installationId", "workspaceId", "category", "resourceKind", "resourceId", "sourceResourceId")
                   VALUES (?1, ?2, ?3, 'flows', ?4, ?5, ?6)"#,
                params![
                    8001 + n as i64,
                    INSTALLATION,
                    WS,
                    kind,
                    resource_id,
                    format!("src-{resource_id}")
                ],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

fn sorted_kinds(rows: &[chatbotx_db::repo::models::TemplateInstalledResource]) -> Vec<String> {
    let mut kinds: Vec<String> = rows.iter().map(|r| r.resource_kind.clone()).collect();
    kinds.sort();
    kinds
}

/// Port of: packages/database/__tests__/integration/template-installed-resource-kinds.test.ts:86 "template installed resource kinds > readers skip an unknown kind instead of crashing, and keep paginating"
#[tokio::test]
async fn template_installed_resource_kinds_readers_skip_an_unknown_kind_instead_of_crashing_and_keep_paginating()
 {
    let db = seeded().await;

    let synced = ok(repo::read(&db, |c| list_installed_resources(c, INSTALLATION)).await);
    assert_eq!(sorted_kinds(&synced), vec!["flow", "tag"]);

    let input = ListPageInput {
        installation_id: INSTALLATION,
        workspace_id: WS,
        limit: 2,
        ..ListPageInput::default()
    };
    let first_input = input.clone();
    let first = ok(repo::read(&db, move |c| list_page(c, &first_input)).await);
    assert!(first.rows.len() <= 2);
    assert!(first.next_cursor.is_some());
    let second_input = ListPageInput {
        after_id: first.next_cursor,
        ..input
    };
    let second = ok(repo::read(&db, move |c| list_page(c, &second_input)).await);
    assert_eq!(second.next_cursor, None);
    let all: Vec<_> = first.rows.iter().chain(&second.rows).cloned().collect();
    assert_eq!(sorted_kinds(&all), vec!["flow", "tag"]);
}

/// Port of: packages/database/__tests__/integration/template-installed-resource-kinds.test.ts:121 "template installed resource kinds > the preflight query reports the unknown kind with its count"
#[tokio::test]
async fn template_installed_resource_kinds_the_preflight_query_reports_the_unknown_kind_with_its_count()
 {
    let db = seeded().await;

    let unknown = ok(repo::read(&db, list_unknown_resource_kinds).await);

    assert!(unknown.contains(&UnknownResourceKind {
        resource_kind: "legacyKindFromANewerBuild".to_owned(),
        rows: 1,
    }));
    assert!(!unknown.iter().any(|r| r.resource_kind == "flow"));
}
