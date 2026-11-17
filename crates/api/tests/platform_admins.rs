//! `packages/business/__tests__/platform-admin.service.test.ts` (Q.D11 slice
//! 10) against a real database and a recording mailer, plus the gates of the
//! `/rpc/platformAdminActions/*` actions.

mod rpc_common;

use std::sync::Mutex;

use axum::http::StatusCode;
use chatbotx_api::mail::{Mailer, SendFuture};
use chatbotx_api::rpc::actions::ActionError;
use chatbotx_api::rpc::platform_admins::{add, remove};
use chatbotx_business::keys::Edition;
use rpc_common::{AGENT, AGENT_TOKEN, OWNER, OWNER_TOKEN, action, fixture_as, scalar};
use serde_json::json;

#[derive(Default)]
struct Recorder {
    fail: bool,
    sent: Mutex<Vec<(String, String, String)>>,
}

impl Mailer for Recorder {
    fn send<'a>(&'a self, to: &'a str, subject: &'a str, text: &'a str) -> SendFuture<'a> {
        Box::pin(async move {
            if self.fail {
                return Err("SMTP unavailable".to_string());
            }
            let mail = (to.to_string(), subject.to_string(), text.to_string());
            self.sent.lock().unwrap().push(mail);
            Ok(())
        })
    }
}

/// `OWNER` is a super admin; `AGENT` a support admin.
async fn seeded(edition: Edition) -> rpc_common::Fixture {
    let fx = fixture_as(edition).await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "PlatformAdmin" ("id", "userId", "role") VALUES (1, ?1, 'super-admin')"#,
                [OWNER],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    fx
}

async fn make_support(fx: &rpc_common::Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "PlatformAdmin" ("id", "userId", "role") VALUES (2, ?1, 'support')"#,
                [AGENT],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

fn code(err: ActionError) -> (Option<String>, Option<u16>) {
    match err {
        ActionError::Exception(e) => (e.code().map(str::to_owned), e.status()),
        other => panic!("not an exception: {other:?}"),
    }
}

const ROLE_OF: &str = r#"SELECT "role" FROM "PlatformAdmin" WHERE "userId" = ?1"#;
const USERS_WITH_EMAIL: &str =
    r#"SELECT COUNT(*) FROM "User" WHERE "email" = 'new@example.com' AND "id" <> ?1"#;

#[tokio::test]
async fn normalizes_an_existing_root_users_email_before_granting_the_role() {
    let fx = seeded(Edition::Community).await;
    let mail = Recorder::default();
    let (admin, created) = add(&fx.api, &mail, OWNER, " Agent@RPC.test ", "support")
        .await
        .unwrap();
    assert_eq!(
        (admin.user_id, admin.role.as_str(), created),
        (AGENT, "support", false)
    );
    assert!(mail.sent.lock().unwrap().is_empty());
    assert_eq!(scalar(&fx, ROLE_OF, AGENT).await, json!("support"));
}

#[tokio::test]
async fn creates_the_user_mails_credentials_then_grants_admin() {
    let fx = seeded(Edition::Cloud).await;
    let mail = Recorder::default();
    let (admin, created) = add(&fx.api, &mail, OWNER, "new@example.com", "support")
        .await
        .unwrap();
    assert!(created);
    let sent = mail.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    let (to, subject, text) = &sent[0];
    assert_eq!(
        (to.as_str(), subject.as_str()),
        ("new@example.com", "Your ChatbotX account is ready")
    );
    assert!(text.contains("Email: new@example.com"), "{text}");
    assert!(text.contains("Sign in: http://app/auth/sign-in"), "{text}");
    let password = text
        .lines()
        .find_map(|l| l.strip_prefix("Temporary password: "))
        .unwrap();
    assert_eq!(password.len(), 16);
    // `insertWithCredential({ email, hashedPassword, tenantId: "1" })`.
    let user = admin.user_id;
    let row = fx
        .db()
        .read(move |c| {
            Ok(c.query_row(
                r#"SELECT u."tenantId", u."mustChangePassword", a."password" FROM "User" u JOIN "Account" a ON a."userId" = u."id" WHERE u."id" = ?1 AND a."providerId" = 'credential'"#,
                [user],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!((row.0, row.1), (1, 1));
    assert!(chatbotx_api::session::verify_password(&row.2, password));
    // `ensureBootstrapPlan({ tenantId: "1", userId })` (cloud).
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "UserQuota" WHERE "userId" = ?1"#,
            user
        )
        .await,
        json!(1)
    );
    assert_eq!(scalar(&fx, ROLE_OF, user).await, json!("support"));
}

#[tokio::test]
async fn does_not_create_a_user_when_the_credentials_email_fails() {
    let fx = seeded(Edition::Community).await;
    let mail = Recorder {
        fail: true,
        ..Recorder::default()
    };
    let err = add(&fx.api, &mail, OWNER, "new@example.com", "support")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ActionError::Internal(m) if m == "SMTP unavailable"),
        "{err:?}"
    );
    assert_eq!(scalar(&fx, USERS_WITH_EMAIL, 0).await, json!(0));
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "PlatformAdmin" WHERE "id" <> ?1"#,
            1
        )
        .await,
        json!(0)
    );
}

