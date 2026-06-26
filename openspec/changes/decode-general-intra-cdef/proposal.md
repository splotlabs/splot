# Change: decode-general-intra-cdef

## Feature IDs

- `DECODE-GENERAL-INTRA-CDEF`
- `RECON-CDEF-FILTER`

## Why

The general intra decode path now reconstructs a frame and applies § 7.17
deblocking (the first in-loop-filter brick), but the route gate still rejects any
frame whose `cdef_frame_enable` is set, so the SECOND in-loop filter (AV2 § 7.18
CDEF) is unreached. The maintainer chose CDEF as the next in-loop-filter brick.
CDEF derings DC block-edge artifacts, so general-intra DC content is a natural
fit, and committed fixtures whose avmdec and dav2d raw outputs agree byte-for-byte
pin the § 7.18.2 direction search, the § 7.18.3 constrain / tap filter, and the
deblock-then-CDEF filter order.

## What Changes

- Add Feature IDs `DECODE-GENERAL-INTRA-CDEF` (the scheduler) and
  `RECON-CDEF-FILTER` (the leaf math).
- Add a new `crates/splot-recon/src/cdef_filter.rs` module with the scheduler-free
  AV2 § 7.18 CDEF sample math: `cdef_direction` (§ 7.18.2 direction search with the
  inline `Div_Table`), `cdef_constrain` (§ 7.18.3 constrain), and
  `cdef_filter_sample` (§ 7.18.3 primary/secondary tap accumulation with the inline
  `Cdef_Pri_Taps` / `Cdef_Sec_Taps`), plus the `Cdef_Directions` and `Cdef_Uv_Dir`
  constants for the caller's `cdef_get_at` addressing.
- Add a new `crates/splot-decode/src/runtime_minimal/cdef.rs` module that
  orchestrates the AV2 § 7.18 / § 7.18.1 64x64-unit → 8x8-block CDEF traversal over
  those primitives: it snapshots the deblocked frame, iterates the 8x8 blocks
  (`cdef_idx == 0` everywhere in the admitted subset), derives the § 7.18.1 luma /
  chroma strengths / direction / damping, fetches each output sample's six § 7.18.3
  directional taps with the § 5.20.9.3 `is_inside_filter_region`
  (single-tile → `is_inside_frame`) availability check, and writes the deringed
  samples back in place after § 7.17 deblocking and before `workspace.freeze()`
  (filter order: deblock → CDEF).
- Relax the general intra route gate to admit a CDEF-active frame in the verified
  subset (`CdefStrengths == 1` so § 5.20.10.1 `read_cdef` reads no per-block symbol;
  `cdef_on_skip_txfm_frame_enable == 1`; 8-bit), keeping GDF/CCSO/loop-restoration
  rejected, and rejecting a multi-strength frame and a 10-bit CDEF-active frame.
- Add the project-owned `syn-2sb-cdef-intra-128x64-q130.ivf`,
  `syn-2sb-cdef-intra-128x64-q120.ivf`, and
  `syn-2sb-cdefdeblock-intra-128x64-q100.ivf` fixtures and decode tests proving
  they reconstruct bit-exactly to the avmdec / dav2d oracle, and that a CDEF-off
  frame stays byte-identical.
- Update decoder tracking (matrix, decoder-support, LOCAL-REFERENCE-EVIDENCE,
  conformance manifest) and the generated status docs.

## Capabilities

### New Capabilities
- `decode-general-intra-cdef`: An AV2 § 7.18 CDEF edge-traversal orchestration on
  the general intra decode path that applies the parsed CDEF parameters in place
  over the deblocked frame, bit-exact vs avmdec/dav2d for the verified 8-bit 4:2:0,
  single-strength-set, single-tile, segmentation-disabled subset.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra § 7.18 CDEF orchestration.

## Impact

- `crates/splot-recon/src/cdef_filter.rs` (new) and `crates/splot-recon/src/lib.rs`
  (exports).
- `crates/splot-decode/src/runtime_minimal/cdef.rs` (new), and the general intra
  route gate + frame reconstruction in
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`.
- Tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`, and the four generated status docs.
