pub(crate) const RUNE_ERROR: u32 = 0xFFFD;

#[expect(
    clippy::inline_always,
    reason = "left to itself the compiler keeps this a call on the per-scalar path, measured at a third of the scan"
)]
#[inline(always)]
pub(crate) fn decode(p: &[u8]) -> (u32, usize) {
    let b0 = p[0];
    if b0 < 0x80 {
        return (u32::from(b0), 1);
    }
    let (width, lo, hi) = match b0 {
        0xC2..=0xDF => (2, 0x80, 0xBF),
        0xE0 => (3, 0xA0, 0xBF),
        0xE1..=0xEC | 0xEE..=0xEF => (3, 0x80, 0xBF),
        0xED => (3, 0x80, 0x9F),
        0xF0 => (4, 0x90, 0xBF),
        0xF1..=0xF3 => (4, 0x80, 0xBF),
        0xF4 => (4, 0x80, 0x8F),
        _ => return (RUNE_ERROR, 1),
    };
    if p.len() < width || !(lo..=hi).contains(&p[1]) {
        return (RUNE_ERROR, 1);
    }
    if p[2..width].iter().any(|&b| !(0x80..=0xBF).contains(&b)) {
        return (RUNE_ERROR, 1);
    }
    let mut r = u32::from(b0) & (0x7F >> width);
    for &b in &p[1..width] {
        r = r << 6 | u32::from(b & 0x3F);
    }
    (r, width)
}

#[inline]
pub(crate) fn is_error(r: u32, width: usize) -> bool {
    r == RUNE_ERROR && width == 1
}

#[cfg(test)]
mod tests {
    use super::{RUNE_ERROR, decode};

    fn std_decode(p: &[u8]) -> (u32, usize) {
        match std::str::from_utf8(p) {
            Ok(s) => {
                let c = s.chars().next().expect("non-empty");
                (c as u32, c.len_utf8())
            }
            Err(e) if e.valid_up_to() > 0 => {
                let s = std::str::from_utf8(&p[..e.valid_up_to()]).expect("valid prefix");
                let c = s.chars().next().expect("non-empty");
                (c as u32, c.len_utf8())
            }
            Err(_) => (RUNE_ERROR, 1),
        }
    }

    #[test]
    fn matches_std_on_every_short_sequence() {
        for b0 in 0..=255u8 {
            assert_eq!(decode(&[b0]), std_decode(&[b0]), "{b0:02x}");
            for b1 in 0..=255u8 {
                let p = [b0, b1];
                assert_eq!(decode(&p), std_decode(&p), "{p:02x?}");
            }
        }
        for b0 in 0xE0..=0xF4u8 {
            for b1 in 0..=255u8 {
                for b2 in 0..=255u8 {
                    let p = [b0, b1, b2, 0x80];
                    assert_eq!(decode(&p), std_decode(&p), "{p:02x?}");
                    assert_eq!(decode(&p[..3]), std_decode(&p[..3]), "{:02x?}", &p[..3]);
                }
            }
        }
    }

    #[test]
    fn every_scalar_round_trips() {
        let mut buf = [0u8; 4];
        for c in (0..=0x10_FFFF).filter_map(char::from_u32) {
            let s = c.encode_utf8(&mut buf);
            assert_eq!(decode(s.as_bytes()), (c as u32, s.len()));
        }
    }

    #[test]
    fn invalid_sequences_have_width_one() {
        for p in [&[0x80][..], &[0xC0, 0xAF], &[0xED, 0xA0, 0x80], &[0xF4, 0x90, 0x80, 0x80], &[0xE2, 0x82], &[0xFF]] {
            assert_eq!(decode(p), (RUNE_ERROR, 1), "{p:02x?}");
        }
        assert_eq!(decode("\u{FFFD}".as_bytes()), (RUNE_ERROR, 3));
    }
}
