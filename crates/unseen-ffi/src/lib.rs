#![expect(unsafe_code, reason = "the C ABI is raw pointers and exported symbols")]
#![expect(non_camel_case_types, reason = "type names follow the C header")]

use std::alloc::{Layout, alloc, dealloc};
use std::ffi::{CStr, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use unseen_core::{Code, FileOutcome, Fragment, Reason, Report};

pub const UNSEEN_OK: u32 = 0;
pub const UNSEEN_UNSUPPORTED_TYPE: u32 = 1;
pub const UNSEEN_TOO_LARGE: u32 = 2;
pub const UNSEEN_MALFORMED: u32 = 3;
pub const UNSEEN_VERIFY_FAILED: u32 = 4;
pub const UNSEEN_BINARY: u32 = 5;
pub const UNSEEN_CANCELLED: u32 = 6;
pub const UNSEEN_INVALID_ARGUMENT: u32 = 7;
pub const UNSEEN_INTERNAL: u32 = 8;

const REASONS: [Reason; 15] = [
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
    Reason::InvalidUtf8,
    Reason::Control,
    Reason::MarkOverflow,
];
pub const UNSEEN_REASON_NONE: u32 = 0;
pub const UNSEEN_EDIT_REASONS: usize = Reason::EDIT.len();

fn reason_code(reason: Reason) -> u32 {
    REASONS.iter().position(|&r| r == reason).map_or(UNSEEN_REASON_NONE, |i| i as u32 + 1)
}

pub type unseen_cancel_fn = Option<unsafe extern "C" fn(context: *mut c_void) -> bool>;

#[repr(C)]
#[derive(Debug)]
pub struct unseen_result {
    pub status: u32,
    pub mime: *const u8,
    pub mime_len: usize,
    pub ext: *const u8,
    pub ext_len: usize,
    pub output: *mut u8,
    pub output_len: usize,
}

impl unseen_result {
    const fn status(status: u32) -> Self {
        Self {
            status,
            mime: ptr::null(),
            mime_len: 0,
            ext: ptr::null(),
            ext_len: 0,
            output: ptr::null_mut(),
            output_len: 0,
        }
    }

    fn typed(status: u32, mime: &'static str, ext: &'static str) -> Self {
        Self {
            mime: mime.as_ptr(),
            mime_len: mime.len(),
            ext: ext.as_ptr(),
            ext_len: ext.len(),
            ..Self::status(status)
        }
    }

    fn with_output(mut self, output: Vec<u8>) -> Self {
        (self.output, self.output_len) = into_raw(output);
        self
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct unseen_fragment {
    pub start: usize,
    pub end: usize,
    pub reason: u32,
    pub with: u32,
}

impl unseen_fragment {
    const NONE: Self = Self { start: 0, end: 0, reason: UNSEEN_REASON_NONE, with: 0 };

    fn new(f: Fragment) -> Self {
        Self { start: f.start, end: f.end, reason: reason_code(f.reason), with: f.with.map_or(0, u32::from) }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct unseen_report {
    pub status: u32,
    pub output: *mut u8,
    pub output_len: usize,
    pub edits: *mut unseen_fragment,
    pub edits_len: usize,
    pub truncated: bool,
    pub spans: [usize; UNSEEN_EDIT_REASONS],
    pub bytes: [usize; UNSEEN_EDIT_REASONS],
    pub at: unseen_fragment,
}

impl unseen_report {
    const fn status(status: u32) -> Self {
        Self {
            status,
            output: ptr::null_mut(),
            output_len: 0,
            edits: ptr::null_mut(),
            edits_len: 0,
            truncated: false,
            spans: [0; UNSEEN_EDIT_REASONS],
            bytes: [0; UNSEEN_EDIT_REASONS],
            at: unseen_fragment::NONE,
        }
    }

    fn new(report: Report) -> Self {
        let mut out = Self::status(UNSEEN_OK);
        for (i, &reason) in Reason::EDIT.iter().enumerate() {
            let total = report.total(reason);
            (out.spans[i], out.bytes[i]) = (total.spans, total.bytes);
        }
        out.truncated = report.truncated;
        (out.output, out.output_len) = into_raw(report.output);
        (out.edits, out.edits_len) = into_raw(report.edits.into_iter().map(unseen_fragment::new).collect());
        out
    }
}

// The offsets packages/npm/unseen.js reads on wasm32.
#[cfg(target_pointer_width = "32")]
const _: () = assert!(
    size_of::<unseen_result>() == 28
        && size_of::<unseen_fragment>() == 16
        && size_of::<unseen_report>() == 136
        && std::mem::offset_of!(unseen_report, truncated) == 20
        && std::mem::offset_of!(unseen_report, spans) == 24
        && std::mem::offset_of!(unseen_report, bytes) == 72
        && std::mem::offset_of!(unseen_report, at) == 120
);

fn into_raw<T>(v: Vec<T>) -> (*mut T, usize) {
    if v.is_empty() {
        return (ptr::null_mut(), 0);
    }
    let len = v.len();
    (Box::into_raw(v.into_boxed_slice()).cast::<T>(), len)
}

/// # Safety
///
/// `p` is null or came from [`into_raw`] with this `len`, and is not used afterwards.
unsafe fn free_raw<T>(p: *mut T, len: usize) {
    if !p.is_null() {
        // SAFETY: `p` came from `Box::into_raw` of a slice of this length.
        drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(p, len)) });
    }
}

trait Out {
    fn status(&self) -> u32;
    fn internal() -> Self;
}

impl Out for unseen_result {
    fn status(&self) -> u32 {
        self.status
    }

    fn internal() -> Self {
        Self::status(UNSEEN_INTERNAL)
    }
}

impl Out for unseen_report {
    fn status(&self) -> u32 {
        self.status
    }

    fn internal() -> Self {
        Self::status(UNSEEN_INTERNAL)
    }
}

const fn code_status(code: Code) -> u32 {
    match code {
        Code::UnsupportedType => UNSEEN_UNSUPPORTED_TYPE,
        Code::TooLarge => UNSEEN_TOO_LARGE,
        Code::Malformed => UNSEEN_MALFORMED,
        Code::VerifyFailed => UNSEEN_VERIFY_FAILED,
        Code::Cancelled => UNSEEN_CANCELLED,
    }
}

/// # Safety
///
/// `p` is null with `len == 0`, or valid for `len` bytes for `'a`.
unsafe fn bytes<'a>(p: *const u8, len: usize) -> Option<&'a [u8]> {
    match (p.is_null(), len) {
        (true, 0) => Some(&[]),
        (true, _) => None,
        // SAFETY: the caller guarantees `p` is valid for `len` bytes.
        (false, _) => Some(unsafe { std::slice::from_raw_parts(p, len) }),
    }
}

fn run<T: Out>(out: *mut T, f: impl FnOnce() -> T) -> u32 {
    if out.is_null() {
        return UNSEEN_INVALID_ARGUMENT;
    }
    let result = catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| T::internal());
    let status = result.status();
    // SAFETY: `out` is non-null and the caller guarantees it is writable.
    unsafe { out.write(result) };
    status
}

/// # Safety
///
/// `input` is null with `input_len == 0` or valid for `input_len` bytes; `out`
/// is writable; `cancel`, when set, is safe to call with `context` from this
/// thread for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unseen_sanitize_text(
    input: *const u8,
    input_len: usize,
    cancel: unseen_cancel_fn,
    context: *mut c_void,
    out: *mut unseen_result,
) -> u32 {
    run(out, || {
        // SAFETY: forwarded from this function's contract.
        let Some(input) = (unsafe { bytes(input, input_len) }) else {
            return unseen_result::status(UNSEEN_INVALID_ARGUMENT);
        };
        // SAFETY: forwarded from this function's contract.
        let poll = || cancel.is_some_and(|f| unsafe { f(context) });
        match unseen_core::sanitize_text_with(input, &poll) {
            Ok(output) => unseen_result::status(UNSEEN_OK).with_output(output),
            Err(code) => unseen_result::status(code_status(code)),
        }
    })
}

