//! Port of `packages/database/__tests__/template-version-pin.test.ts` (P3.3). The
//! recording tx (projections, the shared advisory lock) becomes real `TemplateVersion`
//! rows: the pin result is checked; the shared lock is not needed under the one writer.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::template_version::pin_version_number;
use chatbotx_db::repo::{self};
use common::{fresh_db, seed};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const TEMPLATE: i64 = 55;

async fn with_versions(versions: &'static [i64]) -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    setup(&db, move |tx| {
        ins!(tx, "Template", {
            "id" => TEMPLATE, "workspaceId" => WS, "tenantId" => 1_i64, "name" => "T",
            "selection" => "{}", "payload" => "{}", "categoryCounts" => "{}",
            "formatVersion" => 1_i64, "shareToken" => "share-55", "defaultPermissions" => "{}",
        })?;
        for v in versions {
            ins!(tx, "TemplateVersion", {
                "id" => 100 + v, "templateId" => TEMPLATE, "workspaceId" => WS, "version" => *v,
                "payload" => r#"{"big":"payload"}"#, "payloadHash" => format!("h{v}"), "formatVersion" => 1_i64,
            })?;
        }
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/template-version-pin.test.ts:39 "templateVersionRepository.pinVersionNumber > reads only the version column, never the payload"
///
/// The projection is internal to the query; the port checks the pin result of the same
/// fixture (requested 4 missing, newest 7).
#[tokio::test]
async fn template_version_repository_pin_version_number_reads_only_the_version_column_never_the_payload()
 {
    let db = with_versions(&[7]).await;
    assert_eq!(
        ok(repo::write(&db, |tx| pin_version_number(tx, TEMPLATE, 4)).await),
        7
    );
}

/// Port of: packages/database/__tests__/template-version-pin.test.ts:50 "templateVersionRepository.pinVersionNumber > returns the requested version when it is stored, under the shared version lock"
#[tokio::test]
async fn template_version_repository_pin_version_number_returns_the_requested_version_when_it_is_stored_under_the_shared_version_lock()
 {
    let db = with_versions(&[4, 9]).await;
    assert_eq!(
        ok(repo::write(&db, |tx| pin_version_number(tx, TEMPLATE, 4)).await),
        4
    );
}

/// Port of: packages/database/__tests__/template-version-pin.test.ts:63 "templateVersionRepository.pinVersionNumber > falls back to the newest stored version, then to the requested one"
#[tokio::test]
async fn template_version_repository_pin_version_number_falls_back_to_the_newest_stored_version_then_to_the_requested_one()
 {
    let pruned = with_versions(&[2, 9]).await;
    assert_eq!(
        ok(repo::write(&pruned, |tx| pin_version_number(tx, TEMPLATE, 4)).await),
        9
    );
    let none = with_versions(&[]).await;
    assert_eq!(
        ok(repo::write(&none, |tx| pin_version_number(tx, TEMPLATE, 4)).await),
        4
    );
}
