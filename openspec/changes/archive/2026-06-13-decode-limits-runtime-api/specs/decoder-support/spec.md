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

#### Scenario: Contract cites spec-derived values

- **WHEN** a reader checks the decode limits contract
- **THEN** it cites AV2 § 4.11.6, Annex B.2-B.3, and § 5.2.1 for input and OBU
  byte surfaces; § 6.4.1, § 5.18.4.1, § 6.17.4.1, § 5.18.4.4, and § 6.17.4.4
  for sequence, frame, and output geometry; § 5.18.7.2, § 6.17.7.2, § 5.19,
  § 6.18, § 5.20.1, and § 6.19.1 for tile counts and payload traversal; § 7.1
  and § 7.21 for decoded output arrays; and § 6.4.6 and § 7.23 for reference
  slot and reference store surfaces
- **AND** it distinguishes those spec-derived measured values from the
  repository-owned limit thresholds

#### Scenario: Runtime policy API exists

- **WHEN** `splot-decode` is tested
- **THEN** it exposes dependency-free `DecodeOptions`, `DecodeLimits`, typed
  limit-name and unit types, finite defaults, inclusive limit comparison
  helpers, checked arithmetic helpers, allocation-size handoff checks, and local
  typed errors
- **AND** those types do not require `serde`, new dependencies, `splot-cli`, AVM,
  dav2d, or any byte-consuming decode entry point

#### Scenario: Runtime defaults are finite policy

- **WHEN** a caller constructs default decode options or default decode limits
- **THEN** the runtime API returns finite nonzero thresholds suitable for CI and
  fuzzing rather than AV2 normative conformance limits
- **AND** tests pin the default thresholds so policy changes are explicit

#### Scenario: Allocation is gated by limits

- **WHEN** a future byte-consuming decode planner accepts bytes or traverses OBUs
- **THEN** the planner MUST check `max_input_bytes` before buffering or
  accepting input bytes
- **AND** the planner MUST check `max_obus` before continuing OBU traversal or
  accumulating OBU state

#### Scenario: Derived sizes use checked arithmetic

- **WHEN** a future byte-consuming decode planner derives dimensions, strides,
  tile products, plane sizes, decoded frame bytes, reference-store bytes,
  output bytes, frame counts, or output frame counts from input
- **THEN** it MUST compute the derived `actual` value with checked arithmetic
  before comparing against `DecodeLimits` or allocating
- **AND** arithmetic overflow during derivation MUST be represented by the local
  runtime limit error API before any future diagnostic adaptation

#### Scenario: Derived allocations are gated by limits

- **WHEN** a future byte-consuming decode planner derives dimensions, tile
  counts, output frame counts, reference slot counts, reference-store byte
  sizes, or decoded/output byte sizes from input
- **THEN** the planner MUST check the relevant `DecodeLimits` value before
  allocating, indexing, traversing tile payloads, storing a reference frame,
  producing Y4M, or producing a deterministic frame hash

#### Scenario: Runtime API does not emit diagnostics

- **WHEN** the runtime limit helper rejects a value or reports arithmetic
  overflow
- **THEN** it returns a local typed error and does not emit
  `decode/resource-limit`
- **AND** the emitted decoder diagnostic registry remains unchanged until a
  future byte-consuming decode path maps local helper errors into structured
  diagnostics
