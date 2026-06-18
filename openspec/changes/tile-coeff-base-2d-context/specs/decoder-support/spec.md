## ADDED Requirements

### Requirement: coeff_base significant-coefficient CDF context

The `splot-decode` tile CDF selection subset SHALL derive the AV2 § 8.3.2 `coeff_base` significant-coefficient CDF context, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `CoeffBaseContext::select` SHALL sum the significant-neighbour `Level[]` magnitudes at the generated § 9.2 `Sig_Ref_Diff_Offset[txClass]` offsets — over `SIG_REF_DIFF_OFFSET_NUM` samples for luma, 3 for chroma 2D, and 2 for chroma non-2D — each clamped by the position-dependent `magLimit` (5 for the low-frequency near-DC samples unless the coefficient is the parity-hidden DC, else 3), form `ctx = (mag + 1) >> 1`, and return a `CoeffBaseSelection` naming one of the five `coeff_base` banks with its bank-specific context offset: the parity-hidden DC bank (`Min(ctx, 4)`, overriding the others when `isHidden` and `c == 0`), the chroma and chroma low-frequency banks (`Min(ctx, 3)` plus the plane and 2D offsets), the luma low-frequency bank (the `c == 0` / `row + col` / horiz-col-vert-row sub-branches over `LF_SIG_COEF_CONTEXTS_2D`), and the luma high-frequency bank (`Min(ctx, 4)` plus the `row + col` position buckets, or `+ 15` for non-2D). It SHALL read a caller-provided row-major `txw`-wide `Level[]` slice with checked shifts, saturating flat-index geometry, and a slice-length guard (the spec `refRow < height && refCol < width` guard), so out-of-range or short-slice reads contribute `0` and the function is total and panic-free, and SHALL use the generated `Sig_Ref_Diff_Offset` conversion table rather than a duplicate. It SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The sign contexts, the full per-transform-block level/sign buffers, and the coefficient decode loop remain partial.

#### Scenario: coeff_base selects the right bank and context per branch

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `CoeffBaseContext::select` returns the parity-hidden, chroma,
  chroma-low-frequency, luma-low-frequency, and luma-high-frequency bank variants
  with the spec context offsets, with tests pinning the high-frequency `row + col`
  buckets, the non-2D `+ 15`, the low-frequency sub-branches, the chroma U-vs-V
  and 2D-vs-non-2D offsets, the clamped neighbour sum, the magLimit raise, and the
  parity-hidden override
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: coeff_base is total over short slices and bad geometry

- **WHEN** the `Level[]` slice is shorter than the block or the geometry is
  malformed
- **THEN** out-of-range neighbour reads contribute `0` and `select` returns
  without panicking
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Adding the context does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the context was added (the
  derivation is not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `coeff_base` context
- **AND** broader § 8.3 coefficient CDF selection (the sign contexts and the
  coefficient decode loop) remains partial
