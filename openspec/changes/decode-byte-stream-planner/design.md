## Context

`splot-decode` currently has a parsed-input stream planner:
`DecodeContext::plan_stream(DecodeStreamInput, DecodeOptions)`. It consumes a
`splot_core::stream::ParsedBitstream` that has already been materialized by
`splot-core`, applies `DecodeLimits` to the supplied input length, OBU count,
IVF frame records, and selected frame candidates, and rejects unsupported
structures transactionally.

The missing decoder-mission step is the first byte-consuming decode-side API.
Calling `splot_core::stream::parse_bitstream_partial()` and then
`plan_stream()` is not sufficient for this new surface because the parser
materializes vectors before `splot-decode` can reject many tiny OBUs or IVF
frame records. This change therefore adds single-step Annex B and IVF frame
cursor primitives in `splot-core`, then drives them from `splot-decode` with
decode limits between steps. The parser logic remains single-sourced in
`splot-core`, and `splot-decode` does not copy payloads or add dependencies.

The relevant AV2 citations are Annex B § B.2 for length-delimited bitstreams,
§ 5.2.1 for `open_bitstream_unit`, § 5.2.2 for OBU headers, § 6.2.2 for layer
scope, and § 7.1 for the decoding-process context. IVF/DKIF remains a
non-normative container handled by `splot-core`'s IVF module, not by AV2.

## Goals / Non-Goals

**Goals:**

- Add `DecodeContext::plan_bytes(&[u8], DecodeOptions) -> Result<DecodeStreamPlan>`.
- Detect raw Annex B versus IVF/DKIF bytes exactly as the existing
  `splot-core` stream parser does.
- Enforce `max_input_bytes` before traversal and enforce `max_obus`,
  `max_ivf_frame_records`, and `max_frames_to_decode` before retaining the next
  matching plan record.
- Keep output deterministic across thread counts by executing the whole planner
  inside `DecodeContext`'s context-owned `WorkerPool`.
- Add unit/property/fuzz coverage for malformed byte inputs and configured
  limits.

**Non-Goals:**

- No CLI success behavior and no change to the existing CLI unsupported
  diagnostic.
- No reconstruction, symbol/CDF decode, tile payload decode, frame hash digest,
  AV2 metadata hash verification, Y4M output, output ordering, or reference
  refresh.
- No `splot-recon` dependency and no new third-party dependency.
- No AVM/dav2d source, wrapper, build step, CI use, local path, or fixture
  expectation.

## Decisions

1. Add bounded byte planning driven by `splot-core` cursors.

   `splot-core` exposes `AnnexBObuCursor` and `IvfFrameCursor` so callers can
   consume one parsed OBU envelope or IVF frame record at a time without
   materializing the full source up front. `splot-decode` checks
   `DecodeLimits` before asking those cursors for the next retained OBU or
   complete IVF frame record, then passes each resulting `ObuEnvelope` through
   the existing `PlanBuilder`. This avoids using the vector-producing
   `parse_bitstream_partial()` path for the raw decode-side API while keeping
   structural parser logic single-sourced in `splot-core`.

2. Keep one plan type and one classification path.

   `plan_bytes` returns the same `DecodeStreamPlan` type and reuses the same
   unsupported-structure classifier as `plan_stream`. This avoids creating a
   separate semantic path for raw bytes and preserves the already reviewed
   base-layer-only behavior.

3. Keep concurrency explicit and deterministic.

   The public method lives on `DecodeContext` and executes inside
   `self.pool.install(...)`. The walker remains serial in this slice. Future
   parallelism must use `splot_parallel::prelude::*` inside that worker-pool
   scope and must collect/commit records in source order.

4. Treat all byte parse failures as transactional planner errors.

   The validator keeps parseable prefixes for diagnostics, but this byte
   planner either returns a complete plan or no plan. Annex B parse errors, IVF
   container errors, and Annex B errors inside IVF frame payloads become
   `DecodeError::MalformedSource`.

5. Do not call `payload_status()`.

   This slice only needs envelope/header/layer metadata. Payload parsing belongs
   to later symbol/tile/tier work and will require more limits and diagnostics.

## Risks / Trade-offs

- Exposing cursor primitives expands `splot-core`'s parser API surface -> keep
  the API structural and allocation-free, reuse it from the existing partial
  parsers, and cover behavior against the existing parsed-input planner for
  small valid inputs.
- IVF with many empty frame records can be adversarial -> enforce
  `max_ivf_frame_records` before processing each frame payload.
- Bounded parsing can diverge from `splot-core::stream` container detection ->
  keep detection to `splot_core::ivf::is_ivf(input)` and add unit tests that
  compare the plan to the parsed-input planner for representative raw and IVF
  inputs.
- `decode/resource-limit` remains planned, not emitted by CLI -> library errors
  keep typed `DecodeLimitError` and docs avoid claiming a user-facing diagnostic
  until the CLI diagnostic adapter lands.
