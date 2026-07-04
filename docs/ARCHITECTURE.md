# Architecture

This document covers the workspace split, the dependency rules, the error model,
and the unsafe/SIMD policy; the Repository Boundaries and Coding Standards
sections of [AGENTS.md](../AGENTS.md) remain canonical for the rules summarized
here.

## Crate dependency graph

```text
splot-cli ───────┬──> splot-validate ───> splot-core
                 ├──> splot-decode   ───> splot-core, splot-parallel, splot-recon
                 ├──> splot-encode   ───> splot-core, splot-parallel, splot-recon, splot-tables
                 ├──> splot-parallel
                 └──> splot-core

splot-decode owns the current unsupported diagnostic API, decode runtime
context, byte/parsed stream planning over splot-core output, pipeline
orchestration, reference/filter/output routing, and narrow supported-tier
hash/raw/Y4M output. Its splot-recon dependency is limited to decoded-frame,
reconstruction, reference-store, hash, raw, and Y4M handoff code.

splot-parallel owns the approved concurrency primitives (local Rayon worker
pool + bounded crossbeam queues) and depends on no splot-* crate.
splot-recon depends only on splot-tables (the shared § 9 transform kernels and
quantizer matrix). splot-tables holds the shared generated AV2 § 9 transform-kernel
and quantizer-matrix tables and depends on no splot-* crate and no external crate.
xtask is standalone automation.
fuzz lives outside the workspace and depends on splot-core only.
```

**One-way dependency rule** (canonical: [AGENTS.md](../AGENTS.md) Repository
Boundaries; enforced by `cargo xtask check-dependency-direction`):

- `splot-core` depends on no other `splot-*` crate.
- `splot-parallel` depends on no other `splot-*` crate.
- `splot-tables` depends on no other `splot-*` crate (and no external crate); it
  holds only generated AV2 § 9 spec tables and may be depended on by any crate.
- `splot-recon` depends only on `splot-tables`.
- `splot-decode` depends only on `splot-core`, `splot-parallel`, and
  `splot-recon`; the `splot-recon` edge is limited to runtime
  decode/reconstruction/hash/Y4M output handoff code.
- `splot-validate` depends only on `splot-core`.
- `splot-encode` depends only on `splot-core`, `splot-parallel`, `splot-recon`,
  and `splot-tables`; the `splot-recon` edge is limited to borrowed encoder
  input views plus private lower-level reconstruction-boundary preparation until
  later encoder phases add closed-loop reconstruction APIs.
- `splot-cli` depends only on `splot-core`, `splot-decode`, `splot-parallel`,
  `splot-validate`, and `splot-encode`.
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
- **`splot-parallel`** — the only crate allowed to depend on `rayon` and
  `crossbeam-channel`. It owns the approved concurrency primitives: a local Rayon
  `WorkerPool` (one per encode/decode context, never the global pool or
  `build_global`), bounded `crossbeam-channel` queues (never unbounded), and the
  `ThreadCount` policy (`auto` default, fixed positive integer, `0` aliases to
  `auto`). It depends on no other `splot-*` crate. `splot-core` and
  `splot-validate` stay concurrency-runtime-free. See
  [CONCURRENCY.md](./CONCURRENCY.md).
- **`splot-tables`** — a dependency-free crate holding the shared generated AV2
  § 9 tables that `splot-recon` consumes without depending on `splot-core`: the
  § 9.6 `transform_1d` and § 9.7 `secondary_transform` transform kernels (for the
  § 7.15 inverse transform) and the § 9.4 `quantizer` matrix (for the § 7.14.4
  dequantization). The tables are generated verbatim by
  `cargo xtask gen-tables` (drift-checked); the crate is never hand-edited and
  depends on no other `splot-*` crate and no external crate. Every other § 9
  table stays in `splot-core::tables`.
- **`splot-validate`** — parser output → user-facing conformance diagnostics. A
  `Validator` parses raw Annex B or IVF-wrapped Annex B with `splot-core`, then
  runs a registry of `Check`s. Each `Diagnostic` is structured data (rule id,
  severity, spec section, offset, message). A malformed bitstream or container is
  a report, never a process failure.
- **`splot-encode`** — the *shape* of the future encoder API (configuration,
  borrowed frame input views, explicit retained input sharing, and a push/pull
  `Context`). It implements a typed no-output lifecycle state machine, but no
  coded packet production or successful encode path. Its direct `splot-recon`
  dependency currently supports validated borrowed input views; closed-loop
  reconstruction integration remains a future encoder phase.
- **`splot-cli`** — the thin `splot` binary. It parses arguments (clap),
  initializes logging (tracing), reads/writes files, and calls library APIs. No
  codec logic: the `inspect`/`validate` text and JSON rendering in
  `crates/splot-cli/src/commands/` are presentation over `splot-core` and
  `splot-validate` output, and `decode` renders the current `splot-decode`
  unsupported diagnostic. Exit codes are part of the contract: `0` clean; `1`
  findings (`validate`: validation errors, or warnings under `--strict`;
  `inspect`: a parse error; `decode`: unsupported diagnostic); `2`
  operational error.
