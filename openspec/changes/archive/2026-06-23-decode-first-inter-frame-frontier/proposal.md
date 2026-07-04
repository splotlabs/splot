## Why

The general intra decode path is comprehensive (all intra modes, square and
rectangular partitions, multi-superblock grids), but the decoder cannot yet
decode any INTER frame. Inter prediction is the gate to real content: the
production target `local decoder mission` is ~1 key + ~12961 inter frames. The smallest
bit-exact-verifiable inter step is a two-frame stream (1 intra key + 1 inter
frame) whose inter frame is a single 64x64 block, single reference,
GLOBALMV/NEARESTMV with zero MV and skip=1, so AV2 § 7.13.3.18 zero-fraction
motion compensation reduces to a straight copy of the co-located key block (no
residual). That inter frame therefore decodes bit-exact to a copy of frame 0.

This change lands the verified target fixture and pins the honest current
rejection, ahead of the full inter decode slice. It does NOT fabricate an inter
decode.

## What Changes

- Add Feature ID `DECODE-FIRST-INTER-FRAME-FRONTIER`.
- Add the project-owned `syn-2frame-inter-64x64.ivf` fixture (frame 0 =
  OBU_TEMPORAL_DELIMITER + OBU_SEQUENCE_HEADER + OBU_CLOSED_LOOP_KEY; frame 1 =
  OBU_TEMPORAL_DELIMITER + OBU_REGULAR_TILE_GROUP). Prove avmdec `--rawvideo
  --i420` and dav2d `--demuxer ivf` decode the whole stream byte-for-byte
  identically (decoded-output md5 `4e1bd39f0b541ef1f479cff049e6985c`, 12288
  bytes; frame 1 == a copy of frame 0).
- Register the fixture in the conformance manifest (`expect = "clean"`) and add
  the reciprocal LOCAL-REFERENCE-EVIDENCE entry recording the avm/dav2d
  agreement.
- Add a decode test pinning the honest current behaviour: the initial stream
  planner accepts only OBU_CLOSED_LOOP_KEY as a frame candidate (§ 5.2.1), so the
  inter OBU_REGULAR_TILE_GROUP is rejected up front with a structured
  `decode/unsupported-feature` diagnostic and NO output.
- Document the empirically-established next blocker: the § 7.7 `get_ref_frames()`
  implicit reference-map derivation. This fixture uses PRIMARY_REF_CHOOSE + the
  implicit reference map (the dav2d-compatible path; `--explicit-ref-frame-map=1`
  makes dav2d diverge from avmdec), so the § 5.18.2 inter frame-header parser
  stops at `InterStop::UnmodeledDerivation` and `get_ref_frames()` must be
  modeled before the inter header (NumTotalRefs / ref_frame_idx[] and thus all
  later bit positions) resolves.

## Capabilities

### New Capabilities
- `decode-first-inter-frame-frontier`: A committed, oracle-verified minimal
  two-frame inter decode target (1 intra key + 1 zero-MV skip inter frame) and a
  pinned honest rejection of the inter frame at the stream planner, ahead of the
  full inter decode slice.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the first
  inter frame decode frontier.

## Impact

- Adds `tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf` and a decode
  test in `crates/splot-cli/tests/decode_cli.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, and generated status/coverage
  docs.
- No public API, dependency graph, encoder, or validator changes. The full inter
  decode slice — multi-frame planner + runtime loop, § 7.23 reference retention
  into the splot-recon `ReferenceFrameStore`, the § 7.7 `get_ref_frames()`
  derivation, the § 5.18.2 inter frame-header shared tail, § 5.20 inter
  mode_info, § 7.11 zero-MV derivation, § 7.13.3.18 motion-compensation copy, and
  frame-1 output — remains out of scope for this change.
