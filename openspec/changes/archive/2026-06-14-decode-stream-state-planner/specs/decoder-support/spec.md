## ADDED Requirements

### Requirement: Parsed decode stream planner

The decoder support model SHALL provide a plan-only stream traversal API for
`DECODE-STREAM-STATE-PLANNER` in `splot-decode`. The planner SHALL be owned by
`DecodeContext`, consume already parsed `splot_core::stream::ParsedBitstream`
values plus caller-supplied input length, apply `DecodeOptions`, and return a
deterministic ordered plan for the selected base-layer stream. The planner
SHALL NOT accept raw bytes, read files, change `splot decode` CLI behavior,
decode tile payloads, reconstruct pixels, compute hashes, write Y4M, refresh
references, invoke AVM/dav2d, or add source/build/test/CI integration for
external decoders.

The initial planner SHALL select only the base minimal-tier layer: non-global
OBUs must have `obu_xlayer_id == 0`, `obu_tlayer_id == 0`, and
`obu_mlayer_id == 0`. It SHALL also enforce AV2 § 6.2.2 global/local xlayer
constraints for OBU types that require or forbid `GLOBAL_XLAYER_ID`. It SHALL
preserve AV2 bitstream/container order for raw Annex B and IVF-wrapped Annex B
parser output, including byte offsets and IVF frame context where present. It
SHALL treat `OBU_CLOSED_LOOP_KEY` as the only frame candidate in this slice,
and SHALL reject multistream/layer-selection structures, invalid xlayer scope,
non-base layers, unsupported frame-carrying OBUs, malformed parsed sources, and
resource-limit failures transactionally.

The planner SHALL enforce only the resource limits it can derive honestly from
the parsed stream model: `max_input_bytes` before planner traversal,
`max_obus` before adding the next planned OBU, `max_ivf_frame_records` before
traversing the next IVF frame record, and `max_frames_to_decode` before
accepting the next closed-loop-key frame candidate. A future raw-byte decode
planner SHALL add bounded pre-parse traversal and self-contained fuzz coverage
before it is marked supported.

#### Scenario: Raw Annex B is planned in order

- **WHEN** `DecodeContext::plan_stream` receives a parsed raw Annex B stream
  containing accepted base-layer OBUs
- **THEN** it returns a `DecodeStreamPlan` whose format is Annex B
- **AND** planned OBU records appear in original bitstream order with stable
  OBU indexes, byte offsets, sizes, headers, and roles
- **AND** no payload bytes, decoded frames, hashes, Y4M output, or reference
  updates are exposed as supported output

#### Scenario: IVF Annex B is planned with frame context

- **WHEN** `DecodeContext::plan_stream` receives a parsed IVF stream whose frame
  payloads contain accepted Annex B OBUs
- **THEN** it returns a `DecodeStreamPlan` whose format is IVF
- **AND** planned OBU records preserve source order, absolute byte offsets, IVF
  frame index, PTS, and frame payload offset metadata
- **AND** IVF warnings remain source metadata rather than decode success or
  external reference evidence

#### Scenario: Malformed parsed source is transactional

- **WHEN** raw Annex B parsing, IVF container parsing, or an IVF frame payload
  parse recorded an error in the supplied `ParsedBitstream`
- **THEN** `DecodeContext::plan_stream` returns a typed malformed-source error
- **AND** it returns no partial `DecodeStreamPlan`

#### Scenario: Planner resource limits are enforced

- **WHEN** the supplied input length exceeds `max_input_bytes`, traversed OBU
  count would exceed `max_obus`, traversed IVF frame records would exceed
  `max_ivf_frame_records`, or accepted closed-loop-key frame candidates would
  exceed `max_frames_to_decode`
- **THEN** `DecodeContext::plan_stream` returns a typed local limit error
- **AND** it returns no partial `DecodeStreamPlan`
- **AND** it does not emit the planned `decode/resource-limit` CLI diagnostic

#### Scenario: Unsupported structures are rejected

- **WHEN** the parsed stream contains an invalid global/local xlayer binding,
  non-base-layer OBU, MSDO, layer configuration record, atlas segment,
  operating point set, non-closed-loop frame-carrying OBU,
  metadata/output-effect OBU, reserved OBU, or another structure outside the
  minimal planner tier
- **THEN** `DecodeContext::plan_stream` returns a typed unsupported-structure
  error linked to rule id `decode/unsupported-feature`, matrix row
  `decode-stream-state`, Feature ID `DECODE-STREAM-STATE-PLANNER`, and the
  relevant AV2 section where applicable
- **AND** it returns no partial `DecodeStreamPlan`

#### Scenario: Planning is deterministic across thread policies

- **WHEN** the same parsed stream is planned through `DecodeContext` configured
  with `--threads 1`, `--threads auto`, and a fixed positive thread count
- **THEN** the returned plan metadata is identical
- **AND** planner implementation does not use direct Rayon, direct crossbeam,
  a global pool, ad-hoc codec worker threads, or unbounded queues

#### Scenario: CLI remains intentionally unsupported

- **WHEN** this planner API exists before runtime CLI decode support
- **THEN** `splot decode` continues to emit the existing
  `decode/unsupported-feature` diagnostic for valid invocations
- **AND** it does not read input, write output, invoke AVM/dav2d, or render the
  planner's local library errors as user-facing CLI diagnostics
