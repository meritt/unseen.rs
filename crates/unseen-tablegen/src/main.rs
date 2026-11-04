#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use unseen_test_support::{Sha256, hex};

const UNICODE_VERSION: &str = "17.0.0";
const MAX_CODE_POINT: u32 = 0x10_FFFF;
const MAX_SOURCE_SIZE: usize = 8 << 20;

struct Source {
    name: &'static str,
    url: &'static str,
    sha256: &'static str,
    marker: &'static str,
}

const SOURCES: [Source; 12] = [
    Source {
        name: "DerivedCoreProperties.txt",
        url: "https://www.unicode.org/Public/17.0.0/ucd/DerivedCoreProperties.txt",
        sha256: "24c7fed1195c482faaefd5c1e7eb821c5ee1fb6de07ecdbaa64b56a99da22c08",
        marker: "# DerivedCoreProperties-17.0.0.txt",
    },
    Source {
        name: "DerivedGeneralCategory.txt",
        url: "https://www.unicode.org/Public/17.0.0/ucd/extracted/DerivedGeneralCategory.txt",
        sha256: "d62e5bab70ca74f099343f71224fa051cb1fdd61a1ab45c0488c44cfc0b6102e",
        marker: "# DerivedGeneralCategory-17.0.0.txt",
    },
    Source {
        name: "GraphemeBreakProperty.txt",
        url: "https://www.unicode.org/Public/17.0.0/ucd/auxiliary/GraphemeBreakProperty.txt",
        sha256: "d6b51d1d2ae5c33b451b7ed994b48f1f4dc62b2272a5831e7fd418514a6bae89",
        marker: "# GraphemeBreakProperty-17.0.0.txt",
    },
    Source {
        name: "PropList.txt",
        url: "https://www.unicode.org/Public/17.0.0/ucd/PropList.txt",
        sha256: "130dcddcaadaf071008bdfce1e7743e04fdfbc910886f017d9f9ac931d8c64dd",
        marker: "# PropList-17.0.0.txt",
    },
    Source {
        name: "emoji-data.txt",
        url: "https://www.unicode.org/Public/17.0.0/ucd/emoji/emoji-data.txt",
        sha256: "2cb2bb9455cda83e8481541ecf5b6dfda66a3bb89efa3fa7c5297eccf607b72b",
        marker: "# Version: 17.0",
    },
    Source {
        name: "emoji-sequences.txt",
        url: "https://www.unicode.org/Public/17.0.0/emoji/emoji-sequences.txt",
        sha256: "12cc8267dc33cbd11ed32bcf6fc5dc2ad9c7a77bae1bdfba2f41b1b9b3ead8dd",
        marker: "# Version: 17.0",
    },
    Source {
        name: "emoji-variation-sequences.txt",
        url: "https://www.unicode.org/Public/17.0.0/ucd/emoji/emoji-variation-sequences.txt",
        sha256: "bb3d09ef03f206012c7532dd52dc0a21c9efddba0135ea4cf0d9201b8b9bba7e",
        marker: "# Version: 17.0",
    },
    Source {
        name: "emoji-zwj-sequences.txt",
        url: "https://www.unicode.org/Public/17.0.0/emoji/emoji-zwj-sequences.txt",
        sha256: "5b25441daed2322b068c5e70cda522946a4f0274df864445a1965a92e5fc5cad",
        marker: "# Version: 17.0",
    },
    Source {
        name: "Scripts.txt",
        url: "https://www.unicode.org/Public/17.0.0/ucd/Scripts.txt",
        sha256: "9f5e50d3abaee7d6ce09480f325c706f485ae3240912527e651954d2d6b035bf",
        marker: "# Scripts-17.0.0.txt",
    },
    // UnicodeData.txt has no version header; Tolong Siki is new in 17.0.
    Source {
        name: "UnicodeData.txt",
        url: "https://www.unicode.org/Public/17.0.0/ucd/UnicodeData.txt",
        sha256: "2e1efc1dcb59c575eedf5ccae60f95229f706ee6d031835247d843c11d96470c",
        marker: "11DB0;TOLONG SIKI LETTER I;Lo;",
    },
    Source {
        name: "confusables.txt",
        url: "https://www.unicode.org/Public/17.0.0/security/confusables.txt",
        sha256: "091c7f82fc39ef208faf8f94d29c244de99254675e09de163160c810d13ef22a",
        marker: "# Version: 17.0.0",
    },
    Source {
        name: "IdentifierType.txt",
        url: "https://www.unicode.org/Public/17.0.0/security/IdentifierType.txt",
        sha256: "924ac63faa97ed73420d6ac48d08279d90968c7da0502ab701e08bfbb9683c22",
        marker: "# Version: 17.0.0",
    },
];