- **`splot-recon`** — scaffold for future decoded frame buffers, planes,
  deterministic decoded-frame hashes, reconstruction primitives, and
  reference-frame storage shared by decoder and encoder roundtrip work. Its
  ownership model is view-first: owned `Plane`/`DecodedFrame`/workspace storage
  hands out borrowed `PlaneRef`/`PlaneMut`/`FrameRef`/`FrameMut` views without
  copying, immutable frames are shared without copying pixels via `SharedFrame`,
  and no media-storage type implements `Clone` (see the zero-copy ownership model
  below and [ZERO_COPY.md](./ZERO_COPY.md)). It intentionally exposes no runtime
  reconstruction API yet and depends only on `splot-tables` (the shared § 9
  transform kernels used by the § 7.15.2.1 inverse transform).
- **`splot-decode`** — the decode driver boundary. It owns structured
  `decode/*` diagnostics, `DecodeRuntimeConfig` / `DecodeContext`, byte and
  parsed stream planners, frame-level pipeline orchestration, decode-local
  prediction/residual/reference/filter ordering, and hash/raw/Y4M output routing
  over decoded frames. The current support tier remains narrow; broad AV2
  playback, film-grain output, and complete random-access behavior remain
  unsupported. Decoder module ownership is documented in
  [DECODER-ARCHITECTURE.md](./DECODER-ARCHITECTURE.md).
- **`xtask`** — project automation: the `ci` pipeline; the repository checks
  (`check-license-headers`, `check-dependency-direction`, `check-spec-mirror`,
  `check-feature-status`, `check-diagnostic-registry`,
  `check-conventional-commits`/`-title`); matrix reporting (`feature-status`,
  `spec-coverage`); audit scoping (`audit-scope`); `audit`, `coverage`, and
  `fuzz` wrappers; `gen-tables` (code-generates the AV2 § 9 tables into
  `crates/splot-core/src/tables/` from the committed `all_tables.h` attachment,
  with a `--check` drift gate run in `cargo xtask ci`); plus the `fetch-vectors`
  and `conformance` stubs.

## Reference-informed encoder architecture

rav1e and SVT-AV1 are engineering references only, never sources of AV2 syntax,
code, tables, or constants. The canonical gate before any encoder work is the
Encoder Reference Gate in [AGENTS.md](../AGENTS.md); the research notes live under
[docs/references/](./references/).

## Error model

(Canonical: [AGENTS.md](../AGENTS.md) Coding Standards.)

Libraries use typed errors (`thiserror`); `anyhow` is confined to `splot-cli` and
`xtask`. Library code never panics on malformed input. Recognized-but-unmodeled
functionality returns `Error::Unimplemented { feature }`.

## Unsafe / SIMD policy

`unsafe_code` is **forbidden** workspace-wide. Future SIMD or FFI work may introduce
`unsafe` only inside narrowly-scoped, individually-documented, individually-tested
modules, and only when justified by measurements. The `fuzz` crate sits outside the
workspace so that libFuzzer's `unsafe` runtime is not subject to this lint.

## Concurrency runtime

Rayon (via a local owned `WorkerPool`) and `crossbeam-channel` (bounded queues
only) are the **only** approved concurrency-runtime primitives, and only
`splot-parallel` may depend on them. `splot-core` stays runtime-free so the spec
model never carries a scheduler. Competing runtimes (tokio, async-std, futures,
threadpool, …), the Rayon global pool, and unbounded channels are banned. The
full policy is [CONCURRENCY.md](./CONCURRENCY.md) and it is enforced by
`cargo xtask check-concurrency-policy` (run in `cargo xtask ci`).

## Zero-copy ownership model

Media buffers (frames, planes, reference-frame storage, lookahead, pixel/sample
storage) are owned view-first and never duplicated implicitly. The default is to
borrow: algorithms take `PlaneRef`/`PlaneMut`/`FrameRef`/`FrameMut` views over
existing storage; immutable frames are shared without copying pixels through an
explicit `SharedFrame` (`Arc`-backed, `.share()` only); reference stores move or
share handles and never require `F: Clone`; and no frame/plane/workspace/sample
type implements `Clone`. Every genuine duplication is an explicit, marked
materialization boundary (`splot-copy-ok: <reason>`), not a generic clone.

`zerocopy` is a separate, narrow tool for **private fixed-layout byte/wire view
structs** (e.g. IVF container headers) — it is not the frame-buffer ownership
model. **Dependency direction:** `zerocopy` may be a direct dependency only of
`splot-core` (and `splot-recon` with a documented raw-sample view); never
`splot-decode`/`splot-encode`/`splot-validate`/`splot-cli`/`splot-parallel`. It
never appears in public APIs and never parses AV2 bit-level/entropy/variable-length
syntax. It is in use today for the IVF container header (`splot-core` `ivf.rs`),
added only for that real fixed-layout use site and never unused (see
[docs/references/THIRD-PARTY-NOTICES.md](./references/THIRD-PARTY-NOTICES.md) §12).

This is non-normative codec-runtime infrastructure (no AV2 conformance coverage).
The full policy is [ZERO_COPY.md](./ZERO_COPY.md); it is enforced by `cargo xtask
check-zero-copy-policy` (run in `cargo xtask ci`) together with the view/share
APIs in `splot-recon`, and it composes with the concurrency model (disjoint
mutable views for parallel writes; see [CONCURRENCY.md](./CONCURRENCY.md)).

See [CODE_REVIEW.md](./CODE_REVIEW.md) for the review checklist and
[TESTING.md](./TESTING.md) for the test layers.
