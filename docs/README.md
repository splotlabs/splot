# Developer guide

Everything you need to work on splot. For using splot, see the
[user README](../README.md). Contributor and coding-agent rules are in
[AGENTS.md](../AGENTS.md).

## Toolchain

The workspace pins a nightly in `rust-toolchain.toml` for portable SIMD (edition
2024, resolver 3). Rustup selects and installs it.

```bash
rustup show active-toolchain
cargo build --release
```

x86-64 builds target `x86-64-v3` (AVX2/FMA/BMI2) for decode throughput; an
older CPU traps with `SIGILL`. Use `RUSTFLAGS="-C target-cpu=x86-64"` for a
portable baseline. `aarch64` uses 128-bit NEON as its baseline.

## Crates

```text
crates/splot-core      AV2 bitstream model and parsers
crates/splot-parallel  approved concurrency primitives
crates/splot-tables    generated AV2 § 9 tables
crates/splot-recon     reconstruction primitives and frame storage
crates/splot-decode    decode planning, streaming decoder, runtime output
crates/splot-validate  parser-driven conformance diagnostics
crates/splot-encode    encoder API, Y4M reader, IVF writer
crates/splot-cli       thin `splot` binary
crates/splot-capi      C ABI of the streaming decoder
xtask                  repository automation
fuzz                   cargo-fuzz targets outside the workspace
```

`cargo xtask check-dependency-direction` enforces the dependency direction.

## Gate

```bash
cargo xtask ci
```

It runs formatting, clippy, tests, doctests, rustdoc, optional external tool
checks, dependency and concurrency policy, spec mirror integrity, diagnostic
registry drift, the duplicate-code budget and the documentation budget.

## Documents

- [ARCHITECTURE.md](./ARCHITECTURE.md) - crate boundaries and runtime policies
- [TESTING.md](./TESTING.md) - test layers and proof rules
- [CONFORMANCE.md](./CONFORMANCE.md) - conformance claim policy
- [DIAGNOSTICS.md](./DIAGNOSTICS.md) - CI-enforced diagnostic registry input
- [DECISIONS/](./DECISIONS/) - durable architectural decisions
- [references/THIRD-PARTY-NOTICES.md](./references/THIRD-PARTY-NOTICES.md) -
  legal and source-boundary notices

Machine-readable sources of truth:

- `docs/IMPLEMENTATION-MATRIX.toml` (the canonical status ledger)
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `docs/LOCAL-REFERENCE-EVIDENCE.toml`
- `docs/audits/av2-conformance-ledger.json`
- `tests/conformance/manifest.toml`
- `tests/conformance/decoder-oracle.toml`
- `tests/conformance/decoder-oracle-coverage.toml`

Generated markdown status reports are not committed. Render them on demand:

```bash
cargo xtask feature-status
cargo xtask spec-coverage
cargo xtask decoder-support
cargo xtask decoder-conformance-coverage
cargo xtask decoder-fixtures coverage
```
