use crate::policy::{MAX_INPUT_BYTES, Reason, ScanError};
use crate::unicode17::{
    CATEGORY_LL, CATEGORY_LU, CATEGORY_MC, CATEGORY_MN, CATEGORY_ND, CATEGORY_NO, DIRECT_LIMIT, FLAG_WHITE_SPACE,
    Props, SCRIPT_CHEROKEE, SCRIPT_INVALID, SCRIPT_LATIN, lookup, twin_of,
};
use crate::{Fragment, MAX_EDITS, Total, utf8};

pub const MAX_NORMALIZE_WORK: u64 = 32 * MAX_INPUT_BYTES as u64;

const CHECK_EVERY: u64 = 256;
const LOOKUP_WORK: u64 = 13;
const TWIN_WORK: u64 = 9;

type Counts = [usize; SCRIPT_CHEROKEE as usize + 1];

pub(crate) struct Normalizer<'a> {
    cancel: &'a dyn Fn() -> bool,
    out: Option<&'a mut Vec<u8>>,
    pub ledger: Option<Ledger>,

    written: usize,
    pub edits: usize,
    pub work: u64,
    next_check: u64,

    space: Vec<u8>,
    space_at: usize,
    space_end: usize,
    space_runs: usize,
    token: Vec<u8>,
    token_at: usize,
    token_ascii: bool,
    cr: Option<usize>,
}

impl<'a> Normalizer<'a> {
    pub(crate) fn new(cancel: &'a dyn Fn() -> bool, out: Option<&'a mut Vec<u8>>, report: bool) -> Self {
        Normalizer {
            cancel,
            out,
            ledger: report.then(Ledger::default),
            written: 0,
            edits: 0,
            work: 0,
            next_check: 0,
            space: Vec::new(),
            space_at: 0,
            space_end: 0,
            space_runs: 0,
            token: Vec::new(),
            token_at: 0,
            token_ascii: true,
            cr: None,
        }
    }

    pub(crate) fn write(&mut self, p: &[u8], at: usize) -> Result<(), ScanError> {
        let mut i = 0;
        while i < p.len() {
            self.add_work(1)?;
            let (r, size) = utf8::decode(&p[i..]);
            let (space, line) = self.whitespace(r)?;
            if line {
                self.line(r, at + i, size)?;
            } else if space {
                self.resolve_cr()?;
                self.flush_token()?;
                if self.space.is_empty() {
                    self.space_at = at + i;
                    self.space_runs = 0;
                }
                if self.space.is_empty() || self.space_end != at + i {
                    self.space_runs += 1;
                }
                self.space_end = at + i + size;
                self.space.extend_from_slice(&p[i..i + size]);
            } else {
                self.resolve_cr()?;
                if !self.space.is_empty() {
                    let space = std::mem::take(&mut self.space);
                    let emitted = self.emit(&space);
                    self.space = space;
                    self.space.clear();
                    emitted?;
                    if let Some(ledger) = &mut self.ledger {
                        ledger.drain(&mut Cursor::default());
                    }
                }
                if self.token.is_empty() {
                    self.token_at = at + i;
                }
                self.token.extend_from_slice(&p[i..i + size]);
                self.token_ascii = self.token_ascii && size == 1;
            }
            i += size;
        }
        Ok(())
    }

    pub(crate) fn removed(&mut self, fragment: Fragment) {
        let pending = self.cr.is_some() || !self.space.is_empty() || !self.token.is_empty();
        if let Some(ledger) = &mut self.ledger {
            ledger.removed(fragment, pending);
        }
    }

    pub(crate) fn close(&mut self) -> Result<(), ScanError> {
        self.resolve_cr()?;
        self.flush_token()?;
        if !self.space.is_empty() {
            self.edits += 1;
            self.drop_space();
        }
        Ok(())
    }

    fn whitespace(&mut self, r: u32) -> Result<(bool, bool), ScanError> {
        match r {
            0x20 | 0x09 => return Ok((true, false)),
            0x0A..=0x0D => return Ok((false, true)),
            0..0x80 => return Ok((false, false)),
            _ => {}
        }
        let props = self.props(r)?;
        if props.flags & FLAG_WHITE_SPACE == 0 {
            return Ok((false, false));
        }
        if matches!(r, 0x85 | 0x2028 | 0x2029) {
            return Ok((false, true));
        }
        Ok((true, false))
    }

