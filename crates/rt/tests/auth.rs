//! Port of `packages/realtime-protocol/__tests__/auth.test.ts` (P2.8 spec
//! 4.6, R1-R29). `Date.now()` is a `TestClock`; `jose` `new SignJWT(..)`
//! of raw claims is `support::sign_raw_jwt` (HS256, independent of the
//! crate's signer). R10, R11 and R13 (`issuedAtMs` NaN, Infinity, 1.5) are
//! not applicable (user decision 2026-11-12, DECISIONS P2.8): the Rust
//! parameter is an integer.

mod support;

use chatbotx_rt::TestClock;
use chatbotx_rt::token::{
    Audience, ChatScope, GuestClaims, MEMBER_REVOKED_TTL_SECS, MemberConnect, extract_bearer_token,
    purpose, sign_guest_connect_token, sign_member_connect_token, sign_presence_report_token,
    verify_guest_connect_token, verify_member_connect_token, verify_presence_report_token,
    verify_realtime_token,
};
use serde_json::json;
use support::{secs, sign_raw_jwt};

const SECRET: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_SECRET: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
/// `Date.now()` of these cases.
const NOW_MS: i64 = 1_790_000_000_123;

fn clock() -> TestClock {
    TestClock::new(NOW_MS)
}

fn member(
    workspace_id: &str,
    user_id: &str,
    chat_scope: ChatScope,
    issued_at_ms: i64,
) -> MemberConnect {
    MemberConnect {
        workspace_id: workspace_id.to_owned(),
        user_id: user_id.to_owned(),
        chat_scope,
        team_ids: Vec::new(),
        issued_at_ms,
    }
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:19 "keeps a revocation marker valid for every still-tolerated token"
#[test]
fn keeps_a_revocation_marker_valid_for_every_still_tolerated_token() {
    assert_eq!(MEMBER_REVOKED_TTL_SECS, 65);
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:24 "signPresenceReportToken / verifyPresenceReportToken (generic sign/verify mechanics) > verifies a token signed for the same audience and purpose"
#[test]
fn signpresencereporttoken_verifypresencereporttoken_generic_sign_verify_mechanics_verifies_a_token_signed_for_the_same_audience_and_purpose()
 {
    let clock = clock();
    let token = sign_presence_report_token("ws_1", "hash-1", SECRET, &clock);

    assert!(verify_presence_report_token(&token, "ws_1", SECRET, &clock).is_ok());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:35 "signPresenceReportToken / verifyPresenceReportToken (generic sign/verify mechanics) > rejects when the audience id does not match (room-claim mismatch)"
#[test]
fn signpresencereporttoken_verifypresencereporttoken_generic_sign_verify_mechanics_rejects_when_the_audience_id_does_not_match_room_claim_mismatch()
 {
    let clock = clock();
    let token = sign_presence_report_token("ws_1", "hash-1", SECRET, &clock);

    assert!(verify_presence_report_token(&token, "ws_2", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:46 "signPresenceReportToken / verifyPresenceReportToken (generic sign/verify mechanics) > rejects when signed with a different secret"
#[test]
fn signpresencereporttoken_verifypresencereporttoken_generic_sign_verify_mechanics_rejects_when_signed_with_a_different_secret()
 {
    let clock = clock();
    let token = sign_presence_report_token("ws_1", "hash-1", SECRET, &clock);

    assert!(verify_presence_report_token(&token, "ws_1", OTHER_SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:57 "signPresenceReportToken / verifyPresenceReportToken (generic sign/verify mechanics) > carries its claim through the payload"
#[test]
fn signpresencereporttoken_verifypresencereporttoken_generic_sign_verify_mechanics_carries_its_claim_through_the_payload()
 {
    let clock = clock();
    let token = sign_presence_report_token("ws_1", "hash-1", SECRET, &clock);

    let claims = verify_presence_report_token(&token, "ws_1", SECRET, &clock).unwrap();

    assert_eq!(claims.body_hash, "hash-1");
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:68 "signPresenceReportToken / verifyPresenceReportToken (generic sign/verify mechanics) > embeds the purpose claim in the signed payload (undecoded)"
#[test]
fn signpresencereporttoken_verifypresencereporttoken_generic_sign_verify_mechanics_embeds_the_purpose_claim_in_the_signed_payload_undecoded()
 {
    let clock = clock();
    let token = sign_presence_report_token("ws_1", "hash-1", SECRET, &clock);

    let payload = verify_realtime_token(
        &token,
        Audience::workspace("ws_1"),
        purpose::PRESENCE_REPORT,
        SECRET,
        &clock,
    )
    .unwrap();

    assert_eq!(
        payload.get("purpose"),
        Some(&json!(purpose::PRESENCE_REPORT))
    );
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:84 "signPresenceReportToken / verifyPresenceReportToken (generic sign/verify mechanics) > rejects a token minted for a different purpose — a member-connect token must not verify as a presence-report token (MEDIUM-3)"
#[test]
fn signpresencereporttoken_verifypresencereporttoken_generic_sign_verify_mechanics_rejects_a_token_minted_for_a_different_purpose_a_member_connect_token_must_not_verify_as_a_presence_report_token_medium_3()
 {
    let clock = clock();
    let token =
        sign_member_connect_token(&member("ws_1", "u_1", ChatScope::All, NOW_MS), SECRET).unwrap();

    assert!(verify_presence_report_token(&token, "ws_1", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:100 "signPresenceReportToken / verifyPresenceReportToken (generic sign/verify mechanics) > rejects a purpose-less token"
#[test]
fn signpresencereporttoken_verifypresencereporttoken_generic_sign_verify_mechanics_rejects_a_purpose_less_token()
 {
    let clock = clock();
    let legacy_token = sign_raw_jwt(
        &json!({
            "iat": secs(NOW_MS),
            "aud": "workspace:ws_1",
            "exp": secs(NOW_MS) + 60,
        }),
        SECRET,
    );

    assert!(verify_presence_report_token(&legacy_token, "ws_1", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:115 "signMemberConnectToken / verifyMemberConnectToken > preserves the pre-membership-read millisecond issue time"
#[test]
fn signmemberconnecttoken_verifymemberconnecttoken_preserves_the_pre_membership_read_millisecond_issue_time()
 {
    let clock = clock();
    let issued_at_ms = NOW_MS - 500;
    let token =
        sign_member_connect_token(&member("ws_1", "u_1", ChatScope::All, issued_at_ms), SECRET)
            .unwrap();
    let claims = verify_member_connect_token(&token, "ws_1", SECRET, &clock).unwrap();

    assert_eq!(claims.user_id, "u_1");
    assert_eq!(claims.chat_scope, ChatScope::All);
    assert_eq!(claims.team_ids, Vec::<String>::new());
    assert!(claims.iat.is_finite());
    assert_eq!(claims.iat_ms, issued_at_ms as u64);
    assert_eq!(claims.iat_ms % 1000, (issued_at_ms % 1000) as u64);
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:138 "signMemberConnectToken / verifyMemberConnectToken > rejects invalid issuedAtMs -1"
#[test]
fn signmemberconnecttoken_verifymemberconnecttoken_rejects_invalid_issuedatms_1() {
    assert!(sign_member_connect_token(&member("ws_1", "u_1", ChatScope::All, -1), SECRET).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:157 "signMemberConnectToken / verifyMemberConnectToken > preserves assigned-team scope in a member token"
#[test]
fn signmemberconnecttoken_verifymemberconnecttoken_preserves_assigned_team_scope_in_a_member_token()
{
    let clock = clock();
    let mut input = member("ws_1", "u_1", ChatScope::Assigned, NOW_MS);
    input.team_ids = vec!["team_1".to_owned(), "team_2".to_owned()];
    let token = sign_member_connect_token(&input, SECRET).unwrap();

    let claims = verify_member_connect_token(&token, "ws_1", SECRET, &clock).unwrap();

    assert_eq!(claims.user_id, "u_1");
    assert_eq!(claims.chat_scope, ChatScope::Assigned);
    assert_eq!(
        claims.team_ids,
        vec!["team_1".to_owned(), "team_2".to_owned()]
    );
    assert!(claims.iat.is_finite());
    assert_eq!(claims.iat_ms, NOW_MS as u64);
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:180 "signMemberConnectToken / verifyMemberConnectToken > rejects a cross-room replay — token minted for a different workspace"
#[test]
fn signmemberconnecttoken_verifymemberconnecttoken_rejects_a_cross_room_replay_token_minted_for_a_different_workspace()
 {
    let clock = clock();
    let token =
        sign_member_connect_token(&member("ws_1", "u_1", ChatScope::All, NOW_MS), SECRET).unwrap();

    assert!(verify_member_connect_token(&token, "ws_2", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:196 "signMemberConnectToken / verifyMemberConnectToken > rejects a token missing the userId claim"
#[test]
fn signmemberconnecttoken_verifymemberconnecttoken_rejects_a_token_missing_the_userid_claim() {
    let clock = clock();
    // Right purpose/audience but no claims at all: must never be trusted.
    let token = sign_raw_jwt(
        &json!({
            "purpose": purpose::MEMBER_CONNECT,
            "iat": secs(NOW_MS),
            "aud": "workspace:ws_1",
            "exp": secs(NOW_MS) + 60,
        }),
        SECRET,
    );

    assert!(verify_member_connect_token(&token, "ws_1", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:214 "signMemberConnectToken / verifyMemberConnectToken > rejects a member token missing the millisecond mint time"
#[test]
fn signmemberconnecttoken_verifymemberconnecttoken_rejects_a_member_token_missing_the_millisecond_mint_time()
 {
    let clock = clock();
    let token = sign_raw_jwt(
        &json!({
            "chatScope": "all",
            "purpose": purpose::MEMBER_CONNECT,
            "teamIds": [],
            "userId": "u_1",
            "iat": secs(NOW_MS),
            "aud": "workspace:ws_1",
            "exp": secs(NOW_MS) + 60,
        }),
        SECRET,
    );

    assert!(verify_member_connect_token(&token, "ws_1", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:232 "signMemberConnectToken / verifyMemberConnectToken > rejects a token signed with a different secret"
#[test]
fn signmemberconnecttoken_verifymemberconnecttoken_rejects_a_token_signed_with_a_different_secret()
{
    let clock = clock();
    let token =
        sign_member_connect_token(&member("ws_1", "u_1", ChatScope::All, NOW_MS), SECRET).unwrap();

    assert!(verify_member_connect_token(&token, "ws_1", OTHER_SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:248 "signMemberConnectToken / verifyMemberConnectToken > rejects a purpose-less token — member-connect requires a purpose claim"
#[test]
fn signmemberconnecttoken_verifymemberconnecttoken_rejects_a_purpose_less_token_member_connect_requires_a_purpose_claim()
 {
    let clock = clock();
    let legacy_shaped_token = sign_raw_jwt(
        &json!({
            "userId": "u_1",
            "iat": secs(NOW_MS),
            "aud": "workspace:ws_1",
            "exp": secs(NOW_MS) + 60,
        }),
        SECRET,
    );

    assert!(verify_member_connect_token(&legacy_shaped_token, "ws_1", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:261 "signMemberConnectToken / verifyMemberConnectToken > a freshly minted member-connect token must never verify as a presence report — no cross-purpose confusion despite their shared workspace:<id> audience"
#[test]
fn signmemberconnecttoken_verifymemberconnecttoken_a_freshly_minted_member_connect_token_must_never_verify_as_a_presence_report_no_cross_purpose_confusion_despite_their_shared_workspace_id_audience()
 {
    let clock = clock();
    let token =
        sign_member_connect_token(&member("ws_1", "u_1", ChatScope::All, NOW_MS), SECRET).unwrap();

    assert!(
        verify_realtime_token(
            &token,
            Audience::workspace("ws_1"),
            purpose::PRESENCE_REPORT,
            SECRET,
            &clock,
        )
        .is_err()
    );
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:282 "signMemberConnectToken / verifyMemberConnectToken > a freshly minted presence-report token must never verify as a member-connect token — same cross-purpose confusion, reversed"
#[test]
fn signmemberconnecttoken_verifymemberconnecttoken_a_freshly_minted_presence_report_token_must_never_verify_as_a_member_connect_token_same_cross_purpose_confusion_reversed()
 {
    let clock = clock();
    let token = sign_presence_report_token("ws_1", "hash-1", SECRET, &clock);

    assert!(verify_member_connect_token(&token, "ws_1", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:294 "rejects an expired member token"
#[test]
fn rejects_an_expired_member_token() {
    let clock = clock();
    let token = sign_raw_jwt(
        &json!({
            "chatScope": "all",
            "iatMs": NOW_MS,
            "purpose": purpose::MEMBER_CONNECT,
            "teamIds": [],
            "userId": "u_1",
            "iat": secs(NOW_MS),
            "aud": "workspace:ws_1",
            "exp": secs(NOW_MS) - 10,
        }),
        SECRET,
    );

    assert!(verify_member_connect_token(&token, "ws_1", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:313 "rejects an expired presence-report token"
#[test]
fn rejects_an_expired_presence_report_token() {
    let clock = clock();
    let token = sign_raw_jwt(
        &json!({
            "bodyHash": "hash-1",
            "purpose": purpose::PRESENCE_REPORT,
            "aud": "workspace:ws_1",
            "exp": secs(NOW_MS) - 10,
        }),
        SECRET,
    );

    assert!(verify_presence_report_token(&token, "ws_1", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:329 "signGuestConnectToken / verifyGuestConnectToken > binds the guest token to its conversation and workspace"
#[test]
fn signguestconnecttoken_verifyguestconnecttoken_binds_the_guest_token_to_its_conversation_and_workspace()
 {
    let clock = clock();
    let token = sign_guest_connect_token("guest-conversation-1", "workspace-1", SECRET, &clock);

    assert_eq!(
        verify_guest_connect_token(&token, "guest-conversation-1", SECRET, &clock).unwrap(),
        GuestClaims {
            guest_conversation_id: "guest-conversation-1".to_owned(),
            workspace_id: "workspace-1".to_owned(),
        }
    );
    assert!(verify_guest_connect_token(&token, "guest-conversation-2", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:349 "signGuestConnectToken / verifyGuestConnectToken > rejects an expired guest token"
#[test]
fn signguestconnecttoken_verifyguestconnecttoken_rejects_an_expired_guest_token() {
    let clock = clock();
    let token = sign_raw_jwt(
        &json!({
            "guestConversationId": "guest-conversation-1",
            "purpose": purpose::GUEST_CONNECT,
            "aud": "guest:guest-conversation-1",
            "exp": secs(NOW_MS) - 10,
        }),
        SECRET,
    );

    assert!(verify_guest_connect_token(&token, "guest-conversation-1", SECRET, &clock).is_err());
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:370 "guest token expiry tolerance > accepts a token expired four seconds ago but rejects one expired six seconds ago"
#[test]
fn guest_token_expiry_tolerance_accepts_a_token_expired_four_seconds_ago_but_rejects_one_expired_six_seconds_ago()
 {
    // vi.setSystemTime(new Date("2026-10-03T12:00:00.000Z"))
    let now_ms = 1_791_028_800_000;
    let clock = TestClock::new(now_ms);
    let recently_expired_token = sign_raw_jwt(
        &json!({
            "guestConversationId": "guest-conversation-1",
            "purpose": purpose::GUEST_CONNECT,
            "workspaceId": "workspace-1",
            "aud": "guest:guest-conversation-1",
            "exp": secs(now_ms) - 4,
        }),
        SECRET,
    );

    assert_eq!(
        verify_guest_connect_token(
            &recently_expired_token,
            "guest-conversation-1",
            SECRET,
            &clock
        )
        .unwrap(),
        GuestClaims {
            guest_conversation_id: "guest-conversation-1".to_owned(),
            workspace_id: "workspace-1".to_owned(),
        }
    );

    let expired_token = sign_raw_jwt(
        &json!({
            "guestConversationId": "guest-conversation-1",
            "purpose": purpose::GUEST_CONNECT,
            "workspaceId": "workspace-1",
            "aud": "guest:guest-conversation-1",
            "exp": secs(now_ms) - 6,
        }),
        SECRET,
    );

    assert!(
        verify_guest_connect_token(&expired_token, "guest-conversation-1", SECRET, &clock).is_err()
    );
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:411 "extractBearerToken > extracts the token from a well-formed Bearer header"
#[test]
fn extractbearertoken_extracts_the_token_from_a_well_formed_bearer_header() {
    assert_eq!(
        extract_bearer_token(Some("Bearer abc.def.ghi")),
        Some("abc.def.ghi")
    );
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:415 "extractBearerToken > returns null for a missing header"
#[test]
fn extractbearertoken_returns_null_for_a_missing_header() {
    assert_eq!(extract_bearer_token(None), None);
}

/// Port of: packages/realtime-protocol/__tests__/auth.test.ts:419 "extractBearerToken > returns null for a non-Bearer scheme"
#[test]
fn extractbearertoken_returns_null_for_a_non_bearer_scheme() {
    assert_eq!(extract_bearer_token(Some("Basic abc")), None);
}
