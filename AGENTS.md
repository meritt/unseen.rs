# unseen

Rust core behind a C ABI, shipped as a crate, a WebAssembly npm package and a Swift package. Design: [docs/architecture.md](docs/architecture.md).

## Layout

| Path | Contents |
|---|---|
| `crates/unseen-core` | public Rust API; no dependencies, no `unsafe` |
| `crates/unseen-ffi` | C ABI; `include/unseen.h` is hand-written |
| `crates/unseen-tablegen` | generates `unseen-core/src/unicode17_data.rs` from pinned UCD files |
| `crates/unseen-test-support` | SHA-256 and JSON for tools and tests |
| `xtask` | build, checks, release set |
| `packages/npm`, `packages/swift` | bindings; the Swift manifest is `Package.swift` at the root |
| `testdata/textcorpus` | reference corpus, pinned by `testdata/textcorpus.sha256`; never edited by hand |

## Toolchain

rustup (pinned by `rust-toolchain.toml`; not Homebrew `rust`), macOS 26 with Xcode 26, Node.js per `.nvmrc`, pnpm 12, `wasm-opt` from binaryen 133 (`WASM_OPT` overrides the path).

## Checks

```bash
cargo fmt --all --check
CARGO_BUILD_WARNINGS=deny cargo clippy --workspace --all-targets --locked
CARGO_BUILD_WARNINGS=deny cargo clippy -p unseen-core -p unseen-ffi --locked --target wasm32-unknown-unknown
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo test --workspace --locked
cargo xtask header --check
cargo xtask tables --check
cargo xtask corpus --check
cargo deny --locked check
cargo +nightly-2026-11-01 miri test -p unseen-ffi --locked
cargo xtask dist
```

CI runs the same set: `.github/workflows/ci.yml`.

## Rules

- No external crates.
- `include/unseen.h`, `unseen-ffi` and the wasm32 offsets in `packages/npm/unseen.js` change together.
- The macOS floor changes together in `Package.swift` and `MACOS` in `xtask`.
- Corpus update: `cargo xtask corpus <upstream testdata/textcorpus>`. A corpus failure is a defect in this code.
- Unicode update: change the pinned UCD files in `unseen-tablegen`, then `cargo xtask tables`.
- Commit messages: one Russian line, present-tense verb.

## Release

1. Set `version` in `Cargo.toml` (`[workspace.package]`) and `packages/npm/package.json`; merge to `main`.
2. Take the release set CI built for that commit and pin it:

   ```bash
   gh run download <run-id> --repo meritt/unseen.rs --name release-set --dir /tmp/unseen-set
   ```

   ```bash
   cargo xtask pin /tmp/unseen-set
   ```

3. Merge the pin to `main`, tag it and push the tag:

   ```bash
   git tag -a vX.Y.Z -m "vX.Y.Z"
   ```

   ```bash
   git push origin vX.Y.Z
   ```

Repository settings:

- Immutable releases enabled.
- crates.io: environment `crates-io`, trusted publisher for `release.yml` and `crates-io`. The first version is published by hand: `cargo publish -p unseen-core --locked`.
- npmjs.com: environment `npm`, trusted publisher for `release.yml` and `npm`. The first version is published by hand: `pnpm publish unseen-vX.Y.Z-npm.tgz --access public`.

Recovery:

| Failed at | Action |
|---|---|
| Identity, Gates, Build | rerun; for a code fix delete the tag (`git push origin :refs/tags/vX.Y.Z`), fix on `main`, tag again |
| Draft, Readback, Publish | `gh release delete vX.Y.Z --repo meritt/unseen.rs --yes`, rerun |
| Immutability, Ship, Crates | rerun the failed job |
| Published release is wrong | release the next patch version |
