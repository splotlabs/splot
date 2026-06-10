# Architecture

This document covers the workspace split, the dependency rules, the error model,
and the unsafe/SIMD policy; [AGENTS.md](../AGENTS.md) § 2 and § 5 remain
canonical for the rules summarized here.

## Crate dependency graph

```text
splot-cli ───────┬──> splot-validate ───> splot-core
                 ├──> splot-encode   ───> splot-core
                 └──> splot-core

xtask is standalone automation.
fuzz lives outside the workspace and depends on splot-core only.
```

**One-way dependency rule** (canonical: [AGENTS.md](../AGENTS.md) § 2; enforced
by `cargo xtask check-dependency-direction`):

- `splot-core` depends on no other `splot-*` crate.
- Nothing depends on `splot-cli`.
- Nothing depends on `splot-encode` except `splot-cli`.
- `xtask` depends on no `splot-*` crate.

## Crate responsibilities

- **`splot-core`** — the AV2 spec modeled in Rust. It owns the typed `Error`
  model; no I/O, no other `splot-*` dependency.
  - Strong types (`ObuType`, layer ids, `ByteOffset`/`BitOffset`) and a
    panic-free bit reader for the § 4.11 descriptors.
  - LEB128, the OBU header (§ 5.2.2), Annex B envelopes, IVF container parsing
    and writing (`AV2-IVF-CONTAINER`), and payload dispatch.
  - The implemented § 5 payload parsers, in `crates/splot-core/src/headers/`;
    the generated [SPEC-COVERAGE.md](./SPEC-COVERAGE.md) is the live list.
- **`splot-validate`** — parser output → user-facing conformance diagnostics. A
  `Validator` parses raw Annex B or IVF-wrapped Annex B with `splot-core`, then
  runs a registry of `Check`s. Each `Diagnostic` is structured data (rule id,
  severity, spec section, offset, message). A malformed bitstream or container is
  a report, never a process failure.
- **`splot-encode`** — the *shape* of the future encoder API (configuration plus a
  push/pull `Context`). It implements no encoding; every operation returns
  `Error::Unimplemented`.
- **`splot-cli`** — the thin `splot` binary. It parses arguments (clap),
  initializes logging (tracing), reads/writes files, and calls library APIs. No
  codec logic: the `inspect`/`validate` text and JSON rendering in
  `crates/splot-cli/src/commands/` are presentation over `splot-core` and
  `splot-validate` output. Exit codes are part of the contract: `0` clean;
  `1` findings (`validate`: validation errors, or warnings under `--strict`;
  `inspect`: a parse error); `2` operational error.
- **`xtask`** — project automation: the `ci` pipeline; the repository checks
  (`check-license-headers`, `check-dependency-direction`, `check-spec-mirror`,
  `check-feature-status`, `check-diagnostic-registry`,
  `check-conventional-commits`/`-title`); matrix reporting (`feature-status`,
  `spec-coverage`); audit scoping (`audit-scope`); `audit`, `coverage`, and
  `fuzz` wrappers; plus the `gen-tables`, `fetch-vectors`, and `conformance`
  stubs.

## Reference-informed encoder architecture

rav1e and SVT-AV1 are engineering references only, never sources of AV2 syntax,
code, tables, or constants. The canonical gate before any encoder work is
[AGENTS.md](../AGENTS.md) § 1a; the research notes live under
[docs/references/](./references/).

## Error model

(Canonical: [AGENTS.md](../AGENTS.md) § 5.)

Libraries use typed errors (`thiserror`); `anyhow` is confined to `splot-cli` and
`xtask`. Library code never panics on malformed input. Recognized-but-unmodeled
functionality returns `Error::Unimplemented { feature }`.

## Unsafe / SIMD policy

`unsafe_code` is **forbidden** workspace-wide. Future SIMD or FFI work may introduce
`unsafe` only inside narrowly-scoped, individually-documented, individually-tested
modules, and only when justified by measurements. The `fuzz` crate sits outside the
workspace so that libFuzzer's `unsafe` runtime is not subject to this lint.

See [CODE_REVIEW.md](./CODE_REVIEW.md) for the review checklist and
[TESTING.md](./TESTING.md) for the test layers.
