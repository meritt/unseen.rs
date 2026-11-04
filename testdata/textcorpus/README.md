# Text sanitization corpus

Expected behavior of the text subset: for every input, the verdict, the output bytes and, for the text policy, the work spent. An implementation that matches the whole corpus produces the same results as this one.

The Go implementation generates the JSON files; never edit them by hand.

```bash
export GOTOOLCHAIN=go1.27.3
go test ./internal/sanitize/ ./internal/pipeline/ -run 'Test(Text|File)Corpus$'
go test ./internal/sanitize/ ./internal/pipeline/ -run 'Test(Text|File)Corpus$' -update-text-corpus
go test ./internal/sanitize/ -run '^TestTextCorpus$' -slow-text-corpus
```

The first command fails when behavior changes and the corpus does not. `-update-text-corpus` regenerates the corpus. Slow cases (100 MiB inputs, exhausted budgets) are read from the file unless `-update-text-corpus` or `-slow-text-corpus` replays them.

## `text.json`: text policy

Input is a text body; output is `textReconstructor` Reconstruct followed by Verify of its result. The policy has two stages:

1. Per scalar: keep, remove or refuse.
2. The normalizer reads the kept stream as it is produced:
   - replaces look-alikes by the majority rule within a word and Latin look-alikes in host labels;
   - removes horizontal whitespace before a line end or the end of text;
   - turns CR, CRLF, VT, NEL, LS and PS into LF.

The stages interleave, so the result code is the first error in input order: exceeding the output limit before a later stage-1 refusal yields `too_large`, not `malformed`. Verify runs both stages over the output and fails if stage 1 would remove or stage 2 would change anything.

| Field | Meaning |
|---|---|
| `format` | corpus format version, `1` |
| `unicode_version`, `unicode_tables_sha256` | Unicode version and fingerprint of the generated tables |
| `max_input_bytes`, `max_work` | input limit and stage-1 work budget per pass; the same limit applies to output |
| `max_normalize_work` | normalizer work budget per pass |
| `scalars_sha256` | digest of one record per scalar U+0000..U+10FFFF without surrogates, ascending |
| `random` | random inputs: generator, token alphabet, block digests |
| `cases` | named cases with expected results |

Results: `ok`, `malformed`, `too_large`, `verify_failed`. `work` and `verify_work` are the budget spent by Reconstruct and Verify, both stages combined, up to completion or refusal; a Reconstruct refusal has `verify_work` 0.

Digest record: code byte (`0` ok, `1` malformed, `2` too_large, `3` verify_failed), `work` and `verify_work` as big-endian uint64, and for `ok` the output length as big-endian uint32 followed by the output.

## `files.json`: whole file

Input is a client file name and a body. The path follows `Process` through Verify: extension gate, content detection with ID3v2 skipping, text subtype from the extension, then Reconstruct and Verify. A body the service would hand to a binary reconstructor is recorded by type and extension only, with result `binary`.

Results: `ok`, `unsupported_type`, `too_large`, `malformed`, `verify_failed`, `binary`. `ok` and `binary` carry `mime` and `ext`, the stored type and extension.

Digest record: code byte (`0` ok, `1` unsupported_type, `2` too_large, `3` malformed, `4` verify_failed, `5` binary), `mime` length byte and bytes, `ext` length byte and bytes, and for `ok` the output length as big-endian uint32 followed by the output.

Operational outcomes are out of scope: `busy`, token limits and allowlist, `max_size`, cancellation.

## Inputs and random blocks

A case input is a list of `{hex, repeat}` segments: each segment's bytes repeated `repeat` times, concatenated in order. An `ok` output is given by length, SHA-256 and, up to 256 bytes, the bytes themselves.

Random inputs are not stored; splitmix64 derives them from `seed`:

```text
state += 0x9E3779B97F4A7C15
z = (state ^ (state >> 30)) * 0xBF58476D1CE4E5B9
z = (z ^ (z >> 27)) * 0x94D049BB133111EB
return z ^ (z >> 31)
```

Arithmetic is modulo 2^64. In `text.json` an input is `1 + next() % max_tokens` tokens, each `alphabet[next() % len(alphabet)]`. In `files.json` the name `file_names[next() % len(file_names)]` is drawn first, then the body the same way. Digest records are concatenated; after every `block_size` inputs the block digest is checked against the next `blocks_sha256` entry and reset.
