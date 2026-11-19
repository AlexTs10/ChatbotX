//! Port of `apps/builder/__tests__/whatsapp-voip-turn-credentials-action.test.ts`
//! (the credential part; the access cases are in the api `whatsapp_calling_actions.rs`).
use chatbotx_business::whatsapp_voip::{
    VOIP_TURN_CREDENTIAL_TTL_SECONDS, issue_turn_credentials, mint_turn_credential,
};

#[test]
fn issues_credentials_scoped_to_the_caller_and_the_call() {
    let out = issue_turn_credentials(7, "wacid.1", Some("turn:turn.example:3478"), Some("s3cret"));
    assert_eq!(out["turnConfigured"], true);
    let turn = &out["iceServers"][1];
    assert_eq!(turn["urls"], "turn:turn.example:3478");
    let username = turn["username"].as_str().unwrap();
    assert!(username.ends_with(":7:wacid.1"), "{username}");
}

#[test]
fn mints_the_coturn_rest_credential() {
    let (username, credential) =
        mint_turn_credential("secret", "7:w", VOIP_TURN_CREDENTIAL_TTL_SECONDS, 1_000_000);
    assert_eq!(
        username,
        format!("{}:7:w", 1_000 + VOIP_TURN_CREDENTIAL_TTL_SECONDS)
    );
    // base64(HMAC-SHA1) is 28 characters.
    assert_eq!(credential.len(), 28);
}

#[test]
fn falls_back_to_stun_only_when_turn_is_unconfigured() {
    let out = issue_turn_credentials(7, "w", None, Some("s"));
    assert_eq!(out["turnConfigured"], false);
    assert_eq!(out["iceServers"].as_array().unwrap().len(), 1);
}
