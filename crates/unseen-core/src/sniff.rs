use crate::utf8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Type {
    pub mime: &'static str,
    pub ext: &'static str,
}

const TEXT_PLAIN: Type = Type { mime: "text/plain", ext: "txt" };

const SNIFF_WINDOW: usize = 512;

const BINARY_EXTS: [&str; 19] = [
    "jpg", "jpeg", "png", "gif", "webp", "mp3", "flac", "wav", "m4a", "opus", "aac", "ogg", "mp4", "mov", "webm",
    "avif", "heic", "svg", "pdf",
];

pub(crate) const TEXT_SUBTYPES: [(&str, &str); 16] = [
    ("css", "text/plain"),
    ("csv", "text/csv"),
    ("html", "text/plain"),
    ("ini", "text/plain"),
    ("js", "text/plain"),
    ("json", "application/json"),
    ("json5", "text/plain"),
    ("markdown", "text/plain"),
    ("md", "text/plain"),
    ("mjs", "text/plain"),
    ("toml", "text/plain"),
    ("ts", "text/plain"),
    ("txt", "text/plain"),
    ("xml", "text/plain"),
    ("yaml", "text/plain"),
    ("yml", "text/plain"),
];

pub(crate) fn text_subtype(ext: &str) -> Option<&'static str> {
    TEXT_SUBTYPES.iter().find(|(e, _)| *e == ext).map(|(_, mime)| *mime)
}

pub(crate) fn gate_ext(name: &[u8]) -> Option<&'static str> {
    let dot = name.iter().rposition(|&b| b == b'.' || b == b'/')?;
    if name[dot] == b'/' {
        return None;
    }
    let mut suffix = &name[dot + 1..];
    let mut lower = Vec::with_capacity(suffix.len());
    while !suffix.is_empty() {
        let (r, size) = utf8::decode(suffix);
        suffix = &suffix[size..];
        // The only non-ASCII scalars whose simple lowercase mapping is ASCII.
        let b = match r {
            0x41..=0x5A => r as u8 + 32,
            0x00..=0x7F => r as u8,
            0x0130 => b'i',
            0x212A => b'k',
            _ => return None,
        };
        lower.push(b);
    }
    BINARY_EXTS.iter().copied().chain(TEXT_SUBTYPES.iter().map(|(e, _)| *e)).find(|e| e.as_bytes() == lower)
}

pub(crate) fn sniff_type(body: &[u8]) -> Option<Type> {
    let header = window(body, 0)?;
    if let Some(size) = id3v2_tag_size(header) {
        let behind = usize::try_from(size).ok().and_then(|off| window(body, off));
        if let Some(behind) = behind
            && let Some(typ) = detect(behind)
            && matches!(typ.mime, "audio/flac" | "audio/aac" | "audio/mpeg")
            && id3v2_tag_size(behind).is_none()
        {
            return Some(typ);
        }
    }
    detect(header)
}

fn window(body: &[u8], off: usize) -> Option<&[u8]> {
    if off >= body.len() {
        return None;
    }
    Some(&body[off..body.len().min(off + SNIFF_WINDOW)])
}

fn id3v2_tag_size(header: &[u8]) -> Option<u64> {
    if header.len() < 10 || &header[..3] != b"ID3" {
        return None;
    }
    let size = u64::from(header[6] & 0x7F) << 21
        | u64::from(header[7] & 0x7F) << 14
        | u64::from(header[8] & 0x7F) << 7
        | u64::from(header[9] & 0x7F);
    let footer = if header[5] & 0x10 != 0 { 10 } else { 0 };
    Some(10 + size + footer)
}

