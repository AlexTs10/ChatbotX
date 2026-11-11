//! Small std-only random source (spec 2.13a: no `rand` crate). Not for
//! secrets: it feeds job-id suffixes, the deferral jitter and the boot
//! minute of the license refresh.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static STATE: AtomicU64 = AtomicU64::new(0);

fn seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let local = 0u8;
    let addr = std::ptr::addr_of!(local) as u64;
    nanos ^ (u64::from(std::process::id()) << 32) ^ addr.rotate_left(17)
}

/// The next 64 random bits (splitmix64 over a shared atomic counter).
pub fn next_u64() -> u64 {
    if STATE.load(Ordering::Relaxed) == 0 {
        let _ = STATE.compare_exchange(0, seed() | 1, Ordering::Relaxed, Ordering::Relaxed);
    }
    let mut z = STATE
        .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `Math.random()`: a value in `[0, 1)`.
pub fn random_f64() -> f64 {
    (next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// A random UUID v4 in lowercase hex (`crypto.randomUUID()`).
pub fn uuid_v4() -> String {
    let hi = next_u64();
    let lo = next_u64();
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// A positive decimal id (the numeric `createId()` shape).
pub fn numeric_id() -> String {
    (next_u64() >> 2).max(1).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_has_the_v4_shape_and_values_differ() {
        let a = uuid_v4();
        let b = uuid_v4();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn random_f64_is_in_the_unit_interval() {
        for _ in 0..1000 {
            let r = random_f64();
            assert!((0.0..1.0).contains(&r));
        }
    }
}
