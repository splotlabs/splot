## ADDED Requirements

### Requirement: eob_extra coefficient CDF bank

The `splot-decode` tile CDF selection subset SHALL include the AV2 `TileEobExtraCdf` coefficient CDF bank, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. The bank SHALL be copied from the generated AV2 § 9.3 `Default_Eob_Extra_Cdf` defaults, and SHALL be selectable by `coeff_cdf_q_ctx` with no per-symbol context (AV2 § 8.3.2: the cdf for `eob_extra` is given directly by `TileEobExtraCdf`). A `coeff_cdf_q_ctx`
outside the valid range SHALL return a typed `SelectorOutOfRange` error naming the
`TileEobExtraCdf` array, never panicking. The bank SHALL participate in the
supported-subset tile copy/average and frame-end count-scaling paths. The bank is
loaded but not consumed by a decode loop in this change (the § 5.20.7.27
`coeffs()` syntax that reads it is not wired), so the minimal-fixture decode
output SHALL be unchanged. Broader § 8.3 coefficient CDF selection and the
coefficient decode loop remain partial.

#### Scenario: eob_extra bank loads the generated defaults and selects by q-context

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** the frame CDF subset copies `Default_Eob_Extra_Cdf` into the
  `eob_extra` bank without aliasing, and the `EobExtra { coeff_cdf_q_ctx }`
  selector returns the matching row for each valid `coeff_cdf_q_ctx`
- **AND** an out-of-range `coeff_cdf_q_ctx` returns a typed `SelectorOutOfRange`
  naming `TileEobExtraCdf`, and library code does not panic

#### Scenario: Adding the bank does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the bank was added (the bank
  is loaded but not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `eob_extra` bank
- **AND** broader § 8.3 coefficient CDF selection (the remaining banks and the
  coefficient decode loop) remains partial
