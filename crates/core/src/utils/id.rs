//! `packages/utils/src/id.ts`: numeric snowflake ids,
//! `timestamp(53) << 14 | placeId(4) << 10 | sequence(10)`, ms since
//! 2004-02-01 (the uuniq layout, so stored ids stay decodable).

use std::sync::{Arc, Mutex, OnceLock};

use crate::Clock;
use crate::env::{Env, ProcessEnv};

/// `Date.parse("2004-02-01")`.
pub const SNOWFLAKE_EPOCH_MS: i64 = 1_075_593_600_000;
const SEQUENCE_SPACE: i64 = 1024;
const PLACE_ID_SPACE: i64 = 16;
const TIMESTAMP_SCALE: i128 = (SEQUENCE_SPACE * PLACE_ID_SPACE) as i128;
const PLACE_ID_SCALE: i128 = SEQUENCE_SPACE as i128;
/// The largest place id (4 bits).
pub const SNOWFLAKE_PLACE_ID_MAX: i64 = PLACE_ID_SPACE - 1;
const LEGACY_PLACE_ID: i64 = 0;

/// A source of randomness in `[0, 1)` (`Math.random`).
pub type RandomSource = Box<dyn FnMut() -> f64 + Send>;

/// `Math.random()` from the OS random source.
pub fn math_random() -> f64 {
    let mut bytes = [0u8; 8];
    if ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).is_err() {
        return 0.0;
    }
    (u64::from_le_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64
}

fn random_int_between(random: &mut dyn FnMut() -> f64, min: i64, max: i64) -> i64 {
    min + (random() * (max - min + 1) as f64).floor() as i64
}

/// `resolveSnowflakePlaceId`: `SNOWFLAKE_PLACE_ID` (0–15) wins; otherwise
/// a random slot in 1–15 (never the legacy 0).
pub fn resolve_snowflake_place_id(env: &dyn Env, random: &mut dyn FnMut() -> f64) -> i64 {
    if let Some(raw) = env.var("SNOWFLAKE_PLACE_ID") {
        if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(parsed) = raw.parse::<i64>() {
                if parsed <= SNOWFLAKE_PLACE_ID_MAX {
                    return parsed;
                }
            }
        }
    }
    random_int_between(random, LEGACY_PLACE_ID + 1, SNOWFLAKE_PLACE_ID_MAX)
}

/// `createSnowflakeGenerator` failed: the place id is not 0–15.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceIdError(pub String);

impl std::fmt::Display for PlaceIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PlaceIdError {}

struct GeneratorState {
    last_timestamp: i64,
    sequence_start: i64,
    issued_in_timestamp: i64,
    random: RandomSource,
}

/// `SnowflakeGenerator`: per-process place id, a random sequence offset per
/// millisecond, and a logical clock that never runs backwards or blocks.
pub struct SnowflakeGenerator {
    place_id: i64,
    clock: Arc<dyn Clock>,
    state: Mutex<GeneratorState>,
}

impl std::fmt::Debug for SnowflakeGenerator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnowflakeGenerator")
            .field("place_id", &self.place_id)
            .finish_non_exhaustive()
    }
}

impl SnowflakeGenerator {
    /// `createSnowflakeGenerator({ placeId, random })` on `clock`. The place
    /// id is an `f64` so a fractional value is rejected like the original.
    pub fn new(
        place_id: f64,
        random: RandomSource,
        clock: Arc<dyn Clock>,
    ) -> Result<SnowflakeGenerator, PlaceIdError> {
        if place_id.fract() != 0.0 || !(0.0..=SNOWFLAKE_PLACE_ID_MAX as f64).contains(&place_id) {
            return Err(PlaceIdError(format!(
                "Snowflake place id must be an integer between 0 and {SNOWFLAKE_PLACE_ID_MAX}, received {}",
                crate::js::number_to_string(place_id)
            )));
        }
        Ok(SnowflakeGenerator {
            place_id: place_id as i64,
            clock,
            state: Mutex::new(GeneratorState {
                last_timestamp: -1,
                sequence_start: 0,
                issued_in_timestamp: 0,
                random,
            }),
        })
    }

    /// The place id.
    pub fn place_id(&self) -> i64 {
        self.place_id
    }