/// # Safety
///
/// As [`unseen_sanitize_text`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unseen_sanitize_text_report(
    input: *const u8,
    input_len: usize,
    cancel: unseen_cancel_fn,
    context: *mut c_void,
    out: *mut unseen_report,
) -> u32 {
    run(out, || {
        // SAFETY: forwarded from this function's contract.
        let Some(input) = (unsafe { bytes(input, input_len) }) else {
            return unseen_report::status(UNSEEN_INVALID_ARGUMENT);
        };
        // SAFETY: forwarded from this function's contract.
        let poll = || cancel.is_some_and(|f| unsafe { f(context) });
        match unseen_core::sanitize_text_report_with(input, &poll) {
            Ok(report) => unseen_report::new(report),
            Err(refusal) => unseen_report {
                at: refusal.at.map_or(unseen_fragment::NONE, unseen_fragment::new),
                ..unseen_report::status(code_status(refusal.code))
            },
        }
    })
}

/// # Safety
///
/// As [`unseen_sanitize_text`], with `name` held to the same rule as `input`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unseen_sanitize_file(
    name: *const u8,
    name_len: usize,
    body: *const u8,
    body_len: usize,
    cancel: unseen_cancel_fn,
    context: *mut c_void,
    out: *mut unseen_result,
) -> u32 {
    run(out, || {
        // SAFETY: forwarded from this function's contract.
        let (Some(name), Some(body)) = (unsafe { bytes(name, name_len) }, unsafe { bytes(body, body_len) }) else {
            return unseen_result::status(UNSEEN_INVALID_ARGUMENT);
        };
        // SAFETY: forwarded from this function's contract.
        let poll = || cancel.is_some_and(|f| unsafe { f(context) });
        match unseen_core::sanitize_file_with(name, body, &poll) {
            Ok(FileOutcome::Text { mime, ext, output }) => {
                unseen_result::typed(UNSEEN_OK, mime, ext).with_output(output)
            }
            Ok(FileOutcome::Binary { mime, ext }) => unseen_result::typed(UNSEEN_BINARY, mime, ext),
            Err(code) => unseen_result::status(code_status(code)),
        }
    })
}

