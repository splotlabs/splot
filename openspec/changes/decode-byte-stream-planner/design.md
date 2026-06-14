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
frame records. This change therefore adds a bounded walker owned by
`splot-decode` that reuses public `splot-core` parsing primitives without
copying payloads or adding dependencies.

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

1. Add a bounded byte walker in `splot-decode`.

   The walker will parse one Annex B envelope at a time with
   `splot_core::leb128::read_leb128` and
   `splot_core::obu::read_obu_header_from_slice`, then pass the resulting
   `ObuEnvelope` through the existing `PlanBuilder`. For IVF, it will use
   `splot_core::ivf::parse_ivf_header`, `IVF_HEADER_SIZE`, and
   `IVF_FRAME_HEADER_SIZE` to walk frame records and parse each frame payload as
   bounded Annex B. This avoids using the unbounded vector-producing
   `parse_bitstream_partial()` path for the raw decode-side API.

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

- Bounded walking duplicates a small amount of envelope traversal logic already
  present in `splot-core` -> keep it scoped to `splot-decode` byte-planning
  needs, use public `splot-core` primitives, and cover behavior against the
  existing parsed-input planner for small valid inputs.
- IVF with many empty frame records can be adversarial -> enforce
  `max_ivf_frame_records` before processing each frame payload.
- Bounded parsing can diverge from `splot-core::stream` container detection ->
  keep detection to `splot_core::ivf::is_ivf(input)` and add unit tests that
  compare the plan to the parsed-input planner for representative raw and IVF
  inputs.
- `decode/resource-limit` remains planned, not emitted by CLI -> library errors
  keep typed `DecodeLimitError` and docs avoid claiming a user-facing diagnostic
  until the CLI diagnostic adapter lands.
