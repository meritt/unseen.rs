# Architecture

## Scope

unseen turns untrusted text into text that carries no hidden marks. `sanitize_text` takes bytes and returns sanitized UTF-8 or a refusal code. `sanitize_text_report` returns the same result together with every edit. `sanitize_file` routes a named file to a text type or reports a binary one. Output is deterministic and byte-exact against a reference corpus.

## Components

```text
unseen-tablegen ──► unicode17_data.rs
                          │
                     unseen-core          pure functions; no I/O, no unsafe, no dependencies
                          │
                     unseen-ffi           C ABI; the only unsafe code
                ┌─────────┴──────────┐
     unseen.wasm + unseen.js    UnseenFFI.xcframework + Unseen (Swift)
```

Dependencies point down only. Bindings add no policy: they marshal arguments, map status codes and forward cancellation.

## Pipeline

```text
text:  bytes ─► policy ─► normalize ─► verify ─► UTF-8
                 │          │            │
                 │          │            └─ independent rescan of the output
                 │          └─ homoglyphs, trailing whitespace, line breaks → LF
                 └─ per-scalar keep / remove / reject from Unicode 17 tables

file:  name, bytes ─► extension gate ─► content sniff ─► text subtype ─► text
                                              └─► Binary { mime, ext }
```

| Stage | Removes | Rejects |
|---|---|---|
| policy | BOM, format and bidi controls, tags, private use, noncharacters, unassigned; joiners and variation selectors outside registered emoji sequences; marks without a base | NUL, controls, invalid UTF-8, more than 32 combining marks |
| normalize | horizontal whitespace before a line end or the end of text; CR of CRLF | — |

Normalization replaces a Latin, Cyrillic, Greek, Armenian or Cherokee letter with its look-alike from the script that holds a strict majority of letters in its word. In a host name, look-alikes become Latin unless one of them belongs to the TLD's script. CR, VT, NEL, LS and PS become LF.

The report is the same pass with an edit ledger. Applying its edits to the input yields the output. A refusal names the first rejected run and the reason of its first scalar.

## Bounds

| Limit | Value | On excess |
|---|---|---|
| `MAX_INPUT_BYTES` | 100 MiB, input and output | `too_large` |
| `MAX_WORK` | 16 × `MAX_INPUT_BYTES` work units, policy | `malformed` |
| `MAX_NORMALIZE_WORK` | 32 × `MAX_INPUT_BYTES` work units, normalize | `malformed` |
| `MAX_EDITS` | 100 000 report fragments | `truncated`; totals stay complete |

Cancellation is polled at entry and periodically during the scan, and yields `cancelled`.

## C ABI

- The caller provides an `unseen_result` or `unseen_report`. The library fills it and returns its status.
- A byte pointer may be NULL only with length 0; `cancel` may be NULL.
- Buffers belong to the library until `unseen_result_free` or `unseen_report_free`; freeing twice is a no-op.
- Calls are reentrant and thread-safe. Cancellation is an `unseen_cancel_fn` with a context pointer.
- A panic never crosses the boundary. Native builds return `UNSEEN_INTERNAL`; in wasm32 a panic traps the instance.
- `unseen_version` identifies the build. Bindings refuse a library whose version differs from their own.

## Bindings

**WebAssembly.** `unseen.wasm` is the `cdylib` of `unseen-ffi` with no generated glue. `unseen.js` calls the exports through linear memory. It reads the result structures at wasm32 offsets that `unseen-ffi` asserts at compile time, and checks the module version before any read. Report offsets are UTF-16 indices for strings and byte indices for `Uint8Array`.

**Swift.** `UnseenFFI.xcframework` holds one universal static library for `arm64` and `x86_64` with the header as module `UnseenFFI` under `Headers/UnseenFFI/`, which keeps its module map distinct from other Rust libraries in the same app. `Package.swift` uses a local build when present, otherwise the release binary pinned by URL and checksum. `Unseen` calls synchronously and passes `Task.isCancelled` as the cancellation poll.

## Conformance

`testdata/textcorpus` is a pinned copy of the reference corpus: text and file cases, random blocks, expected codes, bytes and work counts. The Rust, WebAssembly and Swift test suites all replay it. Work counts do not cross the ABI and are checked in Rust only.

`unseen-tablegen` builds the Unicode tables from UCD files pinned by SHA-256. `UNICODE_TABLES_SHA256` fingerprints the result.

## Distribution

| Artifact | Consumer |
|---|---|
| `unseen-core` crate | crates.io |
| `unseen-vX.Y.Z-npm.tgz`, package `@meritt/unseen-wasm` | npmjs.com |
| `unseen-vX.Y.Z-xcframework.zip` | SwiftPM, through `Package.swift` |

The release set is reproducible: the tarball and the zip have fixed entry order, times and modes. The checksum pinned in `Package.swift` before tagging is therefore the checksum of the tagged build.
