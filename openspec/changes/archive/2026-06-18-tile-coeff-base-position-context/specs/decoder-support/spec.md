## ADDED Requirements

### Requirement: Coefficient base position CDF contexts

The `splot-decode` tile CDF selection subset SHALL derive the two position-only AV2 § 8.3.2 coefficient base CDF contexts, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `coeff_base_eob_ctx` SHALL compute the `coeff_base_eob` context by partitioning the scan position `c` against the adjusted transform block's coefficient count `Tx_Height[adjTxSz] << Tx_Width_Log2[adjTxSz]`: `0` when `c` is `0`, `1` when `c` is at most one eighth of the count, `2` when `c` is at most one quarter, and `3` otherwise (the `SIG_COEF_CONTEXTS_EOB - 4 ..= SIG_COEF_CONTEXTS_EOB - 1` contexts). `coeff_base_bob_ctx` SHALL compute the `coeff_base_bob` context by partitioning the begin position `bob` against the segment end-of-block `seg_eob`: `0` when `bob` is at most `seg_eob >> 3`, `1` when at most `seg_eob >> 2`, and `2` otherwise. Both SHALL be pure functions of caller-supplied scan and segment scalars plus caller-resolved adjusted geometry (needing no `Level[]` magnitude buffer), SHALL be total and panic-free (including an out-of-range shift width), and SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The `Level[]`-dependent coefficient contexts, the sign contexts, the per-transform-block level and sign buffers, and the coefficient decode loop remain partial.

#### Scenario: Coefficient base position contexts partition the position

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `coeff_base_eob_ctx` returns the four contexts across the
  `numCoeffs / 8` and `numCoeffs / 4` boundaries for TX_32X32 and TX_4X4
  geometry, and `coeff_base_bob_ctx` returns contexts 0/1/2 across the
  `seg_eob >> 3` and `seg_eob >> 2` boundaries
- **AND** an out-of-range shift width does not panic, and library code does not
  panic, overflow, or unwrap

#### Scenario: Adding the contexts does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the contexts were added (the
  derivations are not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the two position-only
  coefficient base contexts
- **AND** broader § 8.3 coefficient CDF selection (the `Level[]`-dependent
  contexts, the sign contexts, and the coefficient decode loop) remains partial
