//! Port of `packages/utils/__tests__/placeholder-email.test.ts`. The `{}`
//! result is `None`.

use chatbotx_core::utils::placeholder_email::{
    FacebookProfile, PlaceholderUser, facebook_placeholder_email, is_placeholder_email,
    map_facebook_profile_to_user,
};

fn profile(id: Option<&str>, sub: Option<&str>, email: Option<&str>) -> FacebookProfile {
    FacebookProfile {
        id: id.map(str::to_owned),
        sub: sub.map(str::to_owned),
        email: email.map(str::to_owned),
    }
}

/// Port of: packages/utils/__tests__/placeholder-email.test.ts:9 "mapFacebookProfileToUser > keeps a real Facebook email untouched"
#[test]
fn mapfacebookprofiletouser_keeps_a_real_facebook_email_untouched() {
    assert_eq!(
        map_facebook_profile_to_user(&profile(Some("123"), None, Some("a@example.com"))),
        None
    );
}

/// Port of: packages/utils/__tests__/placeholder-email.test.ts:15 "mapFacebookProfileToUser > substitutes a stable, unverified placeholder for a phone-only account"
#[test]
fn mapfacebookprofiletouser_substitutes_a_stable_unverified_placeholder_for_a_phone_only_account() {
    let first = map_facebook_profile_to_user(&profile(Some("123"), None, None));
    let second = map_facebook_profile_to_user(&profile(Some("123"), None, None));

    assert_eq!(
        first,
        Some(PlaceholderUser {
            email: "fb-123@facebook.placeholder.invalid".to_owned(),
            email_verified: false,
        })
    );
    assert_eq!(second, first);
}

/// Port of: packages/utils/__tests__/placeholder-email.test.ts:26 "mapFacebookProfileToUser > uses the subject claim from a limited-login profile"
#[test]
fn mapfacebookprofiletouser_uses_the_subject_claim_from_a_limited_login_profile() {
    assert_eq!(
        map_facebook_profile_to_user(&profile(None, Some("limited-123"), None)),
        Some(PlaceholderUser {
            email: "fb-limited-123@facebook.placeholder.invalid".to_owned(),
            email_verified: false,
        })
    );
}

/// Port of: packages/utils/__tests__/placeholder-email.test.ts:33 "mapFacebookProfileToUser > leaves a profile without an id untouched"
#[test]
fn mapfacebookprofiletouser_leaves_a_profile_without_an_id_untouched() {
    assert_eq!(
        map_facebook_profile_to_user(&profile(Some(""), None, None)),
        None
    );
    assert_eq!(
        map_facebook_profile_to_user(&profile(None, None, None)),
        None
    );
}

/// Port of: packages/utils/__tests__/placeholder-email.test.ts:40 "isPlaceholderEmail > recognizes a placeholder regardless of case"
#[test]
fn isplaceholderemail_recognizes_a_placeholder_regardless_of_case() {
    assert!(is_placeholder_email(Some(&facebook_placeholder_email("9"))));
    assert!(is_placeholder_email(Some(
        "FB-9@FACEBOOK.PLACEHOLDER.INVALID"
    )));
}

/// Port of: packages/utils/__tests__/placeholder-email.test.ts:45 "isPlaceholderEmail > rejects real or missing emails"
#[test]
fn isplaceholderemail_rejects_real_or_missing_emails() {
    assert!(!is_placeholder_email(Some("a@example.com")));
    assert!(!is_placeholder_email(Some("a@placeholder.invalid")));
    assert!(!is_placeholder_email(None));
    assert!(!is_placeholder_email(None));
}
