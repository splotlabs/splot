## Context

The general intra decode path rejected every rectangular partition leaf
(`n4w != n4h`) with `general_intra_non_square_block`, so the PARTITION_HORZ /
PARTITION_VERT family was entirely unreachable. The § 5.20.3.1 partition
traversal already reads the real § 8.3.2 partition CDF and recurses through every
partition type (NONE/HORZ/VERT/SPLIT/HORZ3/VERT3/HORZ4A/B/VERT4A/B) — the gap was
only in the per-leaf block decode and reconstruction, which assumed a square
block.

## Decision

Add a dedicated rectangular leaf path rather than weaving rectangular handling
through the square mode dispatch. After the shared § 5.20.5.3 mode decode, a
rectangular leaf (`n4w != n4h`) branches to `decode_one_general_intra_rect_block`,
which is gated to the verified DC_PRED luma + DC chroma subset; every existing
square mode arm is untouched.

The lower-level machinery is already rectangular-capable:

- The § 5.20.7.27 coefficient geometry (`CoeffOrdinaryTxSizeGeometryConfig`),
  scan order, eob class, dequant, and inverse transform read `Tx_Width` /
  `Tx_Height` independently from the § 9.2 conversion tables. Only the
  general-intra `txb_skip` context-line OR-reduction spans used a single
  `span4 = 1 << tx_size`; those are now `w4 = Tx_Width[txSz] >> 2` (above) and
  `h4 = Tx_Height[txSz] >> 2` (left), which is correct for square too.
- `IntraRectBlockSize`, `intra_dc_edges_for_rect`, `predict_intra_dc_rect_value`,
  and `write_rect_block` already accept a rectangular block size.
- `inverse_transform_2d` already applies the § 7.15.4.1 √2 rescale when
  `|log2_width - log2_height|` is odd (the 64x32 / 32x16 case).

The rectangular `txSz` is derived from the block width/height log2 by scanning
the § 9.2 `Tx_Width_Log2` / `Tx_Height_Log2` conversion tables (no invented
constant) — under TX_MODE_LARGEST the single transform spans the whole block, so
its dimensions equal the block's (TX_64X32 luma, TX_32X16 chroma).

## Rationale

- DC prediction reads only the immediate left column / above row (§ 7.13.2.4), so
  no § 5.20.2.3 `BlockDecoded` sentinel state is needed — exactly the property the
  square deep-split brick relied on, now applied to a rectangular block.
- Non-DC rectangular modes (SMOOTH / directional luma, non-DC chroma) need
  rectangular § 7.13.2.8 / § 7.13.2.13 predictors plus the above-right /
  below-left sentinels, which are not yet modelled; they stay rejected, keeping
  the verified subset tight.

## Verified subset / out of scope

In scope: a DC_PRED luma + DC chroma rectangular leaf (PARTITION_HORZ / VERT) of
the verified 64x32 (TX_64X32) shape, fixtured by `syn-hrect-intra-64x64-q120.ivf`.
Out of scope (still rejected / deferred): non-DC rectangular luma (SMOOTH /
directional), non-DC rectangular chroma, rectangular leaves whose DC neighbour
edge needs the § 7.13.2.1 sentinels, non-64x64 frames, inter prediction, in-loop
filters, and live AVM/dav2d in CI.