    fn line(&mut self, r: u32, at: usize, size: usize) -> Result<(), ScanError> {
        self.flush_token()?;
        if !self.space.is_empty() {
            self.edits += 1;
            self.drop_space();
        }
        if let Some(cr) = self.cr.take() {
            self.emit(b"\n")?;
            let with = if r == 0x0A { None } else { Some('\n') };
            self.report_line_break(cr, 1, with);
            if r == 0x0A {
                return Ok(());
            }
        }
        match r {
            0x0A => self.emit(b"\n"),
            0x0C => self.emit(b"\x0C"),
            0x0D => {
                self.cr = Some(at);
                self.edits += 1;
                Ok(())
            }
            _ => {
                self.edits += 1;
                self.emit(b"\n")?;
                self.report_line_break(at, size, Some('\n'));
                Ok(())
            }
        }
    }

    fn resolve_cr(&mut self) -> Result<(), ScanError> {
        let Some(cr) = self.cr.take() else {
            return Ok(());
        };
        self.emit(b"\n")?;
        self.report_line_break(cr, 1, Some('\n'));
        Ok(())
    }

    fn report_line_break(&mut self, at: usize, size: usize, with: Option<char>) {
        if let Some(ledger) = &mut self.ledger {
            let fragment = Fragment { start: at, end: at + size, reason: Reason::LineBreak, with };
            ledger.count(fragment.reason, 1, size);
            ledger.list(fragment);
            ledger.drain(&mut Cursor::default());
        }
    }

    fn drop_space(&mut self) {
        let len = self.space.len();
        self.space.clear();
        let Some(ledger) = &mut self.ledger else {
            return;
        };
        ledger.count(Reason::TrailingSpace, self.space_runs, len);
        let mut c = Cursor::default();
        let mut cur = 0;
        while cur < len {
            let start = ledger.skip(&mut c, self.space_at + cur);
            let end = ledger.queue.get(c.next).map_or(len, |f| (f.start - self.space_at - c.shift).min(len));
            ledger.list(Fragment { start, end: start + end - cur, reason: Reason::TrailingSpace, with: None });
            cur = end;
        }
        ledger.drain(&mut c);
    }

    fn flush_token(&mut self) -> Result<(), ScanError> {
        if self.token.is_empty() {
            return Ok(());
        }
        let token = std::mem::take(&mut self.token);
        let result = self.normalize_token(&token);
        self.token = token;
        self.token.clear();
        self.token_ascii = true;
        result
    }

    fn normalize_token(&mut self, token: &[u8]) -> Result<(), ScanError> {
        let mut rewritten = None;
        if !self.token_ascii {
            rewritten = self.rewrite_words(token)?;
            let current = rewritten.as_deref().unwrap_or(token);
            if current.contains(&b'.')
                && let Some(host) = self.rewrite_host(current)?
            {
                rewritten = Some(host);
            }
        }
        let out = rewritten.as_deref().unwrap_or(token);
        self.emit(out)?;
        if let Some(ledger) = &mut self.ledger {
            let mut c = Cursor::default();
            if rewritten.is_some() {
                ledger.replacements(&mut c, token, self.token_at, out);
            }
            ledger.drain(&mut c);
        }
        Ok(())
    }

    fn rewrite_words(&mut self, t: &[u8]) -> Result<Option<Vec<u8>>, ScanError> {
        let mut out: Option<Vec<u8>> = None;
        let (mut copied, mut start) = (0, None);
        let mut counts: Counts = [0; SCRIPT_CHEROKEE as usize + 1];
        let mut i = 0;
        loop {
            let (mut word, mut size, mut props) = (false, 0, Props::default());
            if i < t.len() {
                (props, size, _) = self.decode(&t[i..])?;
                word = is_letter(props.category) || is_mark(props.category);
            }
            if word {
                if start.is_none() {
                    start = Some(i);
                    counts = [0; SCRIPT_CHEROKEE as usize + 1];
                }
                if is_letter(props.category) {
                    counts[usize::from(props.script)] += 1;
                }
                i += size;
                continue;
            }
            if let Some(s) = start.take() {
                let main = majority_script(&counts);
                if main != SCRIPT_INVALID {
                    let o = out.get_or_insert_with(Vec::new);
                    self.put(o, &t[copied..s])?;
                    self.replace_word(o, &t[s..i], main)?;
                    copied = i;
                }
            }
            if i == t.len() {
                break;
            }
            i += size;
        }
        self.finish(out, &t[copied..])
    }

    fn replace_word(&mut self, out: &mut Vec<u8>, word: &[u8], main: u8) -> Result<(), ScanError> {
        let mut i = 0;
        while i < word.len() {
            let (props, size, r) = self.decode(&word[i..])?;
            if is_letter(props.category)
                && props.script != SCRIPT_INVALID
                && props.script != main
                && let Some(twin) = self.twin(r, main)?
            {
                self.put_scalar(out, twin)?;
                self.edits += 1;
                i += size;
                continue;
            }
            self.put(out, &word[i..i + size])?;
            i += size;
        }
        Ok(())
    }

