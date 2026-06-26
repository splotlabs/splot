## ADDED Requirements

### Requirement: 10-bit general intra DC reconstruction
The decoder SHALL reconstruct a 10-bit (AV2 § 6.4.1 Table 6.3
`bit_depth_idc == 0`) 4:2:0 general intra key frame, gated to the DC_PRED-luma +
DC-chroma square-leaf subset (single or multi 64x64 superblock, flat or AC
residual) with broad decode tools disabled. The
reconstruction graph SHALL be generic over the sample storage type
`T: ReconSample` (`u8` for 8-bit, `u16` for 10-bit) and SHALL thread a runtime
`BitDepth` derived from the sequence `bit_depth_idc`; the § 7.13.2.1 no-neighbour
prediction fallbacks (`AboveRow = (1 << (BitDepth - 1)) - 1`,
`LeftCol = (1 << (BitDepth - 1)) + 1`, corner `1 << (BitDepth - 1)`) SHALL be
derived from that `BitDepth`, and the § 7.14.4 dequantization, § 7.15.4 inverse
transform, and § 7.14.3 residual add (including the Clip1 sample bound) SHALL use
it. The decoder SHALL serialize the reconstructed 10-bit visible samples
16-bit-little-endian for the raw, Y4M, and `splot-dfh-sha256-v1` hash outputs. It
SHALL validate § 8.2.4 `exit_symbol()` after the whole tile. The 8-bit
reconstruction path SHALL remain byte-identical (the `T == u8` /
`BitDepth::Eight` specialization reproduces the prior 8-bit fallbacks
`127` / `129` / `128` exactly).

This requirement claims 10-bit multi-block (multi-64x64-superblock) square DC and
square AC reconstruction. It SHALL NOT claim 10-bit non-DC intra prediction
(SMOOTH, directional, PAETH, CFL), 10-bit non-64x64 partition-leaf
reconstruction (rectangular or split square sub-block), 10-bit inter prediction
or reference-frame retention, in-loop
filtering, or any AVM / dav2d invocation. Every 10-bit shape outside the verified
subset SHALL be rejected before any coefficient read or sample write with a
structured `decode/unsupported-feature` diagnostic.

#### Scenario: 10-bit flat DC intra decodes to the oracle
- **WHEN** `splot decode` is given the committed 10-bit intra key frame
  `syn-flat-intra-64x64-10bit-q80.ivf`
- **THEN** the general intra path reconstructs the single 64x64 DC_PRED-luma +
  DC-chroma block as a `DecodedFrame<u16>` and succeeds
- **AND** the reconstructed visible planes are flat Y == 400, U == 480, V == 520,
  matching the avmdec and dav2d raw outputs (raw md5
  `9983be8c8398de1db3127db7e6914bfa`)
- **AND** the `--output-format raw` bytes pack each visible sample
  16-bit-little-endian and equal that oracle output exactly

#### Scenario: 8-bit reconstruction stays byte-identical
- **WHEN** the existing 8-bit general intra fixtures
  (`syn-flat-intra-64x64-q80.ivf` and the rest of the corpus) are decoded after
  the genericization
- **THEN** each reconstructs to the same bytes as before
  (the `T == u8` / `BitDepth::Eight` specialization keeps the § 7.13.2.1
  fallbacks `127` / `129` / `128`)
- **AND** their pinned frame hashes are unchanged

#### Scenario: 10-bit AC-residual and multi-superblock DC decode to the oracle
- **WHEN** `splot decode` is given the committed 10-bit intra key frames
  `syn-cos-intra-64x64-10bit-q180.ivf` (single 64x64 DC_PRED-luma with eob > 1 AC
  luma residual) and `syn-2sb-intra-128x64-10bit-q80.ivf` (two 64x64 superblocks,
  left flat DC luma 400, right flat DC luma 460)
- **THEN** each reconstructs as a `DecodedFrame<u16>` and succeeds
- **AND** the `--output-format raw` bytes equal the avmdec / dav2d oracle output
  exactly (raw md5 `2751443b26dc632b6091192587af5ebb` and
  `5cbab50c4ff5ba0ba1ca28bfa8e97dde` respectively)

#### Scenario: 10-bit non-DC and richer shapes still reject
- **WHEN** a 10-bit stream is outside the DC_PRED-luma + DC-chroma square-leaf
  subset (for example a 10-bit SMOOTH / directional / PAETH non-DC stream, the
  10-bit CFL fixture `syn-intra-64x64-10bit.ivf`, a 10-bit non-64x64
  partition-leaf stream (rectangular or split 32x32 / 16x16 square sub-block), or
  a 10-bit `base_q_idx == 255` frozen-minimal-tier stream)
- **THEN** the decoder rejects it before any caller-visible output with a
  structured `decode/unsupported-feature` diagnostic
- **AND** the local `ac0ej3.ivf` 10-bit mission stream still fails closed at its
  current frontier with no new wrong output

#### Scenario: the four 10-bit reject guards are pinned by committed negative fixtures
- **WHEN** `splot decode` is given each of the four committed, validator-clean
  10-bit negative fixtures
- **THEN** `syn-smooth-intra-64x64-10bit-q80.ivf` (10-bit SMOOTH non-DC luma) is
  rejected with `unsupported_10bit_non_dc_intra`
- **AND** `syn-split-intra-64x64-10bit-q110.ivf` (10-bit frame split into DC
  32x32 sub-blocks) is rejected with `unsupported_10bit_non_64x64_leaf`
- **AND** `syn-flat-intra-64x64-10bit-q255.ivf` (10-bit `base_q_idx == 255`
  frozen-minimal-tier frame) is rejected with
  `unsupported_10bit_frozen_minimal_tier`
- **AND** `syn-2frame-inter-64x64-10bit.ivf` (10-bit key + inter frame
  referencing the 10-bit key) is rejected with
  `unsupported_10bit_reference_retention`
- **AND** each rejection is a structured `decode/unsupported-feature` diagnostic
  emitted before any caller-visible output
