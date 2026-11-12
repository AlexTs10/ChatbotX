//! Port of `packages/auth/__tests__/signup-gate.test.ts`.
//!
//! The original reads the hooks out of the mocked `betterAuth(config)` call;
//! here they are the functions `user_create_before` and `email_request_before`
//! with stubbed services (`AUTH_DISABLE_SIGNUP`, `getTenantId() = "tenant-a"`,
//! `invitationService.findActiveByCode`). A `getCookie(name)` stub is a
//! `Cookie` request header.

use std::sync::Mutex;

use axum::http::{HeaderMap, HeaderValue};
use chatbotx_api::session::{NewUser, SignUpGateDeps, email_request_before, user_create_before};

struct Gate {
    disabled: bool,
    invitation_live: Mutex<bool>,
    calls: Mutex<Vec<(String, String)>>,
}

impl Gate {
    fn new(disabled: bool) -> Gate {
        Gate {
            disabled,
            invitation_live: Mutex::new(false),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn live(self, live: bool) -> Gate {
        *self.invitation_live.lock().unwrap() = live;
        self
    }

    fn last_call(&self) -> Option<(String, String)> {
        self.calls.lock().unwrap().last().cloned()
    }
}

impl SignUpGateDeps for Gate {
    fn is_sign_up_disabled(&self) -> bool {
        self.disabled
    }

    fn tenant_id(&self) -> String {
        "tenant-a".to_string()
    }

    async fn find_active_invitation(&self, code: &str, tenant_id: &str) -> bool {
        self.calls
            .lock()
            .unwrap()
            .push((code.to_string(), tenant_id.to_string()));
        *self.invitation_live.lock().unwrap()
    }
}

fn invitation_cookie(code: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "cookie",
        HeaderValue::from_str(&format!("chatbotx_invitation={code}")).unwrap(),
    );
    headers
}

fn invite_call(code: &str) -> Option<(String, String)> {
    Some((code.to_string(), "tenant-a".to_string()))
}

#[tokio::test]
async fn allows_normal_user_creation_when_signup_lockdown_is_disabled() {
    let gate = Gate::new(false);
    assert_eq!(
        user_create_before(&gate, &NewUser::default(), None).await,
        Ok(())
    );
}

#[tokio::test]
async fn rejects_non_anonymous_user_creation_without_an_active_invitation() {
    let gate = Gate::new(true).live(false);
    let error = user_create_before(&gate, &NewUser::default(), None)
        .await
        .unwrap_err();
    assert_eq!(error.message, "signup_disabled");
}

#[tokio::test]
async fn keeps_anonymous_sessions_available_during_signup_lockdown() {
    let gate = Gate::new(true);
    let user = NewUser {
        is_anonymous: Some(true),
        ..NewUser::default()
    };
    assert_eq!(user_create_before(&gate, &user, None).await, Ok(()));
}

#[tokio::test]
async fn allows_only_a_live_invitation_cookie_during_signup_lockdown() {
    let gate = Gate::new(true).live(true);
    let headers = invitation_cookie("invite-code");

    assert_eq!(
        user_create_before(&gate, &NewUser::default(), Some(&headers)).await,
        Ok(())
    );
    assert_eq!(gate.last_call(), invite_call("invite-code"));

    *gate.invitation_live.lock().unwrap() = false;
    let error = user_create_before(&gate, &NewUser::default(), Some(&headers))
        .await
        .unwrap_err();
    assert_eq!(error.message, "signup_disabled");
}

#[tokio::test]
async fn rejects_a_cookie_whose_invitation_belongs_to_a_different_tenant() {
    let gate = Gate::new(true).live(false);
    let headers = invitation_cookie("other-tenant-invite");

    let error = user_create_before(&gate, &NewUser::default(), Some(&headers))
        .await
        .unwrap_err();

    assert_eq!(error.message, "signup_disabled");
    assert!(
        gate.calls
            .lock()
            .unwrap()
            .contains(&invite_call("other-tenant-invite").unwrap())
    );
}

#[tokio::test]
async fn blocks_email_signup_but_leaves_existing_email_sign_in_alone() {
    let gate = Gate::new(true);
    let headers = HeaderMap::new();

    let error = email_request_before(&gate, "/sign-up/email", Some("new@example.com"), &headers)
        .await
        .unwrap_err();
    assert_eq!(error.message, "Sign-up is disabled");

    assert_eq!(
        email_request_before(&gate, "/sign-in/email", Some("new@example.com"), &headers).await,
        Ok(())
    );
}

#[tokio::test]
async fn allows_sign_up_email_through_the_middleware_hook_when_the_request_carries_a_live_invitation_cookie()
 {
    let gate = Gate::new(true).live(true);
    let headers = invitation_cookie("invite-code");

    assert_eq!(
        email_request_before(&gate, "/sign-up/email", Some("new@example.com"), &headers).await,
        Ok(())
    );
    assert_eq!(gate.last_call(), invite_call("invite-code"));
}
