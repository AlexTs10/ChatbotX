//! Port of `packages/auth/__tests__/send-magic-link.test.ts`.
//!
//! The original reads `magicLink.sendMagicLink` out of the mocked
//! `betterAuth(config)` call. Here it is `send_magic_link(deps, email, url,
//! request)`; the mocked services (`resolveTenantSettingsByDomain`,
//! `resolvePlatformSmtpTransport`, `db.query.userModel.findFirst`,
//! `sendMagicLink`) are an `AuthMailDeps` stub, and the `Request` is an
//! `AuthMailRequest` (public origin from the mocked
//! `getPublicOriginFromRequest`, `x-domain` header).

use std::sync::Mutex;

use chatbotx_api::session::{
    AuthMail, AuthMailDeps, AuthMailRequest, MailUser, SmtpResolution, TenantBrand, send_magic_link,
};

struct Mocks {
    user: Option<MailUser>,
    find_calls: Mutex<usize>,
    sent: Mutex<Vec<(String, AuthMail)>>,
}

impl Mocks {
    fn new() -> Mocks {
        Mocks {
            user: Some(MailUser {
                name: Some("Sub".to_string()),
                email: "a@b.test".to_string(),
            }),
            find_calls: Mutex::new(0),
            sent: Mutex::new(Vec::new()),
        }
    }
}

impl AuthMailDeps for Mocks {
    async fn tenant_settings(&self, _domain: &str) -> TenantBrand {
        TenantBrand {
            name: "Brand".to_string(),
            logo_light_url: None,
            magic_link_email_template: None,
            forgot_password_email_template: None,
        }
    }

    async fn smtp_for_tenant(&self) -> SmtpResolution {
        SmtpResolution::Default
    }

    async fn find_tenant_user(&self, _email: &str) -> Option<MailUser> {
        *self.find_calls.lock().unwrap() += 1;
        self.user.clone()
    }

    async fn send_magic_link(&self, to: &str, mail: AuthMail, _transport: Option<&str>) {
        self.sent.lock().unwrap().push((to.to_string(), mail));
    }

    async fn send_reset_password(&self, _to: &str, _mail: AuthMail, _transport: Option<&str>) {}
}

fn request() -> AuthMailRequest {
    AuthMailRequest {
        public_origin: "https://brand.example.com".to_string(),
        domain: "brand.example.com".to_string(),
    }
}

const LINK: &str = "https://app.example.com/api/auth/magic-link/verify?token=t";

#[tokio::test]
async fn rewrites_the_link_hostname_to_the_request_s_public_host() {
    let mocks = Mocks::new();

    send_magic_link(&mocks, "a@b.test", LINK, Some(&request()))
        .await
        .expect("sent");

    let sent = mocks.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    let url = url::Url::parse(&sent[0].1.url).unwrap();
    assert_eq!(url.host_str(), Some("brand.example.com"));
    let token = url
        .query_pairs()
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.into_owned());
    assert_eq!(token.as_deref(), Some("t"));
}

#[tokio::test]
async fn sends_nothing_to_a_placeholder_address() {
    let mocks = Mocks::new();

    send_magic_link(
        &mocks,
        "fb-123@facebook.placeholder.invalid",
        LINK,
        Some(&request()),
    )
    .await
    .expect("no error");

    assert_eq!(*mocks.find_calls.lock().unwrap(), 0);
    assert!(mocks.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rejects_an_email_that_is_not_registered_in_the_tenant() {
    let mut mocks = Mocks::new();
    mocks.user = None;

    let error = send_magic_link(&mocks, "nobody@b.test", LINK, Some(&request()))
        .await
        .expect_err("rejected");

    assert_eq!(error.message, "Your email is not registered with Brand");
    assert!(mocks.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn requires_a_request_object() {
    let mocks = Mocks::new();

    let error = send_magic_link(&mocks, "a@b.test", LINK, None)
        .await
        .expect_err("rejected");

    assert_eq!(error.message, "Unknown request");
}
