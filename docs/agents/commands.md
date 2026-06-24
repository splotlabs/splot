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

CI installs the external tools, so they always gate in CI. Locally,
`cargo xtask ci` skips a missing external tool with an install hint.

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
openspec validate --all --no-interactive
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
```

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
cargo +nightly fuzz list
cargo +nightly fuzz run parse_obu
```

Current fuzz targets include parser, validator, symbol coder/decoder,
tile-payload decode, decode planning/runtime byte surfaces, reconstruction
runtime surfaces, encoder input views, encoder context state machine, and OBU
roundtrips. See [../TESTING.md](../TESTING.md) for the full target list and
coverage intent.