const SCRIPT_VALUES: [&str; 5] = ["Latin", "Cyrillic", "Greek", "Armenian", "Cherokee"];
const TWIN_TYPES: [&str; 2] = ["Recommended", "Limited_Use"];

const CATEGORY_VALUES: [&str; 30] = [
    "Cc", "Cf", "Cn", "Co", "Cs", "Ll", "Lm", "Lo", "Lt", "Lu", "Mc", "Me", "Mn", "Nd", "Nl", "No", "Pc", "Pd", "Pe",
    "Pf", "Pi", "Po", "Ps", "Sc", "Sk", "Sm", "So", "Zl", "Zp", "Zs",
];
const GRAPHEME_VALUES: [&str; 14] = [
    "Other",
    "CR",
    "Control",
    "Extend",
    "L",
    "LF",
    "LV",
    "LVT",
    "Prepend",
    "Regional_Indicator",
    "SpacingMark",
    "T",
    "V",
    "ZWJ",
];
const FLAG_NAMES: [&str; 11] = [
    "WhiteSpace",
    "BidiControl",
    "DefaultIgnorable",
    "JoinControl",
    "VariationSelector",
    "Noncharacter",
    "Emoji",
    "EmojiPresentation",
    "EmojiModifier",
    "EmojiModifierBase",
    "ExtendedPictographic",
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Scalar {
    flags: u16,
    category: u8,
    grapheme: u8,
    script: u8,
}

struct CodeRange {
    lo: u32,
    hi: u32,
    value: Scalar,
}

struct Database {
    ranges: Vec<CodeRange>,
    variations: Vec<(u32, u32)>,
    rgi_sequences: Vec<(u32, u8)>,
    rgi_data: Vec<u32>,
    twins: Vec<(u32, u8, u32)>,
    fingerprint: String,
}

type Result<T> = std::result::Result<T, String>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("unseen-tablegen: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../unseen-core/src/unicode17_data.rs");
    let mut source_dir = None;
    let mut check = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source-dir" => source_dir = Some(PathBuf::from(args.next().ok_or("--source-dir needs a value")?)),
            "--out" => out = PathBuf::from(args.next().ok_or("--out needs a value")?),
            "--check" => check = true,
            "--urls" => {
                for src in &SOURCES {
                    println!("{} {}", src.name, src.url);
                }
                return Ok(());
            }
            _ => {
                return Err(format!(
                    "unknown argument {arg:?}\nusage: unseen-tablegen --source-dir DIR [--out FILE] [--check] | --urls"
                ));
            }
        }
    }
    let source_dir = source_dir.ok_or("--source-dir is required; `cargo xtask tables` downloads the files")?;
    let files = load_sources(&source_dir)?;
    let db = build_database(&files)?;
    let generated = render(&db);
    if check {
        let existing = std::fs::read(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        if existing != generated.as_bytes() {
            return Err(format!("{} is not reproducible from pinned sources", out.display()));
        }
        println!("{} matches the pinned sources (tables {})", out.display(), db.fingerprint);
        return Ok(());
    }
    std::fs::write(&out, generated).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote {} (tables {})", out.display(), db.fingerprint);
    Ok(())
}

