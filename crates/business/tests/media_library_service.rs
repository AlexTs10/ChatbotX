//! Port of `packages/business/__tests__/media-library.service.test.ts`,
//! `media-library-file-service.test.ts` and `media-library-read-image.test.ts`.
//! Adapted: the mocked repositories, uploader and tenant settings become a
//! real database, an in-memory store and a storage URL argument; "called
//! with" becomes the stored rows. A failing S3 delete is a key the store
//! rejects (`..` segment).

mod common;

use std::sync::Arc;

use chatbotx_business::media_library::{self as svc, CreateFile, MEDIA_LIBRARY_FILES_PAGE_SIZE};
use chatbotx_db::repo::media_library_file::ListFilesInput;
use chatbotx_files::{LocalUrlSigner, PutOptions, Uploader};
use common::{OTHER_WS, WS, scalar_i64, scalar_text, setup, two_workspaces};

const CDN: &str = "https://cdn.example.test";

fn uploader() -> Arc<Uploader> {
    let signer = LocalUrlSigner::new("secret", "http://app/files").expect("signer");
    Arc::new(Uploader::in_memory().with_url_signer(signer))
}

// One row per call reads clearer than a builder in these tests.
#[allow(clippy::too_many_arguments)]
fn file_row(
    tx: &chatbotx_db::rusqlite::Connection,
    id: i64,
    ws: i64,
    folder: Option<i64>,
    path: &str,
    mime: &str,
    size: i64,
    favourite: bool,
) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT INTO "MediaLibraryFile" ("id", "workspaceId", "folderId", "name", "path", "mimeType", "size", "isFavourite", "createdAt", "updatedAt")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?1, ?1)"#,
        chatbotx_db::rusqlite::params![id, ws, folder, format!("f{id}"), path, mime, size, favourite],
    )?;
    Ok(())
}

fn folder_row(
    tx: &chatbotx_db::rusqlite::Connection,
    id: i64,
    ws: i64,
    name: &str,
) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT INTO "MediaLibraryFolder" ("id", "workspaceId", "name") VALUES (?1, ?2, ?3)"#,
        chatbotx_db::rusqlite::params![id, ws, name],
    )?;
    Ok(())
}

fn create_input(path: &str) -> CreateFile {
    CreateFile {
        workspace_id: WS,
        folder_id: None,
        name: "logo.png".into(),
        path: path.into(),
        mime_type: "image/png".into(),
        size: 100,
    }
}

async fn count(db: &chatbotx_db::Db, sql: &'static str, id: i64) -> i64 {
    scalar_i64(db, sql, id).await.unwrap_or(0)
}

// ---- createFile ------------------------------------------------------------

#[tokio::test]
async fn media_library_service_create_file_rejects_a_path_outside_both_workspace_prefixes() {
    let db = two_workspaces().await;
    let err = svc::create_file(
        &db,
        CDN,
        create_input(&format!("someone-elses/{OTHER_WS}/logo.png")),
    )
    .await
    .expect_err("invalid path");
    assert_eq!((err.code(), err.status()), (Some("invalidPath"), Some(400)));
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "MediaLibraryFile" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        0
    );
}

#[tokio::test]
async fn media_library_service_create_file_accepts_a_workspaces_id_scoped_path() {
    let db = two_workspaces().await;
    let path = format!("workspaces/{WS}/logo.png");
    let out = svc::create_file(&db, CDN, create_input(&path))
        .await
        .expect("create");
    assert_eq!(
        (out.file.workspace_id, out.file.path.as_str()),
        (WS, path.as_str())
    );
}

#[tokio::test]
async fn media_library_service_create_file_accepts_a_public_space_id_scoped_path() {
    let db = two_workspaces().await;
    let path = format!("public/space/{WS}/logo.png");
    svc::create_file(&db, CDN, create_input(&path))
        .await
        .expect("create");
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "MediaLibraryFile" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        1
    );
}

