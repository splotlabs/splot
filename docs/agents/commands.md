# Agent Command Reference

Use `cargo xtask ci` as the acceptance gate for completed work.

## Acceptance Gate

```bash
cargo xtask ci
```

`cargo xtask ci` runs:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo build --workspace --all-targets --locked`
- `cargo test --workspace --all-targets --locked`
- `cargo test --doc --workspace --locked`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`
- run-if-present external checks: `typos`, `cargo machete`, `cargo deny check
  bans licenses sources`, and `openspec validate --all --no-interactive`
- repository gates listed below

CI installs `typos`, `cargo-machete`, and `cargo-deny`, so those checks gate in
CI. OpenSpec validation is conditional in CI and local runs: it runs when the
`openspec` binary is present and otherwise prints a skip message.

## Focused Checks

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo test --doc --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
typos
cargo machete --with-metadata
cargo deny check bans licenses sources
openspec validate --all --no-interactive   # run when openspec is installed
```

## Repository Gates

```bash
cargo xtask check-license-headers
cargo xtask check-source-lines
cargo xtask check-dependency-direction
cargo xtask check-concurrency-policy
cargo xtask check-zero-copy-policy
cargo xtask check-spec-mirror
cargo xtask check-fuzz-targets
cargo xtask gen-tables --check
cargo xtask gen-explain --check
cargo xtask check-feature-status
cargo xtask check-decoder-support
cargo xtask check-decoder-conformance-coverage
cargo xtask check-reference-evidence
cargo xtask check-diagnostic-registry
cargo xtask check-fixtures
cargo xtask check-duplication   # needs the `dupehound` binary; run-if-present locally
```

`check-duplication` enforces the absolute *production*-duplication ceiling in
`tools/dupehound/budget.toml` using
[dupehound](https://github.com/Rafaelpta/dupehound) (`dupehound scan --json`,
default scope). It fails when the deletable-line count exceeds the budget; lower
the budget in the same commit that removes a duplicate cluster — never raise it.
The scope is dupehound's default, which **excludes `#[test]` bodies**:
deliberately-explicit per-scenario tests are intentional here and are not gated.
This is a ratchet, not a zero mandate — it prevents new duplication and reduces
the production duplication that hurts maintainability. The complementary per-PR
ratchet (`dupehound check --diff <base>`, blocking newly introduced duplication)
runs only in CI. Before reimplementing something, run `dupehound check` (or
`dupehound scan . --explain <N>`) and reuse the original instead.

## Generated Status Docs

```bash
cargo xtask feature-status
cargo xtask feature-status --format json
cargo xtask feature-status --category normative
cargo xtask feature-status --kind bitstream-syntax
cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md

cargo xtask spec-coverage
cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md

cargo xtask writer-coverage --format markdown --output docs/spec-coverage-writer.md
cargo xtask decoder-support --format markdown --output docs/DECODER-SUPPORT-STATUS.md
cargo xtask decoder-conformance-coverage --format markdown --output docs/DECODER-SPEC-COVERAGE.md
```

## Conformance, Coverage, Fuzzing, Audit

```bash
cargo xtask conformance
cargo xtask coverage
cargo xtask fuzz [--time <secs>]
cargo xtask audit
cargo xtask audit-scope --format json
```

Fuzzing requires nightly plus `cargo-fuzz`:

```bash
cargo install cargo-fuzz --locked
cargo xtask seed-fuzz-corpus      # populate fuzz/corpus/<target>/ from the committed fixtures + vectors
cargo +nightly fuzz list
cargo +nightly fuzz run parse_obu
```

`seed-fuzz-corpus` is the Rust home of the corpus seeding the CI fuzz-smoke job
runs (it replaced ~100 lines of inline workflow shell/Python); the byte layouts
are unit-tested in `xtask/src/seed_fuzz_corpus.rs`.

Current fuzz targets include parser, validator, symbol coder/decoder,
tile-payload decode, decode planning/runtime byte surfaces, reconstruction
runtime surfaces, encoder input views, encoder context state machine, and OBU
roundtrips. See [../TESTING.md](../TESTING.md) for the full target list and
coverage intent.