fn load_sources(dir: &Path) -> Result<Vec<String>> {
    SOURCES
        .iter()
        .map(|src| {
            let path = dir.join(src.name);
            let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if data.len() > MAX_SOURCE_SIZE {
                return Err(format!("{} exceeds {MAX_SOURCE_SIZE} bytes", src.name));
            }
            let sum = hex(&Sha256::digest(&data));
            if sum != src.sha256 {
                return Err(format!("{}: SHA-256 {sum}, want {}", src.name, src.sha256));
            }
            let text = String::from_utf8(data).map_err(|_| format!("{} is not UTF-8", src.name))?;
            if !text.contains(src.marker) {
                return Err(format!("{}: missing version marker {:?}", src.name, src.marker));
            }
            Ok(text)
        })
        .collect()
}

fn file<'a>(files: &'a [String], name: &str) -> &'a str {
    let index = SOURCES.iter().position(|s| s.name == name).expect("pinned source");
    &files[index]
}

fn build_database(files: &[String]) -> Result<Database> {
    let mut values = vec![
        Scalar { grapheme: value_id("Other", &GRAPHEME_VALUES), ..Scalar::default() };
        MAX_CODE_POINT as usize + 1
    ];

    parse_required_property(
        file(files, "DerivedGeneralCategory.txt"),
        "General_Category",
        &CATEGORY_VALUES,
        &mut values,
        |s, id| {
            s.category = id;
        },
    )?;
    parse_default_property(
        file(files, "GraphemeBreakProperty.txt"),
        "Grapheme_Cluster_Break",
        &GRAPHEME_VALUES,
        &mut values,
        |s, id| {
            s.grapheme = id;
        },
    )?;
    parse_binary_properties(
        file(files, "PropList.txt"),
        &mut values,
        &[
            ("White_Space", 1 << 0),
            ("Bidi_Control", 1 << 1),
            ("Join_Control", 1 << 3),
            ("Variation_Selector", 1 << 4),
            ("Noncharacter_Code_Point", 1 << 5),
        ],
    )?;
    parse_core_properties(file(files, "DerivedCoreProperties.txt"), &mut values)?;
    parse_binary_properties(
        file(files, "emoji-data.txt"),
        &mut values,
        &[
            ("Emoji", 1 << 6),
            ("Emoji_Presentation", 1 << 7),
            ("Emoji_Modifier", 1 << 8),
            ("Emoji_Modifier_Base", 1 << 9),
            ("Extended_Pictographic", 1 << 10),
        ],
    )?;

    parse_scripts(file(files, "Scripts.txt"), &mut values)?;

    let variations = parse_variations(files)?;
    let (rgi_sequences, rgi_data) = parse_rgi(files)?;
    let twins = build_twins(files, &values)?;
    let mut db = Database {
        ranges: merge_ranges(&values),
        variations,
        rgi_sequences,
        rgi_data,
        twins,
        fingerprint: String::new(),
    };
    db.fingerprint = fingerprint(&db);
    Ok(db)
}

fn parse_required_property(
    data: &str,
    property: &str,
    allowed: &[&str],
    values: &mut [Scalar],
    assign: impl Fn(&mut Scalar, u8),
) -> Result<()> {
    let mut seen = vec![false; values.len()];
    parse_property_file(property, data, |line, lo, hi, value| {
        let id = value_id(value, allowed);
        if id == 0 {
            return Err(format!("line {line}: unknown {property} value {value:?}"));
        }
        assign_range(line, lo, hi, &mut seen, values, |s| assign(s, id))
    })?;
    if let Some(cp) = seen.iter().position(|ok| !ok) {
        return Err(format!("{property}: U+{cp:04X} is unclassified"));
    }
    Ok(())
}

fn parse_default_property(
    data: &str,
    property: &str,
    allowed: &[&str],
    values: &mut [Scalar],
    assign: impl Fn(&mut Scalar, u8),
) -> Result<()> {
    let mut seen = vec![false; values.len()];
    parse_property_file(property, data, |line, lo, hi, value| {
        let id = value_id(value, allowed);
        if id == 0 {
            return Err(format!("line {line}: unknown {property} value {value:?}"));
        }
        assign_range(line, lo, hi, &mut seen, values, |s| assign(s, id))
    })
}

