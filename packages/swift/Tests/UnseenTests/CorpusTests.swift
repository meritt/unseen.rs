import CryptoKit
import Foundation
import Testing
import UnseenFFI

@testable import Unseen

private struct Segment: Decodable {
    let hex: String
    let repeatCount: Int
    enum CodingKeys: String, CodingKey {
        case hex
        case repeatCount = "repeat"
    }
}

private struct Output: Decodable {
    let len: Int
    let sha256: String
    let hex: String?
}

private struct Want: Decodable {
    let result: String
    let mime: String?
    let ext: String?
    let output: Output?
}

private struct Case: Decodable {
    let name: String
    let slow: Bool?
    let fileName: String?
    let input: [Segment]
    let want: Want
    enum CodingKeys: String, CodingKey {
        case name, slow, input, want
        case fileName = "file_name"
    }
}

private struct Random: Decodable {
    let seed: String
    let cases: Int
    let maxTokens: Int
    let blockSize: Int
    let fileNames: [String]?
    let alphabet: [String]
    let blocksSHA256: [String]
    enum CodingKeys: String, CodingKey {
        case seed, cases, alphabet
        case maxTokens = "max_tokens"
        case blockSize = "block_size"
        case fileNames = "file_names"
        case blocksSHA256 = "blocks_sha256"
    }
}

private struct Corpus: Decodable {
    let format: Int
    let unicodeVersion: String?
    let random: Random
    let cases: [Case]
    enum CodingKeys: String, CodingKey {
        case format, random, cases
        case unicodeVersion = "unicode_version"
    }
}

private let corpusDirectory: URL = {
    if let dir = ProcessInfo.processInfo.environment["UNSEEN_CORPUS"] {
        return URL(fileURLWithPath: dir)
    }
    return URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .appendingPathComponent("../../../../testdata/textcorpus")
        .standardized
}()

private func loadCorpus(_ file: String) throws -> Corpus {
    let data = try Data(contentsOf: corpusDirectory.appendingPathComponent(file))
    let corpus = try JSONDecoder().decode(Corpus.self, from: data)
    #expect(corpus.format == 1)
    return corpus
}

private func unhex(_ s: String) -> [UInt8] {
    var out: [UInt8] = []
    out.reserveCapacity(s.utf8.count / 2)
    var it = s.utf8.makeIterator()
    while let hi = it.next(), let lo = it.next() {
        out.append(UInt8(String(decoding: [hi, lo], as: UTF8.self), radix: 16)!)
    }
    return out
}

private func hex<D: Sequence>(_ bytes: D) -> String where D.Element == UInt8 {
    bytes.map { String(format: "%02x", $0) }.joined()
}

private func expand(_ input: [Segment]) -> [UInt8] {
    input.flatMap { segment in Array(repeating: unhex(segment.hex), count: segment.repeatCount).joined() }
}

private func checkOutput(_ name: String, _ want: Want, _ got: [UInt8]) {
    guard let output = want.output else {
        Issue.record("\(name): no recorded output")
        return
    }
    #expect(output.len == got.count, "\(name)")
    #expect(output.sha256 == hex(SHA256.hash(data: got)), "\(name)")
    if let expected = output.hex {
        #expect(expected == hex(got), "\(name)")
    }
}

private struct SplitMix64 {
    var state: UInt64
    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
    mutating func below(_ n: Int) -> Int { Int(next() % UInt64(n)) }
}

private struct FileRecord {
    var code: UInt8
    var result: String
    var mime = ""
    var ext = ""
    var output: [UInt8]? = nil
}

private func fileRecord(_ name: String, _ body: [UInt8]) -> FileRecord {
    do {
        switch try Unseen.sanitizeFile(named: name, contents: body) {
        case let .text(mime, ext, output):
            return FileRecord(code: 0, result: "ok", mime: mime, ext: ext, output: output)
        case let .binary(mime, ext):
            return FileRecord(code: 5, result: "binary", mime: mime, ext: ext)
        }
    } catch {
        let code: UInt8 =
            switch error {
            case .unsupportedType: 1
            case .tooLarge: 2
            case .malformed: 3
            case .verifyFailed: 4
            default: 255
            }
        return FileRecord(code: code, result: error.code)
    }
}

@Test func fileCorpusCases() throws {
    let corpus = try loadCorpus("files.json")
    for c in corpus.cases {
        let got = fileRecord(c.fileName ?? "", expand(c.input))
        #expect(got.result == c.want.result, "\(c.name)")
        #expect(got.mime == (c.want.mime ?? ""), "\(c.name)")
        #expect(got.ext == (c.want.ext ?? ""), "\(c.name)")
        if let output = got.output {
            checkOutput(c.name, c.want, output)
        } else {
            #expect(c.want.output == nil, "\(c.name)")
        }
    }
}

@Test func fileCorpusRandom() throws {
    let corpus = try loadCorpus("files.json")
    let random = corpus.random
    let alphabet = random.alphabet.map(unhex)
    let names = random.fileNames ?? []
    var rng = SplitMix64(state: UInt64(random.seed.dropFirst(2), radix: 16)!)
    var hasher = SHA256()
    #expect(random.blocksSHA256.count * random.blockSize == random.cases)
    for i in 0..<random.cases {
        let name = names[rng.below(names.count)]
        let n = 1 + rng.below(random.maxTokens)
        var body: [UInt8] = []
        for _ in 0..<n { body += alphabet[rng.below(alphabet.count)] }
        let r = fileRecord(name, body)
        var record: [UInt8] = [r.code, UInt8(r.mime.utf8.count)] + Array(r.mime.utf8)
        record += [UInt8(r.ext.utf8.count)] + Array(r.ext.utf8)
        if let output = r.output {
            withUnsafeBytes(of: UInt32(output.count).bigEndian) { record += $0 }
            record += output
        }
        hasher.update(data: record)
        if (i + 1) % random.blockSize == 0 {
            let block = i / random.blockSize
            #expect(hex(hasher.finalize()) == random.blocksSHA256[block], "block \(block)")
            hasher = SHA256()
        }
    }
}

