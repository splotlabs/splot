# coeff-runtime-frame-entry-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-runtime-frame-entry-handoff`.

## Requirements
### Requirement: Runtime coefficient frame-entry handoff

The decoder SHALL route the minimal runtime's traced all-zero coefficient block
applications through the staged frame-facts coefficient wrapper for Feature ID
`DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF`, while preserving the existing output
and keeping nonzero `coeffs()` unsupported.

#### Scenario: luma all-zero enters the frame-facts wrapper

- **WHEN** the minimal flat-intra runtime decodes the traced luma
  `all_zero == 1` coefficient block
- **THEN** it calls the top coefficient frame-facts wrapper with an all-zero
  `TX_64X64` geometry input
- **AND** it applies the same all-zero coefficient state and context-line
  updates as the previous ordinary-branch entry point.

#### Scenario: chroma V all-zero enters the frame-facts wrapper

- **WHEN** the minimal flat-intra runtime decodes the traced V-plane
  `all_zero == 1` coefficient block
- **THEN** it calls the top coefficient frame-facts wrapper with an all-zero
  `TX_16X16` geometry input
- **AND** it applies the same all-zero coefficient state and context-line
  updates as the previous ordinary-branch entry point.

#### Scenario: all-zero bypass remains independent of frame facts

- **WHEN** the frame-facts wrapper receives an all-zero runtime input
- **THEN** it does not require or evaluate segment id, lossless array,
  `allow_tcq`, `allow_parity_hiding`, ordinary-only facts, or FSC-only facts
- **AND** it preserves AV2 § 5.20.7.27 all-zero ordering.

#### Scenario: runtime output remains unchanged

- **WHEN** the minimal runtime hash/raw/Y4M and block-symbol frontier tests run
- **THEN** decoded output bytes and hash identity remain unchanged
- **AND** broad runtime nonzero `coeffs()`, full `compute_tx_type`, segment-map
  derivation, transform-block syntax traversal, dequantization, inverse
  transform, residual add, reference refresh, and full decoder conformance
  remain unsupported.

#### Scenario: failure remains transactional

- **WHEN** the traced block-symbol frontier fails from a symbol mismatch,
  block-symbol CDF error, coefficient-wrapper error, or `exit_symbol()` error
- **THEN** the work-unit tile CDF rows are restored to their pre-frontier state
- **AND** Saved and Frame CDF rows remain unchanged.
