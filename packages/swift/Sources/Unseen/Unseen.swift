import UnseenFFI

public enum Unseen {
    public static let unicodeVersion = String(cString: unseen_unicode_version())

    public static func sanitize(_ text: [UInt8]) throws(UnseenError) -> [UInt8] {
        var result = unseen_result()
        defer { unseen_result_free(&result) }
        let status = text.withUnsafeBufferPointer { input in
            unseen_sanitize_text(input.baseAddress, input.count, pollTaskCancellation, nil, &result)
        }
        try UnseenError.check(status)
        return bytes(result.output, result.output_len)
    }

    public static func sanitize(_ text: String) throws(UnseenError) -> String {
        String(decoding: try sanitize(Array(text.utf8)), as: UTF8.self)
    }

    public static func sanitizeReport(_ text: String) throws(Refusal) -> Report {
        var report = unseen_report()
        defer { unseen_report_free(&report) }
        let input = Array(text.utf8)
        let status = input.withUnsafeBufferPointer { input in
            unseen_sanitize_text_report(input.baseAddress, input.count, pollTaskCancellation, nil, &report)
        }
        do {
            try UnseenError.check(status)
        } catch {
            let at = Reason(code: report.at.reason).map { reason in
                var place = Place(text)
                return Fragment(range: place.range(report.at), reason: reason, replacement: nil)
            }
            throw Refusal(error: error, at: at)
        }
        var place = Place(text)
        let edits = UnsafeBufferPointer(start: report.edits, count: report.edits_len).map { f in
            Fragment(
                range: place.range(f), reason: Reason(code: f.reason) ?? .format,
                replacement: f.with == 0 ? nil : Unicode.Scalar(f.with))
        }
        var totals: [Reason: Total] = [:]
        withUnsafeBytes(of: (report.spans, report.bytes)) { raw in
            let counts = raw.bindMemory(to: Int.self)
            let n = Int(UNSEEN_EDIT_REASONS)
            for (i, reason) in Reason.allCases.prefix(n).enumerated() {
                totals[reason] = Total(spans: counts[i], bytes: counts[n + i])
            }
        }
        return Report(
            output: string(report.output, report.output_len), edits: edits, truncated: report.truncated,
            totals: totals)
    }

    public static func sanitizeFile(named name: String, contents: [UInt8]) throws(UnseenError) -> FileResult {
        var result = unseen_result()
        defer { unseen_result_free(&result) }
        let nameBytes = Array(name.utf8)
        let status = nameBytes.withUnsafeBufferPointer { name in
            contents.withUnsafeBufferPointer { body in
                unseen_sanitize_file(
                    name.baseAddress, name.count, body.baseAddress, body.count, pollTaskCancellation, nil, &result)
            }
        }
        let mime = string(result.mime, result.mime_len)
        let ext = string(result.ext, result.ext_len)
        if status == UInt32(UNSEEN_BINARY) {
            return .binary(mime: mime, ext: ext)
        }
        try UnseenError.check(status)
        return .text(mime: mime, ext: ext, output: bytes(result.output, result.output_len))
    }
}

public enum FileResult: Equatable, Sendable {
    case text(mime: String, ext: String, output: [UInt8])
    case binary(mime: String, ext: String)
}

public enum Reason: String, CaseIterable, Sendable {
    case bom
    case noncharacter
    case unassigned
    case privateUse = "private_use"
    case bidiControl = "bidi_control"
    case joinControl = "join_control"
    case variationSelector = "variation_selector"
    case format
    case detachedMark = "detached_mark"
    case trailingSpace = "trailing_space"
    case homoglyph
    case lineBreak = "line_break"
    case invalidUTF8 = "invalid_utf8"
    case control
    case markOverflow = "mark_overflow"

    init?(code: UInt32) {
        let i = Int(code) - 1
        guard Self.allCases.indices.contains(i) else { return nil }
        self = Self.allCases[i]
    }
}

public struct Fragment: Equatable, Sendable {
    public let range: Range<String.Index>
    public let reason: Reason
    public let replacement: Unicode.Scalar?
}

public struct Total: Equatable, Sendable {
    public let spans: Int
    public let bytes: Int
}

public struct Report: Equatable, Sendable {
    public let output: String
    public let edits: [Fragment]
    public let truncated: Bool
    public let totals: [Reason: Total]
}

public struct Refusal: Error, Equatable, Sendable {
    public let error: UnseenError
    public let at: Fragment?
}

private struct Place {
    let utf8: String.UTF8View
    var index: String.Index
    var offset = 0

    init(_ text: String) {
        utf8 = text.utf8
        index = utf8.startIndex
    }

    mutating func at(_ target: Int) -> String.Index {
        index = utf8.index(index, offsetBy: target - offset)
        offset = target
        return index
    }

    mutating func range(_ f: unseen_fragment) -> Range<String.Index> {
        let start = at(f.start)
        return start..<at(f.end)
    }
}

public enum UnseenError: Error, Equatable, Sendable {
    case unsupportedType
    case tooLarge
    case malformed
    case verifyFailed
    case cancelled
    case invalidArgument
    case `internal`

    public var code: String {
        switch self {
        case .unsupportedType: "unsupported_type"
        case .tooLarge: "too_large"
        case .malformed: "malformed"
        case .verifyFailed: "verify_failed"
        case .cancelled: "cancelled"
        case .invalidArgument: "invalid_argument"
        case .internal: "internal"
        }
    }

    static func check(_ status: UInt32) throws(UnseenError) {
        switch Int(status) {
        case Int(UNSEEN_OK): return
        case Int(UNSEEN_UNSUPPORTED_TYPE): throw .unsupportedType
        case Int(UNSEEN_TOO_LARGE): throw .tooLarge
        case Int(UNSEEN_MALFORMED): throw .malformed
        case Int(UNSEEN_VERIFY_FAILED): throw .verifyFailed
        case Int(UNSEEN_CANCELLED): throw .cancelled
        case Int(UNSEEN_INVALID_ARGUMENT): throw .invalidArgument
        default: throw .internal
        }
    }
}

private let pollTaskCancellation: unseen_cancel_fn = { _ in Task.isCancelled }

private func bytes(_ p: UnsafePointer<UInt8>?, _ count: Int) -> [UInt8] {
    guard let p, count > 0 else { return [] }
    return Array(UnsafeBufferPointer(start: p, count: count))
}

private func string(_ p: UnsafePointer<UInt8>?, _ count: Int) -> String {
    String(decoding: bytes(p, count), as: UTF8.self)
}
