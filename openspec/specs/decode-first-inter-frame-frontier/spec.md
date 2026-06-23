# decode-first-inter-frame-frontier Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `decode-first-inter-frame-frontier`.

## Requirements
### Requirement: First inter frame decode target and verified subset
The project SHALL commit a minimal two-frame inter decode target
`syn-2frame-inter-64x64.ivf` whose frame 0 is an OBU_CLOSED_LOOP_KEY intra key
frame and whose frame 1 is an OBU_REGULAR_TILE_GROUP inter frame coding a single
64x64 block, single reference, GLOBALMV/NEARMV with zero MV and skip=1 so that
AV2 § 7.13.3.18 zero-fraction motion compensation reduces to a straight copy of
the co-located key block (no residual). The fixture SHALL be locally verified so
that avmdec `--rawvideo --i420` and dav2d `--demuxer ivf` decode the whole stream
byte-for-byte identically (decoded-output md5
`4e1bd39f0b541ef1f479cff049e6985c`, 12288 bytes), with frame 1 equal to a copy of
frame 0, and SHALL be registered in the conformance corpus validating clean with
a reciprocal LOCAL-REFERENCE-EVIDENCE entry.

The decoder SHALL decode this verified single-reference zero-MV inter subset
bit-exact: the planner SHALL admit the inter `OBU_REGULAR_TILE_GROUP`, the
runtime SHALL retain the decoded key frame as an AV2 §7.23 reference, the inter
header SHALL parse to completion, the tile payload SHALL read the supported
single-reference skip-mode inter `mode_info`, and motion compensation SHALL copy
the co-located key-frame block before output. The row SHALL remain partial: any
inter syntax outside the verified single-reference zero-MV skip subset SHALL fail
closed with a structured `decode/unsupported-feature` diagnostic before output.

#### Scenario: Inter fixture validates clean and is oracle-verified
- **WHEN** `splot validate` is given `syn-2frame-inter-64x64.ivf`
- **THEN** it reports zero errors
- **AND** the LOCAL-REFERENCE-EVIDENCE manifest records that avmdec and dav2d
  decode the stream to the identical raw output (md5
  `4e1bd39f0b541ef1f479cff049e6985c`)

#### Scenario: Inter frame decodes bit-exact
- **WHEN** `splot decode --output-format raw` is given
  `syn-2frame-inter-64x64.ivf` with an output path
- **THEN** it succeeds and writes 12288 bytes of raw 8-bit 4:2:0 output whose md5
  is `4e1bd39f0b541ef1f479cff049e6985c`
- **AND** frame 1 is byte-identical to frame 0

#### Scenario: Existing intra fixtures are unchanged
- **WHEN** `splot decode` is given the existing general intra fixtures
- **THEN** they decode bit-exactly to their pinned decoded-frame hashes
