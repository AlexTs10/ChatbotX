//! `platform-credential/service.ts` + `can-use-own-credentials.ts` (Q.D11
//! slice 7): encrypted app credentials, user-scoped (`userId`) or
//! platform-global (`userId` NULL). A config is a JSON object; the per-type
//! zod schemas (`credentialSchemas` / `credentialPublicSchemas`) are the
//! field tables below. The TS row cache (`cred:u:*`, `cred:p:*`) is not
//! needed: SQLite reads are local.

use chatbotx_core::encryption::{EncryptedData, Encryption};
use chatbotx_db::repo::models::PlatformCredential;
use chatbotx_db::repo::partials::credential::credential_aad_for;
use chatbotx_db::repo::{self, Json, find_one, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Map, Value};

use crate::errors::{BusinessError, Result};

/// `credentialTypes`.
pub const CREDENTIAL_TYPES: &[&str] = &[
    "whatsapp",
    "messenger",
    "instagram",
    "instagramFacebook",
    "threads",
    "google",
    "googleAds",
    "zalo",
    "giphy",
    "stripe",
    "smtp",
    "paddle",
    "tiktok",
    "make",
];

/// `MetaAppCredentialType`.
pub const META_APP_CREDENTIAL_TYPES: &[&str] =
    &["messenger", "instagram", "instagramFacebook", "threads"];

#[derive(Clone, Copy)]
enum Kind {
    Str,
    PosInt,
    Object,
}

/// `(field, kind, required)` of the full schema and the public pick.
struct Spec {
    fields: &'static [(&'static str, Kind, bool)],
    public: &'static [&'static str],
}

const META_APP: &[(&str, Kind, bool)] = &[
    ("clientId", Kind::Str, true),
    ("version", Kind::Str, true),
    ("verifyToken", Kind::Str, true),
    ("clientSecret", Kind::Str, true),
];
const META_APP_PUBLIC: &[&str] = &["clientId", "version", "verifyToken"];

fn spec(credential_type: &str) -> Option<Spec> {
    use Kind::*;
    Some(match credential_type {
        "whatsapp" => Spec {
            fields: &[
                ("clientId", Str, true),
                ("version", Str, true),
                ("configId", Str, true),
                ("systemUserId", Str, true),
                ("businessId", Str, false),
                ("businessName", Str, true),
                ("verifyToken", Str, true),
                ("clientSecret", Str, true),
                ("systemUserToken", Str, true),
                ("capiConfigId", Str, false),
            ],
            public: &[
                "clientId",
                "version",
                "configId",
                "systemUserId",
                "businessId",
                "businessName",
                "verifyToken",
                "capiConfigId",
            ],
        },
        "messenger" => Spec {
            fields: &[
                ("clientId", Str, true),
                ("version", Str, true),
                ("verifyToken", Str, true),
                ("clientSecret", Str, true),
                ("marketingMessagesConfigId", Str, false),
            ],
            public: &[
                "clientId",
                "version",
                "verifyToken",
                "marketingMessagesConfigId",
            ],
        },
        "instagram" | "instagramFacebook" | "threads" | "zalo" => Spec {
            fields: META_APP,
            public: META_APP_PUBLIC,
        },
        "google" => Spec {
            fields: &[
                ("clientId", Str, true),
                ("clientSecret", Str, true),
                ("verifyToken", Str, true),
            ],
            public: &["clientId"],
        },
        "googleAds" => Spec {
            fields: &[
                ("clientId", Str, true),
                ("clientSecret", Str, true),
                ("developerToken", Str, false),
                ("uploadMethod", Str, false),
            ],
            // + `hasDeveloperToken` (see `public_config`).
            public: &["clientId", "uploadMethod"],
        },
        "giphy" => Spec {
            fields: &[("apiKey", Str, true)],
            public: &[],
        },
        "stripe" => Spec {
            fields: &[
                ("publishableKey", Str, true),
                ("verifyToken", Str, true),
                ("secretKey", Str, true),
                ("connectClientId", Str, false),
                ("connectWebhookEndpointId", Str, false),
                ("connectWebhookSecret", Str, false),
                ("platformAccountId", Str, false),
            ],
            public: &["publishableKey", "verifyToken", "connectClientId"],
        },
        "smtp" => Spec {
            fields: &[
                ("host", Str, true),
                ("port", PosInt, true),
                ("username", Str, true),
                ("password", Str, true),
                ("fromEmail", Str, true),
                ("fromName", Str, false),
            ],
            public: &["host", "port", "username", "fromEmail", "fromName"],
        },
        "paddle" => Spec {
            fields: &[
                ("vendorId", Str, true),
                ("vendorAuthCode", Str, true),
                ("publicKey", Str, false),
                ("verifyToken", Str, false),
            ],
            public: &["vendorId"],
        },
        "tiktok" => Spec {
            fields: &[
                ("clientId", Str, true),
                ("clientSecret", Str, true),
                ("features", Object, false),
            ],
            public: &["clientId", "features"],
        },
        "make" => Spec {
            fields: &[("inviteUrl", Str, true)],
            public: &["inviteUrl"],
        },
        _ => return None,
    })
}

