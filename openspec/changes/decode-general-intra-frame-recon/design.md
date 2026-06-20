## Context

The general intra frame path decodes mode info and the luma transform-block
coefficients, then returns `general_intra_chroma_decode_unimplemented`. The
coefficient-loop machinery, the § 7.14.4 dequantizer, the § 7.15.4 inverse
transform, and the § 7.14.3 residual-add (`reconstruct_transform_block_residual`)
all already exist in `splot-recon`; the gap is decoding the chroma coefficients
and composing reconstruction over all three planes.

## Goals / Non-Goals

**Goals:**
- Decode the U then V 32x32 chroma transform blocks' § 5.20.7.27 `coeffs()`.
- Reconstruct each plane (dequant, inverse transform, residual add over the
  no-neighbour DC prediction) and assemble the decoded frame.
- Validate § 8.2.4 `exit_symbol()` after the coefficients.
- Decode the committed q80 fixture bit-exactly to the avmdec/dav2d oracle.
- Keep the frozen `base_q_idx == 255` minimal hash contract byte-identical.

**Non-Goals:**
- No split partitions, multiple blocks, multiple tiles, or non-64x64 frames.
- No chroma `cctx`/CfL (the fixture has `enable_cctx == 0`), inter prediction,
  or in-loop filters.
- No live in-CI AVM/dav2d dependency; the oracle comparison is recorded as a
  pinned hash.

## Decisions

1. The chroma `all_zero` CDF second index is `is_inter || fsc_mode`, not
   plane_type.

   Rationale: AV2 § 8 parsing states that for plane 0 or 1 the `all_zero` CDF is
   `TileTxbSkipCdf[is_inter || fsc_mode][txSzCtx][ctx]`. The chroma-ness is
   carried in `ctx` (the U-plane `+6` branch), not the CDF's second index. For
   this intra frame `is_inter || fsc_mode == 0`. Plane 2 uses the separate
   `TileVTxbSkipCdf[ctx]` with the `EobU` contribution. Using the wrong index
   desynchronizes the arithmetic decoder.

2. The § 7.14.4 `dqDenom` includes the TCQ term for the luma block only.

   Rationale: `dqDenom = 1 << ((pels > 256) + (pels > 1024) + useTcq)` where
   `pels` is over the original (unadjusted) transform dimensions and `useTcq`
   reduces to the frame `allow_tcq` flag for the luma DCT_DCT (TX_CLASS_2D),
   non-lossless, non-FSC block. TCQ never applies to chroma (`plane != 0`). For
   the 64x64 luma block `dqDenom == 8`; for the 32x32 chroma blocks `dqDenom == 2`.

3. Generalize one `reconstruct_general_intra_block` over plane / log2 side.

   Rationale: the luma and chroma reconstructions differ only in plane id,
   transform size (64x64 reconstructed via the adjusted 32x32 inverse plus
   duplication vs native 32x32), and the TCQ term; a single function keeps them
   consistent.

4. Validate `exit_symbol()` after the coefficients.

   Rationale: the single 64x64 block consumes the entire tile payload, so a
   clean § 8.2.4 `exit_symbol()` is a strong correctness signal that the
   coefficient decode was bit-exact; a failure is surfaced as a structured
   `general_intra_exit_symbol` diagnostic.

## Risks / Trade-offs

- [Risk] The reconstruction is verified against avmdec/dav2d manually, not by a
  live in-CI harness.
  -> Mitigation: the avmdec and dav2d raw outputs agree byte-for-byte (flat
  Y=100, U=120, V=130) and the decoded-frame hash is pinned in both a
  `splot-decode` unit test and the CLI test; any drift fails those tests.
- [Risk] This brick covers only a single 64x64 single-DC block.
  -> Mitigation: the matrix and decoder-support rows stay partial and enumerate
  the unhandled cases (split partitions, multi-block, multi-tile, inter, filters,
  `cctx`/CfL); broader coverage is later bricks.
