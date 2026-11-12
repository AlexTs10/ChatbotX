//! `packages/encryption/src/binary.ts`.

use super::EncryptionError;

/// `hexToBytes`: two characters per byte, parsed like `parseInt(pair, 16)`
/// (leading hex digits; none gives 0, as `NaN` stored in a `Uint8Array`).
pub fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, EncryptionError> {
    let units: Vec<u16> = hex.encode_utf16().collect();
    if units.len() % 2 != 0 {
        return Err(EncryptionError::OddHex);
    }
    Ok(units
        .chunks(2)
        .map(|pair| {
            let mut value = 0u32;
            for unit in pair {
                match char::from_u32(u32::from(*unit)).and_then(|c| c.to_digit(16)) {
                    Some(digit) => value = value * 16 + digit,
                    None => break,
                }
            }
            value as u8
        })
        .collect())
}

/// `bytesToHex`: lower-case, two digits per byte.
pub fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// `concatBytes`.
pub fn concat_bytes(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}