    fn rewrite_host(&mut self, t: &[u8]) -> Result<Option<Vec<u8>>, ScanError> {
        let mut out: Option<Vec<u8>> = None;
        let mut copied = 0;
        let (mut first, mut from, mut last, mut end, mut labels) = (0, None, 0, 0, 0);
        let mut i = 0;
        loop {
            let (mut label, mut size) = (false, 1);
            if i < t.len() && t[i] != b'.' {
                let (props, s, _) = self.decode(&t[i..])?;
                size = s;
                label =
                    is_letter(props.category) || is_mark(props.category) || is_number(props.category) || t[i] == b'-';
            }
            if label {
                if from.is_none() {
                    from = Some(i);
                    if labels == 0 {
                        first = i;
                    }
                }
                i += size;
                continue;
            }
            if let Some(f) = from.take() {
                labels += 1;
                (last, end) = (f, i);
                if i < t.len() && t[i] == b'.' {
                    i += 1;
                    continue;
                }
            }
            if labels >= 2 {
                copied = self.rewrite_labels(&mut out, t, copied, first, last, end)?;
            }
            labels = 0;
            if i >= t.len() {
                break;
            }
            i += size;
        }
        self.finish(out, &t[copied..])
    }

    fn rewrite_labels(
        &mut self,
        out: &mut Option<Vec<u8>>,
        t: &[u8],
        mut copied: usize,
        first: usize,
        last: usize,
        end: usize,
    ) -> Result<usize, ScanError> {
        let Some(top) = self.tld_script(&t[last..end])? else {
            return Ok(copied);
        };
        let mut label = first;
        while label < end {
            let stop = t[label..end].iter().position(|&b| b == b'.').map_or(end, |dot| label + dot);
            if self.label_converts(&t[label..stop], top)? {
                let o = out.get_or_insert_with(Vec::new);
                self.put(o, &t[copied..label])?;
                self.latin_label(o, &t[label..stop])?;
                copied = stop;
            }
            label = stop + 1;
        }
        Ok(copied)
    }

    fn tld_script(&mut self, tld: &[u8]) -> Result<Option<u8>, ScanError> {
        if tld.len() > 4 && tld[..4].eq_ignore_ascii_case(b"xn--") {
            let punycode = tld[4..].iter().all(|&b| is_ascii_letter(b) || b.is_ascii_digit() || b == b'-');
            return Ok(punycode.then_some(SCRIPT_LATIN));
        }
        let (mut top, mut letters) = (SCRIPT_INVALID, 0);
        let mut i = 0;
        while i < tld.len() {
            let (props, size, _) = self.decode(&tld[i..])?;
            if is_letter(props.category) {
                if letters == 0 {
                    top = props.script;
                } else if props.script != top {
                    top = SCRIPT_LATIN;
                }
                letters += 1;
            } else if !is_mark(props.category) {
                return Ok(None);
            }
            i += size;
        }
        Ok((letters >= 2).then_some(top))
    }

    fn label_converts(&mut self, label: &[u8], top: u8) -> Result<bool, ScanError> {
        let mut foreign = false;
        let mut i = 0;
        while i < label.len() {
            let (props, size, r) = self.decode(&label[i..])?;
            if is_letter(props.category) && props.script != SCRIPT_LATIN {
                if props.script == SCRIPT_INVALID || props.script == top || self.twin(r, SCRIPT_LATIN)?.is_none() {
                    return Ok(false);
                }
                foreign = true;
            }
            i += size;
        }
        Ok(foreign)
    }

    fn latin_label(&mut self, out: &mut Vec<u8>, label: &[u8]) -> Result<(), ScanError> {
        let mut i = 0;
        while i < label.len() {
            let (props, size, r) = self.decode(&label[i..])?;
            if is_letter(props.category) && props.script != SCRIPT_LATIN {
                let twin = self.twin(r, SCRIPT_LATIN)?.unwrap_or(r);
                self.put_scalar(out, twin)?;
                self.edits += 1;
            } else {
                self.put(out, &label[i..i + size])?;
            }
            i += size;
        }
        Ok(())
    }

    fn put(&self, out: &mut Vec<u8>, b: &[u8]) -> Result<(), ScanError> {
        if b.len() > MAX_INPUT_BYTES - self.written - out.len() {
            return Err(ScanError::TooLarge);
        }
        out.extend_from_slice(b);
        Ok(())
    }

    fn put_scalar(&self, out: &mut Vec<u8>, r: u32) -> Result<(), ScanError> {
        let c = char::from_u32(r).unwrap_or(char::REPLACEMENT_CHARACTER);
        self.put(out, c.encode_utf8(&mut [0; 4]).as_bytes())
    }

    fn finish(&self, out: Option<Vec<u8>>, rest: &[u8]) -> Result<Option<Vec<u8>>, ScanError> {
        let Some(mut o) = out else {
            return Ok(None);
        };
        self.put(&mut o, rest)?;
        Ok(Some(o))
    }

