# decode-general-intra-frame-recon Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `decode-general-intra-frame-recon`.

## Requirements
### Requirement: General intra full frame reconstruction
The decoder SHALL decode the chroma transform-block coefficients and reconstruct
the full frame of a minimal-tool 64x64 8-bit 4:2:0 intra key frame on the general
intra path. It SHALL decode the U (`plane == 1`) then V (`plane == 2`) 32x32
chroma transform blocks' AV2 § 5.20.7.27 `coeffs()`, reading the `all_zero`
(`txb_skip`) symbol with the § 8 parsing CDF `TileTxbSkipCdf[is_inter ||
fsc_mode][txSzCtx][ctx]` for plane 0 or 1 — where the second index is `is_inter
|| fsc_mode`, not plane_type, and the U-plane offset is carried in `ctx ==
(above != 0) + (left != 0) + 6` — and `TileVTxbSkipCdf[ctx]` for plane 2 with
`ctx == (EobU != 0) ? 6 : 0`, then routing the nonzero pass through the existing
coefficient-loop entry with the chroma plane. It SHALL reconstruct each plane by
composing the § 7.14.4 dequantization (with `dqDenom = 1 << ((pels > 256) + (pels
> 1024) + useTcq)`, the TCQ term applying to the luma DCT_DCT block only), the
§ 7.15.4 inverse transform, and the § 7.14.3 residual add over the § 7.13.2
no-neighbour DC prediction. It SHALL validate § 8.2.4 `exit_symbol()` after the
coefficients and assemble the decoded 8-bit 4:2:0 frame. It SHALL keep the frozen
`base_q_idx == 255` minimal hash contract byte-identical. It SHALL NOT handle
split partitions, multiple blocks, multiple tiles, non-64x64 frames, chroma
`cctx`/CfL, inter prediction, in-loop filters, or invoke AVM or dav2d.

#### Scenario: General intra fixture reconstructs to the AVM oracle
- **WHEN** `splot decode` is given the committed minimal-tool intra key frame
  `syn-flat-intra-64x64-q80.ivf` with the `hash` output format
- **THEN** the general intra path decodes the luma and chroma coefficients,
  reconstructs all three planes, and succeeds (exit code 0)
- **AND** the reconstructed luma plane is flat 100, the U plane flat 120, and the
  V plane flat 130, matching the avmdec and dav2d raw outputs
- **AND** the decoded-frame hash is the pinned
  `ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979`

#### Scenario: exit_symbol validates the coefficient decode
- **WHEN** the single 64x64 block's luma and chroma coefficients have been
  decoded
- **THEN** the AV2 § 8.2.4 `exit_symbol()` check holds, confirming the tile
  payload was consumed bit-exactly
- **AND** a failure is reported as a structured `decode/unsupported-feature`
  diagnostic with reason `general_intra_exit_symbol`

#### Scenario: base_q_idx == 255 frames route to the frozen tier, not the general path
- **WHEN** `splot decode` is given an intra key frame with `base_q_idx == 255`
- **THEN** the general intra reconstruction does not run for that frame; it
  routes to the frozen minimal hash tier
- **AND** the committed `syn-flat-intra-64x64-minimal.ivf` fixture is no longer a
  `base_q_idx == 255` frame: change `decode-minimal-fixture-avm-skip-polarity`
  replaced it with the AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream
  that routes through the general intra path
