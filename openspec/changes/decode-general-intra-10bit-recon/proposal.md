# Change: decode-general-intra-10bit-recon

## Feature IDs

- `DECODE-GENERAL-INTRA-10BIT`

## Why

The general intra decode path reconstructs only 8-bit (`DecodedFrame<u8>`)
frames: the runtime sample-storage gate (`ensure_runtime_storage_bit_depth`,
formerly `ensure_8bit_runtime_storage`) rejected every 10-bit
(`bit_depth_idc == 0`, AV2 § 6.4.1 Table 6.3) sequence with
`unsupported_bit_depth` before any reconstruction. Real AV2 content is routinely
10-bit, so the first 10-bit reconstruction increment is the smallest verifiable
step. The verified subset is the DC_PRED-luma + DC-chroma square-leaf shape
(single or multi 64x64 superblock, flat or AC residual), pinned by three
committed fixtures whose avmdec and dav2d raw outputs agree byte-for-byte: a flat
10-bit single-64x64 key frame (Y == 400, U == 480, V == 520; raw md5
`9983be8c8398de1db3127db7e6914bfa`), a single-64x64 key frame with eob > 1 AC
luma residual (raw md5 `2751443b26dc632b6091192587af5ebb`), and a
multi-64x64-superblock DC key frame (left luma 400, right luma 460; raw md5
`5cbab50c4ff5ba0ba1ca28bfa8e97dde`).

The splot-recon reconstruction primitives (`predict_intra_dc_rect_value`,
`reconstruct_transform_block_residual`, `dc_quantizer` / `ac_quantizer`,
`InverseTransform2dOuter::resolve`, `CurrentFrameWorkspace`, `DecodedFrame`, the
hash / raw / Y4M emitters) are already generic over `T: ReconSample` (`u8` and
`u16`) and already accept a runtime `BitDepth`. This change threads `T` plus a
runtime `bit_depth` through the splot-decode general-intra orchestration so the
same math reconstructs and serializes 8-bit and 10-bit samples.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-10BIT`.
- Genericize the splot-decode general-intra reconstruction graph over
  `T: ReconSample` and thread a runtime `bit_depth: BitDepth` (derived from the
  sequence `bit_depth_idc`): `new_general_intra_workspace`, every
  `reconstruct_general_intra_*_into` in `runtime_minimal_recon.rs`, the edge
  builders, and the `reconstruct_general_intra_block` / `_with_prediction` /
  `_rect` residual helpers in `tile_payload/general_intra_residual.rs`. The
  § 7.13.2.1 no-neighbour fallbacks (`AboveRow = (1 << (BitDepth - 1)) - 1`,
  `LeftCol = (1 << (BitDepth - 1)) + 1`, corner `1 << (BitDepth - 1)`) are
  derived from `bit_depth` instead of the hard-coded 8-bit `127` / `129` / `128`.
- Relax the runtime sample-storage gate to admit 10-bit for the DC_PRED-luma +
  DC-chroma square-leaf subset (single or multi 64x64 superblock, flat or AC
  residual). Every richer 10-bit shape stays rejected before output:
  `unsupported_10bit_non_dc_intra` (10-bit SMOOTH / directional / PAETH non-DC),
  `unsupported_cfl_intra` (10-bit CFL), `unsupported_10bit_non_64x64_leaf` (10-bit
  non-64x64 partition leaves — rectangular, or a split 32x32 / 16x16 square
  sub-block), `unsupported_10bit_frozen_minimal_tier`
  (a 10-bit frame on the frozen `base_q_idx == 255` minimal-tier path, which
  hard-codes 8-bit reconstruction), and `unsupported_10bit_reference_retention`
  (10-bit inter / reference-frame retention).
- Carry the displayed frame as either `DecodedFrame<u8>` or `DecodedFrame<u16>`
  (a `MinimalRuntimeDecodedFrame` enum on `MinimalRuntimeFrame`); the hash, raw,
  and Y4M adapters dispatch on the storage arm and call the already-generic
  splot-recon emitters (10-bit packs each visible sample 16-bit-LE). The inter /
  reference path stays 8-bit only and rejects 10-bit frames with a structured
  diagnostic.
- Add the project-owned `syn-flat-intra-64x64-10bit-q80.ivf`,
  `syn-cos-intra-64x64-10bit-q180.ivf`, and `syn-2sb-intra-128x64-10bit-q80.ivf`
  fixtures and prove each decodes bit-exactly to the avmdec/dav2d oracle.
- Pin each of the four 10-bit fail-closed reject guards with a committed,
  validator-clean negative fixture and a negative decode test:
  `syn-smooth-intra-64x64-10bit-q80.ivf` (`unsupported_10bit_non_dc_intra`),
  `syn-split-intra-64x64-10bit-q110.ivf` (`unsupported_10bit_non_64x64_leaf`),
  `syn-flat-intra-64x64-10bit-q255.ivf` (`unsupported_10bit_frozen_minimal_tier`),
  and `syn-2frame-inter-64x64-10bit.ivf`
  (`unsupported_10bit_reference_retention`).
- Add decode tests pinning the 10-bit flat plane values (Y == 400, U == 480,
  V == 520), the AC-residual frame hash, and the multi-superblock frame hash plus
  its per-superblock luma anchors (left 400, right 460); confirm all existing
  8-bit fixtures still decode bit-exact, the 10-bit CFL fixture still rejects, and
  `local-decoder-mission.ivf` still fails closed.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-10bit`: A 10-bit (`bit_depth_idc == 0`) general intra
  reconstruction gated to the DC_PRED luma + DC chroma single-64x64 subset,
  reconstructed and serialized bit-exact via the `T: ReconSample` + runtime
  `BitDepth` reconstruction path.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the 10-bit
  general intra DC reconstruction.

