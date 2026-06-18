## ADDED Requirements

### Requirement: eob_pt coefficient CDF family

The `splot-decode` tile CDF selection subset SHALL include the AV2 `eob_pt` coefficient CDF family — the seven transform-size class banks `TileEobPt16Cdf` through `TileEobPt1024Cdf` — tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. Each bank SHALL be copied from its generated AV2 § 9.3 `Default_Eob_Pt_<size>_Cdf` defaults, and SHALL be selectable by an `EobPtSize` transform-size class together with `coeff_cdf_q_ctx` and `eobCtx` (AV2 § 8.3.2: `eob_pt_<size>` reads `TileEobPt<size>Cdf[eobCtx]`, with `eobCtx = (plane > 0) ? 2 : is_inter`). A `coeff_cdf_q_ctx` or `eob_ctx` outside its valid range SHALL return a typed `SelectorOutOfRange` error naming the `eob_pt` family, never panicking. The family SHALL participate in the supported-subset tile copy/average and frame-end count-scaling paths. The family is loaded but not consumed by a decode loop in this change (the § 5.20.7.27 `coeffs()` syntax that reads it is not wired), so the minimal-fixture decode output SHALL be unchanged. Broader § 8.3 coefficient CDF selection and the coefficient decode loop remain partial.

#### Scenario: eob_pt banks load defaults and select by size and context

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** each of the seven `eob_pt` banks copies its `Default_Eob_Pt_<size>_Cdf`
  table, and the `EobPt { size, coeff_cdf_q_ctx, eob_ctx }` selector returns the
  matching row for every valid size, `coeff_cdf_q_ctx`, and `eob_ctx`
- **AND** an out-of-range `coeff_cdf_q_ctx` or `eob_ctx` returns a typed
  `SelectorOutOfRange` naming the `eob_pt` family, and library code does not panic

#### Scenario: Adding the family does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the family was added (the banks
  are loaded but not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `eob_pt` family
- **AND** broader § 8.3 coefficient CDF selection (the remaining banks and the
  coefficient decode loop) remains partial
