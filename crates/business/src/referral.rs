//! `packages/business/src/referral/utils.ts` (`encodeRef`; the ref kinds the
//! ported domains link with).

use chatbotx_core::utils::encode::encode_base62;

/// `RefConfig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefConfig {
    Flow {
        flow_id: i64,
        node_id: Option<String>,
    },
    Draft {
        flow_id: i64,
    },
    MinigameShare {
        minigame_id: i64,
        referrer_contact_id: i64,
    },
    Leaderboard {
        leaderboard_id: i64,
        contact_id: i64,
    },
    Poll {
        poll_id: i64,
    },
    QrCode {
        name: String,
    },
    Reflink {
        name: String,
    },
}

fn b62(value: &str) -> String {
    encode_base62(value).unwrap_or_default()
}

/// `encodeRef`.
pub fn encode_ref(config: &RefConfig) -> String {
    todo!("P3.6-D1")
}
