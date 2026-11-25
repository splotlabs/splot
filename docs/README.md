# Developer guide

Everything you need to work on splot. For using splot, see the
[user README](../README.md). Contributor and coding-agent rules are in
[AGENTS.md](../AGENTS.md).

## Toolchain

The workspace pins stable Rust in `rust-toolchain.toml` (edition 2024, resolver
3); rustup selects and installs it. SIMD kernels use `fearless_simd`. Only
`cargo +nightly fuzz` needs a nightly toolchain.

```bash
rustup show active-toolchain
cargo build --release
```

Checkout builds target `x86-64-v3` on x86-64. At startup, Auto asks
`fearless_simd` for its Ice Lake AVX-512 capability, including OS register
state and the required instruction extensions. On a qualifying host, display
grain uses AVX-512 to round, add and clip groups of 32 samples on eligible
8-bit and 10-bit rows. Other work uses AVX2. AArch64 uses NEON.

Rust callers can request a backend through `SimdPolicy`. An unavailable
request returns an error at startup. The C API uses Auto. A streaming decoder
retains its selected backend across frames and flushes. For repeated
synchronous decodes, `DecodeContext::with_typed_backend` selects once around
the caller's loop. The ordinary synchronous methods enter that scope for each
call. A deferred `Picture::apply_film_grain` uses the backend stored with the
picture.

Cargo uses the consuming workspace's configuration when building splot as a
dependency. With generic x86-64 flags the decoder can fall back to SSE2.
On Linux GNU, this command builds a baseline CLI and C library while retaining
the release's packed relative relocations and glibc 2.36 requirement.

```bash
RUSTFLAGS="-C target-cpu=x86-64 -C link-arg=-Wl,-z,pack-relative-relocs" \
  cargo build --release --locked --target-dir target/baseline -p splot-cli -p splot-capi
```

The compiler target also affects parsing, motion prediction and the code
around SIMD kernels. A generic build can therefore decode more slowly on the
same AVX2 machine. [Performance measurements and follow-up](https://github.com/splotlabs/splot/issues/1934).

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
`cargo xtask ci --fast` keeps formatting, clippy and the repo checks and skips
tests, rustdoc, external tools and the C ABI example, which GitHub CI runs.

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