@Test func textCorpusCases() throws {
    let corpus = try loadCorpus("text.json")
    #expect(corpus.unicodeVersion == Unseen.unicodeVersion)
    for c in corpus.cases where c.slow != true {
        let input = expand(c.input)
        do {
            let output = try Unseen.sanitize(input)
            #expect(c.want.result == "ok", "\(c.name)")
            checkOutput(c.name, c.want, output)
            checkReport(c.name, String(decoding: input, as: UTF8.self), output)
        } catch {
            #expect(error.code == c.want.result, "\(c.name)")
        }
    }
}

private func checkReport(_ name: String, _ text: String, _ output: [UInt8]) {
    guard let r = try? Unseen.sanitizeReport(text) else {
        Issue.record("\(name): report refused")
        return
    }
    #expect(Array(r.output.utf8) == output, "\(name)")
    let scalars = text.unicodeScalars
    var rebuilt = String.UnicodeScalarView()
    var cursor = scalars.startIndex
    var listed: [Reason: Total] = [:]
    for edit in r.edits {
        #expect(cursor <= edit.range.lowerBound && !edit.range.isEmpty, "\(name)")
        rebuilt.append(contentsOf: scalars[cursor..<edit.range.lowerBound])
        if let replacement = edit.replacement { rebuilt.append(replacement) }
        cursor = edit.range.upperBound
        let total = listed[edit.reason] ?? Total(spans: 0, bytes: 0)
        let bytes = text.utf8.distance(from: edit.range.lowerBound, to: edit.range.upperBound)
        listed[edit.reason] = Total(spans: total.spans + 1, bytes: total.bytes + bytes)
    }
    rebuilt.append(contentsOf: scalars[cursor...])
    #expect((String(rebuilt) != r.output) == r.truncated, "\(name)")
    if !r.truncated {
        for (reason, total) in r.totals {
            #expect(total == (listed[reason] ?? Total(spans: 0, bytes: 0)), "\(name): \(reason)")
        }
    }
}

@Test func reasonsFollowTheCodes() {
    for (i, reason) in Reason.allCases.enumerated() {
        #expect(reason.rawValue == String(cString: unseen_reason_name(UInt32(i + 1))))
    }
    #expect(Reason(code: 0) == nil)
}

@Test func report() throws {
    let text = "\u{1F600} \u{301}a\u{200B}b\u{202E}"
    let r = try Unseen.sanitizeReport(text)
    #expect(r.output == "\u{1F600} ab")
    #expect(r.edits.map(\.reason) == [.detachedMark, .format, .bidiControl])
    #expect(r.edits.map { String(text.unicodeScalars[$0.range]) } == ["\u{301}", "\u{200B}", "\u{202E}"])
    #expect(r.edits.allSatisfy { $0.replacement == nil })
    #expect(r.totals[.format] == Total(spans: 1, bytes: 3))
    #expect(r.totals[.bom] == Total(spans: 0, bytes: 0))
    #expect(!r.truncated)

    let refused = "\u{1F600}x\u{0}\u{0}y"
    #expect {
        try Unseen.sanitizeReport(refused)
    } throws: { error in
        guard let refusal = error as? Refusal, let at = refusal.at else { return false }
        return refusal.error == .malformed && at.reason == .control
            && String(refused.unicodeScalars[at.range]) == "\u{0}\u{0}"
    }

    let many = try Unseen.sanitizeReport(String(repeating: "a\u{200B}", count: 100_005))
    #expect(many.edits.count == 100_000 && many.truncated)
    #expect(many.totals[.format] == Total(spans: 100_005, bytes: 300_015))
}

@Test func reportEdits() throws {
    let text = "\u{1F600} \u{430}pple.com \r\nx\u{2028}"
    let r = try Unseen.sanitizeReport(text)
    #expect(r.output == "\u{1F600} apple.com\nx\n")
    #expect(r.edits.map(\.reason) == [.homoglyph, .trailingSpace, .lineBreak, .lineBreak])
    #expect(r.edits.map(\.replacement) == ["a", nil, nil, "\n"])
    #expect(r.edits.map { String(text.unicodeScalars[$0.range]) } == ["\u{430}", " ", "\r", "\u{2028}"])
    #expect(r.totals[.homoglyph] == Total(spans: 1, bytes: 2))
    #expect(r.totals[.lineBreak] == Total(spans: 2, bytes: 4))
}

@Test func stringsAndCancellation() async throws {
    #expect(try Unseen.sanitize("Съешь\u{200B}ещё \u{1F469}\u{200D}\u{1F4BB}") == "Съешьещё \u{1F469}\u{200D}\u{1F4BB}")
    #expect(throws: UnseenError.malformed) { try Unseen.sanitize("a\u{0}b") }
    let task = Task {
        withUnsafeCurrentTask { $0?.cancel() }
        return Result { () throws(UnseenError) in try Unseen.sanitize("x") }
    }
    #expect(await task.value == .failure(.cancelled))
}
