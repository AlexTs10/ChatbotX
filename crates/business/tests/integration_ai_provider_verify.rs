//! Port of `packages/business/src/integration-ai-provider/__tests__/verify.test.ts`
//! (Q.D6). Adapted: the mocked `ky.get` call becomes the probe request table
//! (`verify_request`) and the verdict of an answer (`classify`); a thrown
//! non-HTTP error is `None` (no answer).

use chatbotx_business::integration_ai_provider::{AiKeyValidation, classify, verify_request};

fn headers(provider: &str, key: &str) -> Vec<(String, String)> {
    let (_, headers, _) = verify_request(provider, key).expect("provider");
    let mut out: Vec<(String, String)> = headers
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    out.sort();
    out
}

fn verdict(provider: &str, status: Option<u16>) -> AiKeyValidation {
    let (_, _, invalid) = verify_request(provider, "key").expect("provider");
    classify(invalid, status)
}

#[test]
fn returns_valid_after_open_router_accepts_the_credential() {
    let (url, _, _) = verify_request("openrouter", "or-key").expect("provider");
    assert_eq!(url, "https://openrouter.ai/api/v1/key");
    assert_eq!(
        headers("openrouter", "or-key"),
        [("Authorization".to_string(), "Bearer or-key".to_string())]
    );
    assert_eq!(verdict("openrouter", Some(200)), AiKeyValidation::Valid);
}

#[test]
fn uses_the_provider_specific_probe_for_s() {
    let (url, _, _) = verify_request("claude", "provider-key").expect("claude");
    assert_eq!(url, "https://api.anthropic.com/v1/models");
    assert_eq!(
        headers("claude", "provider-key"),
        [
            ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ("x-api-key".to_string(), "provider-key".to_string()),
        ]
    );
    let (url, _, _) = verify_request("gemini", "provider-key").expect("gemini");
    assert_eq!(
        url,
        "https://generativelanguage.googleapis.com/v1beta/models"
    );
    assert_eq!(
        headers("gemini", "provider-key"),
        [("x-goog-api-key".to_string(), "provider-key".to_string())]
    );
    assert_eq!(verdict("gemini", Some(200)), AiKeyValidation::Valid);
}

#[test]
fn returns_invalid_when_the_provider_explicitly_rejects_credentials_with_i() {
    for status in [401, 403] {
        assert_eq!(
            verdict("openrouter", Some(status)),
            AiKeyValidation::Invalid
        );
    }
}

#[test]
fn returns_invalid_when_gemini_rejects_an_api_key_with_api_key_invalid() {
    assert_eq!(verdict("gemini", Some(400)), AiKeyValidation::Invalid);
}

#[test]
fn returns_unknown_for_an_inconclusive_provider_response() {
    for status in [Some(429), Some(503), None] {
        assert_eq!(verdict("openrouter", status), AiKeyValidation::Unknown);
    }
}
