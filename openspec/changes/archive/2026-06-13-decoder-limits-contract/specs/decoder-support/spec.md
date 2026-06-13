## ADDED Requirements

### Requirement: Decode limits contract

The repository SHALL document a future `DecodeOptions { limits: DecodeLimits }`
contract before any `splot decode` path performs bitstream-derived allocation.
The contract SHALL treat limits as `splot` resource policy layered over
spec-derived values, not as AV2 conformance rules. The documented limits SHALL
cover input bytes, OBU count, decoded frame count, output frame count, frame
width, frame height, luma samples per frame, decoded frame bytes, reference
frames, tile count, tile payload bytes, and output bytes. Tracked by
`DOC-DECODE-LIMITS-CONTRACT`.

#### Scenario: Contract cites spec-derived values

- **WHEN** a reader checks the decode limits contract
- **THEN** it cites AV2 § 6.4.1 for sequence maximum frame dimensions,
  § 6.4.6 for reference-frame count, § 6.17.4.1 for per-frame dimensions,
  § 6.17.7.2 for tile grid counts, § 5.19 for tile group count derivation,
  § 5.20 for tile payload traversal, § 7.1 for the general decode input/output
  model, § 7.21 for decoded output arrays, and § 7.23 for reference frame
  storage
- **AND** it distinguishes those spec-derived values from the repository-owned
  limit thresholds

#### Scenario: Allocation is gated by limits

- **WHEN** a future byte-consuming decode planner accepts bytes or traverses OBUs
- **THEN** the planner MUST check `max_input_bytes` before buffering or
  accepting input bytes
- **AND** the planner MUST check `max_obus` before continuing OBU traversal or
  accumulating OBU state

#### Scenario: Derived sizes use checked arithmetic

- **WHEN** a future byte-consuming decode planner derives dimensions, strides,
  tile products, plane sizes, decoded frame bytes, reference-storage bytes,
  output bytes, frame counts, or output frame counts from input
- **THEN** it MUST compute the derived `actual` value with checked arithmetic
  before comparing against `DecodeLimits` or allocating
- **AND** arithmetic overflow during derivation MUST be treated as a
  `decode/resource-limit` failure

#### Scenario: Derived allocations are gated by limits

- **WHEN** a future byte-consuming decode planner derives dimensions, tile
  counts, output frame counts, reference frame counts, or decoded/output byte
  sizes from input
- **THEN** the planner MUST check the relevant `DecodeLimits` value before
  allocating, indexing, traversing tile payloads, storing a reference frame,
  producing Y4M, or producing a deterministic frame hash

### Requirement: Decode resource-limit diagnostic contract

The repository SHALL document `decode/resource-limit` as a planned decoder
diagnostic for future limit violations. Until source emits this diagnostic, it
SHALL NOT appear in the marker-delimited emitted decoder diagnostic registry.
When emitted, the diagnostic SHALL include the stable decoder diagnostic fields
`rule_id`, `severity`, `spec_section`, `matrix_row`, `feature_id`, `message`,
and `remediation`, plus resource fields `limit_name`, `limit`, `actual`, `unit`,
`byte_offset`, and `bit_offset`.

#### Scenario: Planned diagnostic stays out of emitted registry

- **WHEN** `cargo xtask check-diagnostic-registry` runs before
  `decode/resource-limit` is emitted by source
- **THEN** the decoder diagnostic registry contains only emitted `decode/*`
  rule IDs inside its enforced marker region
- **AND** the planned resource-limit diagnostic is documented outside that
  emitted registry region or in support/roadmap text

#### Scenario: Future limit violation reports measured value

- **WHEN** a future `splot decode` path rejects an input because a measured
  spec-derived value exceeds a `DecodeLimits` threshold
- **THEN** it emits `decode/resource-limit` with severity `Error`, matrix row
  `decode-limits-budget`, Feature ID `DOC-DECODE-LIMITS-CONTRACT`, the AV2
  section that supplied the measured value, the limit name, configured limit,
  measured actual value, unit, and any known byte/bit offset
