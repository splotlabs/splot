## ADDED Requirements

### Requirement: First inter frame decode target and honest rejection
The project SHALL commit a minimal two-frame inter decode target
`syn-2frame-inter-64x64.ivf` whose frame 0 is an OBU_CLOSED_LOOP_KEY intra key
frame and whose frame 1 is an OBU_REGULAR_TILE_GROUP inter frame coding a single
64x64 block, single reference, GLOBALMV/NEARESTMV with zero MV and skip=1 so that
AV2 § 7.13.3.18 zero-fraction motion compensation reduces to a straight copy of
the co-located key block (no residual). The fixture SHALL be locally verified so
that avmdec `--rawvideo --i420` and dav2d `--demuxer ivf` decode the whole stream
byte-for-byte identically (decoded-output md5
`4e1bd39f0b541ef1f479cff049e6985c`, 12288 bytes), with frame 1 equal to a copy of
frame 0, and SHALL be registered in the conformance corpus validating clean with
a reciprocal LOCAL-REFERENCE-EVIDENCE entry.

Until the inter decode slice lands, the decoder SHALL reject the inter frame
honestly: the initial stream planner accepts only OBU_CLOSED_LOOP_KEY as a frame
candidate (§ 5.2.1), so an OBU_REGULAR_TILE_GROUP SHALL be rejected with a
structured `decode/unsupported-feature` diagnostic and SHALL produce no output.
The decoder SHALL NOT fabricate an inter decode, SHALL NOT emit partial or wrong
output for the inter frame, and SHALL leave all existing intra fixtures decoding
bit-exact.

#### Scenario: Inter fixture validates clean and is oracle-verified
- **WHEN** `splot validate` is given `syn-2frame-inter-64x64.ivf`
- **THEN** it reports zero errors
- **AND** the LOCAL-REFERENCE-EVIDENCE manifest records that avmdec and dav2d
  decode the stream to the identical raw output (md5
  `4e1bd39f0b541ef1f479cff049e6985c`)

#### Scenario: Inter frame is rejected at the planner without output
- **WHEN** `splot decode --output-format raw` is given
  `syn-2frame-inter-64x64.ivf` with an output path
- **THEN** it fails with a structured `decode/unsupported-feature` diagnostic
  whose feature id is `DECODE-STREAM-STATE-PLANNER` and whose obu type is
  `OBU_REGULAR_TILE_GROUP`
- **AND** no output file is written

#### Scenario: Existing intra fixtures are unchanged
- **WHEN** `splot decode` is given the existing general intra fixtures
- **THEN** they decode bit-exactly to their pinned decoded-frame hashes
