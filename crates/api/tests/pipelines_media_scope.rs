//! Port of `apps/builder/__tests__/pipelines-media-scope.test.ts`. Adapted:
//! the original calls the procedures with a stubbed token and mocked
//! services; here the requests go through `handle_public_request` with a
//! stubbed token lookup (`support::MockDeps`). "Attaches" means "passes the
//! auth of `pipelines.addAttachment`" (the mocked service call and its
//! `scope: workspaceToken` argument have no counterpart; the handler always
//! acts with `PIPELINE_TOKEN_SCOPE`).

mod support;

use chatbotx_api::routes::PUBLIC_ROUTES;
use support::{MockDeps, auth_result, call_op};

fn token(scopes: &[&str]) -> MockDeps {
    MockDeps::with_lookup(Some(auth_result(Some(scopes), "full")))
}

async fn assert_denied(id: &str, scopes: &[&str], scope: &str) {
    let answer = call_op(&token(scopes), id).await;
    assert_eq!(answer.status, 403, "{id}: {}", answer.body);
    assert_eq!(answer.code(), "FORBIDDEN", "{id}");
    assert_eq!(
        answer.message(),
        format!("Token is not authorized for the '{scope}' scope"),
        "{id}"
    );
}

#[test]
fn the_attach_procedure_exists_on_the_public_router() {
    assert!(
        PUBLIC_ROUTES
            .iter()
            .any(|route| route.id == "pipelines.addAttachment")
    );
}

#[tokio::test]
async fn a_token_with_only_the_pipelines_scope_can_attach_an_existing_media_file() {
    let answer = call_op(&token(&["pipelines"]), "pipelines.addAttachment").await;
    assert!(answer.passed_auth(), "{} {}", answer.status, answer.body);
}

#[tokio::test]
async fn a_token_with_only_the_pipelines_scope_cannot_request_an_upload_url() {
    assert_denied("mediaLibrary.createUploadUrl", &["pipelines"], "media").await;
}

#[tokio::test]
async fn a_token_with_only_the_pipelines_scope_cannot_register_a_media_file() {
    assert_denied("mediaLibrary.createFile", &["pipelines"], "media").await;
}

#[tokio::test]
async fn a_token_with_only_the_media_scope_cannot_attach_a_file() {
    assert_denied("pipelines.addAttachment", &["media"], "pipelines").await;
}
