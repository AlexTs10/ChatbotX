//! P3.6-D2 own tests (not ported, not locked): the `/files/{key}` route that
//! serves the local presigned URLs (`chatbotx_api::files`).

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use chatbotx_api::files::{presigned_upload_url, router};
use chatbotx_files::{LocalUrlSigner, PresignMethod, Uploader};
use tower::ServiceExt as _;

const BASE: &str = "http://app/files";

fn signer() -> LocalUrlSigner {
    LocalUrlSigner::new("test-secret", BASE).expect("signer")
}

fn store() -> Arc<Uploader> {
    Arc::new(Uploader::in_memory().with_url_signer(signer()))
}

/// The request target of an absolute app URL.
fn target(url: &str) -> String {
    url.strip_prefix("http://app").expect("app URL").to_string()
}

fn sign(method: PresignMethod, key: &str, now: SystemTime) -> String {
    target(&signer().sign_url(method, key, Duration::from_secs(300), now))
}

async fn send(app: &Router, method: &str, uri: &str, body: &'static [u8]) -> (StatusCode, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "text/csv")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, bytes.to_vec())
}

#[tokio::test]
async fn a_valid_signature_uploads_and_downloads_the_object() {
    let files = store();
    let app = router(Arc::clone(&files));
    let key = "workspaces/1/imports/contacts/import_7.CSV";
    let url = presigned_upload_url(&files, key).await.expect("presign");
    assert!(url.starts_with("http://app/files/workspaces/1/imports/contacts/import_7.CSV?"));
    assert!(url.contains("X-Amz-Expires=300&"), "{url}");
    let (status, _) = send(&app, "PUT", &target(&url), b"a,b\n1,2\n").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        files.get_object(key).await.expect("stored").as_ref(),
        b"a,b\n1,2\n"
    );
    let get = sign(PresignMethod::Get, key, SystemTime::now());
    let (status, body) = send(&app, "GET", &get, b"").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"a,b\n1,2\n");
    // A missing object is 404.
    let missing = sign(
        PresignMethod::Get,
        "workspaces/1/none.csv",
        SystemTime::now(),
    );
    assert_eq!(
        send(&app, "GET", &missing, b"").await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_bad_or_missing_signature_is_refused() {
    let files = store();
    let app = router(Arc::clone(&files));
    let key = "workspaces/1/a.csv";
    let good = sign(PresignMethod::Put, key, SystemTime::now());
    let tampered = format!(
        "{}0",
        good.trim_end_matches(|c: char| c.is_ascii_hexdigit())
    );
    assert_eq!(
        send(&app, "PUT", &tampered, b"x").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, "PUT", "/files/workspaces/1/a.csv", b"x").await.0,
        StatusCode::FORBIDDEN
    );
    // The upload URL cannot download, and it covers only its own key.
    assert_eq!(send(&app, "GET", &good, b"").await.0, StatusCode::FORBIDDEN);
    let other_key = good.replace("a.csv", "b.csv");
    assert_eq!(
        send(&app, "PUT", &other_key, b"x").await.0,
        StatusCode::FORBIDDEN
    );
    assert!(files.get_object(key).await.is_err());
    assert!(files.get_object("workspaces/1/b.csv").await.is_err());
}

#[tokio::test]
async fn an_expired_url_is_refused() {
    let files = store();
    let app = router(Arc::clone(&files));
    let key = "workspaces/1/old.csv";
    let expired = sign(
        PresignMethod::Put,
        key,
        SystemTime::now() - Duration::from_secs(600),
    );
    assert_eq!(
        send(&app, "PUT", &expired, b"x").await.0,
        StatusCode::FORBIDDEN
    );
    assert!(files.get_object(key).await.is_err());
}

#[tokio::test]
async fn path_traversal_is_refused() {
    let files = store();
    let app = router(Arc::clone(&files));
    // Even a correctly signed key with a `..` segment is not a storage key.
    for key in [
        "workspaces/1/../2/a.csv",
        "workspaces/./a.csv",
        "workspaces//a.csv",
    ] {
        let url = sign(PresignMethod::Put, key, SystemTime::now());
        assert_eq!(
            send(&app, "PUT", &url, b"x").await.0,
            StatusCode::BAD_REQUEST,
            "{key}"
        );
    }
    // An unsigned, percent-encoded traversal never reaches the store.
    assert_eq!(
        send(
            &app,
            "PUT",
            "/files/workspaces/1/%2e%2e/2/a.csv?expires=9999999999&signature=00",
            b"x"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(files.get_object("workspaces/2/a.csv").await.is_err());
}
