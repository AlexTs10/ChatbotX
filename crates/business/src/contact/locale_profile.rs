//! `contact-locale` `finalizeContactProfile` (with `profileFromPhoneNumber`
//! and `timezoneFromLocaleRegion`): the locale / language / timezone a new
//! contact is created with.

use chatbotx_core::variables::system_field::{normalize_locale, normalize_stored_timezone};

use super::update_fields::normalize_language;

/// `singleZoneCountryTimezones`.
const SINGLE_ZONE_COUNTRY_TIMEZONES: &[(&str, &str)] = &[
    ("AE", "Asia/Dubai"),
    ("AT", "Europe/Vienna"),
    ("BE", "Europe/Brussels"),
    ("CH", "Europe/Zurich"),
    ("CN", "Asia/Shanghai"),
    ("DE", "Europe/Berlin"),
    ("DK", "Europe/Copenhagen"),
    ("ES", "Europe/Madrid"),
    ("FI", "Europe/Helsinki"),
    ("FR", "Europe/Paris"),
    ("GB", "Europe/London"),
    ("HK", "Asia/Hong_Kong"),
    ("IE", "Europe/Dublin"),
    ("IN", "Asia/Kolkata"),
    ("IT", "Europe/Rome"),
    ("JP", "Asia/Tokyo"),
    ("KH", "Asia/Phnom_Penh"),
    ("KR", "Asia/Seoul"),
    ("LA", "Asia/Vientiane"),
    ("MM", "Asia/Yangon"),
    ("MY", "Asia/Kuala_Lumpur"),
    ("NL", "Europe/Amsterdam"),
    ("NO", "Europe/Oslo"),
    ("NZ", "Pacific/Auckland"),
    ("PH", "Asia/Manila"),
    ("PT", "Europe/Lisbon"),
    ("SA", "Asia/Riyadh"),
    ("SE", "Europe/Stockholm"),
    ("SG", "Asia/Singapore"),
    ("TH", "Asia/Bangkok"),
    ("TW", "Asia/Taipei"),
    ("VN", "Asia/Ho_Chi_Minh"),
];

/// `phoneCountryProfiles`: `(country, locale, timezone)`.
const PHONE_COUNTRY_PROFILES: &[(&str, &str, &str)] = &[
    ("AE", "ar_AE", "Asia/Dubai"),
    ("DE", "de_DE", "Europe/Berlin"),
    ("FR", "fr_FR", "Europe/Paris"),
    ("GB", "en_GB", "Europe/London"),
    ("ID", "id_ID", "Asia/Jakarta"),
    ("IN", "hi_IN", "Asia/Kolkata"),
    ("IT", "it_IT", "Europe/Rome"),
    ("JP", "ja_JP", "Asia/Tokyo"),
    ("KH", "km_KH", "Asia/Phnom_Penh"),
    ("KR", "ko_KR", "Asia/Seoul"),
    ("LA", "lo_LA", "Asia/Vientiane"),
    ("MM", "my_MM", "Asia/Yangon"),
    ("MY", "ms_MY", "Asia/Kuala_Lumpur"),
    ("PH", "fil_PH", "Asia/Manila"),
    ("SG", "en_SG", "Asia/Singapore"),
    ("TH", "th_TH", "Asia/Bangkok"),
    ("VN", "vi_VN", "Asia/Ho_Chi_Minh"),
];

/// `ContactLocaleProfile`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocaleProfile {
    pub locale: Option<String>,
    pub language: Option<String>,
    pub timezone: Option<String>,
}

/// `profileFromPhoneNumber`: `(locale, timezone)` of the phone's country.
fn profile_from_phone_number(phone: Option<&str>) -> Option<(&'static str, &'static str)> {
    let trimmed = phone.map(str::trim).filter(|p| !p.is_empty())?;
    let e164 = if trimmed.starts_with('+') {
        trimmed.to_owned()
    } else {
        format!("+{trimmed}")
    };
    let parsed = phonenumber::parse(None, &e164).ok()?;
    let country = parsed.country().id()?;
    PHONE_COUNTRY_PROFILES
        .iter()
        .find(|(c, _, _)| *c == country.as_ref())
        .map(|(_, locale, zone)| (*locale, *zone))
}

/// `timezoneFromLocaleRegion`.
fn timezone_from_locale_region(locale: Option<&str>) -> Option<String> {
    let normalized = normalize_locale(locale)?;
    let region = normalized.split('_').nth(1)?.to_uppercase();
    SINGLE_ZONE_COUNTRY_TIMEZONES
        .iter()
        .find(|(c, _)| *c == region)
        .map(|(_, zone)| (*zone).to_owned())
}

/// `finalizeContactProfile(profile, { phoneHint, fallbackLocale })`.
pub fn finalize_contact_profile(
    profile: &LocaleProfile,
    phone_hint: Option<&str>,
    fallback_locale: Option<&str>,
) -> LocaleProfile {
    let phone = profile_from_phone_number(phone_hint);
    let locale = normalize_locale(profile.locale.as_deref())
        .or_else(|| phone.map(|(locale, _)| locale.to_owned()))
        .or_else(|| normalize_locale(fallback_locale));
    let timezone = normalize_stored_timezone(profile.timezone.as_deref())
        .or_else(|| phone.map(|(_, zone)| zone.to_owned()))
        .or_else(|| timezone_from_locale_region(locale.as_deref()));
    let language = normalize_language(profile.language.as_deref())
        .or_else(|| normalize_language(locale.as_deref()))
        .or_else(|| normalize_language(phone.map(|(locale, _)| locale)));
    LocaleProfile {
        locale,
        language,
        timezone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_region_and_fallback_fill_the_gaps() {
        let empty = LocaleProfile::default();
        let vn = finalize_contact_profile(&empty, Some("84901234567"), None);
        assert_eq!(vn.locale.as_deref(), Some("vi_VN"));
        assert_eq!(vn.language.as_deref(), Some("vi"));
        assert_eq!(vn.timezone.as_deref(), Some("Asia/Ho_Chi_Minh"));

        let given = LocaleProfile {
            locale: Some("de-at".into()),
            timezone: Some("+07:00".into()),
            ..LocaleProfile::default()
        };
        let out = finalize_contact_profile(&given, None, None);
        assert_eq!(out.locale.as_deref(), Some("de_AT"));
        assert_eq!(out.language.as_deref(), Some("de"));
        assert_eq!(out.timezone.as_deref(), Some("Asia/Bangkok"));

        let region = finalize_contact_profile(
            &LocaleProfile {
                locale: Some("fr_CH".into()),
                ..LocaleProfile::default()
            },
            None,
            None,
        );
        assert_eq!(region.timezone.as_deref(), Some("Europe/Zurich"));

        let zalo = finalize_contact_profile(&empty, None, Some("vi_VN"));
        assert_eq!(zalo.locale.as_deref(), Some("vi_VN"));
        assert_eq!(zalo.timezone.as_deref(), Some("Asia/Ho_Chi_Minh"));
        assert_eq!(finalize_contact_profile(&empty, None, None), empty);
    }
}
