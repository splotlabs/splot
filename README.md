# splot

Spec-faithful AV2 bitstream validation in safe Rust.

`splot` is a validator and inspector for AV2 streams. It accepts raw Annex B
or IVF-wrapped Annex B input and reports structured diagnostics with stable rule
ids, severities, spec sections, offsets, and messages.

[![CI](https://github.com/splotlabs/splot/actions/workflows/ci.yml/badge.svg)](https://github.com/splotlabs/splot/actions/workflows/ci.yml)
[![AV2 spec v1.0.0](https://img.shields.io/badge/AV2%20spec-v1.0.0-blueviolet)](https://av2.aomedia.org/v1.0.0/index.html)
[![Rust 1.96 · edition 2024](https://img.shields.io/badge/rust-1.96%20%C2%B7%20edition%202024-orange)](./rust-toolchain.toml)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success)](./Cargo.toml)
[![License: PolyForm Noncommercial 1.0.0](https://img.shields.io/badge/license-PolyForm%20Noncommercial%201.0.0-blue)](./LICENSE.md)

## Status

Pre-alpha, validator-first. The validator, inspector, diagnostic catalog, and a
narrow experimental decode tier are present. `splot encode` is not a production
encoder, and broad AV2 playback is not claimed.

The canonical status ledger is
[`docs/IMPLEMENTATION-MATRIX.toml`](./docs/IMPLEMENTATION-MATRIX.toml). Generated
views are produced on demand:

```bash
cargo xtask feature-status
cargo xtask spec-coverage
cargo xtask decoder-support
cargo xtask decoder-conformance-coverage
```

## Quick Start

```bash
rustup show active-toolchain
cargo build --release
./target/release/splot --help
```

> **x86-64 CPU requirement:** builds target the `x86-64-v3` microarchitecture
> level (AVX2/FMA/BMI2, Intel Haswell 2013+ / AMD Excavator 2015+) for decode
> throughput. On an older x86 CPU the binary traps with `SIGILL`; build with
> `RUSTFLAGS="-C target-cpu=x86-64" cargo build --release` for a portable
> baseline. `aarch64` (Apple Silicon, ARM servers) is unaffected — 128-bit NEON
> is its baseline.

Common commands:

```bash
splot validate sample.av2
splot validate sample.ivf --json
splot validate sample.av2 --strict
splot validate sample.av2 --summary-only
splot inspect sample.ivf --headers
splot inspect sample.av2 --json
splot explain obu-header/global-xlayer-required
```

Exit codes are stable: `0` means clean, `1` means findings or an unsupported
codec operation, and `2` means operational or usage failure.

## Project Layout

```text
crates/splot-core      AV2 bitstream model and parsers
crates/splot-parallel  approved concurrency primitives
crates/splot-tables    generated AV2 § 9 tables
crates/splot-recon     reconstruction primitives and frame storage
crates/splot-decode    decode planning, diagnostics, and narrow runtime output
crates/splot-validate  parser-driven conformance diagnostics
crates/splot-encode    future encoder API and private tools
crates/splot-cli       thin `splot` binary
xtask                  repository automation
fuzz                   cargo-fuzz targets outside the workspace
```

Dependency direction is enforced by `cargo xtask check-dependency-direction`.
Architecture and ownership rules are in [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md).

## Validation

```bash
cargo xtask ci
```

The CI gate runs formatting, clippy, tests, doctests, rustdoc, optional external
tool checks, dependency and concurrency policy checks, spec mirror integrity,
diagnostic registry drift checks, duplicate-code budget checks, and the manual
documentation budget.

Focused docs:

- [docs/README.md](./docs/README.md) - retained documentation map
- [docs/TESTING.md](./docs/TESTING.md) - test layers and commands
- [docs/CONFORMANCE.md](./docs/CONFORMANCE.md) - conformance proof policy
- [docs/DIAGNOSTICS.md](./docs/DIAGNOSTICS.md) - emitted diagnostic registry
- [AGENTS.md](./AGENTS.md) - contributor and coding-agent rules

## License

Project code, docs, tests, and fixtures are PolyForm Noncommercial 1.0.0.
Third-party/source-boundary material is listed in
[docs/references/THIRD-PARTY-NOTICES.md](./docs/references/THIRD-PARTY-NOTICES.md).
