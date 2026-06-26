## ADDED Requirements

### Requirement: 10-bit general intra DC-luma + top-left SMOOTH-chroma reconstruction
The decoder SHALL reconstruct a 10-bit (AV2 § 6.4.1 Table 6.3
`bit_depth_idc == 0`) 4:2:0 general intra key frame whose single 64x64 leaf is a
DC_PRED luma block (flat or with AC residual) combined with § 7.13.2.13 SMOOTH
chroma at the NO-NEIGHBOUR top-left block (`frontier.r == 0 && frontier.c == 0`),
where the SMOOTH chroma predicts over the § 7.13.2.1 no-neighbour fallback edges.
The reconstruction MUST reuse the bit-depth-generic `T: ReconSample` + runtime
`BitDepth` reconstruction path and SHALL serialize the reconstructed 10-bit
visible samples 16-bit-little-endian for the raw, Y4M, and `splot-dfh-sha256-v1`
hash outputs. It SHALL validate § 8.2.4 `exit_symbol()` after the whole tile.

This requirement extends `DECODE-GENERAL-INTRA-10BIT` (DC_PRED luma + DC chroma)
only by admitting top-left no-neighbour SMOOTH chroma on the same single-64x64
square-leaf shape. It SHALL NOT claim a neighbour-having SMOOTH chroma block
(frame-MI `c != 0`, which would read real reconstructed 10-bit edges no fixture
pins), 10-bit non-DC luma prediction, non-64x64 partition-leaf reconstruction
(rectangular or split square sub-block), 10-bit inter prediction or
reference-frame retention, in-loop filtering, or any AVM / dav2d invocation.
Every 10-bit shape outside this admitted subset SHALL be rejected before any
coefficient read or sample write with a structured `decode/unsupported-feature`
diagnostic (`unsupported_10bit_non_dc_intra`). The 8-bit reconstruction path and
the existing 10-bit DC-chroma subset SHALL remain byte-identical.

#### Scenario: 10-bit DC luma + top-left SMOOTH chroma decodes to the oracle
- **WHEN** `splot decode` is given the committed 10-bit intra key frame
  `syn-smchroma-intra-64x64-10bit-q160.ivf`
- **THEN** the general intra path reconstructs the single 64x64 DC_PRED-luma +
  § 7.13.2.13 SMOOTH-chroma block as a `DecodedFrame<u16>` and succeeds
- **AND** the `--output-format raw` bytes pack each visible sample
  16-bit-little-endian and equal the avmdec and dav2d raw outputs exactly (raw
  md5 `a09a6344f3ec7a1efbb695d4f527d7c8`)
- **AND** the `--output-format hash` digest is the stable `splot-dfh-sha256-v1`
  value `4fe932e5e5dea4a1830eae4853b198c738e8d1919049736d2f4a234c491d5397`

#### Scenario: neighbour-having SMOOTH chroma and non-DC luma still reject
- **WHEN** a 10-bit stream has a neighbour-having SMOOTH chroma block
  (frame-MI `c != 0`), a 10-bit non-DC luma block, or a non-DC / non-(top-left
  SMOOTH) chroma block
- **THEN** the decoder rejects it before any caller-visible output with the
  structured `decode/unsupported-feature` diagnostic
  `unsupported_10bit_non_dc_intra`

#### Scenario: 8-bit and existing 10-bit DC-chroma subset stay byte-identical
- **WHEN** the existing 8-bit general intra fixtures and the three positive
  10-bit DC-chroma fixtures
  (`syn-flat-intra-64x64-10bit-q80.ivf`,
  `syn-cos-intra-64x64-10bit-q180.ivf`,
  `syn-2sb-intra-128x64-10bit-q80.ivf`) are decoded after the gate relaxation
- **THEN** each reconstructs to the same bytes as before
- **AND** the four 10-bit negative fixtures still fail closed with their existing
  `unsupported_*` reasons
