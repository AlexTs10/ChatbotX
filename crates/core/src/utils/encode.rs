//! `packages/utils/src/encode.ts`: base62 of decimal integer strings
//! (arbitrary size, like `BigInt`) and URL-safe base64 of text.

use crate::js;

const CHARS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// A failed conversion, with the original error text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeError(pub String);

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EncodeError {}

/// Little-endian base-`radix` digits times `factor` plus `add`, in place.
fn mul_add(digits: &mut Vec<u32>, radix: u64, factor: u64, add: u64) {
    let mut carry = add;
    for d in digits.iter_mut() {
        let v = u64::from(*d) * factor + carry;
        *d = (v % radix) as u32;
        carry = v / radix;
    }
    while carry > 0 {
        digits.push((carry % radix) as u32);
        carry /= radix;
    }
}

/// Converts big-endian digits from one radix to another (big-endian out).
fn convert(digits: impl Iterator<Item = u64>, from: u64, to: u64) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for d in digits {
        mul_add(&mut out, to, from, d);
    }
    out.reverse();
    out
}

/// `encodeBase62`: a non-negative decimal integer string in base 62.
pub fn encode_base62(input: &str) -> Result<String, EncodeError> {
    let text = js::trim(input);
    if let Some(rest) = text.strip_prefix('-') {
        if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
            if rest.bytes().all(|b| b == b'0') {
                return Ok("0".to_owned());
            }
            return Err(EncodeError(
                "Input must be a non-negative integer string".to_owned(),
            ));
        }
    }
    let digits = text.strip_prefix('+').unwrap_or(text);
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EncodeError(format!("Cannot convert {input} to a BigInt")));
    }
    let out = convert(digits.bytes().map(|b| u64::from(b - b'0')), 10, 62);
    if out.is_empty() {
        return Ok("0".to_owned());
    }
    Ok(out.iter().map(|&d| char::from(CHARS[d as usize])).collect())
}

/// `decodeBase62`: the decimal string of a base-62 text.
pub fn decode_base62(input: &str) -> Result<String, EncodeError> {
    let mut values = Vec::with_capacity(input.len());
    for c in input.chars() {
        let index = u8::try_from(c)
            .ok()
            .and_then(|b| CHARS.iter().position(|&x| x == b))
            .ok_or_else(|| EncodeError(format!("Invalid character in string: {c}")))?;
        values.push(index as u64);
    }
    let out = convert(values.into_iter(), 62, 10);
    if out.is_empty() {
        return Ok("0".to_owned());
    }
    Ok(out.iter().map(|d| char::from(b'0' + *d as u8)).collect())
}

/// `encodeBase64UrlText`: `btoa(encodeURIComponent(text))`, URL-safe, no
/// padding.
pub fn encode_base64_url_text(text: &str) -> String {
    let bytes = js::encode_uri_component(text).into_bytes();
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..=chunk.len() {
            let c = STD[((n >> (18 - 6 * i)) & 63) as usize];
            out.push(match c {
                b'+' => '-',
                b'/' => '_',
                other => char::from(other),
            });
        }
    }
    out
}

/// `decodeBase64UrlText`: the inverse (`atob` then
/// `decodeURIComponent`); `Err` where either throws.
pub fn decode_base64_url_text(encoded: &str) -> Result<String, EncodeError> {
    let invalid = || EncodeError("The string to be decoded is not correctly encoded.".to_owned());
    let mut chars: Vec<u8> = encoded
        .bytes()
        .filter(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\x0c' | b'\r'))
        .map(|b| match b {
            b'-' => b'+',
            b'_' => b'/',
            other => other,
        })
        .collect();
    if chars.len() % 4 == 0 {
        if chars.ends_with(b"==") {
            chars.truncate(chars.len() - 2);
        } else if chars.ends_with(b"=") {
            chars.truncate(chars.len() - 1);
        }
    }
    if chars.len() % 4 == 1 {
        return Err(invalid());
    }
    let mut bytes = Vec::with_capacity(chars.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in chars {
        let v = STD.iter().position(|&x| x == c).ok_or_else(invalid)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push(((acc >> bits) & 0xff) as u8);
        }
    }
    // `atob` yields one char per byte (Latin-1).
    let latin1: String = bytes.iter().map(|&b| char::from(b)).collect();
    js::decode_uri_component(&latin1).map_err(|()| EncodeError("URI malformed".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base62_round_trips_large_ids() {
        assert_eq!(encode_base62("0").unwrap(), "0");
        assert_eq!(encode_base62("61").unwrap(), "z");
        assert_eq!(encode_base62("62").unwrap(), "10");
        let id = "123456789012345678901234567890";
        assert_eq!(decode_base62(&encode_base62(id).unwrap()).unwrap(), id);
        assert_eq!(decode_base62("10").unwrap(), "62");
        assert!(encode_base62("-1").is_err());
        assert!(encode_base62("1.5").is_err());
        assert_eq!(
            decode_base62("a!").unwrap_err().0,
            "Invalid character in string: !"
        );
    }

    #[test]
    fn base64_url_text_round_trips_unicode() {
        // btoa(encodeURIComponent("Xin chào")) === "WGluJTIwY2glQzMlQTBv"
        assert_eq!(encode_base64_url_text("Xin chào"), "WGluJTIwY2glQzMlQTBv");
        for text in ["", "a", "ab", "abc", "Xin chào 😀 ?&="] {
            assert_eq!(
                decode_base64_url_text(&encode_base64_url_text(text)).unwrap(),
                text
            );
        }
        assert!(decode_base64_url_text("a").is_err());
    }
}