pub(crate) fn detect(h: &[u8]) -> Option<Type> {
    if h.len() >= 8 && &h[4..8] == b"ftyp" {
        match ftyp_kind(h) {
            Some(Kind::Unknown) | None => return None,
            Some(_) => {}
        }
    }
    let t = |mime, ext| Some(Type { mime, ext });
    if h.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return t("image/jpeg", "jpg");
    }
    if h.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return t("image/png", "png");
    }
    if h.starts_with(b"GIF87a") || h.starts_with(b"GIF89a") {
        return t("image/gif", "gif");
    }
    if riff_form(h, *b"WEBP") {
        return t("image/webp", "webp");
    }
    if riff_form(h, *b"WAVE") {
        return t("audio/wav", "wav");
    }
    if h.len() >= 2 && h[0] == 0xFF && h[1] & 0xF6 == 0xF0 {
        return t("audio/aac", "aac");
    }
    if h.starts_with(b"ID3") || h.len() >= 2 && h[0] == 0xFF && h[1] & 0xE0 == 0xE0 && h[1] & 0x06 != 0 {
        return t("audio/mpeg", "mp3");
    }
    if h.starts_with(b"fLaC") {
        return t("audio/flac", "flac");
    }
    for (kind, mime, ext) in [
        (Kind::Audio, "audio/mp4", "m4a"),
        (Kind::Avif, "image/avif", "avif"),
        (Kind::Heic, "image/heic", "heic"),
        (Kind::Heif, "image/heic", "heic"),
        (Kind::Video, "video/mp4", "mp4"),
        (Kind::QuickTime, "video/quicktime", "mov"),
    ] {
        if ftyp_kind(h) == Some(kind) {
            return t(mime, ext);
        }
    }
    if h.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) && contains(h, b"webm") {
        return t("video/webm", "webm");
    }
    if h.starts_with(b"OggS") && (contains(h, b"OpusHead") || contains(h, b"\x01vorbis")) {
        return t("audio/ogg", "ogg");
    }
    if h.starts_with(b"%PDF-") {
        return t("application/pdf", "pdf");
    }
    if svg_magic(h) {
        return t("image/svg+xml", "svg");
    }
    if looks_text(h) {
        return Some(TEXT_PLAIN);
    }
    None
}

fn riff_form(h: &[u8], form: [u8; 4]) -> bool {
    h.len() >= 12 && &h[..4] == b"RIFF" && h[8..12] == form
}

fn contains(h: &[u8], needle: &[u8]) -> bool {
    h.windows(needle.len()).any(|w| w == needle)
}

fn svg_magic(h: &[u8]) -> bool {
    let t = h.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(h);
    let start = t.iter().position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n')).unwrap_or(t.len());
    let t = &t[start..];
    t.first() == Some(&b'<') && contains(t, b"<svg")
}