#[tokio::test]
async fn media_library_service_create_file_returns_the_created_file_with_a_resolved_public_url() {
    let db = two_workspaces().await;
    let path = format!("public/space/{WS}/logo.png");
    let out = svc::create_file(&db, CDN, create_input(&path))
        .await
        .expect("create");
    assert_eq!(out.url, format!("{CDN}/public/space/{WS}/logo.png"));
    assert_eq!(out.file.path, path);
}

// ---- deleteFolder ----------------------------------------------------------

#[tokio::test]
async fn media_library_service_delete_folder_swallows_an_s3_delete_failure_and_still_deletes_both_tables_inside_one_transaction()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        folder_row(tx, 1, WS, "A")?;
        // A key the store rejects: its delete fails.
        file_row(
            tx,
            10,
            WS,
            Some(1),
            "workspaces/2001/../a.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    let store = uploader();
    svc::delete_folder(&db, Some(&store), WS, 1)
        .await
        .expect("delete");
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "MediaLibraryFile" WHERE "id" = ?1"#,
            10
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "MediaLibraryFolder" WHERE "id" = ?1"#,
            1
        )
        .await,
        0
    );
}

#[tokio::test]
async fn media_library_service_delete_folder_deletes_files_and_the_folder_in_the_same_transaction_on_the_happy_path()
 {
    let db = two_workspaces().await;
    let store = uploader();
    store
        .put_object(
            "workspaces/2001/a.png",
            vec![1u8],
            PutOptions::default(),
            None,
        )
        .await
        .expect("put");
    setup(&db, |tx| {
        folder_row(tx, 1, WS, "A")?;
        file_row(
            tx,
            10,
            WS,
            Some(1),
            "workspaces/2001/a.png",
            "image/png",
            1,
            false,
        )?;
        file_row(
            tx,
            11,
            WS,
            None,
            "workspaces/2001/root.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    svc::delete_folder(&db, Some(&store), WS, 1)
        .await
        .expect("delete");
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "MediaLibraryFolder" WHERE "id" = ?1"#,
            1
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "MediaLibraryFile" WHERE "id" = ?1"#,
            10
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "MediaLibraryFile" WHERE "id" = ?1"#,
            11
        )
        .await,
        1
    );
    assert!(store.get_object("workspaces/2001/a.png").await.is_err());
}

// ---- deleteFile ------------------------------------------------------------

#[tokio::test]
async fn media_library_service_delete_file_throws_not_found_when_the_file_does_not_exist() {
    let db = two_workspaces().await;
    let err = svc::delete_file(&db, None, WS, 404)
        .await
        .expect_err("missing");
    assert_eq!(err.code(), Some("notFound"));
}

#[tokio::test]
async fn media_library_service_delete_file_best_effort_deletes_the_s3_object_then_the_row() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        file_row(
            tx,
            10,
            WS,
            None,
            "workspaces/2001/../a.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    let store = uploader();
    svc::delete_file(&db, Some(&store), WS, 10)
        .await
        .expect("delete");
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "MediaLibraryFile" WHERE "id" = ?1"#,
            10
        )
        .await,
        0
    );
}

// ---- toggleFavourite / setFavourite / findFile -------------------------------

#[tokio::test]
async fn media_library_service_toggle_favourite_throws_not_found_when_the_file_does_not_exist() {
    let db = two_workspaces().await;
    let err = svc::toggle_favourite(&db, CDN, WS, 404)
        .await
        .expect_err("missing");
    assert_eq!(err.code(), Some("notFound"));
}

async fn favourite_of(db: &chatbotx_db::Db, id: i64) -> i64 {
    count(
        db,
        r#"SELECT "isFavourite" FROM "MediaLibraryFile" WHERE "id" = ?1"#,
        id,
    )
    .await
}

