//! Local port of `createId()` from `packages/utils/src/id.ts` (numeric
//! snowflake: `timestamp << 14 | placeId << 10 | sequence`, ms since
//! 2004-02-01). The JS uses it for upload names and keys. The utils crate is
//! not ported yet; when it is, replace this module with a call to it.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// `Date.parse("2004-02-01T00:00:00.000Z")`.
const SNOWFLAKE_EPOCH_MS: u64 = 1_075_593_600_000;
const SEQUENCE_SPACE: u64 = 1024;
const PLACE_ID_MAX: u64 = 15;

struct State {
    last_timestamp: Option<u64>,
    sequence_start: u64,
    issued: u64,
}

static STATE: Mutex<State> = Mutex::new(State {
    last_timestamp: None,
    sequence_start: 0,
    issued: 0,
});

fn random_u64() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    hasher.write_u128(nanos);
    hasher.finish()
}

/// `SNOWFLAKE_PLACE_ID` (0-15) wins; else a random slot in 1-15.
fn place_id() -> u64 {
    static PLACE: OnceLock<u64> = OnceLock::new();
    *PLACE.get_or_init(|| {
        std::env::var("SNOWFLAKE_PLACE_ID")
            .ok()
            .filter(|raw| !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|id| *id <= PLACE_ID_MAX)
            .unwrap_or_else(|| 1 + random_u64() % PLACE_ID_MAX)
    })
}

/// A new process-unique numeric id, as a decimal string.
pub(crate) fn create_id() -> String {
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
        .saturating_sub(SNOWFLAKE_EPOCH_MS);
    let place = place_id();
    let mut state = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Logical clock: never behind the last id; an exhausted slot moves on.
    let now = match state.last_timestamp {
        Some(last) if wall <= last => {
            if state.sequence_start + state.issued >= SEQUENCE_SPACE {
                last + 1
            } else {
                last
            }
        }
        _ => wall,
    };
    if state.last_timestamp != Some(now) {
        state.last_timestamp = Some(now);
        state.sequence_start = random_u64() % SEQUENCE_SPACE;
        state.issued = 0;
    }
    let sequence = state.sequence_start + state.issued;
    state.issued += 1;
    ((now << 14) | (place << 10) | sequence).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_numeric_and_strictly_increasing() {
        let ids: Vec<u64> = (0..3000)
            .map(|_| create_id().parse::<u64>().expect("numeric"))
            .collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        assert!(ids.iter().all(|id| (id >> 10) & PLACE_ID_MAX == place_id()));
    }
}
