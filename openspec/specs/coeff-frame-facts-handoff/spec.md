# coeff-frame-facts-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-frame-facts-handoff`.

## Requirements
### Requirement: Coefficient frame-facts handoff

The decoder SHALL provide a crate-private loaded-but-unwired coefficient wrapper
for Feature ID `DECODE-COEFF-FRAME-FACTS-HANDOFF` that derives frame and
sequence facts for the staged coefficient branch before delegating to the
existing base-q `useFsc` handoff.

#### Scenario: all-zero bypasses frame facts

- **WHEN** the wrapper receives decoded `all_zero == 1`
- **THEN** it delegates to the existing ordinary all-zero selector path
- **AND** it does not require or evaluate frame facts, segment id, shared
  nonzero facts, ordinary-only facts, or FSC-only facts.

#### Scenario: nonzero ordinary path derives frame facts before delegation

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the shared nonzero facts select the ordinary branch
- **THEN** it derives shared `enable_fsc`, ordinary `enable_chroma_dctonly`,
  ordinary `reduced_tx_set`, nonzero `lossless` from
  `LosslessArray[segmentId]`, and `base_q_idx` from the frame-facts packet
- **AND** it delegates to the existing base-q wrapper with the same result, tile
  CDF state, coefficient context state, consumed bits, and symbol count as an
  explicit base-q input carrying those facts.

#### Scenario: nonzero FSC path derives frame facts before delegation

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the shared nonzero facts select the FSC branch
- **THEN** it derives shared `enable_fsc` and `base_q_idx` from the frame-facts
  packet and derives lossless/ordinary facts before lower input construction
- **AND** it delegates to the existing base-q wrapper with the same result, tile
  CDF state, coefficient context state, consumed bits, and symbol count as an
  explicit base-q input carrying those facts.

#### Scenario: invalid segment id is fail-atomic

- **WHEN** the wrapper receives a nonzero input with `segmentId` outside the
  frame-facts `LosslessArray[]` domain
- **THEN** it returns a typed coefficient branch error
- **AND** it does not mutate coefficient context state, tile CDF state, consumed
  bits, or symbol count.

#### Scenario: parser facts are available to future runtime coeffs

- **WHEN** tile-payload frame facts are derived from a complete intra
  `FrameHeaderCore` plus the active sequence transform/quant/entropy config
- **THEN** the derived tile work unit carries `enable_fsc`,
  `enable_chroma_dctonly`, `reduced_tx_set`, `LosslessArray[]`, and
  `base_q_idx` as crate-private facts
- **AND** the minimal runtime block-symbol trace remains unchanged.

#### Scenario: runtime scope remains unchanged

- **WHEN** focused staged coefficient tests and the minimal runtime tests run
- **THEN** decode output remains unchanged
- **AND** runtime `coeffs()` integration, full `compute_tx_type`, runtime block
  syntax traversal, dequantization, inverse transform, residual add,
  reconstruction, output, reference refresh, and full decoder conformance remain
  unsupported.
