## ADDED Requirements

### Requirement: Coeff all_zero context state handoff

The decoder support model SHALL track
`DECODE-COEFF-ALL-ZERO-CONTEXT-STATE` as a crate-private `splot-decode` row named
`coeff-all-zero-context-state`. The row SHALL cover the first `coeffs()`-adjacent
handoff from owned tile coefficient context state into the existing AV2 §8.3.2
`all_zero` (`txb_skip` / `v_txb_skip`) CDF context formulas. The row SHALL remain
partial until the full §5.20.7.27 `coeffs()` loop reads EOB and coefficient
symbols, fills `Quant[]`, and wires reconstruction.

#### Scenario: Luma all_zero reads level context state

- **WHEN** the luma `all_zero` context is derived for caller-resolved transform
  coordinates and geometry
- **THEN** the decoder OR-reduces `AboveLevelContext[0]` and
  `LeftLevelContext[0]` over the owned in-frame state slices
- **AND** it passes those reductions to the existing §8.3.2 luma `txb_skip`
  context formula
- **AND** out-of-range starts or pathological caller counts are bounded by the
  owned state slices and do not panic or spin

#### Scenario: V all_zero reads level and DC context state

- **WHEN** the V-plane `all_zero` context is derived for caller-resolved
  transform coordinates and geometry
- **THEN** the decoder OR-reduces V-plane above and left level/DC context lines
  over the owned in-frame state slices
- **AND** it passes the resulting above/left nonzero facts to the existing
  §8.3.2 V `v_txb_skip` context formula
- **AND** out-of-range starts or pathological caller counts are bounded by the
  owned state slices and do not panic or spin

#### Scenario: Minimal trace uses state-backed all_zero contexts

- **WHEN** the minimal flat-intra block-symbol trace reads the existing luma
  `txb_skip` and V `v_txb_skip` symbols
- **THEN** it allocates tile coefficient context state from the tile work-unit MI
  ranges and derives the same context values from state that were previously
  supplied as literal first-block reductions
- **AND** the no-output-change symbol-frontier test remains unchanged

#### Scenario: Full coefficient decode remains incomplete

- **WHEN** decoder support and conformance coverage are generated
- **THEN** `coeff-all-zero-context-state` appears as a partial row linked to
  `DECODE-COEFF-ALL-ZERO-CONTEXT-STATE`
- **AND** EOB decode, coefficient scan walk, `Quant[]`, `read_quant`,
  dequantization, reconstruction, and full decoder conformance remain partial
