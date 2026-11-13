//! Port of `packages/auth/__tests__/mobile-resource.test.ts`.
//!
//! `isMobileAccessTokenCandidate` takes the mobile audience as an argument
//! (`getMobileApiAudience()` = `https://app.example.com/api` in the original);
//! the mocked `verifyBearerToken` is a `BearerTokenVerifier` stub.

use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chatbotx_api::auth::{is_mobile_access_token_candidate, peek_jwt_audiences};
use chatbotx_api::session::{
    BearerTokenVerifier, BearerVerifyError, MobileAccessTokenSubject, TokenVerifyError,
    VerifyBearerOptions, verify_mobile_access_token,
};
use serde_json::{Value, json};

const BASE_URL: &str = "https://app.example.com/api/auth";
const AUDIENCE: &str = "https://app.example.com/api";

fn jwt_with(payload: &Value) -> String {
    format!(
        "{}.{}.signature",
        STANDARD.encode(json!({ "alg": "RS256" }).to_string()),
        URL_SAFE_NO_PAD.encode(payload.to_string())
    )
}

fn candidate(token: &str) -> bool {
    is_mobile_access_token_candidate(token, AUDIENCE)
}

#[test]
fn a_mobile_audience_token_is_a_mobile_candidate() {
    assert!(candidate(&jwt_with(
        &json!({ "aud": "https://app.example.com/api" })
    )));
}

#[test]
fn an_array_audience_containing_the_mobile_audience_is_a_candidate() {
    assert!(candidate(&jwt_with(
        &json!({ "aud": ["https://other", "https://app.example.com/api"] })
    )));
}

#[test]
fn s_is_not_a_mobile_candidate() {
    for (name, payload) in [
        (
            "an MCP token",
            json!({ "aud": "https://app.example.com/mcp" }),
        ),
        ("a token with no audience", json!({})),
        ("a token with a non-string audience", json!({ "aud": 42 })),
        (
            "a look-alike audience",
            json!({ "aud": "https://app.example.com/api/evil" }),
        ),
    ] {
        assert!(!candidate(&jwt_with(&payload)), "{name}");
    }
}

#[test]
fn malformed_input_j_yields_no_audiences_and_never_throws() {
    for token in ["", "opaque", "a.b", "a.b.c.d", "a.!!!.c"] {
        assert_eq!(peek_jwt_audiences(token), Vec::<String>::new(), "{token}");
        assert!(!candidate(token), "{token}");
    }
}

struct Verifier {
    answer: Result<Value, BearerVerifyError>,
    calls: Mutex<Vec<(String, VerifyBearerOptions)>>,
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
        self.answer.clone()
    }
}

fn verifier(answer: Result<Value, BearerVerifyError>) -> Verifier {
    Verifier {
        answer,
        calls: Mutex::new(Vec::new()),
    }
}

async fn verify(
    answer: Result<Value, BearerVerifyError>,
) -> Result<Option<MobileAccessTokenSubject>, TokenVerifyError> {
    verify_mobile_access_token(&verifier(answer), BASE_URL, AUDIENCE, "a.b.c").await
}

#[tokio::test]
async fn verifies_against_the_mobile_audience_origin_issuer_and_jwks_and_returns_the_subject() {
    let stub = verifier(Ok(json!({
        "sub": "42",
        "deviceId": "install-1",
        "mustChangePassword": true,
    })));
    assert_eq!(
        verify_mobile_access_token(&stub, BASE_URL, AUDIENCE, "a.b.c").await,
        Ok(Some(MobileAccessTokenSubject {
            user_id: "42".to_string(),
            device_id: Some("install-1".to_string()),
            must_change_password: true,
        }))
    );
    assert_eq!(
        stub.calls.lock().unwrap()[0],
        (
            "a.b.c".to_string(),
            VerifyBearerOptions {
                jwks_url: "https://app.example.com/api/auth/jwks".to_string(),
                audience: "https://app.example.com/api".to_string(),
                issuer: "https://app.example.com".to_string(),
            }
        )
    );
}

#[tokio::test]
async fn tolerates_a_token_without_the_optional_claims() {
    assert_eq!(
        verify(Ok(json!({ "sub": "42" }))).await,
        Ok(Some(MobileAccessTokenSubject {
            user_id: "42".to_string(),
            device_id: None,
            must_change_password: false,
        }))
    );
}

#[tokio::test]
async fn s_is_a_null_result_not_an_error() {
    for (name, error) in [
        (
            "an unauthorized API error",
            BearerVerifyError::Api {
                status: "UNAUTHORIZED".to_string(),
                message: "bad".to_string(),
            },
        ),
        (
            "a jose signature failure",
            BearerVerifyError::Jose("ERR_JWS_SIGNATURE_VERIFICATION_FAILED".to_string()),
        ),
        (
            "an expired token",
            BearerVerifyError::Jose("ERR_JWT_EXPIRED".to_string()),
        ),
        (
            "a wrong audience",
            BearerVerifyError::Jose("ERR_JWT_CLAIM_VALIDATION_FAILED".to_string()),
        ),
    ] {
        assert_eq!(verify(Err(error)).await, Ok(None), "{name}");
    }
}

#[tokio::test]
async fn a_token_with_no_subject_is_rejected() {
    assert_eq!(verify(Ok(json!({ "aud": "x" }))).await, Ok(None));
    assert_eq!(verify(Ok(json!({ "sub": "" }))).await, Ok(None));
}

#[tokio::test]
async fn jwks_infrastructure_failure_s_surfaces_as_a_server_error_not_a_401() {
    for code in [
        "ERR_JWKS_TIMEOUT",
        "ERR_JWKS_INVALID",
        "ERR_JWKS_MULTIPLE_MATCHING_KEYS",
    ] {
        assert_eq!(
            verify(Err(BearerVerifyError::Jose(code.to_string()))).await,
            Err(TokenVerifyError::Unavailable),
            "{code}"
        );
    }
}

#[tokio::test]
async fn an_unexpected_error_propagates() {
    let error = BearerVerifyError::Other("boom".to_string());
    assert_eq!(
        verify(Err(error.clone())).await,
        Err(TokenVerifyError::Rethrown(error))
    );
}
