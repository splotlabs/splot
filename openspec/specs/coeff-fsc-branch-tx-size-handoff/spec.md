# coeff-fsc-branch-tx-size-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-fsc-branch-tx-size-handoff`.

## Requirements
### Requirement: FSC Branch Tx-Size Handoff

The decoder SHALL provide a crate-private loaded-but-unwired FSC/IDTX
coefficient branch handoff for Feature ID
`DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF` that derives tx-size-dependent
nonzero FSC branch facts from generated AV2 § 9.2 conversion tables before
delegating to the existing FSC scan-order branch.

#### Scenario: derive matching FSC facts

- **WHEN** a nonzero luma FSC branch is supplied a block geometry matching
  `Tx_Width[txSz] >> 2` / `Tx_Height[txSz] >> 2`, a valid `txSz`,
  caller-resolved `PlaneTxType`, `is_inter`, and `coeff_cdf_q_ctx`
- **THEN** the wrapper derives `NonZeroCoeffEobContextInput`,
  `CoeffFscLevelPassConfig`, `CoeffFscContextCommitConfig`, and scan order from
  those facts
- **AND** the decoded FSC branch result, tile CDF state, coefficient context
  state, consumed bits, and symbol count match the existing explicit scan-order
  wrapper supplied with the same derived facts.

#### Scenario: reject invalid facts before mutation

- **WHEN** the wrapper receives all-zero routing, non-luma routing, an invalid
  generated transform-size table value, or block geometry inconsistent with
  `txSz`
- **THEN** it returns a typed error before unintended tile CDF, coefficient
  context, or symbol-decoder mutation.