fn parse_binary_properties(data: &str, values: &mut [Scalar], selected: &[(&str, u16)]) -> Result<()> {
    let mut seen: Vec<Vec<bool>> = selected.iter().map(|_| vec![false; values.len()]).collect();
    scan_data("binary property file", data, |line, fields| {
        if fields.len() != 2 {
            return Err(format!("line {line}: want 2 fields, got {}", fields.len()));
        }
        let (lo, hi) = parse_range(fields[0]).map_err(|e| format!("line {line}: {e}"))?;
        let Some(index) = selected.iter().position(|(name, _)| *name == fields[1]) else {
            return Ok(());
        };
        let bit = selected[index].1;
        assign_range(line, lo, hi, &mut seen[index], values, |s| s.flags |= bit)
    })
}

fn parse_core_properties(data: &str, values: &mut [Scalar]) -> Result<()> {
    let mut seen = vec![false; values.len()];
    scan_data("DerivedCoreProperties.txt", data, |line, fields| {
        if fields.len() < 2 || fields.len() > 3 {
            return Err(format!("line {line}: want 2 or 3 fields, got {}", fields.len()));
        }
        let (lo, hi) = parse_range(fields[0]).map_err(|e| format!("line {line}: {e}"))?;
        if fields[1] != "Default_Ignorable_Code_Point" {
            return Ok(());
        }
        if fields.len() != 2 {
            return Err(format!("line {line}: malformed Default_Ignorable_Code_Point"));
        }
        assign_range(line, lo, hi, &mut seen, values, |s| s.flags |= 1 << 2)
    })
}

fn parse_scripts(data: &str, values: &mut [Scalar]) -> Result<()> {
    let mut seen = vec![false; values.len()];
    parse_property_file("Script", data, |line, lo, hi, value| {
        let id = value_id(value, &SCRIPT_VALUES);
        if id == 0 {
            return Ok(());
        }
        assign_range(line, lo, hi, &mut seen, values, |s| s.script = id)
    })
}

struct Decompositions {
    canonical: HashMap<u32, Vec<u32>>,
    combining: HashMap<u32, u8>,
}

impl Decompositions {
    fn parse(data: &str) -> Result<Self> {
        let mut d = Decompositions { canonical: HashMap::new(), combining: HashMap::new() };
        scan_data("UnicodeData.txt", data, |line, fields| {
            let cp = parse_code_point(fields[0]).map_err(|e| format!("line {line}: {e}"))?;
            let class_field = fields.get(3).copied().unwrap_or("");
            let class: u8 = class_field.parse().map_err(|_| format!("line {line}: combining class {class_field:?}"))?;
            if class != 0 {
                d.combining.insert(cp, class);
            }
            let mapping = fields.get(5).copied().unwrap_or("");
            if mapping.is_empty() || mapping.starts_with('<') {
                return Ok(());
            }
            let points = parse_sequence(mapping, false).map_err(|e| format!("line {line}: {e}"))?;
            d.canonical.insert(cp, points);
            Ok(())
        })?;
        Ok(d)
    }

    fn class(&self, cp: u32) -> u8 {
        self.combining.get(&cp).copied().unwrap_or(0)
    }

