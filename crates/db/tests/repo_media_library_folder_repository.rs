//! Port of `packages/database/__tests__/media-library-folder-repository.test.ts` (P3.3).
//! The mocked query/insert/update/delete become real `MediaLibraryFolder` rows.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::media_library_folder::{create, delete_by_id, list_by_workspace, rename};
use chatbotx_db::repo::models::MediaLibraryFolder;
use chatbotx_db::repo::{self, find_one};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;

async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1009, OTHER).await;
    setup(&db, |tx| {
        ins!(tx, "MediaLibraryFolder", { "id" => 1_i64, "name" => "Logos", "workspaceId" => WS })?;
        ins!(tx, "MediaLibraryFolder", { "id" => 2_i64, "name" => "Banners", "workspaceId" => WS })?;
        ins!(tx, "MediaLibraryFolder", { "id" => 3_i64, "name" => "Alien", "workspaceId" => OTHER })?;
        Ok(())
    })
    .await;
    db
}

async fn folder(db: &common::TestDb, id: i64) -> Option<MediaLibraryFolder> {
    ok(repo::read(db, move |c| find_one(c, "\"id\" = ?1", &[&id])).await)
}

/// Port of: packages/database/__tests__/media-library-folder-repository.test.ts:47 "mediaLibraryFolderRepository.listByWorkspace > scopes the query to the given workspace and orders by name asc"
#[tokio::test]
async fn media_library_folder_repository_list_by_workspace_scopes_the_query_to_the_given_workspace_and_orders_by_name_asc()
 {
    let db = seeded().await;
    let rows = ok(repo::read(&db, |c| list_by_workspace(c, WS)).await);
    assert_eq!(
        rows.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        vec!["Banners", "Logos"]
    );
}

/// Port of: packages/database/__tests__/media-library-folder-repository.test.ts:63 "mediaLibraryFolderRepository.create > inserts the given values and returns the created row"
#[tokio::test]
async fn media_library_folder_repository_create_inserts_the_given_values_and_returns_the_created_row()
 {
    let db = seeded().await;
    let created = ok(repo::write(&db, |tx| create(tx, Some(10), WS, "New")).await);
    assert_eq!(
        (created.id, created.workspace_id, created.name.as_str()),
        (10, WS, "New")
    );
    assert_eq!(folder(&db, 10).await, Some(created));
}

/// Port of: packages/database/__tests__/media-library-folder-repository.test.ts:85 "mediaLibraryFolderRepository.rename > scopes the update to both folderId and workspaceId"
#[tokio::test]
async fn media_library_folder_repository_rename_scopes_the_update_to_both_folder_id_and_workspace_id()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| rename(tx, OTHER, 1, "Hijack")).await);
    assert_eq!(
        folder(&db, 1).await.map(|f| f.name).as_deref(),
        Some("Logos")
    );
    ok(repo::write(&db, |tx| rename(tx, WS, 1, "Renamed")).await);
    assert_eq!(
        folder(&db, 1).await.map(|f| f.name).as_deref(),
        Some("Renamed")
    );
    assert_eq!(
        folder(&db, 2).await.map(|f| f.name).as_deref(),
        Some("Banners")
    );
}

/// Port of: packages/database/__tests__/media-library-folder-repository.test.ts:103 "mediaLibraryFolderRepository.deleteById > scopes the delete to both folderId and workspaceId"
#[tokio::test]
async fn media_library_folder_repository_delete_by_id_scopes_the_delete_to_both_folder_id_and_workspace_id()
 {
    let db = seeded().await;
    ok(repo::write(&db, |tx| delete_by_id(tx, OTHER, 1)).await);
    assert!(folder(&db, 1).await.is_some());
    ok(repo::write(&db, |tx| delete_by_id(tx, WS, 1)).await);
    assert_eq!(folder(&db, 1).await, None);
    assert!(folder(&db, 2).await.is_some());
}
