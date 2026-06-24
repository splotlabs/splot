# ac0ej3 Active Intra IST Handoff Specification

## Purpose

Track the ac0ej3 decoder mission boundary where active intra IST syntax is
consumed for Wiener NS LR tx-skip record derivation without claiming secondary
inverse-transform reconstruction.

## Requirements

### Requirement: ac0ej3 Active Intra IST Handoff

The decoder SHALL track `DECODE-AC0EJ3-ACTIVE-INTRA-IST-HANDOFF` as a partial
runtime prerequisite for the local ac0ej3 decode mission. For the ac0ej3 Wiener
NS LR transform-record path, when AV2 §5.20.7.29 requires intra IST
secondary-transform syntax for a covered intra luma DCT_DCT residual block, the
runtime SHALL consume `sec_tx_type` and, when non-zero, the required intra
`most_probable_stx_set` symbol in spec order. That active-IST admission SHALL be
limited to deriving LR tx-skip transform records and SHALL NOT imply secondary
inverse-transform support, decoded samples, raw/Y4M output, reference refresh,
or successful ac0ej3 decode.

#### Scenario: Active IST syntax is handed to LR tx-skip records

- **WHEN** the local ac0ej3 stream reaches Wiener NS LR transform-record
  derivation
- **AND** an intra luma DCT_DCT residual block decodes non-zero `sec_tx_type`
- **THEN** the runtime consumes the required intra `most_probable_stx_set`
  follow-up
- **AND** it may derive the LR tx-skip transform record from the parsed
  skip/EOB facts
- **AND** it records active-IST metadata without applying a secondary inverse
  transform

#### Scenario: Reconstruction paths remain fail-closed

- **WHEN** a residual path would use the luma coefficients for inverse
  transform, reconstructed samples, output, or reference refresh
- **AND** the block decodes non-zero `sec_tx_type`
- **THEN** the decoder emits a structured `decode/unsupported-feature`
  diagnostic for active intra IST
- **AND** it does not continue with pre-secondary-transform coefficients as if
  they were reconstructed-sample input

#### Scenario: No successful ac0ej3 decode claim

- **WHEN** active intra IST handoff has been implemented for LR tx-skip records
- **THEN** `DECODE-AC0EJ3-ACTIVE-INTRA-IST-HANDOFF` remains partial
- **AND** the decoder does not claim decoded frame samples, loop-restoration
  output, raw/Y4M output, reference refresh, AVM/dav2d byte equality, or broad
  AV2 secondary-transform support
