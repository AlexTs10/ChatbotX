//! `contact/phone.ts`: E.164 normalization of contact phone numbers
//! (`libphonenumber-js` = the `phonenumber` crate).

use phonenumber::{Mode, PhoneNumber, Type, country};

const UNKNOWN_TARGET_COUNTRY: &str = "unknown";

/// `resolveDefaultRegion`: the workspace country, `None` for an unset or
/// `"unknown"` country.
pub fn resolve_default_region(target_country: Option<&str>) -> Option<String> {
    target_country
        .filter(|country| !country.is_empty() && *country != UNKNOWN_TARGET_COUNTRY)
        .map(str::to_string)
}

fn region_id(region: Option<&str>) -> Option<country::Id> {
    region.and_then(|region| region.parse::<country::Id>().ok())
}

/// `parsePhoneNumberFromString(raw, region)`: the parsed number, or `None`
/// when `raw` is not a phone number (a national number needs the region).
fn parse(raw: &str, region: Option<&str>) -> Option<PhoneNumber> {
    if raw.trim().is_empty() {
        return None;
    }
    let region = region_id(region);
    if region.is_none() && !raw.trim_start().starts_with('+') {
        return None;
    }
    let parsed = phonenumber::parse(region, raw).ok()?;
    if raw.trim_start().starts_with('+') {
        return Some(parsed);
    }
    // The crate strips a leading calling code of the region more eagerly
    // than libphonenumber-js, which keeps the national reading when the
    // whole number is valid for the region (VN `846366595`).
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    let code = parsed.code().value().to_string();
    let national = parsed.national().value().to_string();
    if digits.starts_with(&code) && national != digits.trim_start_matches('0') {
        if let Some(whole) = phonenumber::parse(None, format!("+{code}{digits}"))
            .ok()
            .filter(PhoneNumber::is_valid)
        {
            return Some(whole);
        }
    }
    Some(parsed)
}

fn e164(number: &PhoneNumber) -> String {
    number.format().mode(Mode::E164).to_string()
}

/// `normalizePhoneToE164` (`libphonenumber-js` `parsePhoneNumberFromString`
/// with the workspace country as the default region).
pub fn normalize_phone_to_e164(raw: &str, target_country: Option<&str>) -> Option<String> {
    let region = resolve_default_region(target_country);
    parse(raw, region.as_deref()).map(|number| e164(&number))
}

/// `parseOwnCountryWaId`: a bare-digit `wa_id` that is a valid mobile number
/// of the workspace country.
fn parse_own_country_wa_id(raw: &str, region: Option<&str>) -> Option<String> {
    let region = region?;
    if raw.starts_with('+') {
        return None;
    }
    let parsed = parse(&format!("+{raw}"), None)?;
    let same_country = parsed.country().id().map(|id| id.as_ref() == region) == Some(true);
    let mobile = matches!(
        parsed.number_type(&phonenumber::metadata::DATABASE),
        Type::Mobile | Type::FixedLineOrMobile
    );
    (parsed.is_valid() && same_country && mobile).then(|| e164(&parsed))
}

/// `normalizeIdentityPhoneToE164`: the webhook identity reading (bare-digit
/// `wa_id`s are international when they are valid).
pub fn normalize_identity_phone_to_e164(raw: &str, target_country: Option<&str>) -> Option<String> {
    let region = resolve_default_region(target_country);
    if let Some(own) = parse_own_country_wa_id(raw, region.as_deref()) {
        return Some(own);
    }
    let national = parse(raw, region.as_deref());
    if let Some(number) = national.as_ref().filter(|n| n.is_valid()) {
        return Some(e164(number));
    }
    if !raw.starts_with('+') {
        if let Some(number) = parse(&format!("+{raw}"), None).filter(|n| n.is_valid()) {
            return Some(e164(&number));
        }
    }
    national.map(|number| e164(&number))
}
