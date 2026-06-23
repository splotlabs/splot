# coeff-cdf-q-context-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-cdf-q-context-handoff`.

## Requirements
### Requirement: Coefficient CDF q-context handoff

The decoder SHALL provide a crate-private loaded-but-unwired coefficient wrapper
for Feature ID `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF` that derives the active
coefficient CDF q-context from frame `base_q_idx` using AV2 § 6.17.2
`init_coeff_cdfs()` threshold buckets and delegates to the staged shared-facts
`useFsc` handoff.

#### Scenario: base_q_idx thresholds derive four q-context buckets

- **WHEN** the q-context helper receives `base_q_idx` values in the AV2
  threshold buckets `<= 90`, `91..=140`, `141..=190`, and `> 190`
- **THEN** it returns coefficient CDF q-context values `0`, `1`, `2`, and `3`
  respectively
- **AND** the helper is total and panic-free for `u32` input values beyond the
  syntax-domain maximum.

#### Scenario: all-zero bypasses base-q facts

- **WHEN** the wrapper receives decoded `all_zero == 1`
- **THEN** it delegates to the existing ordinary all-zero selector path
- **AND** it does not require or evaluate `base_q_idx`, shared nonzero facts,
  ordinary-only facts, or FSC-only facts.

#### Scenario: nonzero ordinary path derives q-context before delegation

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the shared nonzero facts select the ordinary branch
- **THEN** it derives `coeff_cdf_q_ctx` from `base_q_idx`
- **AND** it delegates to the existing shared-facts wrapper with the same
  result, tile CDF state, coefficient context state, consumed bits, and symbol
  count as an explicit shared-facts input carrying that q-context.

#### Scenario: nonzero FSC path derives q-context before delegation

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the shared nonzero facts select the FSC branch
- **THEN** it derives `coeff_cdf_q_ctx` from `base_q_idx`
- **AND** it delegates to the existing shared-facts wrapper with the same
  result, tile CDF state, coefficient context state, consumed bits, and symbol
  count as an explicit shared-facts input carrying that q-context.

#### Scenario: runtime scope remains unchanged

- **WHEN** focused staged coefficient tests and the minimal runtime tests run
- **THEN** decode output remains unchanged
- **AND** runtime `coeffs()` integration, full CDF lifecycle initialization,
  full `compute_tx_type`, runtime `PlaneTxType`, `enable_fsc`, `fsc_mode`,
  `is_inter`, transform geometry derivation, dequantization, inverse transform,
  residual add, reconstruction, output, reference refresh, and full decoder
  conformance remain unsupported.
