//! Port of `packages/utils/__tests__/storage.test.ts`.

use chatbotx_core::utils::storage::get_public_file_url;

const BUCKET_BASE: &str = "https://filesystem.example.com/chatbotx/";

/// Port of: packages/utils/__tests__/storage.test.ts:9 "getPublicFileUrl > joins a bare object key under the bucket base"
#[test]
fn getpublicfileurl_joins_a_bare_object_key_under_the_bucket_base() {
    assert_eq!(
        get_public_file_url("public/space/1/avatar/abc", BUCKET_BASE).unwrap(),
        "https://filesystem.example.com/chatbotx/public/space/1/avatar/abc"
    );
}

/// Port of: packages/utils/__tests__/storage.test.ts:15 "getPublicFileUrl > keeps the bucket even when the key has a leading slash"
#[test]
fn getpublicfileurl_keeps_the_bucket_even_when_the_key_has_a_leading_slash() {
    assert_eq!(
        get_public_file_url("/public/space/1/avatar/abc", BUCKET_BASE).unwrap(),
        "https://filesystem.example.com/chatbotx/public/space/1/avatar/abc"
    );
}

/// Port of: packages/utils/__tests__/storage.test.ts:22 "getPublicFileUrl > keeps the bucket even when the base has no trailing slash"
#[test]
fn getpublicfileurl_keeps_the_bucket_even_when_the_base_has_no_trailing_slash() {
    assert_eq!(
        get_public_file_url(
            "public/space/1/avatar/abc",
            "https://filesystem.example.com/chatbotx"
        )
        .unwrap(),
        "https://filesystem.example.com/chatbotx/public/space/1/avatar/abc"
    );
}

/// Port of: packages/utils/__tests__/storage.test.ts:31 "getPublicFileUrl > works with a bucket-less CDN base"
#[test]
fn getpublicfileurl_works_with_a_bucket_less_cdn_base() {
    assert_eq!(
        get_public_file_url("public/x.png", "https://cdn.example.com/").unwrap(),
        "https://cdn.example.com/public/x.png"
    );
}

/// Port of: packages/utils/__tests__/storage.test.ts:37 "getPublicFileUrl > returns an already-absolute http(s) path unchanged"
#[test]
fn getpublicfileurl_returns_an_already_absolute_http_s_path_unchanged() {
    let absolute = "https://scontent.fbcdn.net/v/t1/avatar.jpg";
    assert_eq!(
        get_public_file_url(absolute, BUCKET_BASE).unwrap(),
        absolute
    );
}
