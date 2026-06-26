# Change: decode-general-intra-cdef-chroma

## Feature IDs

- `DECODE-GENERAL-INTRA-CDEF`

## Why

The merged § 7.18 CDEF brick (`decode-general-intra-cdef`) admits only a verified
subset and, to stay honest, OVER-REJECTS any CDEF-active frame whose chroma (uv)
strengths are nonzero: the route gate requires
`uv_pri_strength == 0 && uv_sec_strength == 0`. The chroma glue — the § 7.18.1
`Cdef_Uv_Dir[SubsamplingX][SubsamplingY][yDir]` direction selection, the 4:2:0
subsampled 4x4 chroma tap addressing, and the `CdefDamping - 1` chroma damping —
is therefore spec-correct-by-inspection but exercised by ZERO oracle fixtures (all
committed CDEF fixtures use uv strengths 0). This change pins and then admits
sample-changing chroma CDEF so the chroma path is hashed against the oracle rather
than over-rejected.

## What Changes

- Add the project-owned `syn-2sb-cdefuv-intra-128x64-q170.ivf` fixture: an 8-bit
  4:2:0 intra key frame the encoder gave NONZERO chroma CDEF strengths
  (`uv_pri 2`, `uv_sec 4`) while staying general-intra-admissible (two 64x64
  PARTITION_NONE superblocks, DC_PRED luma; left non-follow H_PRED chroma, right DC
  chroma). avmdec and dav2d agree byte-for-byte (raw md5
  `d783f353078cf156ba23dcfd3b2b50ad`) and splot reproduces it bit-exact; the chroma
  CDEF changes over a thousand U and V samples each (isolated by re-decoding the
  same bitstream with the chroma strengths forced to zero).
- Add the fixture to the positive CDEF decode-hash test (frame hash
  `9b11d0effa3b93e84c63306e9ac865921e33f6e098cc35fbc472cbd6096ee3e6`).
- Add deterministic `cdef.rs` unit tests that drive a nonzero-uv strength set over
  a synthetic chroma ripple and assert the chroma output changes (chroma-only,
  bounded), and that the `Cdef_Uv_Dir` direction selection tracks the luma `yDir`
  ONLY when `uv_pri != 0` (validating the direction selection, the subsampled tap
  addressing, and the chroma damping deterministically).
- Relax the general intra route gate to admit nonzero-uv CDEF (drop the
  `uv_*_strength == 0` clause, keeping `CdefStrengths == 1`,
  `cdef_on_skip_txfm_frame_enable == 1`, a present damping / strength set, and the
  8-bit restriction), and un-qualify the `cdef.rs` module docstring's "luma-only,
  chroma-no-op subset" wording.
- Update decoder tracking (matrix, decoder-support, LOCAL-REFERENCE-EVIDENCE,
  conformance manifest) and the generated status docs.

## Capabilities

### Modified Capabilities
- `decode-general-intra-cdef`: Admit sample-changing chroma (nonzero-uv) CDEF on
  the general intra decode path, bit-exact vs avmdec/dav2d, pinned by a nonzero-uv
  fixture and deterministic chroma unit tests.
- `decoder-support`: Extend the `general-intra-cdef` decoder-support row to record
  the nonzero-uv chroma CDEF admission and its oracle evidence.

## Impact

- Fixture: `tests/conformance/vectors/valid/syn-2sb-cdefuv-intra-128x64-q170.ivf`
  (new), `tests/conformance/manifest.toml`.
- Route gate + decode-hash test:
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`,
  `crates/splot-decode/src/runtime_minimal/general_intra_tests/general_intra_cdef_tests.rs`.
- Chroma unit tests + docstring:
  `crates/splot-decode/src/runtime_minimal/cdef.rs`.
- Tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`, and the
  four generated status docs.
