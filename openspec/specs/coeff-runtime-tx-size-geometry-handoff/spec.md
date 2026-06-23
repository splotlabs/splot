# coeff-runtime-tx-size-geometry-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-runtime-tx-size-geometry-handoff`.

## Requirements
### Requirement: Runtime coefficient tx-size geometry handoff

The decoder SHALL derive the minimal runtime all-zero coefficient frame-entry
`txSz` input for Feature ID
`DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF` from traced transform geometry
and the generated AV2 section 9.2 transform-size width and height tables,
instead of from local hard-coded transform-size ordinals.

#### Scenario: luma geometry resolves through generated tables

- **WHEN** the minimal flat-intra runtime prepares the traced luma all-zero
  coefficient frame-entry input for AV2 section 5.20.7.27
- **THEN** it derives `txSz` from the traced 64x64 transform geometry by
  matching generated `Tx_Width` and `Tx_Height` table entries
- **AND** it enters the same all-zero coefficient frame-entry wrapper and
  applies the same all-zero context-state updates as before.

#### Scenario: chroma V geometry resolves through generated tables

- **WHEN** the minimal flat-intra runtime prepares the traced V-plane all-zero
  coefficient frame-entry input for AV2 section 5.20.7.27
- **THEN** it derives `txSz` from the traced 16x16 transform geometry by
  matching generated `Tx_Width` and `Tx_Height` table entries
- **AND** it enters the same all-zero coefficient frame-entry wrapper and
  applies the same all-zero context-state updates as before.

#### Scenario: unsupported geometry is rejected before consumption

- **WHEN** a traced all-zero coefficient frame-entry input asks for geometry
  that has no matching generated AV2 transform-size table entry
- **THEN** the runtime returns a typed local block-symbol trace error before
  entering the coefficient wrapper
- **AND** tile CDF rows, saved CDF rows, frame CDF rows, and symbol-decoder
  counters remain unchanged.

#### Scenario: runtime output remains unchanged

- **WHEN** the minimal runtime hash/raw/Y4M and block-symbol frontier tests run
- **THEN** decoded output bytes and hash identity remain unchanged
- **AND** broad runtime nonzero `coeffs()`, transform-block syntax traversal,
  dequantization, inverse transform, residual add, reference refresh, and full
  decoder conformance remain unsupported.
