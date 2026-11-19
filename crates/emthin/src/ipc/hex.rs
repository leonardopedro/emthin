//! Hex encoding for the collaboration frames' binary fields.
//!
//! JSON has no byte string. Base64 would be shorter but needs another
//! dependency or a hand-written codec; hex is short enough here, has no
//! padding rules, and makes a malformed token obvious.

/// Encode bytes as lowercase hex.
pub fn encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Decode lowercase or uppercase hex. Odd length and non-hex characters are
/// rejected rather than skipped.
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(format!("hex length {} is odd", text.len()));
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = nibble(bytes[i])
            .ok_or_else(|| format!("invalid hex character {:?} at byte {i}", bytes[i] as char))?;
        let lo = nibble(bytes[i + 1]).ok_or_else(|| {
            format!(
                "invalid hex character {:?} at byte {}",
                bytes[i + 1] as char,
                i + 1
            )
        })?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_empty_and_binary() {
        assert_eq!(encode(&[]), "");
        assert_eq!(decode("").unwrap(), Vec::<u8>::new());
        let bytes = [0, 1, 15, 16, 127, 128, 255];
        assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
    }

    #[test]
    fn accepts_uppercase() {
        assert_eq!(decode("DEADbeef").unwrap(), vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn rejects_odd_length_and_non_hex() {
        assert!(decode("abc").is_err());
        assert!(decode("zz").is_err());
    }
}
