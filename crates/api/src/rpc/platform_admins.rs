//! The platform admin actions of the builder (Q.D11 slice 10,
//! `enterprise/features/platform-admins/*`) under the `/rpc` key
//! `platformAdminActions`: `addPlatformAdmin`, `removePlatformAdmin`
//! (`superAdminActionClient` + `assertEnterpriseFeatures`), plus
//! `platformAdminService.add / remove` and `findOrProvisionRootUser`.

use std::sync::Arc;

use chatbotx_business::BusinessError;
use chatbotx_db::Db;
use chatbotx_db::repo::models::PlatformAdmin;
use chatbotx_db::repo::user::{InsertWithCredentialInput, insert_with_credential};
use chatbotx_kv::{DurableKv, SystemClock};
use serde_json::{Value, json};

use super::actions::{ActionContext, ActionError, ActionResult, Gate};
use super::platform_credentials::{is_super_admin, unauthorized};
use crate::handlers::contacts::is_email;
use crate::handlers::input::{Input, Str, key};
use crate::handlers::{ApiState, date, id};
use crate::mail::Mailer;

/// `ROOT_TENANT_ID`.
const ROOT_TENANT_ID: i64 = 1;
/// `platformAdminRoles`.
const ROLES: &[&str] = &["super-admin", "support"];
/// `z.email().trim().toLowerCase()` (lowercased by the service).
pub(super) const EMAIL: Str = Str {
    trim: true,
    min: None,
    max: None,
    pattern: Some(is_email),
};
/// `z.string().min(1)`.
const USER_ID: Str = Str {
    trim: false,
    min: Some(1),
    max: None,
    pattern: None,
};
/// `DEFAULT_ACCOUNT_CREDENTIALS_SUBJECT` with the platform name.
const CREDENTIALS_SUBJECT: &str = "Your ChatbotX account is ready";

/// The actions of the `platformAdminActions` key.
pub fn action_gate(name: &str) -> Option<Gate> {
    matches!(name, "addPlatformAdmin" | "removePlatformAdmin").then_some(Gate::Auth)
}

fn db_error(e: chatbotx_db::DbError) -> BusinessError {
    BusinessError::from(e)
}