    fn decode(&mut self, p: &[u8]) -> Result<(Props, usize, u32), ScanError> {
        self.add_work(1)?;
        let (r, size) = utf8::decode(p);
        Ok((self.props(r)?, size, r))
    }

    fn props(&mut self, r: u32) -> Result<Props, ScanError> {
        let cost = match r {
            0..0x80 => 0,
            _ if (r as usize) < DIRECT_LIMIT => 1,
            _ => LOOKUP_WORK,
        };
        self.add_work(cost)?;
        Ok(lookup(r).unwrap_or_default())
    }

    fn twin(&mut self, r: u32, script: u8) -> Result<Option<u32>, ScanError> {
        self.add_work(TWIN_WORK)?;
        Ok(twin_of(r, script))
    }

    fn emit(&mut self, b: &[u8]) -> Result<(), ScanError> {
        if b.is_empty() {
            return Ok(());
        }
        if b.len() > MAX_INPUT_BYTES - self.written {
            return Err(ScanError::TooLarge);
        }
        self.written += b.len();
        if let Some(out) = &mut self.out {
            out.extend_from_slice(b);
        }
        Ok(())
    }

    fn add_work(&mut self, n: u64) -> Result<(), ScanError> {
        if self.work > MAX_NORMALIZE_WORK || n > MAX_NORMALIZE_WORK - self.work {
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
}

fn majority_script(counts: &Counts) -> u8 {
    let (mut total, mut present, mut best) = (0, 0, SCRIPT_INVALID);
    for script in SCRIPT_LATIN..=SCRIPT_CHEROKEE {
        let n = counts[usize::from(script)];
        if n == 0 {
            continue;
        }
        total += n;
        present += 1;
        if best == SCRIPT_INVALID || n > counts[usize::from(best)] {
            best = script;
        }
    }
    if present < 2 || 2 * counts[usize::from(best)] <= total {
        return SCRIPT_INVALID;
    }
    best
}

fn is_ascii_letter(b: u8) -> bool {
    (b | 0x20).is_ascii_lowercase()
}

fn is_letter(c: u8) -> bool {
    (CATEGORY_LL..=CATEGORY_LU).contains(&c)
}

fn is_mark(c: u8) -> bool {
    (CATEGORY_MC..=CATEGORY_MN).contains(&c)
}

fn is_number(c: u8) -> bool {
    (CATEGORY_ND..=CATEGORY_NO).contains(&c)
}

#[derive(Debug, Default)]
pub(crate) struct Ledger {
    pub edits: Vec<Fragment>,
    pub truncated: bool,
    pub totals: [Total; Reason::EDIT.len()],
    queue: Vec<Fragment>,
}

#[derive(Default)]
struct Cursor {
    next: usize,
    shift: usize,
}

impl Ledger {
    fn removed(&mut self, fragment: Fragment, pending: bool) {
        self.count(fragment.reason, 1, fragment.end - fragment.start);
        if !pending {
            self.list(fragment);
        } else if self.queue.len() < MAX_EDITS - self.edits.len() {
            self.queue.push(fragment);
        } else {
            self.truncated = true;
        }
    }

    fn count(&mut self, reason: Reason, spans: usize, bytes: usize) {
        if let Some(i) = Reason::EDIT.iter().position(|&r| r == reason) {
            self.totals[i].spans += spans;
            self.totals[i].bytes += bytes;
        }
    }

    fn list(&mut self, fragment: Fragment) {
        if self.edits.len() < MAX_EDITS {
            self.edits.push(fragment);
        } else {
            self.truncated = true;
        }
    }

    fn skip(&mut self, c: &mut Cursor, at: usize) -> usize {
        while let Some(&f) = self.queue.get(c.next) {
            if f.start > at + c.shift {
                break;
            }
            c.shift += f.end - f.start;
            c.next += 1;
            self.list(f);
        }
        at + c.shift
    }

    fn drain(&mut self, c: &mut Cursor) {
        for i in c.next..self.queue.len() {
            let f = self.queue[i];
            self.list(f);
        }
        self.queue.clear();
        c.next = 0;
    }

    fn replacements(&mut self, c: &mut Cursor, token: &[u8], at: usize, out: &[u8]) {
        let (mut ti, mut oi) = (0, 0);
        while ti < token.len() && oi < out.len() {
            let (from, size) = utf8::decode(&token[ti..]);
            let (to, out_size) = utf8::decode(&out[oi..]);
            if from != to {
                let start = self.skip(c, at + ti);
                self.count(Reason::Homoglyph, 1, size);
                self.list(Fragment { start, end: start + size, reason: Reason::Homoglyph, with: char::from_u32(to) });
            }
            ti += size;
            oi += out_size;
        }
    }
}
