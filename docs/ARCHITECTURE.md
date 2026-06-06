# Architecture

## Crate dependency graph

```text
splot-cli ───────┬──> splot-validate ───> splot-core
                 ├──> splot-encode   ───> splot-core
                 └──> splot-core

xtask is standalone automation.
fuzz lives outside the workspace and depends on splot-core only.
```

**One-way dependency rule** (enforced by `cargo xtask check-dependency-direction`):

- `splot-core` depends on no other `splot-*` crate.
- Nothing depends on `splot-cli`.
- Nothing depends on `splot-encode` except `splot-cli`.
- `xtask` depends on no `splot-*` crate.

## Crate responsibilities

- **`splot-core`** — the AV2 spec modeled in Rust: strong types (`ObuType`, layer
  ids, `ByteOffset`/`BitOffset`), the bit reader, and panic-free parsers for LEB128
  (§ 4.11.6), OBU headers (§ 5.2.2), and Annex B envelopes (Annex B). It owns the
  typed `Error` model. No I/O, no other `splot-*` dependency.
- **`splot-validate`** — parser output → user-facing conformance diagnostics. A
  `Validator` parses with `splot-core`, then runs a registry of `Check`s. Each
  `Diagnostic` is structured data (rule id, severity, spec section, offset, message).
  A malformed bitstream is a report, never a process failure.
- **`splot-encode`** — the *shape* of the future encoder API (configuration plus a
  push/pull `Context`). It implements no encoding; every operation returns
  `Error::Unimplemented`.
- **`splot-cli`** — the thin `splot` binary. It parses arguments (clap), initializes
  logging (tracing), reads/writes files, and calls library APIs. No codec logic.
- **`xtask`** — project automation: the `ci` pipeline and the repository checks
  (`check-license-headers`, `check-dependency-direction`) plus codegen/vector/
  conformance stubs.

## Error model

Libraries use typed errors (`thiserror`); `anyhow` is confined to `splot-cli` and
`xtask`. Library code never panics on malformed input. Recognized-but-unmodeled
functionality returns `Error::Unimplemented { feature }`.

## Unsafe / SIMD policy

`unsafe_code` is **forbidden** workspace-wide. Future SIMD or FFI work may introduce
`unsafe` only inside narrowly-scoped, individually-documented, individually-tested
modules, and only when justified by measurements. The `fuzz` crate sits outside the
workspace so that libFuzzer's `unsafe` runtime is not subject to this lint.
