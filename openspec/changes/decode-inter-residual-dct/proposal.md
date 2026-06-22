## Why

The first inter frames decode bit-exact for the no-residual cases: the zero-MV
copy (`DECODE-FIRST-INTER-FRAME-FRONTIER`) and the sub-pel NEWMV prediction
(`DECODE-INTER-SUBPEL-MV`), both with `skip == 1`. Real inter content carries a
coded residual added over the motion-compensated prediction — this is the most
fundamental remaining inter piece. The intra residual pipeline already exists and
is reusable: the AV2 § 5.20.7.27 coefficient loop, § 7.14.4 dequantization,
§ 7.15.4 inverse transform, and § 7.14.3 residual addition all run on the
general-intra frontier. This change reuses them for inter, adding only the
inter-specific coefficient contexts, and adds the residual over the inter
predictor rather than an intra predictor.

The smallest bit-exact-verifiable inter-residual step is a two-frame stream
(1 intra key + 1 inter frame) whose inter frame is a single 64x64 block, single
reference, zero-MV, `skip == 0`, carrying a low-frequency luma DCT_DCT residual
(flat chroma, no chroma residual). The zero MV isolates the residual path from
sub-pel motion compensation: frame 0 is flat, so the zero-MV copy is also flat
and the entire luma difference is the decoded residual.

## What Changes

- Add Feature ID `DECODE-INTER-RESIDUAL-DCT`.
- Add the project-owned `syn-2frame-inter-residual-64x64.ivf` fixture (frame 0 =
  a flat-100 general-intra DC_PRED key frame; frame 1 = an
  OBU_REGULAR_TILE_GROUP single-reference zero-MV NEARMV inter frame with
  `skip == 0` carrying a § 5.20.7.27 luma DCT_DCT residual over the § 7.13.3.18
  zero-fraction copy; flat chroma, no chroma residual). Prove avmdec
  `--rawvideo --i420` and dav2d `--demuxer ivf` decode the whole stream
  byte-for-byte identically (decoded-output md5
  `ab2b067aed48cf46035fa031cefb3ab1`, 12288 bytes).
- Thread an `is_inter` parameter through `decode_general_intra_plane_coeffs` so
  the § 8.3.2 `all_zero` (txb_skip) CDF selects the inter bank
  (`TileTxbSkipCdf[is_inter || fsc_mode]`, index 1 for inter) and the
  § 5.20.7.27 nonzero pass derives the inter luma `eobCtx = is_inter`. The
  `coeff_base` / `coeff_br` CDF banks are unified across inter/intra (no separate
  inter dimension). All intra callers pass `is_inter = false`, an exact no-op.
- When the inter block decodes `skip == 0`, read the § 5.20.7.27 residual: under
  TX_MODE_LARGEST `read_block_tx_size()` reads no symbol (TxSize = TX_64X64);
  § 5.20.8.3 `get_tx_set(TX_64X64, 0)` returns `TX_SET_DCTONLY`, so § 5.20.8.2
  `transform_type()` reads no `inter_tx_type` symbol and `PlaneTxType == DCT_DCT`
  (the chroma TX_32X32 is also DCT-only). Read the luma TX_64X64 then U/V TX_32X32
  coefficients via the shared intra coefficient loop with `is_inter == true`.
- Add `reconstruct_inter_block_residual_into`: read the § 7.13.3.18 MC prediction
  block from the workspace, compose the § 7.14.4 dequant / § 7.15.4 inverse
  transform / § 7.14.3 residual addition over it (the luma DCT_DCT TCQ `dqDenom`
  term applies only when the frame's `allow_tcq` is set; chroma never), and write
  the reconstruction back. An `all_zero` plane is a no-op (the prediction stands).
- Relax the inter block decode to admit `skip == 0` (the residual subset),
  keeping every assumed-absent transform/coefficient tool rejected: a `skip == 0`
  block whose sequence enables inter-IST (a `sec_tx_type` read), inter-DDT, CCTX,
  FSC, or IDTX-intra is rejected with a structured `decode/unsupported-feature`
  diagnostic before any output. A `skip == 1` block reads no residual and is
  unaffected (the existing sub-pel fixture enables inter-IST / inter-DDT and must
  still decode).

## Impact

- Affected specs: `decode-inter-residual-dct` (new), `decoder-support` (new row).
- Affected code: `crates/splot-decode/src/runtime_minimal/inter/block.rs`,
  `crates/splot-decode/src/runtime_minimal/inter.rs`,
  `crates/splot-decode/src/runtime_minimal/inter/mc.rs`,
  `crates/splot-decode/src/runtime_minimal_recon.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_residual.rs`,
  `crates/splot-decode/src/runtime_minimal/general_intra.rs` (intra callers pass
  `is_inter = false`).
- New fixture: `tests/conformance/vectors/valid/syn-2frame-inter-residual-64x64.ivf`,
  conformance manifest entry, reciprocal LOCAL-REFERENCE-EVIDENCE entry.
- The skip == 1 inter fixtures (zero-MV `4e1bd39f`, sub-pel `a0e82de3`) and the
  general-intra fixtures stay byte-identical (no regression).