fn looks_text(h: &[u8]) -> bool {
    !h.is_empty() && h.iter().all(|&b| b >= 0x20 || matches!(b, b'\t' | b'\n' | 0x0B | 0x0C | b'\r'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Unknown,
    Video,
    QuickTime,
    Audio,
    Heic,
    Avif,
    Heif,
    Vendor,
}

fn ftyp_kind(h: &[u8]) -> Option<Kind> {
    if h.len() < 16 || &h[4..8] != b"ftyp" {
        return None;
    }
    let mut size = u64::from(u32::from_be_bytes(h[..4].try_into().expect("4 bytes")));
    let mut header = 8u64;
    if size == 1 {
        if h.len() < 24 {
            return None;
        }
        size = u64::from_be_bytes(h[8..16].try_into().expect("8 bytes"));
        header = 16;
    }
    if size == 0 {
        size = h.len() as u64;
    }
    if size < header + 8 || !(size - header).is_multiple_of(4) {
        return None;
    }
    let mut end = size.min(h.len() as u64);
    end -= (end - header) % 4;
    classify_brands(&h[header as usize..end as usize])
}

fn brand_kind(brand: &[u8]) -> Kind {
    match brand {
        b"M4A " => Kind::Audio,
        b"avc1" | b"dash" | b"iso2" | b"iso3" | b"iso4" | b"iso5" | b"iso6" | b"isom" | b"mp41" | b"mp42" => {
            Kind::Video
        }
        b"qt  " => Kind::QuickTime,
        b"heic" | b"heim" | b"heis" | b"heix" | b"hevc" | b"hevx" => Kind::Heic,
        b"avif" | b"avis" => Kind::Avif,
        b"heif" | b"mif1" | b"msf1" => Kind::Heif,
        b"ARRI" | b"CAEP" | b"LCAG" | b"MSNV" | b"XAVC" | b"XFVC" | b"caqv" | b"niko" | b"pana" => Kind::Vendor,
        _ => Kind::Unknown,
    }
}

fn classify_brands(p: &[u8]) -> Option<Kind> {
    let (brands, []) = p.as_chunks::<4>() else {
        return None;
    };
    if brands.len() < 2 {
        return None;
    }
    let [mut video, mut qt, mut audio, mut heic, mut avif, mut image, mut camera] = [false; 7];
    for (i, brand) in brands.iter().enumerate() {
        if i == 1 {
            continue;
        }
        match brand_kind(brand) {
            Kind::Video => video = true,
            Kind::QuickTime => qt = true,
            Kind::Audio => audio = true,
            Kind::Heic => heic = true,
            Kind::Avif => avif = true,
            Kind::Heif => image = true,
            Kind::Vendor => camera = true,
            Kind::Unknown => {}
        }
    }
    if heic && avif || (heic || avif || image) && (video || qt || audio || camera) {
        return None;
    }
    if audio && &p[..4] == b"M4A " {
        return Some(Kind::Audio);
    }
    Some(match () {
        () if avif => Kind::Avif,
        () if heic => Kind::Heic,
        () if image => Kind::Heif,
        () if video => Kind::Video,
        () if qt => Kind::QuickTime,
        () if camera => Kind::Video,
        () if audio => return None,
        () => Kind::Unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_follows_go_clean_ext() {
        let ext = |n: &str| gate_ext(n.as_bytes());
        assert_eq!(ext("note.TXT"), Some("txt"));
        assert_eq!(ext("a.tar.json"), Some("json"));
        assert_eq!(ext(".txt"), Some("txt"));
        assert_eq!(ext("a.\u{130}N\u{130}"), Some("ini"));
        assert_eq!(ext("a.MAR\u{212A}DOWN"), Some("markdown"));
        assert_eq!(ext("a.b/c"), None);
        assert_eq!(ext("a.txt/"), None);
        assert_eq!(ext("README"), None);
        assert_eq!(ext("a."), None);
        assert_eq!(ext("a.exe"), None);
        assert_eq!(gate_ext(b"\xff.txt"), Some("txt"));
        assert_eq!(gate_ext(b"a.t\xffxt"), None);
        assert_eq!(ext("a.PNG"), Some("png"));
    }

    #[test]
    fn text_subtypes_are_sorted_and_disjoint_from_binary() {
        assert!(TEXT_SUBTYPES.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(TEXT_SUBTYPES.iter().all(|(e, _)| !BINARY_EXTS.contains(e)));
    }

    #[test]
    fn ftyp_decisions() {
        assert_eq!(detect(b"    ftypisom    mp42text").map(|t| t.mime), Some("video/mp4"));
        assert_eq!(detect(b"    ftypzzzz    text"), None);
        assert_eq!(detect(b"    ftypheic    isom"), None);
        assert_eq!(detect(b"\0\0\0\x18ftypM4A \0\0\0\0M4A isom").map(|t| t.mime), Some("audio/mp4"));
        assert_eq!(detect(b"\0\0\0\x14ftypqt  \0\0\0\0CAEP").map(|t| t.mime), Some("video/quicktime"));
        assert_eq!(detect(b"\0\0\0\x10ftypcaqv\0\0\0\0").map(|t| t.mime), Some("video/mp4"));
    }
}
