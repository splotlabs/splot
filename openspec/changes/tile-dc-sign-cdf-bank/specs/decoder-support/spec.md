## ADDED Requirements

### Requirement: dc_sign coefficient CDF bank

The `splot-decode` tile CDF selection subset SHALL include the AV2 `TileDcSignCdf` coefficient CDF bank, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. The bank SHALL be copied from the generated AV2 § 9.3 `Default_Dc_Sign_Cdf` defaults, and SHALL be selectable by `coeff_cdf_q_ctx`, `plane_type`, the `isHidden` group, and the DC-sign `ctx` (AV2 § 8.3.2: `dc_sign` reads `TileDcSignCdf[ptype][isHidden][ctx]`). Each of the four selector index axes SHALL be bounds-checked and return a typed `SelectorOutOfRange` error naming the `dc_sign` bank, never panicking. The bank SHALL participate in the supported-subset tile copy/average and frame-end count-scaling paths. The § 8.3.2 `ctx` derivation from the Above/Left DC-context buffers is not implemented in this change (those buffers do not exist yet), so the bank is loaded but not consumed by a decode loop, and the minimal-fixture decode output SHALL be unchanged. Broader § 8.3 coefficient CDF selection and the coefficient decode loop remain partial.

#### Scenario: dc_sign bank loads defaults and selects across all indices

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** the `dc_sign` bank copies `Default_Dc_Sign_Cdf`, and the
  `DcSign { coeff_cdf_q_ctx, plane_type, group, ctx }` selector returns the
  matching row for every valid combination of the four indices
- **AND** an out-of-range value on any of the four axes returns a typed
  `SelectorOutOfRange` naming the `dc_sign` bank, and library code does not panic

#### Scenario: Adding the bank does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the bank was added (the bank
  is loaded but not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `dc_sign` bank
- **AND** broader § 8.3 coefficient CDF selection (the `ctx` derivation, the
  remaining banks, and the coefficient decode loop) remains partial
