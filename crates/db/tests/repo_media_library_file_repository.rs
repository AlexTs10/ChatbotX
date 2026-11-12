//! Port of `packages/database/__tests__/media-library-file-repository.test.ts` (P3.3).
//! The mocked select/delete/update/`$count` chains become real `MediaLibraryFile` rows:
//! the asserted `eq`/`and`/`isNull` conditions are checked as rows found, changed or left
//! alone (decoys: another workspace, another folder, non-favourites).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::media_library_file::{
    ListFilesInput, delete_by_id, find_by_id, find_by_path, list, set_favourite,
};
use chatbotx_db::repo::models::MediaLibraryFile;
use chatbotx_db::repo::{self, find_one};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;

/// Files of 2001: 1 root, 2 root favourite, 3 in folder 51, 4 in folder 52 favourite;
/// file 9 of 2002 (path `a/b.png` too).
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1009, OTHER).await;
    setup(&db, |tx| {
        ins!(tx, "MediaLibraryFolder", { "id" => 51_i64, "name" => "A", "workspaceId" => WS })?;
        ins!(tx, "MediaLibraryFolder", { "id" => 52_i64, "name" => "B", "workspaceId" => WS })?;
        for (id, ws, folder, fav, path, created) in [
            (1_i64, WS, None, false, "a/b.png", 1_000_i64),
            (2, WS, None, true, "root-2.png", 2_000),
            (3, WS, Some(51_i64), false, "f/3.png", 3_000),
            (4, WS, Some(52), true, "f/4.png", 4_000),
            (9, OTHER, None, false, "a/b.png", 5_000),
        ] {
            ins!(tx, "MediaLibraryFile", {
                "id" => id, "createdAt" => created, "name" => format!("file {id}"), "path" => path,
                "mimeType" => "image/png", "size" => 1_i64, "isFavourite" => fav,
                "workspaceId" => ws, "folderId" => folder,
            })?;
        }
        Ok(())
    })
    .await;
    db
}

async fn row(db: &common::TestDb, id: i64) -> Option<MediaLibraryFile> {
    ok(repo::read(db, move |c| find_one(c, "\"id\" = ?1", &[&id])).await)
}

fn list_input(filter: Option<&str>, folder_id: Option<i64>) -> ListFilesInput {
    ListFilesInput {
        workspace_id: WS,
        filter: filter.map(str::to_owned),
        folder_id,
        per_page: 50,
        ..Default::default()
    }
}

async fn listed(db: &common::TestDb, input: ListFilesInput) -> (Vec<i64>, i64) {
    let page = ok(repo::read(db, move |c| list(c, &input)).await);
    (page.data.iter().map(|f| f.id).collect(), page.total)
}

/// Port of: packages/database/__tests__/media-library-file-repository.test.ts:93 "mediaLibraryFileRepository.findById > scopes the where clause by workspaceId AND id"
#[tokio::test]
async fn media_library_file_repository_find_by_id_scopes_the_where_clause_by_workspace_id_and_id() {
    let db = seeded().await;
    let found = ok(repo::read(&db, |c| find_by_id(c, WS, 1)).await).expect("file");
    assert_eq!((found.id, found.workspace_id), (1, WS));
}

/// Port of: packages/database/__tests__/media-library-file-repository.test.ts:110 "mediaLibraryFileRepository.findById > returns null for another workspace's file id"
#[tokio::test]
async fn media_library_file_repository_find_by_id_returns_null_for_another_workspace_s_file_id() {
    let db = seeded().await;
    assert_eq!(ok(repo::read(&db, |c| find_by_id(c, OTHER, 1)).await), None);
}

/// Port of: packages/database/__tests__/media-library-file-repository.test.ts:124 "mediaLibraryFileRepository.findByPath > scopes the where clause by workspaceId AND path"
#[tokio::test]
async fn media_library_file_repository_find_by_path_scopes_the_where_clause_by_workspace_id_and_path()
 {
    let db = seeded().await;
    let found = ok(repo::read(&db, |c| find_by_path(c, WS, "a/b.png")).await).expect("file");
    assert_eq!(
        (found.id, found.workspace_id, found.path.as_str()),
        (1, WS, "a/b.png")
    );
}

