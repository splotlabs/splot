## Context

The skip frame's all-zero residual reconstructs to the flat 128 predictor. A coded frame must
carry a real coefficient so the decoder dequantizes and adds residual. The general 64x64 intra
path codes the EOB symbol with a different size class than the minimal tier (`eob_pt_1024` vs
`eob_pt_16`) and the `coeff_base_lf_eob` at the `TX_64X64` `txSzCtx`.

## Decision: pin the coded contexts against the decoder

The coded contexts were pinned by instrumenting the decoder's coefficient reads while decoding
the AVM-validated q80 fixture (itself a coded single-DC frame in all three planes): luma reads
`TxbSkip{tx 4} = 0`, `EobPt{Pt1024} = 0`, `CoeffBaseLfEob{tx 4} = 4`, `CoeffBrLf = 2`,
`DcSign = 1`. The encoder reproduces that exact symbol sequence at the same contexts.

## Decision: magnitude 6, not 7 — the golomb threshold

The decode oracle caught a real bug. q80's luma is level 7 (`coeff_base_eob` 4 + `coeff_br` 2),
reconstructing to 100. But the minimal sequence header's luma uses TCQ, so § 5.20.7.28
`read_quant` reads a golomb bypass tail once `quant >= maxLevel - allowTcq == 7`. At level 7
the decoder reads ~8 bypass bits after `dc_sign` that the encoder did not emit, so it
over-reads into the exit padding and § 8.2.4 `exit_symbol()` fails. Bypass (`read_literal`)
reads are invisible to the CDF-symbol trace, so the bug surfaced only through the cross-crate
decode — confirming the value of the oracle.

This brick emits magnitude **6** (`coeff_base_eob` 4 + `coeff_br` 1), the largest level below
the golomb threshold, reconstructing to a flat luma **127**. The golomb tail needed for level 7
(the q80 luma value 100) is a follow-up brick.

## Decision: skipped chroma

U and V stay skipped (flat 128), so the coded change is isolated to luma. The V `txb_skip`
keeps the neutral `ctx 0` (`EobU == 0`, since U is skipped), as in the skip frame.

## Honesty

Decode is verified against `splot-decode` (AVM/dav2d-validated on the coded q80/q180 fixtures),
not by decoding this exact stream with AVM. This is the first decodable coded frame, not a
general encoder or Baseline Encoder Profile v1.
