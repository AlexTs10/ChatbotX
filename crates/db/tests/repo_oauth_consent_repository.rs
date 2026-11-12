//! Port of `packages/database/__tests__/oauth-consent.repository.test.ts` (P3.3). The
//! rendered predicates (`userId = $1`, `expiresAt > now`, `revoked is null`) and the date
//! decoders become the result over real consent and refresh-token rows.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::oauth_consent::{GivenConsent, list_given_by_user};
use chatbotx_db::repo::{self, Json};
use common::{SEED_CLIENT_ID, fresh_db, seed};
use repo_common::{ms, ok, setup};
use serde_json::json;

/// Port of: packages/database/__tests__/oauth-consent.repository.test.ts:36 "oauthConsentRepository.listGivenByUser > lists given consents and aggregates only live refresh tokens"
#[tokio::test]
async fn oauth_consent_repository_list_given_by_user_lists_given_consents_and_aggregates_only_live_refresh_tokens()
 {
    let db = fresh_db().await;
    let s = seed(&db).await;
    let now = ms("2026-10-29T00:00:00.000Z");
    setup(&db, move |tx| {
        ins!(tx, "User", { "id" => 1002_i64, "email" => "other@example.com" })?;
        tx.execute(
            "UPDATE \"OAuthClient\" SET \"name\" = 'Client One', \"icon\" = 'icon.png' WHERE \"clientId\" = ?1",
            [SEED_CLIENT_ID],
        )?;
        ins!(tx, "OAuthClient", { "id" => 5002_i64, "clientId" => "client-2", "redirectUris" => "[]", "name" => "Client Two" })?;
        ins!(tx, "OAuthConsent", { "id" => 1_i64, "createdAt" => ms("2026-10-01T00:00:00.000Z"), "clientId" => SEED_CLIENT_ID, "userId" => s.user_id, "scopes" => r#"["openid"]"# })?;
        ins!(tx, "OAuthConsent", { "id" => 2_i64, "createdAt" => ms("2026-10-02T00:00:00.000Z"), "clientId" => "client-2", "userId" => s.user_id, "scopes" => r#"["mcp"]"# })?;
        // Another user's consent: not listed.
        ins!(tx, "OAuthConsent", { "id" => 3_i64, "clientId" => SEED_CLIENT_ID, "userId" => 1002_i64, "scopes" => "[]" })?;
        // Live tokens of client-1.
        ins!(tx, "OAuthRefreshToken", { "id" => 11_i64, "createdAt" => ms("2026-10-28T00:00:00.000Z"), "token" => "a", "clientId" => SEED_CLIENT_ID, "userId" => s.user_id, "expiresAt" => ms("2026-11-01T00:00:00.000Z"), "scopes" => "[]" })?;
        ins!(tx, "OAuthRefreshToken", { "id" => 12_i64, "createdAt" => ms("2026-10-30T00:00:00.000Z"), "token" => "b", "clientId" => SEED_CLIENT_ID, "userId" => s.user_id, "expiresAt" => ms("2026-10-31T00:00:00.000Z"), "scopes" => "[]" })?;
        // Expired, revoked and other-user tokens: not aggregated.
        ins!(tx, "OAuthRefreshToken", { "id" => 13_i64, "createdAt" => ms("2026-10-31T00:00:00.000Z"), "token" => "c", "clientId" => SEED_CLIENT_ID, "userId" => s.user_id, "expiresAt" => now, "scopes" => "[]" })?;
        ins!(tx, "OAuthRefreshToken", { "id" => 14_i64, "createdAt" => ms("2026-10-31T00:00:00.000Z"), "token" => "d", "clientId" => SEED_CLIENT_ID, "userId" => s.user_id, "expiresAt" => ms("2026-12-01T00:00:00.000Z"), "revoked" => now, "scopes" => "[]" })?;
        ins!(tx, "OAuthRefreshToken", { "id" => 15_i64, "createdAt" => ms("2026-10-31T00:00:00.000Z"), "token" => "e", "clientId" => SEED_CLIENT_ID, "userId" => 1002_i64, "expiresAt" => ms("2026-12-01T00:00:00.000Z"), "scopes" => "[]" })?;
        // client-2 has only an expired token.
        ins!(tx, "OAuthRefreshToken", { "id" => 16_i64, "createdAt" => ms("2026-10-03T00:00:00.000Z"), "token" => "f", "clientId" => "client-2", "userId" => s.user_id, "expiresAt" => ms("2026-10-04T00:00:00.000Z"), "scopes" => "[]" })?;
        Ok(())
    })
    .await;

    let user_id = s.user_id;
    let listed = ok(repo::read(&db, move |c| list_given_by_user(c, user_id, now)).await);
    assert_eq!(
        listed,
        vec![
            GivenConsent {
                active_until: None,
                client_icon: None,
                client_id: "client-2".to_owned(),
                client_name: Some("Client Two".to_owned()),
                first_authorized_at: ms("2026-10-02T00:00:00.000Z"),
                last_refreshed_at: None,
                scopes: Json(json!(["mcp"])),
            },
            GivenConsent {
                active_until: Some(ms("2026-11-01T00:00:00.000Z")),
                client_icon: Some("icon.png".to_owned()),
                client_id: SEED_CLIENT_ID.to_owned(),
                client_name: Some("Client One".to_owned()),
                first_authorized_at: ms("2026-10-01T00:00:00.000Z"),
                last_refreshed_at: Some(ms("2026-10-30T00:00:00.000Z")),
                scopes: Json(json!(["openid"])),
            },
        ]
    );

    // No consent at all: an empty list.
    let none = ok(repo::read(&db, move |c| list_given_by_user(c, 1002 + 1, now)).await);
    assert_eq!(none, vec![]);
}
