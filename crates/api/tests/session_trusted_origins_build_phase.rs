//! Port of `packages/auth/__tests__/trusted-origins-build-phase.test.ts`.
//!
//! The original reads the `trustedOrigins` thunk out of the mocked
//! `betterAuth(config)` call, with `vi.stubEnv` for the URLs and
//! `NEXT_PHASE`. Here the thunk is `trusted_origins(settings, domains)`:
//! the environment is a `TrustedOriginSettings` value and
//! `customDomainService.listActiveDomains` is a counting stub.

use std::sync::Mutex;

use chatbotx_api::session::{
    CustomDomainSource, PHASE_PRODUCTION_BUILD, TrustedOriginSettings, trusted_origins,
};

const BROKER_URL: &str = "https://broker.example.com";
const BUILDER_URL: &str = "https://app.example.com";

struct Domains {
    active: Vec<String>,
    calls: Mutex<usize>,
}

impl Domains {
    fn new(active: &[&str]) -> Domains {
        Domains {
            active: active.iter().map(|domain| domain.to_string()).collect(),
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl CustomDomainSource for Domains {
    async fn list_active_domains(&self) -> Vec<String> {
        *self.calls.lock().unwrap() += 1;
        self.active.clone()
    }
}

fn settings(next_phase: &str) -> TrustedOriginSettings {
    TrustedOriginSettings {
        broker_url: Some(BROKER_URL.to_string()),
        builder_url: BUILDER_URL.to_string(),
        next_phase: Some(next_phase.to_string()),
    }
}

fn assert_contains_all(origins: &[String], expected: &[&str]) {
    for origin in expected {
        assert!(
            origins.iter().any(|candidate| candidate == origin),
            "{origin} missing from {origins:?}"
        );
    }
}

#[tokio::test]
async fn skips_the_customdomain_lookup_when_next_phase_is_phase_production_build() {
    let domains = Domains::new(&[]);

    let origins = trusted_origins(&settings(PHASE_PRODUCTION_BUILD), &domains).await;

    assert_contains_all(
        &origins,
        &[
            BROKER_URL,
            BUILDER_URL,
            "chatconnectxapp://",
            "chatbotxmobileapp://",
        ],
    );
    assert_eq!(origins.len(), 4);
    assert_eq!(domains.calls(), 0);
}

#[tokio::test]
async fn includes_active_custom_domains_outside_the_build_phase() {
    let domains = Domains::new(&["custom.example.com"]);

    let origins = trusted_origins(&settings(""), &domains).await;

    assert_eq!(domains.calls(), 1);
    assert_contains_all(
        &origins,
        &[BROKER_URL, BUILDER_URL, "https://custom.example.com"],
    );
}
