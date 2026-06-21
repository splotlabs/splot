## Why

The general intra decode reconstructs the § 7.13.2.8 `D135_PRED` (pAngle 135) and
the two cardinal directional modes, but the § 7.13.2.8 luma IDIF 4-tap
interpolation filter `Dr_Interp_Filter` had NEVER been exercised by a real decode.
D135 is the degenerate case: its derivatives `dx = dy = Dr_Intra_Derivative[45] =
64` make every projection land on an integer sample (`shift == 0`), so the IDIF
filter row `Dr_Interp_Filter[0] == {0, 128, 0, 0}` reduces to a sample copy that is
bit-identical to the `enableIdif == 0` bilinear branch — D135 never proves the
4-tap. Every other middle/one-sided angle has `shift != 0`, where IDIF genuinely
differs from bilinear, but no such mode was decodable.

`Dr_Interp_Filter` existed nowhere in the crates (only the spec mirror), so the
luma middle-angle path used the chroma bilinear branch and the workspace rejected
`PlaneId::Y` with `WorkspaceDirectionalAngleIntraPredictionLumaIdifUnsupported`.

This change builds the § 7.13.2.8 / § 9.2 `Dr_Interp_Filter[32][4]` 4-tap kernel in
`splot-recon` and oracle-proves it by decoding a § 7.13.2.8 `D157_PRED` (pAngle 157)
directional luma block. D157 (`dx = Dr_Intra_Derivative[23] = 170`,
`dy = Dr_Intra_Derivative[67] = 24`) projects with a NONZERO shift for 2940 of its
3344 left-branch samples at a `haveLeft && !haveAbove` position, so it reads the
real reconstructed left column through the genuine 4-tap. The OLD code rejected a
D157 block (`general_intra_non_dc_chroma_mode`, because mode value 6 was unmapped).

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-IDIF-D157`.
- Add the § 7.13.2.8 / § 9.2 `Dr_Interp_Filter[32][4]` table (verbatim from the
  committed spec mirror) and the `enableIdif == 1` luma IDIF 4-tap path
  `predict_intra_middle_directional_angle_rect_idif_into` in `splot-recon`
  (tracked by `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION`); lift the
  `splot-recon` workspace luma rejection for the middle directional-angle path.
- Map § 9.2 mode value 6 to `SupportedDirectionalLumaMode::D157` and the
  `uv_mode == 0` follow to `SupportedChromaMode::D157Follow`; admit a
  first-superblock-row, non-first-column full-superblock D157 luma block reading
  the real reconstructed § 7.13.2.1 left column (luma via the IDIF 4-tap, chroma
  via the `enableIdif == 0` bilinear branch over the flat real chroma edge).
- Add the `syn-d157-intra-128x64-q80.ivf` fixture, its conformance manifest entry,
  the decoder support row, the decode matrix row, and the reciprocal
  LOCAL-REFERENCE-EVIDENCE entry.
- Keep D135 luma byte-identical (the `shift == 0` reduction), and keep D113, the
  one-sided angles (D45/D67/D203), non-zero angle deltas, and the top-left /
  first-column / sub-superblock / row>0 D157 positions rejected.

## Impact

- Affected specs: `decode-general-intra-idif-d157`, `decoder-support`.
- Affected code: `crates/splot-recon/src/intra_directional_angle.rs`,
  `crates/splot-recon/src/workspace_intra_directional_angle.rs`,
  `crates/splot-decode/src/runtime_minimal_recon.rs`,
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`,
  `crates/splot-decode/src/tile_payload/cdf/block_context.rs`.
- No dependency-graph change, no new dependency, no public CLI surface change.
