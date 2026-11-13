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
    match config {
        RefConfig::Flow { flow_id, node_id } => {
            let node = node_id
                .as_deref()
                .filter(|n| !n.is_empty())
                .map(|n| format!("_{}", b62(n)))
                .unwrap_or_default();
            format!("f_{}{node}", b62(&flow_id.to_string()))
        }
        RefConfig::Draft { flow_id } => format!("d_{}", b62(&flow_id.to_string())),
        RefConfig::MinigameShare {
            minigame_id,
            referrer_contact_id,
        } => format!(
            "mg_{}_{}",
            b62(&minigame_id.to_string()),
            b62(&referrer_contact_id.to_string())
        ),
        RefConfig::Leaderboard {
            leaderboard_id,
            contact_id,
        } => format!(
            "lb_{}_{}",
            b62(&leaderboard_id.to_string()),
            b62(&contact_id.to_string())
        ),
        RefConfig::Poll { poll_id } => format!("pl_{}", b62(&poll_id.to_string())),
        RefConfig::QrCode { name } => format!("qr_{name}"),
        RefConfig::Reflink { name } => name.clone(),
    }
}
