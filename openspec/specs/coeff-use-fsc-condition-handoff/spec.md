# coeff-use-fsc-condition-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-use-fsc-condition-handoff`.

## Requirements
### Requirement: useFsc condition handoff

The decoder SHALL provide a crate-private loaded-but-unwired coefficient wrapper
for Feature ID `DECODE-COEFF-USE-FSC-CONDITION-HANDOFF` that derives the AV2
section 5.20.7.27 `useFsc` condition for decoded nonzero coefficient blocks from
caller-resolved `enable_fsc`, `PlaneTxType`, `plane`, `fsc_mode`, and `is_inter`
facts before delegating to the existing `useFsc` branch selector.

#### Scenario: all-zero bypasses condition facts

- **WHEN** the wrapper receives decoded `all_zero == 1`
- **THEN** it delegates to the ordinary all-zero branch through the existing
  selector
- **AND** it does not require or evaluate `enable_fsc`, `PlaneTxType`, `plane`,
  `fsc_mode`, `is_inter`, ordinary nonzero facts, or FSC nonzero facts.

#### Scenario: derived false delegates to ordinary

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the caller-resolved condition facts do not satisfy
  `enable_fsc && PlaneTxType == IDTX && plane == 0 && (fsc_mode || is_inter)`
- **THEN** it delegates to the ordinary branch through the existing selector
- **AND** it returns the same ordinary result, tile CDF state, coefficient
  context state, consumed bits, and symbol count as the lower explicit-selector
  path with `use_fsc == false`.

#### Scenario: derived true delegates to FSC

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the caller-resolved condition facts satisfy
  `enable_fsc && PlaneTxType == IDTX && plane == 0 && (fsc_mode || is_inter)`
- **THEN** it delegates to the FSC branch through the existing selector
- **AND** it returns the same FSC result, tile CDF state, coefficient context
  state, consumed bits, and symbol count as the lower explicit-selector path
  with `use_fsc == true`.

#### Scenario: non-selected branch facts are ignored

- **WHEN** the wrapper derives `useFsc == false` with invalid FSC-only facts
- **THEN** it still executes the ordinary branch
- **AND** it does not return the FSC branch error.

#### Scenario: runtime scope remains unchanged

- **WHEN** the minimal runtime and staged coefficient tests run
- **THEN** output bytes remain unchanged
- **AND** runtime `coeffs()` integration, full `compute_tx_type`, runtime
  `PlaneTxType`, `is_inter`, `fsc_mode`, `enable_fsc`, `coeff_cdf_q_ctx`,
  dequantization, inverse transform, residual add, reconstruction, output,
  reference refresh, and full decoder conformance remain unsupported.
