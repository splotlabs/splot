## ADDED Requirements

### Requirement: useFsc coefficient branch handoff

The decoder SHALL provide a crate-private loaded-but-unwired coefficient branch
selector for Feature ID `DECODE-COEFF-USE-FSC-BRANCH-HANDOFF` that accepts a
caller-resolved AV2 § 5.20.7.27 `useFsc` fact and dispatches between the
existing ordinary and FSC branch handoffs. The selector SHALL preserve AV2
ordering by handling decoded `all_zero == 1` before the nonzero `useFsc` split.

#### Scenario: all-zero routes through ordinary all-zero

- **WHEN** the selector receives decoded `all_zero == 1`
- **THEN** it delegates to the ordinary all-zero branch
- **AND** it returns the same ordinary branch result, tile CDF state, coefficient
  context state, consumed bits, and symbol count as the direct ordinary all-zero
  branch
- **AND** it does not require or evaluate `useFsc` or FSC-specific facts.

#### Scenario: nonzero ordinary branch delegates when useFsc is false

- **WHEN** the selector receives decoded `all_zero == 0` with
  caller-resolved `useFsc == false`
- **THEN** it delegates to `apply_coeff_ordinary_branch_from_lossless`
- **AND** it returns the same ordinary branch result, tile CDF state,
  coefficient context state, consumed bits, and symbol count as the direct
  ordinary branch lower-boundary input.

#### Scenario: nonzero FSC branch delegates when useFsc is true

- **WHEN** the selector receives decoded `all_zero == 0` with
  caller-resolved `useFsc == true`
- **THEN** it delegates to `apply_coeff_fsc_branch_from_tx_size`
- **AND** it returns the same FSC branch result, tile CDF state, coefficient
  context state, consumed bits, and symbol count as the direct FSC tx-size
  lower-boundary input.

#### Scenario: selected branch failures are typed and atomic

- **WHEN** the selector receives contradictory or invalid facts for the selected
  branch
- **THEN** it returns that selected lower branch's typed error through a
  selector-specific error wrapper
- **AND** it preserves tile CDF state, coefficient context state, and
  symbol-decoder position according to the selected lower branch's preflight
  guarantees.

#### Scenario: runtime scope remains unchanged

- **WHEN** the minimal runtime and existing staged coefficient tests run
- **THEN** output bytes remain unchanged
- **AND** runtime `useFsc` derivation, full `compute_tx_type`, `transform_type`,
  `cctx_type`, `EobU`, dequantization, inverse transform, residual add,
  reconstruction, output, reference refresh, and full decoder conformance remain
  unsupported.
