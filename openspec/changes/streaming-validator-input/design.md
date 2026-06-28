# Design: streaming-validator-input

## Context

`splot validate <file>` and `splot inspect <file>` read the whole input into a
`Vec<u8>` (`read_input` → `fs::read`) before any parsing. Peak memory is `O(file
size)`. For long streams, corpus sweeps, and many-file CI this is a scaling
hotspot, and unlike `decode` there is no upper bound, so a large or adversarial
file can OOM the process.

The validator's internals are already incremental (per-OBU state machine with an
EOF `finish()`, fully-owned diagnostics, and per-unit cursors in `splot-core`).
The only obstacle is `parse_bitstream_partial`, which eagerly collects **all**
OBUs into a `Vec<ObuEnvelope<'a>>` borrowing the whole input. This change adds a
streaming front-end that feeds the existing engine one temporal unit at a time
from a `std::io::Read`, bounding peak input memory to the largest unit.

The shape is the one every production media stack converges on (byte source →
demuxer → consumer; bounded transfer unit = one temporal unit; forward-only with
seek optional). It is adopted here as engineering precedent only; all AV2 framing
semantics continue to come from the spec mirror and AVM.

## Data model / API

Three tiers, each at its existing crate boundary (no new crate dependencies):

### Tier 1 — byte source: `R: std::io::Read`

No new type. `std::io::Read` is the `AVIOContext`/`Dav2dData`-source analog. The
reader is **forward-only — no `Seek` bound** — because both the leb128/IVF length
prefixes and the OBU parser are forward-only. `File`, `Cursor<&[u8]>`, a pipe, and
`stdin().lock()` all qualify, so streaming `validate` gains stdin support for free.

### Tier 2 — `splot-core`: `TemporalUnitReader<R: Read>` (`INFRA-STREAMING-TU-READER`)

A forward-only container demuxer that yields one temporal unit's bytes at a time:

- Buffer a bounded prefix and probe the container (`is_ivf` / `DKIF`), then dispatch
  to IVF or Annex-B framing.
- **IVF**: parse the 32-byte file header once; then per frame read the 12-byte
  frame header → `frame_size` → `read_exact(frame_size)` into a **reused** buffer.
- **Annex-B**: read `temporal_unit_size` (leb128) → `read_exact` into the reused
  buffer; parse OBUs inside the unit with the existing `AnnexBObuCursor`.
- Yields one unit at a time (e.g. `next_unit(&mut self) -> Result<Option<&[u8]>>`
  borrowing the reused buffer, or an owned-slice variant). Each unit is consumed
  (pushed) and dropped before the next refill — the **push-then-drop** discipline
  that keeps everything in safe Rust with one reused allocation.
- **Per-unit size cap** (`INFRA-STREAMING-TU-READER`): a declared unit size beyond
  the cap returns a typed error instead of allocating. This is where the
  size-guard the original review asked for lives. It is a **local
  `TemporalUnitReader` config** (a byte limit), mirroring the *kind* of guard
  `decode` already applies (`max_input_bytes` / `DecodeLimitError`) and AVM's
  `AVM_MAX_ALLOCABLE_MEMORY` — but `splot-core` MUST NOT depend on `splot-decode`,
  so this is a parallel local guard, not a shared type across that (forbidden)
  edge. If a single limit type is later wanted, it would live in `splot-core` for
  `splot-decode` to adopt downward — never an upward dependency.

Reuses the existing per-unit parse logic unchanged; the only genuinely new code is
the cross-read-boundary reassembly (accumulate bytes until a full leb128-length
OBU / full IVF frame is available, signalled by the length prefix).

### Tier 3 — `splot-validate`: `StreamingValidator` + `validate_reader` (`INFRA-VALIDATE-STREAMING-READER`)

The runner loop body (`validator/runner.rs:27`) — `observe_obu` + `run_checks` per
OBU, then `finish()` — **is** the engine already. This change exposes it as a
state machine and adds a reader-driven front-end:

```text
StreamingValidator {
    ctx: ValidatorContext,
    report: ValidationReport,
    // checks, options
}
  push_unit(&mut self, unit: &[u8])   // run the OBU cursor over `unit`;
                                       // observe_obu + run_checks per OBU
  finish(self) -> ValidationReport     // ctx.finish(...) then return report
```

- `validate_reader<R: Read>(r, options) -> ValidationReport` drives
  `TemporalUnitReader` → `push_unit` loop → `finish`.
- `validate_bytes(&[u8])` stays the stable in-memory API and is re-expressed over
  the **same** `StreamingValidator` engine (feeding OBUs from the in-memory path).
  Both front-ends differ only in *where OBUs come from*; the per-OBU loop body is
  unchanged, which is why unification does not risk the existing behavior.

