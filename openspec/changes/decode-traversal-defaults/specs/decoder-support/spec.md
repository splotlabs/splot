## MODIFIED Requirements

### Requirement: Decode limits contract

The repository SHALL document and source-back a
`DecodeOptions { limits: DecodeLimits }` contract before any `splot decode` path
performs bitstream-derived allocation. The contract SHALL treat limits as
`splot` resource policy layered over spec-derived values, not as AV2 conformance
rules. The documented and source-backed limits SHALL cover input bytes, OBU
count, decoded frame count, output frame count, frame width, frame height, luma
samples per frame, decoded frame bytes, reference slots, reference store bytes,
tile count, tile payload bytes, and output bytes. Tracked by
`DECODE-LIMITS-RUNTIME-API`, while the older docs-only row
`DOC-DECODE-LIMITS-CONTRACT` remains the contract umbrella until byte-consuming
enforcement and diagnostics exist.

#### Scenario: Runtime defaults are finite policy

- **WHEN** a caller constructs default decode options or default decode limits
- **THEN** the runtime API returns finite nonzero thresholds suitable for CI,
  fuzzing, and current large-stream decoder-mission traversal rather than AV2
  normative conformance limits
- **AND** the default OBU and frame-count thresholds are large enough for the
  current `local-decoder-mission.ivf` target's inspected 12964 OBU stream to advance past the
  prior `max_frames_to_decode = 128` planner gate
- **AND** tests pin the default thresholds so policy changes are explicit

### Requirement: Byte-consuming decode stream planner

The decoder support system SHALL provide a source-backed byte-consuming stream
planning entrypoint tracked by Feature ID `DECODE-BYTE-STREAM-PLANNER`. The
entrypoint SHALL accept raw AV2 Annex B length-delimited bytes and IVF/DKIF
bytes whose frame payloads contain Annex B bytes, SHALL return the existing
`DecodeStreamPlan` type, and SHALL preserve the `decode-stream-state` base-layer
selection policy. It SHALL not reconstruct pixels, decode tile payloads, compute
hashes, write Y4M, invoke external decoders, or change `splot decode` CLI
success behavior.

#### Scenario: TIP frame candidates are planned but not decoded

- **WHEN** the byte-consuming planner receives a selected-layer
  `OBU_REGULAR_TIP`
- **THEN** it retains the OBU as a frame candidate for traversal and
  `max_frames_to_decode` accounting
- **AND** downstream runtime decode still rejects TIP before output because no
  TIP reconstruction, reference refresh, or output behavior is claimed
