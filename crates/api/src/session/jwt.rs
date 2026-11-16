//! better-auth `jwt()` plugin: EdDSA (Ed25519) keys in `"Jwk"`, `GET /api/auth/jwks`,
//! `signJWT` and the JWKS check of `verifyJwsAccessToken`.
//!
//! The private JWK is stored as better-auth writes it: `JSON.stringify(
//! symmetricEncrypt(secret, JSON.stringify(jwk)))`, i.e. a JSON string of
//! hex(nonce24 || XChaCha20-Poly1305(SHA-256(secret), jwk)) (an optional
//! `$ba$<version>$` envelope is accepted on read).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair as _, UnparsedPublicKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::email_password::{new_id, random_bytes};
use super::{BearerTokenVerifier, BearerVerifyError, VerifyBearerOptions};

/// A decrypted signing key.
pub struct SigningKey {
    pub kid: String,
    seed: [u8; 32],
}

fn cipher(secret: &str) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new_from_slice(&Sha256::digest(secret.as_bytes())).expect("32-byte key")
}

/// `symmetricEncrypt({ key: secret, data })` (bare hex, no envelope).
pub fn symmetric_encrypt(secret: &str, data: &str) -> String {
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let sealed = cipher(secret)
        .encrypt(&nonce, data.as_bytes())
        .expect("encryption of an in-memory buffer");
    let mut out = nonce.to_vec();
    out.extend(sealed);
    out.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `symmetricDecrypt`: bare hex or a `$ba$<v>$<hex>` envelope.
pub fn symmetric_decrypt(secret: &str, data: &str) -> Option<String> {
    let hex = match data.strip_prefix("$ba$") {
        Some(rest) => rest.split_once('$')?.1,
        None => data,
    };
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    if bytes.len() < 24 {
        return None;
    }
    let (nonce, sealed) = bytes.split_at(24);
    let plain = cipher(secret)
        .decrypt(XNonce::from_slice(nonce), sealed)
        .ok()?;
    String::from_utf8(plain).ok()
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The private JWK of a stored row.
fn decode_private_key(secret: &str, stored: &str) -> Option<[u8; 32]> {
    let jwk: Value = match serde_json::from_str::<Value>(stored).ok()? {
        Value::String(encrypted) => {
            serde_json::from_str(&symmetric_decrypt(secret, &encrypted)?).ok()?
        }
        // `disablePrivateKeyEncryption`.
        object @ Value::Object(_) => object,
        _ => return None,
    };
    URL_SAFE_NO_PAD
        .decode(jwk["d"].as_str()?)
        .ok()?
        .try_into()
        .ok()
}

/// `createJwk`: a new Ed25519 key row.
fn create_jwk(conn: &Connection, secret: &str, now_ms: i64) -> Result<SigningKey, DbError> {
    let seed: [u8; 32] = random_bytes(32).try_into().expect("32 bytes");
    let pair = Ed25519KeyPair::from_seed_unchecked(&seed)
        .map_err(|_| DbError::other("cannot derive an Ed25519 key"))?;
    let x = b64(pair.public_key().as_ref());
    let public = json!({ "crv": "Ed25519", "x": x, "kty": "OKP" });
    let private = json!({ "crv": "Ed25519", "d": b64(&seed), "x": x, "kty": "OKP" });
    let encrypted = Value::String(symmetric_encrypt(secret, &private.to_string())).to_string();
    let id = new_id();
    conn.execute(
        r#"INSERT INTO "Jwk" ("id", "createdAt", "updatedAt", "publicKey", "privateKey", "alg", "crv")
           VALUES (?1, ?2, ?2, ?3, ?4, 'EdDSA', 'Ed25519')"#,
        params![id, now_ms, public.to_string(), encrypted],
    )?;
    Ok(SigningKey {
        kid: id.to_string(),
        seed,
    })
}

/// `resolveSigningKey`: the latest live EdDSA key, else a new one.
pub async fn signing_key(db: &Db, secret: &str, now_ms: i64) -> Result<SigningKey, DbError> {
    let secret = secret.to_string();
    db.write(move |tx| {
        let latest: Option<(i64, String)> = {
            let mut statement = tx.prepare(
                r#"SELECT "id", "privateKey" FROM "Jwk"
                   WHERE ("alg" = 'EdDSA' OR "alg" IS NULL) AND ("expiresAt" IS NULL OR "expiresAt" > ?1)
                   ORDER BY "createdAt" DESC, "id" DESC LIMIT 1"#,
            )?;
            let mut rows = statement.query_map([now_ms], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.next().transpose()?
        };
        if let Some((id, stored)) = latest {
            if let Some(seed) = decode_private_key(&secret, &stored) {
                return Ok(SigningKey {
                    kid: id.to_string(),
                    seed,
                });
            }
            tracing::error!(kid = id, "cannot decrypt the JWKS signing key; minting a new one");
        }
        create_jwk(tx, &secret, now_ms)
    })
    .await
}

/// `signJWT`: `header.payload.signature` with `{ alg: "EdDSA", kid }`.
pub fn sign_jwt(key: &SigningKey, payload: &Value) -> String {
    let pair = Ed25519KeyPair::from_seed_unchecked(&key.seed).expect("valid seed");
    let header = json!({ "alg": "EdDSA", "kid": key.kid });
    let signing_input = format!(
        "{}.{}",
        b64(header.to_string().as_bytes()),
        b64(payload.to_string().as_bytes())
    );
    let signature = pair.sign(signing_input.as_bytes());
    format!("{signing_input}.{}", b64(signature.as_ref()))
}

type JwkRow = (i64, String, Option<String>, Option<String>);

async fn jwk_rows(db: &Db, now_ms: i64) -> Result<Vec<JwkRow>, DbError> {
    db.read(move |conn| {
        let mut statement = conn.prepare(
            r#"SELECT "id", "publicKey", "alg", "crv" FROM "Jwk"
               WHERE "expiresAt" IS NULL OR "expiresAt" + 2592000000 > ?1 ORDER BY "createdAt""#,
        )?;
        let rows = statement.query_map([now_ms], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })
    .await
}

/// `GET /jwks`: `{ keys: [{ alg, crv, ...publicKey, kid }] }`, minting a key
/// when there is none (30-day grace for expired keys).
pub async fn jwks(db: &Db, secret: &str, now_ms: i64) -> Result<Value, DbError> {
    let mut rows = jwk_rows(db, now_ms).await?;
    if rows.is_empty() {
        signing_key(db, secret, now_ms).await?;
        rows = jwk_rows(db, now_ms).await?;
    }
    let keys = rows
        .into_iter()
        .map(|(id, public_key, alg, crv)| {
            let mut key = json!({ "alg": alg.unwrap_or_else(|| "EdDSA".into()) });
            if let Some(crv) = crv {
                key["crv"] = json!(crv);
            }
            if let Ok(Value::Object(public)) = serde_json::from_str::<Value>(&public_key) {
                for (name, value) in public {
                    key[name] = value;
                }
            }
            key["kid"] = json!(id.to_string());
            key
        })
        .collect::<Vec<_>>();
    Ok(json!({ "keys": keys }))
}

fn jose(code: &str) -> BearerVerifyError {
    BearerVerifyError::Jose(code.to_string())
}

/// `verifyJwsAccessToken` against the stored keys: signature, `iss`, `aud`, `exp`.
pub async fn verify_jwt(
    db: &Db,
    token: &str,
    options: &VerifyBearerOptions,
    now_ms: i64,
) -> Result<Value, BearerVerifyError> {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(jose("ERR_JWS_INVALID"));
    };
    let decode = |segment: &str| {
        URL_SAFE_NO_PAD
            .decode(segment)
            .map_err(|_| jose("ERR_JWS_INVALID"))
    };
    let header: Value =
        serde_json::from_slice(&decode(header)?).map_err(|_| jose("ERR_JWS_INVALID"))?;
    if header["alg"] != "EdDSA" {
        return Err(jose("ERR_JOSE_ALG_NOT_ALLOWED"));
    }
    let kid: i64 = header["kid"]
        .as_str()
        .and_then(|kid| kid.parse().ok())
        .ok_or_else(|| jose("ERR_JWKS_NO_MATCHING_KEY"))?;
    let public: Option<String> = db
        .read(move |conn| {
            let mut statement = conn.prepare(r#"SELECT "publicKey" FROM "Jwk" WHERE "id" = ?1"#)?;
            let mut rows = statement.query_map([kid], |row| row.get(0))?;
            Ok(rows.next().transpose()?)
        })
        .await
        .map_err(|error| BearerVerifyError::Other(error.to_string()))?;
    let x = public
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|jwk| URL_SAFE_NO_PAD.decode(jwk["x"].as_str()?).ok())
        .ok_or_else(|| jose("ERR_JWKS_NO_MATCHING_KEY"))?;
    let signing_input = &token[..token.len() - signature.len() - 1];
    UnparsedPublicKey::new(&ED25519, x)
        .verify(signing_input.as_bytes(), &decode(signature)?)
        .map_err(|_| jose("ERR_JWS_SIGNATURE_VERIFICATION_FAILED"))?;
    let claims: Value =
        serde_json::from_slice(&decode(payload)?).map_err(|_| jose("ERR_JWT_INVALID"))?;
    if claims["iss"] != options.issuer.as_str() {
        return Err(jose("ERR_JWT_CLAIM_VALIDATION_FAILED"));
    }
    let audience_ok = match &claims["aud"] {
        Value::String(aud) => aud == &options.audience,
        Value::Array(auds) => auds.iter().any(|aud| aud == options.audience.as_str()),
        _ => false,
    };
    if !audience_ok {
        return Err(jose("ERR_JWT_CLAIM_VALIDATION_FAILED"));
    }
    match claims["exp"].as_i64() {
        Some(exp) if exp > now_ms / 1000 => Ok(claims),
        _ => Err(jose("ERR_JWT_EXPIRED")),
    }
}

/// The production `BearerTokenVerifier`: the local JWKS (no HTTP fetch).
pub struct DbJwtVerifier<'a> {
    pub db: &'a Db,
    pub now_ms: i64,
}

impl BearerTokenVerifier for DbJwtVerifier<'_> {
    async fn verify_bearer_token(
        &self,
        token: &str,
        options: &VerifyBearerOptions,
    ) -> Result<Value, BearerVerifyError> {
        verify_jwt(self.db, token, options, self.now_ms).await
    }
}