/// Port of: packages/database/__tests__/media-library-file-repository.test.ts:150 "mediaLibraryFileRepository.findByPath > returns null for another workspace's file path"
#[tokio::test]
async fn media_library_file_repository_find_by_path_returns_null_for_another_workspace_s_file_path()
{
    let db = seeded().await;
    // 2002 has its own `a/b.png` (9), but not `root-2.png`.
    assert_eq!(
        ok(repo::read(&db, |c| find_by_path(c, OTHER, "root-2.png")).await),
        None
    );
    assert_eq!(
        ok(repo::read(&db, |c| find_by_path(c, OTHER, "a/b.png")).await).map(|f| f.id),
        Some(9)
    );
}

/// Port of: packages/database/__tests__/media-library-file-repository.test.ts:169 "mediaLibraryFileRepository.deleteById > scopes the delete by id AND workspaceId"
#[tokio::test]
async fn media_library_file_repository_delete_by_id_scopes_the_delete_by_id_and_workspace_id() {
    let db = seeded().await;
    ok(repo::write(&db, |tx| delete_by_id(tx, OTHER, 1)).await);
    assert!(row(&db, 1).await.is_some());
    ok(repo::write(&db, |tx| delete_by_id(tx, WS, 1)).await);
    assert_eq!(row(&db, 1).await, None);
    assert!(row(&db, 9).await.is_some());
}

/// Port of: packages/database/__tests__/media-library-file-repository.test.ts:188 "mediaLibraryFileRepository.setFavourite > scopes the update by id AND workspaceId"
#[tokio::test]
async fn media_library_file_repository_set_favourite_scopes_the_update_by_id_and_workspace_id() {
    let db = seeded().await;
    ok(repo::write(&db, |tx| set_favourite(tx, OTHER, 1, true)).await);
    assert!(!row(&db, 1).await.expect("file").is_favourite);
    ok(repo::write(&db, |tx| set_favourite(tx, WS, 1, true)).await);
    assert!(row(&db, 1).await.expect("file").is_favourite);
    assert!(!row(&db, 3).await.expect("file").is_favourite);
}

/// Port of: packages/database/__tests__/media-library-file-repository.test.ts:209 "mediaLibraryFileRepository.list > returns data and total scoped by the same where clause"
#[tokio::test]
async fn media_library_file_repository_list_returns_data_and_total_scoped_by_the_same_where_clause()
{
    let db = seeded().await;
    // workspace 2001 AND folderId IS NULL; newest first.
    assert_eq!(listed(&db, list_input(None, None)).await, (vec![2, 1], 2));
}

/// Port of: packages/database/__tests__/media-library-file-repository.test.ts:228 "mediaLibraryFileRepository.list > scopes to root-level files when no filter and no folderId are given"
#[tokio::test]
async fn media_library_file_repository_list_scopes_to_root_level_files_when_no_filter_and_no_folder_id_are_given()
 {
    let db = seeded().await;
    let (ids, _) = listed(&db, list_input(None, None)).await;
    assert!(!ids.contains(&3) && !ids.contains(&4));
    // With a folder id, that folder only.
    assert_eq!(listed(&db, list_input(None, Some(51))).await, (vec![3], 1));
}

/// Port of: packages/database/__tests__/media-library-file-repository.test.ts:237 "mediaLibraryFileRepository.list > filter 'favourite' ignores folderId and scopes by isFavourite instead"
#[tokio::test]
async fn media_library_file_repository_list_filter_favourite_ignores_folder_id_and_scopes_by_is_favourite_instead()
 {
    let db = seeded().await;
    assert_eq!(
        listed(&db, list_input(Some("favourite"), Some(51))).await,
        (vec![4, 2], 2)
    );
}

/// Port of: packages/database/__tests__/media-library-file-repository.test.ts:259 "mediaLibraryFileRepository.list > filter 'all' spans every folder with no folder condition"
#[tokio::test]
async fn media_library_file_repository_list_filter_all_spans_every_folder_with_no_folder_condition()
{
    let db = seeded().await;
    assert_eq!(
        listed(&db, list_input(Some("all"), None)).await,
        (vec![4, 3, 2, 1], 4)
    );
}
