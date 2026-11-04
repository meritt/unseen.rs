use std::path::PathBuf;

use unseen_core::{
    Code, FileOutcome, MAX_INPUT_BYTES, MAX_NORMALIZE_WORK, MAX_WORK, Reason, Total, UNICODE_TABLES_SHA256,
    UNICODE_VERSION, sanitize_text_report,
};
use unseen_test_support::json::{self, Value};
use unseen_test_support::{Sha256, hex, unhex};

const FORMAT: u64 = 1;

fn corpus(file: &str) -> Value {
    let dir = std::env::var_os("UNSEEN_CORPUS")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/textcorpus"), PathBuf::from);
    let path = dir.join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let value = json::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(value.get("format").as_u64(), Some(FORMAT), "{}: corpus format", path.display());
    value
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).as_str().unwrap_or_else(|| panic!("{key}: string expected"))
}

fn u64_of(v: &Value, key: &str) -> u64 {
    v.get(key).as_u64().unwrap_or_else(|| panic!("{key}: integer expected"))
}

fn usize_of(v: &Value, key: &str) -> usize {
    usize::try_from(u64_of(v, key)).expect("fits usize")
}

fn array_of<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).as_array().unwrap_or_else(|| panic!("{key}: array expected"))
}

fn input_of(case: &Value) -> Vec<u8> {
    let mut input = Vec::new();
    for segment in array_of(case, "input") {
        let unit = unhex(str_of(segment, "hex")).expect("segment hex");
        let repeat = usize_of(segment, "repeat");
        assert!(repeat >= 1, "segment repeat");
        input.extend(unit.repeat(repeat));
    }
    input
}

fn alphabet_of(random: &Value) -> Vec<Vec<u8>> {
    array_of(random, "alphabet").iter().map(|t| unhex(t.as_str().expect("token")).expect("token hex")).collect()
}

fn seed_of(random: &Value) -> u64 {
    let seed = str_of(random, "seed");
    u64::from_str_radix(seed.strip_prefix("0x").expect("hex seed"), 16).expect("seed")
}

