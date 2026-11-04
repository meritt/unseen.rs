pub(crate) use crate::unicode17_data::*;

pub const UNICODE_VERSION: &str = "17.0.0";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Props {
    pub flags: u16,
    pub category: u8,
    pub grapheme: u8,
    pub script: u8,
}

impl Props {
    pub(crate) const fn new(flags: u16, category: u8, grapheme: u8, script: u8) -> Self {
        Self { flags, category, grapheme, script }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Twin {
    pub from: u32,
    pub script: u8,
    pub to: u32,
}

impl Twin {
    pub(crate) const fn new(from: u32, script: u8, to: u32) -> Self {
        Self { from, script, to }
    }
}

pub(crate) fn twin_of(r: u32, script: u8) -> Option<u32> {
    let i = TWINS.partition_point(|t| (t.from, t.script) < (r, script));
    TWINS.get(i).filter(|t| t.from == r && t.script == script).map(|t| t.to)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Range {
    pub lo: u32,
    pub hi: u32,
    pub props: Props,
}

impl Range {
    pub(crate) const fn new(lo: u32, hi: u32, props: Props) -> Self {
        Self { lo, hi, props }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sequence {
    pub offset: u32,
    pub length: u8,
}

impl Sequence {
    pub(crate) const fn new(offset: u32, length: u8) -> Self {
        Self { offset, length }
    }

    pub(crate) fn scalars(self) -> &'static [u32] {
        &RGI_DATA[self.offset as usize..self.offset as usize + usize::from(self.length)]
    }
}

pub(crate) const DIRECT_LIMIT: usize = 0x1000;

static DIRECT: [Props; DIRECT_LIMIT] = build_direct();

#[expect(clippy::large_stack_arrays, reason = "evaluated at compile time into a static")]
const fn build_direct() -> [Props; DIRECT_LIMIT] {
    let mut table = [Props::new(0, CATEGORY_INVALID, 0, 0); DIRECT_LIMIT];
    let mut cp = 0;
    while cp < DIRECT_LIMIT {
        if let Some(props) = search_ranges(cp as u32) {
            table[cp] = props;
        }
        cp += 1;
    }
    table
}

const BLOCK_COUNT: usize = 0x11_0000 >> 8;

static BLOCKS: [u16; BLOCK_COUNT + 1] = build_blocks();

const fn build_blocks() -> [u16; BLOCK_COUNT + 1] {
    assert!(RANGES.len() < u16::MAX as usize);
    let mut table = [0; BLOCK_COUNT + 1];
    let (mut block, mut i) = (0, 0);
    while block <= BLOCK_COUNT {
        while i < RANGES.len() && RANGES[i].hi < (block << 8) as u32 {
            i += 1;
        }
        table[block] = i as u16;
        block += 1;
    }
    table
}

#[inline]
pub(crate) fn lookup(r: u32) -> Option<Props> {
    if (r as usize) < DIRECT_LIMIT {
        let props = DIRECT[r as usize];
        if props.category != CATEGORY_INVALID {
            return Some(props);
        }
    }
    range_index(r).map(|i| RANGES[i].props)
}

#[inline]
pub(crate) fn range_index(r: u32) -> Option<usize> {
    if r > 0x10_FFFF || (0xD800..=0xDFFF).contains(&r) {
        return None;
    }
    let block = (r >> 8) as usize;
    let hi = (usize::from(BLOCKS[block + 1]) + 1).min(RANGES.len());
    search_within(r, usize::from(BLOCKS[block]), hi)
}

pub(crate) const fn search_ranges(r: u32) -> Option<Props> {
    if r > 0x10_FFFF || r >= 0xD800 && r <= 0xDFFF {
        return None;
    }
    match search_within(r, 0, RANGES.len()) {
        Some(i) => Some(RANGES[i].props),
        None => None,
    }
}

const fn search_within(r: u32, lo: usize, hi: usize) -> Option<usize> {
    let (mut lo, mut hi) = (lo, hi);
    while lo < hi {
        let mid = usize::midpoint(lo, hi);
        let entry = RANGES[mid];
        if r < entry.lo {
            hi = mid;
        } else if r > entry.hi {
            lo = mid + 1;
        } else {
            return Some(mid);
        }
    }
    None
}

pub(crate) fn is_variation_sequence(base: u32, selector: u32) -> bool {
    VARIATION_SEQUENCES.binary_search(&(base, selector)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unseen_test_support::{Sha256, hex};

    const GO_TABLES_SHA256: &str = "5b993c2db384a20722df72845ec2b77191186afbd9af7ffc688170dae9d7166f";

    #[test]
    fn tables_match_their_fingerprint_and_the_go_service() {
        let mut h = Sha256::new();
        for r in &RANGES {
            h.update(&r.lo.to_be_bytes());
            h.update(&r.hi.to_be_bytes());
            h.update(&r.props.flags.to_be_bytes());
            h.update(&[r.props.category, r.props.grapheme, r.props.script]);
        }
        for (base, selector) in &VARIATION_SEQUENCES {
            h.update(&base.to_be_bytes());
            h.update(&selector.to_be_bytes());
        }
        for s in &RGI_SEQUENCES {
            h.update(&s.offset.to_be_bytes());
            h.update(&[s.length]);
        }
        for cp in &RGI_DATA {
            h.update(&cp.to_be_bytes());
        }
        for t in &TWINS {
            h.update(&t.from.to_be_bytes());
            h.update(&[t.script]);
            h.update(&t.to.to_be_bytes());
        }
        let sum = hex(&h.finish());
        assert_eq!(sum, TABLES_SHA256);
        assert_eq!(sum, GO_TABLES_SHA256);
        assert_eq!(GENERATED_VERSION, UNICODE_VERSION);
    }

    #[test]
    fn ranges_partition_every_scalar() {
        let mut next = 0;
        for r in &RANGES {
            if next == 0xD800 {
                next = 0xE000;
            }
            assert_eq!(r.lo, next, "gap or overlap before {:X}", r.lo);
            assert!(r.hi >= r.lo);
            assert_ne!(r.props.category, CATEGORY_INVALID);
            assert_ne!(r.props.grapheme, GRAPHEME_INVALID);
            next = r.hi + 1;
        }
        assert_eq!(next, 0x11_0000);
        assert!(lookup(0xD800).is_none() && lookup(0xDFFF).is_none() && lookup(0x11_0000).is_none());
    }

    #[test]
    fn indexed_lookup_matches_ranges() {
        for cp in 0..=0x11_0000 {
            assert_eq!(lookup(cp), search_ranges(cp), "{cp:X}");
        }
    }

    #[test]
    fn sequence_tables_are_sorted_and_unique() {
        assert!(VARIATION_SEQUENCES.windows(2).all(|w| w[0] < w[1]));
        assert!(VARIATION_SEQUENCES.iter().all(|&(_, s)| s == 0xFE0E || s == 0xFE0F));
        assert!(RGI_SEQUENCES.windows(2).all(|w| w[0].scalars() < w[1].scalars()));
        let pool: usize = RGI_SEQUENCES.iter().map(|s| usize::from(s.length)).sum();
        assert_eq!(pool, RGI_DATA.len());
    }

    #[test]
    fn representative_properties() {
        let p = |cp| lookup(cp).expect("assigned");
        assert_eq!(p(u32::from('A')).category, CATEGORY_LU);
        assert_eq!(p(u32::from('ж')).category, CATEGORY_LL);
        assert_eq!(p(0x200B).category, CATEGORY_CF);
        assert_ne!(p(0x200D).flags & FLAG_JOIN_CONTROL, 0);
        assert_ne!(p(0xFE0F).flags & FLAG_VARIATION_SELECTOR, 0);
        assert_eq!(p(0x0301).category, CATEGORY_MN);
        assert_eq!(p(0x1F1E6).grapheme, GRAPHEME_REGIONAL_INDICATOR);
        assert_ne!(p(0x1F469).flags & FLAG_EXTENDED_PICTOGRAPHIC, 0);
        assert_ne!(p(0x1F3FB).flags & FLAG_EMOJI_MODIFIER, 0);
        assert_eq!(p(0x0378).category, CATEGORY_CN);
        assert_eq!(p(0xE000).category, CATEGORY_CO);
        assert_ne!(p(0xFFFE).flags & FLAG_NONCHARACTER, 0);
        assert!(is_variation_sequence(0x263A, 0xFE0F));
        assert!(!is_variation_sequence(u32::from('A'), 0xFE0F));
        assert_eq!(CATEGORY_NAMES[usize::from(CATEGORY_ZS)], "Zs");
        assert_eq!(GRAPHEME_NAMES[usize::from(GRAPHEME_ZWJ)], "ZWJ");
        assert_eq!(p(u32::from('a')).script, SCRIPT_LATIN);
        assert_eq!(p(u32::from('ж')).script, SCRIPT_CYRILLIC);
        assert_eq!(p(0x13AA).script, SCRIPT_CHEROKEE);
        assert_eq!(p(u32::from('中')).script, SCRIPT_INVALID);
    }

    #[test]
    fn twins_cross_an_alphabet_and_keep_the_category() {
        assert_eq!(TWINS.len(), 506);
        assert!(TWINS.windows(2).all(|w| (w[0].from, w[0].script) < (w[1].from, w[1].script)));
        for t in &TWINS {
            let (from, to) = (lookup(t.from).expect("assigned"), lookup(t.to).expect("assigned"));
            assert_ne!(from.script, t.script);
            assert_eq!(to.script, t.script);
            assert_eq!(from.category, to.category);
        }
        let twin = |from: char, script| twin_of(u32::from(from), script).and_then(char::from_u32);
        assert_eq!(twin('а', SCRIPT_LATIN), Some('a'));
        assert_eq!(twin('a', SCRIPT_CYRILLIC), Some('а'));
        assert_eq!(twin('ё', SCRIPT_LATIN), Some('ë'));
        assert_eq!(twin('ο', SCRIPT_LATIN), Some('o'));
        assert_eq!(twin('l', SCRIPT_CYRILLIC), Some('ӏ'));
        assert_eq!(twin('I', SCRIPT_CYRILLIC), Some('І'));
        assert_eq!(twin('Ꭺ', SCRIPT_LATIN), Some('A'));
        assert_eq!(twin('օ', SCRIPT_LATIN), Some('o'));
        for (from, script) in [
            ('ᴙ', SCRIPT_CYRILLIC),
            ('ʙ', SCRIPT_CYRILLIC),
            ('Ａ', SCRIPT_CYRILLIC),
            ('я', SCRIPT_LATIN),
            ('k', SCRIPT_CYRILLIC),
        ] {
            assert_eq!(twin(from, script), None, "{from}");
        }
    }
}
