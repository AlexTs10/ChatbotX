//! Port of `packages/auth/__tests__/mcp-resource.test.ts`.
//!
//! The mocked `oauthProviderResourceClient(auth).getActions().verifyBearerToken`
//! and `verifyOAuthQueryParams` are `BearerTokenVerifier` / `OAuthQueryVerifier`
//! stubs; `auth.$context.baseURL` is the `auth_base_url` argument and
//! `getMcpResourceUrl()` the `mcp_resource_url` argument.

use std::sync::Mutex;

use chatbotx_api::session::{
    BearerTokenVerifier, BearerVerifyError, McpAccessTokenSubject, McpConsentQuery,
    OAuthQueryVerifier, TokenVerifyError, VerifyBearerOptions, verify_mcp_access_token,
    verify_mcp_oauth_query,
};
use serde_json::{Value, json};

const BASE_URL: &str = "https://app.example.com/api/auth";
const MCP_RESOURCE: &str = "https://app.example.com/mcp";

struct Verifier {
    answers: Mutex<Vec<Result<Value, BearerVerifyError>>>,
    calls: Mutex<Vec<(String, VerifyBearerOptions)>>,
}

impl Verifier {
    fn answering(answers: Vec<Result<Value, BearerVerifyError>>) -> Verifier {
        Verifier {
            answers: Mutex::new(answers),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl BearerTokenVerifier for Verifier {
    async fn verify_bearer_token(
        &self,
        token: &str,
        options: &VerifyBearerOptions,
    ) -> Result<Value, BearerVerifyError> {
        self.calls
            .lock()
            .unwrap()
            .push((token.to_string(), options.clone()));
        self.answers.lock().unwrap().remove(0)
    }
}

fn api_error(status: &str, message: &str) -> BearerVerifyError {
    BearerVerifyError::Api {
        status: status.to_string(),
        message: message.to_string(),
    }
}

async fn verify(
    answer: Result<Value, BearerVerifyError>,
) -> Result<Option<McpAccessTokenSubject>, TokenVerifyError> {
    verify_mcp_access_token(
        &Verifier::answering(vec![answer]),
        BASE_URL,
        MCP_RESOURCE,
        "a.b.c",
    )
    .await
}

#[tokio::test]
async fn returns_the_subject_and_verifies_against_the_mcp_audience_origin_issuer_and_jwks() {
    let verifier = Verifier::answering(vec![Ok(json!({ "client_id": "client-1", "sub": "42" }))]);
    let subject = verify_mcp_access_token(&verifier, BASE_URL, MCP_RESOURCE, "a.b.c").await;
    assert_eq!(
        subject,
        Ok(Some(McpAccessTokenSubject {
            client_id: "client-1".to_string(),
            user_id: "42".to_string(),
        }))
    );
    assert_eq!(
        verifier.calls.lock().unwrap()[0],
        (
            "a.b.c".to_string(),
            VerifyBearerOptions {
                jwks_url: "https://app.example.com/api/auth/jwks".to_string(),
                audience: "https://app.example.com/mcp".to_string(),
                issuer: "https://app.example.com".to_string(),
            }
        )
    );
}

#[tokio::test]
async fn returns_null_for_an_expired_token() {
    assert_eq!(
        verify(Err(api_error("UNAUTHORIZED", "token expired"))).await,
        Ok(None)
    );
}

#[tokio::test]
async fn returns_null_for_a_token_with_another_audience_or_a_bad_signature() {
    assert_eq!(
        verify(Err(api_error("UNAUTHORIZED", "invalid access token"))).await,
        Ok(None)
    );
}

#[tokio::test]
async fn returns_null_for_a_raw_jose_token_error() {
    assert_eq!(
        verify(Err(BearerVerifyError::Jose("ERR_JWS_INVALID".into()))).await,
        Ok(None)
    );
}

#[tokio::test]
async fn maps_jwks_infrastructure_failures_to_an_unavailable_error() {
    assert_eq!(
        verify(Err(BearerVerifyError::Jose("ERR_JWKS_TIMEOUT".into()))).await,
        Err(TokenVerifyError::Unavailable)
    );
}

#[tokio::test]
async fn rethrows_non_unauthorized_api_errors() {
    let error = api_error("INTERNAL_SERVER_ERROR", "boom");
    assert_eq!(
        verify(Err(error.clone())).await,
        Err(TokenVerifyError::Rethrown(error))
    );
}

#[tokio::test]
async fn returns_null_when_client_id_or_sub_is_missing() {
    assert_eq!(verify(Ok(json!({ "sub": "42" }))).await, Ok(None));
    assert_eq!(
        verify(Ok(json!({ "client_id": "client-1" }))).await,
        Ok(None)
    );
}

struct QueryVerifier {
    valid: bool,
    calls: Mutex<Vec<(String, String)>>,
}

impl QueryVerifier {
    fn new(valid: bool) -> QueryVerifier {
        QueryVerifier {
            valid,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl OAuthQueryVerifier for QueryVerifier {
    async fn verify_oauth_query_params(&self, query: &str, secret: &str) -> bool {
        self.calls
            .lock()
            .unwrap()
            .push((query.to_string(), secret.to_string()));
        self.valid
    }
}

#[tokio::test]
async fn returns_null_for_a_query_whose_signature_does_not_verify() {
    let verifier = QueryVerifier::new(false);
    assert_eq!(
        verify_mcp_oauth_query(&verifier, "client_id=client-1&sig=forged", "test-secret").await,
        None
    );
    assert_eq!(
        verifier.calls.lock().unwrap()[0],
        (
            "client_id=client-1&sig=forged".to_string(),
            "test-secret".to_string()
        )
    );
}

#[tokio::test]
async fn returns_null_for_a_signed_query_without_a_client_id() {
    assert_eq!(
        verify_mcp_oauth_query(&QueryVerifier::new(true), "scope=openid", "test-secret").await,
        None
    );
}

#[tokio::test]
async fn returns_the_client_id_and_space_separated_scopes() {
    assert_eq!(
        verify_mcp_oauth_query(
            &QueryVerifier::new(true),
            "client_id=client-1&scope=openid%20%20email",
            "test-secret"
        )
        .await,
        Some(McpConsentQuery {
            client_id: "client-1".to_string(),
            scopes: vec!["openid".to_string(), "email".to_string()],
        })
    );
}
