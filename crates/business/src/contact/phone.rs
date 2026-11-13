//! `contact/phone.ts`: E.164 normalization of contact phone numbers.

/// `resolveDefaultRegion`: the workspace country, `None` for an unset or
/// `"unknown"` country.
pub fn resolve_default_region(target_country: Option<&str>) -> Option<String> {
    let _ = target_country;
    todo!("P3.6-D2")
}

/// `normalizePhoneToE164` (`libphonenumber-js` `parsePhoneNumberFromString`
/// with the workspace country as the default region).
pub fn normalize_phone_to_e164(raw: &str, target_country: Option<&str>) -> Option<String> {
    let _ = (raw, target_country);
    todo!("P3.6-D2")
}

/// `normalizeIdentityPhoneToE164`: the webhook identity reading (bare-digit
/// `wa_id`s are international when they are valid).
pub fn normalize_identity_phone_to_e164(raw: &str, target_country: Option<&str>) -> Option<String> {
    let _ = (raw, target_country);
    todo!("P3.6-D2")
}