async fn read<R, F>(db: &Db, f: F) -> Result<R, BusinessError>
where
    R: Send + 'static,
    F: FnOnce(&chatbotx_db::rusqlite::Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
{
    db.read(move |c| f(c).map_err(|e| chatbotx_db::DbError::other(e.message())))
        .await
        .map_err(db_error)
}

/// `findOrProvisionRootUser(email)`: the root-tenant user with this email,
/// or a new one with a mailed temporary password (the mail goes first: a
/// failed mail creates no user). Returns `(userId, created)`.
pub async fn find_or_provision_root_user(
    api: &ApiState,
    mailer: &dyn Mailer,
    email: &str,
) -> Result<(i64, bool), ActionError> {
    let lookup = email.to_owned();
    let existing = read(&api.db, move |c| {
        chatbotx_db::repo::user::find_id_by_email(c, &lookup, ROOT_TENANT_ID)
    })
    .await?;
    if let Some(user_id) = existing {
        return Ok((user_id, false));
    }
    if chatbotx_core::utils::placeholder_email::is_placeholder_email(Some(email)) {
        return Err(BusinessError::exception(
            "Email address is reserved and cannot be provisioned",
            "invalidEmail",
            400,
        )
        .into());
    }
    // better-auth `generateRandomString(16, "a-z", "A-Z", "0-9")`.
    let password = crate::session::random_string(16);
    let hashed = {
        let password = password.clone();
        tokio::task::spawn_blocking(move || crate::session::hash_password(&password))
            .await
            .map_err(|e| ActionError::Internal(e.to_string()))?
    };
    let sign_in = format!("{}/auth/sign-in", api.app_url.trim_end_matches('/'));
    // ponytail: plain text, no branded template or reseller SMTP (as the auth
    // mails) until the mail package is ported.
    let text = format!(
        "Hi {email},\n\nYour account is ready. Use the credentials below to sign in.\n\n\
         Email: {email}\nTemporary password: {password}\n\nSign in: {sign_in}\n\n\
         You will be asked to change this password the first time you sign in.\n"
    );
    mailer
        .send(email, CREDENTIALS_SUBJECT, &text)
        .await
        .map_err(ActionError::Internal)?;
    let inserted = insert_with_credential(
        &api.db,
        InsertWithCredentialInput {
            email: email.to_owned(),
            name: None,
            hashed_password: hashed,
            tenant_id: ROOT_TENANT_ID,
        },
    )
    .await
    .map_err(|e| ActionError::Internal(e.message()))?;
    let kv = DurableKv::new(Arc::clone(&api.db), Arc::new(SystemClock));
    chatbotx_business::user_quota::ensure_bootstrap_plan(
        &api.db,
        Some(&kv),
        api.edition,
        inserted.user_id,
        Some(ROOT_TENANT_ID),
    )
    .await?;
    Ok((inserted.user_id, inserted.created))
}

/// `assertCanManageAdmins`: only a `super-admin` row.
async fn assert_can_manage_admins(db: &Db, actor_id: i64) -> Result<(), BusinessError> {
    let actor = read(db, move |c| {
        chatbotx_db::repo::platform_admin::find_for_auth(c, actor_id)
    })
    .await?;
    if actor.is_some_and(|a| a.role == "super-admin") {
        Ok(())
    } else {
        Err(BusinessError::exception(
            "Only super-admins can manage platform administrators",
            "forbidden",
            403,
        ))
    }
}

/// `platformAdminService.add`. Returns the admin row and whether the user
/// was provisioned.
pub async fn add(
    api: &ApiState,
    mailer: &dyn Mailer,
    actor_id: i64,
    email: &str,
    role: &str,
) -> Result<(PlatformAdmin, bool), ActionError> {
    assert_can_manage_admins(&api.db, actor_id).await?;
    let email = email.trim().to_lowercase();
    let (user_id, created) = find_or_provision_root_user(api, mailer, &email).await?;
    if actor_id == user_id && role != "super-admin" {
        return Err(BusinessError::exception(
            "You cannot lower your own platform administrator role",
            "cannotRemoveSelf",
            400,
        )
        .into());
    }
    let role_owned = role.to_owned();
    let admin = api
        .db
        .write(move |tx| {
            chatbotx_db::repo::platform_admin::upsert_role(tx, &role_owned, user_id)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .map_err(db_error)?;
    let Some(admin) = admin else {
        return Err(BusinessError::exception(
            format!("Platform admin {user_id} was deleted during assignment"),
            "platformAdminAssignmentConflict",
            409,
        )
        .into());
    };
    tracing::info!(
        actor_id,
        user_id,
        provisioned = created,
        "added platform admin"
    );
    Ok((admin, created))
}

/// `platformAdminService.remove`.
pub async fn remove(api: &ApiState, actor_id: i64, user_id: i64) -> Result<(), BusinessError> {
    assert_can_manage_admins(&api.db, actor_id).await?;
    if actor_id == user_id {
        return Err(BusinessError::exception(
            "You cannot remove yourself as a platform admin",
            "cannotRemoveSelf",
            400,
        ));
    }
    api.db
        .write(move |tx| {
            chatbotx_db::repo::platform_admin::delete_by_user_id(tx, user_id)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .map_err(db_error)?;
    tracing::info!(actor_id, user_id, "removed platform admin");
    Ok(())
}

fn admin_json(admin: &PlatformAdmin) -> Value {
    json!({
        "id": id(admin.id),
        "createdAt": date(admin.created_at),
        "updatedAt": date(admin.updated_at),
        "userId": id(admin.user_id),
        "role": admin.role,
    })
}

/// Runs one action of the `platformAdminActions` key.
pub async fn run_action(
    api: &ApiState,
    name: &str,
    context: &ActionContext,
    raw: &Value,
) -> ActionResult {
    // `superAdminActionClient` runs before the input schema.
    if !is_super_admin(api, context.user_id).await? {
        return Err(unauthorized());
    }
    let mut input = Input::from_object(raw.as_object().cloned().unwrap_or_default());
    match name {
        "addPlatformAdmin" => {
            let email = input.string("email", EMAIL);
            let role = {
                let value = input.get("role").cloned();
                input.enum_at(&key("role"), value.as_ref(), ROLES, true)
            };
            input.finish()?;
            assert_enterprise_features(api).await?;
            let (admin, _) = add(
                api,
                context.mailer.as_ref(),
                context.user_id,
                &email.unwrap_or_default(),
                &role.unwrap_or_default(),
            )
            .await?;
            Ok(Some(admin_json(&admin)))
        }
        "removePlatformAdmin" => {
            let user = input.string("userId", USER_ID);
            input.finish()?;
            assert_enterprise_features(api).await?;
            // A non-numeric id matches no row.
            let user_id = user.and_then(|u| u.parse().ok()).unwrap_or(-1);
            remove(api, context.user_id, user_id).await?;
            Ok(None)
        }
        _ => Err(ActionError::Internal(format!("unknown action {name}"))),
    }
}

async fn assert_enterprise_features(api: &ApiState) -> Result<(), ActionError> {
    Ok(api.assert_enterprise_features().await?)
}
