## ADDED Requirements

### Requirement: General intra single-block plain SMOOTH luma decode
The decoder SHALL reconstruct the § 7.13.2.13 plain `SMOOTH_PRED` luma
prediction mode (canonical § 9.2 mode 9) for the top-left (no-neighbour) 64x64
block of an 8-bit 4:2:0 intra key frame on the general intra path. It SHALL build
the smooth prediction over the § 7.13.2.1 no-neighbour fallback edges (8-bit:
`AboveRow` samples `(1 << (BitDepth - 1)) - 1`, `LeftCol` samples
`(1 << (BitDepth - 1)) + 1`, the smooth sentinels `AboveRow[w]` / `LeftCol[h]`
sharing those fallbacks) using the shared `splot-recon` smooth predictor in its
plain 2-D mode (`Round2(predV2 + predH2, 1)`, blending BOTH the above row +
top-right and the left column + bottom-left), and SHALL add the § 5.20.7.27 AC
residual over that per-sample prediction. It SHALL validate § 8.2.4
`exit_symbol()` after the coefficients. It SHALL gate the plain SMOOTH block
decode to DC chroma at the top-left (no-neighbour) 64x64 superblock — the
§ 5.20.8.2 `get_tx_set` `TX_SET_DCTONLY` size where the transform is forced
`DCT_DCT` with no `intra_tx_type` signaled — rejecting non-DC chroma,
neighbour-having plain SMOOTH (which reads the real reconstructed § 7.13.2.1
above-right / below-left sentinels), and sub-64x64 plain SMOOTH (which can signal
a mode-dependent transform type) with a structured `decode/unsupported-feature`
diagnostic before any reconstruction. It SHALL NOT handle plain SMOOTH chroma,
non-64x64 frames, inter prediction, in-loop filters, or invoke AVM or dav2d.

#### Scenario: 2-D smooth frame decodes to the oracle
- **WHEN** `splot decode` is given the committed single-block intra key frame
  `syn-smooth-intra-64x64-q124.ivf`
- **THEN** the general intra path reconstructs one 64x64 plain `SMOOTH_PRED` luma
  block over the no-neighbour fallback edges plus AC residual, with DC chroma,
  and succeeds
- **AND** the reconstructed luma rises along both the top row and the left column
  (neither is constant, distinct from SMOOTH_V / SMOOTH_H) matching the avmdec
  and dav2d raw outputs
- **AND** the decoded-frame hash is the pinned
  `9b054c6fff47397fbe88a9eb45a34fac018efc7748fc697edebddd3f14bd88d3`

#### Scenario: Plain SMOOTH luma with non-DC chroma is rejected before reconstruction
- **WHEN** `splot decode` is given the committed single-block intra key frame
  `syn-smoothnondc-intra-64x64-q132.ivf` whose luma is plain `SMOOTH_PRED` but
  whose chroma is a non-DC mode
- **THEN** the decoder returns a structured `decode/unsupported-feature`
  diagnostic (`general_intra_non_dc_chroma_mode`) without reconstructing the
  block
