## ADDED Requirements

### Requirement: local decoder mission FSC transform-record residual handoff

The decoder SHALL extend `DECODE-SELECTABLE-TRANSFORM-RECORDS` with a
bounded live FSC residual handoff for the local decoder mission Wiener NS LR
transform-record path. When the path consumes supported AV2 §5.20.5.3
`fsc_mode` syntax and §5.20.7.27 nonzero luma coefficients, it SHALL derive the
coefficient branch from AV2 `useFsc = enable_fsc && PlaneTxType == IDTX &&
plane == 0 && (fsc_mode || is_inter)` and SHALL use that branch only for
syntax/metadata needed by live `LrTxSkip` retention. The runtime SHALL remain
fail-closed for decoded samples, reconstruction, loop-restoration filtering and
output, reference refresh, and successful local decoder mission decode.

#### Scenario: Active luma FSC residual records are consumed

- **WHEN** the local decoder mission Wiener NS LR transform-record path reaches a
  supported luma transform record with active `fsc_mode`
- **AND** AV2 §5.20.8.2 derives luma `PlaneTxType == IDTX`
- **AND** AV2 §5.20.7.27 derives `useFsc == true`
- **THEN** the runtime consumes the FSC coefficient syntax through the existing
  coefficient frame-facts handoff
- **AND** it retains the resulting transform-record `skip_flag` and `eob` facts
  for live `LrTxSkip` population
- **AND** it advances to the next structured unsupported-feature frontier

#### Scenario: Non-FSC and skipped residual behavior is preserved

- **WHEN** an admitted selectable transform record has `all_zero == 1` or
  derives `useFsc == false`
- **THEN** the runtime preserves the existing all-zero or ordinary non-FSC
  residual behavior
- **AND** it does not require or validate FSC-only branch facts for the
  non-selected branch

#### Scenario: Unsupported FSC reconstruction remains fail-closed

- **WHEN** FSC syntax has been consumed for LR tx-skip record derivation
- **THEN** the decoder SHALL NOT populate decoded `CurrFrame` or `CdefFrame`
  samples from that FSC coefficient state
- **AND** it SHALL NOT claim inverse transforms, residual add,
  loop-restoration filtering/output, reference refresh, AVM/dav2d byte
  equality, or successful local decoder mission decode