#[tokio::test]
async fn media_library_service_toggle_favourite_flips_is_favourite_from_false_to_true() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        file_row(
            tx,
            10,
            WS,
            None,
            "workspaces/2001/a.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    svc::toggle_favourite(&db, CDN, WS, 10)
        .await
        .expect("toggle");
    assert_eq!(favourite_of(&db, 10).await, 1);
}

#[tokio::test]
async fn media_library_service_toggle_favourite_flips_is_favourite_from_true_to_false() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        file_row(
            tx,
            10,
            WS,
            None,
            "workspaces/2001/a.png",
            "image/png",
            1,
            true,
        )
    })
    .await;
    svc::toggle_favourite(&db, CDN, WS, 10)
        .await
        .expect("toggle");
    assert_eq!(favourite_of(&db, 10).await, 0);
}

#[tokio::test]
async fn media_library_service_set_favourite_throws_not_found_when_the_file_does_not_exist() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        file_row(
            tx,
            10,
            OTHER_WS,
            None,
            "workspaces/2002/a.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    let err = svc::set_favourite(&db, CDN, WS, 10, true)
        .await
        .expect_err("missing");
    assert_eq!(err.code(), Some("notFound"));
    assert_eq!(favourite_of(&db, 10).await, 0);
}

#[tokio::test]
async fn media_library_service_set_favourite_sets_the_explicit_value_regardless_of_the_file_s_current_state()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        file_row(
            tx,
            10,
            WS,
            None,
            "workspaces/2001/a.png",
            "image/png",
            1,
            true,
        )
    })
    .await;
    let out = svc::set_favourite(&db, CDN, WS, 10, true)
        .await
        .expect("set");
    assert_eq!(favourite_of(&db, 10).await, 1);
    assert!(out.file.is_favourite);
    assert_eq!(out.url, format!("{CDN}/workspaces/2001/a.png"));
}

#[tokio::test]
async fn media_library_service_find_file_throws_not_found_when_the_file_does_not_exist() {
    let db = two_workspaces().await;
    let err = svc::find_file(&db, CDN, WS, 404)
        .await
        .expect_err("missing");
    assert_eq!(err.code(), Some("notFound"));
}

#[tokio::test]
async fn media_library_service_find_file_returns_the_file_scoped_to_the_workspace_with_a_resolved_url()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        file_row(
            tx,
            10,
            WS,
            None,
            "workspaces/2001/a.png",
            "image/png",
            1,
            false,
        )?;
        file_row(
            tx,
            11,
            OTHER_WS,
            None,
            "workspaces/2002/a.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    let out = svc::find_file(&db, CDN, WS, 10).await.expect("find");
    assert_eq!(out.url, format!("{CDN}/workspaces/2001/a.png"));
    assert!(svc::find_file(&db, CDN, WS, 11).await.is_err());
}

// ---- presignUpload ---------------------------------------------------------

#[tokio::test]
async fn media_library_service_presign_upload_derives_a_workspace_scoped_key_and_records_a_pending_file_with_a_null_user_id()
 {
    let db = two_workspaces().await;
    let store = uploader();
    let out = svc::presign_upload(&db, &store, CDN, WS, "cat.png", "image/png")
        .await
        .expect("presign");
    let prefix = format!("public/space/{WS}/media-library/");
    let key = out.path.strip_prefix(&prefix).expect("prefix");
    assert!(!key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()));
    assert!(
        out.upload_url
            .starts_with(&format!("http://app/files/{}?", out.path))
    );
    assert!(out.upload_url.contains("X-Amz-Expires=300"));
    assert_eq!(out.public_url, format!("{CDN}/{}", out.path));
    let pending = db.read({
        let path = out.path.clone();
        move |conn| {
            Ok(conn.query_row(
                r#"SELECT "workspaceId", "userId", "contextType", "subType", "fileName", "mimeType", "status" FROM "File" WHERE "path" = ?1"#,
                [path],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                    ))
                },
            )?)
        }
    })
    .await
    .expect("pending file");
    assert_eq!(
        pending,
        (
            WS,
            None,
            "generic".into(),
            "generic".into(),
            "cat.png".into(),
            "image/png".into(),
            "pending".into()
        )
    );
}