    /// `generate()`: the next id, as a decimal string.
    pub fn generate(&self) -> String {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let wall_clock = (self.clock.now_ms() - SNOWFLAKE_EPOCH_MS).max(state.last_timestamp);
        let exhausted = wall_clock == state.last_timestamp
            && state.sequence_start + state.issued_in_timestamp >= SEQUENCE_SPACE;
        let now = if exhausted {
            state.last_timestamp + 1
        } else {
            wall_clock
        };
        if now != state.last_timestamp {
            state.last_timestamp = now;
            state.sequence_start = random_int_between(&mut *state.random, 0, SEQUENCE_SPACE - 1);
            state.issued_in_timestamp = 0;
        }
        let sequence = state.sequence_start + state.issued_in_timestamp;
        state.issued_in_timestamp += 1;
        let id = i128::from(now) * TIMESTAMP_SCALE
            + i128::from(self.place_id) * PLACE_ID_SCALE
            + i128::from(sequence);
        id.to_string()
    }
}

fn process_generator() -> &'static SnowflakeGenerator {
    static GENERATOR: OnceLock<SnowflakeGenerator> = OnceLock::new();
    GENERATOR.get_or_init(|| {
        let mut random = math_random;
        let place = resolve_snowflake_place_id(&ProcessEnv, &mut random);
        SnowflakeGenerator::new(
            place as f64,
            Box::new(math_random),
            Arc::new(crate::SystemClock),
        )
        .expect("place id in range")
    })
}

/// `createId()`: an id from the process generator.
pub fn create_id() -> String {
    process_generator().generate()
}

/// `resolveId` (uuniq decoder output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedId {
    /// ISO instant.
    pub created_at: String,
    pub place_id: i64,
    pub sequence: i64,
}

/// `resolveId`: the fields packed into an id; `None` when not an integer.
pub fn resolve_id(id: &str) -> Option<ResolvedId> {
    let value = parse_bigint(id)?;
    let timestamp = ((value >> 14) & ((1i128 << 53) - 1)) as i64;
    Some(ResolvedId {
        created_at: crate::js::to_iso_string(SNOWFLAKE_EPOCH_MS + timestamp)?,
        place_id: ((value >> 10) & 0xf) as i64,
        sequence: (value & 0x3ff) as i64,
    })
}

/// `BigInt(text)`: a trimmed decimal (or `0x`/`0o`/`0b`) integer; the
/// empty string is 0.
fn parse_bigint(text: &str) -> Option<i128> {
    let t = crate::js::trim(text);
    if t.is_empty() {
        return Some(0);
    }
    let (radix, digits) = match t.get(..2) {
        Some("0x") | Some("0X") => (16, &t[2..]),
        Some("0o") | Some("0O") => (8, &t[2..]),
        Some("0b") | Some("0B") => (2, &t[2..]),
        _ => (10, t),
    };
    if radix == 10 {
        let (negative, rest) = match t.as_bytes()[0] {
            b'-' => (true, &t[1..]),
            b'+' => (false, &t[1..]),
            _ => (false, t),
        };
        if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let value: i128 = rest.parse().ok()?;
        return Some(if negative { -value } else { value });
    }
    if digits.is_empty() {
        return None;
    }
    i128::from_str_radix(digits, radix).ok()
}

/// `parseBigIntId`: the canonical decimal of an id, or `None`.
pub fn parse_bigint_id(id: Option<&str>) -> Option<String> {
    let id = id.filter(|i| !i.is_empty())?;
    parse_bigint(id).map(|v| v.to_string())
}

/// `isNumericId`.
pub fn is_numeric_id(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// `createdAtFromId`: the creation time (epoch ms) packed into an id;
/// `None` for a non-numeric id or one at/before the epoch.
pub fn created_at_from_id(id: &str) -> Option<i64> {
    let value = parse_bigint(id)?;
    let millis = (value / TIMESTAMP_SCALE) as f64 + SNOWFLAKE_EPOCH_MS as f64;
    if millis.is_finite()
        && millis > SNOWFLAKE_EPOCH_MS as f64
        && millis.abs() <= crate::js::MAX_TIME_MS as f64
    {
        Some(millis as i64)
    } else {
        None
    }
}
