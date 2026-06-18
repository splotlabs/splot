## ADDED Requirements

### Requirement: coeff_br coefficient base-range CDF context

The `splot-decode` tile CDF selection subset SHALL derive the AV2 § 8.3.2 `coeff_br` coefficient base-range CDF context, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `CoeffBrContext::ctx` SHALL, for a coefficient at scan position `pos` in an adjusted transform block of caller-resolved geometry (`bwl`, `txw`, `txh`) and a caller-provided row-major `Level[]` magnitude slice, compute the context by: deriving `row`/`col` from `pos` and `bwl`; summing up to three neighbour magnitudes at the § 8.3.2 `Mag_Ref_Offset_With_Tx_Class` offsets for the transform class (only the first two offsets when the transform class is not 2D and the plane is chroma), each clamped to `MAX_BASE_BR_RANGE - 1`; halving and clamping the sum as `Min((mag + 1) >> 1, 6)`; and offsetting it by plane (chroma `Min(mag, 3)`), DC position (non-2D `mag + 7`), or low-frequency (`mag + 7`). It SHALL read the level magnitudes over the caller-provided slice with the spec `refRow < txh && refCol < txw` bound (and a slice-length guard), so out-of-bounds and short-slice neighbour reads contribute `0` and the function is total and panic-free. It SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The remaining `Level[]`-dependent contexts, the sign contexts, the full per-transform-block level/sign buffers, and the coefficient decode loop remain partial.

#### Scenario: coeff_br sums and offsets the context per the spec

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `CoeffBrContext::ctx` sums the clamped neighbour magnitudes, halves and
  clamps to 6, and applies the plane / DC-position / low-frequency offsets, with
  tests pinning the chroma `Min(mag, 3)` clamp, the non-2D `mag + 7` offset, and
  the non-2D-chroma two-neighbour case (distinguished from the three-neighbour
  case)
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: coeff_br is total over out-of-bounds and short slices

- **WHEN** neighbour offsets leave the transform block, or the `Level[]` slice is
  shorter than the block
- **THEN** those reads contribute `0` and `ctx` returns without panicking
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Adding the context does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the context was added (the
  derivation is not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `coeff_br` context
- **AND** broader § 8.3 coefficient CDF selection (the remaining `Level[]`-dependent
  contexts, the sign contexts, and the coefficient decode loop) remains partial
