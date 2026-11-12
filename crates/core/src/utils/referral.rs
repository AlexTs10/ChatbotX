//! `packages/utils/src/referral.ts`: Meta referral payload values.

string_enum! {
    /// `metaReferralSources`.
    pub enum MetaReferralSource {
        Ads = "ADS",
        Shortlink = "SHORTLINK",
    }
}

string_enum! {
    /// `metaReferralTypes`.
    pub enum MetaReferralType {
        OpenThread = "OPEN_THREAD",
    }
}

/// `PAID_AD_REFERRAL_SOURCE.meta`.
pub const META_PAID_AD_REFERRAL_SOURCE: MetaReferralSource = MetaReferralSource::Ads;

/// `PAID_AD_REFERRAL_SOURCES`: the referral sources that are a paid ad.
pub const PAID_AD_REFERRAL_SOURCES: &[&str] = &["ADS"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_meta_ads_is_a_paid_ad_source() {
        assert_eq!(
            PAID_AD_REFERRAL_SOURCES,
            [META_PAID_AD_REFERRAL_SOURCE.as_str()]
        );
        assert_eq!(
            MetaReferralType::parse("OPEN_THREAD"),
            Some(MetaReferralType::OpenThread)
        );
    }
}