    fn nfd(&self, points: &[u32]) -> Vec<u32> {
        fn expand(d: &Decompositions, cp: u32, out: &mut Vec<u32>) {
            match d.canonical.get(&cp) {
                Some(mapping) => mapping.iter().for_each(|&c| expand(d, c, out)),
                None => out.push(cp),
            }
        }
        let mut out = Vec::new();
        for &cp in points {
            expand(self, cp, &mut out);
        }
        let mut i = 0;
        while i < out.len() {
            if self.class(out[i]) == 0 {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < out.len() && self.class(out[j]) != 0 {
                j += 1;
            }
            out[i..j].sort_by_key(|&c| self.class(c));
            i = j;
        }
        out
    }
}

fn strip_bom(data: &str) -> &str {
    data.strip_prefix('\u{FEFF}').unwrap_or(data)
}

fn parse_confusables(data: &str) -> Result<HashMap<u32, Vec<u32>>> {
    let mut prototypes = HashMap::new();
    scan_data("confusables.txt", strip_bom(data), |line, fields| {
        if fields.len() != 3 {
            return Err(format!("line {line}: want 3 fields, got {}", fields.len()));
        }
        let from = parse_code_point(fields[0]).map_err(|e| format!("line {line}: {e}"))?;
        let to = parse_sequence(fields[1], false).map_err(|e| format!("line {line}: {e}"))?;
        if prototypes.insert(from, to).is_some() {
            return Err(format!("line {line}: duplicate source U+{from:04X}"));
        }
        Ok(())
    })?;
    Ok(prototypes)
}

fn parse_twin_types(data: &str) -> Result<Vec<bool>> {
    let mut allowed = vec![false; MAX_CODE_POINT as usize + 1];
    let mut seen = vec![false; MAX_CODE_POINT as usize + 1];
    parse_property_file("Identifier_Type", strip_bom(data), |line, lo, hi, value| {
        let ok = TWIN_TYPES.contains(&value);
        for cp in lo..=hi {
            let i = cp as usize;
            if seen[i] {
                return Err(format!("line {line}: overlapping assignment at U+{cp:04X}"));
            }
            seen[i] = true;
            allowed[i] = ok;
        }
        Ok(())
    })?;
    Ok(allowed)
}

fn skeleton(d: &Decompositions, prototypes: &HashMap<u32, Vec<u32>>, cp: u32) -> Vec<u32> {
    let mut mapped = Vec::new();
    for c in d.nfd(&[cp]) {
        match prototypes.get(&c) {
            Some(prototype) => mapped.extend_from_slice(prototype),
            None => mapped.push(c),
        }
    }
    d.nfd(&mapped)
}

fn build_twins(files: &[String], values: &[Scalar]) -> Result<Vec<(u32, u8, u32)>> {
    let d = Decompositions::parse(file(files, "UnicodeData.txt"))?;
    let prototypes = parse_confusables(file(files, "confusables.txt"))?;
    let allowed = parse_twin_types(file(files, "IdentifierType.txt"))?;
    let mut groups: HashMap<Vec<u32>, Vec<u32>> = HashMap::new();
    for cp in 0..=MAX_CODE_POINT {
        let v = values[cp as usize];
        if v.script == 0 || !allowed[cp as usize] || !is_letter(v.category) {
            continue;
        }
        groups.entry(skeleton(&d, &prototypes, cp)).or_default().push(cp);
    }
    let mut twins = Vec::new();
    for members in groups.values() {
        for &from in members {
            for script in 1..=u8::try_from(SCRIPT_VALUES.len()).expect("small enum") {
                if script == values[from as usize].script {
                    continue;
                }
                // Members ascend, so the first match is the lowest.
                if let Some(&to) = members.iter().find(|&&to| {
                    values[to as usize].script == script
                        && values[to as usize].category == values[from as usize].category
                }) {
                    twins.push((from, script, to));
                }
            }
        }
    }
    twins.sort_unstable();
    Ok(twins)
}

fn is_letter(category: u8) -> bool {
    category != 0 && CATEGORY_VALUES[usize::from(category) - 1].starts_with('L')
}

fn parse_variations(files: &[String]) -> Result<Vec<(u32, u32)>> {
    let name = "emoji-variation-sequences.txt";
    let mut seen = BTreeSet::new();
    let mut records = HashSet::new();
    scan_data(name, file(files, name), |line, fields| {
        if fields.len() < 2 {
            return Err(format!("line {line}: malformed variation record"));
        }
        let points = match parse_sequence(fields[0], false) {
            Ok(points) if points.len() == 2 => points,
            _ => return Err(format!("line {line}: variation sequence must contain exactly two scalars")),
        };
        let entry = (points[0], points[1]);
        if entry.1 != 0xFE0E && entry.1 != 0xFE0F {
            return Err(format!("line {line}: U+{:04X} is not an emoji presentation selector", entry.1));
        }
        if !records.insert(fields.join(";")) {
            return Err(format!("line {line}: duplicate variation record"));
        }
        if !seen.insert(entry) {
            return Err(format!("line {line}: duplicate variation sequence"));
        }
        Ok(())
    })?;
    Ok(seen.into_iter().collect())
}

type RgiTables = (Vec<(u32, u8)>, Vec<u32>);

fn parse_rgi(files: &[String]) -> Result<RgiTables> {
    let inputs: [(&str, &[&str]); 2] = [
        (
            "emoji-sequences.txt",
            &[
                "Basic_Emoji",
                "Emoji_Keycap_Sequence",
                "RGI_Emoji_Flag_Sequence",
                "RGI_Emoji_Tag_Sequence",
                "RGI_Emoji_Modifier_Sequence",
            ],
        ),
        ("emoji-zwj-sequences.txt", &["RGI_Emoji_ZWJ_Sequence"]),
    ];
    let mut list: Vec<Vec<u32>> = Vec::new();
    let mut seen = HashSet::new();
    for (name, allowed) in inputs {
        scan_data(name, file(files, name), |line, fields| {
            if fields.len() < 2 || !allowed.contains(&fields[1]) {
                return Err(format!("line {line}: unknown emoji sequence type {:?}", fields.get(1).unwrap_or(&"")));
            }
            if fields[0].contains("..") {
                if fields[1] != "Basic_Emoji" {
                    return Err(format!("line {line}: only Basic_Emoji may use a range"));
                }
                let (lo, hi) = parse_range(fields[0]).map_err(|e| format!("line {line}: {e}"))?;
                for cp in lo..=hi {
                    add_sequence(&mut list, &mut seen, vec![cp]).map_err(|e| format!("line {line}: {e}"))?;
                }
                return Ok(());
            }
            let points = parse_sequence(fields[0], false).map_err(|e| format!("line {line}: {e}"))?;
            add_sequence(&mut list, &mut seen, points).map_err(|e| format!("line {line}: {e}"))
        })?;
    }
    list.sort();
    let mut entries = Vec::with_capacity(list.len());
    let mut pool = Vec::new();
    for points in list {
        let length =
            u8::try_from(points.len()).map_err(|_| format!("RGI sequence length {} exceeds u8", points.len()))?;
        entries.push((u32::try_from(pool.len()).expect("pool fits u32"), length));
        pool.extend(points);
    }
    Ok((entries, pool))
}

fn add_sequence(list: &mut Vec<Vec<u32>>, seen: &mut HashSet<Vec<u32>>, points: Vec<u32>) -> Result<()> {
    if points.is_empty() {
        return Err("empty sequence".into());
    }
    if !seen.insert(points.clone()) {
        return Err("duplicate emoji sequence".into());
    }
    list.push(points);
    Ok(())
}

fn parse_property_file(
    name: &str,
    data: &str,
    mut visit: impl FnMut(usize, u32, u32, &str) -> Result<()>,
) -> Result<()> {
    scan_data(name, data, |line, fields| {
        if fields.len() != 2 {
            return Err(format!("line {line}: want 2 fields, got {}", fields.len()));
        }
        let (lo, hi) = parse_range(fields[0]).map_err(|e| format!("line {line}: {e}"))?;
        visit(line, lo, hi, fields[1])
    })
}

fn scan_data(name: &str, data: &str, mut visit: impl FnMut(usize, &[&str]) -> Result<()>) -> Result<()> {
    for (index, raw) in data.split_terminator('\n').enumerate() {
        let line = index + 1;
        let mut text = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(comment) = text.find('#') {
            text = &text[..comment];
        }
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let mut fields: Vec<&str> = text.split(';').map(str::trim).collect();
        while fields.last() == Some(&"") {
            fields.pop();
        }
        if fields.first().is_none_or(|f| f.is_empty()) {
            return Err(format!("{name}:{line}: empty data field"));
        }
        visit(line, &fields).map_err(|e| format!("{name}:{e}"))?;
    }
    Ok(())
}

fn parse_range(text: &str) -> Result<(u32, u32)> {
    let parts: Vec<&str> = text.split("..").collect();
    if parts.len() > 2 {
        return Err(format!("malformed range {text:?}"));
    }
    let lo = parse_code_point(parts[0])?;
    let hi = match parts.get(1) {
        Some(part) => parse_code_point(part)?,
        None => lo,
    };
    if lo > hi {
        return Err(format!("descending range {text:?}"));
    }
    Ok((lo, hi))
}

fn parse_sequence(text: &str, allow_range: bool) -> Result<Vec<u32>> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.is_empty() {
        return Err("empty code point sequence".into());
    }
    parts
        .into_iter()
        .map(|part| {
            if part.contains("..") && !allow_range {
                return Err(format!("range {part:?} is not a sequence scalar"));
            }
            let cp = parse_code_point(part)?;
            if (0xD800..=0xDFFF).contains(&cp) {
                return Err(format!("U+{cp:04X} is a surrogate"));
            }
            Ok(cp)
        })
        .collect()
}

