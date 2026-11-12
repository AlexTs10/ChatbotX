//! `packages/utils/src/spintax.ts`: `{Hi|Hello}` blocks, one branch picked
//! per occurrence. Distinct from the `{{variable}}` grammar.

use std::sync::OnceLock;

use regex::{Captures, Regex};

/// `SPINTAX_BLOCK_SOURCE`: brace-free, single-line content with a `|`.
pub const SPINTAX_BLOCK_SOURCE: &str = r"\{([^{}\n]*\|[^{}\n]*)\}";
/// A longer block (UTF-16 units of its content) is left verbatim.
pub const SPINTAX_MAX_BLOCK_LENGTH: usize = 500;
/// A block with more branches is left verbatim.
pub const SPINTAX_MAX_BRANCHES: usize = 20;

fn block_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(SPINTAX_BLOCK_SOURCE).expect("valid regex"))
}

/// `containsSpintax`.
pub fn contains_spintax(value: &str) -> bool {
    block_re().is_match(value)
}

fn random_index(count: usize) -> usize {
    let mut bytes = [0u8; 8];
    if ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).is_err() {
        return 0;
    }
    (u64::from_le_bytes(bytes) % count.max(1) as u64) as usize
}

/// `applySpintax` with a uniform random pick.
pub fn apply_spintax(text: &str) -> String {
    apply_spintax_with(text, &mut random_index)
}

/// `applySpintax` with an injected picker: `pick(count)` chooses a branch
/// index in `[0, count)`; an out-of-range index leaves the block verbatim.
pub fn apply_spintax_with(text: &str, pick: &mut dyn FnMut(usize) -> usize) -> String {
    block_re()
        .replace_all(text, |caps: &Captures<'_>| {
            let whole = caps[0].to_owned();
            let content = &caps[1];
            if crate::js::utf16_len(content) > SPINTAX_MAX_BLOCK_LENGTH {
                return whole;
            }
            let branches: Vec<&str> = content.split('|').collect();
            if branches.len() > SPINTAX_MAX_BRANCHES {
                return whole;
            }
            let index = pick(branches.len());
            branches
                .get(index)
                .map_or(whole.clone(), |branch| crate::js::trim(branch).to_owned())
        })
        .into_owned()
}
