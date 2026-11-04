use crate::unicode17::{
    CATEGORY_CC, CATEGORY_CF, CATEGORY_CN, CATEGORY_CO, CATEGORY_MC, CATEGORY_ME, CATEGORY_MN, DIRECT_LIMIT,
    FLAG_BIDI_CONTROL, FLAG_DEFAULT_IGNORABLE, FLAG_EMOJI, FLAG_EMOJI_MODIFIER, FLAG_EXTENDED_PICTOGRAPHIC,
    FLAG_JOIN_CONTROL, FLAG_NONCHARACTER, FLAG_VARIATION_SELECTOR, FLAG_WHITE_SPACE, GRAPHEME_EXTEND,
    GRAPHEME_SPACING_MARK, Props, RANGES, RGI_SEQUENCES, Sequence, is_variation_sequence, lookup, range_index,
    search_ranges,
};
use crate::utf8;

pub const MAX_INPUT_BYTES: usize = 100 << 20;
pub const MAX_WORK: u64 = 16 * MAX_INPUT_BYTES as u64;

const MAX_EMOJI_SCALARS: usize = 10;
const MAX_EXTENDERS: usize = 32;
const CHECK_EVERY: u64 = 256;
const LOOKUP_WORK: u64 = 13;
const VARIATION_WORK: u64 = 15;

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    Bom,
    Noncharacter,
    Unassigned,
    PrivateUse,
    BidiControl,
    JoinControl,
    VariationSelector,
    Format,
    DetachedMark,
    TrailingSpace,
    Homoglyph,
    LineBreak,
    InvalidUtf8,
    Control,
    MarkOverflow,
}