fn parse_code_point(text: &str) -> Result<u32> {
    if text.is_empty() || text.len() > 6 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid code point {text:?}"));
    }
    match u32::from_str_radix(text, 16) {
        Ok(n) if n <= MAX_CODE_POINT => Ok(n),
        _ => Err(format!("invalid code point {text:?}")),
    }
}

fn assign_range(
    line: usize,
    lo: u32,
    hi: u32,
    seen: &mut [bool],
    values: &mut [Scalar],
    mut assign: impl FnMut(&mut Scalar),
) -> Result<()> {
    for cp in lo..=hi {
        let i = cp as usize;
        if seen[i] {
            return Err(format!("line {line}: overlapping assignment at U+{cp:04X}"));
        }
        seen[i] = true;
        assign(&mut values[i]);
    }
    Ok(())
}

fn value_id(value: &str, values: &[&str]) -> u8 {
    values.iter().position(|v| *v == value).map_or(0, |i| u8::try_from(i + 1).expect("small enum"))
}

fn merge_ranges(values: &[Scalar]) -> Vec<CodeRange> {
    let mut result: Vec<CodeRange> = Vec::new();
    for cp in (0..0xD800).chain(0xE000..=MAX_CODE_POINT) {
        let value = values[cp as usize];
        if let Some(last) = result.last_mut()
            && last.hi + 1 == cp
            && last.value == value
        {
            last.hi = cp;
            continue;
        }
        result.push(CodeRange { lo: cp, hi: cp, value });
    }
    result
}

