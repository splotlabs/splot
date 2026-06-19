## ADDED Requirements

### Requirement: useFsc shared-facts handoff

The decoder SHALL provide a crate-private loaded-but-unwired coefficient wrapper
for Feature ID `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF` that accepts decoded
all-zero coefficient inputs or one shared nonzero coefficient fact packet,
derives the AV2 section 5.20.7.27 `useFsc` condition from that packet, and
constructs only the selected ordinary or FSC lower branch input.

#### Scenario: all-zero bypasses shared nonzero facts

- **WHEN** the wrapper receives decoded `all_zero == 1`
- **THEN** it delegates to the ordinary all-zero branch through the existing
  selector path
- **AND** it does not require or evaluate nonzero shared facts, condition facts,
  ordinary-only facts, or FSC-only facts.

#### Scenario: derived false constructs only ordinary input

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the shared condition facts do not satisfy
  `enable_fsc && PlaneTxType == IDTX && plane == 0 && (fsc_mode || is_inter)`
- **THEN** it constructs the ordinary nonzero lower input from the shared
  geometry, CDF q-context, ordinary base config, `is_inter`, and lossless facts
- **AND** it delegates to the ordinary branch with the same result, tile CDF
  state, coefficient context state, consumed bits, and symbol count as the lower
  explicit ordinary path.

#### Scenario: derived true constructs only FSC input

- **WHEN** the wrapper receives decoded `all_zero == 0`
- **AND** the shared condition facts satisfy
  `enable_fsc && PlaneTxType == IDTX && plane == 0 && (fsc_mode || is_inter)`
- **THEN** it derives the FSC block input from shared geometry and generated
  `Tx_Width[txSz]` / `Tx_Height[txSz]` table values
- **AND** it delegates to the FSC branch with the same result, tile CDF state,
  coefficient context state, consumed bits, and symbol count as the lower
  explicit FSC path.

#### Scenario: non-selected branch facts are ignored

- **WHEN** the wrapper derives `useFsc == true` with invalid ordinary-only facts
- **THEN** it still executes the FSC branch
- **AND** it does not return the ordinary branch error.

#### Scenario: false predicate does not validate FSC-only facts

- **WHEN** the wrapper derives `useFsc == false`
- **THEN** it does not construct or validate the FSC lower input
- **AND** it does not return an FSC-only error from non-selected FSC facts.

#### Scenario: runtime scope remains unchanged

- **WHEN** the minimal runtime and staged coefficient tests run
- **THEN** output bytes remain unchanged
- **AND** runtime `coeffs()` integration, full `compute_tx_type`, runtime
  `PlaneTxType`, `enable_fsc`, `fsc_mode`, `is_inter`, `coeff_cdf_q_ctx`,
  transform geometry derivation, dequantization, inverse transform, residual
  add, reconstruction, output, reference refresh, and full decoder conformance
  remain unsupported.