fn check_output(name: &str, want: &Value, got: &[u8]) {
    let output = want.get("output");
    assert_eq!(usize_of(output, "len"), got.len(), "{name}: output length");
    assert_eq!(str_of(output, "sha256"), hex(&Sha256::digest(got)), "{name}: output digest");
    if let Some(expected) = output.get("hex").as_str() {
        assert_eq!(expected, hex(got), "{name}: output bytes");
    }
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn check_blocks(random: &Value, mut record: impl FnMut(&mut Sha256)) {
    let cases = usize_of(random, "cases");
    let block_size = usize_of(random, "block_size");
    let blocks = array_of(random, "blocks_sha256");
    assert_eq!(blocks.len() * block_size, cases, "random block layout");
    let mut h = Sha256::new();
    for i in 0..cases {
        record(&mut h);
        if (i + 1) % block_size == 0 {
            let block = i / block_size;
            assert_eq!(hex(&h.finish()), blocks[block].as_str().expect("block digest"), "random block {block}");
        }
    }
}

fn text_code(code: Option<Code>) -> (u8, &'static str) {
    match code {
        None => (0, "ok"),
        Some(Code::Malformed) => (1, "malformed"),
        Some(Code::TooLarge) => (2, "too_large"),
        Some(Code::VerifyFailed) => (3, "verify_failed"),
        Some(other) => panic!("{other} is outside the text corpus"),
    }
}

fn check_report(name: &str, input: &[u8], want: &Result<Vec<u8>, Code>) {
    let report = sanitize_text_report(input);
    let report = match (want, report) {
        (Ok(output), Ok(report)) => {
            assert_eq!(&report.output, output, "{name}: report output");
            report
        }
        (Err(code), Err(refusal)) => {
            assert_eq!(refusal.code, *code, "{name}: report code");
            if let Some(at) = refusal.at {
                assert_eq!(*code, Code::Malformed, "{name}: located {code}");
                assert!(at.start < at.end && at.end <= input.len(), "{name}: refusal bounds");
                assert!(!Reason::EDIT.contains(&at.reason), "{name}: refusal reason");
            }
            return;
        }
        (want, got) => panic!("{name}: report {got:?}, sanitize {want:?}"),
    };
    let mut rebuilt = Vec::with_capacity(report.output.len());
    let mut cursor = 0;
    let mut listed = [Total::default(); Reason::EDIT.len()];
    for f in &report.edits {
        assert!(cursor <= f.start && f.start < f.end && f.end <= input.len(), "{name}: fragment order");
        let reason = Reason::EDIT.iter().position(|&r| r == f.reason).unwrap_or_else(|| panic!("{name}: edit reason"));
        match f.reason {
            Reason::Homoglyph => assert!(f.with.is_some(), "{name}: homoglyph without replacement"),
            Reason::LineBreak => assert!(f.with.is_none_or(|c| c == '\n'), "{name}: line break replacement"),
            _ => assert!(f.with.is_none(), "{name}: removal with replacement"),
        }
        let boundary = |at: usize| input.get(at).is_none_or(|&b| b & 0xC0 != 0x80);
        assert!(boundary(f.start) && boundary(f.end), "{name}: fragment splits a scalar");
        rebuilt.extend_from_slice(&input[cursor..f.start]);
        if let Some(c) = f.with {
            rebuilt.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
        }
        cursor = f.end;
        listed[reason].spans += 1;
        listed[reason].bytes += f.end - f.start;
    }
    rebuilt.extend_from_slice(&input[cursor..]);
    assert_eq!(report.truncated, rebuilt != report.output, "{name}: input with edits applied");
    if !report.truncated {
        for (i, &reason) in Reason::EDIT.iter().enumerate() {
            assert_eq!(report.total(reason), listed[i], "{name}: {} total", reason.as_str());
        }
    }
}

fn write_text_record(h: &mut Sha256, input: &[u8]) {
    let trace = unseen_core::trace(input);
    check_report("record", input, &trace.result);
    let (code, _) = text_code(trace.result.as_ref().err().copied());
    h.update(&[code]);
    h.update(&trace.work.to_be_bytes());
    h.update(&trace.verify_work.to_be_bytes());
    if let Ok(output) = &trace.result {
        h.update(&u32::try_from(output.len()).expect("fits u32").to_be_bytes());
        h.update(output);
    }
}

#[test]
fn text_corpus_header() {
    let corpus = corpus("text.json");
    assert_eq!(str_of(&corpus, "unicode_version"), UNICODE_VERSION);
    assert_eq!(str_of(&corpus, "unicode_tables_sha256"), UNICODE_TABLES_SHA256);
    assert_eq!(usize_of(&corpus, "max_input_bytes"), MAX_INPUT_BYTES);
    assert_eq!(u64_of(&corpus, "max_work"), MAX_WORK);
    assert_eq!(u64_of(&corpus, "max_normalize_work"), MAX_NORMALIZE_WORK);
}

#[test]
fn text_corpus_every_scalar() {
    let corpus = corpus("text.json");
    let mut h = Sha256::new();
    let mut buf = [0u8; 4];
    for c in (0..=0x10_FFFF).filter_map(char::from_u32) {
        write_text_record(&mut h, c.encode_utf8(&mut buf).as_bytes());
    }
    assert_eq!(hex(&h.finish()), str_of(&corpus, "scalars_sha256"));
}

#[test]
fn text_corpus_random() {
    let corpus = corpus("text.json");
    let random = corpus.get("random");
    let alphabet = alphabet_of(random);
    let max_tokens = usize_of(random, "max_tokens");
    let mut rng = SplitMix64(seed_of(random));
    check_blocks(random, |h| {
        let n = 1 + rng.below(max_tokens);
        let input: Vec<u8> = (0..n).flat_map(|_| alphabet[rng.below(alphabet.len())].iter().copied()).collect();
        write_text_record(h, &input);
    });
}

#[test]
fn text_corpus_cases() {
    let corpus = corpus("text.json");
    for case in array_of(&corpus, "cases") {
        let name = str_of(case, "name");
        let want = case.get("want");
        let input = input_of(case);
        let trace = unseen_core::trace(&input);
        check_report(name, &input, &trace.result);
        let (_, result) = text_code(trace.result.as_ref().err().copied());
        assert_eq!(result, str_of(want, "result"), "{name}: result");
        assert_eq!(trace.work, u64_of(want, "work"), "{name}: work");
        assert_eq!(trace.verify_work, u64_of(want, "verify_work"), "{name}: verify work");
        match &trace.result {
            Ok(output) => check_output(name, want, output),
            Err(_) => assert!(want.get("output").is_null(), "{name}: output recorded for a refusal"),
        }
    }
}

struct FileResult {
    code: u8,
    result: &'static str,
    mime: &'static str,
    ext: &'static str,
    output: Option<Vec<u8>>,
}

fn file_result(name: &str, body: &[u8]) -> FileResult {
    let refused = |code, result| FileResult { code, result, mime: "", ext: "", output: None };
    match unseen_core::sanitize_file(name, body) {
        Ok(FileOutcome::Text { mime, ext, output }) => {
            FileResult { code: 0, result: "ok", mime, ext, output: Some(output) }
        }
        Ok(FileOutcome::Binary { mime, ext }) => FileResult { code: 5, result: "binary", mime, ext, output: None },
        Err(Code::UnsupportedType) => refused(1, "unsupported_type"),
        Err(Code::TooLarge) => refused(2, "too_large"),
        Err(Code::Malformed) => refused(3, "malformed"),
        Err(Code::VerifyFailed) => refused(4, "verify_failed"),
        Err(Code::Cancelled) => panic!("cancelled without a cancel source"),
    }
}

fn write_file_record(h: &mut Sha256, r: &FileResult) {
    h.update(&[r.code, u8::try_from(r.mime.len()).expect("short mime")]);
    h.update(r.mime.as_bytes());
    h.update(&[u8::try_from(r.ext.len()).expect("short ext")]);
    h.update(r.ext.as_bytes());
    if let Some(output) = &r.output {
        h.update(&u32::try_from(output.len()).expect("fits u32").to_be_bytes());
        h.update(output);
    }
}

#[test]
fn file_corpus_random() {
    let corpus = corpus("files.json");
    let random = corpus.get("random");
    let alphabet = alphabet_of(random);
    let names: Vec<&str> = array_of(random, "file_names").iter().map(|n| n.as_str().expect("file name")).collect();
    let max_tokens = usize_of(random, "max_tokens");
    let mut rng = SplitMix64(seed_of(random));
    check_blocks(random, |h| {
        let name = names[rng.below(names.len())];
        let n = 1 + rng.below(max_tokens);
        let body: Vec<u8> = (0..n).flat_map(|_| alphabet[rng.below(alphabet.len())].iter().copied()).collect();
        write_file_record(h, &file_result(name, &body));
    });
}

#[test]
fn file_corpus_cases() {
    let corpus = corpus("files.json");
    for case in array_of(&corpus, "cases") {
        let name = str_of(case, "name");
        let want = case.get("want");
        let got = file_result(str_of(case, "file_name"), &input_of(case));
        assert_eq!(got.result, str_of(want, "result"), "{name}: result");
        assert_eq!(got.mime, want.get("mime").as_str().unwrap_or(""), "{name}: mime");
        assert_eq!(got.ext, want.get("ext").as_str().unwrap_or(""), "{name}: ext");
        match &got.output {
            Some(output) => check_output(name, want, output),
            None => assert!(want.get("output").is_null(), "{name}: output recorded for a refusal"),
        }
    }
}
