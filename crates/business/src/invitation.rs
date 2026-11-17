//! `invitation/service.ts`: `create` and `buildInviteUrl` (Q.D11 slice 2).

use base64::Engine as _;
use chatbotx_db::Db;
use chatbotx_db::repo::{new_id, now_ms};
use serde_json::Value;

use crate::{Result, tx};

/// `INVITATION_TTL_DAYS`: 1.
pub const INVITATION_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// `generateInvitationCode`: 16 CSPRNG bytes, base64url without padding.
pub fn generate_invitation_code() -> String {
    let mut bytes = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .expect("system random");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A new invitation.
#[derive(Debug, Clone, PartialEq)]
pub struct Invitation {
    pub id: i64,
    pub code: String,
    pub expires_at: i64,
}

/// `create`: a shareable code for the workspace, valid one day.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    invited_by: i64,
    permissions: Value,
) -> Result<Invitation> {
    tx::write(db, move |tx| {
        let invitation = Invitation {
            id: new_id(),
            code: generate_invitation_code(),
            expires_at: now_ms() + INVITATION_TTL_MS,
        };
        tx.execute(
            r#"INSERT INTO "Invitation" ("id", "code", "permissions", "expiresAt", "workspaceId", "invitedBy")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            chatbotx_db::rusqlite::params![
                invitation.id,
                invitation.code,
                permissions.to_string(),
                invitation.expires_at,
                workspace_id,
                invited_by
            ],
        )
        .map_err(chatbotx_db::repo::Error::from)?;
        Ok(invitation)
    })
    .await
}

/// `buildInviteUrl`: `<appUrl>/invitations/<code>` (one tenant: the app URL
/// serves every workspace).
pub fn build_invite_url(app_url: &str, code: &str) -> String {
    let encoded: String = code
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => (b as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect();
    format!("{}/invitations/{encoded}", app_url.trim_end_matches('/'))
}
