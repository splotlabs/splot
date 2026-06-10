# Architecture

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

- **`splot-core`** — the AV2 spec modeled in Rust: strong types (`ObuType`, layer
  ids, `ByteOffset`/`BitOffset`), the bit reader, and panic-free parsers for the
  § 4.11 descriptors, OBU headers (§ 5.2.2), Annex B envelopes, payload dispatch,
  and the implemented § 5 payloads (sequence header, HLS OBUs, metadata/padding,
  quantizer matrix, film grain, the frame-header subset — see the generated
  [SPEC-COVERAGE.md](./SPEC-COVERAGE.md) for the live list). It owns the typed
  `Error` model. No I/O, no other `splot-*` dependency.
- **`splot-validate`** — parser output → user-facing conformance diagnostics. A
  `Validator` parses with `splot-core`, then runs a registry of `Check`s. Each
  `Diagnostic` is structured data (rule id, severity, spec section, offset, message).
  A malformed bitstream is a report, never a process failure.
- **`splot-encode`** — the *shape* of the future encoder API (configuration plus a
  push/pull `Context`). It implements no encoding; every operation returns
  `Error::Unimplemented`.
- **`splot-cli`** — the thin `splot` binary. It parses arguments (clap), initializes
  logging (tracing), reads/writes files, and calls library APIs. No codec logic.
- **`xtask`** — project automation: the `ci` pipeline; the repository checks
  (`check-license-headers`, `check-dependency-direction`, `check-spec-mirror`,
  `check-feature-status`, `check-diagnostic-registry`,
  `check-conventional-commits`/`-title`); matrix reporting (`feature-status`,
  `spec-coverage`); audit scoping (`audit-scope`); `audit`, `coverage`, and
  `fuzz` wrappers; plus codegen/vector/conformance stubs.

## Reference-informed encoder architecture

Future `splot-encode` work should combine:

- rav1e-style Rust API, RDO, tile/plane-region, fuzzing, and profiling discipline;
- SVT-AV1-style production pipeline, resource ownership, mode-decision, ME, RC, and filter-search
  architecture;
- AV2-spec and AVM-derived syntax, semantics, reconstruction, and conformance behavior.

Before implementing encoder features, read `docs/references/ENCODER-RESEARCH-NOTES.md` and find or
create the matrix row for any syntax/reconstruction behavior (see [AGENTS.md](../AGENTS.md) § 1a).

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