/// # Safety
///
/// `result` is null or points to a result filled by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unseen_result_free(result: *mut unseen_result) {
    // SAFETY: the caller guarantees `result` is null or valid.
    let Some(result) = (unsafe { result.as_mut() }) else { return };
    // SAFETY: the library set `output` with `into_raw`, or left it null.
    unsafe { free_raw(result.output, result.output_len) };
    *result = unseen_result::status(result.status);
}

/// # Safety
///
/// `report` is null or points to a report filled by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unseen_report_free(report: *mut unseen_report) {
    // SAFETY: the caller guarantees `report` is null or valid.
    let Some(report) = (unsafe { report.as_mut() }) else { return };
    // SAFETY: the library set both buffers with `into_raw`, or left them null.
    unsafe {
        free_raw(report.output, report.output_len);
        free_raw(report.edits, report.edits_len);
    }
    (report.output, report.output_len) = (ptr::null_mut(), 0);
    (report.edits, report.edits_len) = (ptr::null_mut(), 0);
}

#[unsafe(no_mangle)]
pub extern "C" fn unseen_reason_name(reason: u32) -> *const c_char {
    let name = match reason.checked_sub(1).and_then(|i| REASONS.get(i as usize)) {
        Some(Reason::Bom) => c"bom",
        Some(Reason::Noncharacter) => c"noncharacter",
        Some(Reason::Unassigned) => c"unassigned",
        Some(Reason::PrivateUse) => c"private_use",
        Some(Reason::BidiControl) => c"bidi_control",
        Some(Reason::JoinControl) => c"join_control",
        Some(Reason::VariationSelector) => c"variation_selector",
        Some(Reason::Format) => c"format",
        Some(Reason::DetachedMark) => c"detached_mark",
        Some(Reason::TrailingSpace) => c"trailing_space",
        Some(Reason::Homoglyph) => c"homoglyph",
        Some(Reason::LineBreak) => c"line_break",
        Some(Reason::InvalidUtf8) => c"invalid_utf8",
        Some(Reason::Control) => c"control",
        Some(Reason::MarkOverflow) => c"mark_overflow",
        None if reason == UNSEEN_REASON_NONE => c"none",
        None => c"unknown",
    };
    name.as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn unseen_status_name(status: u32) -> *const c_char {
    match status {
        UNSEEN_OK => c"ok",
        UNSEEN_UNSUPPORTED_TYPE => c"unsupported_type",
        UNSEEN_TOO_LARGE => c"too_large",
        UNSEEN_MALFORMED => c"malformed",
        UNSEEN_VERIFY_FAILED => c"verify_failed",
        UNSEEN_BINARY => c"binary",
        UNSEEN_CANCELLED => c"cancelled",
        UNSEEN_INVALID_ARGUMENT => c"invalid_argument",
        UNSEEN_INTERNAL => c"internal",
        _ => c"unknown",
    }
    .as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn unseen_unicode_version() -> *const c_char {
    c"17.0.0".as_ptr()
}

const VERSION: &CStr = match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
    Ok(version) => version,
    Err(_) => panic!("the package version holds a NUL"),
};

#[unsafe(no_mangle)]
pub extern "C" fn unseen_version() -> *const c_char {
    VERSION.as_ptr()
}

const ALLOC_ALIGN: usize = 8;

#[unsafe(no_mangle)]
pub extern "C" fn unseen_alloc(len: usize) -> *mut u8 {
    match Layout::from_size_align(len, ALLOC_ALIGN) {
        // SAFETY: the layout has a non-zero size.
        Ok(layout) if len > 0 => unsafe { alloc(layout) },
        _ => ptr::null_mut(),
    }
}

/// # Safety
///
/// `p` is null or came from `unseen_alloc(len)` and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unseen_dealloc(p: *mut u8, len: usize) {
    if p.is_null() {
        return;
    }
    // SAFETY: `unseen_alloc` accepted this layout for the same `len`.
    unsafe { dealloc(p, Layout::from_size_align_unchecked(len, ALLOC_ALIGN)) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    fn file(name: &str, body: &[u8]) -> (u32, String, String, Vec<u8>) {
        let mut out = unseen_result::status(u32::MAX);
        // SAFETY: valid slices and a writable result.
        let status = unsafe {
            unseen_sanitize_file(
                name.as_ptr(),
                name.len(),
                body.as_ptr(),
                body.len(),
                None,
                ptr::null_mut(),
                &raw mut out,
            )
        };
        assert_eq!(status, out.status);
        // SAFETY: the library set these pointers from static strings and an owned buffer.
        let (mime, ext, output) = unsafe {
            (
                String::from_utf8(bytes(out.mime, out.mime_len).unwrap().to_vec()).unwrap(),
                String::from_utf8(bytes(out.ext, out.ext_len).unwrap().to_vec()).unwrap(),
                bytes(out.output, out.output_len).unwrap().to_vec(),
            )
        };
        // SAFETY: filled above; freed twice to prove idempotence.
        unsafe {
            unseen_result_free(&raw mut out);
            unseen_result_free(&raw mut out);
        }
        assert!(out.output.is_null());
        (status, mime, ext, output)
    }

    #[test]
    fn file_statuses() {
        assert_eq!(
            file("a.md", "a\u{200B}b".as_bytes()),
            (UNSEEN_OK, "text/plain".into(), "md".into(), b"ab".to_vec())
        );
        assert_eq!(file("a.txt", b"\xEF\xBB\xBF"), (UNSEEN_OK, "text/plain".into(), "txt".into(), vec![]));
        assert_eq!(file("a.txt", b"GIF89a"), (UNSEEN_BINARY, "image/gif".into(), "gif".into(), vec![]));
        assert_eq!(file("a.exe", b"x").0, UNSEEN_UNSUPPORTED_TYPE);
        assert_eq!(file("a.txt", b"a\0b").0, UNSEEN_UNSUPPORTED_TYPE);
        assert_eq!(file("a.txt", b"a\xFFb").0, UNSEEN_MALFORMED);
    }

    fn report(input: &[u8]) -> (unseen_report, Vec<u8>, Vec<unseen_fragment>) {
        let mut out = unseen_report::status(u32::MAX);
        // SAFETY: a valid slice and a writable report.
        let status =
            unsafe { unseen_sanitize_text_report(input.as_ptr(), input.len(), None, ptr::null_mut(), &raw mut out) };
        assert_eq!(status, out.status);
        let removals = if out.edits.is_null() {
            vec![]
        } else {
            // SAFETY: the library set this buffer of this length.
            unsafe { std::slice::from_raw_parts(out.edits, out.edits_len) }.to_vec()
        };
        // SAFETY: the library set this buffer, or left it null with length 0.
        let output = unsafe { bytes(out.output, out.output_len) }.unwrap().to_vec();
        // SAFETY: filled above; freed twice to prove idempotence.
        unsafe {
            unseen_report_free(&raw mut out);
            unseen_report_free(&raw mut out);
        }
        assert!(out.output.is_null() && out.edits.is_null());
        (out, output, removals)
    }

    #[test]
    fn report_statuses() {
        let fragment = |start, end, reason| unseen_fragment { start, end, reason: reason_code(reason), with: 0 };
        let (out, output, edits) = report("\u{FEFF}a\u{200B}b\u{202E}".as_bytes());
        assert_eq!((out.status, output), (UNSEEN_OK, b"ab".to_vec()));
        assert_eq!(
            edits,
            [fragment(0, 3, Reason::Bom), fragment(4, 7, Reason::Format), fragment(8, 11, Reason::BidiControl)]
        );
        let format = reason_code(Reason::Format) as usize - 1;
        assert_eq!((out.spans[format], out.bytes[format], out.truncated), (1, 3, false));
        assert_eq!(out.at, unseen_fragment::NONE);

        let (out, output, edits) = report("\u{430}bc \r\n".as_bytes());
        assert_eq!((out.status, output), (UNSEEN_OK, b"abc\n".to_vec()));
        assert_eq!(
            edits,
            [
                unseen_fragment { with: u32::from('a'), ..fragment(0, 2, Reason::Homoglyph) },
                fragment(4, 5, Reason::TrailingSpace),
                fragment(5, 6, Reason::LineBreak),
            ]
        );

        let (out, output, edits) = report(b"a\xE2\x80\x8Bb\0");
        assert_eq!((out.status, output, edits), (UNSEEN_MALFORMED, vec![], vec![]));
        assert_eq!((out.at, out.spans), (fragment(5, 6, Reason::Control), [0; UNSEEN_EDIT_REASONS]));

        assert_eq!(report(b"").0.status, UNSEEN_OK);
        let mut out = unseen_report::status(u32::MAX);
        // SAFETY: null with a non-zero length is rejected before any read.
        let status = unsafe { unseen_sanitize_text_report(ptr::null(), 1, None, ptr::null_mut(), &raw mut out) };
        assert_eq!(status, UNSEEN_INVALID_ARGUMENT);
    }

    #[test]
    fn reason_codes() {
        assert_eq!(REASONS[..UNSEEN_EDIT_REASONS], Reason::EDIT);
        for (i, reason) in REASONS.iter().enumerate() {
            assert_eq!(reason_code(*reason), i as u32 + 1);
            // SAFETY: returns a static NUL-terminated string.
            let name = unsafe { CStr::from_ptr(unseen_reason_name(i as u32 + 1)) };
            assert_eq!(name.to_str(), Ok(reason.as_str()));
        }
        // SAFETY: as above.
        unsafe {
            assert_eq!(CStr::from_ptr(unseen_reason_name(0)).to_str(), Ok("none"));
            assert_eq!(CStr::from_ptr(unseen_reason_name(16)).to_str(), Ok("unknown"));
        }
    }

    #[test]
    fn cancellation_and_arguments() {
        unsafe extern "C" fn yes(_: *mut c_void) -> bool {
            true
        }
        let mut out = unseen_result::status(u32::MAX);
        // SAFETY: valid input and a writable result.
        let status = unsafe { unseen_sanitize_text(b"x".as_ptr(), 1, Some(yes), ptr::null_mut(), &raw mut out) };
        assert_eq!(status, UNSEEN_CANCELLED);
        // SAFETY: null with a non-zero length is rejected before any read.
        let status = unsafe { unseen_sanitize_text(ptr::null(), 1, None, ptr::null_mut(), &raw mut out) };
        assert_eq!(status, UNSEEN_INVALID_ARGUMENT);
        // SAFETY: a null result pointer is rejected before any write.
        let status = unsafe { unseen_sanitize_text(ptr::null(), 0, None, ptr::null_mut(), ptr::null_mut()) };
        assert_eq!(status, UNSEEN_INVALID_ARGUMENT);
        // SAFETY: an empty input may be null.
        let status = unsafe { unseen_sanitize_text(ptr::null(), 0, None, ptr::null_mut(), &raw mut out) };
        assert_eq!(status, UNSEEN_OK);
    }

    #[test]
    fn static_strings() {
        // SAFETY: both return static NUL-terminated strings.
        unsafe {
            assert_eq!(CStr::from_ptr(unseen_unicode_version()).to_str(), Ok(unseen_core::UNICODE_VERSION));
            assert_eq!(CStr::from_ptr(unseen_version()).to_str(), Ok(env!("CARGO_PKG_VERSION")));
            assert_eq!(CStr::from_ptr(unseen_status_name(UNSEEN_VERIFY_FAILED)).to_str(), Ok("verify_failed"));
            assert_eq!(CStr::from_ptr(unseen_status_name(99)).to_str(), Ok("unknown"));
        }
        let p = unseen_alloc(28);
        assert_eq!(p as usize % ALLOC_ALIGN, 0);
        // SAFETY: allocated just above with the same length.
        unsafe { unseen_dealloc(p, 28) };
        assert!(unseen_alloc(0).is_null());
    }
}
