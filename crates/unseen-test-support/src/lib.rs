#![forbid(unsafe_code)]

pub mod json;
pub mod sha256;

pub use sha256::Sha256;

#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 0x0F)] as char);
    }
    out
}

#[must_use]
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let (pairs, []) = text.as_bytes().as_chunks::<2>() else {
        return None;
    };
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    pairs.iter().map(|&[hi, lo]| Some(digit(hi)? << 4 | digit(lo)?)).collect()
}

#[cfg(test)]
mod tests {
    use super::{hex, unhex};

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex(&[0x00, 0x7f, 0xff]), "007fff");
        assert_eq!(unhex("007FfF"), Some(vec![0x00, 0x7f, 0xff]));
        assert_eq!(unhex("0"), None);
        assert_eq!(unhex("zz"), None);
    }
}
