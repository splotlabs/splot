## ADDED Requirements

### Requirement: Byte-consuming decode stream planner

The decoder support system SHALL provide a source-backed byte-consuming stream
planning entrypoint tracked by Feature ID `DECODE-BYTE-STREAM-PLANNER`. The
entrypoint SHALL accept raw AV2 Annex B length-delimited bytes and IVF/DKIF
bytes whose frame payloads contain Annex B bytes, SHALL return the existing
`DecodeStreamPlan` type, and SHALL preserve the `decode-stream-state` base-layer
selection policy. It SHALL not reconstruct pixels, decode tile payloads, compute
hashes, write Y4M, invoke external decoders, or change `splot decode` CLI
success behavior.

#### Scenario: Raw Annex B bytes produce the same bounded plan

- **WHEN** the byte-consuming planner receives a complete raw Annex B input that
  contains only structures accepted by `decode-stream-state`
- **THEN** it returns a `DecodeStreamPlan` with source format `annex_b`
- **AND** planned OBUs retain source order, byte offsets, declared OBU size,
  payload length, parsed OBU header metadata, and planner roles
- **AND** the plan is equivalent to planning the same bytes through the existing
  parsed-input planner for representative self-contained tests

#### Scenario: IVF bytes preserve frame context

- **WHEN** the byte-consuming planner receives a complete IVF/DKIF input whose
  frame payloads are accepted Annex B payloads
- **THEN** it returns a `DecodeStreamPlan` with source format `ivf`
- **AND** each planned OBU from an IVF frame records the IVF frame index, frame
  header offset, frame payload offset, declared frame payload size, and PTS
  metadata
- **AND** IVF timestamps are preserved only as container metadata and are not
  used for output scheduling or media-player behavior

#### Scenario: Limits are enforced during byte traversal

- **WHEN** the byte-consuming planner receives configured `DecodeLimits`
- **THEN** it checks `max_input_bytes` before traversing bytes
- **AND** it checks `max_obus` before retaining the next OBU
- **AND** it checks `max_ivf_frame_records` before processing the next IVF frame
  record
- **AND** it checks `max_frames_to_decode` before retaining the next accepted
  frame candidate
- **AND** a limit failure returns a typed `DecodeError::Limit` and no partial
  plan

#### Scenario: Malformed bytes are transactional

- **WHEN** raw Annex B bytes, IVF container bytes, or Annex B bytes inside an IVF
  frame payload are malformed
- **THEN** the byte-consuming planner returns `DecodeError::MalformedSource`
- **AND** the error records a `DecodeSourceIssue` with the source category,
  offset when known, IVF frame index when frame-local, and parser message
- **AND** no partial plan is returned

#### Scenario: Unsupported structures stay structured and CLI-neutral

- **WHEN** the byte-consuming planner encounters a non-base layer, invalid
  global/local layer scope, unsupported multistream/external-HLS structure,
  reserved OBU type, non-CLK frame-carrying OBU, or output-affecting OBU outside
  the initial planner tier
- **THEN** it returns `DecodeError::UnsupportedStructure`
- **AND** the unsupported metadata uses rule id `decode/unsupported-feature`,
  matrix row `decode-stream-state`, and Feature ID
  `DECODE-STREAM-STATE-PLANNER`
- **AND** the `splot decode` CLI remains intentionally unsupported until a
  later change adds a diagnostic adapter and input-read policy

#### Scenario: Byte planner is fuzzed without external decoders

- **WHEN** the repository fuzz smoke is run for decoder entrypoints
- **THEN** the byte-consuming stream planner has a finite-limit fuzz target that
  feeds arbitrary bytes through `DecodeContext::plan_bytes`
- **AND** the target does not require AVM, dav2d, network access, generated
  external fixtures, or checked-in local reference paths

#### Scenario: Runtime concurrency model is preserved

- **WHEN** callers use the byte-consuming planner with thread count `1`, `auto`,
  or a fixed non-zero count
- **THEN** planning executes inside `DecodeContext`'s single owned
  `splot_parallel::WorkerPool`
- **AND** plan records, errors, and source issue ordering are deterministic
  across thread counts
- **AND** no direct Rayon, crossbeam, global worker pool, ad-hoc worker thread,
  or decode pipeline queue is introduced