Ownership: `ObuEnvelope<'a>` need only outlive a single `push_unit` call
(`observe_obu` borrows it and returns nothing borrowed), so each envelope can
borrow the still-resident reused buffer and be dropped before refill.

## Spec mapping

No new normative AV2 syntax or semantics. The reader frames units using existing,
already-modeled structures:

- leb128 length decoding — `AV2-4.11.6-LEB128` (`crates/splot-core/src/annexb.rs`).
- Annex-B temporal-unit / OBU envelope framing — `AV2-B-ANNEXB-OBU-ENVELOPE`.
- IVF container framing — `AV2-IVF-CONTAINER` (`crates/splot-core/src/ivf.rs`).

The validation *semantics* applied to each OBU are unchanged — this change only
alters how bytes are delivered to the existing checks.

## Diagnostics

No new `rule_id`s in v1. Streaming preserves the existing diagnostic set:

- Container/parse errors (truncation, oversize) surface through the existing typed
  `Error` paths and the same container diagnostics the in-memory runner emits.
- The per-unit size cap returns a typed reader `Error` (consumed by the CLI as an
  I/O-class failure), not a `Diagnostic` — consistent with how `decode`'s
  `DecodeLimitError` is handled, so it does not touch the diagnostic registry.

**Ordering obligation**: today a trailing structural/parse error is appended
*after* `finish()` (`runner.rs:36`). A streaming model encounters truncation
before EOF is known. The implementation MUST preserve the existing diagnostic
ordering so reports stay byte-identical; if ordering genuinely cannot be
preserved, snapshot re-baselining becomes an explicit, reviewed step (never a
silent change). This is the headline correctness risk and is covered by the
equivalence test below.

## Tests

- **Golden equivalence (headline)**: for every existing fixture, assert
  `validate_reader(Cursor::new(bytes))` produces a `ValidationReport` byte-identical
  to `validate_bytes(bytes)` — same diagnostics, order, and offsets.
- **Cross-boundary reassembly**: a `Read` adapter that returns one byte per call,
  asserting identical results to the whole-buffer path (stresses the reassembly
  buffer).
- **Positive**: IVF and Annex-B streams validated end to end through a reader.
- **Negative/EOF**: unit truncated mid-stream; empty input; EOF between units.
- **Size cap**: a declared unit size over the cap returns a typed error, allocates
  nothing oversized, and never panics.
- **Bounded memory**: drive a stream of many small units far exceeding one unit's
  size and assert the reader's buffer high-water mark stays bounded by the largest
  unit (e.g. via an instrumented `Read` / capacity assertion).
- **CLI**: `splot validate -` (stdin) and `splot validate <file>` agree; update the
  `validate --help` snapshot only if the surface changes.

## Alternatives considered

- **Generic pluggable demuxer framework (dav2d/FFmpeg-style trait + `Packet`
  type).** Rejected for now: it adds a `Demuxer` trait, a transfer type, and an
  indirection layer over a seam the crate boundary already enforces, for two
  container formats and a forward-only parser. Violates "no flexibility not needed
  yet." Reachable later as a cheap, motivated refactor under the rule of three;
  this design forecloses nothing (the concrete `TemporalUnitReader` is public and
  reusable by `decode`/`inspect` if that day comes).
- **`memmap2`-backed whole-file `&[u8]`.** Rejected: not streaming, requires a
  third-party dependency + crate-graph change (§10 sign-off), and carries a
  SIGBUS-on-truncation hazard.
- **`read_to_end` convenience wrapper.** Rejected: a `validate_reader` that just
  buffers the whole stream gives the signature but none of the memory benefit —
  it does not address the stated problem.

## Risks

- **Spec ambiguity**: none — no new normative behavior; framing structures already
  modeled.
- **Performance**: a reused buffer plus `read_exact` per unit should match or beat
  one big `fs::read` (smaller working set); verify no per-unit syscall regression
  for typical files. Bounded memory is the explicit win.
- **Compatibility**: `validate_bytes` and all current diagnostics/exit codes are
  preserved; the equivalence test is the guard. Diagnostic *ordering* is the one
  thing that could drift (see Diagnostics).
- **Maintenance**: one engine, two front-ends reduces duplication; the new surface
  is a single concrete reader plus a thin wrapper.

## Open questions

- **Feature-ID granularity**: two IDs (`INFRA-STREAMING-TU-READER` for the core
  reader, `INFRA-VALIDATE-STREAMING-READER` for the validator entry) map cleanly to
  the two reusable units and the PR split. They could instead collapse into a
  single umbrella row if the maintainer prefers fewer matrix rows. Default: two.
- **`inspect` follow-on**: once the reader exists, migrating `inspect` to it is a
  separate change; `collect_obus` / `--json` would need a streaming or
  bounded-accumulation strategy.