impl Reason {
    pub const EDIT: [Reason; 12] = [
        Reason::Bom,
        Reason::Noncharacter,
        Reason::Unassigned,
        Reason::PrivateUse,
        Reason::BidiControl,
        Reason::JoinControl,
        Reason::VariationSelector,
        Reason::Format,
        Reason::DetachedMark,
        Reason::TrailingSpace,
        Reason::Homoglyph,
        Reason::LineBreak,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Reason::Bom => "bom",
            Reason::Noncharacter => "noncharacter",
            Reason::Unassigned => "unassigned",
            Reason::PrivateUse => "private_use",
            Reason::BidiControl => "bidi_control",
            Reason::JoinControl => "join_control",
            Reason::VariationSelector => "variation_selector",
            Reason::Format => "format",
            Reason::DetachedMark => "detached_mark",
            Reason::TrailingSpace => "trailing_space",
            Reason::Homoglyph => "homoglyph",
            Reason::LineBreak => "line_break",
            Reason::InvalidUtf8 => "invalid_utf8",
            Reason::Control => "control",
            Reason::MarkOverflow => "mark_overflow",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Keep,
    Remove(Reason),
    Reject(Reason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    pub action: Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Visible,
    Whitespace,
    Extender,
    Remove(Reason),
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanError {
    Malformed,
    TooLarge,
    Cancelled,
    VerifyFailed,
}

const PLAIN_NO: u8 = 0;
const PLAIN_BASE: u8 = 1;
const PLAIN_SPACE: u8 = 2;
const PLAIN_KEYCAP: u8 = 3;

static PLAIN: [u8; DIRECT_LIMIT] = build_plain();

const fn build_plain() -> [u8; DIRECT_LIMIT] {
    let mut table = [PLAIN_NO; DIRECT_LIMIT];
    let mut r = 0;
    while r < DIRECT_LIMIT as u32 {
        if let Some(props) = search_ranges(r) {
            table[r as usize] = plain_class(r, props);
            if r < 0x80
                && props.flags & (FLAG_EMOJI | FLAG_EXTENDED_PICTOGRAPHIC) == FLAG_EMOJI
                && matches!(classify(r, props), Class::Visible)
            {
                table[r as usize] = PLAIN_KEYCAP;
            }
        }
        r += 1;
    }
    table
}

const fn plain_class(r: u32, p: Props) -> u8 {
    if p.flags & (FLAG_EMOJI | FLAG_EXTENDED_PICTOGRAPHIC) != 0 {
        return PLAIN_NO;
    }
    match classify(r, p) {
        Class::Visible => PLAIN_BASE,
        Class::Whitespace => PLAIN_SPACE,
        _ => PLAIN_NO,
    }
}

static RANGE_PLAIN: [u8; RANGES.len()] = build_range_plain();

const fn build_range_plain() -> [u8; RANGES.len()] {
    let mut table = [PLAIN_NO; RANGES.len()];
    let mut i = 0;
    while i < RANGES.len() {
        if RANGES[i].lo != 0 {
            table[i] = plain_class(RANGES[i].lo, RANGES[i].props);
        }
        i += 1;
    }
    table
}

#[inline]
fn plain_kind(r: u32) -> u8 {
    if (r as usize) < DIRECT_LIMIT {
        return PLAIN[r as usize];
    }
    range_index(r).map_or(PLAIN_NO, |i| RANGE_PLAIN[i])
}

const fn classify(r: u32, p: Props) -> Class {
    if r == 0 || p.category == CATEGORY_CC && p.flags & FLAG_WHITE_SPACE == 0 {
        return Class::Reject;
    }
    if p.flags & FLAG_WHITE_SPACE != 0 {
        return Class::Whitespace;
    }
    if p.flags & FLAG_NONCHARACTER != 0 {
        return Class::Remove(Reason::Noncharacter);
    }
    if p.category == CATEGORY_CN {
        return Class::Remove(Reason::Unassigned);
    }
    if p.category == CATEGORY_CO {
        return Class::Remove(Reason::PrivateUse);
    }
    if p.flags & FLAG_BIDI_CONTROL != 0 {
        return Class::Remove(Reason::BidiControl);
    }
    if p.flags & FLAG_JOIN_CONTROL != 0 {
        return Class::Remove(Reason::JoinControl);
    }
    if p.flags & FLAG_VARIATION_SELECTOR != 0 {
        return Class::Remove(Reason::VariationSelector);
    }
    if p.category == CATEGORY_CF || p.flags & FLAG_DEFAULT_IGNORABLE != 0 {
        return Class::Remove(Reason::Format);
    }
    if p.flags & FLAG_EMOJI_MODIFIER != 0 {
        return Class::Visible;
    }
    if p.category == CATEGORY_MN
        || p.category == CATEGORY_MC
        || p.category == CATEGORY_ME
        || p.grapheme == GRAPHEME_EXTEND
        || p.grapheme == GRAPHEME_SPACING_MARK
    {
        return Class::Extender;
    }
    Class::Visible
}

pub(crate) fn scan(
    input: &[u8],
    cancel: &dyn Fn() -> bool,
    visit: &mut dyn FnMut(Span) -> Result<(), ScanError>,
) -> (Result<(), ScanError>, u64) {
    if cancel() {
        return (Err(ScanError::Cancelled), 0);
    }
    if input.len() > MAX_INPUT_BYTES {
        return (Err(ScanError::TooLarge), 0);
    }
    let mut w = Walker::new(input, cancel, visit);
    let result = w.scan();
    (result, w.work)
}

const RGI_CACHE_SLOTS: usize = 64;

#[derive(Clone, Copy)]
struct RgiEntry {
    pair: (u32, u32),
    lo: u16,
    hi: u16,
    work: u16,
}

struct Walker<'a> {
    input: &'a [u8],
    cancel: &'a dyn Fn() -> bool,
    visit: &'a mut dyn FnMut(Span) -> Result<(), ScanError>,
    work: u64,
    next_check: u64,
    span: Option<Span>,
    cursor: usize,
    rgi_cache: [RgiEntry; RGI_CACHE_SLOTS],
}

impl<'a> Walker<'a> {
    fn new(
        input: &'a [u8],
        cancel: &'a dyn Fn() -> bool,
        visit: &'a mut dyn FnMut(Span) -> Result<(), ScanError>,
    ) -> Self {
        let empty = RgiEntry { pair: (u32::MAX, u32::MAX), lo: 0, hi: 0, work: 0 };
        Walker {
            input,
            cancel,
            visit,
            work: 0,
            next_check: 0,
            span: None,
            cursor: 0,
            rgi_cache: [empty; RGI_CACHE_SLOTS],
        }
    }
}

impl Walker<'_> {
    fn scan(&mut self) -> Result<(), ScanError> {
        let input = self.input;
        let mut pos = 0;
        let mut cluster_base = false;
        if input.starts_with(&BOM) {
            self.emit(0, BOM.len(), Action::Remove(Reason::Bom))?;
            pos = BOM.len();
        }

        while pos < input.len() {
            self.add_work(1)?;
            let byte = input[pos];
            if byte < 0x80 && matches!(PLAIN[usize::from(byte)], PLAIN_BASE | PLAIN_SPACE) {
                self.emit(pos, pos + 1, Action::Keep)?;
                cluster_base = PLAIN[usize::from(byte)] == PLAIN_BASE;
                pos = self.plain_run(pos + 1, &mut cluster_base)?;
                continue;
            }
            let (r, size) = utf8::decode(&input[pos..]);
            if utf8::is_error(r, size) {
                self.emit(pos, pos + 1, Action::Reject(Reason::InvalidUtf8))?;
                pos += 1;
                cluster_base = false;
                continue;
            }

            // Unreachable while `decode` refuses surrogates and scalars past
            // U+10FFFF, which are all the tables leave out.
            let Some(props) = self.lookup(r)? else {
                self.emit(pos, pos + size, Action::Reject(Reason::InvalidUtf8))?;
                pos += size;
                cluster_base = false;
                continue;
            };

            if let Some(end) = self.longest_rgi(pos, r, size, props)? {
                self.emit(pos, end, Action::Keep)?;
                pos = end;
                cluster_base = true;
                continue;
            }
            if let Some(end) = self.emoji_variation_pair(pos, r, size, props)? {
                self.emit(pos, end, Action::Keep)?;
                pos = end;
                cluster_base = true;
                continue;
            }

            match classify(r, props) {
                Class::Visible => {
                    self.emit(pos, pos + size, Action::Keep)?;
                    cluster_base = true;
                    pos = self.plain_run(pos + size, &mut cluster_base)?;
                }
                Class::Whitespace => {
                    self.emit(pos, pos + size, Action::Keep)?;
                    cluster_base = false;
                    pos = self.plain_run(pos + size, &mut cluster_base)?;
                }
                Class::Reject => {
                    self.emit(pos, pos + size, Action::Reject(Reason::Control))?;
                    cluster_base = false;
                    pos += size;
                }
                Class::Remove(reason) => {
                    self.emit(pos, pos + size, Action::Remove(reason))?;
                    cluster_base = false;
                    pos += size;
                }
                Class::Extender => {
                    let (end, count) = self.extender_chain(pos)?;
                    let action = match (cluster_base, count > MAX_EXTENDERS) {
                        (false, _) => Action::Remove(Reason::DetachedMark),
                        (true, false) => Action::Keep,
                        (true, true) => Action::Reject(Reason::MarkOverflow),
                    };
                    self.emit(pos, end, action)?;
                    cluster_base = action == Action::Keep;
                    pos = end;
                }
            }
        }
        self.flush()
    }

    fn plain_run(&mut self, start: usize, cluster_base: &mut bool) -> Result<usize, ScanError> {
        let input = self.input;
        let mut at = start;
        while at < input.len() {
            let byte = input[at];
            if byte < 0x80 {
                match PLAIN[usize::from(byte)] {
                    PLAIN_BASE | PLAIN_SPACE => {
                        let end = input[at + 1..]
                            .iter()
                            .position(|&b| b >= 0x80 || !matches!(PLAIN[usize::from(b)], PLAIN_BASE | PLAIN_SPACE))
                            .map_or(input.len(), |n| at + 1 + n);
                        self.charge(end - at, &[1])?;
                        *cluster_base = PLAIN[usize::from(input[end - 1])] == PLAIN_BASE;
                        at = end;
                    }
                    PLAIN_KEYCAP if input.get(at + 1).is_some_and(|&b| b < 0x80) => {
                        self.charge(1, &[1, LOOKUP_WORK, 1])?;
                        *cluster_base = true;
                        at += 1;
                    }
                    _ => break,
                }
                continue;
            }
            let (r, size) = utf8::decode(&input[at..]);
            if utf8::is_error(r, size) {
                break;
            }
            let kind = plain_kind(r);
            if kind != PLAIN_NO {
                self.charge(1, &[1, LOOKUP_WORK])?;
                *cluster_base = kind == PLAIN_BASE;
                at += size;
                continue;
            }
            if *cluster_base && let Some(end) = self.kept_marks(at, r)? {
                at = end;
                continue;
            }
            break;
        }
        if at > start {
            self.emit(start, at, Action::Keep)?;
        }
        Ok(at)
    }

    fn kept_marks(&mut self, at: usize, first: u32) -> Result<Option<usize>, ScanError> {
        match lookup(first) {
            Some(p)
                if p.flags & (FLAG_EMOJI | FLAG_EXTENDED_PICTOGRAPHIC) == 0
                    && classify(first, p) == Class::Extender => {}
            _ => return Ok(None),
        }
        let input = self.input;
        let (mut end, mut marks) = (at, 0);
        let mut tail: &[u64] = &[];
        while end < input.len() {
            let (r, size) = utf8::decode(&input[end..]);
            if utf8::is_error(r, size) {
                tail = &[1];
                break;
            }
            match lookup(r) {
                Some(p) if classify(r, p) == Class::Extender => {}
                _ => {
                    tail = &[1, LOOKUP_WORK];
                    break;
                }
            }
            marks += 1;
            end += size;
            if marks > MAX_EXTENDERS {
                return Ok(None);
            }
        }
        self.charge(marks + 1, &[1, LOOKUP_WORK])?;
        if !tail.is_empty() {
            self.charge(1, tail)?;
        }
        Ok(Some(end))
    }

    #[inline]
    fn charge(&mut self, count: usize, round: &[u64]) -> Result<(), ScanError> {
        let total = count as u64 * round.iter().sum::<u64>();
        if self.work + total < self.next_check && self.work + total <= MAX_WORK {
            self.work += total;
            return Ok(());
        }
        self.charge_rounds(count, round)
    }

    #[cold]
    fn charge_rounds(&mut self, mut count: usize, round: &[u64]) -> Result<(), ScanError> {
        if count == 1 {
            return round.iter().try_for_each(|&n| self.add_work(n));
        }
        let per: u64 = round.iter().sum();
        loop {
            let room = self.next_check.min(MAX_WORK + 1).saturating_sub(self.work + 1) / per;
            let free = usize::try_from(room).unwrap_or(usize::MAX).min(count);
            self.work += free as u64 * per;
            count -= free;
            if count == 0 {
                return Ok(());
            }
            for &n in round {
                self.add_work(n)?;
            }
            count -= 1;
        }
    }

    fn longest_rgi(
        &mut self,
        pos: usize,
        first: u32,
        first_size: usize,
        props: Props,
    ) -> Result<Option<usize>, ScanError> {
        if props.flags & (FLAG_EMOJI | FLAG_EXTENDED_PICTOGRAPHIC) == 0 {
            return Ok(None);
        }
        let input = self.input;
        let mut runes = [0u32; MAX_EMOJI_SCALARS];
        let mut ends = [0usize; MAX_EMOJI_SCALARS];
        runes[0] = first;
        ends[0] = pos + first_size;
        let mut at = pos + first_size;
        if at >= input.len() {
            return Ok(None);
        }
        self.add_work(1)?;
        let (second, size) = utf8::decode(&input[at..]);
        if utf8::is_error(second, size) || !is_rgi_continuation(second) {
            return Ok(None);
        }
        runes[1] = second;
        at += size;
        ends[1] = at;
        let mut count = 2;
        let (mut lo, mut hi) = self.first_rgi_range(first, second)?;
        if lo == hi {
            return Ok(None);
        }

        let mut best_end = None;
        loop {
            let exact = usize::from(RGI_SEQUENCES[lo].length) == count;
            if exact {
                best_end = Some(ends[count - 1]);
            }
            let longer = if exact { lo + 1 } else { lo };
            if longer == hi || count == runes.len() || at >= input.len() {
                break;
            }
            self.add_work(1)?;
            let (r, size) = utf8::decode(&input[at..]);
            if utf8::is_error(r, size) {
                break;
            }
            runes[count] = r;
            at += size;
            ends[count] = at;
            count += 1;
            (lo, hi) = self.rgi_prefix_range(&runes[..count], longer, hi)?;
            if lo == hi {
                break;
            }
        }
        Ok(best_end)
    }

    fn emoji_variation_pair(
        &mut self,
        pos: usize,
        base: u32,
        size: usize,
        props: Props,
    ) -> Result<Option<usize>, ScanError> {
        if props.flags & FLAG_EXTENDED_PICTOGRAPHIC == 0 {
            return Ok(None);
        }
        let next = pos + size;
        if next >= self.input.len() {
            return Ok(None);
        }
        self.add_work(1)?;
        let (selector, selector_size) = utf8::decode(&self.input[next..]);
        if selector != 0xFE0E && selector != 0xFE0F {
            return Ok(None);
        }
        self.add_work(VARIATION_WORK)?;
        if !is_variation_sequence(base, selector) {
            return Ok(None);
        }
        Ok(Some(next + selector_size))
    }

    fn first_rgi_range(&mut self, first: u32, second: u32) -> Result<(usize, usize), ScanError> {
        let slot = (first ^ second.rotate_left(5)) as usize % RGI_CACHE_SLOTS;
        let entry = self.rgi_cache[slot];
        if entry.pair == (first, second) {
            self.charge(usize::from(entry.work), &[1])?;
            return Ok((usize::from(entry.lo), usize::from(entry.hi)));
        }
        let before = self.work;
        let (lo, hi) = self.rgi_prefix_range(&[first, second], 0, RGI_SEQUENCES.len())?;
        let work = u16::try_from(self.work - before).expect("bounded search work");
        let (lo16, hi16) = (u16::try_from(lo).expect("table index"), u16::try_from(hi).expect("table index"));
        self.rgi_cache[slot] = RgiEntry { pair: (first, second), lo: lo16, hi: hi16, work };
        Ok((lo, hi))
    }

    fn rgi_prefix_range(&mut self, prefix: &[u32], lo: usize, hi: usize) -> Result<(usize, usize), ScanError> {
        let (mut start_lo, mut start_hi) = (lo, hi);
        while start_lo < start_hi {
            let mid = usize::midpoint(start_lo, start_hi);
            if self.compare_rgi_prefix(RGI_SEQUENCES[mid], prefix)?.is_lt() {
                start_lo = mid + 1;
            } else {
                start_hi = mid;
            }
        }
        let (mut end_lo, mut end_hi) = (start_lo, hi);
        while end_lo < end_hi {
            let mid = usize::midpoint(end_lo, end_hi);
            if self.compare_rgi_prefix(RGI_SEQUENCES[mid], prefix)?.is_le() {
                end_lo = mid + 1;
            } else {
                end_hi = mid;
            }
        }
        Ok((start_lo, end_lo))
    }

    fn compare_rgi_prefix(&mut self, candidate: Sequence, prefix: &[u32]) -> Result<std::cmp::Ordering, ScanError> {
        use std::cmp::Ordering;
        let stored = candidate.scalars();
        let same = stored.iter().zip(prefix).take_while(|(s, p)| s == p).count();
        let order = match (stored.get(same), prefix.get(same)) {
            (Some(s), Some(p)) => {
                self.charge(same + 1, &[1])?;
                return Ok(s.cmp(p));
            }
            (None, Some(_)) => Ordering::Less,
            _ => Ordering::Equal,
        };
        self.charge(same, &[1])?;
        Ok(order)
    }

    fn extender_chain(&mut self, pos: usize) -> Result<(usize, usize), ScanError> {
        let input = self.input;
        let (mut at, mut count) = (pos, 0);
        while at < input.len() {
            self.add_work(1)?;
            let (r, size) = utf8::decode(&input[at..]);
            if utf8::is_error(r, size) {
                break;
            }
            match self.lookup(r)? {
                Some(props) if classify(r, props) == Class::Extender => {}
                _ => break,
            }
            count += 1;
            at += size;
            if count > MAX_EXTENDERS {
                while at < input.len() {
                    self.add_work(1)?;
                    let (r, size) = utf8::decode(&input[at..]);
                    let props = self.lookup(r)?;
                    match props {
                        Some(props) if !utf8::is_error(r, size) && classify(r, props) == Class::Extender => {}
                        _ => break,
                    }
                    count += 1;
                    at += size;
                }
                break;
            }
        }
        Ok((at, count))
    }

    fn lookup(&mut self, r: u32) -> Result<Option<Props>, ScanError> {
        self.add_work(LOOKUP_WORK)?;
        Ok(lookup(r))
    }

    fn add_work(&mut self, n: u64) -> Result<(), ScanError> {
        if self.work > MAX_WORK || n > MAX_WORK - self.work {
            return Err(ScanError::Malformed);
        }
        self.work += n;
        if self.work < self.next_check {
            return Ok(());
        }
        self.next_check = self.work + CHECK_EVERY;
        if (self.cancel)() {
            return Err(ScanError::Cancelled);
        }
        Ok(())
    }

    fn emit(&mut self, start: usize, end: usize, action: Action) -> Result<(), ScanError> {
        if start != self.cursor || end <= start || end > self.input.len() {
            return Err(ScanError::Malformed);
        }
        if let Some(span) = &mut self.span
            && (span.action == action || matches!((span.action, action), (Action::Reject(_), Action::Reject(_))))
        {
            span.end = end;
            self.cursor = end;
            return Ok(());
        }
        self.flush()?;
        self.span = Some(Span { start, end, action });
        self.cursor = end;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), ScanError> {
        match self.span.take() {
            Some(span) => (self.visit)(span),
            None => Ok(()),
        }
    }
}

fn is_rgi_continuation(r: u32) -> bool {
    is_variation_selector(r)
        || r == 0x200D
        || r == 0x20E3
        || (0x1F1E6..=0x1F1FF).contains(&r)
        || (0x1F3FB..=0x1F3FF).contains(&r)
        || (0xE0020..=0xE007F).contains(&r)
}

fn is_variation_selector(r: u32) -> bool {
    (0x180B..=0x180D).contains(&r) || r == 0x180F || (0xFE00..=0xFE0F).contains(&r) || (0xE0100..=0xE01EF).contains(&r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> bool {
        false
    }

    fn spans(input: &[u8]) -> (Vec<Span>, Result<(), ScanError>) {
        let mut out = Vec::new();
        let (result, _) = scan(input, &never, &mut |span| {
            out.push(span);
            Ok(())
        });
        (out, result)
    }

    fn apply(input: &[u8]) -> (Vec<u8>, bool) {
        let (spans, result) = spans(input);
        result.expect("scan");
        let mut kept = Vec::new();
        let mut reject = false;
        let mut cursor = 0;
        for (i, s) in spans.iter().enumerate() {
            assert_eq!(s.start, cursor, "partition");
            assert!(s.end > s.start);
            if i > 0 {
                assert_ne!(spans[i - 1].action, s.action, "coalesced");
            }
            cursor = s.end;
            match s.action {
                Action::Keep => kept.extend_from_slice(&input[s.start..s.end]),
                Action::Reject(_) => reject = true,
                Action::Remove(_) => {}
            }
        }
        assert_eq!(cursor, input.len());
        (kept, reject)
    }

    #[test]
    fn single_scalars_follow_their_class() {
        let mut buf = [0u8; 4];
        for c in (0..=0x10_FFFF).filter_map(char::from_u32) {
            let r = c as u32;
            let s = c.encode_utf8(&mut buf).as_bytes();
            let class = classify(r, lookup(r).expect("assigned"));
            let want = match class {
                Class::Visible | Class::Whitespace => Action::Keep,
                Class::Reject => Action::Reject(Reason::Control),
                Class::Extender => Action::Remove(Reason::DetachedMark),
                Class::Remove(_) if r == 0xFEFF => Action::Remove(Reason::Bom),
                Class::Remove(reason) => Action::Remove(reason),
            };
            let (spans, result) = spans(s);
            result.expect("scan");
            assert_eq!(spans, [Span { start: 0, end: s.len(), action: want }], "{r:04X}");
        }
    }

    #[test]
    fn plain_table_matches_the_policy() {
        for r in 0..DIRECT_LIMIT as u32 {
            let Some(p) = lookup(r) else {
                assert_eq!(PLAIN[r as usize], PLAIN_NO);
                continue;
            };
            let emoji = p.flags & (FLAG_EMOJI | FLAG_EXTENDED_PICTOGRAPHIC);
            let want = match classify(r, p) {
                Class::Visible if emoji == 0 => PLAIN_BASE,
                Class::Whitespace if emoji == 0 => PLAIN_SPACE,
                Class::Visible if r < 0x80 && emoji == FLAG_EMOJI => PLAIN_KEYCAP,
                _ => PLAIN_NO,
            };
            assert_eq!(PLAIN[r as usize], want, "{r:04X}");
        }
        assert!(b"#*0123456789".iter().all(|&b| PLAIN[usize::from(b)] == PLAIN_KEYCAP));
    }

    #[test]
    fn extender_limit() {
        let chain = |n: usize| "\u{301}".repeat(n);
        assert_eq!(apply(format!("A{}B", chain(32)).as_bytes()), (format!("A{}B", chain(32)).into_bytes(), false));
        assert!(apply(format!("A{}B", chain(33)).as_bytes()).1);
        assert_eq!(apply(format!("{}B", chain(33)).as_bytes()), (b"B".to_vec(), false));
    }

    #[test]
    fn budget_is_exact() {
        let mut visit = |_: Span| Ok(());
        let mut w = Walker::new(b"", &never, &mut visit);
        w.work = MAX_WORK;
        assert_eq!(w.add_work(1), Err(ScanError::Malformed));
        w.work = MAX_WORK - 1;
        assert_eq!(w.add_work(1), Ok(()));
        assert_eq!(w.add_work(1), Err(ScanError::Malformed));
        assert_eq!(w.work, MAX_WORK);
    }

    #[test]
    fn charge_replays_add_work() {
        let rounds: [&[u64]; 4] = [&[1], &[1, LOOKUP_WORK], &[1, LOOKUP_WORK, 1], &[1, 0, 2]];
        let starts = [(0, 0), (240, 250), (255, 256), (MAX_WORK - 40, MAX_WORK - 30), (MAX_WORK - 3, MAX_WORK + 300)];
        for round in rounds {
            for (work, next_check) in starts {
                for count in [0, 1, 2, 7, 40, 300] {
                    let run = |batched: bool| {
                        let polls = std::cell::Cell::new(0);
                        let cancel = || {
                            polls.set(polls.get() + 1);
                            polls.get() == 3
                        };
                        let mut visit = |_: Span| Ok(());
                        let mut w = Walker::new(b"", &cancel, &mut visit);
                        (w.work, w.next_check) = (work, next_check);
                        let result = if batched {
                            w.charge(count, round)
                        } else {
                            (0..count).try_for_each(|_| round.iter().try_for_each(|&n| w.add_work(n)))
                        };
                        (result, w.work, w.next_check, polls.get())
                    };
                    assert_eq!(run(true), run(false), "{round:?} from {work}/{next_check} x{count}");
                }
            }
        }
    }

    #[test]
    fn cancellation_is_checked_at_start_and_every_256_units() {
        let (result, work) = scan(b"x", &|| true, &mut |_| Ok(()));
        assert_eq!((result, work), (Err(ScanError::Cancelled), 0));
        let calls = std::cell::Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() > 2
        };
        let (result, work) = scan(&[b'a'; 1000], &cancel, &mut |_| Ok(()));
        assert_eq!(result, Err(ScanError::Cancelled));
        assert_eq!(work, 257);
    }
}
