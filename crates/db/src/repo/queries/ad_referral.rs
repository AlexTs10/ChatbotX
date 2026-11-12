//! Port of `queries/ad-referral.ts`: the "conversation from a paid ad" predicates over
//! `ContactInbox.referral` (JSON text).

use crate::repo::sql::Sql;

/// `PAID_AD_REFERRAL_SOURCE.meta` (`referral.source` of a paid Meta ad click).
pub const PAID_AD_REFERRAL_SOURCE_META: &str = "ADS";

/// `(<ci>.referral->>'adId' IS NOT NULL AND <ci>.referral->>'source' = ?)`.
pub fn paid_ad_referral(ci: &str, source: &str) -> Sql {
    Sql::bind(
        format!(
            "({ci}.\"referral\"->>'adId' IS NOT NULL AND {ci}.\"referral\"->>'source' = "
        ),
        source.to_owned(),
    )
    .closed(")")
}

/// `(<ci>.referral->>'ctwaClid' IS NOT NULL AND <ci>.referral->>'ctwaClid' <> '')`.
pub fn ctwa_click_id(ci: &str) -> Sql {
    Sql::raw(format!(
        "({ci}.\"referral\"->>'ctwaClid' IS NOT NULL AND {ci}.\"referral\"->>'ctwaClid' <> '')"
    ))
}

/// `adConversationPredicate(channel)`: whatsapp -> click id; messenger/instagram -> paid
/// Meta ad referral.
pub fn ad_conversation_predicate(ci: &str, channel: &str) -> Sql {
    if channel == "whatsapp" {
        ctwa_click_id(ci)
    } else {
        paid_ad_referral(ci, PAID_AD_REFERRAL_SOURCE_META)
    }
}

/// `anyChannelAdConversationPredicate()`: click id OR paid Meta ad referral.
pub fn any_channel_ad_conversation_predicate(ci: &str) -> Sql {
    let mut out = Sql::raw("(");
    out.append(ctwa_click_id(ci));
    out.push(" OR ");
    out.append(paid_ad_referral(ci, PAID_AD_REFERRAL_SOURCE_META));
    out.push(")");
    out
}

