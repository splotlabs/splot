## ADDED Requirements

### Requirement: General intra single-block non-DC luma smooth decode
The decoder SHALL reconstruct the § 7.13.2.13 `SMOOTH_V_PRED` and
`SMOOTH_H_PRED` luma prediction modes for the top-left (no-neighbour) block of a
64x64 8-bit 4:2:0 intra key frame on the general intra path. It SHALL build the
smooth prediction over the § 7.13.2.1 no-neighbour fallback edges (8-bit:
`AboveRow` samples `(1 << (BitDepth - 1)) - 1`, `LeftCol` samples
`(1 << (BitDepth - 1)) + 1`, the smooth sentinels sharing those fallbacks) using
the shared `splot-recon` smooth predictor, and SHALL add the § 5.20.7.27 AC
residual over that per-sample prediction. It SHALL validate § 8.2.4
`exit_symbol()` after the coefficients. It SHALL gate the block decode to DC
chroma and the supported non-DC luma modes only at the top-left (no-neighbour)
block, rejecting non-DC chroma, the unsupported non-DC luma modes (`SMOOTH`,
`PAETH`), directional modes, and non-first-block non-DC prediction with a
structured `decode/unsupported-feature` diagnostic before any reconstruction. It
SHALL NOT handle multi-block non-DC prediction, non-64x64 frames, inter
prediction, in-loop filters, or invoke AVM or dav2d.

#### Scenario: Vertical-gradient frame decodes to the oracle
- **WHEN** `splot decode` is given the committed single-block intra key frame
  `syn-vsmooth-intra-64x64-q120.ivf`
- **THEN** the general intra path reconstructs one 64x64 `SMOOTH_V_PRED` luma
  block over the no-neighbour fallback edges plus AC residual, with DC chroma,
  and succeeds
- **AND** the reconstructed luma is a vertical gradient (each row constant across
  columns, increasing top-to-bottom) matching the avmdec and dav2d raw outputs
- **AND** the decoded-frame hash is the pinned
  `3aebe2eb215d4878bbc40aa2f97e2178b6140ef51c03afaaae478e69dbbf6bcd`

#### Scenario: Horizontal-gradient frame decodes to the oracle
- **WHEN** `splot decode` is given the committed single-block intra key frame
  `syn-hsmooth-intra-64x64-q120.ivf`
- **THEN** the general intra path reconstructs one 64x64 `SMOOTH_H_PRED` luma
  block, with DC chroma, and succeeds
- **AND** the reconstructed luma is a horizontal gradient (each column constant
  across rows, increasing left-to-right) matching the avmdec and dav2d raw outputs
- **AND** the decoded-frame hash is the pinned
  `cfc6debd26760cdebf1d1a4497792461f0f68bc7e7773741ddf2cbc34561e702`

#### Scenario: Unsupported non-DC cases are rejected before reconstruction
- **WHEN** a block uses a non-DC chroma mode, an unsupported non-DC luma mode
  (`SMOOTH`, `PAETH`, or a directional mode), or a supported non-DC luma mode on
  a block that has an above or left neighbour
- **THEN** the decoder returns a structured `decode/unsupported-feature`
  diagnostic without reconstructing the block
