#![forbid(unsafe_code)]

mod normalize;
mod policy;
mod sniff;
mod unicode17;
#[rustfmt::skip]
#[allow(dead_code, clippy::all, clippy::pedantic, reason = "generated tables")]
mod unicode17_data;
mod utf8;

use normalize::{Ledger, Normalizer};
use policy::{Action, ScanError};

pub use normalize::MAX_NORMALIZE_WORK;
pub use policy::{MAX_INPUT_BYTES, MAX_WORK, Reason};
pub use unicode17::UNICODE_VERSION;

#[doc(hidden)]
pub const UNICODE_TABLES_SHA256: &str = unicode17::TABLES_SHA256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Code {
    UnsupportedType,
    TooLarge,
    Malformed,
    VerifyFailed,
    Cancelled,
}

impl Code {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedType => "unsupported_type",
            Self::TooLarge => "too_large",
            Self::Malformed => "malformed",
            Self::VerifyFailed => "verify_failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for Code {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOutcome {
    Text { mime: &'static str, ext: &'static str, output: Vec<u8> },
    Binary { mime: &'static str, ext: &'static str },
}

pub fn sanitize_text(input: &[u8]) -> Result<Vec<u8>, Code> {
    sanitize_text_with(input, &|| false)
}

pub fn sanitize_text_with(input: &[u8], cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, Code> {
    trace_with(input, cancel).result
}

pub fn sanitize_file(name: impl AsRef<[u8]>, body: &[u8]) -> Result<FileOutcome, Code> {
    sanitize_file_with(name, body, &|| false)
}

pub fn sanitize_file_with(name: impl AsRef<[u8]>, body: &[u8], cancel: &dyn Fn() -> bool) -> Result<FileOutcome, Code> {
    let ext = sniff::gate_ext(name.as_ref()).ok_or(Code::UnsupportedType)?;
    let typ = sniff::sniff_type(body).ok_or(Code::UnsupportedType)?;
    if typ.mime != "text/plain" {
        return Ok(FileOutcome::Binary { mime: typ.mime, ext: typ.ext });
    }
    let mime = sniff::text_subtype(ext).ok_or(Code::UnsupportedType)?;
    let output = sanitize_text_with(body, cancel)?;
    Ok(FileOutcome::Text { mime, ext, output })
}

pub const MAX_EDITS: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fragment {
    pub start: usize,
    pub end: usize,
    pub reason: Reason,
    pub with: Option<char>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Total {
    pub spans: usize,
    pub bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub output: Vec<u8>,
    pub edits: Vec<Fragment>,
    pub truncated: bool,
    totals: [Total; Reason::EDIT.len()],
}

impl Report {
    #[must_use]
    pub fn total(&self, reason: Reason) -> Total {
        Reason::EDIT.iter().position(|&r| r == reason).map_or_else(Total::default, |i| self.totals[i])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Refusal {
    pub code: Code,
    pub at: Option<Fragment>,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.at {
            Some(at) => write!(f, "{} at bytes {}..{}: {}", self.code, at.start, at.end, at.reason.as_str()),
            None => self.code.fmt(f),
        }
    }
}

impl std::error::Error for Refusal {}

pub fn sanitize_text_report(input: &[u8]) -> Result<Report, Refusal> {
    sanitize_text_report_with(input, &|| false)
}

pub fn sanitize_text_report_with(input: &[u8], cancel: &dyn Fn() -> bool) -> Result<Report, Refusal> {
    let run = run(input, cancel, true);
    match run.trace.result {
        Ok(output) => {
            let Ledger { edits, truncated, totals, .. } = run.ledger.unwrap_or_default();
            Ok(Report { output, edits, truncated, totals })
        }
        Err(code) => Err(Refusal { code, at: run.rejected }),
    }
}

#[must_use]
pub fn text_extensions() -> &'static [(&'static str, &'static str)] {
    &sniff::TEXT_SUBTYPES
}

#[doc(hidden)]
#[derive(Debug)]
pub struct Trace {
    pub result: Result<Vec<u8>, Code>,
    pub work: u64,
    pub verify_work: u64,
}

#[doc(hidden)]
#[must_use]
pub fn trace(input: &[u8]) -> Trace {
    trace_with(input, &|| false)
}

fn trace_with(input: &[u8], cancel: &dyn Fn() -> bool) -> Trace {
    run(input, cancel, false).trace
}

struct Run {
    trace: Trace,
    rejected: Option<Fragment>,
    ledger: Option<Ledger>,
}

fn run(input: &[u8], cancel: &dyn Fn() -> bool, report: bool) -> Run {
    // Oversized input is refused by the scan; reserving for it first could
    // abort on the allocation instead.
    let mut output = Vec::with_capacity(if input.len() > MAX_INPUT_BYTES { 0 } else { input.len() });
    let mut rejected = None;
    let mut n = Normalizer::new(cancel, Some(&mut output), report);
    let (mut scanned, work) = policy::scan(input, cancel, &mut |span| match span.action {
        Action::Keep => n.write(&input[span.start..span.end], span.start),
        Action::Remove(reason) => {
            n.removed(Fragment { start: span.start, end: span.end, reason, with: None });
            Ok(())
        }
        Action::Reject(reason) => {
            rejected = Some(Fragment { start: span.start, end: span.end, reason, with: None });
            Err(ScanError::Malformed)
        }
    });
    if scanned.is_ok() {
        scanned = n.close();
    }
    let work = work + n.work;
    let ledger = n.ledger.take();
    if let Err(err) = scanned {
        let code = match err {
            ScanError::TooLarge => Code::TooLarge,
            ScanError::Cancelled => Code::Cancelled,
            ScanError::Malformed | ScanError::VerifyFailed => Code::Malformed,
        };
        return Run { trace: Trace { result: Err(code), work, verify_work: 0 }, rejected, ledger: None };
    }

    let mut v = Normalizer::new(cancel, None, false);
    let (mut verified, verify_work) = policy::scan(&output, cancel, &mut |span| match span.action {
        Action::Keep => v.write(&output[span.start..span.end], span.start),
        Action::Remove(_) | Action::Reject(_) => Err(ScanError::VerifyFailed),
    });
    if verified.is_ok() {
        verified = v.close();
    }
    if verified.is_ok() && v.edits != 0 {
        verified = Err(ScanError::VerifyFailed);
    }
    let verify_work = verify_work + v.work;
    let result = match verified {
        Ok(()) => Ok(output),
        Err(ScanError::VerifyFailed) => Err(Code::VerifyFailed),
        Err(ScanError::Cancelled) => Err(Code::Cancelled),
        Err(ScanError::Malformed | ScanError::TooLarge) => Err(Code::Malformed),
    };
    Run { trace: Trace { result, work, verify_work }, rejected: None, ledger }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_examples() {
        assert_eq!(sanitize_text(b"\xEF\xBB\xBFa\xE2\x80\x8Bb"), Ok(b"ab".to_vec()));
        assert_eq!(sanitize_text(b"a\0b"), Err(Code::Malformed));
        assert_eq!(sanitize_text(b"a\xFFb"), Err(Code::Malformed));
        assert_eq!(
            sanitize_text("e\u{301} \u{1F469}\u{200D}\u{1F4BB}".as_bytes()),
            Ok("e\u{301} \u{1F469}\u{200D}\u{1F4BB}".into())
        );
        assert_eq!(sanitize_text_with(b"a", &|| true), Err(Code::Cancelled));
    }

    fn removals(input: &str) -> Vec<(usize, usize, Reason)> {
        let report = sanitize_text_report(input.as_bytes()).expect("accepted");
        assert!(report.edits.iter().all(|f| f.with.is_none()));
        report.edits.iter().map(|f| (f.start, f.end, f.reason)).collect()
    }

    type Edit = (usize, usize, Reason, Option<char>);

    fn edits(input: &str) -> (String, Vec<Edit>) {
        let report = sanitize_text_report(input.as_bytes()).expect("accepted");
        let mut applied = Vec::new();
        let mut at = 0;
        for f in &report.edits {
            assert!(f.start >= at && f.end > f.start, "{f:?} out of order");
            applied.extend_from_slice(&input.as_bytes()[at..f.start]);
            if let Some(c) = f.with {
                applied.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
            }
            at = f.end;
        }
        applied.extend_from_slice(&input.as_bytes()[at..]);
        assert_eq!(applied, report.output, "fragments of {input:?}");
        let output = String::from_utf8(report.output).expect("UTF-8");
        (output, report.edits.iter().map(|f| (f.start, f.end, f.reason, f.with)).collect())
    }

    #[test]
    fn normalization_examples() {
        let text = |input: &str| sanitize_text(input.as_bytes()).map(|o| String::from_utf8(o).expect("UTF-8"));
        assert_eq!(text("\u{430}pple g\u{3BF}ogle"), Ok("apple google".into()));
        assert_eq!(text("\u{43F}o\u{43B}\u{435}"), Ok("\u{43F}\u{43E}\u{43B}\u{435}".into()));
        assert_eq!(
            text("[a-z\u{430}-\u{44F}] IT-\u{43E}\u{442}\u{434}\u{435}\u{43B}"),
            Ok("[a-z\u{430}-\u{44F}] IT-\u{43E}\u{442}\u{434}\u{435}\u{43B}".into())
        );
        assert_eq!(
            text("https://\u{440}\u{430}\u{443}\u{440}\u{430}\u{4CF}.com/login"),
            Ok("https://paypal.com/login".into())
        );
        assert_eq!(
            text("\u{43F}\u{440}\u{438}\u{43C}\u{435}\u{440}.\u{440}\u{444}"),
            Ok("\u{43F}\u{440}\u{438}\u{43C}\u{435}\u{440}.\u{440}\u{444}".into())
        );
        assert_eq!(text("a \t\r\nb\rc\x0Bd\u{85}e\u{2028}f\x0Cg \u{A0}"), Ok("a\nb\nc\nd\ne\nf\x0Cg".into()));
        assert_eq!(text("  x\u{A0} y"), Ok("  x\u{A0} y".into()));
    }

    #[test]
    fn report_normalization() {
        use Reason::{Format, Homoglyph, LineBreak, TrailingSpace};
        assert_eq!(edits("\u{430}pple"), ("apple".into(), vec![(0, 2, Homoglyph, Some('a'))]));
        assert_eq!(
            edits("\u{430}\u{200B}ppl\u{435}"),
            ("apple".into(), vec![(0, 2, Homoglyph, Some('a')), (2, 5, Format, None), (8, 10, Homoglyph, Some('e'))])
        );
        assert_eq!(
            edits("\u{440}\u{430}\u{443}\u{440}\u{430}l.com"),
            (
                "paypal.com".into(),
                (0..5).map(|i| (2 * i, 2 * i + 2, Homoglyph, Some(['p', 'a', 'y', 'p', 'a'][i]))).collect()
            )
        );
        assert_eq!(
            edits("a \u{200B} \nb\r\u{200B}\nc\rd\u{2028}"),
            (
                "a\nb\nc\nd\n".into(),
                vec![
                    (1, 2, TrailingSpace, None),
                    (2, 5, Format, None),
                    (5, 6, TrailingSpace, None),
                    (8, 9, LineBreak, None),
                    (9, 12, Format, None),
                    (14, 15, LineBreak, Some('\n')),
                    (16, 19, LineBreak, Some('\n')),
                ]
            )
        );
        let report = sanitize_text_report("x \u{200B}\u{A0}".as_bytes()).expect("accepted");
        assert_eq!(report.total(TrailingSpace), Total { spans: 2, bytes: 3 });
        assert_eq!(report.total(Format), Total { spans: 1, bytes: 3 });
        assert_eq!(report.total(Homoglyph), Total::default());
    }

    fn refusal(input: &[u8]) -> Refusal {
        sanitize_text_report(input).expect_err("refused")
    }

    #[test]
    fn report_reasons() {
        use Reason::VariationSelector;
        use Reason::{BidiControl, Bom, DetachedMark, Format, JoinControl, Noncharacter, PrivateUse, Unassigned};
        assert_eq!(removals("\u{FEFF}a\u{FEFF}"), [(0, 3, Bom), (4, 7, Format)]);
        assert_eq!(removals("a\u{200B}\u{200B}\u{200D}b"), [(1, 7, Format), (7, 10, JoinControl)]);
        assert_eq!(
            removals("\u{202E}\u{E000}\u{FDD0}\u{378}"),
            [(0, 3, BidiControl), (3, 6, PrivateUse), (6, 9, Noncharacter), (9, 11, Unassigned)]
        );
        assert_eq!(removals("a\u{FE0F}\u{E0041}"), [(1, 4, VariationSelector), (4, 8, Format)]);
        assert_eq!(removals(" \u{301}\u{301}x"), [(1, 5, DetachedMark)]);
        assert_eq!(removals("\u{1F469}\u{200D}\u{1F4BB} \u{2764}\u{FE0F}"), []);
        assert_eq!(removals("plain text"), []);
    }

    #[test]
    fn report_refusals() {
        let at = |start, end, reason| Some(Fragment { start, end, reason, with: None });
        assert_eq!(refusal(b"ab\0\0c\0"), Refusal { code: Code::Malformed, at: at(2, 4, Reason::Control) });
        assert_eq!(refusal(b"a\xFF"), Refusal { code: Code::Malformed, at: at(1, 2, Reason::InvalidUtf8) });
        assert_eq!(refusal(b"a\xFF\0b"), Refusal { code: Code::Malformed, at: at(1, 3, Reason::InvalidUtf8) });
        let marks = format!("xA{}", "\u{301}".repeat(33));
        assert_eq!(refusal(marks.as_bytes()), Refusal { code: Code::Malformed, at: at(2, 68, Reason::MarkOverflow) });
        assert_eq!(sanitize_text_report_with(b"a", &|| true), Err(Refusal { code: Code::Cancelled, at: None }));
        let large = vec![b'a'; MAX_INPUT_BYTES + 1];
        assert_eq!(sanitize_text_report(&large), Err(Refusal { code: Code::TooLarge, at: None }));
    }

    #[test]
    fn report_caps_the_list_but_not_the_totals() {
        let n = MAX_EDITS + 5;
        let input = "a\u{200B}".repeat(n);
        let report = sanitize_text_report(input.as_bytes()).expect("accepted");
        assert_eq!(report.output, "a".repeat(n).into_bytes());
        assert_eq!(report.edits.len(), MAX_EDITS);
        assert!(report.truncated);
        assert_eq!(report.total(Reason::Format), Total { spans: n, bytes: 3 * n });
        assert_eq!(report.total(Reason::Control), Total::default());
        let last = report.edits[MAX_EDITS - 1];
        assert_eq!((last.start, last.end), (4 * MAX_EDITS - 3, 4 * MAX_EDITS));
        assert!(!sanitize_text_report(b"a\xE2\x80\x8B").expect("accepted").truncated);

        let input = "\u{430}pple ".repeat(n);
        let report = sanitize_text_report(input.as_bytes()).expect("accepted");
        assert_eq!(report.edits.len(), MAX_EDITS);
        assert!(report.truncated);
        assert_eq!(report.total(Reason::Homoglyph), Total { spans: n, bytes: 2 * n });
        assert_eq!(report.total(Reason::TrailingSpace), Total { spans: 1, bytes: 1 });
    }

    #[test]
    fn file_routing() {
        let text = |mime, ext, output: &str| Ok(FileOutcome::Text { mime, ext, output: output.into() });
        assert_eq!(sanitize_file("a.CSV", b"x,y\n"), text("text/csv", "csv", "x,y\n"));
        assert_eq!(sanitize_file("a.json", b"{}"), text("application/json", "json", "{}"));
        assert_eq!(sanitize_file("a.png", b"hello"), Err(Code::UnsupportedType));
        assert_eq!(sanitize_file("a.exe", b"hello"), Err(Code::UnsupportedType));
        assert_eq!(sanitize_file("a.txt", b""), Err(Code::UnsupportedType));
        assert_eq!(sanitize_file("a.txt", b"GIF89a"), Ok(FileOutcome::Binary { mime: "image/gif", ext: "gif" }));
        assert_eq!(sanitize_file("a.txt", b"<svg/>"), Ok(FileOutcome::Binary { mime: "image/svg+xml", ext: "svg" }));
    }
}