fn invalid(credential_type: &str, detail: String) -> BusinessError {
    BusinessError::exception(
        format!("Invalid {credential_type} credential: {detail}"),
        "invalidCredential",
        500,
    )
}

/// `credentialSchemas[type].parse(config)`: the known fields (unknown keys
/// stripped), required ones present, kinds checked.
pub fn parse_config(credential_type: &str, config: &Value) -> Result<Map<String, Value>> {
    let spec =
        spec(credential_type).ok_or_else(|| invalid(credential_type, "unknown type".into()))?;
    let object = config
        .as_object()
        .ok_or_else(|| invalid(credential_type, "expected object".into()))?;
    let mut out = Map::new();
    for &(field, kind, required) in spec.fields {
        match object.get(field) {
            None | Some(Value::Null) if !required => {}
            None | Some(Value::Null) => {
                return Err(invalid(credential_type, format!("{field} is required")));
            }
            Some(value) => {
                let ok = match kind {
                    Kind::Str => value.is_string(),
                    Kind::PosInt => value.as_i64().is_some_and(|n| n > 0),
                    Kind::Object => value.is_object(),
                };
                if !ok {
                    return Err(invalid(credential_type, format!("{field} is invalid")));
                }
                out.insert(field.to_string(), value.clone());
            }
        }
    }
    Ok(out)
}

/// `credentialPublicSchemas[type].parse(config)`.
pub fn public_config(credential_type: &str, config: &Map<String, Value>) -> Value {
    let Some(spec) = spec(credential_type) else {
        return Value::Object(Map::new());
    };
    let mut out: Map<String, Value> = spec
        .public
        .iter()
        .filter_map(|&k| config.get(k).map(|v| (k.to_string(), v.clone())))
        .collect();
    if credential_type == "googleAds" {
        let has = config
            .get("developerToken")
            .and_then(Value::as_str)
            .is_some_and(|t| !t.is_empty());
        out.insert("hasDeveloperToken".into(), Value::Bool(has));
    }
    Value::Object(out)
}

