//! `packages/utils/src/has-link.ts`: does a text carry a link?

use std::sync::OnceLock;

use regex::Regex;

fn scheme_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)https?://|www\.").expect("valid regex"))
}

/// `BARE_DOMAIN_RE` (case-sensitive, ASCII word boundaries like JS `\b`).
fn bare_domain_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?-u:\b)[a-z0-9-]+\.(?:com|net|org|io|vn|shop|store|info|biz)(?-u:\b)")
            .expect("valid regex")
    })
}

/// `hasLink`: a `http(s)://` / `www.` link or a bare domain on a common TLD.
pub fn has_link(text: &str) -> bool {
    scheme_link_re().is_match(text) || bare_domain_re().is_match(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_scheme_www_and_bare_domains() {
        assert!(has_link("see HTTPS://x.y"));
        assert!(has_link("go to WWW.shop"));
        assert!(has_link("buy at shop.vn now"));
        assert!(!has_link("Shop.VN in capitals is not a bare domain"));
        assert!(!has_link("no links here, just a.b"));
        assert!(!has_link("example.community"));
    }
}
