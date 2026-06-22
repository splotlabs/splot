## ADDED Requirements

### Requirement: Sub-pel inter frame decodes bit-exact
The decoder SHALL decode a single-reference NEWMV sub-pel inter frame bit-exact
against the avmdec and dav2d oracles. The project SHALL commit a minimal
two-frame target `syn-2frame-subpel-inter-64x64.ivf` whose frame 0 is an
OBU_CLOSED_LOOP_KEY intra key frame (a single 64x64 DC_PRED half-cosine block,
decoded by the general-intra frontier) and whose frame 1 is an
OBU_REGULAR_TILE_GROUP inter frame coding a single 64x64 block, single reference,
NEWMV with an EighthPel `(0, -4)` horizontal half-sample sub-pel motion vector, a
SWITCHABLE `EIGHTTAP_SHARP` interpolation filter, and skip=1 (no residual). The
fixture SHALL be locally verified so that avmdec `--rawvideo --i420` and dav2d
`--demuxer ivf` decode the whole stream byte-for-byte identically (decoded-output
md5 `a0e82de3a95bb4b519c4c84ffa2ba816`, 12288 bytes), and SHALL be registered in
the conformance corpus validating clean with a reciprocal LOCAL-REFERENCE-EVIDENCE
entry.

The decoder SHALL read the motion vector from the bitstream via the AV2
§ 5.20.7.20 SHELL-coded `read_mv()` (the shell magnitude split plus the
§ 5.20.7.13 explicit `mv_sign` pass), read the § 5.20.7.6 `interp_filter` symbol
when the frame interpolation filter is SWITCHABLE and `needs_interp_filter()` is
1, derive the § 7.13.3.17 motion-vector scaling and § 7.13.3.18 reference-clipping
bounds, and run the § 7.13.3.18 separable interpolation-filter convolution to
reconstruct the prediction. No motion vector or interpolation filter SHALL be
hardcoded; § 8.2.4 `exit_symbol()` SHALL validate that every symbol read was
bit-exact, and a wrong read SHALL reject the frame rather than emit wrong output.

The decoder SHALL keep the verified subset narrow: residual (skip=0), compound,
multi-reference, motion modes (OBMC / warp), non-64x64 / multi-block inter,
flexible MV resolution, adaptive MVD, BAWP, and CWP SHALL be rejected with a
structured `decode/unsupported-feature` diagnostic before any output. The zero-MV
inter fixture and all existing intra fixtures SHALL keep decoding byte-identical.

#### Scenario: Sub-pel inter fixture validates clean and is oracle-verified
- **WHEN** `splot validate` is given `syn-2frame-subpel-inter-64x64.ivf`
- **THEN** it reports zero errors
- **AND** the LOCAL-REFERENCE-EVIDENCE manifest records that avmdec and dav2d
  decode the stream to the identical raw output (md5
  `a0e82de3a95bb4b519c4c84ffa2ba816`)

#### Scenario: Sub-pel inter frame decodes bit-exact via the convolution kernel
- **WHEN** `splot decode --output-format raw` is given
  `syn-2frame-subpel-inter-64x64.ivf`
- **THEN** the whole-stream raw output equals avmdec `--rawvideo --i420` and
  dav2d `--demuxer ivf` byte-for-byte (md5
  `a0e82de3a95bb4b519c4c84ffa2ba816`, 12288 bytes)
- **AND** frame 1 is a fractionally shifted version of frame 0 (it is NOT a copy),
  reconstructed by the § 7.13.3.18 interpolation-filter convolution over the
  decoded EighthPel sub-pel motion vector
- **AND** § 8.2.4 `exit_symbol()` passes over the inter tile payload

#### Scenario: Out-of-subset inter facts are rejected without output
- **WHEN** an inter frame uses a coded residual (skip=0), compound or
  multi-reference prediction, a motion mode, flexible MV resolution, adaptive
  MVD, BAWP, or CWP
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic and produces no output

#### Scenario: Existing zero-MV inter and intra fixtures are unchanged
- **WHEN** `splot decode` is given the zero-MV inter fixture
  (`syn-2frame-inter-64x64.ivf`) and the existing general intra fixtures
- **THEN** they decode bit-exactly to their pinned decoded-frame hashes (no
  regression)
