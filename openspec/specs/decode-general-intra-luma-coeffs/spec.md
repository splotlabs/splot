# decode-general-intra-luma-coeffs Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `decode-general-intra-luma-coeffs`.

## Requirements
### Requirement: General intra luma coefficient decode
The decoder SHALL provide a crate-private AV2 § 5.20.7.27 `coeffs()` decode for
the single non-partitioned 64x64 luma transform block of a minimal-tool intra
key frame. It SHALL read the `all_zero` (`txb_skip`) symbol with the § 8.3.2
context derived from `coeff_cdf_q_ctx` (from `base_q_idx`), the transform-size
context `(Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1` over the generated
§ 9.2 tables, and the first-block tx-fills-block luma context, and when
`all_zero == 0` it SHALL route the nonzero coefficient pass through the existing
coefficient-loop entry (`PlaneTxType == DCT_DCT`, plane 0, intra) and return the
decoded `Quant[]` and end-of-block. It SHALL be wired into the general intra
frame path after mode decode so the structured `decode/unsupported-feature`
rejection advances to chroma decode. It SHALL NOT decode chroma coefficients,
dequantize, inverse transform, add residuals, reconstruct pixels, commit tile
context lines, or invoke AVM or dav2d.

#### Scenario: General intra fixture decodes luma coefficients and reaches chroma
- **WHEN** `splot decode` is given the committed minimal-tool intra key frame
  `syn-flat-intra-64x64-q80.ivf`
- **THEN** the general intra path reads the luma `all_zero` symbol as `0` and
  decodes the § 5.20.7.27 luma transform-block coefficients through the nonzero
  coefficient pass without error
- **AND** it emits a `decode/unsupported-feature` diagnostic with reason
  `general_intra_chroma_decode_unimplemented`

#### Scenario: Transform-size context matches the spec formula
- **WHEN** the `txb_skip` transform-size context is derived for a square
  transform size
- **THEN** it equals `(Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1`
  (4 for `TX_64X64`)
- **AND** an out-of-range transform size derives 0 rather than panicking

#### Scenario: base_q_idx == 255 frames route to the frozen tier, not the general path
- **WHEN** `splot decode` is given an intra key frame with `base_q_idx == 255`
- **THEN** the general intra luma coefficient decode does not run for that frame;
  it routes to the frozen minimal hash tier
- **AND** the committed `syn-flat-intra-64x64-minimal.ivf` fixture is no longer a
  `base_q_idx == 255` frame: change `decode-minimal-fixture-avm-skip-polarity`
  replaced it with the AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream
  that routes through the general intra path
