## ADDED Requirements

### Requirement: Inter block residual decodes bit-exact
The decoder SHALL decode a single-reference zero-MV inter frame with `skip == 0`
(a coded § 5.20.7.27 residual added over the § 7.13.3.18 motion-compensated
prediction) bit-exact against the avmdec and dav2d oracles. The project SHALL
commit a minimal two-frame target `syn-2frame-inter-residual-64x64.ivf` whose
frame 0 is an OBU_CLOSED_LOOP_KEY intra key frame (a single flat 64x64 DC_PRED
block, decoded by the general-intra frontier) and whose frame 1 is an
OBU_REGULAR_TILE_GROUP inter frame coding a single 64x64 block, single reference,
zero-MV NEARMV, `skip == 0`, with a § 5.20.7.27 luma DCT_DCT residual over the
zero-fraction copy of frame 0 and flat chroma (no chroma residual). The fixture
SHALL be locally verified so that avmdec `--rawvideo --i420` and dav2d
`--demuxer ivf` decode the whole stream byte-for-byte identically (decoded-output
md5 `ab2b067aed48cf46035fa031cefb3ab1`, 12288 bytes), and SHALL be registered in
the conformance corpus validating clean with a reciprocal LOCAL-REFERENCE-EVIDENCE
entry.

The decoder SHALL read the residual coefficients from the bitstream via the AV2
§ 5.20.7.27 `coeffs()` syntax with the inter coefficient contexts: the § 8.3.2
`all_zero` (txb_skip) CDF SHALL be selected as
`TileTxbSkipCdf[is_inter || fsc_mode]` (the inter bank for `is_inter == 1`), and
the luma EOB context SHALL be `eobCtx = is_inter`. Under TX_MODE_LARGEST the
§ 5.20.6 `read_block_tx_size()` SHALL read no symbol (TxSize = TX_64X64 luma,
TX_32X32 chroma), and § 5.20.8.3 `get_tx_set` SHALL return `TX_SET_DCTONLY` for
those sizes so § 5.20.8.2 `transform_type()` reads no `inter_tx_type` symbol and
`PlaneTxType == DCT_DCT`. The decoder SHALL dequantize (§ 7.14.4), inverse
transform (§ 7.15.4), and add the residual (§ 7.14.3) over the motion-compensated
prediction, then clip to the bit depth. No coefficient, sign, or end-of-block
value SHALL be hardcoded; § 8.2.4 `exit_symbol()` SHALL validate that every
symbol read was bit-exact, and a wrong read SHALL reject the frame rather than
emit wrong output.

The decoder SHALL keep the verified subset narrow: a `skip == 0` block whose
sequence enables inter secondary transform (a `sec_tx_type` read), inter
data-driven transform, cross-chroma-component transform, forward skip coding, or
intra IDTX SHALL be rejected with a structured `decode/unsupported-feature`
diagnostic before any output, and compound / multi-reference prediction, motion
modes, non-64x64 / multi-block inter, flexible MV resolution, adaptive MVD, BAWP,
and CWP SHALL remain rejected. A `skip == 1` inter block reads no residual and
SHALL be unaffected by the residual-tool rejections. The skip == 1 inter fixtures
and all existing intra fixtures SHALL keep decoding byte-identical.

#### Scenario: Inter-residual fixture validates clean and is oracle-verified
- **WHEN** `splot validate` is given `syn-2frame-inter-residual-64x64.ivf`
- **THEN** it reports zero errors
- **AND** the LOCAL-REFERENCE-EVIDENCE manifest records that avmdec and dav2d
  decode the stream to the identical raw output (md5
  `ab2b067aed48cf46035fa031cefb3ab1`)

#### Scenario: Inter block with a coded residual decodes bit-exact
- **WHEN** `splot decode --output-format raw` is given
  `syn-2frame-inter-residual-64x64.ivf`
- **THEN** the whole-stream raw output equals avmdec `--rawvideo --i420` and
  dav2d `--demuxer ivf` byte-for-byte (md5
  `ab2b067aed48cf46035fa031cefb3ab1`, 12288 bytes)
- **AND** frame 1's luma differs from frame 0's flat luma (it is NOT a copy),
  reconstructed by the § 5.20.7.27 residual decode dequantized / inverse
  transformed / added over the § 7.13.3.18 zero-fraction prediction
- **AND** frame 1's chroma equals frame 0's chroma (the residual is luma-only)
- **AND** § 8.2.4 `exit_symbol()` passes over the inter tile payload

#### Scenario: Out-of-subset inter-residual facts are rejected without output
- **WHEN** a `skip == 0` inter frame's sequence enables inter secondary
  transform, inter data-driven transform, cross-chroma-component transform,
  forward skip coding, intra IDTX, or a non-zero effective quantizer delta
  (`DeltaQ* + Base*DeltaQ`)
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic and produces no output

#### Scenario: Existing skip-1 inter and intra fixtures are unchanged
- **WHEN** `splot decode` is given the zero-MV inter fixture
  (`syn-2frame-inter-64x64.ivf`), the sub-pel inter fixture
  (`syn-2frame-subpel-inter-64x64.ivf`), and the existing general intra fixtures
- **THEN** they decode bit-exactly to their pinned decoded-frame hashes (no
  regression), and the sub-pel fixture (which enables inter-IST / inter-DDT but
  is skip == 1) is not rejected by the residual-tool gate