/// `DecryptedCredential`.
#[derive(Debug, Clone, PartialEq)]
pub struct DecryptedCredential {
    pub id: i64,
    pub user_id: Option<i64>,
    pub r#type: String,
    pub public_config: Value,
    pub config: Map<String, Value>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl DecryptedCredential {
    /// A string field of the config.
    pub fn text(&self, field: &str) -> Option<&str> {
        self.config.get(field).and_then(Value::as_str)
    }
}

/// `_decrypt`.
pub fn decrypt(encryption: &Encryption, row: &PlatformCredential) -> Result<DecryptedCredential> {
    let blob: EncryptedData = serde_json::from_value(row.value.0.clone())
        .map_err(|e| invalid(&row.r#type, e.to_string()))?;
    let user = row.user_id.map(|id| id.to_string());
    let aad = credential_aad_for(user.as_deref(), &row.r#type);
    let raw: Value = encryption
        .decrypt_object(&blob, Some(&aad))
        .map_err(|e| invalid(&row.r#type, e.to_string()))?;
    Ok(DecryptedCredential {
        id: row.id,
        user_id: row.user_id,
        r#type: row.r#type.clone(),
        public_config: row.public_config.0.clone(),
        config: parse_config(&row.r#type, &raw)?,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// `findForUser`.
pub fn find_for_user(
    conn: &Connection,
    user_id: i64,
    credential_type: &str,
) -> repo::Result<Option<PlatformCredential>> {
    find_one::<PlatformCredential>(
        conn,
        r#""userId" = ?1 AND "type" = ?2"#,
        &[&user_id, &credential_type],
    )
}

/// `findPlatform`.
pub fn find_platform(
    conn: &Connection,
    credential_type: &str,
) -> repo::Result<Option<PlatformCredential>> {
    find_one::<PlatformCredential>(
        conn,
        r#""userId" IS NULL AND "type" = ?1"#,
        &[&credential_type],
    )
}

/// `find`: `None` user = platform-scoped.
pub fn find(
    conn: &Connection,
    user_id: Option<i64>,
    credential_type: &str,
) -> repo::Result<Option<PlatformCredential>> {
    match user_id {
        Some(user) => find_for_user(conn, user, credential_type),
        None => find_platform(conn, credential_type),
    }
}

/// `findDecrypted*` options.
#[derive(Debug, Clone, Copy, Default)]
pub struct DecryptOptions {
    /// Ignore a user row that delegates to the platform.
    pub require_own_credential: bool,
    /// Rethrow read / decrypt failures instead of "no credential".
    pub strict: bool,
}

fn lenient(
    outcome: Result<Option<DecryptedCredential>>,
    strict: bool,
    what: &str,
) -> Result<Option<DecryptedCredential>> {
    match outcome {
        Err(err) if !strict => {
            tracing::error!(err = %err.message(), "{what}");
            Ok(None)
        }
        other => other,
    }
}

/// `findDecrypted` (`findDecryptedForUser` / `findDecryptedPlatform`).
pub fn find_decrypted(
    conn: &Connection,
    encryption: &Encryption,
    user_id: Option<i64>,
    credential_type: &str,
    options: DecryptOptions,
) -> Result<Option<DecryptedCredential>> {
    let outcome = (|| {
        let Some(row) = find(conn, user_id, credential_type)? else {
            return Ok(None);
        };
        if user_id.is_some() && options.require_own_credential && row.use_platform_credential {
            return Ok(None);
        }
        decrypt(encryption, &row).map(Some)
    })();
    lenient(outcome, options.strict, "Failed to decrypt credential")
}

/// `findDecryptedForWebhook`: an addressed user-owned row that does not
/// delegate to the platform.
pub fn find_decrypted_for_webhook(
    conn: &Connection,
    encryption: &Encryption,
    id: i64,
    credential_type: &str,
) -> Result<Option<DecryptedCredential>> {
    find_one::<PlatformCredential>(
        conn,
        r#""id" = ?1 AND "type" = ?2 AND "userId" IS NOT NULL AND "usePlatformCredential" = 0"#,
        &[&id, &credential_type],
    )?
    .map(|row| decrypt(encryption, &row))
    .transpose()
}

/// `findDecryptedById`.
pub fn find_decrypted_by_id(
    conn: &Connection,
    encryption: &Encryption,
    id: i64,
    credential_type: &str,
) -> Result<Option<DecryptedCredential>> {
    find_one::<PlatformCredential>(
        conn,
        r#""id" = ?1 AND "type" = ?2"#,
        &[&id, &credential_type],
    )?
    .map(|row| decrypt(encryption, &row))
    .transpose()
}

/// `findDecryptedThreadsByClientId`: a non-delegating `threads` row whose
/// public `clientId` matches; the platform row first, then by user and id.
pub fn find_decrypted_threads_by_client_id(
    conn: &Connection,
    encryption: &Encryption,
    client_id: &str,
) -> Option<DecryptedCredential> {
    let outcome = (|| {
        find_one::<PlatformCredential>(
            conn,
            r#""type" = 'threads' AND "usePlatformCredential" = 0 AND json_extract("publicConfig", '$.clientId') = ?1
               ORDER BY CASE WHEN "userId" IS NULL THEN 0 ELSE 1 END, "userId" ASC, "id" ASC"#,
            &[&client_id],
        )?
        .map(|row| decrypt(encryption, &row))
        .transpose()
    })();
    lenient(
        outcome,
        false,
        "Failed to resolve threads credential by clientId",
    )
    .ok()
    .flatten()
}

/// `resolvePlatformAppAccessToken`: `clientId|clientSecret` of the platform
/// Meta app.
pub fn resolve_platform_app_access_token(
    conn: &Connection,
    encryption: &Encryption,
    credential_type: &str,
) -> Result<Option<String>> {
    let credential = find_decrypted(
        conn,
        encryption,
        None,
        credential_type,
        DecryptOptions::default(),
    )?;
    Ok(credential.map(|c| {
        format!(
            "{}|{}",
            c.text("clientId").unwrap_or_default(),
            c.text("clientSecret").unwrap_or_default()
        )
    }))
}

/// `upsertForUser` / `upsertPlatform` input.
#[derive(Debug, Clone)]
pub struct UpsertCredential {
    /// The id of a new user-scoped row (`None` = a generated one); an
    /// existing row keeps its id.
    pub id: Option<i64>,
    pub user_id: Option<i64>,
    pub credential_type: String,
    pub config: Value,
    pub use_platform_credential: bool,
    pub is_verified: bool,
    pub verified_at: Option<i64>,
}

impl UpsertCredential {
    pub fn new(user_id: Option<i64>, credential_type: &str, config: Value) -> UpsertCredential {
        UpsertCredential {
            id: None,
            user_id,
            credential_type: credential_type.to_string(),
            config,
            use_platform_credential: false,
            is_verified: false,
            verified_at: None,
        }
    }
}

/// `upsert`: encrypts the config (AAD of the scope) and writes the row (the
/// one per scope and type).
pub fn upsert(conn: &Connection, encryption: &Encryption, input: UpsertCredential) -> Result<i64> {
    let config = parse_config(&input.credential_type, &input.config)?;
    let public = Json(public_config(&input.credential_type, &config));
    let user = input.user_id.map(|id| id.to_string());
    let aad = credential_aad_for(user.as_deref(), &input.credential_type);
    let value = encryption
        .encrypt_object(&config, Some(&aad))
        .map_err(|e| invalid(&input.credential_type, e.to_string()))?;
    let value = Json(
        serde_json::to_value(value).map_err(|e| invalid(&input.credential_type, e.to_string()))?,
    );
    let now = now_ms();
    let id: i64 = match input.user_id {
        Some(user_id) => conn.query_row(
            r#"INSERT INTO "PlatformCredential" ("id", "userId", "type", "publicConfig", "value", "usePlatformCredential", "isVerified", "verifiedAt", "createdAt", "updatedAt")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
               ON CONFLICT ("userId", "type") WHERE "userId" IS NOT NULL DO UPDATE SET
                 "publicConfig" = excluded."publicConfig", "value" = excluded."value",
                 "usePlatformCredential" = excluded."usePlatformCredential",
                 "isVerified" = excluded."isVerified", "verifiedAt" = excluded."verifiedAt",
                 "updatedAt" = excluded."updatedAt"
               RETURNING "id""#,
            params![
                input.id.unwrap_or_else(new_id),
                user_id,
                input.credential_type,
                public,
                value,
                input.use_platform_credential,
                input.is_verified,
                input.verified_at,
                now
            ],
            |row| row.get(0),
        )?,
        None => conn.query_row(
            r#"INSERT INTO "PlatformCredential" ("id", "type", "publicConfig", "value", "createdAt", "updatedAt")
               VALUES (?1, ?2, ?3, ?4, ?5, ?5)
               ON CONFLICT ("type") WHERE "userId" IS NULL DO UPDATE SET
                 "publicConfig" = excluded."publicConfig", "value" = excluded."value",
                 "updatedAt" = excluded."updatedAt"
               RETURNING "id""#,
            params![new_id(), input.credential_type, public, value, now],
            |row| row.get(0),
        )?,
    };
    Ok(id)
}

/// `remove`: deletes the scope's row of the type.
pub fn remove(conn: &Connection, user_id: Option<i64>, credential_type: &str) -> repo::Result<()> {
    match user_id {
        Some(user) => conn.execute(
            r#"DELETE FROM "PlatformCredential" WHERE "userId" = ?1 AND "type" = ?2"#,
            params![user, credential_type],
        )?,
        None => conn.execute(
            r#"DELETE FROM "PlatformCredential" WHERE "userId" IS NULL AND "type" = ?1"#,
            [credential_type],
        )?,
    };
    Ok(())
}

/// `resolveForOwner`: the owner's own (non-delegating) credential, else the
/// platform one.
pub fn resolve_for_owner(
    conn: &Connection,
    encryption: &Encryption,
    owner_id: i64,
    credential_type: &str,
    strict: bool,
) -> Result<Option<DecryptedCredential>> {
    let own = (|| {
        Ok(match find_for_user(conn, owner_id, credential_type)? {
            Some(row) if !row.use_platform_credential => Some(decrypt(encryption, &row)?),
            _ => None,
        })
    })();
    match own {
        Ok(Some(credential)) => return Ok(Some(credential)),
        Ok(None) => {}
        Err(err) if strict => return Err(err),
        Err(err) => {
            tracing::error!(err = %err.message(), "Failed to resolve own credential");
            return Ok(None);
        }
    }
    find_decrypted(
        conn,
        encryption,
        None,
        credential_type,
        DecryptOptions {
            strict,
            ..Default::default()
        },
    )
}

/// `resolveWhatsappSystemUserToken`.
pub fn resolve_whatsapp_system_user_token(
    conn: &Connection,
    encryption: &Encryption,
    owner_id: i64,
) -> Result<Option<String>> {
    Ok(
        resolve_for_owner(conn, encryption, owner_id, "whatsapp", false)?
            .and_then(|c| c.text("systemUserToken").map(str::to_string)),
    )
}

/// `_publicWithDerivedFlags`: a `googleAds` row saved before
/// `hasDeveloperToken` existed gets the flag from its secret.
fn public_with_derived_flags(encryption: &Encryption, row: &PlatformCredential) -> Value {
    let stored = row.public_config.0.clone();
    if row.r#type != "googleAds"
        || stored
            .get("hasDeveloperToken")
            .is_some_and(Value::is_boolean)
    {
        return stored;
    }
    match decrypt(encryption, row) {
        Ok(credential) => {
            let mut out = stored.as_object().cloned().unwrap_or_default();
            let has = credential
                .text("developerToken")
                .is_some_and(|t| !t.is_empty());
            out.insert("hasDeveloperToken".into(), Value::Bool(has));
            Value::Object(out)
        }
        Err(err) => {
            tracing::error!(err = %err.message(), credential_id = row.id, "Failed to derive credential public flags");
            stored
        }
    }
}

/// `resolvePublicForUser`: the user's own public config, else the inherited
/// platform one.
#[derive(Debug, Clone, PartialEq)]
pub struct PublicCredential {
    pub credential_id: Option<i64>,
    pub public_config: Value,
    pub is_inherited: bool,
}

pub fn resolve_public_for_user(
    conn: &Connection,
    encryption: &Encryption,
    user_id: i64,
    credential_type: &str,
) -> repo::Result<Option<PublicCredential>> {
    if let Some(row) = find_for_user(conn, user_id, credential_type)?
        && !row.use_platform_credential
    {
        return Ok(Some(PublicCredential {
            credential_id: Some(row.id),
            public_config: public_with_derived_flags(encryption, &row),
            is_inherited: false,
        }));
    }
    Ok(
        find_platform(conn, credential_type)?.map(|row| PublicCredential {
            credential_id: None,
            public_config: public_with_derived_flags(encryption, &row),
            is_inherited: true,
        }),
    )
}

/// `findPlatformPublic`.
pub fn find_platform_public(
    conn: &Connection,
    encryption: &Encryption,
    credential_type: &str,
) -> repo::Result<Option<Value>> {
    Ok(
        find_platform(conn, credential_type)?
            .map(|row| public_with_derived_flags(encryption, &row)),
    )
}

/// `hasOwnCredentialAccess`.
pub fn has_own_credential_access(
    tenant_status: Option<&str>,
    custom_app_credentials: bool,
) -> bool {
    tenant_status == Some("active") || custom_app_credentials
}

/// `canUseOwnCredentials`: an active tenant owner, else the effective
/// quota's `customAppCredentials` (default-plan overlay included).
pub async fn can_use_own_credentials(
    db: &chatbotx_db::Db,
    kv: Option<&chatbotx_kv::durable::DurableKv>,
    edition: crate::keys::Edition,
    owner_id: i64,
) -> Result<bool> {
    let tenant = crate::tx::read(db, move |conn| {
        Ok(repo::tenant::find_by_owner(conn, owner_id)?)
    })
    .await?;
    if tenant.as_ref().is_some_and(|t| t.status == "active") {
        return Ok(true);
    }
    let quota = crate::user_quota::get_effective_for_user(db, kv, edition, owner_id).await?;
    Ok(has_own_credential_access(
        tenant.as_ref().map(|t| t.status.as_str()),
        quota.is_some_and(|q| q.custom_app_credentials),
    ))
}