// ---- listFolders / createFolder / renameFolder -------------------------------

#[tokio::test]
async fn media_library_service_list_folders_scopes_both_the_folder_list_and_the_grouped_file_count_to_workspace_id()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        folder_row(tx, 1, OTHER_WS, "Foreign")?;
        file_row(
            tx,
            10,
            OTHER_WS,
            Some(1),
            "workspaces/2002/a.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    assert!(svc::list_folders(&db, WS).await.expect("list").is_empty());
}

#[tokio::test]
async fn media_library_service_list_folders_merges_the_matching_file_count_onto_each_folder() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        folder_row(tx, 1, WS, "A")?;
        folder_row(tx, 2, WS, "B")?;
        for id in 10..13 {
            file_row(
                tx,
                id,
                WS,
                Some(1),
                "workspaces/2001/a.png",
                "image/png",
                1,
                false,
            )?;
        }
        Ok(())
    })
    .await;
    let out: Vec<(i64, String, i64)> = svc::list_folders(&db, WS)
        .await
        .expect("list")
        .into_iter()
        .map(|row| (row.folder.id, row.folder.name, row.file_count))
        .collect();
    assert_eq!(out, vec![(1, "A".into(), 3), (2, "B".into(), 0)]);
}

#[tokio::test]
async fn media_library_service_list_folders_defaults_file_count_to_0_for_a_folder_missing_from_the_grouped_counts()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| folder_row(tx, 1, WS, "Empty")).await;
    let out = svc::list_folders(&db, WS).await.expect("list");
    assert_eq!((out.len(), out[0].file_count), (1, 0));
}

#[tokio::test]
async fn media_library_service_list_folders_ignores_a_count_row_whose_folder_is_not_in_the_workspace_list()
 {
    let db = two_workspaces().await;
    // Root files make a `folderId` NULL count row; no folder matches it.
    setup(&db, |tx| {
        file_row(
            tx,
            10,
            WS,
            None,
            "workspaces/2001/a.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    assert!(svc::list_folders(&db, WS).await.expect("list").is_empty());
}

#[tokio::test]
async fn media_library_service_create_folder_mints_the_id_and_scopes_the_row_to_the_workspace() {
    let db = two_workspaces().await;
    let folder = svc::create_folder(&db, WS, "Campaign assets")
        .await
        .expect("create");
    assert!(folder.id > 0);
    assert_eq!(
        (folder.workspace_id, folder.name.as_str()),
        (WS, "Campaign assets")
    );
}

#[tokio::test]
async fn media_library_service_rename_folder_scopes_the_rename_by_workspace_id_so_a_foreign_folder_id_matches_nothing()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| folder_row(tx, 1, OTHER_WS, "Theirs")).await;
    svc::rename_folder(&db, WS, 1, "Renamed")
        .await
        .expect("rename");
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "name" FROM "MediaLibraryFolder" WHERE "id" = ?1"#,
            1
        )
        .await
        .as_deref(),
        Some("Theirs")
    );
    setup(&db, |tx| folder_row(tx, 2, WS, "Mine")).await;
    svc::rename_folder(&db, WS, 2, "Renamed")
        .await
        .expect("rename");
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "name" FROM "MediaLibraryFolder" WHERE "id" = ?1"#,
            2
        )
        .await
        .as_deref(),
        Some("Renamed")
    );
}

// ---- moveFiles / recordFileAccess --------------------------------------------

async fn folder_of(db: &chatbotx_db::Db, id: i64) -> Option<i64> {
    scalar_i64(
        db,
        r#"SELECT "folderId" FROM "MediaLibraryFile" WHERE "id" = ?1"#,
        id,
    )
    .await
}