#[tokio::test]
async fn rejects_a_placeholder_email_support_actors_and_lowering_the_own_role() {
    let fx = seeded(Edition::Community).await;
    let mail = Recorder::default();
    let err = add(
        &fx.api,
        &mail,
        OWNER,
        "fb-9@facebook.placeholder.invalid",
        "support",
    )
    .await
    .unwrap_err();
    assert_eq!(code(err), (Some("invalidEmail".into()), Some(400)));
    assert!(mail.sent.lock().unwrap().is_empty());

    let err = add(&fx.api, &mail, OWNER, "owner@rpc.test", "support")
        .await
        .unwrap_err();
    assert_eq!(code(err), (Some("cannotRemoveSelf".into()), Some(400)));
    assert_eq!(scalar(&fx, ROLE_OF, OWNER).await, json!("super-admin"));

    make_support(&fx).await;
    let err = add(&fx.api, &mail, AGENT, "new@example.com", "support")
        .await
        .unwrap_err();
    assert_eq!(code(err).1, Some(403));
    assert!(mail.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn remove_refuses_self_and_support_actors_and_removes_an_admin() {
    let fx = seeded(Edition::Community).await;
    make_support(&fx).await;
    let err = remove(&fx.api, OWNER, OWNER).await.unwrap_err();
    assert_eq!(
        (err.code(), err.status()),
        (Some("cannotRemoveSelf"), Some(400))
    );
    let err = remove(&fx.api, AGENT, OWNER).await.unwrap_err();
    assert_eq!(err.status(), Some(403));
    assert_eq!(scalar(&fx, ROLE_OF, OWNER).await, json!("super-admin"));
    remove(&fx.api, OWNER, AGENT).await.unwrap();
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "PlatformAdmin" WHERE "userId" = ?1"#,
            AGENT
        )
        .await,
        json!(0)
    );
}

/// `superAdminActionClient` first, then the schema, then
/// `assertEnterpriseFeatures()` (never entitled on community).
#[tokio::test]
async fn the_actions_need_a_super_admin_valid_input_and_an_enterprise_license() {
    let fx = seeded(Edition::Community).await;
    let key = "platformAdminActions";
    let input = json!({ "email": "new@example.com", "role": "support" });
    let (status, out) = action(&fx, key, "addPlatformAdmin", AGENT_TOKEN, input.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
    let bad = json!({ "email": "not-an-email", "role": "owner" });
    let (status, out) = action(&fx, key, "addPlatformAdmin", OWNER_TOKEN, bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    let (status, out) = action(&fx, key, "addPlatformAdmin", OWNER_TOKEN, input).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
    assert_eq!(
        out["json"]["code"],
        json!("enterpriseFeatureRequired"),
        "{out}"
    );
    let remove = json!({ "userId": AGENT.to_string() });
    let (status, out) = action(&fx, key, "removePlatformAdmin", OWNER_TOKEN, remove).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
}
