//! Port of `packages/database/__tests__/reference-existence-repository.test.ts` (P3.3).
//! The fake tx (rows by table + recorded `where`) becomes real rows in each table: the
//! asserted workspace scope is a decoy row of the same id kind in another workspace that
//! is not reported; "no query" is an empty result.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo3.rs"]
mod repo3;

use std::collections::BTreeSet;

use chatbotx_db::repo::reference_existence::find_existing_ids;
use chatbotx_db::repo::{self};
use common::{fresh_db, seed, seed_integration, seed_workspace};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;

async fn base() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1009, OTHER).await;
    db
}

async fn find(db: &common::TestDb, kind: &'static str, ids: &[&str]) -> Vec<String> {
    let ids: Vec<String> = ids.iter().map(|s| (*s).to_owned()).collect();
    let found: BTreeSet<String> =
        ok(repo::read(db, move |c| find_existing_ids(c, WS, kind, &ids)).await);
    found.into_iter().collect()
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:95 "referenceExistenceRepository.findExistingIds > a direct kind is scoped by its own workspaceId"
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_a_direct_kind_is_scoped_by_its_own_workspace_id()
 {
    let db = base().await;
    setup(&db, |tx| {
        ins!(tx, "Product", { "id" => 1_i64, "name" => "p1", "workspaceId" => WS })?;
        ins!(tx, "Product", { "id" => 2_i64, "name" => "p2", "workspaceId" => OTHER })?;
        Ok(())
    })
    .await;
    assert_eq!(find(&db, "product", &["1", "2"]).await, vec!["1"]);
}

async fn natural_key_kind(kind: &'static str) {
    let db = base().await;
    setup(&db, move |tx| {
        for (id, ws) in [(1_i64, WS), (2_i64, OTHER)] {
            match kind {
                "tag" => ins!(tx, "Tag", { "id" => id, "name" => format!("t{id}"), "workspaceId" => ws })?,
                "pipeline" => ins!(tx, "Pipeline", { "id" => id, "workspaceId" => ws, "name" => format!("p{id}"), "type" => "custom", "referenceName" => format!("r{id}"), "settings" => "{}" })?,
                "folder" => ins!(tx, "Folder", { "id" => id, "name" => format!("f{id}"), "folderType" => "flow", "workspaceId" => ws, "paths" => "[]" })?,
                "productCategory" => ins!(tx, "ProductCategory", { "id" => id, "workspaceId" => ws, "name" => format!("c{id}") })?,
                other => panic!("{other}"),
            };
        }
        Ok(())
    })
    .await;
    assert_eq!(find(&db, kind, &["1", "2"]).await, vec!["1"]);
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:106 "referenceExistenceRepository.findExistingIds > %s (a natural-key kind a single-flow import verifies) is workspace-scoped" (tag)
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_tag_a_natural_key_kind_a_single_flow_import_verifies_is_workspace_scoped()
 {
    natural_key_kind("tag").await;
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:106 "referenceExistenceRepository.findExistingIds > %s (a natural-key kind a single-flow import verifies) is workspace-scoped" (pipeline)
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_pipeline_a_natural_key_kind_a_single_flow_import_verifies_is_workspace_scoped()
 {
    natural_key_kind("pipeline").await;
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:106 "referenceExistenceRepository.findExistingIds > %s (a natural-key kind a single-flow import verifies) is workspace-scoped" (folder)
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_folder_a_natural_key_kind_a_single_flow_import_verifies_is_workspace_scoped()
 {
    natural_key_kind("folder").await;
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:106 "referenceExistenceRepository.findExistingIds > %s (a natural-key kind a single-flow import verifies) is workspace-scoped" (productCategory)
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_product_category_a_natural_key_kind_a_single_flow_import_verifies_is_workspace_scoped()
 {
    natural_key_kind("productCategory").await;
}

/// Integration 11 (+ satellites) of 2001 and 12 of 2002, with one template/flow row each.
async fn integration_scoped() -> common::TestDb {
    let db = base().await;
    setup(&db, |tx| {
        repo3::integration_messenger(tx, 11, WS, 3101, "[]")?;
        repo3::integration_messenger(tx, 12, OTHER, 3102, "[]")?;
        repo3::integration_whatsapp(tx, 21, WS, 3201, "waba-1")?;
        repo3::integration_whatsapp(tx, 22, OTHER, 3202, "waba-2")?;
        ins!(tx, "MessengerMessageTemplate", { "id" => 9_i64, "name" => "t", "integrationMessengerId" => 11_i64, "sourceId" => "s9", "language" => "en", "category" => "c", "status" => "s" })?;
        ins!(tx, "MessengerMessageTemplate", { "id" => 10_i64, "name" => "t", "integrationMessengerId" => 12_i64, "sourceId" => "s10", "language" => "en", "category" => "c", "status" => "s" })?;
        ins!(tx, "WhatsappMessageTemplate", { "id" => 9_i64, "name" => "t", "integrationWhatsappId" => 21_i64, "sourceId" => "s9", "language" => "en", "category" => "c", "status" => "s" })?;
        ins!(tx, "WhatsappMessageTemplate", { "id" => 10_i64, "name" => "t", "integrationWhatsappId" => 22_i64, "sourceId" => "s10", "language" => "en", "category" => "c", "status" => "s" })?;
        ins!(tx, "WhatsappFlow", { "id" => 9_i64, "name" => "f", "integrationWhatsappId" => 21_i64, "sourceId" => "s9", "status" => "s" })?;
        ins!(tx, "WhatsappFlow", { "id" => 10_i64, "name" => "f", "integrationWhatsappId" => 22_i64, "sourceId" => "s10", "status" => "s" })?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:122 "referenceExistenceRepository.findExistingIds > %s is scoped through its integration's workspace" (messengerTemplate)
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_messenger_template_is_scoped_through_its_integration_s_workspace()
 {
    let db = integration_scoped().await;
    assert_eq!(
        find(&db, "messengerTemplate", &["9", "10"]).await,
        vec!["9"]
    );
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:122 "referenceExistenceRepository.findExistingIds > %s is scoped through its integration's workspace" (whatsappTemplate)
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_whatsapp_template_is_scoped_through_its_integration_s_workspace()
 {
    let db = integration_scoped().await;
    assert_eq!(find(&db, "whatsappTemplate", &["9", "10"]).await, vec!["9"]);
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:122 "referenceExistenceRepository.findExistingIds > %s is scoped through its integration's workspace" (whatsappFlow)
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_whatsapp_flow_is_scoped_through_its_integration_s_workspace()
 {
    let db = integration_scoped().await;
    assert_eq!(find(&db, "whatsappFlow", &["9", "10"]).await, vec!["9"]);
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:139 "referenceExistenceRepository.findExistingIds > a persona exists only in a messenger integration of the workspace"
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_a_persona_exists_only_in_a_messenger_integration_of_the_workspace()
 {
    let db = base().await;
    setup(&db, |tx| {
        repo3::integration_messenger(tx, 11, WS, 3101, r#"[{"id":"p1"},{"id":"p2"}]"#)?;
        repo3::integration_messenger(tx, 13, WS, 3103, "[]")?;
        repo3::integration_messenger(tx, 12, OTHER, 3102, r#"[{"id":"other"}]"#)?;
        Ok(())
    })
    .await;
    assert_eq!(
        find(&db, "messengerPersona", &["p2", "other"]).await,
        vec!["p2"]
    );
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:153 "referenceExistenceRepository.findExistingIds > an integration id may name a row of any of the integration tables"
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_an_integration_id_may_name_a_row_of_any_of_the_integration_tables()
 {
    let db = base().await;
    setup(&db, |tx| {
        repo3::inbox(tx, 3301, WS, "email")?;
        ins!(tx, "IntegrationSmtp", { "id" => 31_i64, "auth" => "{}", "name" => "smtp", "fromAddress" => "a@b.c", "workspaceId" => WS, "inboxId" => 3301_i64 })?;
        repo3::integration_messenger(tx, 32, WS, 3302, "[]")?;
        seed_integration(tx, 33, WS, "openaiCompatible")?;
        ins!(tx, "IntegrationOpenaiCompatible", { "id" => 34_i64, "baseURL" => "https://x", "defaultModel" => "m", "integrationId" => 33_i64, "name" => "o", "preset" => "custom", "workspaceId" => WS })?;
        // "foreign": the same kinds of row in another workspace.
        seed_integration(tx, 35, OTHER, "googleSheet")?;
        repo3::integration_messenger(tx, 36, OTHER, 3303, "[]")?;
        Ok(())
    })
    .await;
    assert_eq!(
        find(&db, "integration", &["31", "32", "33", "34", "35", "36"]).await,
        vec!["31", "32", "33", "34"]
    );
}

/// Port of: packages/database/__tests__/reference-existence-repository.test.ts:171 "referenceExistenceRepository.findExistingIds > an empty id list costs no query"
#[tokio::test]
async fn reference_existence_repository_find_existing_ids_an_empty_id_list_costs_no_query() {
    let db = integration_scoped().await;
    assert!(find(&db, "whatsappFlow", &[]).await.is_empty());
}