#[tokio::test]
async fn media_library_service_move_files_passes_the_target_folder_through() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        folder_row(tx, 9, WS, "Target")?;
        file_row(
            tx,
            10,
            WS,
            None,
            "workspaces/2001/a.png",
            "image/png",
            1,
            false,
        )?;
        file_row(
            tx,
            11,
            WS,
            None,
            "workspaces/2001/b.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    svc::move_files(&db, WS, vec![10, 11], Some(9))
        .await
        .expect("move");
    assert_eq!(
        (folder_of(&db, 10).await, folder_of(&db, 11).await),
        (Some(9), Some(9))
    );
}

#[tokio::test]
async fn media_library_service_move_files_normalises_an_omitted_folder_id_to_null_so_files_move_to_the_root()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        folder_row(tx, 9, WS, "Target")?;
        file_row(
            tx,
            10,
            WS,
            Some(9),
            "workspaces/2001/a.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    svc::move_files(&db, WS, vec![10], None)
        .await
        .expect("move");
    assert_eq!(folder_of(&db, 10).await, None);
}

#[tokio::test]
async fn media_library_service_record_file_access_scopes_the_touch_by_workspace_id() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        file_row(
            tx,
            10,
            WS,
            None,
            "workspaces/2001/a.png",
            "image/png",
            1,
            false,
        )?;
        file_row(
            tx,
            11,
            OTHER_WS,
            None,
            "workspaces/2002/a.png",
            "image/png",
            1,
            false,
        )
    })
    .await;
    svc::record_file_access(&db, WS, 10).await.expect("touch");
    svc::record_file_access(&db, WS, 11)
        .await
        .expect("touch foreign");
    let at = |id| {
        scalar_i64(
            &db,
            r#"SELECT "lastAccessedAt" FROM "MediaLibraryFile" WHERE "id" = ?1"#,
            id,
        )
    };
    assert!(at(10).await.is_some());
    assert!(at(11).await.is_none());
}

// ---- mediaLibraryFileService -------------------------------------------------

fn list_input(per_page: i64) -> ListFilesInput {
    ListFilesInput {
        workspace_id: WS,
        filter: Some("all".into()),
        per_page,
        ..ListFilesInput::default()
    }
}

#[tokio::test]
async fn media_library_file_service_list_passes_the_page_size_and_input_through_to_the_repository()
{
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for id in 1..=61 {
            file_row(
                tx,
                id,
                WS,
                None,
                "workspaces/2001/a.png",
                "image/png",
                1,
                false,
            )?;
        }
        Ok(())
    })
    .await;
    // `perPage` unset (0) = `MEDIA_LIBRARY_FILES_PAGE_SIZE`.
    let out = svc::list_files(&db, CDN, list_input(0))
        .await
        .expect("list");
    assert_eq!(out.data.len() as i64, MEDIA_LIBRARY_FILES_PAGE_SIZE);
    assert_eq!(out.page_count, 2);
}

#[tokio::test]
async fn media_library_file_service_list_passes_an_explicit_per_page_through_instead_of_the_default_page_size()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for id in 1..=12 {
            file_row(
                tx,
                id,
                WS,
                None,
                "workspaces/2001/a.png",
                "image/png",
                1,
                false,
            )?;
        }
        Ok(())
    })
    .await;
    let out = svc::list_files(&db, CDN, list_input(10))
        .await
        .expect("list");
    assert_eq!(out.data.len(), 10);
}

#[tokio::test]
async fn media_library_file_service_list_maps_each_row_s_path_and_the_workspace_s_storage_url_into_a_public_url()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        file_row(tx, 1, WS, None, "ws/1/a.png", "image/png", 1, false)?;
        file_row(tx, 2, WS, None, "ws/1/b.png", "image/png", 1, false)
    })
    .await;
    let out = svc::list_files(&db, CDN, list_input(0))
        .await
        .expect("list");
    let urls: Vec<(i64, String)> = out.data.into_iter().map(|f| (f.file.id, f.url)).collect();
    assert_eq!(
        urls,
        vec![
            (2, format!("{CDN}/ws/1/b.png")),
            (1, format!("{CDN}/ws/1/a.png"))
        ]
    );
}

