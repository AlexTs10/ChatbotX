//! `packages/utils/src/placeholder-email.ts`.

const PLACEHOLDER_EMAIL_DOMAIN: &str = "facebook.placeholder.invalid";

/// `facebookPlaceholderEmail`.
pub fn facebook_placeholder_email(facebook_id: &str) -> String {
    format!("fb-{facebook_id}@{PLACEHOLDER_EMAIL_DOMAIN}")
}

/// `isPlaceholderEmail` (case-insensitive).
pub fn is_placeholder_email(email: Option<&str>) -> bool {
    email.is_some_and(|e| {
        e.to_lowercase()
            .ends_with(&format!("@{PLACEHOLDER_EMAIL_DOMAIN}"))
    })
}

/// The Facebook profile fields `mapFacebookProfileToUser` reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FacebookProfile {
    pub id: Option<String>,
    pub sub: Option<String>,
    pub email: Option<String>,
}

/// What `mapFacebookProfileToUser` overrides (`{}` = `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderUser {
    pub email: String,
    pub email_verified: bool,
}

/// `mapFacebookProfileToUser`: a real email is kept (`None`); a phone-only
/// account gets the stable unverified placeholder; no id → `None`.
pub fn map_facebook_profile_to_user(profile: &FacebookProfile) -> Option<PlaceholderUser> {
    // `profile.id ?? profile.sub`: only null/undefined fall through.
    let facebook_id = profile.id.as_deref().or(profile.sub.as_deref());
    if profile.email.as_deref().is_some_and(|e| !e.is_empty()) {
        return None;
    }
    let facebook_id = facebook_id.filter(|id| !id.is_empty())?;
    Some(PlaceholderUser {
        email: facebook_placeholder_email(facebook_id),
        email_verified: false,
    })
}
