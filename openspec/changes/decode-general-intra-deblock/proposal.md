# Change: decode-general-intra-deblock

## Feature IDs

- `DECODE-GENERAL-INTRA-DEBLOCK`

## Why

The general intra decode path reconstructs a frame bit-exactly but the route gate
rejects any frame whose `apply_deblocking_filter` is not all-false, so the FIRST
in-loop filter (AV2 § 7.17 deblocking) is unreached. The `splot-recon`
deblocking sample math (§ 7.17.7.1 sample filter, § 7.17.7.2 filter choice,
§ 7.17.3 max-width, § 7.17.5 adaptive strength) is already unit-tested but has no
caller: the § 7.17.1 / § 7.17.2 edge traversal and § 7.17.6 filter-level
derivation are the missing orchestration. The maintainer chose deblock as the
first in-loop-filter brick. This is pinned by committed fixtures whose avmdec and
dav2d raw outputs agree byte-for-byte and whose ONLY difference from the
deblock-off reconstruction is the § 7.17 pass.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-DEBLOCK`.
- Add a new `crates/splot-decode/src/runtime_minimal/deblock.rs` module that
  orchestrates the AV2 § 7.17.1 / § 7.17.2 deblocking-filter edge traversal over
  the existing `splot-recon` per-edge primitives: it derives the per-(plane,
  pass) § 7.17.6 filter level, the § 7.17.5 (qThr, side) strengths, iterates the
  plane × pass × MI edge loop (with 4:2:0 chroma row/col steps), gathers each
  perpendicular sample line from the `CurrentFrameWorkspace`, and applies the
  § 7.17.7.2 filter choice + § 7.17.7.1 sample filter in place after the block
  walk and before `workspace.freeze()`.
- Add a checked single-sample writer `CurrentFrameWorkspace::set_reconstructed_sample`
  to `splot-recon` so the deblock pass can write modified samples back across
  block boundaries.
- Relax the general intra route gate to admit deblock-active frames in the
  verified subset (`df_delta_q` all zero; 8-bit for a deblock-active frame; the
  apply pattern this fixture uses), keeping the other in-loop filters
  (GDF/CDEF/CCSO/loop-restoration) rejected, and rejecting nonzero `df_delta_q`
  and a 10-bit deblock-active frame.
- Add the project-owned `syn-2sb-deblock-intra-128x64-q100.ivf`,
  `syn-2sb-deblock-intra-128x64-q98.ivf`, and
  `syn-2sb-deblockwide-intra-128x64-q100.ivf` fixtures and decode tests proving
  they reconstruct bit-exactly to the avmdec / dav2d oracle, and that a
  deblock-off frame stays byte-identical.
- Update decoder tracking (matrix, decoder-support, LOCAL-REFERENCE-EVIDENCE,
  conformance manifest) and the generated status docs.

## Capabilities

### New Capabilities
- `decode-general-intra-deblock`: An AV2 § 7.17 deblocking-filter edge-traversal
  orchestration on the general intra decode path that applies the parsed
  `apply_deblocking_filter` passes in place over the reconstructed frame, bit-exact
  vs avmdec/dav2d for the verified 8-bit 4:2:0, zero-`df_delta_q`, single-tile,
  segmentation-disabled subset.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra § 7.17 deblocking-filter orchestration.

## Impact

- `crates/splot-decode/src/runtime_minimal/deblock.rs` (new), and the general
  intra route gate + block walk + frame reconstruction in
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`.
- `crates/splot-recon/src/workspace.rs` (new checked single-sample writer).
- Tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`, and the four generated status docs.
