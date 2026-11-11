//! Port of `packages/filesystem/__tests__/copy-source.test.ts` (P2.9).
//!
//! JS `buildCopySource` returns a string; Rust returns `Result` (an invalid
//! endpoint is an error, JS throws `TypeError`), so the tests unwrap.

use chatbotx_files::build_copy_source;

/// Port of: packages/filesystem/__tests__/copy-source.test.ts:5 "buildCopySource > uses bucket/key when no endpoint is configured"
#[test]
fn build_copy_source_uses_bucket_key_when_no_endpoint_is_configured() {
    assert_eq!(
        build_copy_source("public/space/1/file.jpg", "mybucket", None).unwrap(),
        "mybucket/public/space/1/file.jpg",
    );
}

/// Port of: packages/filesystem/__tests__/copy-source.test.ts:11 "buildCopySource > uses bucket/key when endpoint has no path"
#[test]
fn build_copy_source_uses_bucket_key_when_endpoint_has_no_path() {
    assert_eq!(
        build_copy_source(
            "public/space/1/file.jpg",
            "mybucket",
            Some("https://account.r2.cloudflarestorage.com"),
        )
        .unwrap(),
        "mybucket/public/space/1/file.jpg",
    );
}

/// Port of: packages/filesystem/__tests__/copy-source.test.ts:21 "buildCopySource > replicates the endpoint path prefix when endpoint includes it"
#[test]
fn build_copy_source_replicates_the_endpoint_path_prefix_when_endpoint_includes_it() {
    // Endpoint mistakenly (or intentionally) carries the bucket in its path:
    // every URL-addressed op stores objects under "chatconnectx/<key>", so the
    // header-addressed CopySource must point at the same real key.
    assert_eq!(
        build_copy_source(
            "public/space/1/file.jpg",
            "chatconnectx",
            Some("https://account.r2.cloudflarestorage.com/chatconnectx"),
        )
        .unwrap(),
        "chatconnectx/chatconnectx/public/space/1/file.jpg",
    );
}

/// Port of: packages/filesystem/__tests__/copy-source.test.ts:34 "buildCopySource > handles endpoint path with trailing slash"
#[test]
fn build_copy_source_handles_endpoint_path_with_trailing_slash() {
    assert_eq!(
        build_copy_source("a/b.png", "bkt", Some("https://host.example.com/prefix/")).unwrap(),
        "prefix/bkt/a/b.png",
    );
}

/// Port of: packages/filesystem/__tests__/copy-source.test.ts:40 "buildCopySource > URL-encodes each key segment"
#[test]
fn build_copy_source_url_encodes_each_key_segment() {
    assert_eq!(
        build_copy_source("a b/c#d.png", "bkt", None).unwrap(),
        "bkt/a%20b/c%23d.png",
    );
}