fn fingerprint(db: &Database) -> String {
    let mut h = Sha256::new();
    for r in &db.ranges {
        h.update(&r.lo.to_be_bytes());
        h.update(&r.hi.to_be_bytes());
        h.update(&r.value.flags.to_be_bytes());
        h.update(&[r.value.category, r.value.grapheme, r.value.script]);
    }
    for (base, selector) in &db.variations {
        h.update(&base.to_be_bytes());
        h.update(&selector.to_be_bytes());
    }
    for (offset, length) in &db.rgi_sequences {
        h.update(&offset.to_be_bytes());
        h.update(&[*length]);
    }
    for cp in &db.rgi_data {
        h.update(&cp.to_be_bytes());
    }
    for (from, script, to) in &db.twins {
        h.update(&from.to_be_bytes());
        h.update(&[*script]);
        h.update(&to.to_be_bytes());
    }
    hex(&h.finish())
}

fn constant(value: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for c in value.chars() {
        if c == '_' || c == '-' || c == ' ' {
            out.push('_');
            previous_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && previous_lower {
            out.push('_');
        }
        previous_lower = c.is_ascii_lowercase();
        out.push(c.to_ascii_uppercase());
    }
    out
}

fn render(db: &Database) -> String {
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(
        w,
        "// @generated by unseen-tablegen from the pinned Unicode {UNICODE_VERSION} data files; do not edit."
    );
    let _ = writeln!(w, "//");
    for src in &SOURCES {
        let _ = writeln!(w, "// {} {}", src.sha256, src.url);
    }
    let _ = writeln!(w);
    let _ = writeln!(w, "use crate::unicode17::{{Props, Range, Sequence, Twin}};");
    let _ = writeln!(w);
    let _ = writeln!(w, "pub(crate) const GENERATED_VERSION: &str = {UNICODE_VERSION:?};");
    let _ = writeln!(w, "pub(crate) const TABLES_SHA256: &str = {:?};", db.fingerprint);
    let _ = writeln!(w);
    render_enum(w, "CATEGORY", &CATEGORY_VALUES);
    render_enum(w, "GRAPHEME", &GRAPHEME_VALUES);
    render_enum(w, "SCRIPT", &SCRIPT_VALUES);
    for (i, name) in FLAG_NAMES.iter().enumerate() {
        let _ = writeln!(w, "pub(crate) const FLAG_{}: u16 = 1 << {i};", constant(name));
    }
    let _ = writeln!(w);
    let _ = writeln!(w, "pub(crate) static RANGES: [Range; {}] = [", db.ranges.len());
    for r in &db.ranges {
        let v = r.value;
        let _ = writeln!(
            w,
            "    Range::new(0x{:X}, 0x{:X}, Props::new({}, {}, {}, {})),",
            r.lo, r.hi, v.flags, v.category, v.grapheme, v.script
        );
    }
    let _ = writeln!(w, "];");
    let _ = writeln!(w);
    let _ = writeln!(w, "pub(crate) static VARIATION_SEQUENCES: [(u32, u32); {}] = [", db.variations.len());
    for (base, selector) in &db.variations {
        let _ = writeln!(w, "    (0x{base:X}, 0x{selector:X}),");
    }
    let _ = writeln!(w, "];");
    let _ = writeln!(w);
    let _ = writeln!(w, "pub(crate) static RGI_SEQUENCES: [Sequence; {}] = [", db.rgi_sequences.len());
    for (offset, length) in &db.rgi_sequences {
        let _ = writeln!(w, "    Sequence::new({offset}, {length}),");
    }
    let _ = writeln!(w, "];");
    let _ = writeln!(w);
    let _ = writeln!(w, "pub(crate) static RGI_DATA: [u32; {}] = [", db.rgi_data.len());
    for chunk in db.rgi_data.chunks(12) {
        let line: Vec<String> = chunk.iter().map(|cp| format!("0x{cp:X},")).collect();
        let _ = writeln!(w, "    {}", line.join(" "));
    }
    let _ = writeln!(w, "];");
    let _ = writeln!(w);
    let _ = writeln!(w, "pub(crate) static TWINS: [Twin; {}] = [", db.twins.len());
    for (from, script, to) in &db.twins {
        let _ = writeln!(w, "    Twin::new(0x{from:X}, {script}, 0x{to:X}),");
    }
    let _ = writeln!(w, "];");
    out
}

fn render_enum(w: &mut String, prefix: &str, values: &[&str]) {
    let _ = writeln!(w, "pub(crate) const {prefix}_INVALID: u8 = 0;");
    for (i, value) in values.iter().enumerate() {
        let _ = writeln!(w, "pub(crate) const {prefix}_{}: u8 = {};", constant(value), i + 1);
    }
    let names: Vec<String> =
        std::iter::once("Invalid").chain(values.iter().copied()).map(|v| format!("{v:?}")).collect();
    let _ = writeln!(w, "pub(crate) static {prefix}_NAMES: [&str; {}] = [{}];", names.len(), names.join(", "));
    let _ = writeln!(w);
}
