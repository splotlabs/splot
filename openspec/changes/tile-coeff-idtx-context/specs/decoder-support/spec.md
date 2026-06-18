## ADDED Requirements

### Requirement: IDTX coefficient magnitude CDF contexts

The `splot-decode` tile CDF selection subset SHALL derive the two AV2 § 8.3.2 identity-transform coefficient magnitude CDF contexts, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `coeff_base_idtx_ctx` SHALL compute the `coeff_base_idtx` context as `Min(3, Level[row][col-1]) + Min(3, Level[row-1][col])` (each neighbour included only when in range), and `coeff_br_idtx_ctx` SHALL compute the `coeff_br_idtx` context the same way with the `MAX_BASE_BR_RANGE - 1` per-neighbour clamp followed by `Min(mag, 6)`. Both SHALL read a caller-provided row-major `txw`-wide `Level[]` slice (`level[row * txw + col]`), with saturating flat-index geometry and a slice-length guard so out-of-range or short-slice reads contribute `0` and the functions are total and panic-free. Both results SHALL be the spec `mag`, used directly as the inner CDF index. They SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The remaining `Level[]`-dependent context, the sign contexts, the full level/sign buffers, and the coefficient decode loop remain partial.

#### Scenario: IDTX magnitude contexts sum the clamped neighbours

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `coeff_base_idtx_ctx` sums the left and above neighbours clamped to 3,
  and `coeff_br_idtx_ctx` sums them clamped to `MAX_BASE_BR_RANGE - 1` then clamps
  the total to 6, with tests pinning the col==0 / row==0 missing-neighbour skips
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: IDTX contexts are total over short slices and bad geometry

- **WHEN** the `Level[]` slice is shorter than the block or the geometry is
  malformed
- **THEN** out-of-range reads contribute `0` and the functions return without
  panicking
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Adding the contexts does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the contexts were added (the
  derivations are not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the two IDTX magnitude contexts
- **AND** broader § 8.3 coefficient CDF selection (the `coeff_base` context, the
  sign contexts, and the coefficient decode loop) remains partial
