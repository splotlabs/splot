# coeff-parity-tcq-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-parity-tcq-handoff`.

## Requirements
### Requirement: Coefficient parity and TCQ handoff

The decoder SHALL provide a crate-private loaded-but-unwired coefficient
derivation for Feature ID `DECODE-COEFF-PARITY-TCQ-HANDOFF` that derives AV2
§ 5.20.7.27 `parityHiding` and `useTcq` from parsed frame flags and existing
block facts before delegating to the staged base-q `useFsc` handoff.

#### Scenario: all-zero bypasses parity and TCQ facts

- **WHEN** the wrapper receives decoded `all_zero == 1`
- **THEN** it delegates to the existing ordinary all-zero selector path
- **AND** it does not require or evaluate frame `allow_*` facts, segment id,
  shared nonzero facts, ordinary-only facts, or FSC-only facts.

#### Scenario: ordinary path derives parity hiding

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the shared nonzero facts select the ordinary branch
- **THEN** it derives `parity_hiding` as
  `allow_parity_hiding && !Lossless && plane == 0 && PlaneTxType != IDTX`
- **AND** it delegates to the existing base-q wrapper with the same result, tile
  CDF state, coefficient context state, consumed bits, and symbol count as an
  explicit base-q input carrying that derived value.

#### Scenario: ordinary path derives TCQ

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the shared nonzero facts select the ordinary branch
- **THEN** it derives `use_tcq` as
  `allow_tcq && plane == 0 && !Lossless && txClass == TX_CLASS_2D && !useFsc`
- **AND** it delegates to the existing base-q wrapper with the same result, tile
  CDF state, coefficient context state, consumed bits, and symbol count as an
  explicit base-q input carrying that derived value.

#### Scenario: FSC path suppresses lower TCQ

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the shared nonzero facts select the FSC branch
- **THEN** it derives `useFsc` before constructing lower ordinary facts
- **AND** it derives `use_tcq == false` for the lower base-q packet even when
  frame `allow_tcq == true`.

#### Scenario: lossless and chroma suppress parity and TCQ

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the derived block is lossless or the coefficient plane is chroma
- **THEN** it derives `parity_hiding == false`
- **AND** it derives `use_tcq == false`.

#### Scenario: parser allow flags are available to future runtime coeffs

- **WHEN** tile-payload frame facts are derived from a complete intra
  `FrameHeaderCore` plus the active sequence transform/quant/entropy config
- **THEN** the derived tile work unit carries `allow_tcq` and
  `allow_parity_hiding` as crate-private facts
- **AND** the minimal runtime block-symbol trace remains unchanged.

#### Scenario: invalid segment id remains fail-atomic

- **WHEN** the wrapper receives a nonzero input with `segmentId` outside the
  frame-facts `LosslessArray[]` domain
- **THEN** it returns a typed coefficient branch error
- **AND** it does not mutate coefficient context state, tile CDF state, consumed
  bits, or symbol count.

#### Scenario: runtime scope remains unchanged

- **WHEN** focused staged coefficient tests and the minimal runtime tests run
- **THEN** decode output remains unchanged
- **AND** runtime `coeffs()` integration, full `compute_tx_type`, runtime block
  syntax traversal, segment-map derivation, dequantization, inverse transform,
  residual add, reconstruction, output, reference refresh, and full decoder
  conformance remain unsupported.