#[tokio::test]
async fn media_library_file_service_list_derives_page_count_from_the_repository_s_total_and_the_effective_per_page()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for id in 1..=25 {
            file_row(
                tx,
                id,
                WS,
                None,
                "workspaces/2001/a.png",
                "image/png",
                1,
                false,
            )?;
        }
        Ok(())
    })
    .await;
    let out = svc::list_files(&db, CDN, list_input(10))
        .await
        .expect("list");
    assert_eq!(out.page_count, 3);
}

#[tokio::test]
async fn media_library_file_service_find_by_path_scopes_the_lookup_by_both_workspace_id_and_path() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        file_row(tx, 1, WS, None, "ws/1/a.png", "image/png", 1, false)?;
        file_row(tx, 2, OTHER_WS, None, "ws/2/a.png", "image/png", 1, false)
    })
    .await;
    assert_eq!(
        svc::find_by_path(&db, WS, "ws/1/a.png")
            .await
            .expect("find")
            .map(|f| f.id),
        Some(1)
    );
    assert!(
        svc::find_by_path(&db, WS, "ws/2/a.png")
            .await
            .expect("find")
            .is_none()
    );
}

#[tokio::test]
async fn media_library_file_service_find_by_id_scopes_the_lookup_by_both_workspace_id_and_id() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        file_row(tx, 2, OTHER_WS, None, "ws/2/a.png", "image/png", 1, false)
    })
    .await;
    assert_eq!(
        svc::find_by_id(&db, OTHER_WS, 2)
            .await
            .expect("find")
            .map(|f| f.id),
        Some(2)
    );
    assert!(svc::find_by_id(&db, WS, 2).await.expect("find").is_none());
}

// ---- readImage ---------------------------------------------------------------

#[tokio::test]
async fn media_library_service_read_image_reads_a_workspace_image_as_base64() {
    let db = two_workspaces().await;
    let store = uploader();
    store
        .put_object(
            "public/space/9/media-library/1",
            vec![1u8, 2, 3],
            PutOptions::default(),
            None,
        )
        .await
        .expect("put");
    setup(&db, |tx| {
        file_row(
            tx,
            1,
            WS,
            None,
            "public/space/9/media-library/1",
            "image/png",
            100,
            false,
        )
    })
    .await;
    let out = svc::read_image(&db, &store, WS, 1, 1000)
        .await
        .expect("read");
    assert_eq!(out, ("image/png".to_string(), "AQID".to_string()));
}

#[tokio::test]
async fn media_library_service_read_image_refuses_missing_files_non_images_and_files_over_the_limit()
 {
    let db = two_workspaces().await;
    // The object is never stored: a read of it would fail the test.
    let store = uploader();
    let err = svc::read_image(&db, &store, WS, 1, 1000)
        .await
        .expect_err("missing");
    assert_eq!(err.code(), Some("notFound"));
    setup(&db, |tx| {
        file_row(
            tx,
            1,
            WS,
            None,
            "public/space/9/a",
            "image/svg+xml",
            100,
            false,
        )?;
        file_row(
            tx,
            2,
            WS,
            None,
            "public/space/9/b",
            "image/png",
            5000,
            false,
        )
    })
    .await;
    for id in [1, 2] {
        let err = svc::read_image(&db, &store, WS, id, 1000)
            .await
            .expect_err("refused");
        let chatbotx_business::BusinessError::Exception { field, .. } = err else {
            panic!("exception");
        };
        assert_eq!(field.as_deref(), Some("fileId"));
    }
}