## Impact

- Adds `tests/conformance/vectors/valid/syn-flat-intra-64x64-10bit-q80.ivf`,
  `tests/conformance/vectors/valid/syn-cos-intra-64x64-10bit-q180.ivf`, and
  `tests/conformance/vectors/valid/syn-2sb-intra-128x64-10bit-q80.ivf` plus decode
  tests in `crates/splot-decode/src/runtime_minimal/general_intra_tests.rs`.
- Adds the four validator-clean 10-bit negative fixtures
  `tests/conformance/vectors/valid/syn-smooth-intra-64x64-10bit-q80.ivf`,
  `syn-split-intra-64x64-10bit-q110.ivf`,
  `syn-flat-intra-64x64-10bit-q255.ivf`, and
  `syn-2frame-inter-64x64-10bit.ivf` plus negative decode tests pinning the four
  10-bit fail-closed reject guards.
- Modifies `crates/splot-decode/src/runtime_minimal.rs`,
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`,
  `crates/splot-decode/src/runtime_minimal_recon.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_residual.rs`,
  `crates/splot-decode/src/runtime_hash.rs`,
  `crates/splot-decode/src/runtime_raw.rs`,
  `crates/splot-decode/src/runtime_y4m.rs`, and
  `crates/splot-decode/src/runtime_minimal/{inter,reference_buffer}.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`, and generated status/coverage docs.
- No public API, dependency graph, encoder, or validator changes. 10-bit non-DC
  intra (SMOOTH / directional / PAETH / CFL), 10-bit non-64x64 partition-leaf
  frames (rectangular or split square sub-block), 10-bit inter prediction /
  reference storage, in-loop filters, and live in-CI AVM/dav2d remain out of
  scope.

## Non-goals

- No 10-bit non-DC prediction (SMOOTH, directional, PAETH, CFL).
- No 10-bit non-64x64 partition-leaf reconstruction (rectangular or split square
  sub-block).
- No 10-bit inter prediction or 10-bit reference-frame retention.
- No change to the successful 8-bit fixture subset.

## Acceptance criteria

- [ ] `splot decode syn-flat-intra-64x64-10bit-q80.ivf`,
      `syn-cos-intra-64x64-10bit-q180.ivf`, and
      `syn-2sb-intra-128x64-10bit-q80.ivf` with `--output-format raw` each produce
      output byte-identical to avmdec/dav2d (md5 `9983be8c8398de1db3127db7e6914bfa`,
      `2751443b26dc632b6091192587af5ebb`, `5cbab50c4ff5ba0ba1ca28bfa8e97dde`).
- [ ] `--output-format hash` succeeds and emits a stable `splot-dfh-sha256-v1`
      digest.
- [ ] Every existing 8-bit conformance fixture stays byte-identical.
- [ ] The 10-bit CFL fixture `syn-intra-64x64-10bit.ivf` still rejects, and a
      10-bit non-DC / rectangular-leaf / frozen-tier / inter shape rejects before
      output.
- [ ] Feature tracking, OpenSpec, and generated docs are updated.
